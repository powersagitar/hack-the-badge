import { describe, expect, test } from "bun:test";
import {
  createProvisioner,
  PUT_CHUNK_BYTES,
  PUT_CHUNK_GAP_STEPS,
} from "../src/runtime/provisioner";
import type { FirmwareEmulatorHandle } from "../src/cpu/bridge";

/**
 * A fake console: records each `serialInput` call as a string; `print`
 * appends firmware output; `advance` moves `totalSteps()` on.
 */
function fakeHandle(accept: (bytes: Uint8Array) => number = (b) => b.length) {
  let output = "";
  let steps = 0;
  const typed: string[] = [];
  const handle = {
    serialInput(bytes: Uint8Array) {
      const n = accept(bytes);
      typed.push(new TextDecoder().decode(bytes.subarray(0, n)));
      return n;
    },
    serialPending: () => 0,
    consoleOutput: () => output,
    totalSteps: () => steps,
  } as unknown as FirmwareEmulatorHandle;
  return {
    handle,
    typed,
    print: (s: string) => {
      output += s;
    },
    advance: (n: number) => {
      steps += n;
    },
  };
}

/** Drives a fake through the prompt and `READY`, so the first chunk is sent. */
function toFirstChunk(json: string) {
  const f = fakeHandle();
  const p = createProvisioner(f.handle, json);
  f.print("badge> ");
  p.poll();
  f.print("READY\r\n");
  p.poll();
  return { f, p };
}

describe("provisioner", () => {
  test("provisioner waits for the prompt before typing", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, '{"a":1}');
    expect(p.poll().kind).toBe("waiting");
    f.advance(10_000_000);
    expect(p.poll().kind).toBe("waiting");
    expect(f.typed).toEqual([]);
    f.print("badge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed).toEqual(["put /littlefs/identity.json 7\n"]);
  });

  test("sends the payload only after READY", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, '{"a":1}');
    f.print("badge> ");
    p.poll();
    f.advance(1_000_000);
    expect(p.poll().kind).toBe("typing");
    expect(f.typed.length).toBe(1);
    f.print("READY\r\n");
    p.poll();
    expect(f.typed[1]).toBe('{"a":1}');
  });

  test("paces the payload: 64-byte chunks at least 48,000 steps apart, one per poll", () => {
    expect(PUT_CHUNK_BYTES).toBe(64);
    expect(PUT_CHUNK_GAP_STEPS).toBe(48_000);
    const json = "x".repeat(150);
    const { f, p } = toFirstChunk(json);
    expect(f.typed.slice(1)).toEqual(["x".repeat(64)]);
    f.advance(PUT_CHUNK_GAP_STEPS - 1);
    p.poll();
    expect(f.typed.length).toBe(2);
    f.advance(1);
    p.poll();
    p.poll();
    expect(f.typed.slice(1)).toEqual(["x".repeat(64), "x".repeat(64)]);
    // A slow poller (the browser's frame loop) still sends one chunk per poll.
    f.advance(10 * PUT_CHUNK_GAP_STEPS);
    p.poll();
    p.poll();
    expect(f.typed.slice(1)).toEqual(["x".repeat(64), "x".repeat(64), "x".repeat(22)]);
    expect(f.typed.slice(1).join("")).toBe(json);
  });

  test("waits for OK and the prompt, then types prov apply and reports PROV OK", () => {
    const { f, p } = toFirstChunk('{"a":1}');
    f.print("OK 7\r\n");
    p.poll();
    expect(f.typed.length).toBe(2);
    f.print("badge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed[2]).toBe("prov apply\n");
    f.print("PROV OK id=test-fake");
    expect(p.poll().kind).toBe("typing");
    f.print("-badge-0001\r\n");
    expect(p.poll()).toEqual({ kind: "ok", line: "PROV OK id=test-fake-badge-0001" });
    expect(p.poll()).toEqual({ kind: "ok", line: "PROV OK id=test-fake-badge-0001" });
    expect(f.typed.length).toBe(3);
  });

  test("reports a put error", () => {
    const { f, p } = toFirstChunk("{}");
    f.print("short read: 2 bytes missing\r\n");
    expect(p.poll()).toEqual({ kind: "failed", reason: "short read: 2 bytes missing" });
  });

  test("reports an error before READY", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, "{}");
    f.print("badge> ");
    p.poll();
    f.print("write error\r\n");
    expect(p.poll()).toEqual({ kind: "failed", reason: "write error" });
  });

  test("reports PROV FAIL", () => {
    const { f, p } = toFirstChunk("{}");
    f.print("OK 2\r\nbadge> ");
    p.poll();
    f.print("PROV FAIL invalid or missing /littlefs/identity.json\r\n");
    expect(p.poll()).toEqual({
      kind: "failed",
      reason: "PROV FAIL invalid or missing /littlefs/identity.json",
    });
  });

  test("fails when the serial queue refuses input", () => {
    const f = fakeHandle(() => 0);
    const p = createProvisioner(f.handle, "{}");
    f.print("badge> ");
    expect(p.poll().kind).toBe("failed");
  });

  test("ignores console text from before it started", () => {
    const f = fakeHandle();
    f.print("badge> READY\r\nOK 2\r\nbadge> ");
    const p = createProvisioner(f.handle, "{}");
    p.poll();
    p.poll();
    p.poll();
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
  });
});

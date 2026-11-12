import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  BADGE_TOKEN_FAIL,
  BADGE_TOKEN_OK,
  BADGE_TOKEN_READY,
  createProvisioner,
  FAKE_UPLOAD_TOKEN,
  PUT_CHUNK_BYTES,
  PUT_CHUNK_GAP_STEPS,
  type ProvisionOptions,
} from "../src/runtime/provisioner";
import { consoleOffsetOf, consoleSince } from "../src/runtime/console-io";
import type { FirmwareEmulatorHandle } from "../src/cpu/bridge";

/**
 * A fake console: records each `serialInput` call as a string; `print`
 * appends firmware output; `advance` moves `totalSteps()` on; `reboot`
 * bumps `softResetCount()`. The console mirrors the core's byte ring
 * (`console.rs`): it keeps the newest `cap` bytes (256 KiB by default),
 * `consoleWritten()` counts every byte ever written, and `consoleSince`
 * slices bytes from an absolute offset (the whole ring if it is older)
 * before decoding.
 */
function fakeHandle(accept: (bytes: Uint8Array) => number = (b) => b.length, cap = 256 * 1024) {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  let ring = new Uint8Array(0);
  let written = 0;
  let steps = 0;
  let softResets = 0;
  const typed: string[] = [];
  const handle = {
    serialInput(bytes: Uint8Array) {
      const n = accept(bytes);
      typed.push(decoder.decode(bytes.subarray(0, n)));
      return n;
    },
    serialPending: () => 0,
    consoleOutput: () => decoder.decode(ring),
    consoleWritten: () => written,
    consoleSince: (at: number) =>
      at >= written ? "" : decoder.decode(ring.subarray(Math.max(0, ring.length - (written - at)))),
    softResetCount: () => softResets,
    totalSteps: () => steps,
  } as unknown as FirmwareEmulatorHandle;
  return {
    handle,
    typed,
    print: (s: string) => {
      const bytes = encoder.encode(s);
      const next = new Uint8Array(ring.length + bytes.length);
      next.set(ring);
      next.set(bytes, ring.length);
      ring = next.subarray(Math.max(0, next.length - cap));
      written += bytes.length;
    },
    advance: (n: number) => {
      steps += n;
    },
    reboot: () => {
      softResets++;
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

const PROV_OK_LINE = "PROV OK id=test-fake-badge-0001 mac=02:00:00:00:10:01";

/**
 * Drives a fake through `put` and `prov apply` to a complete `PROV OK`
 * line (no prompt after it yet). With `expectTyping`, checks the state is
 * still `typing` (the token phase is pending).
 */
function toProvOk(options?: ProvisionOptions, expectTyping = true) {
  const f = fakeHandle();
  const p = createProvisioner(f.handle, "{}", () => 0, options);
  f.print("badge> ");
  p.poll();
  f.print("READY\r\n");
  p.poll();
  f.print("OK 2\r\nbadge> ");
  p.poll();
  f.print("prov apply\r\n");
  f.print(`${PROV_OK_LINE}\r\n`);
  if (expectTyping) expect(p.poll().kind).toBe("typing");
  return { f, p };
}

/** [`toProvOk`], then the prompt and `BADGE_TOKEN READY`, so the token is sent. */
function toTokenReady() {
  const { f, p } = toProvOk();
  f.print("badge> ");
  p.poll();
  f.print("badge_token set\r\nBADGE_TOKEN READY\r\n");
  p.poll();
  expect(f.typed[4]).toBe(FAKE_UPLOAD_TOKEN);
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

  // docs/milestone-11-decisions.md, "Traces": the two provisioning hosts
  // pace differently but legally. This pins the frontend's chunk schedule
  // against the Rust host's (tests/common/provision.rs), read from its
  // source, so a change to either side fails here.
  test("provisioner_pacing_matches_the_rust_host", () => {
    const rust = readFileSync(
      new URL("../../emulator-core/tests/common/provision.rs", import.meta.url),
      "utf8",
    );
    const rustGap = Number(
      /pub const PUT_CHUNK_GAP_STEPS: u32 = ([\d_]+);/.exec(rust)?.[1]?.replaceAll("_", ""),
    );
    const rustChunk = Number(/payload\.chunks\((\d+)\)/.exec(rust)?.[1]);
    expect(rustChunk).toBe(64);
    expect(rustGap).toBe(48_000);
    expect(PUT_CHUNK_BYTES).toBe(rustChunk);
    expect(PUT_CHUNK_GAP_STEPS).toBe(rustGap);

    const full = "x".repeat(rustChunk);
    const { f, p } = toFirstChunk(full + full + "x".repeat(8));
    expect(f.typed.slice(1)).toEqual([full]);
    // One step short of the Rust host's gap: nothing goes out.
    f.advance(rustGap - 1);
    p.poll();
    expect(f.typed.slice(1)).toEqual([full]);
    // At the gap, the next chunk goes out.
    f.advance(1);
    p.poll();
    expect(f.typed.slice(1)).toEqual([full, full]);
    f.advance(rustGap);
    p.poll();
    expect(f.typed.slice(1)).toEqual([full, full, "x".repeat(8)]);
  });

  test("waits for OK and the prompt, then types prov apply and waits for PROV OK", () => {
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
    // By default the token phase follows, so `ok` waits for it.
    expect(p.poll().kind).toBe("typing");
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

  // Review Focus 1: the console keeps the old boot's text across a soft
  // reset, so its idle `badge> ` must not be taken for the new REPL's.
  test("provisioner_ignores_a_prompt_from_before_the_provision", () => {
    const f = fakeHandle();
    f.print("I (10) app: started\r\nbadge> ");
    // The firmware reboots here: the new boot's console starts at this offset.
    let bootStart = f.handle.consoleWritten();
    const p = createProvisioner(f.handle, "{}", () => bootStart);
    expect(p.poll().kind).toBe("waiting");
    f.advance(1_000_000);
    // The new boot prints, but has not prompted yet.
    f.print("I (5) boot: ESP-IDF\r\n");
    expect(p.poll().kind).toBe("waiting");
    expect(f.typed).toEqual([]);
    f.print("badge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
    // A prompt is taken once: a later reboot does not retype the command.
    bootStart = f.handle.consoleWritten();
    f.print("badge> ");
    p.poll();
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
  });

  // ------------------------------------------------------------------
  // Milestone 12: the fake upload credential (`badge_token set`), typed
  // after `PROV OK` and its prompt. The fake console answers as v392 does
  // (docs/milestone-12-decisions.md, "Traces"): the REPL echoes the
  // command, prints `BADGE_TOKEN READY`, reads 64 bytes (not echoed),
  // then prints `BADGE_TOKEN OK` and a prompt.

  test("the fake upload token is 64 lowercase hex characters", () => {
    expect(FAKE_UPLOAD_TOKEN).toMatch(/^[0-9a-f]{64}$/);
    expect(BADGE_TOKEN_READY).toBe("BADGE_TOKEN READY");
    expect(BADGE_TOKEN_OK).toBe("BADGE_TOKEN OK");
    expect(BADGE_TOKEN_FAIL).toBe("BADGE_TOKEN FAIL");
  });

  test("provisioner_sets_the_token_after_prov_ok", () => {
    const { f, p } = toProvOk();
    // The PROV OK line alone does not start the token: its prompt follows.
    expect(f.typed.length).toBe(3);
    f.print("name=Test Hacker role=Hacker attendee=1 tutorial=reset\r\nbadge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed[3]).toBe("badge_token set\n");
    f.print("badge_token set\r\n");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed.length).toBe(4);
    f.print("BADGE_TOKEN READY\r\n");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed[4]).toBe(FAKE_UPLOAD_TOKEN);
    expect(new TextEncoder().encode(f.typed[4]).length).toBe(64);
    expect(f.typed.length).toBe(5);
    f.print("BADGE_TOKEN OK\r\n");
    // OK alone is not enough: the REPL prompts again after it.
    expect(p.poll().kind).toBe("typing");
    f.print("badge> ");
    expect(p.poll()).toEqual({ kind: "ok", line: PROV_OK_LINE });
    expect(p.poll()).toEqual({ kind: "ok", line: PROV_OK_LINE });
    expect(f.typed.length).toBe(5);
  });

  test("provisioner_fails_on_badge_token_fail", () => {
    const { f, p } = toTokenReady();
    f.print("BADGE_TOKEN FAIL\r\n");
    expect(p.poll()).toEqual({ kind: "failed", reason: "BADGE_TOKEN FAIL" });
    f.print("badge> ");
    expect(p.poll()).toEqual({ kind: "failed", reason: "BADGE_TOKEN FAIL" });
    expect(f.typed.length).toBe(5);
  });

  test("provisioner_fails_on_badge_token_fail_before_ready", () => {
    const { f, p } = toProvOk();
    f.print("badge> ");
    p.poll();
    f.print("badge_token set\r\nBADGE_TOKEN FAIL\r\n");
    expect(p.poll()).toEqual({ kind: "failed", reason: "BADGE_TOKEN FAIL" });
    // The token is never sent.
    expect(f.typed.length).toBe(4);
  });

  test("provisioner_fails_on_badge_token_usage", () => {
    const { f, p } = toProvOk();
    f.print("badge> ");
    p.poll();
    f.print("badge_token set\r\nusage: badge_token <set|get>");
    // The line is reported only once it is complete.
    expect(p.poll().kind).toBe("typing");
    f.print("\r\nbadge> ");
    expect(p.poll()).toEqual({ kind: "failed", reason: "usage: badge_token <set|get>" });
    expect(f.typed.length).toBe(4);
  });

  test("provisioner_skips_the_token_when_asked", () => {
    const { f, p } = toProvOk({ uploadToken: null }, false);
    expect(p.poll()).toEqual({ kind: "ok", line: PROV_OK_LINE });
    f.print("badge> ");
    expect(p.poll()).toEqual({ kind: "ok", line: PROV_OK_LINE });
    expect(f.typed.length).toBe(3);
    expect(f.typed.some((t) => t.includes("badge_token"))).toBe(false);
  });

  test("provisioner_sends_a_given_upload_token", () => {
    const token = "f".repeat(64);
    const { f, p } = toProvOk({ uploadToken: token });
    f.print("badge> ");
    p.poll();
    f.print("badge_token set\r\nBADGE_TOKEN READY\r\n");
    p.poll();
    expect(f.typed[4]).toBe(token);
  });

  // Review Focus 1, re-provisioning. The probe (decisions file, "Traces")
  // found that `prov apply` on a registered badge does not reboot: its
  // REPL prints `PROV OK`, the identity line and a prompt in the same
  // boot. The old prompts (the launcher's idle one, `put`'s) are already
  // in the console and must not start the token phase.
  test("provisioner_sets_the_token_after_prov_ok_without_a_reboot", () => {
    const f = fakeHandle();
    // A registered badge idle at its prompt, from an earlier provision.
    f.print("PROV OK id=test-fake-badge-0001\r\nbadge> badge_token set\r\n");
    f.print("BADGE_TOKEN READY\r\nBADGE_TOKEN OK\r\nbadge> ");
    const p = createProvisioner(f.handle, "{}", () => 0);
    expect(p.poll().kind).toBe("typing");
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
    f.print("put /littlefs/identity.json 2\r\nREADY\r\n");
    p.poll();
    f.print("OK 2\r\nbadge> ");
    p.poll();
    expect(f.typed[2]).toBe("prov apply\n");
    f.print("prov apply\r\nI (94586) hal_identity: identity: Test Organizer\r\n");
    expect(p.poll().kind).toBe("typing");
    f.print("PROV OK id=test-fake-badge-0002 mac=02:00:00:00:10:02\r\n");
    // Neither the old boot's text nor put's prompt counts.
    expect(p.poll().kind).toBe("typing");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed.length).toBe(3);
    f.print("name=Test Organizer role=Organizer attendee=2 tutorial=reset\r\nbadge> ");
    p.poll();
    expect(f.typed[3]).toBe("badge_token set\n");
    // The old `BADGE_TOKEN READY`/`OK` lines do not count either.
    expect(p.poll().kind).toBe("typing");
    expect(f.typed.length).toBe(4);
    f.print("badge_token set\r\nBADGE_TOKEN READY\r\n");
    p.poll();
    expect(f.typed[4]).toBe(FAKE_UPLOAD_TOKEN);
    expect(p.poll().kind).toBe("typing");
    f.print("BADGE_TOKEN OK\r\nbadge> ");
    expect(p.poll()).toEqual({
      kind: "ok",
      line: "PROV OK id=test-fake-badge-0002 mac=02:00:00:00:10:02",
    });
  });

  test("provisioner_waits_for_the_current_boots_prompt_after_prov_ok", () => {
    // Not seen on v392 (the probe found no reboot), but the token phase
    // follows the first prompt's rule (R-T13-3): a prompt counts only at
    // or after the current boot's console start.
    const f = fakeHandle();
    let bootStart = 0;
    const p = createProvisioner(f.handle, "{}", () => bootStart);
    f.print("badge> ");
    p.poll();
    f.print("READY\r\n");
    p.poll();
    f.print("OK 2\r\nbadge> ");
    p.poll();
    f.print(`${PROV_OK_LINE}\r\n`);
    // The old boot prompts, then the firmware reboots in the same frame;
    // the runtime records the new boot's start after both.
    f.print("badge> ");
    bootStart = f.handle.consoleWritten();
    f.print("I (5) boot: ESP-IDF\r\n");
    expect(p.poll().kind).toBe("typing");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed.length).toBe(3);
    f.print("badge> ");
    p.poll();
    expect(f.typed[3]).toBe("badge_token set\n");
  });

  test("provisioner_never_types_a_token_into_a_prompt_before_prov_ok", () => {
    const { f, p } = toFirstChunk("{}");
    f.print("OK 2\r\nbadge> ");
    p.poll();
    expect(f.typed[2]).toBe("prov apply\n");
    // A stray prompt after `prov apply` but before its result.
    f.print("prov apply\r\nbadge> ");
    expect(p.poll().kind).toBe("typing");
    f.print(`${PROV_OK_LINE}\r\n`);
    expect(p.poll().kind).toBe("typing");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed.length).toBe(3);
    f.print("badge> ");
    p.poll();
    expect(f.typed[3]).toBe("badge_token set\n");
  });

  // ------------------------------------------------------------------
  // Milestone 13 Task 3: absolute console offsets, and a reboot mid-provision.

  test("consoleSince is exact past a wrap", () => {
    const f = fakeHandle(undefined, 64);
    const text = Array.from({ length: 200 }, (_, i) => String.fromCharCode(97 + (i % 26))).join("");
    f.print(text);
    expect(f.handle.consoleWritten()).toBe(200);
    expect(f.handle.consoleOutput()).toBe(text.slice(-64));
    expect(consoleSince(f.handle, 190)).toBe(text.slice(190));
    // Older than the ring: the whole ring; at or past the end: nothing.
    expect(consoleSince(f.handle, 0)).toBe(text.slice(-64));
    expect(consoleSince(f.handle, 200)).toBe("");
  });

  // R-T13-3 past a wrap (Review Focus 5): the 256 KiB ring is full, so an
  // index into the ring's text is no longer an absolute offset.
  test("finds the first prompt in a full console after a soft reset", () => {
    const f = fakeHandle();
    f.print("I (1) app: idle\r\n".repeat(17_000));
    f.print("x".repeat(300_000 - 7 - f.handle.consoleWritten()));
    // The old boot's idle prompt, the last thing before the reset.
    f.print("badge> ");
    expect(f.handle.consoleWritten()).toBe(300_000);
    expect(f.handle.consoleOutput().length).toBe(256 * 1024);
    // The badge reboots at absolute 300,000.
    const bootStart = f.handle.consoleWritten();
    const p = createProvisioner(f.handle, "{}", () => bootStart);
    expect(p.poll().kind).toBe("waiting");
    f.print("I (5) boot: ESP-IDF\r\n");
    expect(p.poll().kind).toBe("waiting");
    f.print("badge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
    // The rest of the flow reads through absolute markers too.
    f.print("put /littlefs/identity.json 2\r\nREADY\r\n");
    p.poll();
    expect(f.typed[1]).toBe("{}");
    f.print("OK 2\r\nbadge> ");
    p.poll();
    expect(f.typed[2]).toBe("prov apply\n");
    f.print(`prov apply\r\n${PROV_OK_LINE}\r\nbadge> `);
    p.poll();
    p.poll();
    expect(f.typed[3]).toBe("badge_token set\n");
    f.print("badge_token set\r\nBADGE_TOKEN READY\r\n");
    p.poll();
    expect(f.typed[4]).toBe(FAKE_UPLOAD_TOKEN);
    f.print("BADGE_TOKEN OK\r\nbadge> ");
    expect(p.poll()).toEqual({ kind: "ok", line: PROV_OK_LINE });
  });

  // consoleOffsetOf: the absolute byte offset of text[i], counted back
  // from consoleWritten() by the UTF-8 length of the tail.
  test("consoleOffsetOf counts multi-byte text after i in bytes", () => {
    const f = fakeHandle();
    f.print("ab—cd"); // a b e2 80 94 c d: 7 bytes, 5 chars
    const text = consoleSince(f.handle, 0);
    expect(consoleOffsetOf(f.handle, text, 1)).toBe(1); // "b"
    expect(consoleOffsetOf(f.handle, text, 3)).toBe(5); // "c"
    expect(consoleOffsetOf(f.handle, text, text.length)).toBe(7);
  });

  test("consoleOffsetOf is exact when the slice is the whole ring", () => {
    const f = fakeHandle(undefined, 10);
    f.print("0123456789—z"); // 14 bytes; the ring keeps bytes 4..13
    const text = consoleSince(f.handle, 0); // older than the ring
    expect(text).toBe("456789—z");
    // "6" is at absolute 6, not at 0 + its index in the text.
    expect(consoleOffsetOf(f.handle, text, text.indexOf("6"))).toBe(6);
    expect(consoleOffsetOf(f.handle, text, text.indexOf("z"))).toBe(13);
  });

  test("consoleOffsetOf is exact when the slice starts mid-character", () => {
    const f = fakeHandle();
    f.print("a—b"); // a e2 80 94 b
    const text = consoleSince(f.handle, 2); // 80 94 b: two U+FFFD, then b
    expect(text.startsWith("�")).toBe(true);
    expect(consoleOffsetOf(f.handle, text, text.indexOf("b"))).toBe(4);
  });

  // v392 prints non-ASCII (an em dash): markers are byte offsets. Here a
  // char-index provOkEnd would land 80 bytes early, before a stale prompt.
  test("provOkEnd is a byte offset past multi-byte text", () => {
    const { f, p } = toFirstChunk("{}");
    f.print("OK 2\r\nbadge> ");
    p.poll();
    expect(f.typed[2]).toBe("prov apply\n");
    f.print(`prov apply\r\n${"—".repeat(40)}\r\nbadge> \r\n${PROV_OK_LINE}\r\n`);
    expect(p.poll().kind).toBe("typing");
    expect(p.poll().kind).toBe("typing");
    // The stray prompt before the PROV OK line does not start the token.
    expect(f.typed.length).toBe(3);
    f.print("badge> ");
    p.poll();
    expect(f.typed[3]).toBe("badge_token set\n");
  });

  // The same for the marker past `BADGE_TOKEN READY`: a char-index advance
  // would land early enough to re-see a stale OK and prompt before READY.
  test("the token marker is a byte offset past multi-byte text", () => {
    const { f, p } = toProvOk();
    f.print("badge> ");
    p.poll();
    expect(f.typed[3]).toBe("badge_token set\n");
    f.print(`badge_token set\r\n${"—".repeat(40)}\r\nBADGE_TOKEN OK\r\nbadge> \r\nBADGE_TOKEN READY\r\n`);
    p.poll();
    expect(f.typed[4]).toBe(FAKE_UPLOAD_TOKEN);
    // Only an OK and a prompt after READY finish it.
    expect(p.poll().kind).toBe("typing");
    f.print("BADGE_TOKEN OK\r\nbadge> ");
    expect(p.poll()).toEqual({ kind: "ok", line: PROV_OK_LINE });
  });

  test("fails when the badge reboots mid-provision", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, "{}");
    f.print("badge> ");
    p.poll();
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
    f.reboot();
    const failed = { kind: "failed", reason: "the badge rebooted during provisioning" } as const;
    expect(p.poll()).toEqual(failed);
    // The new boot's REPL is never typed into.
    f.print("READY\r\nOK 2\r\nbadge> ");
    expect(p.poll()).toEqual(failed);
    f.advance(10 * PUT_CHUNK_GAP_STEPS);
    expect(p.poll()).toEqual(failed);
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
  });

  test("a reboot during the token phase also fails the provision", () => {
    const { f, p } = toProvOk();
    f.reboot();
    f.print("badge> ");
    expect(p.poll()).toEqual({ kind: "failed", reason: "the badge rebooted during provisioning" });
    expect(f.typed.length).toBe(3);
  });

  test("a reboot while it waits for the first prompt is fine", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, "{}");
    f.reboot();
    expect(p.poll().kind).toBe("waiting");
    f.print("badge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed).toEqual(["put /littlefs/identity.json 2\n"]);
  });
});

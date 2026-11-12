import { describe, expect, test } from "bun:test";
import type { FirmwareEmulatorHandle } from "../src/cpu/bridge";
import {
  createSetupSkipper,
  LAUNCHED,
  REBOOT_LINE,
  SETUP_LAUNCHED,
  setupOnScreen,
  SKIP_SETUP_CONFIG,
  SKIP_SETUP_CONFIG_OK,
  SKIP_SETUP_TIMEOUT_STEPS,
} from "../src/runtime/setup-skip";

/**
 * A fake console, as in provisioner.test.ts: records each `serialInput`
 * call as a string; `print` appends firmware output; `reboot` bumps
 * `softResetCount()`. The console mirrors the core's byte ring
 * (`console.rs`): it keeps the newest `cap` bytes, `consoleWritten()`
 * counts every byte ever written, and `consoleSince` slices bytes from an
 * absolute offset (the whole ring if it is older) before decoding.
 */
function fakeHandle(accept: (bytes: Uint8Array) => number = (b) => b.length, cap = 256 * 1024) {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  let ring = new Uint8Array(0);
  let written = 0;
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
    totalSteps: () => 0,
  } as unknown as FirmwareEmulatorHandle;
  return {
    handle,
    typed,
    written: () => written,
    print: (s: string) => {
      const bytes = encoder.encode(s);
      const next = new Uint8Array(ring.length + bytes.length);
      next.set(ring);
      next.set(bytes, ring.length);
      ring = next.subarray(Math.max(0, next.length - cap));
      written += bytes.length;
    },
    reboot: () => {
      softResets++;
    },
  };
}

const CONFIG_TYPED = "config system onboard_build 2147483647\n";
const REBOOT_TYPED = "reboot\n";

describe("setup-skip constants", () => {
  test("are the spec's lines and needles, verbatim", () => {
    expect(SKIP_SETUP_CONFIG).toBe("config system onboard_build 2147483647");
    expect(SKIP_SETUP_CONFIG_OK).toBe("OK system.onboard_build=2147483647");
    expect(REBOOT_LINE).toBe("reboot");
    expect(SETUP_LAUNCHED).toBe("app_reg: launched Setup");
    expect(LAUNCHED).toBe("app_reg: launched ");
    expect(SKIP_SETUP_TIMEOUT_STEPS).toBe(60_000_000);
  });
});

describe("setupOnScreen", () => {
  test("setupOnScreen follows the last launch line of the current boot", () => {
    expect(setupOnScreen("")).toBe(false);
    expect(setupOnScreen("I (10) app_reg: launched My Badge\r\nI (20) app_reg: launched Setup\r\n")).toBe(true);
    expect(setupOnScreen("I (10) app_reg: launched Setup\r\nI (20) app_reg: launched Launcher\r\n")).toBe(false);
    // Re-provisioning from the launcher: Setup launches again.
    expect(setupOnScreen("I (10) app_reg: launched Launcher\r\nI (20) app_reg: launched Setup\r\n")).toBe(true);
    // An unterminated line doesn't count, either way.
    expect(setupOnScreen("I (10) app_reg: launched Se")).toBe(false);
    expect(setupOnScreen("I (10) app_reg: launched Setup\r\nI (20) app_reg: launched Laun")).toBe(true);
    expect(setupOnScreen("I (10) app_reg: launched Launcher\r\nI (20) app_reg: launched Se")).toBe(false);
  });

  test("names Setup exactly, not a longer name", () => {
    expect(setupOnScreen("I (10) app_reg: launched Setup2\r\n")).toBe(false);
    expect(setupOnScreen("I (10) app_reg: launched Setup\n")).toBe(true);
  });

  test("ignores other lines after the launch", () => {
    expect(setupOnScreen("I (10) app_reg: launched Setup\r\nI (11) other: x\r\nbadge> ")).toBe(true);
  });
});

describe("setup skipper", () => {
  test("types config, then reboot, and is done at the reset", () => {
    const f = fakeHandle();
    const s = createSetupSkipper(f.handle, () => 0);
    expect(s.poll()).toEqual({ kind: "typing" });
    expect(f.typed).toEqual([]);
    f.print("I (20) app_reg: launched Setup\r\nbadge> ");
    expect(s.poll()).toEqual({ kind: "typing" });
    expect(f.typed).toEqual([CONFIG_TYPED]);
    // The echo, and the old prompt, are not the reply.
    f.print("config system onboard_build 2147483647\r\n");
    expect(s.poll()).toEqual({ kind: "typing" });
    f.print("OK system.onboard_build=2147483647\r\n");
    expect(s.poll()).toEqual({ kind: "typing" });
    expect(f.typed).toEqual([CONFIG_TYPED]);
    f.print("badge> ");
    expect(s.poll()).toEqual({ kind: "typing" });
    expect(f.typed).toEqual([CONFIG_TYPED, REBOOT_TYPED]);
    f.print("reboot\r\nrebooting...\r\n");
    expect(s.poll()).toEqual({ kind: "typing" });
    f.reboot();
    expect(s.poll()).toEqual({ kind: "done" });
    // Nothing more is typed after done, whatever the new boot prints.
    f.print("new boot\r\nbadge> OK system.onboard_build=2147483647\r\nbadge> ");
    expect(s.poll()).toEqual({ kind: "done" });
    expect(f.typed).toEqual([CONFIG_TYPED, REBOOT_TYPED]);
  });

  test("a prompt before the OK line is not the config's prompt", () => {
    const f = fakeHandle();
    const s = createSetupSkipper(f.handle, () => 0);
    f.print("badge> ");
    s.poll();
    f.print("badge> \r\n");
    s.poll();
    expect(f.typed).toEqual([CONFIG_TYPED]);
  });

  test("fails on a usage reply", () => {
    const f = fakeHandle();
    const s = createSetupSkipper(f.handle, () => 0);
    f.print("badge> ");
    s.poll();
    f.print("usage: config <slug> <key>");
    expect(s.poll()).toEqual({ kind: "typing" });
    f.print(" <value>\r\nbadge> ");
    expect(s.poll()).toEqual({ kind: "failed", reason: "usage: config <slug> <key> <value>" });
    f.print("OK system.onboard_build=2147483647\r\nbadge> ");
    expect(s.poll()).toEqual({ kind: "failed", reason: "usage: config <slug> <key> <value>" });
    f.reboot();
    s.poll();
    expect(f.typed).toEqual([CONFIG_TYPED]);
  });

  test("waits for a prompt in the current boot", () => {
    const f = fakeHandle();
    f.print("old boot\r\nbadge> ");
    const bootStart = f.written();
    const s = createSetupSkipper(f.handle, () => bootStart);
    expect(s.poll()).toEqual({ kind: "typing" });
    expect(f.typed).toEqual([]);
    f.print("new boot\r\nbadge> ");
    expect(s.poll()).toEqual({ kind: "typing" });
    expect(f.typed).toEqual([CONFIG_TYPED]);
  });

  // Review Focus 5: absolute offsets past the ring's wrap.
  test("finds the prompt and the reply in a wrapped console", () => {
    const f = fakeHandle(undefined, 64);
    f.print(`${"x".repeat(500)}\r\nold\r\nbadge> `);
    const bootStart = f.written();
    const s = createSetupSkipper(f.handle, () => bootStart);
    s.poll();
    expect(f.typed).toEqual([]);
    f.print(`${"é".repeat(200)}\r\nbadge> `);
    s.poll();
    expect(f.typed).toEqual([CONFIG_TYPED]);
    f.print(`${"y".repeat(10)}\r\nOK system.onboard_build=2147483647\r\nbadge> `);
    s.poll();
    expect(f.typed).toEqual([CONFIG_TYPED, REBOOT_TYPED]);
  });

  test("a reboot before reboot is typed fails the skip, and nothing more is typed", () => {
    const f = fakeHandle();
    const s = createSetupSkipper(f.handle, () => 0);
    f.print("badge> ");
    s.poll();
    f.reboot();
    expect(s.poll()).toEqual({ kind: "failed", reason: "the badge rebooted during the skip" });
    f.print("OK system.onboard_build=2147483647\r\nbadge> ");
    s.poll();
    expect(f.typed).toEqual([CONFIG_TYPED]);
  });

  test("a refused config fails the skip", () => {
    const f = fakeHandle(() => 0);
    const s = createSetupSkipper(f.handle, () => 0);
    f.print("badge> ");
    expect(s.poll()).toEqual({ kind: "failed", reason: "the serial queue refused the input" });
    f.print("OK system.onboard_build=2147483647\r\nbadge> ");
    s.poll();
    expect(f.typed).toEqual([""]);
  });
});

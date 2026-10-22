import { describe, expect, test } from "bun:test";
import { BUTTON_NAMES } from "../src/badge/input";
import type { CpuRunSummary, FirmwareEmulatorHandle } from "../src/cpu/bridge";
import { blitFramebuffer } from "../src/render/framebuffer";
import {
  type FramePresenter,
  createFirmwareRuntime,
  DEFAULT_STEPS_PER_FRAME,
  frameTargetUs,
  MAX_FRAME_DELTA_MS,
  PRESS_HOLD_US,
  PROVISION_TIMEOUT_STEPS,
  RAW_SLOT_BY_BUTTON,
  SHAKE_HALF_US,
  shakeAcceleration,
  stepFirmwareFrame,
} from "../src/runtime/firmware-runtime";

/**
 * `bun test` has no global `ImageData` — same fact `test/framebuffer.test.ts`
 * documents and shims around. `stepFirmwareFrame`/`createFirmwareRuntime`
 * call `blitFramebuffer`, which constructs one, so this test file needs the
 * same minimal shim.
 */
class FakeImageData {
  data: Uint8ClampedArray;
  width: number;
  height: number;
  constructor(data: Uint8ClampedArray, width: number, height: number) {
    this.data = data;
    this.width = width;
    this.height = height;
  }
}
(globalThis as unknown as { ImageData: typeof FakeImageData }).ImageData = FakeImageData;

function makeStubCtx() {
  let putImageDataCalls = 0;
  const ctx = {
    putImageData: () => {
      putImageDataCalls++;
    },
  };
  const canvas = ctx as unknown as CanvasRenderingContext2D;
  // Milestone 7 Task 8: the runtime presents frames through a callback; the
  // in-thread presenter is the old path, a blit to the canvas.
  const present: FramePresenter = (fb, width, height) => blitFramebuffer(canvas, fb, width, height);
  return { ctx: canvas, present, getPutImageDataCalls: () => putImageDataCalls };
}

interface FakeHandleOptions extends Partial<FirmwareEmulatorHandle> {
  /**
   * Emulated µs one step advances the fake clock by. Default 1, so the
   * default 1,000,000-step cap never binds; 0.0125 is one step at 80 MHz.
   */
  usPerStep?: number;
}

/**
 * A hand-written fake `FirmwareEmulatorHandle` — no WASM involved, per
 * `bridge.ts`'s design intent. `runUntil(target, max)` advances a fake
 * emulated clock to `min(floor(target), now + max * usPerStep)`; the floor
 * mimics the wasm shim's `target_us as u64` truncation.
 */
function makeFakeHandle(options: FakeHandleOptions = {}) {
  const { usPerStep = 1, ...overrides } = options;
  let timeUs = 0;
  const runBudgets: number[] = [];
  const runUntilCalls: Array<[number, number]> = [];
  const buttonCalls: Array<[number, boolean]> = [];
  const counters = { resetCalls: 0, disposeCalls: 0 };
  const width = 4;
  const height = 4;

  const handle: FirmwareEmulatorHandle = {
    run(budget: number): CpuRunSummary {
      runBudgets.push(budget);
      return {
        steps: budget,
        traps: 0,
        romStubCalls: 0,
        pc: 0x4200_0000,
        lastInstructionFault: undefined,
      };
    },
    runUntil(targetUs: number, maxSteps: number): CpuRunSummary {
      runUntilCalls.push([targetUs, maxSteps]);
      const before = timeUs;
      timeUs = Math.max(before, Math.min(Math.floor(targetUs), before + maxSteps * usPerStep));
      return {
        steps: Math.round((timeUs - before) / usPerStep),
        traps: 0,
        romStubCalls: 0,
        pc: 0x4200_0000,
        lastInstructionFault: undefined,
      };
    },
    reset(): void {
      counters.resetCalls++;
    },
    setRawButton(slot: number, pressed: boolean): void {
      buttonCalls.push([slot, pressed]);
    },
    framebuffer(): Uint16Array {
      return new Uint16Array(width * height);
    },
    screenWidth: width,
    screenHeight: height,
    pc(): number {
      return 0x4200_0000;
    },
    totalSteps(): number {
      return 0;
    },
    emulatedTimeUs: () => timeUs,
    serialInput: () => 0,
    serialPending: () => 0,
    consoleOutput: () => "",
    setAcceleration: () => {},
    dispose(): void {
      counters.disposeCalls++;
    },
    ...overrides,
  };

  return { handle, runBudgets, runUntilCalls, buttonCalls, counters };
}

describe("RAW_SLOT_BY_BUTTON", () => {
  test("is a total mapping over BUTTON_NAMES with no duplicate slots", () => {
    const slots = BUTTON_NAMES.map((name) => RAW_SLOT_BY_BUTTON[name]);
    expect(slots).toHaveLength(BUTTON_NAMES.length);
    expect(new Set(slots).size).toBe(BUTTON_NAMES.length);
    for (const slot of slots) {
      expect(Number.isInteger(slot)).toBe(true);
      expect(slot).toBeGreaterThanOrEqual(0);
      expect(slot).toBeLessThan(BUTTON_NAMES.length);
    }
  });

  test("matches emulator-core's documented slot numbering exactly", () => {
    // slot 0 = PIN_START (direct GPIO9); slots 1..=8 = Hc165 index 0..=7,
    // whose slot->button table (emulator-core/src/peripherals/gpio.rs) is
    // A, B, HOME, DOWN, LEFT, RIGHT, UP, AUX1 — see
    // emulator-core/src/runtime.rs's module doc for the slot-0-is-START,
    // slot-n-is-Hc165-index-(n-1) composition.
    expect(RAW_SLOT_BY_BUTTON).toEqual({
      START: 0,
      A: 1,
      B: 2,
      HOME: 3,
      DOWN: 4,
      LEFT: 5,
      RIGHT: 6,
      UP: 7,
      AUX1: 8,
    });
  });
});

describe("DEFAULT_STEPS_PER_FRAME", () => {
  test("fits about 15 ms of work at the measured browser rate", () => {
    // About 10M steps/s in the browser (1M-step frames ran at 9.6 fps) and
    // 12.3M under Bun's JavaScriptCore: 150,000 steps is 12 to 15 ms.
    expect(DEFAULT_STEPS_PER_FRAME).toBe(150_000);
  });
});

describe("frameTargetUs", () => {
  test("advances emulated time by the wall delta", () => {
    expect(frameTargetUs(1_000, 16)).toBe(17_000);
  });
  test("frame target clamps the wall delta", () => {
    expect(frameTargetUs(0, 5_000)).toBe(MAX_FRAME_DELTA_MS * 1000);
    expect(frameTargetUs(500, -3)).toBe(500);
  });
});

describe("stepFirmwareFrame", () => {
  test("runs to the frame target and blits once", () => {
    const { handle, runUntilCalls } = makeFakeHandle();
    const { present, getPutImageDataCalls } = makeStubCtx();
    const r = stepFirmwareFrame(handle, present, 16, 1_000_000);
    expect(runUntilCalls).toEqual([[16_000, 1_000_000]]);
    expect(r).toEqual({ emulatedDeltaUs: 16_000, capped: false });
    expect(getPutImageDataCalls()).toBe(1);
  });
  test("a capped frame drops its debt", () => {
    const { handle, runUntilCalls } = makeFakeHandle({ usPerStep: 0.0125 });
    const { present } = makeStubCtx();
    // 100 steps at 80 MHz = 1.25 µs of the 16,000 µs asked for.
    const r = stepFirmwareFrame(handle, present, 16, 100);
    expect(r.capped).toBe(true);
    // The next frame targets now + 16 ms, not the missed time.
    stepFirmwareFrame(handle, present, 16, 100);
    expect(handle.emulatedTimeUs()).toBeCloseTo(2.5, 5);
    // Debt carried over would target 16,000 + 16,000 or the old target + 16 ms.
    expect(runUntilCalls).toHaveLength(2);
    expect(runUntilCalls[0]![0]).toBeCloseTo(16_000, 5);
    expect(runUntilCalls[1]![0]).toBeCloseTo(1.25 + 16_000, 5);
  });
  test("a fractional target the emulator truncates to whole µs is not capped", () => {
    const { handle } = makeFakeHandle();
    const { present } = makeStubCtx();
    // A 60 Hz rAF delta (16.667 ms): target 16,666.67 µs, run to 16,666.
    const r = stepFirmwareFrame(handle, present, 1000 / 60, 1_000_000);
    expect(handle.emulatedTimeUs()).toBe(16_666);
    expect(r.capped).toBe(false);
  });
});

describe("createFirmwareRuntime", () => {
  test("start() schedules frames via the injected scheduler, capped at DEFAULT_STEPS_PER_FRAME by default", () => {
    const { handle, runUntilCalls } = makeFakeHandle();
    const { present } = makeStubCtx();
    const scheduledCbs: Array<(t: number) => void> = [];
    const runtime = createFirmwareRuntime(handle, present, {
      scheduleFrame: (cb) => {
        scheduledCbs.push(cb);
        return scheduledCbs.length;
      },
      cancelFrame: () => {},
    });

    runtime.start();
    expect(scheduledCbs).toHaveLength(1);

    // Fire the scheduled frame manually (simulating one rAF tick).
    scheduledCbs[0](0);
    expect(runUntilCalls).toEqual([[0, DEFAULT_STEPS_PER_FRAME]]);
    // The loop reschedules itself for the next frame.
    expect(scheduledCbs).toHaveLength(2);
  });

  test("start() is idempotent while already running", () => {
    const { handle } = makeFakeHandle();
    const { present } = makeStubCtx();
    let scheduleCalls = 0;
    const runtime = createFirmwareRuntime(handle, present, {
      scheduleFrame: () => {
        scheduleCalls++;
        return scheduleCalls;
      },
      cancelFrame: () => {},
    });

    runtime.start();
    runtime.start();
    expect(scheduleCalls).toBe(1);
  });

  test("stop() cancels the pending frame and further callbacks do not reschedule", () => {
    const { handle, runUntilCalls } = makeFakeHandle();
    const { present } = makeStubCtx();
    const scheduledCbs: Array<(t: number) => void> = [];
    const canceled: number[] = [];
    const runtime = createFirmwareRuntime(handle, present, {
      scheduleFrame: (cb) => {
        scheduledCbs.push(cb);
        return scheduledCbs.length;
      },
      cancelFrame: (id) => canceled.push(id),
    });

    runtime.start();
    runtime.stop();
    expect(canceled).toEqual([1]);

    // A late-firing callback (as if the browser had already queued it before
    // cancellation raced it) must not step the CPU or reschedule.
    scheduledCbs[0](0);
    expect(runUntilCalls).toEqual([]);
    expect(scheduledCbs).toHaveLength(1);
  });

  test("injectButton maps a button name to its raw slot and press/release state", () => {
    let timeUsNow = 0;
    const { handle, buttonCalls } = makeFakeHandle({ emulatedTimeUs: () => timeUsNow });
    const { present } = makeStubCtx();
    const runtime = createFirmwareRuntime(handle, present, { scheduleFrame: () => 1, cancelFrame: () => {} });

    runtime.injectButton("UP", "pressed");
    timeUsNow = PRESS_HOLD_US;
    runtime.injectButton("UP", "released");
    runtime.injectButton("AUX1", "pressed");

    expect(buttonCalls).toEqual([
      [RAW_SLOT_BY_BUTTON.UP, true],
      [RAW_SLOT_BY_BUTTON.UP, false],
      [RAW_SLOT_BY_BUTTON.AUX1, true],
    ]);
  });

  test("reset() delegates to the handle", () => {
    const fake = makeFakeHandle();
    const { present } = makeStubCtx();
    const runtime = createFirmwareRuntime(fake.handle, present, { scheduleFrame: () => 1, cancelFrame: () => {} });
    runtime.reset();
    expect(fake.counters.resetCalls).toBe(1);
  });

  test("isExitRequested is always false (no Lua-style exit-to-launcher in real-firmware mode)", () => {
    const { handle } = makeFakeHandle();
    const { present } = makeStubCtx();
    const runtime = createFirmwareRuntime(handle, present, { scheduleFrame: () => 1, cancelFrame: () => {} });
    expect(runtime.isExitRequested()).toBe(false);
  });

  test("dispose() stops the loop and releases the handle exactly once, and forbids restart", () => {
    const fake = makeFakeHandle();
    const { present } = makeStubCtx();
    let canceled = 0;
    const runtime = createFirmwareRuntime(fake.handle, present, {
      scheduleFrame: () => 1,
      cancelFrame: () => {
        canceled++;
      },
    });

    runtime.start();
    runtime.dispose();
    runtime.dispose(); // idempotent

    expect(canceled).toBe(1);
    expect(fake.counters.disposeCalls).toBe(1);
    expect(() => runtime.start()).toThrow();
  });
});

/**
 * A fake console handle, a settable step counter and emulated time, and a
 * manual frame scheduler.
 */
function rig() {
  let consoleText = "";
  let steps = 0;
  let timeUs = 0;
  const typed: string[] = [];
  const accel: Array<[number, number, number]> = [];
  const fake = makeFakeHandle({
    serialInput(bytes: Uint8Array): number {
      typed.push(new TextDecoder().decode(bytes));
      return bytes.length;
    },
    serialPending: () => 0,
    consoleOutput: () => consoleText,
    totalSteps: () => steps,
    emulatedTimeUs: () => timeUs,
    setAcceleration: (x, y, z) => {
      accel.push([x, y, z]);
    },
  });
  const queued: Array<(t: number) => void> = [];
  const rt = createFirmwareRuntime(fake.handle, makeStubCtx().present, {
    scheduleFrame: (cb) => queued.push(cb),
    cancelFrame: () => {},
  });
  const frame = () => queued.shift()?.(0);
  return {
    rt,
    typed,
    accel,
    frame,
    buttonCalls: fake.buttonCalls,
    print: (s: string) => (consoleText += s),
    setSteps: (n: number) => (steps = n),
    setTimeUs: (n: number) => (timeUs = n),
  };
}

/**
 * Like {@link rig}, but each queued frame callback is invoked with an
 * explicit `requestAnimationFrame` time, and the fake clock advances through
 * `runUntil` (see {@link makeFakeHandle}).
 */
function pacedRig(opts: { usPerStep?: number; stepsPerFrame?: number } = {}) {
  const fake = makeFakeHandle({ usPerStep: opts.usPerStep });
  const queued: Array<(t: number) => void> = [];
  const rt = createFirmwareRuntime(fake.handle, makeStubCtx().present, {
    stepsPerFrame: opts.stepsPerFrame,
    scheduleFrame: (cb) => queued.push(cb),
    cancelFrame: () => {},
  });
  const frameAt = (t: number) => queued.shift()?.(t);
  return { rt, frameAt, fake: fake.handle, buttonCalls: fake.buttonCalls };
}

describe("FirmwareRuntime pacing", () => {
  test("the first frame runs nothing and later frames follow the wall clock", () => {
    const { rt, frameAt, fake } = pacedRig();
    rt.start();
    frameAt(1_000);
    expect(fake.emulatedTimeUs()).toBe(0);
    frameAt(1_016);
    expect(fake.emulatedTimeUs()).toBe(16_000);
  });
  test("speed() reports the emulated/wall ratio", () => {
    const { rt, frameAt } = pacedRig({ usPerStep: 0.0125, stepsPerFrame: 640_000 });
    rt.start();
    frameAt(0);
    for (let t = 16; t <= 16 * 60; t += 16) frameAt(t);
    // 640,000 steps x 0.0125 µs = 8,000 µs per 16 ms frame: 0.5x.
    expect(rt.speed()).toBeCloseTo(0.5, 1);
  });
  test("speed() divides by the real frame time, not the clamped one", () => {
    const { rt, frameAt } = pacedRig();
    rt.start();
    frameAt(0);
    // 100 ms frames: each target clamps to 50 ms, uncapped, so 0.5x.
    for (let t = 100; t <= 100 * 30; t += 100) frameAt(t);
    expect(rt.speed()).toBeCloseTo(0.5, 5);
  });
  test("speed() smooths a changing ratio", () => {
    const { rt, frameAt } = pacedRig();
    rt.start();
    frameAt(0);
    frameAt(16); // ratio 1: seeds speed
    expect(rt.speed()).toBeCloseTo(1, 5);
    frameAt(116); // ratio 0.5 (50 ms of a 100 ms frame)
    expect(rt.speed()).toBeCloseTo(0.9 * 1 + 0.1 * 0.5, 5);
  });
  test("stop() flushes deferred releases", () => {
    const { rt, buttonCalls, frameAt } = pacedRig();
    rt.start();
    frameAt(0);
    rt.injectButton("A", "pressed");
    rt.injectButton("A", "released");
    rt.stop();
    expect(buttonCalls.at(-1)).toEqual([RAW_SLOT_BY_BUTTON.A, false]);
    // The flushed release is not replayed after a restart.
    rt.start();
    for (let t = 1_000; t <= 1_000 + 4 * 50; t += 50) frameAt(t);
    const releasesOfA = buttonCalls.filter(([slot, p]) => slot === RAW_SLOT_BY_BUTTON.A && !p);
    expect(releasesOfA.length).toBe(1);
  });
});

describe("FirmwareRuntime provisioning", () => {
  test("provision() types once the prompt appears, polled by the frame loop", () => {
    const { rt, typed, frame, print } = rig();
    rt.provision('{"a":1}');
    rt.start();
    frame();
    expect(typed).toEqual([]);
    expect(rt.provisionState()?.kind).toBe("waiting");
    print("badge> ");
    frame();
    expect(typed).toEqual(["put /littlefs/identity.json 7\n"]);
  });

  test("reset abandons an in-flight provisioner", () => {
    const { rt, typed, frame, print } = rig();
    rt.provision('{"a":1}');
    rt.start();
    print("badge> ");
    frame();
    expect(typed.length).toBe(1);
    rt.reset();
    expect(rt.provisionState()).toBeNull();
    print("OK 7\r\nbadge> ");
    frame();
    frame();
    expect(typed.length).toBe(1);
  });

  test("a second provision() replaces the first", () => {
    const { rt, typed, frame, print } = rig();
    rt.provision('{"a":1}');
    rt.provision('{"b":22}');
    rt.start();
    print("badge> ");
    frame();
    expect(typed).toEqual(["put /littlefs/identity.json 8\n"]);
  });

  test("a provisioner that never finishes times out in emulated steps", () => {
    const { rt, frame, setSteps } = rig();
    setSteps(1000);
    rt.provision('{"a":1}');
    rt.start();
    setSteps(1000 + PROVISION_TIMEOUT_STEPS - 1);
    frame();
    expect(rt.provisionState()?.kind).toBe("waiting");
    setSteps(1000 + PROVISION_TIMEOUT_STEPS + 1);
    frame();
    expect(rt.provisionState()).toEqual({ kind: "failed", reason: "timed out" });
    frame();
    expect(rt.provisionState()).toEqual({ kind: "failed", reason: "timed out" });
  });
});

describe("shakeAcceleration", () => {
  test("alternates six half-periods then rests", () => {
    expect(SHAKE_HALF_US).toBe(62_500);
    expect(shakeAcceleration(0)).toEqual([2000, 2000, 1000]);
    expect(shakeAcceleration(SHAKE_HALF_US - 1)).toEqual([2000, 2000, 1000]);
    expect(shakeAcceleration(SHAKE_HALF_US)).toEqual([-2000, -2000, 1000]);
    expect(shakeAcceleration(5 * SHAKE_HALF_US)).toEqual([-2000, -2000, 1000]);
    expect(shakeAcceleration(6 * SHAKE_HALF_US)).toBeNull();
  });
});

describe("FirmwareRuntime shake", () => {
  test("is scheduled from the frame loop off emulated time, then rests", () => {
    const { rt, accel, frame, setTimeUs } = rig();
    setTimeUs(500);
    rt.start();
    rt.shake();
    frame();
    expect(accel).toEqual([[2000, 2000, 1000]]);
    frame(); // unchanged half-period: no repeated call
    expect(accel.length).toBe(1);
    setTimeUs(500 + SHAKE_HALF_US);
    frame();
    expect(accel.at(-1)).toEqual([-2000, -2000, 1000]);
    setTimeUs(500 + 6 * SHAKE_HALF_US);
    frame();
    expect(accel.at(-1)).toEqual([0, 0, 1000]);
    expect(accel.length).toBe(3);
    const n = accel.length;
    setTimeUs(500 + 20 * SHAKE_HALF_US);
    frame();
    expect(accel.length).toBe(n);
  });

  test("reset cancels a shake and restores rest acceleration", () => {
    const { rt, accel, frame, setTimeUs } = rig();
    rt.start();
    rt.shake();
    frame();
    rt.reset();
    expect(accel.at(-1)).toEqual([0, 0, 1000]);
    const n = accel.length;
    setTimeUs(SHAKE_HALF_US);
    frame();
    expect(accel.length).toBe(n);
  });
});

describe("FirmwareRuntime press hold", () => {
  test("a release before PRESS_HOLD_US is deferred to the frame loop", () => {
    const { rt, buttonCalls, frame, setTimeUs } = rig();
    rt.start();
    setTimeUs(100);
    rt.injectButton("A", "pressed");
    setTimeUs(200);
    rt.injectButton("A", "released");
    expect(buttonCalls).toEqual([[RAW_SLOT_BY_BUTTON.A, true]]);
    setTimeUs(100 + PRESS_HOLD_US - 1);
    frame();
    expect(buttonCalls.length).toBe(1);
    setTimeUs(100 + PRESS_HOLD_US);
    frame();
    expect(buttonCalls).toEqual([
      [RAW_SLOT_BY_BUTTON.A, true],
      [RAW_SLOT_BY_BUTTON.A, false],
    ]);
  });

  test("a release after the hold has elapsed is immediate", () => {
    const { rt, buttonCalls, setTimeUs } = rig();
    rt.injectButton("B", "pressed");
    setTimeUs(PRESS_HOLD_US);
    rt.injectButton("B", "released");
    expect(buttonCalls.at(-1)).toEqual([RAW_SLOT_BY_BUTTON.B, false]);
  });

  test("a re-press before the deferred release cancels it", () => {
    const { rt, buttonCalls, frame, setTimeUs } = rig();
    rt.start();
    rt.injectButton("A", "pressed");
    setTimeUs(10);
    rt.injectButton("A", "released");
    setTimeUs(20);
    rt.injectButton("A", "pressed");
    setTimeUs(20 + PRESS_HOLD_US + 5);
    frame();
    expect(buttonCalls.every(([, p]) => p)).toBe(true);
  });

  test("reset clears pending releases", () => {
    const { rt, buttonCalls, frame, setTimeUs } = rig();
    rt.start();
    rt.injectButton("A", "pressed");
    rt.injectButton("A", "released");
    rt.reset();
    const n = buttonCalls.length;
    setTimeUs(PRESS_HOLD_US * 2);
    frame();
    expect(buttonCalls.length).toBe(n);
  });
});

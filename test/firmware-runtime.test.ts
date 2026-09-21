import { describe, expect, test } from "bun:test";
import { BUTTON_NAMES } from "../src/badge/input";
import type { CpuRunSummary, FirmwareEmulatorHandle } from "../src/cpu/bridge";
import {
  createFirmwareRuntime,
  DEFAULT_CYCLES_PER_FRAME,
  RAW_SLOT_BY_BUTTON,
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
  return { ctx: ctx as unknown as CanvasRenderingContext2D, getPutImageDataCalls: () => putImageDataCalls };
}

/** A hand-written fake `FirmwareEmulatorHandle` — no WASM involved, per `bridge.ts`'s design intent. */
function makeFakeHandle(overrides: Partial<FirmwareEmulatorHandle> = {}) {
  const runBudgets: number[] = [];
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
    dispose(): void {
      counters.disposeCalls++;
    },
    ...overrides,
  };

  return { handle, runBudgets, buttonCalls, counters };
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

describe("stepFirmwareFrame", () => {
  test("runs exactly cyclesPerFrame steps and blits the resulting framebuffer", () => {
    const { handle, runBudgets } = makeFakeHandle();
    const { ctx, getPutImageDataCalls } = makeStubCtx();

    stepFirmwareFrame(handle, ctx, 12345);

    expect(runBudgets).toEqual([12345]);
    expect(getPutImageDataCalls()).toBe(1);
  });
});

describe("createFirmwareRuntime", () => {
  test("start() schedules frames via the injected scheduler, using DEFAULT_CYCLES_PER_FRAME by default", () => {
    const { handle, runBudgets } = makeFakeHandle();
    const { ctx } = makeStubCtx();
    const scheduledCbs: Array<(t: number) => void> = [];
    const runtime = createFirmwareRuntime(handle, ctx, {
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
    expect(runBudgets).toEqual([DEFAULT_CYCLES_PER_FRAME]);
    // The loop reschedules itself for the next frame.
    expect(scheduledCbs).toHaveLength(2);
  });

  test("start() is idempotent while already running", () => {
    const { handle } = makeFakeHandle();
    const { ctx } = makeStubCtx();
    let scheduleCalls = 0;
    const runtime = createFirmwareRuntime(handle, ctx, {
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
    const { handle, runBudgets } = makeFakeHandle();
    const { ctx } = makeStubCtx();
    const scheduledCbs: Array<(t: number) => void> = [];
    const canceled: number[] = [];
    const runtime = createFirmwareRuntime(handle, ctx, {
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
    expect(runBudgets).toEqual([]);
    expect(scheduledCbs).toHaveLength(1);
  });

  test("injectButton maps a button name to its raw slot and press/release state", () => {
    const { handle, buttonCalls } = makeFakeHandle();
    const { ctx } = makeStubCtx();
    const runtime = createFirmwareRuntime(handle, ctx, { scheduleFrame: () => 1, cancelFrame: () => {} });

    runtime.injectButton("UP", "pressed");
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
    const { ctx } = makeStubCtx();
    const runtime = createFirmwareRuntime(fake.handle, ctx, { scheduleFrame: () => 1, cancelFrame: () => {} });
    runtime.reset();
    expect(fake.counters.resetCalls).toBe(1);
  });

  test("isExitRequested is always false (no launcher concept in real-firmware mode)", () => {
    const { handle } = makeFakeHandle();
    const { ctx } = makeStubCtx();
    const runtime = createFirmwareRuntime(handle, ctx, { scheduleFrame: () => 1, cancelFrame: () => {} });
    expect(runtime.isExitRequested()).toBe(false);
  });

  test("dispose() stops the loop and releases the handle exactly once, and forbids restart", () => {
    const fake = makeFakeHandle();
    const { ctx } = makeStubCtx();
    let canceled = 0;
    const runtime = createFirmwareRuntime(fake.handle, ctx, {
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

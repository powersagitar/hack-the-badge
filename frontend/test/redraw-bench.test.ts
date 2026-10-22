import { describe, expect, test } from "bun:test";
import type { CpuRunSummary, FirmwareEmulatorHandle } from "../src/cpu/bridge";
import { fnv1a, PRESS_HOLD_US, pressAndSettle } from "../src/bench/walk";
import { DOWN_REDRAW, timeRedraw } from "../src/bench/redraw-bench";

const BEFORE = Uint16Array.of(1, 2, 3, 4);
const AFTER = Uint16Array.of(5, 6, 7, 8);
const WRONG = Uint16Array.of(9, 10, 11, 12);
/** One step at 80 MHz, in µs. */
const US_PER_STEP = 0.0125;
/** The fake wall clock: 10M steps per second of wall time. */
const STEPS_PER_WALL_MS = 10_000;
/** The fake redraws this many steps after the press (past the 8M-step hold). */
const REDRAW_AT = 9_000_000;

/**
 * A hand-written fake `FirmwareEmulatorHandle` (as
 * `test/firmware-runtime.test.ts` fakes one): steps run at 80 MHz of
 * emulated time, and the frame switches from BEFORE to `after`
 * REDRAW_AT steps after a button goes down. `now` is a wall clock that
 * advances with the steps run.
 */
function makeFake(after: Uint16Array) {
  let steps = 0;
  let pressedAt: number | undefined;
  const summary = (n: number): CpuRunSummary => ({
    steps: n,
    traps: 0,
    romStubCalls: 0,
    pc: 0x4200_0000,
    lastInstructionFault: undefined,
  });
  const handle: FirmwareEmulatorHandle = {
    run(budget) {
      steps += budget;
      return summary(budget);
    },
    runUntil(targetUs, maxSteps) {
      const n = Math.min(maxSteps, Math.max(0, Math.ceil(targetUs / US_PER_STEP - steps)));
      steps += n;
      return summary(n);
    },
    reset() {},
    setRawButton(_slot, pressed) {
      if (pressed) pressedAt = steps;
    },
    framebuffer() {
      return pressedAt !== undefined && steps >= pressedAt + REDRAW_AT ? after : BEFORE;
    },
    screenWidth: 2,
    screenHeight: 2,
    pc: () => 0x4200_0000,
    totalSteps: () => steps,
    emulatedTimeUs: () => steps * US_PER_STEP,
    serialInput: () => 0,
    serialPending: () => 0,
    consoleOutput: () => "",
    setAcceleration() {},
    dispose() {},
  };
  // Start somewhere other than zero, so the timing is relative to the press.
  handle.run(1_000_000);
  return { handle, now: () => steps / STEPS_PER_WALL_MS };
}

describe("timeRedraw", () => {
  test("measures a redraw from the press to the first sample showing the pinned end hash", () => {
    const { handle, now } = makeFake(AFTER);
    const r = timeRedraw("down", fnv1a(AFTER), handle, now, (onSample) =>
      pressAndSettle(handle, 4, fnv1a(BEFORE), onSample),
    );
    // The hold ends at 8,000,000 steps; samples follow every 250,000, and
    // the one at 9,000,000 is the first to show AFTER.
    expect(PRESS_HOLD_US / US_PER_STEP).toBe(8_000_000);
    const { pinFactor, ...measured } = r;
    expect(measured).toEqual({
      name: "down",
      emulatedMs: 112.5,
      steps: REDRAW_AT,
      wallMs: 900,
      factor: 0.125,
      mStepsPerS: 10,
    });
    // The pinned DOWN redraw's 115 ms over its 6,858,235 steps at 10M/s.
    expect(DOWN_REDRAW.emulatedUs).toBe(115_000);
    expect(pinFactor).toBeCloseTo(0.115 / (6_858_235 / 10e6), 12);
    // The settle still ran to the stable frame (8M more steps).
    expect(handle.totalSteps()).toBe(1_000_000 + REDRAW_AT + 8_000_000);
  });

  test("throws when a redraw ends on the wrong hash", () => {
    const { handle, now } = makeFake(WRONG);
    expect(() =>
      timeRedraw("home", fnv1a(AFTER), handle, now, (onSample) =>
        pressAndSettle(handle, 3, fnv1a(BEFORE), onSample),
      ),
    ).toThrow(/home redraw ended on/);
  });
});

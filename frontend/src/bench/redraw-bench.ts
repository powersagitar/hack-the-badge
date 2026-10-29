/**
 * Milestone 7's WASM redraw benchmark, host-independent: it runs over any
 * `FirmwareEmulatorHandle` and takes the wall clock and the log sink from
 * its host (`bench/bun-bench.ts` under Bun, `./bench-worker.ts` in a
 * browser Worker).
 *
 * The WASM handle cannot clone, so instead of replaying each redraw from a
 * checkpoint (as the native `bench_redraws` in
 * `emulator-core/tests/boot_progress.rs` does) the benchmark runs the
 * rungs' own walk once, with exactly their input schedule (that of
 * `test/cpu-wasm.test.ts`'s provisioned twin, then
 * `launcher_responds_to_navigation`'s DOWN), and times three presses in
 * place:
 *
 * - onboarding: A from page 1 to page 2, the first press after provisioning;
 * - home: HOME from the registered screen to the launcher;
 * - down: DOWN on the launcher.
 *
 * Each is timed from the press to the first 250,000-step sample whose
 * frame hash is the redraw's end hash (see `timeRedraw`). This in-walk
 * method includes the sampling's framebuffer copies and hashing, which the
 * native bench's unsampled timed passes do not; compare like with like only
 * (docs/milestone-7-decisions.md, R-T2-1). That sample can lie well past
 * the redraw's end in emulated time, so the comparable factor is
 * `pinFactor` (R-T2-2), not `factor`.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";
import { createProvisioner, type ProvisionState } from "../runtime/provisioner";
import {
  AFTER_PRESS_STABLE_FOR,
  AFTER_SHAKE_PAGES,
  ANIMATED_PAGE_STEPS,
  BOOT_STABLE_FOR,
  FIRST_RUN_HASH,
  FIRST_RUN_MAX_STEPS,
  LAUNCHER_AFTER_NAV_HASH,
  LAUNCHER_HASH,
  LAUNCHER_RESPONSE_STEPS,
  NAV_SLOT,
  ONBOARDING_RESPONSE_MARGIN,
  OWN_ROLE_PAGE1_HASH,
  OWN_ROLE_REGISTERED_HASH,
  press,
  pressAndSettle,
  PROVISION_DEADLINE,
  REGISTERED_RESPONSE_STEPS,
  runForUs,
  runUntilStable,
  SHAKE_HALF_US,
  SHAKEN_AT_REST_HASH,
  SPLASH_HASH,
  TO_LAUNCHER_SLOT,
  TO_SHAKE_PAGE,
} from "./walk";

export type RedrawName = "home" | "down" | "onboarding";

/** A press-driven redraw as the native bench pins it. */
export interface Redraw {
  /** Emulated time from the press to the last frame change (1 ms grid). */
  emulatedUs: number;
  /** Steps from the press to the last frame change. */
  steps: number;
  /** The frame the redraw ends on. */
  endHash: bigint;
}

/*
 * boot_progress.rs's HOME_REDRAW, DOWN_REDRAW and ONBOARDING_REDRAW (Task 1,
 * pinned at 6cd0a6e; re-pinned for v392 in Milestone 9), copied. The bench
 * uses `endHash`; the native emulated time and steps are printed beside the
 * in-walk numbers.
 */
export const HOME_REDRAW: Redraw = {
  emulatedUs: 244_000,
  steps: 18_292_336,
  endHash: LAUNCHER_HASH,
};
export const DOWN_REDRAW: Redraw = {
  emulatedUs: 109_000,
  steps: 7_942_619,
  endHash: LAUNCHER_AFTER_NAV_HASH,
};
export const ONBOARDING_REDRAW: Redraw = {
  emulatedUs: 52_000,
  steps: 2_547_134,
  endHash: TO_SHAKE_PAGE[0][1],
};

export interface RedrawTiming {
  name: RedrawName;
  /** Emulated time from the press to the first sample showing the end hash. */
  emulatedMs: number;
  steps: number;
  wallMs: number;
  /**
   * `emulatedMs` over `wallMs`. Not comparable with the native factor:
   * the first sample after a redraw can lie far past its end in emulated
   * time (the idle tail; onboarding's end lies inside the press hold).
   */
  factor: number;
  mStepsPerS: number;
  /**
   * The native factor's formula at this run's steps/s: the pinned
   * redraw's emulated time over the wall time its pinned steps take at
   * `mStepsPerS` (R-T2-2). 1.0 is the badge's speed.
   */
  pinFactor: number;
}

/** The native pin for each redraw. */
export const PINNED: Record<RedrawName, Redraw> = {
  home: HOME_REDRAW,
  down: DOWN_REDRAW,
  onboarding: ONBOARDING_REDRAW,
};

/** `PINNED[name]`'s emulated time over the wall time its steps take at `mStepsPerS`. */
export function pinFactor(name: RedrawName, mStepsPerS: number): number {
  const pin = PINNED[name];
  return pin.emulatedUs / 1e6 / (pin.steps / (mStepsPerS * 1e6));
}

export interface BenchResult {
  /** Boot through provisioning and onboarding to the launcher (before DOWN). */
  walk: { steps: number; wallMs: number };
  /** In the order home, down, onboarding. */
  redraws: RedrawTiming[];
}

const hex = (h: bigint) => `0x${h.toString(16).padStart(16, "0")}`;

/**
 * Times one redraw: `act` presses and settles (passing its `onSample` on to
 * `runUntilStable`) and returns the settled hash. Measured from just before
 * `act` to the first sample whose hash is `endHash`, with the sample's
 * hashing included. Throws if the redraw settles on another frame.
 */
export function timeRedraw(
  name: RedrawName,
  endHash: bigint,
  handle: FirmwareEmulatorHandle,
  now: () => number,
  act: (onSample: (hash: bigint) => void) => bigint,
): RedrawTiming {
  const s0 = handle.totalSteps();
  const t0 = handle.emulatedTimeUs();
  const w0 = now();
  let seen: { steps: number; us: number; wall: number } | undefined;
  const settled = act((h) => {
    if (seen === undefined && h === endHash) {
      seen = { wall: now(), steps: handle.totalSteps(), us: handle.emulatedTimeUs() };
    }
  });
  if (settled !== endHash || seen === undefined) {
    throw new Error(`${name} redraw ended on ${hex(settled)}, expected ${hex(endHash)}`);
  }
  const wallMs = seen.wall - w0;
  const emulatedMs = (seen.us - t0) / 1000;
  const steps = seen.steps - s0;
  const mStepsPerS = steps / wallMs / 1000;
  return {
    name,
    emulatedMs,
    steps,
    wallMs,
    factor: emulatedMs / wallMs,
    mStepsPerS,
    pinFactor: pinFactor(name, mStepsPerS),
  };
}

/** Throws unless the walk reached `expected`. */
function expectHash(what: string, got: bigint, expected: bigint): void {
  if (got !== expected) throw new Error(`${what}: got ${hex(got)}, expected ${hex(expected)}`);
}

/** One `BENCH` line for a redraw, with the native pin beside it. */
export function formatRedraw(r: RedrawTiming): string {
  const pinned = PINNED[r.name];
  return (
    `BENCH ${r.name} emulated_ms=${r.emulatedMs.toFixed(1)} steps=${r.steps} ` +
    `wall_ms=${r.wallMs.toFixed(1)} factor=${r.factor.toFixed(3)} ` +
    `msteps_per_s=${r.mStepsPerS.toFixed(2)} pin_factor=${r.pinFactor.toFixed(3)} ` +
    `(native pin: emulated_ms=${(pinned.emulatedUs / 1000).toFixed(1)} steps=${pinned.steps})`
  );
}

/** One `BENCH` line for the walk. */
export function formatWalk(walk: BenchResult["walk"]): string {
  return (
    `BENCH walk steps=${walk.steps} wall_s=${(walk.wallMs / 1000).toFixed(2)} ` +
    `msteps_per_s=${(walk.steps / walk.wallMs / 1000).toFixed(2)}`
  );
}

function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 === 1 ? s[m] : (s[m - 1] + s[m]) / 2;
}

/**
 * The median over several runs, as `BENCH median ...` lines: the walk's
 * steps/s, then per redraw the median wall time and the factor, steps/s
 * and pin factor it gives (emulated time and steps are the same in every
 * run).
 */
export function summarizeRuns(results: BenchResult[]): string[] {
  const n = results.length;
  const walkRate = median(results.map((r) => r.walk.steps / r.walk.wallMs / 1000));
  const lines = [`BENCH median runs=${n} walk msteps_per_s=${walkRate.toFixed(2)}`];
  for (const name of ["home", "down", "onboarding"] as const) {
    const runs = results.map((r) => r.redraws.find((x) => x.name === name)!);
    const wallMs = median(runs.map((x) => x.wallMs));
    const { emulatedMs, steps } = runs[0];
    if (runs.some((x) => x.emulatedMs !== emulatedMs || x.steps !== steps)) {
      throw new Error(`${name}: runs disagree on emulated time or steps`);
    }
    const rate = steps / wallMs / 1000;
    lines.push(
      `BENCH median runs=${n} ${name} emulated_ms=${emulatedMs.toFixed(1)} steps=${steps} ` +
        `wall_ms=${wallMs.toFixed(1)} factor=${(emulatedMs / wallMs).toFixed(3)} ` +
        `msteps_per_s=${rate.toFixed(2)} pin_factor=${pinFactor(name, rate).toFixed(3)}`,
    );
  }
  return lines;
}

/**
 * Runs the walk on a freshly booted `handle` (provisioning `identityJson`,
 * the hacker fixture) and times the three redraws. `now` is a wall clock in
 * milliseconds; `log` receives progress and one `BENCH` line per item.
 */
export function runRedrawBench(
  handle: FirmwareEmulatorHandle,
  identityJson: string,
  now: () => number,
  log: (line: string) => void,
): BenchResult {
  const start = now();
  expectHash(
    "first-run screen",
    runUntilStable(handle, FIRST_RUN_MAX_STEPS, BOOT_STABLE_FOR, [SPLASH_HASH]),
    FIRST_RUN_HASH,
  );
  log(`booted to the first-run screen at step ${handle.totalSteps()}`);

  const p = createProvisioner(handle, identityJson);
  let state: ProvisionState = p.poll();
  while (
    (state.kind === "waiting" || state.kind === "typing") &&
    handle.totalSteps() < PROVISION_DEADLINE
  ) {
    const fault = handle.run(50_000).lastInstructionFault;
    if (fault !== undefined) throw new Error(`fault at 0x${fault.toString(16)} while provisioning`);
    state = p.poll();
  }
  if (state.kind !== "ok" || !state.line.startsWith("PROV OK id=test-")) {
    throw new Error(`provisioning did not succeed: ${JSON.stringify(state)}`);
  }
  log(`provisioned at step ${handle.totalSteps()}`);

  let cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
  let previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [FIRST_RUN_HASH]);
  expectHash("onboarding page 1", previous, OWN_ROLE_PAGE1_HASH);

  const [[firstSlot, firstHash], ...restToShake] = TO_SHAKE_PAGE;
  const before = previous;
  const onboarding = timeRedraw("onboarding", ONBOARDING_REDRAW.endHash, handle, now, (onSample) =>
    pressAndSettle(handle, firstSlot, before, onSample),
  );
  previous = firstHash;
  log(formatRedraw(onboarding));
  for (const [slot, expected] of restToShake) {
    previous = pressAndSettle(handle, slot, previous);
    expectHash(`onboarding press of slot ${slot}`, previous, expected);
  }
  for (let k = 0; k < 6; k++) {
    const s = k % 2 === 0 ? 1 : -1;
    handle.setAcceleration(s * 2000, s * 2000, 1000);
    runForUs(handle, SHAKE_HALF_US);
  }
  handle.setAcceleration(0, 0, 1000);
  cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
  previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous]);
  expectHash("shaken, at rest", previous, SHAKEN_AT_REST_HASH);
  for (const [slot, expected] of AFTER_SHAKE_PAGES) {
    if (expected === null) {
      press(handle, slot);
      const fault = handle.run(ANIMATED_PAGE_STEPS).lastInstructionFault;
      if (fault !== undefined) throw new Error(`fault at 0x${fault.toString(16)} on page 8`);
    } else {
      previous = pressAndSettle(handle, slot, previous);
      expectHash(`onboarding press of slot ${slot}`, previous, expected);
    }
  }
  press(handle, 0); // START finishes Setup
  cap = handle.totalSteps() + REGISTERED_RESPONSE_STEPS;
  previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous]);
  expectHash("registered screen", previous, OWN_ROLE_REGISTERED_HASH);
  log(`registered screen at step ${handle.totalSteps()}`);

  const registered = previous;
  const home = timeRedraw("home", HOME_REDRAW.endHash, handle, now, (onSample) => {
    press(handle, TO_LAUNCHER_SLOT);
    const homeCap = handle.totalSteps() + LAUNCHER_RESPONSE_STEPS;
    return runUntilStable(handle, homeCap, AFTER_PRESS_STABLE_FOR, [registered], onSample);
  });
  if (!handle.consoleOutput().includes("launched Launcher")) {
    throw new Error("the console never said launched Launcher");
  }
  const walk = { steps: handle.totalSteps(), wallMs: now() - start };
  log(formatRedraw(home));
  log(formatWalk(walk));

  const down = timeRedraw("down", DOWN_REDRAW.endHash, handle, now, (onSample) =>
    pressAndSettle(handle, NAV_SLOT, LAUNCHER_HASH, onSample),
  );
  log(formatRedraw(down));

  return { walk, redraws: [home, down, onboarding] };
}

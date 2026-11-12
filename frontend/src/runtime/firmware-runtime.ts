/**
 * Real-firmware runtime: mirrors `src/runtime/lifecycle.ts`'s role, but for
 * the CPU-emulator side (Milestone 2) instead of the Lua sandbox
 * (Milestone 1). Owns a `FirmwareEmulatorHandle` (`src/cpu/bridge.ts`), drives
 * a frame loop that runs the CPU to the frame's wall-clock time target and
 * hands the reconstructed framebuffer to a `present` callback, and exposes an
 * `injectButton`-shaped API close enough to `lifecycle.ts`'s `AppRuntime` that
 * `shell.ts`'s existing button-pad/keyboard wiring can drive either runtime
 * without knowing which one it's holding.
 *
 * ## Where it runs (Milestone 7 Task 8)
 *
 * In the app this runtime runs inside a dedicated Web Worker
 * (`./firmware-worker.ts`): its loop is driven by slices on `setTimeout`
 * (or an immediate yield while the core is behind, ruling R-T8-1), capped
 * at `WORKER_STEPS_PER_SLICE` steps, timed by the `now` option, and
 * `present` posts the framebuffer to the main thread, where
 * `./firmware-worker-client.ts` paints it. The button tap queue, the
 * shake and bump schedule and the provisioner below therefore all live in the
 * Worker, unchanged. The in-thread shape (a `requestAnimationFrame` loop
 * whose `present` blits with `src/render/framebuffer.ts`, capped at
 * `DEFAULT_STEPS_PER_FRAME`) is what the unit tests drive.
 *
 * ## Why `requestAnimationFrame`, unlike `lifecycle.ts`'s `setInterval`
 *
 * `lifecycle.ts` ticks Lua logic on a timer decoupled from paint (a separate
 * `requestAnimationFrame` loop in `main.ts` repaints the widget tree
 * whenever the browser feels like it). Here there's no such split: "step the
 * CPU" and "show what it drew" are the same visible unit of work each frame,
 * so both happen in one `requestAnimationFrame` callback, the same shape
 * `main.ts`'s existing paint loop already uses for Lua mode.
 *
 * `scheduleFrame`/`cancelFrame` are injectable (default: the real
 * `requestAnimationFrame`/`cancelAnimationFrame`) so this module's frame-step
 * logic is testable under `bun test`, which has no `requestAnimationFrame`
 * global (confirmed via `bun -e 'console.log(typeof requestAnimationFrame)'`
 * -> `undefined`, the same fact `src/render/framebuffer.ts`'s module doc
 * records about `ImageData`).
 *
 * ## Pacing
 *
 * Emulated time follows `requestAnimationFrame` time: each frame runs the
 * emulator (`FirmwareEmulatorHandle.runUntil`) until its emulated clock has
 * advanced by the wall-clock time since the previous frame (exactly: an idle
 * wait stops at the target instead of jumping to the next alarm,
 * `emulator-core`'s `boot::idle_until`, ruling R-T7-4), so the firmware's
 * own timers (Snake's move period, the button polling, animations) run at the
 * badge's speed. The first frame after `start()` (or `reset()`) has no
 * previous frame and runs nothing, so a paused or backgrounded runtime does
 * not count the time it was stopped.
 *
 * Two bounds keep one frame from freezing the tab:
 *
 * - The wall delta is clamped to `MAX_FRAME_DELTA_MS` (50 ms). A tab that
 *   comes back from the background, or a frame the browser delayed, gets at
 *   most 50 ms of emulated time, not a catch-up burst.
 * - `DEFAULT_STEPS_PER_FRAME` (150,000 steps) caps the work per frame. The
 *   emulated clock advances by more than one step's worth per step while the
 *   CPU waits in WFI (the SYSTIMER fast-forward), so a mostly idle firmware
 *   reaches its target in far fewer steps than 80 MHz x the delta. Snake
 *   needs about 4,000 to 30,000 steps per 8.3 ms frame, and about 230,000
 *   for a frame that redraws a move. The cap is measured (Milestone 6
 *   Task 7, ruling R-T7-5; `docs/milestone-6-decisions.md`). With the
 *   first cap of 1,000,000 steps, a busy stretch after onboarding ran at
 *   9.6 fps in the human's browser, so about 104 ms per frame, or about
 *   10M steps/s. Bun's JavaScriptCore runs the WASM at 12.3M steps/s.
 *   150,000 steps is 12 to 15 ms of work, about one 60 Hz frame. Long
 *   frames also delay input: a key event waits for the running frame, and
 *   its paint waits for the next one, so 104 ms frames gave the measured
 *   328 ms INP.
 *
 * When the cap binds, the frame stops short of its target and the missed
 * emulated time (the debt) is dropped: the next frame targets its own
 * `now + delta`, not the missed time. The badge then runs in slow motion
 * rather than stalling the tab to catch up; `FirmwareRuntime.speed()` (the
 * smoothed emulated/wall ratio, shown in the firmware status line) reports
 * it.
 *
 * `speed()` divides each frame's emulated advance by the real, unclamped
 * wall delta. Frames longer than 50 ms are slow motion even when the step
 * cap does not bind (each one advances only 50 ms), and the readout has to
 * show that. A tab returning from the background therefore contributes one
 * near-zero sample, at the smoothing's 0.1 weight.
 *
 * ## The reboot's pause (Milestone 11 Task 8)
 *
 * A software reset adds `rebootUs()` (the core's `REBOOT_US`, 466 ms) to
 * emulated time in one jump, inside the frame that reached it. Measured
 * from the new emulated time, the pacing would absorb the jump at once.
 * Instead the frame reports it ({@link FrameResult.rebootUs}, `n` resets
 * x `rebootUs()`), and the runtime carries it as wall-clock debt: each
 * later frame's clamped wall delta pays the debt down before it runs any
 * new emulated time, so the screen holds for about 466 ms, as the
 * badge's does. While the debt is paid, emulated time does not move.
 *
 * The speed reading leaves the jump out, and counts the debt a frame paid
 * as emulated time: the badge is rebooting at its own speed, not running
 * slow. A tap's `downSince` and `upSince` move forward by the jump, so a
 * hold or a release gap counts only non-jump emulated time, and a tap
 * held across a reboot still lasts its full `PRESS_HOLD_US`.
 *
 * ## Tap sources and taps across `stop()` (Milestone 11 Task 13)
 *
 * A button can be held by the keyboard and the pad at once
 * ({@link ButtonSource}). It is down while any source holds it, and its
 * tap is let go when every source has; a source releasing a button it
 * never pressed changes nothing. `stop()` releases a tap whose user has
 * let go (its deferred release, R-M8-6) but keeps the taps queued behind
 * it: emulated time does not run while stopped, so they play after the
 * next `start()`, with their full hold and release gap.
 *
 * The runtime also records where the current boot's console text begins
 * (the console's length at the frame that ended at a soft reset), for the
 * provisioner's first prompt (`./provisioner.ts`).
 *
 * ## Skip setup (Milestone 13)
 *
 * `skipSetup()` starts a `./setup-skip.ts` skipper, polled in the frame
 * loop beside the provisioner and bounded the same way, in steps. It is
 * refused (returns `false`, does nothing) unless Setup is on screen, no
 * provision is in flight (its token phase runs after Setup appears, and
 * the two must never interleave lines) and no skip is in flight, so a
 * double press types one `config` and one `reboot`. `setupOnScreen()`
 * reads the current boot's console from `bootConsoleStart` and is false
 * while a skip runs; it copies that text, so it belongs in a status, not
 * in a per-step path.
 */
import type { ButtonName, ButtonSource } from "../badge/input";
import type { FirmwareEmulatorHandle } from "../cpu/bridge";
import { consoleSince } from "./console-io";
import { createProvisioner, type ProvisionState, type Provisioner } from "./provisioner";
import {
  createSetupSkipper,
  setupOnScreen as setupOnScreenIn,
  SKIP_SETUP_TIMEOUT_STEPS,
  type SetupSkipper,
  type SkipState,
} from "./setup-skip";

/** Largest wall-clock delta (ms) one frame advances emulated time by; see "Pacing" above. */
export const MAX_FRAME_DELTA_MS = 50;

/** Most instruction steps one frame runs; see "Pacing" above. */
export const DEFAULT_STEPS_PER_FRAME = 150_000;

/**
 * Most instruction steps one Worker slice runs (Milestone 7 Task 8): about
 * 5 ms of work at the badge's 80M steps/s, about 10 ms at Chrome's measured
 * 40.5M steps/s (Task 7). A slice that reaches its target needs far fewer
 * steps, so the cap binds only when the core is slower than the badge; it
 * bounds how long a button message or a frame acknowledgement waits behind
 * a slice. The Worker does no painting, so a slice longer than one 60 Hz
 * frame does not delay the page. See `docs/milestone-7-decisions.md`'s
 * Task 8 rulings.
 */
export const WORKER_STEPS_PER_SLICE = 400_000;

/**
 * Human button name -> raw slot index, as consumed by
 * `FirmwareEmulatorHandle.setRawButton`. Verified directly against
 * `emulator-core`'s own source, not trusted from a prior report's prose:
 *
 * - `emulator-core/src/runtime.rs`'s module doc + `FirmwareRuntime::apply_button`:
 *   slot 0 is `PIN_START` (direct GPIO9, i.e. `START`); slots 1..=8 map to
 *   `Hc165::set_button`'s index `n - 1`, i.e. slot `n` drives shift-register
 *   index `n - 1`.
 * - `emulator-core/src/peripherals/gpio.rs`'s module doc (the `Hc165`
 *   slot->button table, itself sourced from the same third-party reference
 *   firmware the task brief names,
 *   `github.com/abigail-liang/hack-the-north-2026-badge`'s `firmware/main.c`):
 *   shift-register index 0..=7 is `A, B, HOME, DOWN, LEFT, RIGHT, UP, AUX1`.
 *
 * Composing the two gives the table below. Total mapping: `NUM_RAW_BUTTON_SLOTS`
 * (`emulator-core/src/runtime.rs`) is 9, exactly `BUTTON_NAMES.length`
 * (`src/badge/input.ts`) — nothing left over on either side.
 */
export const RAW_SLOT_BY_BUTTON: Record<ButtonName, number> = {
  START: 0,
  A: 1,
  B: 2,
  HOME: 3,
  DOWN: 4,
  LEFT: 5,
  RIGHT: 6,
  UP: 7,
  AUX1: 8,
};

/**
 * Emulated steps after `provision()` before an unfinished provisioner is
 * abandoned and reported as failed ("timed out"). Steps, not wall time, so
 * a slow browser does not time out early.
 */
export const PROVISION_TIMEOUT_STEPS = 60_000_000;

/**
 * Minimum emulated time a button stays pressed: 100 ms (same value as
 * `PRESS_HOLD_US` in `emulator-core/tests/boot_progress.rs`). A shorter
 * click would be missed by the firmware's button polling.
 */
export const PRESS_HOLD_US = 100_000;

/**
 * Minimum emulated time a button stays released before its next press:
 * 40 ms. The firmware's debouncer posts a change after two equal scans, so
 * a shorter gap merges two taps into one press. Measured from the launcher
 * (Milestone 8): two 100 ms DOWN presses move the selection twice from a
 * 20 ms gap up, at ten scan phases, and once from 15 ms down; 40 ms is
 * twice the measured minimum.
 */
export const RELEASE_GAP_US = 40_000;

/**
 * Most taps one button queues, the held one included. Each takes at least
 * `PRESS_HOLD_US` + `RELEASE_GAP_US` (140 ms) of emulated time, so the cap
 * bounds how long a burst of taps keeps replaying after the hand stops;
 * taps beyond it are dropped.
 */
export const MAX_QUEUED_TAPS = 8;

/** Emulated µs per shake half-period (`SHAKE_HALF_US` in boot_progress.rs). */
export const SHAKE_HALF_US = 62_500;
const SHAKE_HALF_PERIODS = 6;
const REST_ACCELERATION: [number, number, number] = [0, 0, 1000];

/**
 * The acceleration (mg) `elapsedUs` of emulated time into a shake: six
 * alternating half-periods (+,-,+,...) of (s*2000, s*2000, 1000), then
 * `null` (done; the caller restores the rest acceleration). Mirrors
 * `shake()` in `emulator-core/tests/boot_progress.rs`.
 */
export function shakeAcceleration(elapsedUs: number): [number, number, number] | null {
  const k = Math.floor(elapsedUs / SHAKE_HALF_US);
  if (k >= SHAKE_HALF_PERIODS) return null;
  const s = k % 2 === 0 ? 1 : -1;
  return [s * 2000, s * 2000, 1000];
}

/** Emulated µs a bump's pulse lasts (`BUMP_US` in boot_progress.rs, ruling R-T8-2). */
export const BUMP_US = 100_000;
/** The bump's acceleration (mg): +1,000 mg on x over the face-up rest (`BUMP_MG`). */
const BUMP_ACCELERATION: [number, number, number] = [1000, 0, 1000];

/**
 * The acceleration (mg) `elapsedUs` of emulated time into a bump:
 * (1000, 0, 1000) for `BUMP_US`, then `null` (done; the caller restores
 * the rest acceleration). Mirrors `bump()` in
 * `emulator-core/tests/boot_progress.rs` (ruling R-T8-2): one step on one
 * axis, more than the 400 mg Connect's detector needs between two
 * samples, held longer than its longest sample gap (70 ms) and shorter
 * than its hold-off (320 ms).
 */
export function bumpAcceleration(elapsedUs: number): [number, number, number] | null {
  return elapsedUs < BUMP_US ? [...BUMP_ACCELERATION] : null;
}

export type FrameScheduler = (cb: (timeMs: number) => void) => number;
export type FrameCanceler = (handle: number) => void;
/** Receives a frame's framebuffer: RGB565, row-major, `width * height` elements. */
export type FramePresenter = (fb: Uint16Array, width: number, height: number) => void;

export interface FirmwareRuntimeOptions {
  /** Most instruction steps to run per animation frame. Default: {@link DEFAULT_STEPS_PER_FRAME}. */
  stepsPerFrame?: number;
  /** Injectable for testing; defaults to the real `requestAnimationFrame`. */
  scheduleFrame?: FrameScheduler;
  /** Injectable for testing; defaults to the real `cancelAnimationFrame`. */
  cancelFrame?: FrameCanceler;
  /**
   * Wall-clock time (ms) a frame is timed by. When set, the loop reads it
   * instead of the time the scheduler passes (the Worker's `setTimeout`
   * passes none). Default: the scheduler's time argument.
   */
  now?: () => number;
  /**
   * Called after each frame's polls with what the frame did and its real
   * (unclamped) wall delta. The Worker uses it to choose how to yield and
   * to post its status.
   */
  onFrame?: (result: FrameResult, wallDeltaMs: number) => void;
}

/**
 * The emulated time (µs) a frame starting at `emulatedUs` runs to: the wall
 * delta, clamped to `[0, MAX_FRAME_DELTA_MS]`, added as emulated time.
 */
export function frameTargetUs(emulatedUs: number, wallDeltaMs: number): number {
  const delta = Math.min(Math.max(wallDeltaMs, 0), MAX_FRAME_DELTA_MS);
  return emulatedUs + delta * 1000;
}

/** What one {@link stepFirmwareFrame} did. */
export interface FrameResult {
  /** Emulated time (µs) the frame advanced, any reboot's jump included. */
  emulatedDeltaUs: number;
  /** The step cap stopped the frame short of its target (the debt is dropped). */
  capped: boolean;
  /**
   * Emulated time (µs) the frame's software resets added in one jump:
   * the resets it ran x `rebootUs()`, or 0. See "The reboot's pause".
   */
  rebootUs: number;
}

/**
 * Runs one frame's worth of work: run the CPU until its emulated clock has
 * advanced by `wallDeltaMs` (clamped, see {@link frameTargetUs}) or
 * `stepsPerFrame` steps have run, then hand its current framebuffer to
 * `present`. Pure with respect to scheduling (no `requestAnimationFrame`
 * involved) so it's directly unit-testable against a fake
 * `FirmwareEmulatorHandle` and a fake presenter.
 */
export function stepFirmwareFrame(
  handle: FirmwareEmulatorHandle,
  present: FramePresenter,
  wallDeltaMs: number,
  stepsPerFrame: number,
): FrameResult {
  const before = handle.emulatedTimeUs();
  const resetsBefore = handle.softResetCount();
  const target = frameTargetUs(before, wallDeltaMs);
  handle.runUntil(target, stepsPerFrame);
  const after = handle.emulatedTimeUs();
  const resets = handle.softResetCount() - resetsBefore;
  present(handle.framebuffer(), handle.screenWidth, handle.screenHeight);
  // The wasm shim truncates the target to whole µs (`target_us as u64`), so
  // an uncapped frame with a fractional rAF delta ends at floor(target).
  return {
    emulatedDeltaUs: after - before,
    capped: after < Math.floor(target),
    rebootUs: resets > 0 ? resets * handle.rebootUs() : 0,
  };
}

export interface FirmwareRuntime {
  /** Begins the per-frame step+blit loop. Idempotent while already running. */
  start(): void;
  /**
   * Stops the loop and releases any button whose release was deferred (see
   * `PRESS_HOLD_US`). Taps queued behind it are kept, and play after the
   * next `start()`. Does not dispose the underlying emulator handle.
   */
  stop(): void;
  /**
   * `lifecycle.ts`'s `AppRuntime.injectButton`, plus the press's source
   * (default `"pad"`): see "Tap sources and taps across `stop()`".
   */
  injectButton(button: ButtonName, kind: "pressed" | "released", source?: ButtonSource): void;
  /** Starts typing the provisioning flow for `identityJson`, replacing any in flight. */
  provision(identityJson: string): void;
  /** The current provisioning state, or `null` if none was started since the last reset. */
  provisionState(): ProvisionState | null;
  /**
   * Starts skipping Setup (see "Skip setup"): `true` if a skip started,
   * `false` (and nothing done) unless Setup is on screen, no provision is
   * in flight and no skip is in flight.
   */
  skipSetup(): boolean;
  /** The last skip's state, or `null` if none was started since the last reset. */
  skipState(): SkipState | null;
  /** Setup is on screen in the current boot, and no skip is running. */
  setupOnScreen(): boolean;
  /** A soft reset's pause is still being paid in wall time (see "The reboot's pause"). */
  rebooting(): boolean;
  /** Shakes the emulated accelerometer (the onboarding's "Shake it!" page). */
  shake(): void;
  /**
   * Bumps the badge ({@link bumpAcceleration}): Connect's bump, which
   * starts its radio. Replaces a shake still playing, as a shake replaces
   * a bump; the pulse starts at once (ruling R-T9-2).
   */
  bump(): void;
  /** Re-boots the firmware from the same image, keeping held buttons. */
  reset(): void;
  /**
   * Smoothed ratio of emulated time to wall-clock time over recent frames
   * (1 = the badge's speed; below 1 when the step cap binds or frames take
   * longer than `MAX_FRAME_DELTA_MS`). `0` until the
   * second frame after `start()` or `reset()`.
   */
  speed(): number;
  /**
   * Real-firmware mode has no Lua-style exit-to-launcher (that's
   * a Lua-sandbox-mode-only affordance — see `badge.app.exit()` in
   * `lifecycle.ts`). Always `false`; exists so `shell.ts`/`main.ts` can treat
   * both runtimes uniformly without a type-level special case.
   */
  isExitRequested(): boolean;
  /** Stops the loop (if running) and releases the WASM-side emulator. */
  dispose(): void;
}

/**
 * `true` when `name` is one of the badge's buttons. Guards input that
 * crossed a message boundary: `RAW_SLOT_BY_BUTTON["toString"]` is not
 * `undefined`, so a lookup alone is not a check.
 */
export function isButtonName(name: unknown): name is ButtonName {
  return typeof name === "string" && Object.prototype.hasOwnProperty.call(RAW_SLOT_BY_BUTTON, name);
}

/**
 * Builds a {@link FirmwareRuntime} around an already-constructed
 * `FirmwareEmulatorHandle` (see `src/cpu/bridge.ts`'s `createFirmwareEmulator`)
 * and the callback each frame's framebuffer goes to.
 */
export function createFirmwareRuntime(
  handle: FirmwareEmulatorHandle,
  present: FramePresenter,
  options: FirmwareRuntimeOptions = {},
): FirmwareRuntime {
  const stepsPerFrame = options.stepsPerFrame ?? DEFAULT_STEPS_PER_FRAME;
  const scheduleFrame: FrameScheduler =
    options.scheduleFrame ??
    ((cb) => {
      if (typeof requestAnimationFrame !== "function") {
        throw new Error(
          "runtime/firmware-runtime: no global requestAnimationFrame available; pass scheduleFrame explicitly (e.g. in tests)",
        );
      }
      return requestAnimationFrame(cb);
    });
  const cancelFrame: FrameCanceler =
    options.cancelFrame ??
    ((id) => {
      if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(id);
    });
  const now = options.now;
  const onFrame = options.onFrame;

  let frameHandle: number | null = null;
  let disposed = false;
  /** `requestAnimationFrame` time of the previous frame, or `null` (none since start/reset). */
  let lastWallMs: number | null = null;
  /** Exponentially smoothed emulated/wall ratio; `0` before the second frame. */
  let speed = 0;
  /**
   * Emulated µs of reboot jumps not yet paid in wall time: later frames
   * run no new emulated time until it is paid. See "The reboot's pause".
   */
  let rebootDebtUs = 0;

  let provisioner: Provisioner | null = null;
  let lastState: ProvisionState | null = null;
  let provisionStartedAt = 0;
  let skipper: SetupSkipper | null = null;
  let lastSkip: SkipState | null = null;
  let skipStartedAt = 0;
  /**
   * Absolute console offset (bytes, `consoleWritten()`) where the current
   * boot's text begins: 0 until a soft reset, then the console's written
   * count in the frame that ended at it (the core's `run_until` stops at
   * a reset whose jump passes the target, which a frame's at most 50 ms
   * target always is). A cold `reset()` starts the count at 0 again.
   */
  let bootConsoleStart = 0;
  /**
   * The accelerometer motion playing (a shake or a bump): its profile and
   * the emulated time (µs) it began, or `null`.
   */
  let motion: { profile: (elapsedUs: number) => [number, number, number] | null; startedAt: number } | null = null;
  let lastMotionAccel: string | null = null;
  /**
   * Per button: its taps in order, each `true` once the user has let go
   * (at most `MAX_QUEUED_TAPS`). The first is the one held in the emulator
   * while `downSince` is set; the rest wait their turn. `upSince` is the
   * emulated time (µs) of the button's last release, for `RELEASE_GAP_US`.
   * `held` is the sources holding the last tap: it is non-empty exactly
   * while the last tap exists and is not yet let go.
   */
  interface ButtonTaps {
    taps: boolean[];
    downSince: number | null;
    upSince: number | null;
    held: Set<ButtonSource>;
  }
  const buttonTaps = new Map<ButtonName, ButtonTaps>();

  function pollProvisioner(now: number): void {
    if (!provisioner) return;
    lastState = provisioner.poll();
    if (lastState.kind === "ok" || lastState.kind === "failed") {
      provisioner = null;
    } else if (now - provisionStartedAt > PROVISION_TIMEOUT_STEPS) {
      provisioner = null;
      lastState = { kind: "failed", reason: "timed out" };
    }
  }

  function pollSkipper(now: number): void {
    if (!skipper) return;
    lastSkip = skipper.poll();
    if (lastSkip.kind !== "typing") {
      skipper = null;
    } else if (now - skipStartedAt > SKIP_SETUP_TIMEOUT_STEPS) {
      skipper = null;
      lastSkip = { kind: "failed", reason: "timed out" };
    }
  }

  function pollMotion(now: number): void {
    if (motion === null) return;
    const a = motion.profile(now - motion.startedAt);
    if (a === null) {
      motion = null;
      lastMotionAccel = null;
      handle.setAcceleration(...REST_ACCELERATION);
      return;
    }
    const key = a.join(",");
    if (key === lastMotionAccel) return;
    lastMotionAccel = key;
    handle.setAcceleration(...a);
  }

  /**
   * Starts `profile` now, replacing any motion playing, and applies its
   * first acceleration at once, so the whole first period is played
   * (ruling R-T9-2).
   */
  function startMotion(profile: (elapsedUs: number) => [number, number, number] | null): void {
    const now = handle.emulatedTimeUs();
    motion = { profile, startedAt: now };
    lastMotionAccel = null;
    pollMotion(now);
  }

  /**
   * Plays `button`'s taps up to emulated time `now`: releases the held tap
   * once the user has let go and it has been down `PRESS_HOLD_US`, and
   * presses the next once the button has been up `RELEASE_GAP_US`.
   */
  function serviceTaps(button: ButtonName, state: ButtonTaps, now: number): void {
    const slot = RAW_SLOT_BY_BUTTON[button];
    for (;;) {
      if (state.downSince !== null) {
        if (!state.taps[0] || now - state.downSince < PRESS_HOLD_US) return;
        handle.setRawButton(slot, false);
        state.taps.shift();
        state.downSince = null;
        state.upSince = now;
      } else {
        if (state.taps.length === 0) return;
        if (state.upSince !== null && now - state.upSince < RELEASE_GAP_US) return;
        handle.setRawButton(slot, true);
        state.downSince = now;
      }
    }
  }

  function pollTaps(now: number): void {
    for (const [button, state] of buttonTaps) serviceTaps(button, state, now);
  }

  /**
   * Moves every tap's `downSince` and `upSince` forward by a reboot's
   * jump (`jumpUs`), so holds and release gaps count only non-jump time.
   */
  function shiftTapsPastReboot(jumpUs: number): void {
    for (const state of buttonTaps.values()) {
      if (state.downSince !== null) state.downSince += jumpUs;
      if (state.upSince !== null) state.upSince += jumpUs;
    }
  }

  /**
   * At `stop()`: releases a tap held down in the emulator that the user
   * has let go of (its deferred release), so nothing is left down while
   * the loop is stopped. The taps behind it stay queued, and one the user
   * is still holding stays down. The release time is kept: emulated time
   * does not move while stopped, so the next press still waits
   * `RELEASE_GAP_US` after the next `start()`.
   */
  function releaseLetGoTaps(): void {
    const nowUs = handle.emulatedTimeUs();
    for (const [button, state] of buttonTaps) {
      if (state.downSince === null || !state.taps[0]) continue;
      handle.setRawButton(RAW_SLOT_BY_BUTTON[button], false);
      state.taps.shift();
      state.downSince = null;
      state.upSince = nowUs;
    }
  }

  /**
   * At a cold `reset()`: drops every queued tap and releases a button held
   * down in the emulator that the user has let go of. One the user is
   * still holding stays down, as its only tap, and counts as held long
   * enough: emulated time restarts at zero, so it is released as soon as
   * the user lets go, and the firmware boots with no release to keep a gap
   * from.
   */
  function dropTapsForReset(): void {
    for (const [button, state] of buttonTaps) {
      if (state.downSince !== null && !state.taps[0]) {
        state.downSince = -PRESS_HOLD_US;
      } else {
        // Don't strand a button whose deferred release we are dropping.
        if (state.downSince !== null) handle.setRawButton(RAW_SLOT_BY_BUTTON[button], false);
        state.taps.length = 0;
        state.downSince = null;
        state.held.clear();
      }
      state.upSince = null;
    }
  }

  function loop(timeMs: number): void {
    // Guards against a callback that was already queued by the real
    // scheduler before stop() canceled it — a race a test's fake scheduler
    // can also simulate by invoking a "canceled" callback directly.
    if (frameHandle === null) return;
    const wallMs = now ? now() : timeMs;
    const delta = lastWallMs === null ? 0 : wallMs - lastWallMs;
    lastWallMs = wallMs;
    // The clamped wall delta pays the reboot's debt first; the rest runs.
    // Without debt the delta passes through untouched (no µs round trip).
    const paidUs = Math.min(rebootDebtUs, frameTargetUs(0, delta));
    rebootDebtUs -= paidUs;
    const runMs = paidUs > 0 ? Math.min(Math.max(delta, 0), MAX_FRAME_DELTA_MS) - paidUs / 1000 : delta;
    const r = stepFirmwareFrame(handle, present, runMs, stepsPerFrame);
    if (r.rebootUs > 0) {
      rebootDebtUs += r.rebootUs;
      shiftTapsPastReboot(r.rebootUs);
      bootConsoleStart = handle.consoleWritten();
    }
    if (delta > 0) {
      // The real frame time, not the clamped one: see "Pacing" above. The
      // jump is left out and the paid debt counted: see "The reboot's pause".
      const ratio = (r.emulatedDeltaUs - r.rebootUs + paidUs) / (delta * 1000);
      speed = speed === 0 ? ratio : 0.9 * speed + 0.1 * ratio;
    }
    // The provisioner's timeout is in steps; holds and shakes in emulated time.
    pollProvisioner(handle.totalSteps());
    pollSkipper(handle.totalSteps());
    const nowUs = handle.emulatedTimeUs();
    pollMotion(nowUs);
    pollTaps(nowUs);
    onFrame?.(r, delta);
    if (frameHandle !== null) {
      frameHandle = scheduleFrame(loop);
    }
  }

  function start(): void {
    if (disposed) throw new Error("runtime/firmware-runtime: cannot start a disposed runtime");
    if (frameHandle !== null) return;
    // A resume must not count the time the loop was stopped.
    lastWallMs = null;
    frameHandle = scheduleFrame(loop);
  }

  function stop(): void {
    if (frameHandle !== null) {
      cancelFrame(frameHandle);
      frameHandle = null;
    }
    // A deferred release would wait for the loop; the queued taps can.
    releaseLetGoTaps();
  }

  function injectButton(button: ButtonName, kind: "pressed" | "released", source: ButtonSource = "pad"): void {
    if (!isButtonName(button)) return;
    let state = buttonTaps.get(button);
    if (!state) {
      state = { taps: [], downSince: null, upSince: null, held: new Set() };
      buttonTaps.set(button, state);
    }
    if (kind === "pressed") {
      // A press from a source already holding it is a repeat, not a tap.
      if (state.held.has(source)) return;
      // A second source joins the held tap; otherwise this is a new tap.
      if (state.held.size === 0) {
        if (state.taps.length >= MAX_QUEUED_TAPS) return;
        state.taps.push(false);
      }
      state.held.add(source);
    } else {
      // A source that never pressed it releases nothing.
      if (!state.held.delete(source)) return;
      // Let go only once every source has.
      if (state.held.size > 0) return;
      state.taps[state.taps.length - 1] = true;
    }
    serviceTaps(button, state, handle.emulatedTimeUs());
  }

  function provision(identityJson: string): void {
    provisioner = createProvisioner(handle, identityJson, () => bootConsoleStart);
    lastState = { kind: "waiting" };
    provisionStartedAt = handle.totalSteps();
  }

  function setupOnScreen(): boolean {
    return skipper === null && setupOnScreenIn(consoleSince(handle, bootConsoleStart));
  }

  function skipSetup(): boolean {
    if (provisioner !== null || skipper !== null || !setupOnScreen()) return false;
    skipper = createSetupSkipper(handle, () => bootConsoleStart);
    lastSkip = { kind: "typing" };
    skipStartedAt = handle.totalSteps();
    return true;
  }

  function shake(): void {
    startMotion(shakeAcceleration);
  }

  function bump(): void {
    startMotion(bumpAcceleration);
  }

  function reset(): void {
    provisioner = null;
    lastState = null;
    skipper = null;
    lastSkip = null;
    motion = null;
    lastMotionAccel = null;
    dropTapsForReset();
    // A cold power-on starts an empty console.
    bootConsoleStart = 0;
    speed = 0;
    lastWallMs = null;
    // A cold power-on starts at 0 µs with no reboot to wait out.
    rebootDebtUs = 0;
    // The emulator's acceleration survives a reset; put it back at rest.
    handle.setAcceleration(...REST_ACCELERATION);
    handle.reset();
  }

  function dispose(): void {
    if (disposed) return;
    stop();
    disposed = true;
    handle.dispose();
  }

  return {
    start,
    stop,
    injectButton,
    reset,
    provision,
    provisionState: () => lastState,
    skipSetup,
    skipState: () => lastSkip,
    setupOnScreen,
    rebooting: () => rebootDebtUs > 0,
    shake,
    bump,
    speed: () => speed,
    isExitRequested: () => false,
    dispose,
  };
}

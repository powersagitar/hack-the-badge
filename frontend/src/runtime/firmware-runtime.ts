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
 * `./firmware-worker-client.ts` paints it. The press-hold deferral, the
 * shake schedule and the provisioner below therefore all live in the
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
 */
import type { ButtonName } from "../badge/input";
import type { FirmwareEmulatorHandle } from "../cpu/bridge";
import { createProvisioner, type ProvisionState, type Provisioner } from "./provisioner";

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
  /** Emulated time (µs) the frame advanced. */
  emulatedDeltaUs: number;
  /** The step cap stopped the frame short of its target (the debt is dropped). */
  capped: boolean;
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
  const target = frameTargetUs(before, wallDeltaMs);
  handle.runUntil(target, stepsPerFrame);
  const after = handle.emulatedTimeUs();
  present(handle.framebuffer(), handle.screenWidth, handle.screenHeight);
  // The wasm shim truncates the target to whole µs (`target_us as u64`), so
  // an uncapped frame with a fractional rAF delta ends at floor(target).
  return { emulatedDeltaUs: after - before, capped: after < Math.floor(target) };
}

export interface FirmwareRuntime {
  /** Begins the per-frame step+blit loop. Idempotent while already running. */
  start(): void;
  /**
   * Stops the loop and releases any button whose release was deferred (see
   * `PRESS_HOLD_US`). Does not dispose the underlying emulator handle.
   */
  stop(): void;
  /** Mirrors `lifecycle.ts`'s `AppRuntime.injectButton` signature. */
  injectButton(button: ButtonName, kind: "pressed" | "released"): void;
  /** Starts typing the provisioning flow for `identityJson`, replacing any in flight. */
  provision(identityJson: string): void;
  /** The current provisioning state, or `null` if none was started since the last reset. */
  provisionState(): ProvisionState | null;
  /** Shakes the emulated accelerometer (the onboarding's "Shake it!" page). */
  shake(): void;
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

  let provisioner: Provisioner | null = null;
  let lastState: ProvisionState | null = null;
  let provisionStartedAt = 0;
  /** Emulated time (µs) at which the current shake began, or `null`. */
  let shakeStartedAt: number | null = null;
  let lastShakeAccel: string | null = null;
  /**
   * Per button: emulated time (µs) of its latest press, and a release
   * deferred to an emulated time.
   */
  const pressedAt = new Map<ButtonName, number>();
  const pendingRelease = new Map<ButtonName, number>();

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

  function pollShake(now: number): void {
    if (shakeStartedAt === null) return;
    const a = shakeAcceleration(now - shakeStartedAt);
    if (a === null) {
      shakeStartedAt = null;
      lastShakeAccel = null;
      handle.setAcceleration(...REST_ACCELERATION);
      return;
    }
    const key = a.join(",");
    if (key === lastShakeAccel) return;
    lastShakeAccel = key;
    handle.setAcceleration(...a);
  }

  function pollReleases(now: number): void {
    for (const [button, at] of pendingRelease) {
      if (now < at) continue;
      pendingRelease.delete(button);
      pressedAt.delete(button);
      handle.setRawButton(RAW_SLOT_BY_BUTTON[button], false);
    }
  }

  function releaseAllPending(): void {
    // Don't strand a button whose deferred release we are dropping.
    for (const button of pendingRelease.keys()) {
      handle.setRawButton(RAW_SLOT_BY_BUTTON[button], false);
      pressedAt.delete(button);
    }
    pendingRelease.clear();
  }

  function loop(timeMs: number): void {
    // Guards against a callback that was already queued by the real
    // scheduler before stop() canceled it — a race a test's fake scheduler
    // can also simulate by invoking a "canceled" callback directly.
    if (frameHandle === null) return;
    const wallMs = now ? now() : timeMs;
    const delta = lastWallMs === null ? 0 : wallMs - lastWallMs;
    lastWallMs = wallMs;
    const r = stepFirmwareFrame(handle, present, delta, stepsPerFrame);
    if (delta > 0) {
      // The real frame time, not the clamped one: see "Pacing" above.
      const ratio = r.emulatedDeltaUs / (delta * 1000);
      speed = speed === 0 ? ratio : 0.9 * speed + 0.1 * ratio;
    }
    // The provisioner's timeout is in steps; holds and shakes in emulated time.
    pollProvisioner(handle.totalSteps());
    const nowUs = handle.emulatedTimeUs();
    pollShake(nowUs);
    pollReleases(nowUs);
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
    // With the loop stopped nothing would poll a deferred release.
    releaseAllPending();
  }

  function injectButton(button: ButtonName, kind: "pressed" | "released"): void {
    if (!isButtonName(button)) return;
    const slot = RAW_SLOT_BY_BUTTON[button];
    const now = handle.emulatedTimeUs();
    if (kind === "pressed") {
      pendingRelease.delete(button);
      pressedAt.set(button, now);
      handle.setRawButton(slot, true);
      return;
    }
    const since = pressedAt.get(button);
    if (since !== undefined && now - since < PRESS_HOLD_US) {
      pendingRelease.set(button, since + PRESS_HOLD_US);
      return;
    }
    pendingRelease.delete(button);
    pressedAt.delete(button);
    handle.setRawButton(slot, false);
  }

  function provision(identityJson: string): void {
    provisioner = createProvisioner(handle, identityJson);
    lastState = { kind: "waiting" };
    provisionStartedAt = handle.totalSteps();
  }

  function shake(): void {
    shakeStartedAt = handle.emulatedTimeUs();
    lastShakeAccel = null;
  }

  function reset(): void {
    provisioner = null;
    lastState = null;
    shakeStartedAt = null;
    lastShakeAccel = null;
    releaseAllPending();
    pressedAt.clear();
    speed = 0;
    lastWallMs = null;
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
    shake,
    speed: () => speed,
    isExitRequested: () => false,
    dispose,
  };
}

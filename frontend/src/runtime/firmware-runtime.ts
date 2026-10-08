/**
 * Real-firmware runtime: mirrors `src/runtime/lifecycle.ts`'s role, but for
 * the CPU-emulator side (Milestone 2) instead of the Lua sandbox
 * (Milestone 1). Owns a `FirmwareEmulatorHandle` (`src/cpu/bridge.ts`), drives
 * a `requestAnimationFrame` loop that steps the CPU a fixed instruction
 * budget and blits the reconstructed framebuffer (`src/render/framebuffer.ts`)
 * onto the same `<canvas>` `src/ui/shell.ts` already owns, and exposes an
 * `injectButton`-shaped API close enough to `lifecycle.ts`'s `AppRuntime` that
 * `shell.ts`'s existing button-pad/keyboard wiring can drive either runtime
 * without knowing which one it's holding.
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
 * ## Cycles-per-frame budget
 *
 * No real clock-rate emulation exists (a known, out-of-scope plan-level
 * risk — see the Task 6 brief). `DEFAULT_CYCLES_PER_FRAME` is a documented
 * guess, not a measurement:
 *
 * - The real badge runs its CPU at 80 MHz (per its own boot log; the
 *   emulator's ROM-stub `CPU_FREQ_MHZ` = 160 in `emulator-core/src/rom.rs`
 *   is a separate, documented flagged guess). At 60 fps that's ~1.33M
 *   instructions of real time per frame — more than is safe to spend
 *   synchronously inside one `requestAnimationFrame` callback.
 * - The prior session measured ~40M steps/s for `cargo test --release`
 *   (native, no WASM/JS-boundary overhead, no framebuffer copy/paint cost
 *   sharing the frame budget). WASM plus the per-`run()` `Vec<u16>` copy
 *   (`bridge.ts`'s buffer-aliasing-safety mechanism) and the RGB565->RGBA8888
 *   conversion `blitFramebuffer` does over all 76,800 pixels will all be
 *   slower than that native figure by an unmeasured but real amount.
 * - 500,000 steps/frame is chosen as a conservative fraction of a 16ms
 *   budget: enough to make visible progress every frame (roughly 3/8 of a
 *   real frame's worth of instructions, so boot/gameplay isn't rendered in
 *   slow motion relative to real time) while leaving comfortable headroom
 *   for the WASM call, the copy, and the paint even if the in-browser rate
 *   turns out to be a fraction of the native one. If profiling in a live
 *   browser (the controller's follow-up step, not this session's) shows a
 *   frame consistently taking too long, lowering this constant is the fix;
 *   nothing else in this module needs to change.
 */
import type { ButtonName } from "../badge/input";
import type { FirmwareEmulatorHandle } from "../cpu/bridge";
import { blitFramebuffer } from "../render/framebuffer";
import { createProvisioner, type ProvisionState, type Provisioner } from "./provisioner";

/** See this module's doc for the reasoning behind this number. */
export const DEFAULT_CYCLES_PER_FRAME = 500_000;

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
 * Minimum emulated steps a button stays pressed (100 ms at 16 MHz; same
 * value as `PRESS_HOLD_STEPS` in `emulator-core/tests/boot_progress.rs`).
 * A shorter click would be missed by the firmware's button polling.
 */
export const PRESS_HOLD_STEPS = 1_600_000;

/** Steps per shake half-period (`SHAKE_HALF_STEPS` in boot_progress.rs). */
export const SHAKE_HALF_STEPS = 1_000_000;
const SHAKE_HALF_PERIODS = 6;
const REST_ACCELERATION: [number, number, number] = [0, 0, 1000];

/**
 * The acceleration (mg) `elapsed` steps into a shake: six alternating
 * half-periods (+,-,+,...) of (s*2000, s*2000, 1000), then `null` (done;
 * the caller restores the rest acceleration). Mirrors `shake()` in
 * `emulator-core/tests/boot_progress.rs`.
 */
export function shakeAcceleration(elapsed: number): [number, number, number] | null {
  const k = Math.floor(elapsed / SHAKE_HALF_STEPS);
  if (k >= SHAKE_HALF_PERIODS) return null;
  const s = k % 2 === 0 ? 1 : -1;
  return [s * 2000, s * 2000, 1000];
}

export type FrameScheduler = (cb: (timeMs: number) => void) => number;
export type FrameCanceler = (handle: number) => void;

export interface FirmwareRuntimeOptions {
  /** Instruction steps to run per animation frame. Default: {@link DEFAULT_CYCLES_PER_FRAME}. */
  cyclesPerFrame?: number;
  /** Injectable for testing; defaults to the real `requestAnimationFrame`. */
  scheduleFrame?: FrameScheduler;
  /** Injectable for testing; defaults to the real `cancelAnimationFrame`. */
  cancelFrame?: FrameCanceler;
}

/**
 * Runs one frame's worth of work: step the CPU `cyclesPerFrame` instructions,
 * then blit its current framebuffer to `ctx`. Pure with respect to scheduling
 * (no `requestAnimationFrame` involved) so it's directly unit-testable
 * against a fake `FirmwareEmulatorHandle` and a fake canvas context, the same
 * way `test/canvas.test.ts`/`test/framebuffer.test.ts` already stub those.
 */
export function stepFirmwareFrame(
  handle: FirmwareEmulatorHandle,
  ctx: CanvasRenderingContext2D,
  cyclesPerFrame: number,
): void {
  handle.run(cyclesPerFrame);
  const pixels = handle.framebuffer();
  blitFramebuffer(ctx, pixels, handle.screenWidth, handle.screenHeight);
}

export interface FirmwareRuntime {
  /** Begins the per-frame step+blit loop. Idempotent while already running. */
  start(): void;
  /** Stops the loop. Does not dispose the underlying emulator handle. */
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
   * Real-firmware mode has no launcher/exit concept in this milestone (that's
   * a Lua-sandbox-mode-only affordance — see `badge.app.exit()` in
   * `lifecycle.ts`). Always `false`; exists so `shell.ts`/`main.ts` can treat
   * both runtimes uniformly without a type-level special case.
   */
  isExitRequested(): boolean;
  /** Stops the loop (if running) and releases the WASM-side emulator. */
  dispose(): void;
}

/**
 * Builds a {@link FirmwareRuntime} around an already-constructed
 * `FirmwareEmulatorHandle` (see `src/cpu/bridge.ts`'s `createFirmwareEmulator`)
 * and the `<canvas>` 2D context to paint into.
 */
export function createFirmwareRuntime(
  handle: FirmwareEmulatorHandle,
  ctx: CanvasRenderingContext2D,
  options: FirmwareRuntimeOptions = {},
): FirmwareRuntime {
  const cyclesPerFrame = options.cyclesPerFrame ?? DEFAULT_CYCLES_PER_FRAME;
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

  let frameHandle: number | null = null;
  let disposed = false;

  let provisioner: Provisioner | null = null;
  let lastState: ProvisionState | null = null;
  let provisionStartedAt = 0;
  /** Step at which the current shake began, or `null`. */
  let shakeStartedAt: number | null = null;
  let lastShakeAccel: string | null = null;
  /** Per button: step of its latest press, and a release deferred to a step. */
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

  function loop(): void {
    // Guards against a callback that was already queued by the real
    // scheduler before stop() canceled it — a race a test's fake scheduler
    // can also simulate by invoking a "canceled" callback directly.
    if (frameHandle === null) return;
    stepFirmwareFrame(handle, ctx, cyclesPerFrame);
    const now = handle.totalSteps();
    pollProvisioner(now);
    pollShake(now);
    pollReleases(now);
    if (frameHandle !== null) {
      frameHandle = scheduleFrame(loop);
    }
  }

  function start(): void {
    if (disposed) throw new Error("runtime/firmware-runtime: cannot start a disposed runtime");
    if (frameHandle !== null) return;
    frameHandle = scheduleFrame(loop);
  }

  function stop(): void {
    if (frameHandle !== null) {
      cancelFrame(frameHandle);
      frameHandle = null;
    }
  }

  function injectButton(button: ButtonName, kind: "pressed" | "released"): void {
    const slot = RAW_SLOT_BY_BUTTON[button];
    if (slot === undefined) return;
    const now = handle.totalSteps();
    if (kind === "pressed") {
      pendingRelease.delete(button);
      pressedAt.set(button, now);
      handle.setRawButton(slot, true);
      return;
    }
    const since = pressedAt.get(button);
    if (since !== undefined && now - since < PRESS_HOLD_STEPS) {
      pendingRelease.set(button, since + PRESS_HOLD_STEPS);
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
    shakeStartedAt = handle.totalSteps();
    lastShakeAccel = null;
  }

  function reset(): void {
    provisioner = null;
    lastState = null;
    shakeStartedAt = null;
    lastShakeAccel = null;
    // Don't strand a button whose deferred release we are dropping.
    for (const button of pendingRelease.keys()) handle.setRawButton(RAW_SLOT_BY_BUTTON[button], false);
    pressedAt.clear();
    pendingRelease.clear();
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
    isExitRequested: () => false,
    dispose,
  };
}

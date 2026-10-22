/**
 * Bridge-level walk helpers: boot_progress.rs's frame sampling and press
 * helpers over a `FirmwareEmulatorHandle`, and the walk's constants.
 * Shared by the WASM twin test (`test/cpu-wasm.test.ts`) and the redraw
 * benchmark (`./redraw-bench.ts`). Where the test used `expect`, these
 * throw an `Error`.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";

/**
 * FNV-1a (64-bit) over each pixel's little-endian bytes, as boot_progress.rs's
 * framebuffer_fnv1a. Computed in 32-bit halves (BigInt per byte is too slow
 * for the provisioned twin's ~1,400 samples): the prime is 2^40 + 0x1b3, so
 * h * prime = h * 0x1b3 + (lo << 40).
 */
export function fnv1a(fb: Uint16Array): bigint {
  let hi = 0xcbf29ce4;
  let lo = 0x84222325;
  const step = (b: number) => {
    lo = (lo ^ b) >>> 0;
    const p = lo * 0x1b3;
    hi = (Math.imul(hi, 0x1b3) + Math.floor(p / 0x1_0000_0000) + (lo << 8)) >>> 0;
    lo = p >>> 0;
  };
  for (const px of fb) {
    step(px & 0xff);
    step(px >> 8);
  }
  return (BigInt(hi) << 32n) | BigInt(lo);
}

/** boot_progress.rs's SPLASH_HASH (boots_to_first_real_frame). */
export const SPLASH_HASH = 0xfb305408a9e50e98n;
/** boot_progress.rs's FIRST_RUN_HASH (boots_to_first_run_screen). */
export const FIRST_RUN_HASH = 0x8c5027cec0f79490n;

/*
 * boot_progress.rs's constants, copied (same names and values; see that
 * file for where each was measured, re-measured in Milestone 6 and
 * re-pinned for v392 in Milestone 9).
 * The finish line walks OWN_ROLE from boot through provisioning and
 * onboarding to the launcher.
 */
export const FIRST_RUN_MAX_STEPS = 16_500_000;
export const BOOT_STABLE_FOR = 1_000_000;
export const AFTER_PRESS_STABLE_FOR = 8_000_000;
/** Button hold: 100 ms of emulated time (boot_progress.rs's PRESS_HOLD_US). */
export const PRESS_HOLD_US = 100_000;
/** Step cap for one runForUs call (boot_progress.rs's HOLD_MAX_STEPS). */
export const HOLD_MAX_STEPS = 40_000_000;
export const PROVISION_DEADLINE = 36_000_000;
export const ONBOARDING_RESPONSE_MARGIN = 16_000_000;
export const OWN_ROLE = "hacker";
/** ROLE_ROWS' hacker row: onboarding page 1, and the registered screen. */
export const OWN_ROLE_PAGE1_HASH = 0xbf9ccb7e80b44b1an;
export const OWN_ROLE_REGISTERED_HASH = 0xbd6d761308a7bf24n;
export const TO_SHAKE_PAGE: Array<[number, bigint]> = [
  [1, 0xe788a78193281b08n], // A: page 2 "This is you"
  [1, 0x38a9d040707cc4e8n], // A: page 3 "Try every button"
  [1, 0x8a6896e1b5c95d6bn], // A lit
  [2, 0x8b0b8358e2b5d6e8n], // B
  [7, 0xf38f281c4b05dabdn], // UP
  [5, 0xb1850f5c89312eb3n], // LEFT
  [6, 0x526668ba6df4b25en], // RIGHT
  [4, 0x3d7be2d172657413n], // DOWN
  [3, 0x1fd715c46f187c4cn], // HOME (captured on page 3)
  [0, 0xc2ebc5fd53532d8cn], // START: all eight done
  [1, 0xc1ad004ba5f77b78n], // A: page 4 "Lights"
  [1, 0x7598def30e838832n], // A: page 5 "Tap to unlock"
  [1, 0x4a54e5a40d669898n], // A: page 6 "Shake it!"
];
/** Shake half-period: 62.5 ms of emulated time (SHAKE_HALF_US). */
export const SHAKE_HALF_US = 62_500;
export const SHAKEN_AT_REST_HASH = 0xa80a279b127e1bb1n;
/** `null`: page 8, animated, not pinned (see boot_progress.rs). */
export const AFTER_SHAKE_PAGES: Array<[number, bigint | null]> = [
  [1, 0x110e8782bc4e5ff0n], // A: page 7 "Badge Connect"
  [1, null], // A: page 8 "Bump to connect", animated
  [1, 0x0831b6b675e6fe01n], // A: page 9 "HEAT SAFETY"
  [1, 0x21e9c7e1b05439e3n], // A: page 10 "Go explore"
  [1, 0x7dd45fd39362faacn], // A: page 11 "Badge rules", unticked
  [1, 0xe8c67af97ac7968en], // A: ticked, "Finished setup  Start"
];
export const ANIMATED_PAGE_STEPS = 9_000_000;
export const REGISTERED_RESPONSE_STEPS = AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
export const TO_LAUNCHER_SLOT = 3;
export const LAUNCHER_RESPONSE_STEPS = 25_000_000;
export const LAUNCHER_HASH = 0x94d7a7a85fa93ea3n;
/** boot_progress.rs's NAV_SLOT: DOWN moves the launcher's selection one row. */
export const NAV_SLOT = 4;
/** boot_progress.rs's LAUNCHER_AFTER_NAV_HASH: the launcher after one DOWN. */
export const LAUNCHER_AFTER_NAV_HASH = 0xa2b9da1f0e36757cn;

/** Throws if `fault` is set (the bridge's `lastInstructionFault`). */
function assertNoFault(fault: number | undefined): void {
  if (fault !== undefined) {
    throw new Error(`unmapped instruction fetch at 0x${fault.toString(16)}`);
  }
}

/**
 * boot_progress.rs's run_until_stable_frame (250,000-step chunks), over the
 * bridge. `onSample`, if given, sees each sample's hash right after it is
 * computed (the redraw benchmark times from it); it never changes the run.
 */
export function runUntilStable(
  handle: FirmwareEmulatorHandle,
  maxSteps: number,
  stableFor: number,
  skip: bigint[],
  onSample?: (hash: bigint) => void,
): bigint {
  let hash = fnv1a(handle.framebuffer());
  let since = handle.totalSteps();
  while (handle.totalSteps() < maxSteps) {
    assertNoFault(handle.run(250_000).lastInstructionFault);
    const fb = handle.framebuffer();
    const h = fnv1a(fb);
    onSample?.(h);
    if (h !== hash) {
      hash = h;
      since = handle.totalSteps();
    } else if (
      handle.totalSteps() - since >= stableFor &&
      fb.some((px) => px !== fb[0]) &&
      !skip.includes(h)
    ) {
      return hash;
    }
  }
  throw new Error(`no stable frame by ${maxSteps}; last hash ${hash.toString(16)}`);
}

/**
 * boot_progress.rs's run_for_us: runs `us` µs of emulated time, throwing on
 * a fault or if the time was not reached within HOLD_MAX_STEPS.
 */
export function runForUs(handle: FirmwareEmulatorHandle, us: number): void {
  const target = handle.emulatedTimeUs() + us;
  assertNoFault(handle.runUntil(target, HOLD_MAX_STEPS).lastInstructionFault);
  if (handle.emulatedTimeUs() < target) {
    throw new Error(`${us} us not reached within ${HOLD_MAX_STEPS} steps`);
  }
}

/** boot_progress.rs's press: holds `slot` for PRESS_HOLD_US of emulated time. */
export function press(handle: FirmwareEmulatorHandle, slot: number): void {
  handle.setRawButton(slot, true);
  runForUs(handle, PRESS_HOLD_US);
  handle.setRawButton(slot, false);
}

/** boot_progress.rs's press_and_settle (`onSample` as in runUntilStable). */
export function pressAndSettle(
  handle: FirmwareEmulatorHandle,
  slot: number,
  previous: bigint,
  onSample?: (hash: bigint) => void,
): bigint {
  press(handle, slot);
  const cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
  return runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous], onSample);
}

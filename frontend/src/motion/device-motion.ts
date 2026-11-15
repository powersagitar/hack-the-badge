/**
 * The phone's own motion sensor as a motion source (spec "Phone motion").
 * While it is on, the phone is the badge: DeviceMotion's
 * `accelerationIncludingGravity` is the phone's proper acceleration, the
 * same quantity the SC7A20H measures, so it maps straight to mg.
 *
 * - Phone axes (portrait, screen toward the visitor): x right, y up, z
 *   out of the screen. They map onto the board's right/up/out through an
 *   AxisMap, so an upright phone reads +1000 mg up and a flat one
 *   +1000 mg out. Each axis is clamped to ±CLAMP_MG.
 * - A peer's bump adds PEER_BUMP_MG on chip x to the readings for
 *   PEER_BUMP_MS.
 * - A jump of more than IMPACT_MG between consecutive raw readings (the
 *   pulse excluded, so a peer's bump is never echoed) fires onImpact, at
 *   most once per IMPACT_COOLDOWN_MS.
 *
 * Design calls: docs/milestone-14-decisions.md, "Phone motion (Task 10)".
 */
import { BOARD_AXES, type AxisMap } from "./axes";
import { clampRound } from "./sensor";
import type { Mg, MotionSource } from "./source";

export const STANDARD_GRAVITY = 9.80665;
export const PEER_BUMP_MG = 1000;
export const PEER_BUMP_MS = 100;
export const IMPACT_MG = 1500;
export const IMPACT_COOLDOWN_MS = 800;

const finite = (v: number) => (Number.isFinite(v) ? v : 0);

/** `a` is `accelerationIncludingGravity` in m/s², in the phone's axes. */
export function deviceMotionToMg(
  a: { x: number; y: number; z: number },
  signFix: 1 | -1,
  axes: AxisMap,
): Mg {
  const k = (1000 / STANDARD_GRAVITY) * signFix;
  const phone = { right: finite(a.x) * k, up: finite(a.y) * k, out: finite(a.z) * k };
  const out: Mg = [0, 0, 0];
  for (const key of ["right", "up", "out"] as const) {
    const [axis, sign] = axes[key];
    out[axis] += phone[key] * sign;
  }
  return [clampRound(out[0]), clampRound(out[1]), clampRound(out[2])];
}

/**
 * The sign that makes this platform's `accelerationIncludingGravity`
 * follow the W3C convention (proper acceleration: a flat, face-up phone
 * reads +Z, an upright one +Y, and tilting the right edge down drives X
 * negative). Ruling R21 (docs/milestone-14-decisions.md):
 *
 * - iOS and iPadOS: −1. On the iPhone checked (2026-11-14, the self-test's
 *   accelerometer page with no flip), every axis read inverted: face up
 *   Z ≈ −1000 mg, face down all positive, right edge down X positive,
 *   upright Y ≈ −1000 mg. The older-iOS quirk, still present. Every iOS
 *   browser is WebKit, so CriOS, FxiOS and the rest flip too.
 * - iPadOS's desktop-class UA says "Macintosh"; a Mac reporting touch
 *   points (`maxTouchPoints` > 1) is taken as an iPad.
 * - Everything else: 1. Android is untested (no real-device check yet).
 */
export function platformSignFix(ua: string, maxTouchPoints = 0): 1 | -1 {
  if (/iPhone|iPad|iPod/.test(ua)) return -1;
  if (/Macintosh/.test(ua) && maxTouchPoints > 1) return -1;
  return 1;
}

interface DeviceMotionEventStatic {
  requestPermission?: () => Promise<string>;
}

function deviceMotionEvent(): DeviceMotionEventStatic | undefined {
  const g = globalThis as { DeviceMotionEvent?: DeviceMotionEventStatic };
  return g.DeviceMotionEvent ?? undefined;
}

/**
 * Whether this page can get DeviceMotion at all: the API exists, in a
 * secure context (Chrome hides it outside one; iOS Safari keeps it but
 * never grants it). A desktop browser may pass and still never send an
 * event; it then simply never drives the badge.
 */
export function motionAvailable(): boolean {
  if (deviceMotionEvent() === undefined) return false;
  return (globalThis as { isSecureContext?: boolean }).isSecureContext !== false;
}

/**
 * Asks for motion access. Call it from a user gesture: iOS's
 * `DeviceMotionEvent.requestPermission()` rejects otherwise, which reads
 * as "denied". Where there is no requestPermission (Android, desktop),
 * the events need no permission: "granted".
 */
export async function requestMotionPermission(): Promise<"granted" | "denied" | "unsupported"> {
  if (!motionAvailable()) return "unsupported";
  const request = deviceMotionEvent()?.requestPermission;
  if (typeof request !== "function") return "granted";
  try {
    return (await request.call(deviceMotionEvent())) === "granted" ? "granted" : "denied";
  } catch {
    return "denied";
  }
}

interface MotionLike {
  accelerationIncludingGravity?: { x: number | null; y: number | null; z: number | null } | null;
}

function notify<T extends unknown[]>(listeners: Set<(...args: T) => void>, what: string, ...args: T) {
  for (const cb of [...listeners]) {
    try {
      cb(...args);
    } catch (err) {
      // A listener's failure never stops the readings.
      console.error(`device-motion: ${what} listener threw`, err);
    }
  }
}

/**
 * Listens for `devicemotion` on `win` between start() and stop(). Each
 * event with all three components finite becomes one sample for the sink
 * and the onMotion listeners (the lanyard follows the phone with them);
 * an event with a null or non-finite reading is ignored. `now` (ms) times
 * the peer pulse and the impact cooldown.
 */
export function createDeviceMotionSource(
  win: Pick<Window, "addEventListener" | "removeEventListener">,
  signFix: 1 | -1,
  now: () => number = () => performance.now(),
): MotionSource & { onMotion(cb: (mg: Mg) => void): () => void } {
  let sink: ((mg: Mg) => void) | null = null;
  let listening = false;
  let prevRaw: Mg | null = null;
  let pulseUntil = Number.NEGATIVE_INFINITY;
  let lastImpact = Number.NEGATIVE_INFINITY;
  const motionListeners = new Set<(mg: Mg) => void>();
  const impactListeners = new Set<() => void>();

  function onDeviceMotion(ev: Event): void {
    const a = (ev as MotionLike).accelerationIncludingGravity;
    if (!a) return;
    const { x, y, z } = a;
    if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) return;
    const raw = deviceMotionToMg({ x: x!, y: y!, z: z! }, signFix, BOARD_AXES);
    const t = now();
    if (prevRaw) {
      const jump = Math.hypot(raw[0] - prevRaw[0], raw[1] - prevRaw[1], raw[2] - prevRaw[2]);
      if (jump > IMPACT_MG && t - lastImpact > IMPACT_COOLDOWN_MS) {
        lastImpact = t;
        notify(impactListeners, "onImpact");
      }
    }
    prevRaw = raw;
    const mg: Mg = t < pulseUntil ? [clampRound(raw[0] + PEER_BUMP_MG), raw[1], raw[2]] : raw;
    sink?.(mg);
    notify(motionListeners, "onMotion", mg);
  }

  return {
    start(s) {
      sink = s;
      if (!listening) {
        win.addEventListener("devicemotion", onDeviceMotion);
        listening = true;
      }
    },
    stop() {
      sink = null;
      if (listening) {
        win.removeEventListener("devicemotion", onDeviceMotion);
        listening = false;
      }
      prevRaw = null;
    },
    peerBump() {
      pulseUntil = now() + PEER_BUMP_MS;
    },
    onImpact(cb) {
      impactListeners.add(cb);
      return () => {
        impactListeners.delete(cb);
      };
    },
    onMotion(cb) {
      motionListeners.add(cb);
      return () => {
        motionListeners.delete(cb);
      };
    },
  };
}

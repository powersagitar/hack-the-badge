/**
 * The pure physics -> mg mapping (spec "Physics (matter.js)", "The
 * sensor reading"). The reading is the badge's proper acceleration in the
 * board's frame:
 *
 * - gravity: a true 1 g (REST_MG), its reaction pointing up the screen,
 *   seen through the badge's tilt (not the simulation's px/s² value);
 * - motion: the simulated acceleration at the chip's spot (screen frame,
 *   px/s², y down), rotated into the board frame, times one gain.
 *
 * The badge hangs in the screen plane, so the face normal gets nothing
 * from motion or gravity. Each axis is clamped to the firmware's full
 * scale (`CTRL_REG4` = 0x80, FS ±2 g; `docs/milestone-5-decisions.md`
 * ACCEL) and rounded. Non-finite input reads as 0 (rest), never NaN.
 */
import type { AxisMap } from "./axes";
import type { Mg } from "./source";

export const REST_MG = 1000;
export const CLAMP_MG = 2000;

const finite = (v: number) => (Number.isFinite(v) ? v : 0);

function clampRound(v: number): number {
  const r = Math.round(Math.max(-CLAMP_MG, Math.min(CLAMP_MG, v)));
  return r === 0 ? 0 : r; // no -0
}

/**
 * `angleRad` is the badge's screen rotation (clockwise positive, as CSS
 * `rotate()`, 0 hanging upright). `accelPx` is the chip spot's
 * acceleration in screen px/s² (y down).
 */
export function sensorMg(
  angleRad: number,
  accelPx: { x: number; y: number },
  gainMgPerPxS2: number,
  axes: AxisMap,
): Mg {
  const a = finite(angleRad);
  const g = finite(gainMgPerPxS2);
  // Specific force in screen coords (y down), in mg: motion minus gravity;
  // gravity points down the screen, so its reaction reads -REST_MG on y.
  const fx = finite(accelPx.x) * g;
  const fy = finite(accelPx.y) * g - REST_MG;
  const c = Math.cos(a);
  const s = Math.sin(a);
  // The board's axes on screen: right = (cos, sin), up = (sin, -cos).
  const board = { right: fx * c + fy * s, up: fx * s - fy * c, out: 0 };
  const out: Mg = [0, 0, 0];
  for (const k of ["right", "up", "out"] as const) {
    const [axis, sign] = axes[k];
    out[axis] += board[k] * sign;
  }
  return [clampRound(out[0]), clampRound(out[1]), clampRound(out[2])];
}

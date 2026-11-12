/**
 * Which SC7A20H axis each board axis lands on, and with what sign.
 *
 * `right` and `up` are the board's in-plane axes as the visitor sees the
 * badge (screen right, and from the bottom edge toward the lanyard), and
 * `out` is the face normal toward the visitor.
 *
 * BOARD_AXES is UNVERIFIED (the human partner, 2026-11-12, "assume,
 * verify later"): v392 reads single axes in several apps (Level, Tilt
 * Maze, Accel, Diag, Hopper, 6-7 Counter, Lua `badge.sensor`), and the
 * photo does not show the chip's pin 1 legibly. The human partner checks
 * the signs on the physical badge in Task 14's browser check. Evidence:
 * `docs/milestone-14-decisions.md`, "Accelerometer axes (Task 9)".
 */
export type AxisIndex = 0 | 1 | 2;
export type AxisSign = 1 | -1;

export interface AxisMap {
  right: [axis: AxisIndex, sign: AxisSign];
  up: [axis: AxisIndex, sign: AxisSign];
  out: [axis: AxisIndex, sign: AxisSign];
}

export const BOARD_AXES: AxisMap = { right: [0, 1], up: [1, 1], out: [2, 1] };

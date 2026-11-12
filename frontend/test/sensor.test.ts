import { describe, expect, test } from "bun:test";
import { BOARD_AXES, type AxisMap } from "../src/motion/axes";
import { CLAMP_MG, REST_MG, sensorMg } from "../src/motion/sensor";

const G = 0.05;
const mag = (v: number[]) => Math.hypot(...v);

describe("sensorMg", () => {
  test("rest hanging reads 1000 mg along up", () => {
    const mg = sensorMg(0, { x: 0, y: 0 }, G, BOARD_AXES);
    const [upAxis, upSign] = BOARD_AXES.up;
    expect(Math.abs(mg[upAxis] * upSign - REST_MG)).toBeLessThanOrEqual(1);
    for (let a = 0; a < 3; a++) if (a !== upAxis) expect(mg[a]).toBe(0);
    expect(Math.abs(mag(mg) - 1000)).toBeLessThanOrEqual(1);
  });

  test("tilted 90° moves gravity to right", () => {
    // Rotated a quarter turn clockwise on screen: the board's right edge
    // points down, so the 1 g reaction (pointing up the screen) reads on
    // the board's -right.
    const mg = sensorMg(Math.PI / 2, { x: 0, y: 0 }, G, BOARD_AXES);
    const [rAxis, rSign] = BOARD_AXES.right;
    expect(mg[rAxis] * rSign).toBe(-1000);
    for (let a = 0; a < 3; a++) if (a !== rAxis) expect(Math.abs(mg[a])).toBeLessThanOrEqual(1);
  });

  test("linear motion adds gain-scaled", () => {
    const rest = sensorMg(0, { x: 0, y: 0 }, G, BOARD_AXES);
    const moving = sensorMg(0, { x: 1000, y: 0 }, G, BOARD_AXES);
    const [rAxis, rSign] = BOARD_AXES.right;
    // Accelerating toward the board's right reads +right (1000 px/s² × G).
    expect((moving[rAxis] - rest[rAxis]) * rSign).toBe(1000 * G);
    expect(moving[BOARD_AXES.up[0]]).toBe(rest[BOARD_AXES.up[0]]);
    // Accelerating down the screen at angle 0 lowers the up reading.
    const falling = sensorMg(0, { x: 0, y: 2400 }, G, BOARD_AXES);
    expect(falling[BOARD_AXES.up[0]] * BOARD_AXES.up[1]).toBe(1000 - 2400 * G);
  });

  test("the face-normal axis gets nothing from motion", () => {
    const mg = sensorMg(0.7, { x: 30000, y: -20000 }, G, BOARD_AXES);
    expect(mg[BOARD_AXES.out[0]]).toBe(0);
  });

  test("clamps each axis to ±2000", () => {
    const mg = sensorMg(0, { x: 1e6, y: 1e6 }, G, BOARD_AXES);
    expect(mg[BOARD_AXES.right[0]] * BOARD_AXES.right[1]).toBe(CLAMP_MG);
    expect(mg[BOARD_AXES.up[0]] * BOARD_AXES.up[1]).toBe(-CLAMP_MG);
    const neg = sensorMg(0, { x: -1e6, y: -1e6 }, G, BOARD_AXES);
    expect(neg[BOARD_AXES.right[0]] * BOARD_AXES.right[1]).toBe(-CLAMP_MG);
    expect(neg[BOARD_AXES.up[0]] * BOARD_AXES.up[1]).toBe(CLAMP_MG);
  });

  test("non-finite input reads as rest, never NaN", () => {
    for (const mg of [
      sensorMg(Number.NaN, { x: 0, y: 0 }, G, BOARD_AXES),
      sensorMg(0, { x: Number.POSITIVE_INFINITY, y: Number.NaN }, G, BOARD_AXES),
    ]) {
      for (const v of mg) expect(Number.isFinite(v)).toBe(true);
      expect(mg).toEqual(sensorMg(0, { x: 0, y: 0 }, G, BOARD_AXES));
    }
  });

  test("the axis map routes and signs each board axis", () => {
    const flipped: AxisMap = { right: [1, -1], up: [2, 1], out: [0, 1] };
    const mg = sensorMg(0, { x: 1000, y: 0 }, G, flipped);
    expect(mg).toEqual([0, -50, 1000]);
  });

  test("BOARD_AXES is right +x, up +y, out +z (unverified, Milestone 14)", () => {
    expect(BOARD_AXES).toEqual({ right: [0, 1], up: [1, 1], out: [2, 1] });
  });
});

import { describe, expect, test } from "bun:test";
import { createLanyard } from "../src/motion/lanyard";
import { BOARD_AXES } from "../src/motion/axes";
import { CLAMP_MG, REST_MG } from "../src/motion/sensor";
import type { Mg } from "../src/motion/source";

const FRAME_MS = 1000 / 60;
const REST: Mg = [0, 0, 0];
REST[BOARD_AXES.up[0]] = REST_MG * BOARD_AXES.up[1];

const dist = (a: Mg, b: Mg) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);

/** A desktop lanyard whose sink records every reading. */
function rig(width = 1000, height = 780, layout: "desktop" | "mobile" = "desktop") {
  const l = createLanyard({ width, height, layout });
  const readings: Mg[] = [];
  l.start((mg) => readings.push(mg));
  const run = (ms: number) => {
    for (let t = 0; t < ms; t += FRAME_MS) l.step(FRAME_MS);
  };
  return { l, readings, run };
}

/** The badge's centre, from pose(). */
function centre(l: ReturnType<typeof createLanyard>) {
  const b = l.pose().badge;
  return { x: b.x, y: b.y };
}

describe("lanyard", () => {
  test("settles to rest", () => {
    const { readings, run } = rig();
    run(5000);
    expect(dist(readings.at(-1)!, REST)).toBeLessThan(30);
  });

  test("pose has about 10 rope points from the anchor to the badge", () => {
    const { l, run } = rig();
    run(1000);
    const p = l.pose();
    expect(p.rope.length).toBeGreaterThanOrEqual(10);
    expect(p.rope[0]).toEqual({ x: 500, y: -10 });
    // Desktop clamp: bh = max(280, min(560, H-230, (W-48)*1513/974)).
    expect(p.badge.h).toBeCloseTo(550, 5); // min(560, 780-230, 952*1513/974)
    expect(p.badge.w).toBeCloseTo((550 * 974) / 1513, 5);
    expect(Math.abs(p.badge.angle)).toBeLessThan(0.05);
  });

  test("mobile layout uses its own clamp", () => {
    const l = createLanyard({ width: 390, height: 640, layout: "mobile" });
    // max(260, min(560, 640-190, (390-40)*1513/974)) = 450
    expect(l.pose().badge.h).toBeCloseTo(450, 5);
  });

  test("drag ends on pointercancel and lostpointercapture", () => {
    // The page calls release() for pointerup, pointercancel,
    // lostpointercapture and a hidden page; release() returns to free
    // swinging, so the reading goes back to rest.
    const { l, readings, run } = rig();
    run(500);
    const c = centre(l);
    expect(l.grab(c)).toBe(true);
    for (let i = 0; i < 60; i++) {
      l.drag({ x: c.x + 150 * Math.sin(i / 4), y: c.y - 40 });
      run(FRAME_MS);
    }
    expect(dist(readings.at(-1)!, REST)).toBeGreaterThan(30);
    l.release();
    run(5000);
    expect(dist(readings.at(-1)!, REST)).toBeLessThan(30);
  });

  test("grab misses outside the badge", () => {
    const { l } = rig();
    expect(l.grab({ x: 5, y: 5 })).toBe(false);
  });

  test("relayout mid-swing stays finite and clamped", () => {
    const { l, readings, run } = rig();
    run(300);
    const c = centre(l);
    l.grab(c);
    for (let i = 0; i < 40; i++) {
      l.drag({ x: c.x + (i % 2 ? 300 : -300), y: c.y });
      run(FRAME_MS);
    }
    l.relayout(390, 640);
    const from = readings.length;
    run(2000);
    expect(readings.length - from).toBeGreaterThan(100);
    for (const mg of readings.slice(from)) {
      for (const v of mg) {
        expect(Number.isFinite(v)).toBe(true);
        expect(Math.abs(v)).toBeLessThanOrEqual(CLAMP_MG);
      }
    }
    const b = l.pose().badge;
    for (const v of [b.x, b.y, b.angle]) expect(Number.isFinite(v)).toBe(true);
    expect(b.x).toBeGreaterThan(0);
    expect(b.x).toBeLessThan(390);
  });

  test("knock fires onImpact once", () => {
    const { l, readings, run } = rig();
    run(1000);
    let impacts = 0;
    const off = l.onImpact(() => impacts++);
    const from = readings.length;
    l.impulse("knock");
    run(800);
    expect(impacts).toBe(1);
    // Connect's bump: > 400 mg between samples 40–70 ms apart.
    const window = readings.slice(from);
    let best = 0;
    for (let i = 0; i < window.length; i++) {
      for (let j = i + 3; j < window.length && (j - i) * FRAME_MS <= 70; j++) {
        if ((j - i) * FRAME_MS >= 40) best = Math.max(best, dist(window[i], window[j]));
      }
    }
    expect(best).toBeGreaterThan(400);
    off();
    l.impulse("knock");
    run(800);
    expect(impacts).toBe(1);
  });

  test("knock fires onImpact on the mobile stage too", () => {
    const { l, run } = rig(390, 640, "mobile");
    run(1000);
    let impacts = 0;
    l.onImpact(() => impacts++);
    l.impulse("knock");
    run(800);
    expect(impacts).toBe(1);
  });

  test("a slow push into the wall is not an impact", () => {
    const { l, run } = rig();
    run(1000);
    let impacts = 0;
    l.onImpact(() => impacts++);
    const c = centre(l);
    l.grab(c);
    for (let i = 1; i <= 90; i++) {
      l.drag({ x: c.x + ((1040 - c.x) * i) / 90, y: c.y });
      run(FRAME_MS);
    }
    l.release();
    run(3000);
    expect(impacts).toBe(0);
  });

  test("gentle swinging fires no impact", () => {
    const { l, run } = rig();
    run(300);
    let impacts = 0;
    l.onImpact(() => impacts++);
    const c = centre(l);
    l.grab(c);
    for (let i = 0; i < 120; i++) {
      l.drag({ x: c.x + 20 * Math.sin((2 * Math.PI * i) / 60), y: c.y });
      run(FRAME_MS);
    }
    l.release();
    run(2000);
    expect(impacts).toBe(0);
  });

  test("shake and peerBump move the reading; stop() silences the sink", () => {
    const { l, readings, run } = rig();
    run(1000);
    l.impulse("shake");
    run(600);
    const peak = Math.max(...readings.slice(-36).map((m) => dist(m, REST)));
    expect(peak).toBeGreaterThan(400);
    run(4000);
    const from = readings.length;
    l.peerBump();
    run(200);
    const bump = Math.max(...readings.slice(from).map((m) => dist(m, REST)));
    expect(bump).toBeGreaterThan(400);
    l.stop();
    const n = readings.length;
    run(200);
    expect(readings.length).toBe(n);
  });
});

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
    // Desktop clamp: bh = max(280, min(560, H-230, (W-48)*1523/974)).
    expect(p.badge.h).toBeCloseTo(550, 5); // min(560, 780-230, 952*1523/974)
    expect(p.badge.w).toBeCloseTo((550 * 974) / 1523, 5);
    expect(Math.abs(p.badge.angle)).toBeLessThan(0.05);
  });

  test("mobile layout uses its own clamp", () => {
    const l = createLanyard({ width: 390, height: 640, layout: "mobile" });
    // max(260, min(560, 640-190, (390-40)*1523/974)) = 450
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

  test("the body has the board's rounded corners, the bottom-right the largest", () => {
    // A point 1.5% of the width in from both edges is inside the photo's
    // rectangle but outside every corner's arc (the smallest radius is
    // 9.3%: the arc is 2.7% in along the diagonal).
    const { l, run } = rig();
    run(1000);
    const { x, y, w, h, angle } = l.pose().badge;
    expect(Math.abs(angle)).toBeLessThan(0.01);
    const d = 0.015 * w;
    for (const [sx, sy] of [
      [-1, -1],
      [1, -1],
      [1, 1],
      [-1, 1],
    ]) {
      expect(l.grab({ x: x + sx * (w / 2 - d), y: y + sy * (h / 2 - d) })).toBe(false);
    }
    // 8% of the width in along the bottom edge is board at the bottom-left
    // (radius 9.6%) but not yet at the bottom-right (radius 18.9%).
    const along = 0.08 * w;
    expect(l.grab({ x: x - w / 2 + along, y: y + h / 2 - d })).toBe(true);
    l.release();
    expect(l.grab({ x: x + w / 2 - along, y: y + h / 2 - d })).toBe(false);
  });

  test("the pose's centre is the photo rectangle's centre, despite the unequal corners", () => {
    // Half a pixel inside each straight edge's middle is board, half a
    // pixel outside is not: the body sits on the rectangle the photo is drawn in.
    const { l, run } = rig();
    run(1000);
    const { x, y, w, h } = l.pose().badge;
    for (const [dx, dy] of [
      [w / 2, 0],
      [-w / 2, 0],
      [0, h / 2],
      [0, -h / 2],
    ]) {
      const ux = Math.sign(dx);
      const uy = Math.sign(dy);
      expect(l.grab({ x: x + dx - ux * 0.5, y: y + dy - uy * 0.5 })).toBe(true);
      l.release();
      expect(l.grab({ x: x + dx + ux * 0.5, y: y + dy + uy * 0.5 })).toBe(false);
    }
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

  /** The largest vector change between readings 40–70 ms apart (Connect's bump). */
  function bumpDelta(rs: Mg[]): number {
    let best = 0;
    for (let i = 0; i < rs.length; i++) {
      for (let j = i + 3; j < rs.length && (j - i) * FRAME_MS <= 70; j++) {
        if ((j - i) * FRAME_MS >= 40) best = Math.max(best, dist(rs[i], rs[j]));
      }
    }
    return best;
  }

  for (const [W, H, layout] of [
    [1000, 780, "desktop"],
    [820, 780, "desktop"],
    [390, 640, "mobile"],
    [430, 800, "mobile"],
  ] as const) {
    for (const ms of [100, 250]) {
      test(`a fast drag into the wall fires onImpact once and reads >400 mg 40–70 ms apart (${W}×${H}, ${ms} ms)`, () => {
        const { l, readings, run } = rig(W, H, layout);
        run(2000);
        let impacts = 0;
        l.onImpact(() => impacts++);
        const from = readings.length;
        const c = centre(l);
        expect(l.grab(c)).toBe(true);
        const frames = Math.round(ms / FRAME_MS);
        for (let i = 1; i <= frames; i++) {
          l.drag({ x: c.x + ((W + 40 - c.x) * i) / frames, y: c.y });
          run(FRAME_MS);
        }
        run(150); // still holding past the wall
        l.release();
        run(800 - ms - 150);
        expect(impacts).toBe(1);
        expect(bumpDelta(readings.slice(from))).toBeGreaterThan(400);
      });
    }
  }

  test("relayout at the same badge size keeps the swing", () => {
    // 1000×780 and 1100×780 both give bh = 550: the design moves the
    // anchor and walls instead of resetting.
    const { l, readings, run } = rig();
    run(300);
    const c = centre(l);
    l.grab(c);
    for (let i = 1; i <= 20; i++) {
      l.drag({ x: c.x + 8 * i, y: c.y });
      run(FRAME_MS);
    }
    l.release();
    run(100);
    const before = l.pose().badge;
    expect(Math.abs(before.angle)).toBeGreaterThan(0.02);
    l.relayout(1100, 780);
    const after = l.pose();
    expect(after.rope[0]).toEqual({ x: 550, y: -10 });
    expect(after.badge.h).toBeCloseTo(550, 5);
    expect(after.badge.angle).toBeCloseTo(before.angle, 6);
    expect(after.badge.x).toBeCloseTo(before.x + 50, 6);
    const from = readings.length;
    run(2000);
    for (const mg of readings.slice(from)) {
      for (const v of mg) {
        expect(Number.isFinite(v)).toBe(true);
        expect(Math.abs(v)).toBeLessThanOrEqual(CLAMP_MG);
      }
    }
  });

  test("a shake does not cancel a peer bump in flight", () => {
    // Both rigs hold the badge 60 px left of centre, then let go: the
    // bump and the shake's first half both push right (away from the
    // nearer wall). A small gain keeps the readings off the ±2000 clamp.
    const make = () => {
      const l = createLanyard({ width: 1000, height: 780, layout: "desktop", gain: 0.005 });
      const rs: Mg[] = [];
      l.start((mg) => rs.push(mg));
      const run = (ms: number) => {
        for (let t = 0; t < ms; t += FRAME_MS) l.step(FRAME_MS);
      };
      run(1000);
      const c = centre(l);
      l.grab(c);
      l.drag({ x: c.x - 60, y: c.y });
      run(2000);
      l.release();
      return { l, rs, run };
    };
    const both = make();
    const shakeOnly = make();
    both.l.peerBump();
    for (const r of [both, shakeOnly]) r.run(2 * FRAME_MS);
    for (const r of [both, shakeOnly]) {
      r.l.impulse("shake");
      r.run(3 * FRAME_MS);
    }
    const [axis, sign] = BOARD_AXES.right;
    const withBump = both.rs.at(-1)![axis] * sign;
    const alone = shakeOnly.rs.at(-1)![axis] * sign;
    expect(withBump).toBeLessThan(CLAMP_MG);
    // The bump's 30,000 px/s² on top of the shake's 40,000: about 150 mg more.
    expect(withBump - alone).toBeGreaterThan(100);
  });

  test("a throwing onImpact listener never aborts step()", () => {
    const { l, readings, run } = rig();
    run(1000);
    let after = 0;
    l.onImpact(() => {
      throw new Error("listener bug");
    });
    l.onImpact(() => after++);
    const errors = console.error;
    console.error = () => {};
    try {
      l.impulse("knock");
      const n = readings.length;
      expect(() => run(800)).not.toThrow();
      expect(readings.length - n).toBeGreaterThan(40);
    } finally {
      console.error = errors;
    }
    expect(after).toBe(1);
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

  test("a fast upward fling never swings the badge over its anchor (R15)", () => {
    // Before the ceiling, an up-and-sideways fling (on 1000×780, 600 px
    // right and 300 px up over 2 frames, or over 16 frames at 300 px)
    // swung the badge over the anchor, where it stayed upside down,
    // leaning on a wall above the stage, until "Reset swing".
    const cases: Array<[number, number, "desktop" | "mobile", number, number, number]> = [
      [1000, 780, "desktop", 600, -300, 2],
      [1000, 780, "desktop", 300, -300, 16],
      [857, 778, "desktop", 960, -300, 8],
      [857, 778, "desktop", -960, -600, 4],
      [600, 780, "desktop", 1500, -600, 2],
      [390, 640, "mobile", 600, -600, 2],
    ];
    for (const [W, H, layout, dx, dy, frames] of cases) {
      const { l, run } = rig(W, H, layout);
      run(500);
      const c = centre(l);
      l.grab(c);
      for (let i = 1; i <= frames; i++) {
        l.drag({ x: c.x + (dx * i) / frames, y: c.y + (dy * i) / frames });
        run(FRAME_MS);
      }
      l.release();
      for (let t = 0; t < 5000; t += FRAME_MS) {
        run(FRAME_MS);
        const p = l.pose();
        // Never inverted: the centre stays below the ring it hangs from.
        expect(p.badge.y).toBeGreaterThan(p.rope.at(-1)!.y);
      }
      const p = l.pose();
      expect(Math.abs(p.badge.angle)).toBeLessThan(0.3);
      expect(Math.abs(p.badge.x - W / 2)).toBeLessThan(40);
    }
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

  test("following the phone sways the badge, and stays finite and clamped", () => {
    const { l, readings, run } = rig();
    run(1000);
    const x0 = centre(l).x;
    // Ruling R19: the reading leans right (the phone tilted right, as
    // iOS reports it): the drawn badge swings right, with the tilt.
    const lean: Mg = [...REST];
    lean[BOARD_AXES.right[0]] = 500 * BOARD_AXES.right[1];
    l.follow(lean);
    run(600);
    expect(centre(l).x).toBeGreaterThan(x0 + 40);
    // And left for a left tilt.
    lean[BOARD_AXES.right[0]] = -500 * BOARD_AXES.right[1];
    l.follow(lean);
    run(1500);
    expect(centre(l).x).toBeLessThan(x0 - 40);
    // Upside down: the badge still hangs below the anchor, on the stage.
    const upsideDown: Mg = [0, 0, 0];
    upsideDown[BOARD_AXES.up[0]] = -2000 * BOARD_AXES.up[1];
    l.follow(upsideDown);
    run(1500);
    expect(centre(l).y).toBeGreaterThan(l.pose().rope[0].y);
    // A violent, jittery phone (and a NaN reading) never explodes the world.
    for (let i = 0; i < 120; i++) {
      l.follow(i % 7 === 0 ? [Number.NaN, 0, 0] : [i % 2 ? 2000 : -2000, i % 3 ? 2000 : -2000, 0]);
      l.step(FRAME_MS);
    }
    for (const mg of readings) for (const v of mg) expect(Math.abs(v)).toBeLessThanOrEqual(CLAMP_MG);
    const p = l.pose();
    expect(Number.isFinite(p.badge.x + p.badge.y + p.badge.angle)).toBe(true);
    // Not following: it settles back under the anchor.
    l.follow(null);
    run(8000);
    expect(Math.abs(centre(l).x - x0)).toBeLessThan(10);
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

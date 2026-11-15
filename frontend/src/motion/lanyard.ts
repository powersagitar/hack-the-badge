/**
 * The badge on its lanyard, in matter.js (spec "Physics (matter.js)").
 * Headless: no DOM, so the tests run it directly; the page (Task 11)
 * draws pose() and forwards pointer events to grab/drag/release.
 *
 * - A rope of ROPE_SEGMENTS jointed point masses from a fixed anchor at
 *   (W/2, -10) to the badge's ring, and the badge as one rigid body
 *   pinned at its ring point, `clip` above its top edge (the design's
 *   `dPC = clip + bh/2`). The stage's side walls are static colliders.
 * - Fixed 1/120 s substeps with gravity 2400 px/s² (the design's look),
 *   decoupled from the frame rate; a step() owes at most MAX_SUBSTEPS
 *   and drops the rest (a hidden tab does not replay its backlog).
 * - The sensor: the chip spot's acceleration from velocity differences
 *   across substeps (so the rotational terms are in it), averaged over
 *   the substeps of one step() call, through sensorMg with
 *   GAIN_MG_PER_PX_S2. The sink gets one sample per step() that ran a
 *   substep.
 * - Impacts: a new contact (a bounds edge reaching its wall after
 *   WALL_REARM_MS away) whose edge approached faster than IMPACT_PX_S
 *   fires onImpact, at most once per IMPACT_COOLDOWN_MS of simulated
 *   time (the design's `detect()`; see wallCheck).
 * - Following the phone (follow()): while phone motion drives the
 *   sensor, the phone's in-plane reading is the anchor's (the hand's)
 *   acceleration, felt by the rope and the badge as the equivalent
 *   inertial force, so the drawn badge sways with the phone; sideways
 *   it swings with the tilt instead (Ruling R19, deliberately
 *   non-physical).
 * - The stage has a ceiling at y = 0 (a collider and the bounds clamp),
 *   so a fast fling can't swing the badge over its anchor (R15's fix).
 *
 * Constants and their tuning: docs/milestone-14-decisions.md, "Lanyard
 * physics and the sensor (Task 9)".
 */
import Matter from "matter-js";
import { BOARD_CORNERS, PHOTO } from "../ui/badge-geometry";
import { BOARD_AXES, type AxisMap } from "./axes";
import { REST_MG, sensorMg } from "./sensor";
import type { Mg, MotionSource } from "./source";

const { Bodies, Body, Composite, Constraint, Engine, Vertices } = Matter;

/** The photo's size (`PHOTO`), for the aspect ratio. */
export const IMG_W = PHOTO.w;
export const IMG_H = PHOTO.h;
export const ROPE_SEGMENTS = 10;
export const GRAVITY_PX_S2 = 2400;
export const STEP_MS = 1000 / 120;
const STEP_S = STEP_MS / 1000;
/** matter.js velocities are px (or rad) per 1000/60 ms; times this is per second. */
const PER_SECOND = 60;
/** At most 100 ms of simulation per step() call. */
export const MAX_SUBSTEPS = 12;
/**
 * mg per px/s² of simulated motion (Step 6's tuning against the real
 * firmware's "Shake it!" page; see the decisions doc).
 */
export const GAIN_MG_PER_PX_S2 = 0.2;
/** Normal speed at a wall that counts as an impact (design: 7 px/step at 120 Hz). */
export const IMPACT_PX_S = 840;
export const IMPACT_COOLDOWN_MS = 800;
/**
 * The accelerometer's spot on the board, as fractions of the photo
 * (x from the left, y from the top): the centre of the 2×2 mm LGA-12
 * below the ESP32-C3 module, the board's only such part (located on the
 * scan by its side pads, 4 a side; Task 11c).
 */
export const CHIP_SPOT = { x: 0.406, y: 0.6234 };

/** A sideways push: the "hand" accelerates the badge at `ax` px/s². */
interface Push {
  from: number;
  to: number;
  ax: number;
  /** Part of impulse("shake")'s train (a new shake replaces only these). */
  shake?: boolean;
}

const SHAKE_HALF_MS = 80;
const SHAKE_HALVES = 6;
const SHAKE_PX_S2 = 40_000;
const KNOCK_PX_S2 = 30_000;
const KNOCK_MAX_MS = 400;
const PEER_BUMP_MS = 100;
const PEER_BUMP_PX_S2 = 30_000;
/** No outgoing impact right after a peer's bump: never echo it back. */
const PEER_BUMP_QUIET_MS = 1000;
/** Following the phone: px/s² per mg, so 1 g is the simulation's gravity. */
const FOLLOW_PX_S2_PER_MG = GRAVITY_PX_S2 / REST_MG;
/**
 * Following the phone: the most the reading may lift against gravity
 * (px/s²). A phone held flat or upside down leaves a quarter g down, so
 * the badge still hangs below its anchor, on the stage.
 */
const FOLLOW_MAX_LIFT_PX_S2 = 0.75 * GRAVITY_PX_S2;
/** A runaway (or NaN) body speed; the world is rebuilt at rest past it. */
const RUNAWAY_PX_S = 50_000;
/**
 * How far past a wall (and past the rope's reach) the grip may pull: the
 * wall collider, not the grip, stops a fling, so it arrives at speed.
 */
const GRIP_OVERREACH_PX = 40;
/** Silent substeps at (re)build, so the world starts at rest (2 s). */
const SETTLE_SUBSTEPS = 240;
/** An edge this close to its wall touches it. */
const TOUCH_EPS_PX = 1;
/** Time away from a wall before touching it again starts a new contact. */
const WALL_REARM_MS = 50;
/** Speed caps, as the design's Verlet (60 px per 1/120 s step). */
const MAX_SPEED_PX_S = 7200;
const MAX_SPIN_RAD_S = 40;

export interface Point {
  x: number;
  y: number;
}

export interface Pose {
  rope: Point[];
  /** `clip`: the ring's height above the badge's top edge (px), as the physics pins it. */
  badge: { x: number; y: number; angle: number; w: number; h: number; clip: number };
}

export interface Lanyard {
  step(dtMs: number): void;
  relayout(width: number, height: number): void;
  pose(): Pose;
  /** Starts a drag if `p` is on the badge body; returns whether it did. */
  grab(p: Point): boolean;
  drag(p: Point): void;
  /** Ends a drag (pointerup, pointercancel, lostpointercapture, hidden). */
  release(): void;
  impulse(kind: "shake" | "knock"): void;
  /** The latest sensor sample. */
  reading(): Mg;
  /** Re-centres the badge, at rest ("Reset swing"). */
  reset(): void;
  /**
   * Follows the phone: `mg` (the phone's reading, in the chip's axes) is
   * the anchor's motion until the next call; `null` stops following.
   */
  follow(mg: Mg | null): void;
}

export interface LanyardOptions {
  width: number;
  height: number;
  layout: "desktop" | "mobile";
  gain?: number;
  axes?: AxisMap;
}

/** The design's layout(): the badge height and the rope length. */
export function lanyardGeometry(width: number, height: number, layout: "desktop" | "mobile") {
  const W = Math.max(1, Number.isFinite(width) ? width : 1);
  const H = Math.max(1, Number.isFinite(height) ? height : 1);
  const bh =
    layout === "desktop"
      ? Math.max(280, Math.min(560, H - 230, ((W - 48) * IMG_H) / IMG_W))
      : Math.max(260, Math.min(560, H - 190, ((W - 40) * IMG_H) / IMG_W));
  const bw = (bh * IMG_W) / IMG_H;
  const clip = Math.round(bw * 0.2);
  const anchor = { x: W / 2, y: -10 };
  const ropeLen =
    layout === "desktop"
      ? Math.max(60, H - 96 - bh - clip - anchor.y)
      : Math.max(50, H - 84 - bh - clip - anchor.y);
  return { W, H, bw, bh, clip, anchor, ropeLen };
}

type Geometry = ReturnType<typeof lanyardGeometry>;

export function createLanyard(opts: LanyardOptions): Lanyard & MotionSource {
  const gain = opts.gain ?? GAIN_MG_PER_PX_S2;
  const axes = opts.axes ?? BOARD_AXES;
  const layout = opts.layout;

  let geo: Geometry;
  let engine: Matter.Engine;
  let badge: Matter.Body;
  let nodes: Matter.Body[];
  /** The rope's links, the first one anchored; and the two wall bodies. */
  let links: Matter.Constraint[];
  let wallBodies: Matter.Body[];
  let grip: Matter.Constraint | null = null;
  let pointer: Point = { x: 0, y: 0 };

  let simMs = 0;
  let debtMs = 0;
  let pushes: Push[] = [];
  let knockUntil = -1;
  let knockDir = 1;
  let lastImpact = Number.NEGATIVE_INFINITY;
  let quietUntil = Number.NEGATIVE_INFINITY;
  /** The phone's pull on every body (px/s²), from follow(); zero when not following. */
  let hand: Point = { x: 0, y: 0 };

  // The chip spot's last position and velocity (px, px/s), for the
  // acceleration's finite differences; null right after a (re)build.
  /** The badge's x velocity (px/s) and spin (rad/s) over the last substep. */
  /** Per wall: when the badge last touched it, and its edge's last speed toward it. */
  let walls = { left: { lastTouch: -Infinity, lastV: 0 }, right: { lastTouch: -Infinity, lastV: 0 } };
  let qPrev: Point | null = null;
  let vPrev: Point | null = null;
  let current: Mg = sensorMg(0, { x: 0, y: 0 }, gain, axes);

  let sink: ((mg: Mg) => void) | null = null;
  const impactListeners = new Set<() => void>();

  function build(width: number, height: number): void {
    geo = lanyardGeometry(width, height, layout);
    const { W, H, bw, bh, clip, anchor, ropeLen } = geo;
    engine = Engine.create({
      gravity: { x: 0, y: 1, scale: GRAVITY_PX_S2 / 1e6 },
      positionIterations: 10,
      velocityIterations: 8,
      constraintIterations: 12,
    });
    const seg = ropeLen / ROPE_SEGMENTS;
    nodes = [];
    links = [];
    const parts: Array<Matter.Body | Matter.Constraint> = [];
    for (let i = 1; i <= ROPE_SEGMENTS; i++) {
      const n = Bodies.circle(anchor.x, anchor.y + i * seg, 3, {
        frictionAir: 0.03,
        collisionFilter: { group: 0, category: 0x2, mask: 0 },
      });
      Body.setMass(n, 0.04);
      nodes.push(n);
      parts.push(n);
      const link = Constraint.create(
        i === 1
          ? { pointA: { ...anchor }, bodyB: n, length: seg, stiffness: 1, damping: 0.05 }
          : { bodyA: nodes[i - 2], bodyB: n, length: seg, stiffness: 1, damping: 0.05 },
      );
      links.push(link);
      parts.push(link);
    }
    const ring = nodes[nodes.length - 1].position;
    const dPC = clip + bh / 2;
    badge = Bodies.rectangle(ring.x, ring.y + dPC, bw, bh, {
      // The board's outline: each corner its own radius (TL, TR, BR, BL).
      chamfer: { radius: [BOARD_CORNERS.tl, BOARD_CORNERS.tr, BOARD_CORNERS.br, BOARD_CORNERS.bl].map((r) => r * bw) },
      frictionAir: 0.022,
      restitution: 0.35,
      friction: 0.05,
      collisionFilter: { group: 0, category: 0x1, mask: 0x4 },
    });
    // matter.js centres the polygon on its centroid, which the unequal
    // chamfers move off the rectangle's centre (about 1 px). The photo and
    // the hang point are placed from body.position, so make the rectangle's
    // centre (its bounds' centre) the body's, and put it where it hangs.
    const { min, max } = badge.bounds;
    Body.setCentre(badge, { x: (min.x + max.x) / 2, y: (min.y + max.y) / 2 });
    Body.setPosition(badge, { x: ring.x, y: ring.y + dPC });
    Body.setMass(badge, 1);
    parts.push(badge);
    parts.push(
      Constraint.create({
        bodyA: nodes[nodes.length - 1],
        bodyB: badge,
        pointB: { x: 0, y: -dPC },
        length: 0,
        stiffness: 1,
        damping: 0.05,
      }),
    );
    wallBodies = makeWalls(W, H);
    parts.push(...wallBodies);
    Composite.add(engine.world, parts);
    // Settle silently: the rope stretches as it takes the badge's weight,
    // which would otherwise read as a ±180 mg bounce at the start.
    for (let i = 0; i < SETTLE_SUBSTEPS; i++) Engine.update(engine, STEP_MS);
    for (const b of [badge, ...nodes]) {
      Body.setVelocity(b, { x: 0, y: 0 });
      Body.setAngularVelocity(b, 0);
    }
    grip = null;
    pushes = [];
    knockUntil = -1;
    qPrev = null;
    vPrev = null;
    walls = { left: { lastTouch: -Infinity, lastV: 0 }, right: { lastTouch: -Infinity, lastV: 0 } };
    debtMs = 0;
  }

  /**
   * The stage's side walls, static colliders just outside 0 and W, and its
   * ceiling, just above y = 0 (the rope, which collides with nothing,
   * passes through it to the anchor). Without the ceiling a fast upward
   * fling swung the badge over its anchor, and it stayed upside down,
   * leaning on a wall above the stage (Ruling R15's flip-over).
   */
  function makeWalls(W: number, H: number): Matter.Body[] {
    const T = 400;
    const wallOpts = {
      isStatic: true,
      restitution: 0.35,
      friction: 0.05,
      collisionFilter: { group: 0, category: 0x4, mask: 0x1 },
      label: "wall",
    };
    const left = Bodies.rectangle(-T / 2, H / 2, T, H * 6, wallOpts);
    const right = Bodies.rectangle(W + T / 2, H / 2, T, H * 6, wallOpts);
    const ceiling = Bodies.rectangle(W / 2, -T / 2, W + 2 * T, T, { ...wallOpts, label: "ceiling" });
    return [left, right, ceiling];
  }

  /**
   * The design's relayout: a new badge size rebuilds the world at rest;
   * the same size (within 0.5 px) keeps the swing, moving the rope, the
   * badge and any grip with the anchor and re-placing the walls.
   */
  function relayout(width: number, height: number): void {
    const next = lanyardGeometry(width, height, layout);
    if (Math.abs(next.bh - geo.bh) > 0.5) {
      build(width, height);
      current = sensorMg(0, { x: 0, y: 0 }, gain, axes);
      return;
    }
    const shift = { x: next.anchor.x - geo.anchor.x, y: next.anchor.y - geo.anchor.y };
    geo = next;
    for (const b of [badge, ...nodes]) Body.translate(b, shift);
    if (grip) {
      grip.pointA = { x: grip.pointA.x + shift.x, y: grip.pointA.y + shift.y };
      pointer = { x: pointer.x + shift.x, y: pointer.y + shift.y };
    }
    links[0].pointA = { ...geo.anchor };
    for (const link of links) link.length = geo.ropeLen / ROPE_SEGMENTS;
    Composite.remove(engine.world, wallBodies);
    wallBodies = makeWalls(geo.W, geo.H);
    Composite.add(engine.world, wallBodies);
    // The shift is not motion: restart the finite differences.
    qPrev = null;
    vPrev = null;
    keepInside();
  }

  function chipSpot(): Point {
    const lx = (CHIP_SPOT.x - 0.5) * geo.bw;
    const ly = (CHIP_SPOT.y - 0.5) * geo.bh;
    const c = Math.cos(badge.angle);
    const s = Math.sin(badge.angle);
    return { x: badge.position.x + lx * c - ly * s, y: badge.position.y + lx * s + ly * c };
  }

  /**
   * Wall impacts, measured on the badge's bounds (so the leading corner's
   * rotation counts) from its actual motion, which includes the grip's
   * position corrections. A contact episode starts when an edge reaches
   * its wall after WALL_REARM_MS away from it; it is an impact if the
   * edge approached faster than IMPACT_PX_S over the substep before or
   * the substep of contact. A badge held against the wall stays in one
   * episode, however the solver jitters, and never re-fires.
   */
  function wallCheck(prevMin: number, prevMax: number): void {
    const { min, max } = badge.bounds;
    const sides = [
      { touching: min.x <= TOUCH_EPS_PX, v: (prevMin - min.x) / STEP_S, wall: walls.left },
      { touching: max.x >= geo.W - TOUCH_EPS_PX, v: (max.x - prevMax) / STEP_S, wall: walls.right },
    ];
    for (const side of sides) {
      const w = side.wall;
      if (side.touching) {
        if (simMs - w.lastTouch > WALL_REARM_MS) {
          if (knockUntil >= 0) knockUntil = -1; // the knock's push ends at the wall
          if (Math.max(side.v, w.lastV) > IMPACT_PX_S) fireImpact();
        }
        w.lastTouch = simMs;
      }
      w.lastV = side.v;
    }
  }

  /**
   * The design's hard clamp: the badge's bounds never leave the walls, nor
   * rise past the ceiling (y = 0), so the badge can never swing over its
   * anchor.
   */
  function keepInside(): void {
    const { min, max } = badge.bounds;
    const dx = min.x < 0 ? -min.x : max.x > geo.W ? geo.W - max.x : 0;
    const dy = min.y < 0 ? -min.y : 0;
    if (dx !== 0 || dy !== 0) Body.translate(badge, { x: dx, y: dy });
  }

  function fireImpact(): void {
    if (simMs - lastImpact <= IMPACT_COOLDOWN_MS || simMs < quietUntil) return;
    lastImpact = simMs;
    for (const cb of [...impactListeners]) {
      try {
        cb();
      } catch (err) {
        // A listener's failure never aborts the simulation step.
        console.error("lanyard: onImpact listener threw", err);
      }
    }
  }

  function applyPushes(): void {
    let ax = 0;
    for (const p of pushes) if (simMs >= p.from && simMs < p.to) ax += p.ax;
    pushes = pushes.filter((p) => simMs < p.to);
    if (knockUntil >= 0 && simMs < knockUntil) ax += knockDir * KNOCK_PX_S2;
    else knockUntil = -1;
    if (ax !== 0) {
      // matter.js forces are in mass·px/ms².
      Body.applyForce(badge, badge.position, { x: (badge.mass * ax) / 1e6, y: 0 });
    }
    if (hand.x !== 0 || hand.y !== 0) {
      for (const b of [badge, ...nodes]) {
        Body.applyForce(b, b.position, { x: (b.mass * hand.x) / 1e6, y: (b.mass * hand.y) / 1e6 });
      }
    }
  }

  function healthy(): boolean {
    for (const b of [badge, ...nodes]) {
      const v = Body.getVelocity(b);
      const speed = Math.hypot(v.x, v.y) * PER_SECOND;
      if (!Number.isFinite(speed + b.angle + b.position.x + b.position.y) || speed > RUNAWAY_PX_S) {
        return false;
      }
    }
    return true;
  }

  /**
   * The grip's target: the pointer, clamped to the stage (±40 px, as the
   * design), within the rope's reach of the anchor, and so that the badge
   * centre stays between the walls; the hand can't pull the badge through
   * a wall or stretch the rope, which would whip it on release.
   */
  function gripTarget(): Point {
    const g = grip!;
    const off = g.pointB;
    let x = Math.max(-40, Math.min(geo.W + 40, pointer.x));
    let y = Math.max(-40, Math.min(geo.H + 40, pointer.y));
    const hw = geo.bw / 2;
    x = Math.max(off.x + hw - GRIP_OVERREACH_PX, Math.min(geo.W - hw + off.x + GRIP_OVERREACH_PX, x));
    const reach = geo.ropeLen + geo.clip + geo.bh / 2 + Math.hypot(off.x, off.y) + GRIP_OVERREACH_PX;
    const dx = x - geo.anchor.x;
    const dy = y - geo.anchor.y;
    const d = Math.hypot(dx, dy);
    if (d > reach) {
      x = geo.anchor.x + (dx * reach) / d;
      y = geo.anchor.y + (dy * reach) / d;
    }
    return { x, y };
  }

  function capSpeeds(): void {
    for (const b of [badge, ...nodes]) {
      const v = Body.getVelocity(b);
      const speed = Math.hypot(v.x, v.y) * PER_SECOND;
      if (speed > MAX_SPEED_PX_S) {
        const k = MAX_SPEED_PX_S / speed;
        Body.setVelocity(b, { x: v.x * k, y: v.y * k });
      }
    }
    const w = Body.getAngularVelocity(badge) * PER_SECOND;
    if (Math.abs(w) > MAX_SPIN_RAD_S) {
      Body.setAngularVelocity(badge, (Math.sign(w) * MAX_SPIN_RAD_S) / PER_SECOND);
    }
  }

  function substep(): Point {
    applyPushes();
    if (grip) grip.pointA = gripTarget();
    const prevMin = badge.bounds.min.x;
    const prevMax = badge.bounds.max.x;
    Engine.update(engine, STEP_MS);
    simMs += STEP_MS;
    capSpeeds();
    if (!healthy()) {
      build(geo.W, geo.H);
      return { x: 0, y: 0 };
    }
    wallCheck(prevMin, prevMax);
    keepInside();
    const q = chipSpot();
    let a = { x: 0, y: 0 };
    if (qPrev) {
      const v = { x: (q.x - qPrev.x) / STEP_S, y: (q.y - qPrev.y) / STEP_S };
      if (vPrev) a = { x: (v.x - vPrev.x) / STEP_S, y: (v.y - vPrev.y) / STEP_S };
      vPrev = v;
    }
    qPrev = q;
    return a;
  }

  build(opts.width, opts.height);

  const self: Lanyard & MotionSource = {
    step(dtMs) {
      if (!Number.isFinite(dtMs) || dtMs <= 0) return;
      debtMs += dtMs;
      let n = Math.floor(debtMs / STEP_MS + 1e-9);
      if (n > MAX_SUBSTEPS) {
        n = MAX_SUBSTEPS;
        debtMs = n * STEP_MS;
      }
      if (n === 0) return;
      debtMs -= n * STEP_MS;
      let sx = 0;
      let sy = 0;
      for (let i = 0; i < n; i++) {
        const a = substep();
        sx += a.x;
        sy += a.y;
      }
      current = sensorMg(badge.angle, { x: sx / n, y: sy / n }, gain, axes);
      sink?.(current);
    },
    relayout(width, height) {
      relayout(width, height);
    },
    pose() {
      return {
        rope: [{ ...geo.anchor }, ...nodes.map((n) => ({ x: n.position.x, y: n.position.y }))],
        badge: {
          x: badge.position.x,
          y: badge.position.y,
          angle: badge.angle,
          w: geo.bw,
          h: geo.bh,
          clip: geo.clip,
        },
      };
    },
    grab(p) {
      if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) return false;
      if (!Vertices.contains(badge.vertices, p)) return false;
      self.release();
      pointer = { x: p.x, y: p.y };
      grip = Constraint.create({
        pointA: { x: p.x, y: p.y },
        bodyB: badge,
        pointB: { x: p.x - badge.position.x, y: p.y - badge.position.y },
        length: 0,
        stiffness: 0.6,
        damping: 0.1,
      });
      Composite.add(engine.world, grip);
      return true;
    },
    drag(p) {
      if (!grip || !Number.isFinite(p.x) || !Number.isFinite(p.y)) return;
      pointer = { x: p.x, y: p.y };
    },
    release() {
      if (grip) Composite.remove(engine.world, grip);
      grip = null;
    },
    impulse(kind) {
      if (kind === "shake") {
        const dir = badge.position.x < geo.W / 2 ? 1 : -1;
        pushes = pushes.filter((p) => !p.shake); // a peer bump's push survives
        for (let k = 0; k < SHAKE_HALVES; k++) {
          pushes.push({
            from: simMs + k * SHAKE_HALF_MS,
            to: simMs + (k + 1) * SHAKE_HALF_MS,
            ax: (k % 2 === 0 ? dir : -dir) * SHAKE_PX_S2,
            shake: true,
          });
        }
      } else {
        knockDir = badge.position.x < geo.W / 2 ? -1 : 1;
        knockUntil = simMs + KNOCK_MAX_MS;
      }
    },
    reading() {
      return current;
    },
    reset() {
      build(geo.W, geo.H);
      current = sensorMg(0, { x: 0, y: 0 }, gain, axes);
    },
    follow(mg) {
      if (!mg) {
        hand = { x: 0, y: 0 };
        return;
      }
      const along = ([axis, sign]: AxisMap["right"]) => {
        const v = mg[axis] * sign;
        return Number.isFinite(v) ? v : 0;
      };
      // The reading is proper acceleration, f = a − g. In the anchor's
      // frame each body feels −f; the simulation's gravity already gives
      // the rest's share, so the rest is the difference (screen y is down).
      // Sideways, Ruling R19 (the human partner, 2026-11-14) takes the
      // opposite, non-physical sign: the drawn badge swings with the tilt
      // (tilt the phone right, it swings right). On the iOS phone checked,
      // a tilt to the right reads +right, and the physical −f swung the
      // badge left. Only the drawing changes; the firmware's readings are
      // the phone's, untouched.
      const x = along(axes.right) * FOLLOW_PX_S2_PER_MG;
      const y = (along(axes.up) - REST_MG) * FOLLOW_PX_S2_PER_MG;
      hand = { x, y: Math.max(-FOLLOW_MAX_LIFT_PX_S2, y) };
    },
    start(s) {
      sink = s;
    },
    stop() {
      sink = null;
    },
    peerBump() {
      const dir = badge.position.x < geo.W / 2 ? 1 : -1; // away from the nearer wall
      pushes.push({ from: simMs, to: simMs + PEER_BUMP_MS, ax: dir * PEER_BUMP_PX_S2 });
      quietUntil = simMs + PEER_BUMP_QUIET_MS;
    },
    onImpact(cb) {
      impactListeners.add(cb);
      return () => {
        impactListeners.delete(cb);
      };
    },
  };
  return self;
}

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
 * - Impacts: a badge-wall contact whose contact point approaches the
 *   wall faster than IMPACT_PX_S fires onImpact, at most once per
 *   IMPACT_COOLDOWN_MS of simulated time (the design's `detect()`).
 *
 * Constants and their tuning: docs/milestone-14-decisions.md, "Lanyard
 * physics and the sensor (Task 9)".
 */
import Matter from "matter-js";
import { BOARD_AXES, type AxisMap } from "./axes";
import { sensorMg } from "./sensor";
import type { Mg, MotionSource } from "./source";

const { Bodies, Body, Composite, Constraint, Engine, Events, Vertices } = Matter;

/** The photo's size (local/m14-design/front.webp), for the aspect ratio. */
export const IMG_W = 974;
export const IMG_H = 1513;
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
export const GAIN_MG_PER_PX_S2 = 0.1;
/** Normal speed at a wall that counts as an impact (design: 7 px/step at 120 Hz). */
export const IMPACT_PX_S = 840;
export const IMPACT_COOLDOWN_MS = 800;
/**
 * The accelerometer's spot on the board, as fractions of the photo
 * (x from the left, y from the top). Estimate: the small LGA part just
 * below the ESP32-C3 module; its marking is not legible in the photo.
 */
export const CHIP_SPOT = { x: 0.414, y: 0.621 };

/** A sideways push: the "hand" accelerates the badge at `ax` px/s². */
interface Push {
  from: number;
  to: number;
  ax: number;
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
/** A runaway (or NaN) body speed; the world is rebuilt at rest past it. */
const RUNAWAY_PX_S = 50_000;
/** Speed caps, as the design's Verlet (60 px per 1/120 s step). */
const MAX_SPEED_PX_S = 7200;
const MAX_SPIN_RAD_S = 40;

export interface Point {
  x: number;
  y: number;
}

export interface Pose {
  rope: Point[];
  badge: { x: number; y: number; angle: number; w: number; h: number };
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
  let grip: Matter.Constraint | null = null;
  let pointer: Point = { x: 0, y: 0 };

  let simMs = 0;
  let debtMs = 0;
  let pushes: Push[] = [];
  let knockUntil = -1;
  let knockDir = 1;
  let lastImpact = Number.NEGATIVE_INFINITY;
  let quietUntil = Number.NEGATIVE_INFINITY;

  // The chip spot's last position and velocity (px, px/s), for the
  // acceleration's finite differences; null right after a (re)build.
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
    const parts: Array<Matter.Body | Matter.Constraint> = [];
    for (let i = 1; i <= ROPE_SEGMENTS; i++) {
      const n = Bodies.circle(anchor.x, anchor.y + i * seg, 3, {
        frictionAir: 0.03,
        collisionFilter: { group: 0, category: 0x2, mask: 0 },
      });
      Body.setMass(n, 0.04);
      nodes.push(n);
      parts.push(n);
      parts.push(
        Constraint.create(
          i === 1
            ? { pointA: { ...anchor }, bodyB: n, length: seg, stiffness: 1, damping: 0.05 }
            : { bodyA: nodes[i - 2], bodyB: n, length: seg, stiffness: 1, damping: 0.05 },
        ),
      );
    }
    const ring = nodes[nodes.length - 1].position;
    const dPC = clip + bh / 2;
    badge = Bodies.rectangle(ring.x, ring.y + dPC, bw, bh, {
      frictionAir: 0.022,
      restitution: 0.35,
      friction: 0.05,
      collisionFilter: { group: 0, category: 0x1, mask: 0x4 },
    });
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
    left.label = "wall-left";
    right.label = "wall-right";
    parts.push(left, right);
    Composite.add(engine.world, parts);
    Events.on(engine, "collisionStart", onCollision);
    grip = null;
    pushes = [];
    knockUntil = -1;
    qPrev = null;
    vPrev = null;
    debtMs = 0;
  }

  function chipSpot(): Point {
    const lx = (CHIP_SPOT.x - 0.5) * geo.bw;
    const ly = (CHIP_SPOT.y - 0.5) * geo.bh;
    const c = Math.cos(badge.angle);
    const s = Math.sin(badge.angle);
    return { x: badge.position.x + lx * c - ly * s, y: badge.position.y + lx * s + ly * c };
  }

  function onCollision(ev: Matter.IEventCollision<Matter.Engine>): void {
    for (const pair of ev.pairs) {
      const wall = pair.bodyA === badge ? pair.bodyB : pair.bodyB === badge ? pair.bodyA : null;
      if (!wall || !wall.label.startsWith("wall")) continue;
      const leftWall = wall.label === "wall-left";
      // The contact point's velocity: linear plus the rotation's ω × r.
      const vx = Body.getVelocity(badge).x * PER_SECOND;
      const w = Body.getAngularVelocity(badge) * PER_SECOND;
      let speed = 0;
      for (const sup of pair.collision.supports) {
        if (!sup) continue;
        const ry = sup.y - badge.position.y;
        const pvx = vx - w * ry;
        speed = Math.max(speed, leftWall ? -pvx : pvx);
      }
      if (knockUntil >= 0) knockUntil = -1; // the knock's push ends at the wall
      if (
        speed > IMPACT_PX_S &&
        simMs - lastImpact > IMPACT_COOLDOWN_MS &&
        simMs >= quietUntil
      ) {
        lastImpact = simMs;
        for (const cb of [...impactListeners]) cb();
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
    x = Math.max(off.x + hw, Math.min(geo.W - hw + off.x, x));
    const reach = geo.ropeLen + geo.clip + geo.bh / 2 + Math.hypot(off.x, off.y);
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
    Engine.update(engine, STEP_MS);
    simMs += STEP_MS;
    capSpeeds();
    if (!healthy()) {
      build(geo.W, geo.H);
      return { x: 0, y: 0 };
    }
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
      build(width, height);
      current = sensorMg(0, { x: 0, y: 0 }, gain, axes);
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
        pushes = [];
        for (let k = 0; k < SHAKE_HALVES; k++) {
          pushes.push({
            from: simMs + k * SHAKE_HALF_MS,
            to: simMs + (k + 1) * SHAKE_HALF_MS,
            ax: (k % 2 === 0 ? dir : -dir) * SHAKE_PX_S2,
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

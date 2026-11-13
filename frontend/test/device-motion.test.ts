import { afterEach, describe, expect, test } from "bun:test";
import { BOARD_AXES } from "../src/motion/axes";
import {
  createDeviceMotionSource,
  deviceMotionToMg,
  motionAvailable,
  platformSignFix,
  requestMotionPermission,
} from "../src/motion/device-motion";
import { CLAMP_MG, REST_MG } from "../src/motion/sensor";
import type { Mg } from "../src/motion/source";

const G = 9.80665;

/** A window stand-in that dispatches `devicemotion` events by hand. */
function fakeWindow() {
  const target = new EventTarget();
  const fire = (a: { x: number | null; y: number | null; z: number | null } | null) => {
    const ev = new Event("devicemotion") as Event & {
      accelerationIncludingGravity: typeof a;
    };
    ev.accelerationIncludingGravity = a;
    target.dispatchEvent(ev);
  };
  const win = {
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
  } as Pick<Window, "addEventListener" | "removeEventListener">;
  return { win, fire };
}

/** A source over a fake window and a hand-driven clock (ms). */
function rig(signFix: 1 | -1 = 1) {
  const { win, fire } = fakeWindow();
  let t = 0;
  const src = createDeviceMotionSource(win, signFix, () => t);
  const readings: Mg[] = [];
  src.start((mg) => readings.push(mg));
  const advance = (ms: number) => {
    t += ms;
  };
  return { src, fire, readings, advance };
}

const up = (mg: Mg) => mg[BOARD_AXES.up[0]] * BOARD_AXES.up[1];
const out = (mg: Mg) => mg[BOARD_AXES.out[0]] * BOARD_AXES.out[1];

describe("deviceMotionToMg", () => {
  test("upright phone reads 1000 mg up", () => {
    const mg = deviceMotionToMg({ x: 0, y: G, z: 0 }, 1, BOARD_AXES);
    expect(up(mg)).toBe(REST_MG);
    expect(out(mg)).toBe(0);
    // The sign fix flips a platform that reports the opposite sign.
    expect(up(deviceMotionToMg({ x: 0, y: -G, z: 0 }, -1, BOARD_AXES))).toBe(REST_MG);
  });

  test("flat phone reads 1000 mg out", () => {
    const mg = deviceMotionToMg({ x: 0, y: 0, z: G }, 1, BOARD_AXES);
    expect(out(mg)).toBe(REST_MG);
    expect(up(mg)).toBe(0);
  });

  test("clamps to the sensor's full scale and reads non-finite as 0", () => {
    const mg = deviceMotionToMg({ x: 5 * G, y: -5 * G, z: Number.NaN }, 1, BOARD_AXES);
    expect(mg).toEqual([CLAMP_MG, -CLAMP_MG, 0]);
  });
});

describe("createDeviceMotionSource", () => {
  test("upright phone reads 1000 mg up through the sink and onMotion", () => {
    const { src, fire, readings } = rig();
    const followed: Mg[] = [];
    src.onMotion((mg) => followed.push(mg));
    fire({ x: 0, y: G, z: 0 });
    expect(readings).toHaveLength(1);
    expect(up(readings[0])).toBe(REST_MG);
    expect(followed).toEqual(readings);
  });

  test("peer bump with phone motion pulses readings", () => {
    const { src, fire, readings, advance } = rig();
    fire({ x: 0, y: G, z: 0 });
    src.peerBump();
    fire({ x: 0, y: G, z: 0 });
    advance(50);
    fire({ x: 1.5 * G, y: G, z: 0 }); // 1500 + 1000 clamps to 2000
    advance(49);
    fire({ x: 0, y: G, z: 0 });
    advance(1); // the 100 ms are over
    fire({ x: 0, y: G, z: 0 });
    expect(readings.map((mg) => mg[0])).toEqual([0, 1000, CLAMP_MG, 1000, 0]);
    expect(readings.map((mg) => mg[1])).toEqual([1000, 1000, 1000, 1000, 1000]);
  });

  test("a peer's pulse is not echoed as an impact", () => {
    const { src, fire, advance } = rig();
    let impacts = 0;
    src.onImpact(() => impacts++);
    fire({ x: 0, y: G, z: 0 });
    src.peerBump();
    // The phone moves 600 mg; with the pulse the reading jumps 1600 mg.
    fire({ x: 0.6 * G, y: G, z: 0 });
    advance(100);
    fire({ x: 0, y: G, z: 0 });
    expect(impacts).toBe(0);
  });

  test("jolt fires onImpact once per 800 ms", () => {
    const { src, fire, advance } = rig();
    let impacts = 0;
    const off = src.onImpact(() => impacts++);
    fire({ x: 0, y: G, z: 0 });
    fire({ x: 1.6 * G, y: G, z: 0 }); // a 1600 mg jump
    expect(impacts).toBe(1);
    advance(400);
    fire({ x: 0, y: G, z: 0 }); // another jump, inside 800 ms
    expect(impacts).toBe(1);
    advance(401);
    fire({ x: 1.6 * G, y: G, z: 0 });
    expect(impacts).toBe(2);
    advance(1000);
    fire({ x: 0.2 * G, y: G, z: 0 }); // 1400 mg: under the threshold
    expect(impacts).toBe(2);
    off();
    advance(1000);
    fire({ x: 1.8 * G, y: G, z: 0 });
    expect(impacts).toBe(2);
  });

  test("null accelerationIncludingGravity is ignored", () => {
    const { fire, readings } = rig();
    fire(null);
    fire({ x: null, y: null, z: null });
    expect(readings).toHaveLength(0);
    fire({ x: 0, y: G, z: 0 });
    expect(readings).toHaveLength(1);
  });

  test("stop removes the listener; start listens again", () => {
    const { src, fire, readings } = rig();
    src.stop();
    fire({ x: 0, y: G, z: 0 });
    expect(readings).toHaveLength(0);
    const again: Mg[] = [];
    src.start((mg) => again.push(mg));
    src.start((mg) => again.push(mg)); // a second start replaces the sink, one listener
    fire({ x: 0, y: G, z: 0 });
    expect(again).toHaveLength(1);
  });

  test("a throwing onMotion or onImpact listener never stops the sink", () => {
    const { src, fire, readings } = rig();
    const error = console.error;
    console.error = () => {};
    try {
      src.onMotion(() => {
        throw new Error("boom");
      });
      src.onImpact(() => {
        throw new Error("boom");
      });
      fire({ x: 0, y: G, z: 0 });
      fire({ x: 2 * G, y: G, z: 0 });
    } finally {
      console.error = error;
    }
    expect(readings).toHaveLength(2);
  });
});

describe("platform", () => {
  const g = globalThis as Record<string, unknown>;
  const saved = { dme: g.DeviceMotionEvent, secure: g.isSecureContext };
  afterEach(() => {
    g.DeviceMotionEvent = saved.dme;
    g.isSecureContext = saved.secure;
  });

  test("platformSignFix starts at 1 everywhere", () => {
    expect(platformSignFix("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)")).toBe(1);
    expect(platformSignFix("Mozilla/5.0 (Linux; Android 15; Pixel 9)")).toBe(1);
    expect(platformSignFix("")).toBe(1);
  });

  test("no DeviceMotionEvent is unsupported", async () => {
    g.DeviceMotionEvent = undefined;
    expect(motionAvailable()).toBe(false);
    expect(await requestMotionPermission()).toBe("unsupported");
  });

  test("an insecure context is unsupported", async () => {
    g.DeviceMotionEvent = class {};
    g.isSecureContext = false;
    expect(motionAvailable()).toBe(false);
    expect(await requestMotionPermission()).toBe("unsupported");
  });

  test("no requestPermission (Android, desktop) is granted", async () => {
    g.DeviceMotionEvent = class {};
    g.isSecureContext = true;
    expect(motionAvailable()).toBe(true);
    expect(await requestMotionPermission()).toBe("granted");
  });

  test("iOS's requestPermission decides; a rejection is denied", async () => {
    g.isSecureContext = true;
    let answer: Promise<string> = Promise.resolve("granted");
    g.DeviceMotionEvent = class {
      static requestPermission = () => answer;
    };
    expect(await requestMotionPermission()).toBe("granted");
    answer = Promise.resolve("denied");
    expect(await requestMotionPermission()).toBe("denied");
    answer = Promise.reject(new Error("NotAllowedError: needs a user gesture"));
    expect(await requestMotionPermission()).toBe("denied");
  });
});

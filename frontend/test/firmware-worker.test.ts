import { describe, expect, test } from "bun:test";
import type { CpuRunSummary, FirmwareEmulatorHandle } from "../src/cpu/bridge";
import {
  BUMP_US,
  MAX_FRAME_DELTA_MS,
  PRESS_HOLD_US,
  RAW_SLOT_BY_BUTTON,
  SHAKE_HALF_US,
  WORKER_STEPS_PER_SLICE,
} from "../src/runtime/firmware-runtime";
import { createWorkerState, handleMessage, type WorkerHost, type WorkerState } from "../src/runtime/firmware-worker";
import {
  createWorkerFirmwareRuntime,
  type VisibilitySource,
  type WorkerClientOptions,
  type WorkerFirmwareRuntime,
  type WorkerLike,
} from "../src/runtime/firmware-worker-client";
import { encodeBase64, LINKED_PEER_RSSI, RadioHub } from "../src/radio/hub";
import { ADV_REPOST_MS, type FromWorker, type ToWorker } from "../src/runtime/firmware-worker-protocol";

/** `bun test` has no `ImageData`; the client's paint blits with one (see test/framebuffer.test.ts). */
class FakeImageData {
  constructor(
    public data: Uint8ClampedArray,
    public width: number,
    public height: number,
  ) {}
}
(globalThis as unknown as { ImageData: typeof FakeImageData }).ImageData = FakeImageData;

/**
 * A fake handle whose clock advances through `runUntil` by `usPerStep` per
 * step (see test/firmware-runtime.test.ts's `makeFakeHandle`), with a
 * console the test can print to.
 */
function makeFakeHandle(usPerStep = 1) {
  let timeUs = 0;
  let steps = 0;
  let consoleText = "";
  const runUntilCalls: Array<[number, number]> = [];
  const buttonCalls: Array<[number, boolean]> = [];
  const accel: Array<[number, number, number]> = [];
  const typed: string[] = [];
  const macs: number[][] = [];
  const delivered: Array<{ bytes: number[]; rssi: number }> = [];
  let adv = new Uint8Array(0);
  let advReads = 0;
  let softResets = 0;
  let rebootArmed = false;
  /** What `setFactoryMac` does: succeed, refuse (`false`) or throw. */
  let macOutcome: "ok" | "refuse" | "throw" = "ok";
  const summary = (n: number): CpuRunSummary => ({
    steps: n,
    traps: 0,
    romStubCalls: 0,
    pc: 0x4200_0000,
    lastInstructionFault: undefined,
  });
  const handle: FirmwareEmulatorHandle = {
    run: (budget) => summary(budget),
    runUntil(targetUs, maxSteps) {
      runUntilCalls.push([targetUs, maxSteps]);
      const before = timeUs;
      timeUs = Math.max(before, Math.min(Math.floor(targetUs), before + maxSteps * usPerStep));
      const n = Math.round((timeUs - before) / usPerStep);
      steps += n;
      if (rebootArmed) {
        // A soft reset at the target: the core adds REBOOT_US at once.
        rebootArmed = false;
        softResets++;
        timeUs += REBOOT_US;
      }
      return summary(n);
    },
    reset() {},
    setRawButton: (slot, pressed) => {
      buttonCalls.push([slot, pressed]);
    },
    framebuffer: () => new Uint16Array(4 * 4),
    screenWidth: 4,
    screenHeight: 4,
    pc: () => 0x4200_0000,
    totalSteps: () => steps,
    emulatedTimeUs: () => timeUs,
    serialInput(bytes) {
      typed.push(new TextDecoder().decode(bytes));
      return bytes.length;
    },
    serialPending: () => 0,
    consoleOutput: () => consoleText,
    consoleWritten: () => new TextEncoder().encode(consoleText).length,
    consoleSince: (at) =>
      new TextDecoder().decode(new TextEncoder().encode(consoleText).subarray(Math.max(0, at))),
    setAcceleration: (x, y, z) => {
      accel.push([x, y, z]);
    },
    setFactoryMac(mac) {
      macs.push([...mac]);
      if (macOutcome === "throw") throw new Error("emulator gone");
      return macOutcome === "ok" && mac.length === 6;
    },
    radioAdvertisement() {
      advReads++;
      // A fresh copy, as the wasm glue returns.
      return adv.slice();
    },
    deliverAdvertisement(bytes, rssi) {
      delivered.push({ bytes: [...bytes], rssi });
      return true;
    },
    softResetCount: () => softResets,
    rebootUs: () => REBOOT_US,
    dispose() {},
  };
  return {
    handle,
    runUntilCalls,
    buttonCalls,
    accel,
    typed,
    macs,
    delivered,
    setAdv: (bytes: number[]) => (adv = new Uint8Array(bytes)),
    advReads: () => advReads,
    setMacOutcome: (o: "ok" | "refuse" | "throw") => (macOutcome = o),
    print: (s: string) => (consoleText += s),
    setTimeUs: (t: number) => (timeUs = t),
    /** The next `runUntil` ends in one soft reset. */
    armReboot: () => (rebootArmed = true),
  };
}

/** `emulator_core::rom::REBOOT_US` (test/cpu-wasm.test.ts checks the passthrough). */
const REBOOT_US = 466_000;

/** A Worker state over a fake handle, a settable wall clock and a manual slice queue. */
function workerRig(usPerStep = 1) {
  const fake = makeFakeHandle(usPerStep);
  let wallMs = 0;
  const slices: Array<{ cb: () => void; immediate: boolean }> = [];
  const host: WorkerHost = {
    createHandle: () => fake.handle,
    now: () => wallMs,
    schedule: (cb, immediate) => {
      slices.push({ cb, immediate });
    },
  };
  const state = createWorkerState(host);
  const posted: FromWorker[] = [];
  const transfers: Array<Transferable[] | undefined> = [];
  const post = (m: FromWorker, transfer?: Transferable[]) => {
    posted.push(m);
    transfers.push(transfer);
  };
  const send = (m: ToWorker) => handleMessage(state, m, post);
  /** Runs the next scheduled slice at wall time `t` (ms). */
  const sliceAt = (t: number) => {
    wallMs = t;
    slices.shift()?.cb();
  };
  return { fake, state, posted, transfers, send, sliceAt, slices };
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

async function bootedRig(usPerStep = 1) {
  const r = workerRig(usPerStep);
  r.send({ kind: "init", wasm: new ArrayBuffer(8), image: new ArrayBuffer(8) });
  await flush();
  expect(r.posted).toEqual([{ kind: "ready" }]);
  r.posted.length = 0;
  r.transfers.length = 0;
  return r;
}

const framesOf = (posted: FromWorker[]) => posted.filter((m) => m.kind === "frame");

describe("firmware worker", () => {
  test("worker slice clamps the wall delta", async () => {
    const { fake, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(1_000); // first slice: no previous slice, runs nothing
    expect(fake.handle.emulatedTimeUs()).toBe(0);
    // A tab back from the background: 10 s of wall time, one 50 ms slice.
    sliceAt(11_000);
    expect(fake.runUntilCalls.at(-1)![0]).toBe(MAX_FRAME_DELTA_MS * 1000);
    expect(fake.handle.emulatedTimeUs()).toBe(MAX_FRAME_DELTA_MS * 1000);
    sliceAt(11_004);
    expect(fake.handle.emulatedTimeUs()).toBe(MAX_FRAME_DELTA_MS * 1000 + 4_000);
  });

  test("worker slice is capped at WORKER_STEPS_PER_SLICE", async () => {
    // One step at 80 MHz: a 50 ms target needs 4,000,000 steps.
    const { fake, send, sliceAt, slices } = await bootedRig(0.0125);
    send({ kind: "start" });
    sliceAt(0);
    expect(fake.runUntilCalls.at(-1)![1]).toBe(WORKER_STEPS_PER_SLICE);
    sliceAt(50);
    expect(fake.runUntilCalls.at(-1)![1]).toBe(WORKER_STEPS_PER_SLICE);
    expect(fake.handle.emulatedTimeUs()).toBeCloseTo(WORKER_STEPS_PER_SLICE * 0.0125, 5);
    // A capped slice yields without the timer clamp (ruling R-T8-1).
    expect(slices.at(-1)!.immediate).toBe(true);
    // The debt is dropped: the next slice targets its own delta.
    sliceAt(54);
    expect(fake.runUntilCalls.at(-1)![0]).toBeCloseTo(WORKER_STEPS_PER_SLICE * 0.0125 + 4_000, 5);
  });

  test("an uncapped slice yields through the timer", async () => {
    const { send, sliceAt, slices } = await bootedRig();
    send({ kind: "start" });
    expect(slices.at(-1)!.immediate).toBe(false);
    sliceAt(0);
    sliceAt(16);
    expect(slices.at(-1)!.immediate).toBe(false);
  });

  test("at most one frame is in flight until frameAck", async () => {
    const { posted, transfers, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    for (let t = 0; t <= 40; t += 8) sliceAt(t);
    const frames = framesOf(posted);
    expect(frames).toHaveLength(1);
    const f = frames[0] as Extract<FromWorker, { kind: "frame" }>;
    expect([f.width, f.height, f.pixels.byteLength]).toEqual([4, 4, 32]);
    // The pixel buffer is transferred, not copied.
    expect(transfers[posted.indexOf(f)]).toEqual([f.pixels]);
    send({ kind: "frameAck" });
    sliceAt(48);
    sliceAt(56);
    expect(framesOf(posted)).toHaveLength(2);
  });

  test("status goes out at most every 250 ms", async () => {
    const { posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    for (let t = 0; t <= 1_000; t += 10) sliceAt(t);
    const statuses = posted.filter((m) => m.kind === "status");
    // At 0, 250, 500, 750 and 1,000 ms.
    expect(statuses).toHaveLength(5);
    expect(statuses.at(-1)).toEqual({ kind: "status", speed: expect.closeTo(1, 5), provision: null, softResets: 0 });
  });

  test("status_is_posted_at_once_on_a_soft_reset", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0); // posts the first status
    sliceAt(10);
    expect(posted.filter((m) => m.kind === "status")).toHaveLength(1);
    // A reset 20 ms in, well inside the 250 ms throttle window.
    fake.armReboot();
    sliceAt(20);
    const statuses = posted.filter((m) => m.kind === "status");
    expect(statuses).toHaveLength(2);
    expect(statuses.at(-1)).toEqual({ kind: "status", speed: expect.closeTo(1, 5), provision: null, softResets: 1 });
    // No reset since: back to the throttle.
    sliceAt(30);
    expect(posted.filter((m) => m.kind === "status")).toHaveLength(2);
    sliceAt(270);
    expect(posted.filter((m) => m.kind === "status")).toHaveLength(3);
  });

  test("a button message for an unknown name is ignored without throwing", async () => {
    const { fake, posted, send } = await bootedRig();
    for (const button of ["FOO", "toString", "constructor", "__proto__", 3, null, undefined]) {
      expect(() => send({ kind: "button", button, pressed: true } as unknown as ToWorker)).not.toThrow();
    }
    expect(() => send({ kind: "nonsense" } as unknown as ToWorker)).not.toThrow();
    expect(() => send(null as unknown as ToWorker)).not.toThrow();
    expect(fake.buttonCalls).toEqual([]);
    expect(posted).toEqual([]);
    send({ kind: "button", button: "A", pressed: true });
    expect(fake.buttonCalls).toEqual([[RAW_SLOT_BY_BUTTON.A, true]]);
  });

  test("buttons are applied before the next slice runs", async () => {
    const { fake, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    send({ kind: "button", button: "DOWN", pressed: true });
    expect(fake.buttonCalls).toEqual([[RAW_SLOT_BY_BUTTON.DOWN, true]]);
    // The press hold is the core runtime's, in emulated time.
    send({ kind: "button", button: "DOWN", pressed: false });
    expect(fake.buttonCalls).toHaveLength(1);
    sliceAt(50);
    sliceAt(100);
    expect(fake.buttonCalls.at(-1)).toEqual([RAW_SLOT_BY_BUTTON.DOWN, false]);
    expect(fake.handle.emulatedTimeUs()).toBe(PRESS_HOLD_US);
  });

  test("a button message's source reaches the runtime; a missing or unknown one is the pad", async () => {
    const { fake, send, sliceAt } = await bootedRig();
    const A = RAW_SLOT_BY_BUTTON.A;
    send({ kind: "start" });
    sliceAt(0);
    send({ kind: "button", button: "A", pressed: true, source: "key" });
    send({ kind: "button", button: "A", pressed: true });
    send({ kind: "button", button: "A", pressed: false, source: "bogus" } as unknown as ToWorker);
    for (let t = 50; t <= 500; t += 50) sliceAt(t);
    expect(fake.buttonCalls).toEqual([[A, true]]);
    send({ kind: "button", button: "A", pressed: false, source: "key" });
    expect(fake.buttonCalls).toEqual([
      [A, true],
      [A, false],
    ]);
  });

  test("stop cancels the scheduled slice and a restart runs one chain", async () => {
    const { fake, send, sliceAt, slices } = await bootedRig();
    send({ kind: "start" });
    send({ kind: "stop" });
    send({ kind: "start" });
    expect(slices).toHaveLength(2);
    sliceAt(0); // the canceled slice: does nothing
    expect(fake.runUntilCalls).toHaveLength(0);
    sliceAt(10);
    expect(fake.runUntilCalls).toHaveLength(1);
    expect(slices).toHaveLength(1);
  });

  test("reset abandons a provision and a shake", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    send({ kind: "provision", identityJson: '{"a":1}' });
    send({ kind: "shake" });
    fake.print("badge> ");
    sliceAt(0);
    expect(fake.typed).toEqual(["put /littlefs/identity.json 7\n"]);
    expect(fake.accel).toEqual([[2000, 2000, 1000]]);
    send({ kind: "reset" });
    expect(fake.accel.at(-1)).toEqual([0, 0, 1000]);
    const nAccel = fake.accel.length;
    fake.print("OK 7\r\nbadge> ");
    fake.setTimeUs(SHAKE_HALF_US);
    sliceAt(300);
    sliceAt(310);
    expect(fake.typed).toHaveLength(1);
    expect(fake.accel).toHaveLength(nAccel);
    expect(posted.filter((m) => m.kind === "status").at(-1)).toEqual({
      kind: "status",
      speed: expect.any(Number),
      provision: null,
      softResets: 0,
    });
  });

  test("a failing slice posts an error and stops the loop", async () => {
    const { fake, posted, send, sliceAt, slices } = await bootedRig();
    fake.handle.runUntil = () => {
      throw new Error("unreachable");
    };
    send({ kind: "start" });
    expect(() => sliceAt(0)).not.toThrow();
    expect(posted.at(-1)?.kind).toBe("error");
    expect(slices).toHaveLength(0);
  });

  test("a failed boot posts an error, and messages before ready are ignored", async () => {
    const r = workerRig();
    (r.state as { host: WorkerHost }).host.createHandle = () => {
      throw new Error("bad image");
    };
    r.send({ kind: "start" });
    r.send({ kind: "init", wasm: new ArrayBuffer(8), image: new ArrayBuffer(8) });
    await flush();
    expect(r.posted).toHaveLength(1);
    expect(r.posted[0]!.kind).toBe("error");
    expect(r.slices).toHaveLength(0);
  });
});

const advsOf = (posted: FromWorker[]) =>
  posted.filter((m) => m.kind === "advOut").map((m) => [...(m as Extract<FromWorker, { kind: "advOut" }>).bytes]);

describe("firmware worker radio", () => {
  test("setMac is applied once, before the first start", async () => {
    const { fake, send } = await bootedRig();
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) });
    expect(fake.macs).toEqual([[2, 0, 0, 0, 0x10, 1]]);
    // A second MAC, or one after start, is ignored: the firmware reads it at boot.
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 2]) });
    send({ kind: "start" });
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 3]) });
    expect(fake.macs).toEqual([[2, 0, 0, 0, 0x10, 1]]);
  });

  test("a malformed setMac is ignored without throwing", async () => {
    const { fake, posted, send } = await bootedRig();
    for (const mac of [null, "02:00:00:00:10:01", [2, 0, 0, 0, 0x10, 1], new Uint8Array(5), new Uint8Array(7)]) {
      expect(() => send({ kind: "setMac", mac } as unknown as ToWorker)).not.toThrow();
    }
    expect(fake.macs).toEqual([]);
    expect(posted).toEqual([]);
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) });
    expect(fake.macs).toHaveLength(1);
  });

  test("mac_locked_only_when_set_factory_mac_succeeds", async () => {
    const { fake, state, posted, send } = await bootedRig();
    // Refused: still unlocked, so a later MAC is tried.
    fake.setMacOutcome("refuse");
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) });
    expect(state.macLocked).toBe(false);
    // Thrown: posted as an error, still unlocked.
    fake.setMacOutcome("throw");
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) });
    expect(state.macLocked).toBe(false);
    expect(posted.map((m) => m.kind)).toEqual(["error"]);
    // Accepted: locked, and the next is ignored.
    fake.setMacOutcome("ok");
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 2]) });
    expect(state.macLocked).toBe(true);
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 3]) });
    expect(fake.macs).toEqual([
      [2, 0, 0, 0, 0x10, 1],
      [2, 0, 0, 0, 0x10, 1],
      [2, 0, 0, 0, 0x10, 2],
    ]);
  });

  test("setMac after a start is ignored", async () => {
    const { fake, send } = await bootedRig();
    send({ kind: "start" });
    send({ kind: "stop" });
    send({ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) });
    expect(fake.macs).toEqual([]);
  });

  test("advIn reaches the emulator; malformed ones are dropped", async () => {
    const { fake, posted, send } = await bootedRig();
    const ok = new Uint8Array([1, 2, 3, 4, 5, 6, 0, 2, 1, 6]);
    send({ kind: "advIn", bytes: ok, rssi: -40 });
    expect(fake.delivered).toEqual([{ bytes: [...ok], rssi: -40 }]);
    const bad: unknown[] = [
      { kind: "advIn", bytes: [1, 2, 3, 4, 5, 6, 0], rssi: -40 },
      { kind: "advIn", bytes: "AAAA", rssi: -40 },
      { kind: "advIn", bytes: ok, rssi: "-40" },
      { kind: "advIn", bytes: ok, rssi: -40.5 },
      { kind: "advIn", bytes: ok, rssi: -129 },
      { kind: "advIn", bytes: ok, rssi: 128 },
      { kind: "advIn", bytes: ok, rssi: Number.NaN },
      { kind: "advIn", bytes: new Uint8Array(237), rssi: -40 },
      { kind: "advIn" },
    ];
    for (const m of bad) expect(() => send(m as ToWorker)).not.toThrow();
    expect(fake.delivered).toHaveLength(1);
    expect(posted).toEqual([]);
  });

  test("bump plays the pulse in emulated time", async () => {
    const { fake, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    send({ kind: "bump" });
    expect(fake.accel).toEqual([[1000, 0, 1000]]);
    sliceAt(50);
    sliceAt(99);
    expect(fake.accel).toHaveLength(1);
    sliceAt(100);
    expect(fake.handle.emulatedTimeUs()).toBe(BUMP_US);
    expect(fake.accel.at(-1)).toEqual([0, 0, 1000]);
  });

  test("advOut is posted on a change, at most every ADV_REPOST_MS, and re-posted that often while advertising", async () => {
    expect(ADV_REPOST_MS).toBe(100);
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    sliceAt(10);
    // Not advertising (Connect's idle home): nothing posted.
    expect(advsOf(posted)).toEqual([]);
    fake.setAdv([1, 2, 3, 4, 5, 6, 0, 3, 0xff, 0xff, 0xff]);
    sliceAt(20);
    expect(advsOf(posted)).toEqual([[1, 2, 3, 4, 5, 6, 0, 3, 0xff, 0xff, 0xff]]);
    sliceAt(30);
    sliceAt(119);
    expect(advsOf(posted)).toHaveLength(1);
    // A change once ADV_REPOST_MS have passed goes out at once.
    fake.setAdv([1, 2, 3, 4, 5, 6, 0, 4, 0x42, 0x50, 0x41, 0x31]);
    sliceAt(121);
    expect(advsOf(posted)).toHaveLength(2);
    expect(advsOf(posted).at(-1)).toEqual([1, 2, 3, 4, 5, 6, 0, 4, 0x42, 0x50, 0x41, 0x31]);
    // The same advertisement again after 100 ms.
    for (let t = 125; t <= 213; t += 8) sliceAt(t);
    expect(advsOf(posted)).toHaveLength(2);
    sliceAt(221);
    expect(advsOf(posted)).toHaveLength(3);
    // At most 10 Hz over a second of slices.
    for (let t = 230; t <= 1_230; t += 4) sliceAt(t);
    expect(advsOf(posted).length).toBeLessThanOrEqual(3 + 10);
    expect(advsOf(posted).length).toBeGreaterThanOrEqual(3 + 9);
    // Stopping advertising is a change: one empty advOut, on the next tick, then silence.
    const n = advsOf(posted).length;
    fake.setAdv([]);
    sliceAt(1_340);
    expect(advsOf(posted)).toHaveLength(n + 1);
    expect(advsOf(posted).at(-1)).toEqual([]);
    for (let t = 1_350; t <= 2_000; t += 10) sliceAt(t);
    expect(advsOf(posted)).toHaveLength(n + 1);
  });

  test("changed_advertisements_are_coalesced_to_10_hz", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    // 20 changes in 100 ms, a slice after each.
    const advAt = (i: number) => [1, 2, 3, 4, 5, 6, 0, 2, 0xff, i];
    for (let i = 0; i < 20; i++) {
      fake.setAdv(advAt(i));
      sliceAt(1_000 + 5 * i);
    }
    // The first change after a quiet spell goes out at once.
    expect(advsOf(posted)).toEqual([advAt(0)]);
    sliceAt(1_099);
    expect(advsOf(posted)).toHaveLength(1);
    // The tick: the latest, not the ones between.
    sliceAt(1_100);
    expect(advsOf(posted)).toEqual([advAt(0), advAt(19)]);
    // A final change is not dropped: it goes out on the next tick.
    fake.setAdv([]);
    sliceAt(1_150);
    expect(advsOf(posted)).toHaveLength(2);
    sliceAt(1_200);
    expect(advsOf(posted).at(-1)).toEqual([]);
    expect(advsOf(posted)).toHaveLength(3);
  });

  test("a stopped worker posts no advOut", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    fake.setAdv([1, 2, 3, 4, 5, 6, 0, 1, 0]);
    send({ kind: "start" });
    sliceAt(0);
    expect(advsOf(posted)).toHaveLength(1);
    send({ kind: "stop" });
    for (let t = 100; t <= 1_000; t += 100) sliceAt(t);
    expect(advsOf(posted)).toHaveLength(1);
  });
});

/** A fake Worker that records what the client posts and lets the test reply. */
function fakeWorker() {
  const sent: ToWorker[] = [];
  let terminated = 0;
  const w: WorkerLike = {
    postMessage: (m) => {
      sent.push(m);
    },
    onmessage: null,
    terminate: () => {
      terminated++;
    },
  };
  const reply = (m: FromWorker) => w.onmessage?.({ data: m });
  return { w, sent, reply, terminated: () => terminated };
}

function makeCtx() {
  let puts = 0;
  const ctx = { putImageData: () => puts++ } as unknown as CanvasRenderingContext2D;
  return { ctx, puts: () => puts };
}

/** A client over a fake Worker with a manual animation-frame queue. */
function clientRig(options: WorkerClientOptions = {}) {
  const fw = fakeWorker();
  const { ctx, puts } = makeCtx();
  const rafs = new Map<number, (t: number) => void>();
  let rafSeq = 0;
  const rt = createWorkerFirmwareRuntime(fw.w, ctx, new ArrayBuffer(8), new ArrayBuffer(8), {
    scheduleFrame: (cb) => {
      rafs.set(++rafSeq, cb);
      return rafSeq;
    },
    cancelFrame: (id) => {
      rafs.delete(id);
    },
    ...options,
  });
  const animationFrame = () => {
    const cbs = [...rafs.values()];
    rafs.clear();
    for (const cb of cbs) cb(0);
  };
  return { ...fw, rt, puts, animationFrame, rafs };
}

const frameMsg = (): FromWorker => ({ kind: "frame", pixels: new ArrayBuffer(32), width: 4, height: 4 });

describe("firmware worker client", () => {
  test("messages before the worker is ready are queued in order", () => {
    const { rt, sent, reply } = clientRig();
    expect(sent.map((m) => m.kind)).toEqual(["init"]);
    rt.injectButton("A", "pressed");
    rt.provision('{"a":1}');
    rt.start();
    rt.injectButton("A", "released");
    rt.stop();
    expect(sent.map((m) => m.kind)).toEqual(["init"]);
    reply({ kind: "ready" });
    expect(sent.slice(1)).toEqual([
      { kind: "button", button: "A", pressed: true, source: "pad" },
      { kind: "provision", identityJson: '{"a":1}' },
      { kind: "start" },
      { kind: "button", button: "A", pressed: false, source: "pad" },
      { kind: "stop" },
    ]);
    // After ready, messages go straight through.
    rt.shake();
    expect(sent.at(-1)).toEqual({ kind: "shake" });
    rt.injectButton("B", "pressed", "key");
    expect(sent.at(-1)).toEqual({ kind: "button", button: "B", pressed: true, source: "key" });
  });

  test("init transfers the WASM bytes and the image", () => {
    const fw = fakeWorker();
    const transfers: Array<Transferable[] | undefined> = [];
    fw.w.postMessage = (m, t) => {
      fw.sent.push(m);
      transfers.push(t);
    };
    const wasm = new ArrayBuffer(8);
    const image = new ArrayBuffer(16);
    createWorkerFirmwareRuntime(fw.w, makeCtx().ctx, wasm, image, { scheduleFrame: () => 1, cancelFrame: () => {} });
    expect(fw.sent).toEqual([{ kind: "init", wasm, image }]);
    expect(transfers[0]).toEqual([wasm, image]);
  });

  test("worker client flushes a deferred release on stop", async () => {
    // The client wired to a real Worker state over a fake handle.
    const fake = makeFakeHandle();
    let wallMs = 0;
    const slices: Array<() => void> = [];
    const state: WorkerState = createWorkerState({
      createHandle: () => fake.handle,
      now: () => wallMs,
      schedule: (cb) => {
        slices.push(cb);
      },
    });
    const w: WorkerLike = {
      postMessage: (m) => handleMessage(state, m, (out) => queueMicrotask(() => w.onmessage?.({ data: out }))),
      onmessage: null,
      terminate: () => {},
    };
    const rt = createWorkerFirmwareRuntime(w, makeCtx().ctx, new ArrayBuffer(8), new ArrayBuffer(8), {
      scheduleFrame: () => 1,
      cancelFrame: () => {},
    });
    // Sent before the Worker is ready: queued, then replayed in order.
    rt.start();
    rt.injectButton("A", "pressed");
    await flush();
    expect(fake.buttonCalls).toEqual([[RAW_SLOT_BY_BUTTON.A, true]]);
    wallMs = 0;
    slices.shift()?.();
    wallMs = 30;
    slices.shift()?.();
    // Released and stopped within PRESS_HOLD_US: the release is deferred, then flushed.
    expect(fake.handle.emulatedTimeUs()).toBeLessThan(PRESS_HOLD_US);
    rt.injectButton("A", "released");
    expect(fake.buttonCalls).toHaveLength(1);
    rt.stop();
    expect(fake.buttonCalls).toEqual([
      [RAW_SLOT_BY_BUTTON.A, true],
      [RAW_SLOT_BY_BUTTON.A, false],
    ]);
  });

  test("a frame is painted on the next animation frame, then acknowledged", () => {
    const { rt, sent, reply, puts, animationFrame } = clientRig();
    reply({ kind: "ready" });
    rt.start();
    reply(frameMsg());
    expect(puts()).toBe(0);
    expect(sent.filter((m) => m.kind === "frameAck")).toHaveLength(0);
    animationFrame();
    expect(puts()).toBe(1);
    expect(sent.at(-1)).toEqual({ kind: "frameAck" });
  });

  test("a frame while stopped, or waiting at stop(), is dropped and acknowledged", () => {
    const { rt, sent, reply, puts, animationFrame, rafs } = clientRig();
    reply({ kind: "ready" });
    reply(frameMsg()); // not started
    expect(sent.filter((m) => m.kind === "frameAck")).toHaveLength(1);
    rt.start();
    reply(frameMsg());
    rt.stop();
    expect(rafs.size).toBe(0);
    expect(sent.filter((m) => m.kind === "frameAck")).toHaveLength(2);
    animationFrame();
    expect(puts()).toBe(0);
  });

  test("speed() and provisionState() return the last status", () => {
    const { rt, reply } = clientRig();
    expect(rt.speed()).toBe(0);
    expect(rt.provisionState()).toBeNull();
    reply({ kind: "ready" });
    rt.provision("{}");
    expect(rt.provisionState()).toEqual({ kind: "waiting" });
    reply({ kind: "status", speed: 0.75, provision: { kind: "typing" } as never, softResets: 0 });
    expect(rt.speed()).toBe(0.75);
    expect(rt.provisionState()).toEqual({ kind: "typing" } as never);
    rt.reset();
    expect(rt.speed()).toBe(0);
    expect(rt.provisionState()).toBeNull();
    expect(rt.isExitRequested()).toBe(false);
  });

  test("ready rejects on a boot error, and dispose terminates the worker", async () => {
    const { rt, reply, terminated } = clientRig();
    const original = console.error;
    console.error = () => {};
    try {
      reply({ kind: "error", message: "firmware boot failed: bad image" });
    } finally {
      console.error = original;
    }
    await expect(rt.ready).rejects.toThrow("bad image");
    rt.dispose();
    rt.dispose();
    expect(terminated()).toBe(1);
    expect(() => rt.start()).toThrow();
  });
});

describe("firmware worker client radio", () => {
  test("setMac is sent before start, and bump is forwarded", () => {
    const { rt, sent, reply } = clientRig();
    rt.setMac(new Uint8Array([2, 0, 0, 0, 0x10, 1]));
    rt.start();
    reply({ kind: "ready" });
    rt.bump();
    expect(sent.slice(1)).toEqual([
      { kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) },
      { kind: "start" },
      { kind: "bump" },
    ]);
  });

  test("advOut reaches onAdvertisement, and a throwing listener does not escape", () => {
    const heard: number[][] = [];
    const { rt, reply } = clientRig({ onAdvertisement: (b) => heard.push([...b]) });
    reply({ kind: "ready" });
    rt.start();
    reply({ kind: "advOut", bytes: new Uint8Array([1, 2, 3]) });
    expect(heard).toEqual([[1, 2, 3]]);
    reply({ kind: "advOut", bytes: "nope" } as unknown as FromWorker);
    expect(heard).toHaveLength(1);
    const throwing = clientRig({
      onAdvertisement: () => {
        throw new Error("hub gone");
      },
    });
    throwing.reply({ kind: "ready" });
    throwing.rt.start();
    const original = console.error;
    console.error = () => {};
    try {
      expect(() => throwing.reply({ kind: "advOut", bytes: new Uint8Array([1]) })).not.toThrow();
    } finally {
      console.error = original;
    }
  });

  test("deliverAdvertisement posts advIn only while running and visible", () => {
    const vis = fakeVisibility();
    const { rt, sent, reply } = clientRig({ visibility: vis.source });
    const adv = new Uint8Array([1, 2, 3, 4, 5, 6, 0, 1, 0]);
    rt.deliverAdvertisement(adv, -40); // not ready, not started: dropped, not queued
    reply({ kind: "ready" });
    rt.deliverAdvertisement(adv, -40); // not started
    rt.start();
    rt.deliverAdvertisement(adv, -40);
    vis.set(true);
    rt.deliverAdvertisement(adv, -40); // hidden
    vis.set(false);
    rt.stop();
    rt.deliverAdvertisement(adv, -40); // stopped
    expect(sent.filter((m) => m.kind === "advIn")).toEqual([{ kind: "advIn", bytes: adv, rssi: -40 }]);
  });

  test("a stop during the hidden-page pause ends the pause: the hub is resumed", () => {
    const vis = fakeVisibility();
    // Wired as main.ts wires it: the hub delivers to the client.
    let client: WorkerFirmwareRuntime | null = null;
    const hub = new RadioHub({
      deliver: (b, r) => client?.deliverAdvertisement(b, r),
      bumpLocal: () => client?.bump(),
    });
    const paused: boolean[] = [];
    const { rt, sent, reply } = clientRig({
      visibility: vis.source,
      onPausedChange: (p) => {
        paused.push(p);
        hub.setPaused(p);
      },
    });
    client = rt;
    reply({ kind: "ready" });
    rt.start();
    vis.set(true); // hidden: paused
    rt.stop(); // a takeover's onLost while hidden
    expect(paused).toEqual([true, false]);
    vis.set(false); // visible, but stopped: nothing changes
    rt.start(); // the badge taken back
    expect(paused).toEqual([true, false]);
    expect(sent.at(-1)).toEqual({ kind: "start" });
    // The hub is un-paused: a peer's advert reaches the running badge.
    let heard: ((m: unknown) => void) | null = null;
    hub.addLink({ id: "p", send: () => {}, onMessage: (cb) => (heard = cb) });
    const adv = new Uint8Array([1, 2, 3, 4, 5, 6, 0, 1, 0]);
    heard!({ t: "adv", b: encodeBase64(adv) });
    expect(sent.filter((m) => m.kind === "advIn")).toEqual([{ kind: "advIn", bytes: adv, rssi: LINKED_PEER_RSSI }]);
  });

  test("a start while hidden pauses the hub until the page is visible", () => {
    const vis = fakeVisibility(true);
    const paused: boolean[] = [];
    const { rt, reply } = clientRig({ visibility: vis.source, onPausedChange: (p) => paused.push(p) });
    reply({ kind: "ready" });
    rt.start();
    expect(paused).toEqual([true]);
    vis.set(false);
    expect(paused).toEqual([true, false]);
  });

  test("bump is dropped while stopped or hidden, not replayed later", () => {
    const vis = fakeVisibility();
    const { rt, sent, reply } = clientRig({ visibility: vis.source });
    reply({ kind: "ready" });
    rt.bump(); // not started
    rt.start();
    vis.set(true);
    rt.bump(); // hidden
    vis.set(false);
    rt.bump();
    rt.stop();
    rt.bump(); // stopped
    expect(sent.filter((m) => m.kind === "bump")).toHaveLength(1);
  });

  test("the hidden-page pause calls onPausedChange: paused before stop, resumed after start", () => {
    const vis = fakeVisibility();
    const log: string[] = [];
    const { rt, sent, reply } = clientRig({
      visibility: vis.source,
      onPausedChange: (p) => log.push(`${p ? "pause" : "resume"} after ${sent.at(-1)?.kind}`),
    });
    reply({ kind: "ready" });
    rt.start();
    vis.set(true);
    vis.set(false);
    expect(log).toEqual(["pause after start", "resume after start"]);
    expect(sent.map((m) => m.kind).filter((k) => k === "stop" || k === "start")).toEqual(["start", "stop", "start"]);
  });
});

/** A settable visibility source (see `VisibilitySource`). */
function fakeVisibility(initiallyHidden = false) {
  let hidden = initiallyHidden;
  const subs = new Set<() => void>();
  const source: VisibilitySource = {
    hidden: () => hidden,
    subscribe(cb) {
      subs.add(cb);
      return () => subs.delete(cb);
    },
  };
  const set = (h: boolean) => {
    hidden = h;
    for (const cb of [...subs]) cb();
  };
  return { source, set, subscribers: () => subs.size };
}

describe("firmware worker client visibility", () => {
  test("hidden tab pauses the worker and resumes on visible", async () => {
    // The client wired to a real Worker state over a fake handle.
    const fake = makeFakeHandle();
    let wallMs = 0;
    const slices: Array<() => void> = [];
    const state = createWorkerState({
      createHandle: () => fake.handle,
      now: () => wallMs,
      schedule: (cb) => {
        slices.push(cb);
      },
    });
    const sent: ToWorker[] = [];
    const w: WorkerLike = {
      postMessage: (m) => {
        sent.push(m);
        handleMessage(state, m, (out) => queueMicrotask(() => w.onmessage?.({ data: out })));
      },
      onmessage: null,
      terminate: () => {},
    };
    const vis = fakeVisibility();
    const rt = createWorkerFirmwareRuntime(w, makeCtx().ctx, new ArrayBuffer(8), new ArrayBuffer(8), {
      scheduleFrame: () => 1,
      cancelFrame: () => {},
      visibility: vis.source,
    });
    await flush();
    rt.start();
    slices.shift()?.();
    wallMs = 16;
    slices.shift()?.();
    rt.injectButton("A", "pressed");
    rt.injectButton("A", "released"); // deferred: inside PRESS_HOLD_US
    expect(fake.buttonCalls).toEqual([[RAW_SLOT_BY_BUTTON.A, true]]);

    vis.set(true);
    expect(sent.at(-1)).toEqual({ kind: "stop" });
    // The deferred release is flushed, and the scheduled slice is dead.
    expect(fake.buttonCalls.at(-1)).toEqual([RAW_SLOT_BY_BUTTON.A, false]);
    const calls = fake.runUntilCalls.length;
    for (const s of slices.splice(0)) s();
    expect(fake.runUntilCalls).toHaveLength(calls);

    // Visible again: a fresh start, whose first slice counts no hidden time.
    wallMs = 60_000;
    vis.set(false);
    expect(sent.at(-1)).toEqual({ kind: "start" });
    const before = fake.handle.emulatedTimeUs();
    slices.shift()?.();
    expect(fake.handle.emulatedTimeUs()).toBe(before);
    wallMs = 60_016;
    slices.shift()?.();
    expect(fake.handle.emulatedTimeUs()).toBe(before + 16_000);
  });

  test("a runtime the user stopped is not started by a visible tab", () => {
    const fw = fakeWorker();
    const vis = fakeVisibility();
    const rt = createWorkerFirmwareRuntime(fw.w, makeCtx().ctx, new ArrayBuffer(8), new ArrayBuffer(8), {
      scheduleFrame: () => 1,
      cancelFrame: () => {},
      visibility: vis.source,
    });
    fw.reply({ kind: "ready" });
    // Never started: visibility changes send nothing.
    vis.set(true);
    vis.set(false);
    expect(fw.sent.map((m) => m.kind)).toEqual(["init"]);
    rt.start();
    rt.stop(); // a mode switch
    vis.set(true);
    vis.set(false);
    expect(fw.sent.map((m) => m.kind)).toEqual(["init", "start", "stop"]);
  });

  test("starting while hidden waits for visible, and frames while paused are acknowledged", () => {
    const fw = fakeWorker();
    const vis = fakeVisibility(true);
    const rt = createWorkerFirmwareRuntime(fw.w, makeCtx().ctx, new ArrayBuffer(8), new ArrayBuffer(8), {
      scheduleFrame: () => 1,
      cancelFrame: () => {},
      visibility: vis.source,
    });
    fw.reply({ kind: "ready" });
    rt.start();
    expect(fw.sent.map((m) => m.kind)).toEqual(["init"]);
    fw.reply(frameMsg());
    expect(fw.sent.at(-1)).toEqual({ kind: "frameAck" });
    vis.set(false);
    expect(fw.sent.at(-1)).toEqual({ kind: "start" });
    rt.dispose();
    expect(vis.subscribers()).toBe(0);
  });
});

test("a restart after a capped slice yields through the timer", async () => {
  const { send, sliceAt, slices } = await bootedRig(0.0125);
  send({ kind: "start" });
  sliceAt(0);
  sliceAt(50);
  expect(slices.at(-1)!.immediate).toBe(true);
  send({ kind: "stop" });
  send({ kind: "start" });
  expect(slices.at(-1)!.immediate).toBe(false);
});

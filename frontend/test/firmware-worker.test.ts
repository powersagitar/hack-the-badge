import { describe, expect, test } from "bun:test";
import type { CpuRunSummary, FirmwareEmulatorHandle } from "../src/cpu/bridge";
import {
  MAX_FRAME_DELTA_MS,
  PRESS_HOLD_US,
  RAW_SLOT_BY_BUTTON,
  SHAKE_HALF_US,
  WORKER_STEPS_PER_SLICE,
} from "../src/runtime/firmware-runtime";
import { createWorkerState, handleMessage, type WorkerHost, type WorkerState } from "../src/runtime/firmware-worker";
import { createWorkerFirmwareRuntime, type WorkerLike } from "../src/runtime/firmware-worker-client";
import type { FromWorker, ToWorker } from "../src/runtime/firmware-worker-protocol";

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
    setAcceleration: (x, y, z) => {
      accel.push([x, y, z]);
    },
    dispose() {},
  };
  return {
    handle,
    runUntilCalls,
    buttonCalls,
    accel,
    typed,
    print: (s: string) => (consoleText += s),
    setTimeUs: (t: number) => (timeUs = t),
  };
}

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
    expect(statuses.at(-1)).toEqual({ kind: "status", speed: expect.closeTo(1, 5), provision: null });
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
function clientRig() {
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
      { kind: "button", button: "A", pressed: true },
      { kind: "provision", identityJson: '{"a":1}' },
      { kind: "start" },
      { kind: "button", button: "A", pressed: false },
      { kind: "stop" },
    ]);
    // After ready, messages go straight through.
    rt.shake();
    expect(sent.at(-1)).toEqual({ kind: "shake" });
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
    reply({ kind: "status", speed: 0.75, provision: { kind: "typing" } as never });
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

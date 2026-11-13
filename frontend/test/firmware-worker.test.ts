import { describe, expect, test } from "bun:test";
import type { CpuRunSummary, FirmwareEmulatorHandle } from "../src/cpu/bridge";
import {
  MAX_FRAME_DELTA_MS,
  PRESS_HOLD_US,
  RAW_SLOT_BY_BUTTON,
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
import { REBOOTED_REASON } from "../src/runtime/provisioner";
import { SKIP_REBOOTED_REASON } from "../src/runtime/setup-skip";

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
  /** The console's bytes: a test can print a partial UTF-8 sequence. */
  let consoleBytes = new Uint8Array(0);
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
  let powerCycles = 0;
  let leds = new Uint8Array(0);
  let ledFrames = 0;
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
    consoleOutput: () => new TextDecoder().decode(consoleBytes),
    consoleWritten: () => consoleBytes.length,
    consoleSince: (at) => new TextDecoder().decode(consoleBytes.subarray(Math.max(0, at))),
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
    powerCycle() {
      // A cold power-on: the core's counters, clock, console and LEDs start over.
      powerCycles++;
      timeUs = 0;
      steps = 0;
      softResets = 0;
      consoleBytes = new Uint8Array(0);
      leds = new Uint8Array(0);
      ledFrames = 0;
    },
    ledColors: () => leds.slice(),
    ledFrameCount: () => ledFrames,
    dispose() {},
  };
  function printBytes(b: Uint8Array): void {
    const next = new Uint8Array(consoleBytes.length + b.length);
    next.set(consoleBytes);
    next.set(b, consoleBytes.length);
    consoleBytes = next;
  }
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
    print: (s: string) => printBytes(new TextEncoder().encode(s)),
    printBytes: (b: number[]) => printBytes(new Uint8Array(b)),
    setTimeUs: (t: number) => (timeUs = t),
    /** The next `runUntil` ends in one soft reset. */
    armReboot: () => (rebootArmed = true),
    powerCycles: () => powerCycles,
    /** The firmware sends one LED frame. */
    showLeds: (rgb: number[]) => {
      leds = new Uint8Array(rgb);
      ledFrames++;
    },
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

/** A `status` with nothing in flight: no provision, no skip, Setup not on screen, not rebooting. */
const idleStatus = (speed: number): Extract<FromWorker, { kind: "status" }> => ({
  kind: "status",
  speed,
  rebooting: false,
  provision: null,
  skip: null,
  setupOnScreen: false,
});

const statusesOf = (posted: FromWorker[]) =>
  posted.filter((m): m is Extract<FromWorker, { kind: "status" }> => m.kind === "status");

const SETUP_LINE = "I (20) app_reg: launched Setup\r\n";
const CONFIG_TYPED = "config system onboard_build 2147483647\n";

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
    expect(statuses.at(-1)).toEqual(idleStatus(expect.closeTo(1, 5)));
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
    expect(statuses.at(-1)).toEqual({ ...idleStatus(expect.closeTo(1, 5)), rebooting: true });
    // No reset since: back to the throttle.
    sliceAt(30);
    expect(posted.filter((m) => m.kind === "status")).toHaveLength(2);
    sliceAt(270);
    expect(posted.filter((m) => m.kind === "status")).toHaveLength(3);
  });

  test("skipSetup reaches the runtime", async () => {
    const { fake, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    fake.print(`${SETUP_LINE}badge> `);
    // The first skip, with Setup on screen and nothing in flight, carries
    // extra fields: they are ignored, and only the kind is read.
    expect(() => send({ kind: "skipSetup", junk: 1 } as unknown as ToWorker)).not.toThrow();
    // A second one is refused by the runtime: one config, never two.
    send({ kind: "skipSetup" });
    sliceAt(10);
    expect(fake.typed).toEqual([CONFIG_TYPED]);
    sliceAt(20);
    expect(fake.typed).toEqual([CONFIG_TYPED]);
  });

  test("status carries rebooting, skip and setupOnScreen, and no softResets", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    expect(statusesOf(posted).at(-1)).toEqual(idleStatus(0));
    expect("softResets" in statusesOf(posted).at(-1)!).toBe(false);
    fake.print(`${SETUP_LINE}badge> `);
    sliceAt(250);
    expect(statusesOf(posted).at(-1)).toEqual({ ...idleStatus(expect.any(Number)), setupOnScreen: true });
    send({ kind: "skipSetup" });
    sliceAt(500);
    expect(statusesOf(posted).at(-1)).toEqual({ ...idleStatus(expect.any(Number)), skip: { kind: "typing" } });
    fake.print("OK system.onboard_build=2147483647\r\nbadge> ");
    sliceAt(510);
    // The skip's reboot: posted at once, rebooting, the skip done.
    fake.armReboot();
    sliceAt(520);
    expect(statusesOf(posted)).toHaveLength(4);
    expect(statusesOf(posted).at(-1)).toEqual({
      ...idleStatus(expect.any(Number)),
      rebooting: true,
      skip: { kind: "done" },
    });
    // The reboot's 466 ms paid, one status interval later: no longer rebooting.
    for (let t = 570; t <= 1_020; t += 50) sliceAt(t);
    expect(statusesOf(posted).at(-1)).toEqual({ ...idleStatus(expect.any(Number)), skip: { kind: "done" } });
    for (const s of statusesOf(posted)) expect("softResets" in s).toBe(false);
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

  test("reset abandons a provision and puts the sensor at rest", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    send({ kind: "provision", identityJson: '{"a":1}' });
    send({ kind: "accel", mg: [2000, 2000, 1000] });
    fake.print("badge> ");
    sliceAt(0);
    expect(fake.typed).toEqual(["put /littlefs/identity.json 7\n"]);
    expect(fake.accel).toEqual([[2000, 2000, 1000]]);
    send({ kind: "reset" });
    expect(fake.accel.at(-1)).toEqual([0, 0, 1000]);
    const nAccel = fake.accel.length;
    fake.print("OK 7\r\nbadge> ");
    sliceAt(300);
    sliceAt(310);
    expect(fake.typed).toHaveLength(1);
    expect(fake.accel).toHaveLength(nAccel);
    expect(posted.filter((m) => m.kind === "status").at(-1)).toEqual(idleStatus(expect.any(Number)));
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
    rt.powerCycle();
    expect(sent.at(-1)).toEqual({ kind: "powerCycle" });
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
    reply({ ...idleStatus(0.75), provision: { kind: "typing" } as never });
    expect(rt.speed()).toBe(0.75);
    expect(rt.provisionState()).toEqual({ kind: "typing" } as never);
    rt.reset();
    expect(rt.speed()).toBe(0);
    expect(rt.provisionState()).toBeNull();
  });

  test("skipSetup hides the skip until the next status", () => {
    const { rt, sent, reply } = clientRig();
    expect([rt.skipState(), rt.setupOnScreen(), rt.rebooting()]).toEqual([null, false, false]);
    reply({ kind: "ready" });
    // Setup not on screen yet: nothing sent.
    rt.skipSetup();
    expect(sent.filter((m) => m.kind === "skipSetup")).toHaveLength(0);
    reply({ ...idleStatus(1), setupOnScreen: true });
    expect(rt.setupOnScreen()).toBe(true);
    rt.skipSetup();
    expect(sent.at(-1)).toEqual({ kind: "skipSetup" });
    expect(rt.skipState()).toEqual({ kind: "typing" });
    expect(rt.setupOnScreen()).toBe(false);
    // A double click sends once.
    rt.skipSetup();
    expect(sent.filter((m) => m.kind === "skipSetup")).toHaveLength(1);
    // The next status replaces the optimistic state.
    reply({ ...idleStatus(1), rebooting: true, skip: { kind: "done" } });
    expect([rt.skipState(), rt.setupOnScreen(), rt.rebooting()]).toEqual([{ kind: "done" }, false, true]);
    // Setup again (a failed skip, or a re-provision): a fresh skip is sent.
    reply({ ...idleStatus(1), skip: { kind: "failed", reason: "timed out" }, setupOnScreen: true });
    rt.skipSetup();
    expect(sent.filter((m) => m.kind === "skipSetup")).toHaveLength(2);
    rt.reset();
    expect([rt.skipState(), rt.setupOnScreen(), rt.rebooting()]).toEqual([null, false, false]);
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
  test("setMac is sent before start", () => {
    const { rt, sent, reply } = clientRig();
    rt.setMac(new Uint8Array([2, 0, 0, 0, 0x10, 1]));
    rt.start();
    reply({ kind: "ready" });
    expect(sent.slice(1)).toEqual([{ kind: "setMac", mac: new Uint8Array([2, 0, 0, 0, 0x10, 1]) }, { kind: "start" }]);
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
      bumpLocal: () => {},
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

const consolesOf = (posted: FromWorker[]) =>
  posted.filter((m): m is Extract<FromWorker, { kind: "console" }> => m.kind === "console");
const ledsOf = (posted: FromWorker[]) =>
  posted.filter((m): m is Extract<FromWorker, { kind: "leds" }> => m.kind === "leds").map((m) => [...m.rgb]);

describe("firmware worker motion, power, LEDs and console (Milestone 14 Task 6)", () => {
  test("accel applies the latest reading between slices", async () => {
    const { fake, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    send({ kind: "accel", mg: [10, 20, 30] });
    send({ kind: "accel", mg: [40, 50, 60] });
    // Not applied by the message itself: at the start of the next slice.
    expect(fake.accel).toEqual([]);
    sliceAt(10);
    expect(fake.accel).toEqual([[40, 50, 60]]);
    sliceAt(20);
    expect(fake.accel).toHaveLength(1);
    // While stopped it waits for the first slice after the next start.
    send({ kind: "stop" });
    send({ kind: "accel", mg: [0, -1000, 0] });
    expect(fake.accel).toHaveLength(1);
    send({ kind: "start" });
    sliceAt(30); // the slice stop() canceled: does nothing
    expect(fake.accel).toHaveLength(1);
    sliceAt(40);
    expect(fake.accel.at(-1)).toEqual([0, -1000, 0]);
  });

  test("accel rejects non-finite and clamps", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    posted.length = 0;
    const bad: unknown[] = [
      { kind: "accel", mg: [NaN, 3000, -9000] },
      { kind: "accel", mg: [Infinity, 0, 0] },
      { kind: "accel", mg: [0, 0] },
      { kind: "accel", mg: [0, 0, 0, 0] },
      { kind: "accel", mg: [0, 0, "1000"] },
      { kind: "accel", mg: "0,0,1000" },
      { kind: "accel", mg: null },
      { kind: "accel" },
    ];
    for (const m of bad) expect(() => send(m as ToWorker)).not.toThrow();
    sliceAt(10);
    expect(fake.accel).toEqual([]);
    expect(posted.filter((m) => m.kind === "error")).toEqual([]);
    send({ kind: "accel", mg: [100, 3000, -9000] });
    sliceAt(20);
    expect(fake.accel).toEqual([[100, 2000, -2000]]);
    // Truncated to integers, toward zero.
    send({ kind: "accel", mg: [12.9, -0.5, -999.99] });
    sliceAt(30);
    expect(fake.accel.at(-1)).toEqual([12, 0, -999]);
    expect(Object.is(fake.accel.at(-1)![1], 0)).toBe(true);
    // A malformed message after a valid one leaves the valid one pending.
    send({ kind: "accel", mg: [1, 2, 3] });
    send({ kind: "accel", mg: [NaN, 2, 3] });
    sliceAt(40);
    expect(fake.accel.at(-1)).toEqual([1, 2, 3]);
  });

  test("powerCycle while rebooting is ignored", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    sliceAt(10);
    posted.length = 0;
    send({ kind: "powerCycle" });
    expect(fake.powerCycles()).toBe(1);
    // A status at once: the badge is booting.
    expect(statusesOf(posted).at(-1)?.rebooting).toBe(true);
    // A second press within 500 ms, while it boots: ignored.
    send({ kind: "powerCycle" });
    sliceAt(26);
    send({ kind: "powerCycle" });
    expect(fake.powerCycles()).toBe(1);
    // The pause paid and 500 ms gone: a press counts again.
    // (The last status is the one at 522 ms: statuses go out every 250 ms.)
    for (let t = 42; t <= 538; t += 16) sliceAt(t);
    expect(statusesOf(posted).at(-1)?.rebooting).toBe(false);
    send({ kind: "powerCycle" });
    expect(fake.powerCycles()).toBe(2);
    // A soft reset's pause blocks it too, long after the last press.
    for (let t = 554; t <= 1_100; t += 16) sliceAt(t);
    fake.armReboot();
    sliceAt(1_116);
    send({ kind: "powerCycle" });
    expect(fake.powerCycles()).toBe(2);
  });

  test("powerCycle fails a running provision", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    send({ kind: "provision", identityJson: '{"a":1}' });
    fake.print("badge> ");
    sliceAt(0);
    expect(fake.typed).toEqual(["put /littlefs/identity.json 7\n"]);
    send({ kind: "powerCycle" });
    expect(statusesOf(posted).at(-1)?.provision).toEqual({ kind: "failed", reason: REBOOTED_REASON });
    // Nothing more is typed in the new boot.
    fake.print("badge> ");
    for (let t = 16; t <= 600; t += 16) sliceAt(t);
    expect(fake.typed).toHaveLength(1);
    expect(statusesOf(posted).at(-1)?.provision).toEqual({ kind: "failed", reason: REBOOTED_REASON });

    // A running skip fails the same way.
    const skip = await bootedRig();
    skip.send({ kind: "start" });
    skip.fake.print(SETUP_LINE + "badge> ");
    skip.sliceAt(0);
    skip.send({ kind: "skipSetup" });
    skip.sliceAt(10);
    expect(skip.fake.typed).toEqual([CONFIG_TYPED]);
    skip.send({ kind: "powerCycle" });
    expect(statusesOf(skip.posted).at(-1)?.skip).toEqual({ kind: "failed", reason: SKIP_REBOOTED_REASON });
  });

  test("leds posts only on change", async () => {
    const { fake, posted, transfers, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    sliceAt(0);
    sliceAt(100);
    expect(ledsOf(posted)).toEqual([]);
    fake.showLeds([1, 2, 3, 4, 5, 6]);
    sliceAt(110);
    expect(ledsOf(posted)).toEqual([[1, 2, 3, 4, 5, 6]]);
    const first = posted.find((m) => m.kind === "leds") as Extract<FromWorker, { kind: "leds" }>;
    expect(transfers[posted.indexOf(first)]).toEqual([first.rgb.buffer]);
    sliceAt(200);
    sliceAt(300);
    expect(ledsOf(posted)).toHaveLength(1);
    // Two frames inside 50 ms: only the latest goes out, on the tick.
    fake.showLeds([7, 7, 7]);
    sliceAt(310);
    fake.showLeds([9, 9, 9]);
    sliceAt(330);
    expect(ledsOf(posted)).toEqual([[1, 2, 3, 4, 5, 6], [7, 7, 7]]);
    sliceAt(350);
    sliceAt(360);
    expect(ledsOf(posted)).toEqual([[1, 2, 3, 4, 5, 6], [7, 7, 7], [9, 9, 9]]);
    // A power cycle clears the chain: the dark frame goes out once.
    send({ kind: "powerCycle" });
    sliceAt(420);
    sliceAt(500);
    expect(ledsOf(posted).slice(3)).toEqual([[]]);
    // A stopped Worker posts none.
    send({ kind: "stop" });
    fake.showLeds([1, 1, 1]);
    expect(ledsOf(posted)).toHaveLength(4);
  });

  test("console streams deltas with the boot counter", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    fake.print("hello ");
    sliceAt(0);
    expect(consolesOf(posted)).toEqual([{ kind: "console", text: "hello ", at: 0, boot: 0 }]);
    fake.print("world\r\n");
    sliceAt(100); // no status yet: no console either
    expect(consolesOf(posted)).toHaveLength(1);
    sliceAt(300);
    expect(consolesOf(posted).at(-1)).toEqual({ kind: "console", text: "world\r\n", at: 6, boot: 0 });
    // Each status carries a console chunk only when there is something new.
    sliceAt(600);
    expect(consolesOf(posted)).toHaveLength(2);
    // A soft reset: the old boot's tail, then the new boot at once.
    fake.print("bye\r\n");
    fake.armReboot();
    sliceAt(610);
    expect(consolesOf(posted).slice(2)).toEqual([
      { kind: "console", text: "bye\r\n", at: 13, boot: 0 },
      { kind: "console", text: "", at: 18, boot: 1 },
    ]);
    fake.print("new\r\n");
    for (let t = 626; t <= 1_200; t += 16) sliceAt(t);
    expect(consolesOf(posted).at(-1)).toEqual({ kind: "console", text: "new\r\n", at: 18, boot: 1 });
    // A power cycle: the old boot's tail goes out, then boot 2 from offset 0.
    fake.print("tail");
    send({ kind: "powerCycle" });
    expect(consolesOf(posted).slice(-2)).toEqual([
      { kind: "console", text: "tail", at: 23, boot: 1 },
      { kind: "console", text: "", at: 0, boot: 2 },
    ]);
    fake.print("fresh\r\n");
    for (let t = 1_216; t <= 1_800; t += 16) sliceAt(t);
    expect(consolesOf(posted).at(-1)).toEqual({ kind: "console", text: "fresh\r\n", at: 0, boot: 2 });
    // The chunks are contiguous: together they are each boot's whole console.
    const byBoot = new Map<number, string>();
    for (const c of consolesOf(posted)) byBoot.set(c.boot, (byBoot.get(c.boot) ?? "") + c.text);
    expect([...byBoot]).toEqual([
      [0, "hello world\r\nbye\r\n"],
      [1, "new\r\ntail"],
      [2, "fresh\r\n"],
    ]);
  });

  test("console holds back a UTF-8 sequence cut off at the end", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    fake.print("a");
    fake.printBytes([0xe2, 0x82]); // the first two of "€"'s three bytes
    sliceAt(0);
    expect(consolesOf(posted)).toEqual([{ kind: "console", text: "a", at: 0, boot: 0 }]);
    fake.printBytes([0xac, 0x0a]);
    sliceAt(300);
    expect(consolesOf(posted).at(-1)).toEqual({ kind: "console", text: "€\n", at: 1, boot: 0 });
    // Bytes that are not UTF-8 at all still go out, once more text follows.
    fake.printBytes([0xff]);
    sliceAt(600);
    fake.print("x");
    sliceAt(900);
    expect(consolesOf(posted).map((c) => c.text).join("")).toBe("a€\n�x");
  });

  test("a reset restarts the console at 0 under a new boot", async () => {
    const { fake, posted, send, sliceAt } = await bootedRig();
    send({ kind: "start" });
    fake.print("one\r\n");
    sliceAt(0);
    send({ kind: "reset" });
    expect(consolesOf(posted).at(-1)).toEqual({ kind: "console", text: "", at: 0, boot: 1 });
  });
});

describe("firmware worker client motion, power, LEDs and console (Milestone 14 Task 6)", () => {
  test("setAcceleration posts the latest once per animation frame", () => {
    const { rt, sent, reply, animationFrame } = clientRig();
    const accels = () => sent.filter((m) => m.kind === "accel");
    rt.setAcceleration([1, 2, 3]); // before ready: held, never queued per sample
    rt.setAcceleration([4, 5, 6]);
    animationFrame();
    expect(accels()).toEqual([]);
    reply({ kind: "ready" });
    rt.start();
    animationFrame();
    expect(accels()).toEqual([{ kind: "accel", mg: [4, 5, 6] }]);
    const mg: [number, number, number] = [7, 8, 9];
    rt.setAcceleration(mg);
    mg[0] = 0; // the caller's array is copied
    rt.setAcceleration([10, 11, 12]);
    rt.setAcceleration([13, 14, 15]);
    animationFrame();
    animationFrame();
    expect(accels().slice(1)).toEqual([{ kind: "accel", mg: [13, 14, 15] }]);
  });

  test("a reading set while hidden reaches the Worker before it resumes", () => {
    const vis = fakeVisibility();
    const { rt, sent, reply, animationFrame } = clientRig({ visibility: vis.source });
    reply({ kind: "ready" });
    rt.start();
    vis.set(true);
    rt.setAcceleration([0, -1000, 0]); // the rest reading; no animation frame while hidden
    vis.set(false);
    const tail = sent.slice(-2);
    expect(tail).toEqual([{ kind: "accel", mg: [0, -1000, 0] }, { kind: "start" }]);
    animationFrame();
    expect(sent.filter((m) => m.kind === "accel")).toHaveLength(1);
  });

  test("onLeds and onConsole hear the Worker; bad messages and throwing listeners are contained", () => {
    const { rt, reply } = clientRig();
    reply({ kind: "ready" });
    const leds: number[][] = [];
    const chunks: Array<{ text: string; boot: number }> = [];
    const offLeds = rt.onLeds((rgb) => leds.push([...rgb]));
    rt.onConsole((c) => chunks.push(c));
    let offThrowing = () => {};
    const original = console.error;
    console.error = () => {};
    try {
      offThrowing = rt.onLeds(() => {
        throw new Error("listener gone");
      });
      reply({ kind: "leds", rgb: new Uint8Array([1, 2, 3]) });
      reply({ kind: "leds", rgb: [1, 2, 3] } as unknown as FromWorker);
      reply({ kind: "console", text: "hi\n", at: 0, boot: 0 });
      reply({ kind: "console", text: 5, at: 0, boot: 0 } as unknown as FromWorker);
    } finally {
      console.error = original;
    }
    expect(leds).toEqual([[1, 2, 3]]);
    expect(chunks).toEqual([{ text: "hi\n", boot: 0 }]);
    offLeds();
    offThrowing();
    reply({ kind: "leds", rgb: new Uint8Array([4]) });
    expect(leds).toHaveLength(1);
  });

  test("the client has no shake or bump", () => {
    const { rt } = clientRig();
    expect("shake" in rt).toBe(false);
    expect("bump" in rt).toBe(false);
  });
});

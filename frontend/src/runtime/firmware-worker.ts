/**
 * The firmware Worker (Milestone 7 Task 8): owns the WASM emulator and the
 * whole of `./firmware-runtime.ts`'s `FirmwareRuntime` (pacing, the button
 * tap queue, shake schedule, provisioner), so the main thread does no
 * emulation. `./firmware-worker-client.ts` is the main-thread side; the
 * messages are `./firmware-worker-protocol.ts`.
 *
 * ## Slices
 *
 * The runtime's frame loop runs as slices. `requestAnimationFrame` is not
 * available in every browser's dedicated workers, so a slice is scheduled
 * by {@link WorkerHost.schedule}: `setTimeout(…, 0)` after a slice that
 * reached its emulated-time target, and an immediate yield (a
 * `MessageChannel` message, which the HTML timer clamp does not apply to)
 * after a slice the step cap stopped short, ruling R-T8-1. Either way the
 * slice yields to the event loop, so button and `frameAck` messages are
 * handled between slices: a press reaches the emulator at the start of the
 * next slice.
 *
 * Each slice keeps Milestone 6's pacing rule (`stepFirmwareFrame`): the
 * wall delta since the previous slice, read from {@link WorkerHost.now}, is
 * clamped to `MAX_FRAME_DELTA_MS`, the work is capped at
 * `WORKER_STEPS_PER_SLICE` steps, and missed time is dropped. A Worker in a
 * backgrounded tab that the browser throttled therefore comes back to one
 * 50 ms slice, not a catch-up burst.
 *
 * ## Frames and status
 *
 * A slice with no frame in flight posts the current framebuffer, its
 * buffer transferred; the next frame waits for the client's `frameAck`.
 * `status` (speed, provisioning state and the software reset count) goes
 * out at most every `STATUS_INTERVAL_MS`, and at once, outside that
 * throttle, when the reset count changed since the last `status`
 * (Milestone 11 Task 8), so the status line never shows the old boot's
 * state for up to 250 ms.
 *
 * ## The radio (Milestone 10 Task 9)
 *
 * `setMac` is applied once, before the first `start`: the firmware reads
 * the factory MAC at boot. Only a `setFactoryMac` that returns `true`
 * locks it; a refused or failed one leaves the next `setMac` to try
 * (Milestone 11 Task 11). After a slice that ends `ADV_REPOST_MS` or more
 * after the last `advOut` (R-PF-3's 10 Hz bound), the Worker reads the
 * badge's advertisement and posts it if it changed, or is unchanged and
 * non-empty (the re-post). A change inside the window
 * is not posted at once: the next post, on the tick, carries the latest
 * advertisement, so changes between are coalesced and the last is never
 * dropped (Milestone 11 Task 11). A stopped Worker runs no slices, so it
 * posts none. `advIn` is checked (a `Uint8Array` of at most
 * `MAX_ADVERTISEMENT_BYTES`, an integer RSSI in -128..127) before the
 * emulator sees it; the emulator's decoder rejects the rest.
 *
 * {@link handleMessage} is pure apart from its `WorkerHost`, so it is
 * tested under `bun test` without a real Worker (`test/firmware-worker.test.ts`);
 * the `self.onmessage` wiring at the bottom runs only inside a Worker.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";
import {
  createFirmwareRuntime,
  type FirmwareRuntime,
  isButtonName,
  WORKER_STEPS_PER_SLICE,
} from "./firmware-runtime";
import { MAX_ADVERTISEMENT_BYTES } from "../radio/hub";
import { ADV_REPOST_MS, type FromWorker, STATUS_INTERVAL_MS, type ToWorker } from "./firmware-worker-protocol";

export type Post = (m: FromWorker, transfer?: Transferable[]) => void;

/** What the Worker needs from its environment; injectable for tests. */
export interface WorkerHost {
  /** Boots the emulator from the WASM module's bytes and `firmware.bin`. */
  createHandle(
    wasm: ArrayBuffer,
    image: ArrayBuffer,
  ): FirmwareEmulatorHandle | Promise<FirmwareEmulatorHandle>;
  /** Wall-clock time in ms. */
  now(): number;
  /**
   * Runs `cb` after yielding to the event loop: without a timer clamp when
   * `immediate`, else via `setTimeout(…, 0)`.
   */
  schedule(cb: () => void, immediate: boolean): void;
}

export interface WorkerState {
  readonly host: WorkerHost;
  handle: FirmwareEmulatorHandle | null;
  runtime: FirmwareRuntime | null;
  initializing: boolean;
  disposed: boolean;
  /** A posted frame has not been acknowledged yet. */
  frameInFlight: boolean;
  /** The last slice was stopped by the step cap (yield without a clamp). */
  lastCapped: boolean;
  /** `host.now()` of the last `status`, or `null` (none yet). */
  lastStatusMs: number | null;
  /** The `softResets` the last `status` carried. */
  lastStatusSoftResets: number;
  /** The token of the one live scheduled slice; `0` when none. */
  liveSlice: number;
  sliceSeq: number;
  /** A MAC was accepted, or the runtime was started: `setMac` is ignored from now on. */
  macLocked: boolean;
  /** The advertisement last posted as `advOut` (empty: none, or not advertising). */
  lastAdv: Uint8Array;
  /** `host.now()` of the last `advOut`, or `null` (none yet). */
  lastAdvMs: number | null;
}

export function createWorkerState(host: WorkerHost): WorkerState {
  return {
    host,
    handle: null,
    runtime: null,
    initializing: false,
    disposed: false,
    frameInFlight: false,
    lastCapped: false,
    lastStatusMs: null,
    lastStatusSoftResets: 0,
    liveSlice: 0,
    sliceSeq: 0,
    macLocked: false,
    lastAdv: new Uint8Array(0),
    lastAdvMs: null,
  };
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? (e.stack ?? e.message) : String(e);
}

/** The whole-buffer `ArrayBuffer` of `fb`, copied only if `fb` is a view into a larger one. */
function ownBuffer(fb: Uint16Array): ArrayBuffer {
  if (fb.byteOffset === 0 && fb.byteLength === fb.buffer.byteLength && fb.buffer instanceof ArrayBuffer) {
    return fb.buffer;
  }
  return fb.slice().buffer as ArrayBuffer;
}

/**
 * Posts the badge's latest advertisement at most every `ADV_REPOST_MS`:
 * on the tick, if it changed since the last post, or to re-post it while
 * it advertises.
 */
function pollAdvertisement(state: WorkerState, handle: FirmwareEmulatorHandle, post: Post, t: number): void {
  if (state.lastAdvMs !== null && t - state.lastAdvMs < ADV_REPOST_MS) return;
  const adv = handle.radioAdvertisement();
  if (adv.length === 0 && state.lastAdv.length === 0) return;
  state.lastAdv = adv;
  state.lastAdvMs = t;
  // Not transferred: the Worker keeps it to compare, and it is small.
  post({ kind: "advOut", bytes: adv.slice() });
}

/** A peer's advertisement as the emulator takes it: bytes within bounds, an `i8` RSSI. */
function isAdvIn(msg: { bytes?: unknown; rssi?: unknown }): msg is { bytes: Uint8Array; rssi: number } {
  return (
    msg.bytes instanceof Uint8Array &&
    msg.bytes.length <= MAX_ADVERTISEMENT_BYTES &&
    typeof msg.rssi === "number" &&
    Number.isInteger(msg.rssi) &&
    msg.rssi >= -128 &&
    msg.rssi <= 127
  );
}

function buildRuntime(state: WorkerState, handle: FirmwareEmulatorHandle, post: Post): FirmwareRuntime {
  const { host } = state;
  const rt: FirmwareRuntime = createFirmwareRuntime(
    handle,
    (fb, width, height) => {
      if (state.frameInFlight) return;
      state.frameInFlight = true;
      const pixels = ownBuffer(fb);
      post({ kind: "frame", pixels, width, height }, [pixels]);
    },
    {
      stepsPerFrame: WORKER_STEPS_PER_SLICE,
      now: () => host.now(),
      onFrame: (r) => {
        state.lastCapped = r.capped;
        const t = host.now();
        pollAdvertisement(state, handle, post, t);
        const softResets = handle.softResetCount();
        if (
          state.lastStatusMs === null ||
          t - state.lastStatusMs >= STATUS_INTERVAL_MS ||
          softResets !== state.lastStatusSoftResets
        ) {
          state.lastStatusMs = t;
          state.lastStatusSoftResets = softResets;
          post({ kind: "status", speed: rt.speed(), provision: rt.provisionState(), softResets });
        }
      },
      scheduleFrame: (cb) => {
        const token = ++state.sliceSeq;
        state.liveSlice = token;
        host.schedule(() => {
          // A slice canceled by stop() (or replaced by a restart) never runs.
          if (state.liveSlice !== token) return;
          try {
            cb(0);
          } catch (e) {
            post({ kind: "error", message: `firmware slice failed: ${errorMessage(e)}` });
            state.liveSlice = 0;
            try {
              rt.stop();
            } catch {
              // The emulator already failed; nothing more to release.
            }
          }
        }, state.lastCapped);
        return token;
      },
      cancelFrame: (token) => {
        if (state.liveSlice === token) state.liveSlice = 0;
      },
    },
  );
  return rt;
}

function startInit(state: WorkerState, msg: Extract<ToWorker, { kind: "init" }>, post: Post): void {
  if (state.handle || state.initializing || state.disposed) return;
  state.initializing = true;
  Promise.resolve()
    .then(() => state.host.createHandle(msg.wasm, msg.image))
    .then((handle) => {
      state.initializing = false;
      if (state.disposed) {
        handle.dispose();
        return;
      }
      state.handle = handle;
      state.runtime = buildRuntime(state, handle, post);
      post({ kind: "ready" });
    })
    .catch((e: unknown) => {
      state.initializing = false;
      post({ kind: "error", message: `firmware boot failed: ${errorMessage(e)}` });
    });
}

/**
 * Applies one message from the main thread. Never throws: a malformed
 * message is ignored, and an emulator failure is posted as `error`.
 * Messages other than `init` before the emulator is ready are ignored (the
 * client queues them until `ready`).
 */
export function handleMessage(state: WorkerState, msg: ToWorker, post: Post): void {
  if (typeof msg !== "object" || msg === null) return;
  try {
    if (msg.kind === "init") {
      startInit(state, msg, post);
      return;
    }
    const rt = state.runtime;
    if (!rt) return;
    switch (msg.kind) {
      case "start":
        // A fresh start has no previous slice to have been capped.
        state.lastCapped = false;
        state.macLocked = true;
        rt.start();
        break;
      case "stop":
        rt.stop();
        break;
      case "button":
        if (isButtonName(msg.button)) rt.injectButton(msg.button, msg.pressed === true ? "pressed" : "released");
        break;
      case "provision":
        if (typeof msg.identityJson === "string") rt.provision(msg.identityJson);
        break;
      case "shake":
        rt.shake();
        break;
      case "bump":
        rt.bump();
        break;
      case "setMac":
        // Once, before the first start: the firmware reads it at boot.
        if (state.macLocked || !(msg.mac instanceof Uint8Array) || msg.mac.length !== 6) break;
        // Locked only when the emulator took it; a refusal (or a throw) leaves it open.
        if (state.handle?.setFactoryMac(msg.mac) === true) state.macLocked = true;
        break;
      case "advIn":
        if (isAdvIn(msg)) state.handle?.deliverAdvertisement(msg.bytes, msg.rssi);
        break;
      case "reset":
        rt.reset();
        break;
      case "frameAck":
        state.frameInFlight = false;
        break;
      case "dispose":
        state.disposed = true;
        state.runtime = null;
        state.handle = null;
        rt.dispose();
        break;
      default:
        break;
    }
  } catch (e) {
    post({ kind: "error", message: errorMessage(e) });
  }
}

/** The real Worker's host: the WASM bridge, `performance.now`, timers. */
export function browserWorkerHost(): WorkerHost {
  let channel: MessageChannel | null = null;
  let pending: (() => void) | null = null;
  return {
    async createHandle(wasm, image) {
      // Imported lazily, as main.ts did before the Worker: the bridge pulls
      // in the generated wasm-pkg glue.
      const { createFirmwareEmulator, initCpuWasmSync } = await import("../cpu/bridge");
      initCpuWasmSync(wasm);
      return createFirmwareEmulator(new Uint8Array(image));
    },
    now: () => performance.now(),
    schedule(cb, immediate) {
      if (!immediate || typeof MessageChannel !== "function") {
        setTimeout(cb, 0);
        return;
      }
      if (!channel) {
        channel = new MessageChannel();
        channel.port1.onmessage = () => {
          const run = pending;
          pending = null;
          run?.();
        };
      }
      // At most one slice is ever scheduled (the runtime reschedules only
      // from the running slice), so one pending slot suffices.
      pending = cb;
      channel.port2.postMessage(null);
    },
  };
}

interface DedicatedWorkerScope {
  postMessage(m: FromWorker, transfer: Transferable[]): void;
  onmessage: ((e: MessageEvent<ToWorker>) => void) | null;
}

if (typeof (globalThis as { WorkerGlobalScope?: unknown }).WorkerGlobalScope !== "undefined") {
  const scope = self as unknown as DedicatedWorkerScope;
  const state = createWorkerState(browserWorkerHost());
  const post: Post = (m, transfer = []) => scope.postMessage(m, transfer);
  scope.onmessage = (e) => handleMessage(state, e.data, post);
}

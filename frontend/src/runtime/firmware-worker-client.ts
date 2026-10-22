/**
 * The main-thread side of the firmware Worker (Milestone 7 Task 8): a
 * `FirmwareRuntime` whose methods post messages to `./firmware-worker.ts`
 * and whose synchronous getters (`speed()`, `provisionState()`) return the
 * last `status` the Worker posted, so `main.ts` and `shell.ts` use it
 * exactly as they used the in-thread runtime.
 *
 * - `init` (the WASM bytes and `factory.bin`, transferred) is posted at
 *   once; every other message sent before the Worker's `ready` is queued
 *   and posted in order on `ready`. A press, a release and a `stop()` sent
 *   while the Worker boots therefore reach it in that order, and the
 *   Worker's `stop()` flushes the deferred release (no stranded button).
 * - Frames are painted on the next animation frame and then acknowledged
 *   (`frameAck`); only the latest is painted. While stopped, a frame is
 *   dropped and acknowledged at once, and `stop()` drops and acknowledges
 *   one waiting to be painted, so the Worker never waits on a frame the
 *   client will not paint, and a firmware frame never lands on the canvas
 *   after a switch to Lua mode.
 * - While the page is hidden (`document.visibilitychange`) a started
 *   runtime is paused: the client posts `stop` (which flushes any
 *   deferred release) and posts `start` again when the page is visible.
 *   A dedicated Worker is not throttled like main-thread
 *   `requestAnimationFrame`, so without this the firmware (Snake) would
 *   keep running unseen; this restores Milestone 6's pause (ruling
 *   R-T8-10). A runtime the user stopped (a mode switch) stays stopped.
 * - `provision()` and `reset()` update the cached provisioning state (and
 *   `reset()` the speed) at once, as the in-thread runtime did; the next
 *   `status` replaces them. A `status` the Worker posted before it handled
 *   the message can show the old state for up to one status interval.
 */
import type { ButtonName } from "../badge/input";
import { blitFramebuffer } from "../render/framebuffer";
import type { FirmwareRuntime, FrameCanceler, FrameScheduler } from "./firmware-runtime";
import type { FromWorker, ToWorker } from "./firmware-worker-protocol";
import type { ProvisionState } from "./provisioner";

export interface WorkerLike {
  postMessage(m: ToWorker, transfer?: Transferable[]): void;
  onmessage: ((e: { data: FromWorker }) => void) | null;
  terminate(): void;
}

/** Where the client learns whether the page is hidden; injectable for tests. */
export interface VisibilitySource {
  hidden(): boolean;
  /** Calls `cb` whenever visibility changes; returns an unsubscribe function. */
  subscribe(cb: () => void): () => void;
}

/** `document`'s visibility, or a never-hidden source where there is no `document`. */
export function documentVisibility(): VisibilitySource {
  if (typeof document === "undefined") return { hidden: () => false, subscribe: () => () => {} };
  return {
    hidden: () => document.visibilityState === "hidden",
    subscribe(cb) {
      document.addEventListener("visibilitychange", cb);
      return () => document.removeEventListener("visibilitychange", cb);
    },
  };
}

export interface WorkerClientOptions {
  /** Injectable for testing; defaults to {@link documentVisibility}. */
  visibility?: VisibilitySource;
  /** Injectable for testing; defaults to the real `requestAnimationFrame`. */
  scheduleFrame?: FrameScheduler;
  /** Injectable for testing; defaults to the real `cancelAnimationFrame`. */
  cancelFrame?: FrameCanceler;
}

/** The client runtime, plus a promise that settles when the Worker has booted the emulator (or failed to). */
export interface WorkerFirmwareRuntime extends FirmwareRuntime {
  readonly ready: Promise<void>;
}

export function createWorkerFirmwareRuntime(
  worker: WorkerLike,
  ctx: CanvasRenderingContext2D,
  wasm: ArrayBuffer,
  image: ArrayBuffer,
  options: WorkerClientOptions = {},
): WorkerFirmwareRuntime {
  const scheduleFrame: FrameScheduler = options.scheduleFrame ?? ((cb) => requestAnimationFrame(cb));
  const cancelFrame: FrameCanceler = options.cancelFrame ?? ((id) => cancelAnimationFrame(id));

  let isReady = false;
  let disposed = false;
  /** The user started the runtime (and has not stopped it). */
  let running = false;
  /** Running, but paused because the page is hidden. */
  let pausedHidden = false;
  const visibility = options.visibility ?? documentVisibility();
  const queue: ToWorker[] = [];
  let lastSpeed = 0;
  let lastProvision: ProvisionState | null = null;
  let pendingFrame: Extract<FromWorker, { kind: "frame" }> | null = null;
  let paintHandle: number | null = null;

  let resolveReady!: () => void;
  let rejectReady!: (e: Error) => void;
  const ready = new Promise<void>((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  // Callers await it; this only keeps an unawaited failure from being reported twice.
  ready.catch(() => {});

  function send(m: ToWorker): void {
    if (disposed) return;
    if (isReady) worker.postMessage(m);
    else queue.push(m);
  }

  function paint(): void {
    paintHandle = null;
    const f = pendingFrame;
    if (!f) return;
    pendingFrame = null;
    blitFramebuffer(ctx, new Uint16Array(f.pixels), f.width, f.height);
    send({ kind: "frameAck" });
  }

  function dropPendingFrame(): void {
    if (paintHandle !== null) {
      cancelFrame(paintHandle);
      paintHandle = null;
    }
    if (pendingFrame) {
      pendingFrame = null;
      send({ kind: "frameAck" });
    }
  }

  worker.onmessage = (e) => {
    if (disposed) return;
    const msg = e.data;
    switch (msg.kind) {
      case "ready": {
        isReady = true;
        for (const m of queue.splice(0)) worker.postMessage(m);
        resolveReady();
        break;
      }
      case "frame": {
        if (!running || pausedHidden) {
          send({ kind: "frameAck" });
          break;
        }
        pendingFrame = msg;
        paintHandle ??= scheduleFrame(paint);
        break;
      }
      case "status":
        lastSpeed = msg.speed;
        lastProvision = msg.provision;
        break;
      case "error":
        console.error("firmware worker:", msg.message);
        if (!isReady) rejectReady(new Error(msg.message));
        break;
    }
  };

  worker.postMessage({ kind: "init", wasm, image }, [wasm, image]);

  const unsubscribeVisibility = visibility.subscribe(() => {
    if (disposed || !running) return;
    const hidden = visibility.hidden();
    if (hidden && !pausedHidden) {
      pausedHidden = true;
      send({ kind: "stop" });
      dropPendingFrame();
    } else if (!hidden && pausedHidden) {
      pausedHidden = false;
      send({ kind: "start" });
    }
  });

  return {
    ready,
    start(): void {
      if (disposed) throw new Error("runtime/firmware-worker-client: cannot start a disposed runtime");
      if (running) return;
      running = true;
      pausedHidden = visibility.hidden();
      if (!pausedHidden) send({ kind: "start" });
    },
    stop(): void {
      running = false;
      pausedHidden = false;
      // Sent even when paused or never started: stop() also flushes deferred releases.
      send({ kind: "stop" });
      dropPendingFrame();
    },
    injectButton(button: ButtonName, kind: "pressed" | "released"): void {
      send({ kind: "button", button, pressed: kind === "pressed" });
    },
    provision(identityJson: string): void {
      lastProvision = { kind: "waiting" };
      send({ kind: "provision", identityJson });
    },
    provisionState: () => lastProvision,
    shake(): void {
      send({ kind: "shake" });
    },
    reset(): void {
      lastProvision = null;
      lastSpeed = 0;
      send({ kind: "reset" });
    },
    speed: () => lastSpeed,
    isExitRequested: () => false,
    dispose(): void {
      if (disposed) return;
      running = false;
      unsubscribeVisibility();
      dropPendingFrame();
      if (isReady) worker.postMessage({ kind: "dispose" });
      disposed = true;
      worker.onmessage = null;
      worker.terminate();
      rejectReady(new Error("runtime/firmware-worker-client: disposed before the worker was ready"));
    },
  };
}

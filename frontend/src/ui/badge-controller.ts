/**
 * The page's controller store (Milestone 14 Task 12; from Milestone 11
 * Task 12's controller): non-visual state that the React page reads
 * through {@link BadgeController.subscribe} and
 * {@link BadgeController.getSnapshot} (`useSyncExternalStore`). Everything
 * a test must replace comes in through {@link BadgeControllerDeps};
 * `main.tsx` passes the browser's, and `test/badge-controller.test.ts`
 * drives it without a DOM.
 *
 * - **The firmware Worker**, booted at once once this tab holds the badge
 *   (or on a provision), and kept until unload. The emulator runs in a
 *   dedicated Worker (`runtime/firmware-worker.ts`); this thread fetches
 *   the bytes, blits the frames the Worker posts into the badge's screen
 *   canvas ({@link BadgeController.setCanvas}) and forwards input. A failed
 *   boot is not cached: Run here or a provision retries it.
 * - **One badge per browser** (`radio/lock.ts`, Milestone 10 Task 9): the
 *   firmware runs only in the tab holding the `htn-badge` Web Lock.
 *   Another tab shows the tab notice (`tabNotice`), whose Run here takes
 *   the badge over (the old tab stops). The badge's factory MAC is this
 *   browser's, sent before its first start.
 * - **The hub's bump gate** (Milestone 11 Task 11): each start and stop of
 *   the runtime is paired with `hub.setRunning`, and the hidden-page pause
 *   pauses the hub (`onPausedChange`).
 * - **Provisioning and Skip setup** (Milestones 5 and 13): `provision`
 *   boots and starts the badge if needed, then types the role's identity;
 *   `skipSetup` is refused unless the badge runs here and its Setup app is
 *   on screen (Milestone 13's R-FR-1). A boot failure says
 *   {@link START_FAILED_NOTE}, and an identity that cannot be fetched says
 *   `could not load <role>`.
 * - **CONSOLE.LOG** ({@link BadgeController.console}): the Worker's serial
 *   chunks go to `appendSerial`. The page's status (today's status-line
 *   texts) goes to `appendPage` as it changes: `rebooting…` when a reboot
 *   starts, the provision's state (`waiting for the console…`,
 *   `provisioning…`, its `PROV OK` line, `failed: <reason>`), the skip's
 *   (`skipping setup…`, `skip failed: <reason>`; `done` adds nothing, the
 *   screen shows it), failures, links, and phone motion. Each stream is
 *   logged when its own text changes, so an old skip failure the Worker
 *   keeps reporting is not repeated under a newer provision.
 * - **Motion** (spec "Phone motion"): exactly one source drives the
 *   sensor, through {@link BadgeController.motionSink}. `motion` is
 *   `"physics"` (the stage's lanyard) or `"phone"` (DeviceMotion, after
 *   `usePhoneMotion(true)` is granted; asked synchronously, so iOS sees the
 *   click's gesture). **While the page is hidden** the sensor gets
 *   {@link HANGING_REST_MG} and the sink drops samples until it is visible
 *   (Review Focus 1); the client holds that reading and posts it just
 *   before the badge's `start` when the page is visible again (Task 6).
 * - **Links** (`radio/links.ts`): `host`, `cancelHost` and `join`, with
 *   the summary as `linkStatus`.
 *
 * Status comes from the client's last `status`, which `refreshStatus`
 * reads: `main.tsx` polls it (as the old `main.ts` did), and every action
 * here calls it.
 */
import { BOARD_AXES, type AxisMap } from "../motion/axes";
import {
  createDeviceMotionSource,
  motionAvailable,
  platformSignFix,
  requestMotionPermission,
} from "../motion/device-motion";
import { REST_MG } from "../motion/sensor";
import type { Mg, MotionSource } from "../motion/source";
import type { RadioHub } from "../radio/hub";
import { createLinks, type LinksOptions } from "../radio/links";
import { acquireBadge, type BadgeHold, type LockManagerLike, takeOver } from "../radio/lock";
import type { SignalDeps } from "../radio/signal-client";
import {
  createWorkerFirmwareRuntime,
  documentVisibility,
  type WorkerClientOptions,
  type WorkerFirmwareRuntime,
  type WorkerLike,
} from "../runtime/firmware-worker-client";
import type { ProvisionState } from "../runtime/provisioner";
import type { SkipState } from "../runtime/setup-skip";
import type { BadgeClient } from "./Badge";
import { type ConsoleLog, createConsoleLog } from "./console-log";

/** The Worker the controller boots: a {@link WorkerLike} that also reports its own load failure. */
export interface BadgeWorker extends WorkerLike {
  addEventListener(type: "error", cb: (e: { message: string }) => void, options: { once: true }): void;
}

/** Phone motion's browser seams; the real DeviceMotion when omitted. */
export interface PhoneMotionDeps {
  available(): boolean;
  /** Asks for access; called synchronously from the click. */
  request(): Promise<"granted" | "denied" | "unsupported">;
  create(): MotionSource;
}

export interface BadgeControllerDeps {
  hub: Pick<RadioHub, "fromLocal" | "setPaused" | "setRunning" | "addLink" | "removeLink">;
  /** Fetches the WASM module and `firmware.bin`. */
  loadImages(): Promise<{ wasm: ArrayBuffer; image: ArrayBuffer }>;
  /** Constructs the firmware Worker; called only once the badge is held (or provisioning). */
  createWorker(): BadgeWorker;
  /** This browser's factory MAC. */
  mac(): Uint8Array;
  /** A test identity's `identity.json` text. */
  fetchIdentity(role: string): Promise<string>;
  /** The Web Locks to hold the badge with; the browser's when omitted. */
  locks?: LockManagerLike | null;
  /** Reaches `/api/signal` and WebRTC; the browser's when omitted. */
  signal?: SignalDeps;
  /** Where `pagehide` comes from (it closes the links); `window` when omitted. */
  pageEvents?: LinksOptions["pageEvents"];
  /** The Worker client's visibility and frame scheduling; the document's when omitted. */
  clientOptions?: Pick<WorkerClientOptions, "visibility" | "scheduleFrame" | "cancelFrame">;
  motion?: PhoneMotionDeps;
}

/** The page's state, as the components read it. A new object on every change. */
export interface BadgeSnapshot {
  /** The emulated/wall ratio (0 before the first status). */
  speed: number;
  /** The badge is in a reboot's pause. */
  rebooting: boolean;
  provision: ProvisionState | null;
  skip: SkipState | null;
  /** Setup is on screen and the badge runs in this tab. */
  setupOnScreen: boolean;
  /** Another tab runs the badge ("Badge running in another tab"). */
  tabNotice: boolean;
  /** BADGE LINK's one-line summary. */
  linkStatus: string;
  /** The hosted room waiting for a peer, or null (its button cancels it). */
  hostingRoom: string | null;
  motion: "physics" | "phone";
  phoneMotionAvailable: boolean;
}

/** The page line after the firmware failed to boot. */
export const START_FAILED_NOTE = "could not start the badge";

/** The board's hanging rest (1 g up, nothing in-plane or out), in the chip's axes. */
export function hangingRest(axes: AxisMap = BOARD_AXES): Mg {
  const out: Mg = [0, 0, 0];
  const [axis, sign] = axes.up;
  out[axis] = REST_MG * sign;
  return out;
}

/** What the sensor reads while the page is hidden: `[0, 1000, 0]` through `BOARD_AXES`. */
export const HANGING_REST_MG: Mg = hangingRest();

export interface BadgeController {
  subscribe(cb: () => void): () => void;
  getSnapshot(): BadgeSnapshot;
  /** The booted firmware runtime, or `null` before the first boot. */
  firmware(): WorkerFirmwareRuntime | null;
  /** The board's client: the firmware's input, power and LEDs (dropped before its boot). */
  readonly client: BadgeClient;
  readonly console: ConsoleLog;
  /** The badge's screen canvas (the stage's `onCanvas`); frames are dropped without one. */
  setCanvas(canvas: HTMLCanvasElement | null): void;
  provision(role: string): Promise<void>;
  skipSetup(): void;
  powerCycle(): void;
  runHere(): Promise<void>;
  host(): Promise<string>;
  cancelHost(): void;
  join(code: string): Promise<void>;
  usePhoneMotion(on: boolean): Promise<void>;
  /** The active motion source's sink (stable). */
  motionSink(): (mg: Mg) => void;
  /** Re-reads the runtime's last status into the snapshot and the console. */
  refreshStatus(): void;
  /** Page unload: disposes the runtime, frees the badge, closes the links. */
  dispose(): void;
}

function defaultMotion(): PhoneMotionDeps {
  return {
    available: motionAvailable,
    request: requestMotionPermission,
    create: () =>
      createDeviceMotionSource(
        window,
        platformSignFix(typeof navigator === "undefined" ? "" : navigator.userAgent),
      ),
  };
}

function provisionText(st: ProvisionState | null): string {
  if (!st) return "";
  if ("line" in st) return st.line;
  if ("reason" in st) return `failed: ${st.reason}`;
  return st.kind === "waiting" ? "waiting for the console…" : "provisioning…";
}

function skipText(skip: SkipState | null): string {
  if (skip?.kind === "typing") return "skipping setup…";
  if (skip?.kind === "failed") return `skip failed: ${skip.reason}`;
  return "";
}

function sameJson(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

export function createBadgeController(deps: BadgeControllerDeps): BadgeController {
  const { hub, locks } = deps;
  const log = createConsoleLog();
  const note = (text: string) => log.appendPage(text);
  const visibility = deps.clientOptions?.visibility ?? documentVisibility();
  const motionDeps = deps.motion ?? defaultMotion();

  // --- The store ---------------------------------------------------------
  const listeners = new Set<() => void>();
  /** Set by dispose(): nothing boots, starts or turns on after it. */
  let disposed = false;
  let snapshot: BadgeSnapshot = {
    speed: 0,
    rebooting: false,
    provision: null,
    skip: null,
    setupOnScreen: false,
    tabNotice: false,
    linkStatus: "",
    hostingRoom: null,
    motion: "physics",
    phoneMotionAvailable: motionDeps.available(),
  };
  function update(next: Partial<BadgeSnapshot>): void {
    let changed = false;
    for (const k of Object.keys(next) as (keyof BadgeSnapshot)[]) {
      if (!sameJson(snapshot[k], next[k])) changed = true;
    }
    if (!changed) return;
    snapshot = { ...snapshot, ...next };
    for (const cb of [...listeners]) {
      try {
        cb();
      } catch (e) {
        // One failing subscriber must not abort refreshStatus or the others.
        console.error("badge controller: listener failed:", e);
      }
    }
  }

  // --- The screen --------------------------------------------------------
  let canvas: HTMLCanvasElement | null = null;
  let ctx: CanvasRenderingContext2D | null = null;
  /** What the client blits into: the current canvas, whenever it mounts. */
  const screen: Pick<CanvasRenderingContext2D, "putImageData"> = {
    putImageData: (image: ImageData, dx: number, dy: number) => ctx?.putImageData(image, dx, dy),
  };
  function clearScreen(): void {
    if (canvas && ctx) ctx.clearRect(0, 0, canvas.width, canvas.height);
  }

  // --- The firmware Worker -----------------------------------------------
  let firmwareRuntime: WorkerFirmwareRuntime | null = null;
  let firmwareRuntimePromise: Promise<WorkerFirmwareRuntime> | null = null;
  const ledListeners = new Set<(rgb: Uint8Array) => void>();
  /** The page is hidden: the sensor holds the hanging rest. */
  let hidden = visibility.hidden();

  async function ensureFirmwareRuntime(): Promise<WorkerFirmwareRuntime> {
    if (firmwareRuntime) return firmwareRuntime;
    firmwareRuntimePromise ??= (async () => {
      const { wasm, image } = await deps.loadImages();
      const worker = deps.createWorker();
      const rt = createWorkerFirmwareRuntime(worker, screen, wasm, image, {
        ...deps.clientOptions,
        visibility,
        onAdvertisement: (bytes) => hub.fromLocal(bytes),
        onPausedChange: (paused) => hub.setPaused(paused),
      });
      // Before any start: the firmware reads its factory MAC at boot.
      rt.setMac(deps.mac());
      rt.onConsole(({ text, boot }) => log.appendSerial(text, boot));
      rt.onLeds((rgb) => {
        for (const cb of [...ledListeners]) cb(rgb);
      });
      // Booted on a hidden page: it starts from the rest, not the default.
      if (hidden) rt.setAcceleration(HANGING_REST_MG);
      try {
        await new Promise<void>((resolve, reject) => {
          rt.ready.then(resolve, reject);
          worker.addEventListener("error", (e) => reject(new Error(`firmware worker: ${e.message}`)), {
            once: true,
          });
        });
      } catch (e) {
        rt.dispose();
        throw e;
      }
      if (disposed) {
        // Unloaded while it booted: it never runs.
        rt.dispose();
        throw new Error("badge controller: disposed while booting");
      }
      firmwareRuntime = rt;
      return rt;
    })().catch((e: unknown) => {
      // A failed boot (e.g. a transient fetch failure) is not cached: the
      // next attempt (Run here, a provision) retries it.
      firmwareRuntimePromise = null;
      throw e;
    });
    return firmwareRuntimePromise;
  }

  // --- The badge lock ----------------------------------------------------
  let badgeHold: BadgeHold | null = null;
  let claiming: Promise<boolean> | null = null;
  /**
   * Whether the firmware runs in this tab, set beside each
   * `hub.setRunning`. A stopped badge posts no status, so Skip setup is
   * off on it (Milestone 13's final review, R-FR-1).
   */
  let running = false;

  /** Another tab took the badge over: stop here and offer it back. */
  function onBadgeLost(): void {
    badgeHold = null;
    firmwareRuntime?.stop();
    running = false;
    hub.setRunning(false);
    showNotice();
    refreshStatus();
  }

  /** Frees the badge for another tab (a failed start, unload). Idempotent. */
  function releaseBadge(): void {
    const hold = badgeHold;
    badgeHold = null;
    hold?.release();
  }

  /** Shows the tab notice; the screen behind it is cleared. */
  function showNotice(): void {
    clearScreen();
    update({ tabNotice: true });
  }

  /** Takes the badge if no other tab holds it; `true` if this tab holds it. */
  function claimBadge(): Promise<boolean> {
    if (badgeHold) return Promise.resolve(true);
    claiming ??= acquireBadge(onBadgeLost, locks).then((hold) => {
      claiming = null;
      badgeHold = hold;
      return hold !== null;
    });
    return claiming;
  }

  /**
   * Starts the firmware if this tab holds (or can take) the badge, else
   * shows the notice: booting it if needed and starting it if stopped.
   */
  async function startFirmware(): Promise<void> {
    if (!(await claimBadge())) {
      showNotice();
      return;
    }
    update({ tabNotice: false });
    let rt: WorkerFirmwareRuntime;
    try {
      rt = await ensureFirmwareRuntime();
    } catch (e) {
      releaseBadge(); // nothing runs here: let another tab have the badge
      throw e;
    }
    // Another tab took the badge while it booted, or it already runs.
    if (!badgeHold || running || disposed) return;
    rt.start();
    running = true;
    hub.setRunning(true);
    refreshStatus();
  }

  function startFailed(e: unknown): void {
    console.error("Failed to start the firmware:", e);
    note(START_FAILED_NOTE);
    refreshStatus();
  }

  async function runHere(): Promise<void> {
    await takeOver(locks);
    try {
      await startFirmware();
    } catch (e) {
      startFailed(e);
    }
  }

  // --- Status -> snapshot and page lines --------------------------------
  let wasRebooting = false;
  let lastProvisionText = "";
  let lastSkipText = "";

  function refreshStatus(): void {
    const rt = firmwareRuntime;
    const provision = rt?.provisionState() ?? null;
    const skip = rt?.skipState() ?? null;
    const rebooting = rt?.rebooting() ?? false;
    if (rebooting && !wasRebooting) note("rebooting…");
    wasRebooting = rebooting;
    const p = provisionText(provision);
    if (p !== lastProvisionText) {
      lastProvisionText = p;
      if (p) note(p);
    }
    const s = skipText(skip);
    if (s !== lastSkipText) {
      lastSkipText = s;
      if (s) note(s);
    }
    update({
      speed: rt?.speed() ?? 0,
      rebooting,
      provision,
      skip,
      setupOnScreen: running && rt !== null && rt.setupOnScreen(),
    });
  }

  // --- Visibility and motion ---------------------------------------------
  const sink = (mg: Mg): void => {
    if (hidden) return;
    firmwareRuntime?.setAcceleration(mg);
  };
  const unsubscribeVisibility = visibility.subscribe(() => {
    const now = visibility.hidden();
    if (now === hidden) return;
    hidden = now;
    if (hidden) firmwareRuntime?.setAcceleration(HANGING_REST_MG);
  });

  let phone: MotionSource | null = null;
  /**
   * Counts phone-motion turns, so a permission answer that arrives after
   * a later turn (off, or dispose) is ignored; `requesting` keeps a second
   * "on" click from asking again while the first is pending.
   */
  let motionSeq = 0;
  let requesting = false;

  async function usePhoneMotion(on: boolean): Promise<void> {
    if (!on) {
      motionSeq++;
      requesting = false;
      phone?.stop();
      if (snapshot.motion === "phone") note("phone motion off: the swing drives the badge");
      update({ motion: "physics" });
      return;
    }
    if (snapshot.motion === "phone" || requesting) return;
    const seq = ++motionSeq;
    requesting = true;
    // Asked before any await: iOS needs the click's user gesture.
    const pending = motionDeps.request();
    let answer: Awaited<typeof pending>;
    try {
      answer = await pending;
    } catch {
      answer = "denied";
    }
    if (seq !== motionSeq || disposed) return; // turned off meanwhile
    requesting = false;
    if (answer !== "granted") {
      note(
        answer === "denied"
          ? "phone motion was denied: the swing still drives the badge"
          : "phone motion is not available here (it needs a phone and HTTPS): the swing still drives the badge",
      );
      return;
    }
    phone ??= motionDeps.create();
    phone.start(sink);
    update({ motion: "phone" });
    note("phone motion on: the phone is the badge");
  }

  // --- Links -------------------------------------------------------------
  const links = createLinks(hub, deps.signal, { note, pageEvents: deps.pageEvents });
  const syncLinks = () => update({ linkStatus: links.summary(), hostingRoom: links.hosting() });
  const unsubscribeLinks = links.subscribe(syncLinks);

  // --- The board's client ------------------------------------------------
  const client: BadgeClient = {
    injectButton: (button, kind, source) => firmwareRuntime?.injectButton(button, kind, source),
    powerCycle: () => controller.powerCycle(),
    rebooting: () => firmwareRuntime?.rebooting() ?? false,
    onLeds(cb) {
      ledListeners.add(cb);
      return () => {
        ledListeners.delete(cb);
      };
    },
  };

  const controller: BadgeController = {
    subscribe(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    getSnapshot: () => snapshot,
    firmware: () => firmwareRuntime,
    client,
    console: log,
    setCanvas(next) {
      canvas = next;
      ctx = next?.getContext("2d") ?? null;
      if (snapshot.tabNotice) clearScreen();
    },
    async provision(role) {
      try {
        // Boots and starts the badge if it isn't running (a retry after a
        // failed first boot). Without the badge it boots nothing and shows
        // the notice: another tab runs it.
        await startFirmware();
      } catch (e) {
        startFailed(e);
        return;
      }
      const rt = firmwareRuntime;
      if (!badgeHold || !rt) return;
      let identity: string;
      try {
        identity = await deps.fetchIdentity(role);
      } catch (e) {
        console.error(`Could not load test identity ${role}:`, e);
        note(`could not load ${role}`);
        return;
      }
      rt.provision(identity);
      refreshStatus();
    },
    skipSetup() {
      // A stopped badge types nothing, and the client refuses a click
      // unless Setup is on screen (Milestone 13's R-FR-1 and R-FR-3).
      const rt = firmwareRuntime;
      if (!running || !rt?.setupOnScreen()) return;
      rt.skipSetup();
      refreshStatus();
    },
    powerCycle() {
      // The Worker's runtime ignores a press while it reboots (Review Focus 2).
      if (!running) return;
      firmwareRuntime?.powerCycle();
    },
    runHere,
    host: () => links.host(),
    cancelHost: () => links.cancelHost(),
    join: (code) => links.join(code),
    usePhoneMotion,
    motionSink: () => sink,
    refreshStatus,
    dispose() {
      if (disposed) return;
      disposed = true;
      motionSeq++;
      unsubscribeVisibility();
      unsubscribeLinks();
      phone?.stop();
      links.dispose();
      const rt = firmwareRuntime;
      firmwareRuntime = null;
      rt?.dispose();
      if (running) {
        running = false;
        hub.setRunning(false);
      }
      releaseBadge();
    },
  };

  void startFirmware().catch(startFailed);
  return controller;
}

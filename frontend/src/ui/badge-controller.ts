/**
 * The page's badge wiring, factored out of `main.ts` (Milestone 11 Task 12)
 * so `test/shell.test.ts` can drive it over a real DOM: `main.ts` runs
 * itself on import and reaches `fetch`, `Worker` and `navigator.locks`
 * directly, so it cannot be driven from a test. Everything a test must
 * replace comes in through {@link BadgeControllerDeps}; `main.ts` passes
 * the browser's.
 *
 * - {@link createBadgeController}: the firmware Worker (booted at once,
 *   once this tab holds the badge), the one-badge-per-browser lock and its notice, the
 *   provisioning controls, and the bump gate's `hub.setRunning` calls.
 * - {@link mountLinks}: the radio link panel and its links, with the
 *   hosted room's cancel.
 *
 * **The radio (Milestone 10 Task 9).** One badge per browser: the firmware
 * runs only in the tab holding the `htn-badge` Web Lock
 * (`src/radio/lock.ts`); another tab shows "Badge running in another tab —
 * Run here", whose button takes the badge over (the old tab stops). The
 * badge's factory MAC is this browser's, sent to the Worker before its
 * first start. The hub's bump gate is open only while the badge runs here
 * and the page is visible: each `rt.start()` and `firmwareRuntime?.stop()`
 * below is paired with `hub.setRunning`, and the hidden-page pause pauses
 * the hub (`onPausedChange`), Milestone 11 Task 11.
 *
 * **Links (Milestone 10 Task 10).** The link panel ({@link mountLinks})
 * makes WebRTC links (`src/radio/peer-link.ts`) to other browsers: "Host
 * link" creates a room on `/api/signal` and shows its code, and "Join
 * link" joins one. Each link joins the hub as soon as it exists; it carries
 * nothing until its channel opens, and leaves the hub when it closes.
 * Cancelling a hosted room ({@link mountLinks}'s `cancelHosting`: hosting
 * again, unload) also closes the room on the server, best effort
 * (Milestone 11 Task 10).
 */
import type { RadioHub } from "../radio/hub";
import { acquireBadge, type BadgeHold, type LockManagerLike, takeOver } from "../radio/lock";
import type { PeerLink } from "../radio/peer-link";
import {
  type HostedRoom,
  SignalError,
  type SignalDeps,
  hostRoom,
  joinRoom,
} from "../radio/signal-client";
import {
  createWorkerFirmwareRuntime,
  type WorkerFirmwareRuntime,
  type WorkerLike,
} from "../runtime/firmware-worker-client";
import type { ButtonInjector, LinkPanel, ProvisionControls, ShellHandles } from "./shell";

/** The Worker the controller boots: a {@link WorkerLike} that also reports its own load failure. */
export interface BadgeWorker extends WorkerLike {
  addEventListener(type: "error", cb: (e: { message: string }) => void, options: { once: true }): void;
}

export interface BadgeControllerDeps {
  shell: Pick<ShellHandles, "ctx" | "renderLeds" | "mountBadgeNotice" | "mountProvisionControls">;
  hub: Pick<RadioHub, "fromLocal" | "setPaused" | "setRunning">;
  /** The firmware roles the provisioning controls offer. */
  roles: readonly string[];
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
}

/** The status line after the firmware failed to boot. */
export const START_FAILED_NOTE = "could not start the badge";

export interface BadgeController {
  /** The booted firmware runtime, or `null` before the first boot. */
  firmware(): WorkerFirmwareRuntime | null;
  /** The button pad's and keyboard's input, to the firmware (dropped before its boot). */
  inject: ButtonInjector;
  /** The provisioning controls' visibility and status line (`main.ts` polls it). */
  refreshStatus(): void;
  /** Page unload: disposes the runtime and frees the badge. */
  dispose(): void;
}

/**
 * Mounts the badge notice and the provisioning controls, and starts the
 * firmware: it claims the badge and, once this tab holds it, boots the
 * Worker (or shows the notice if another tab holds it).
 */
export function createBadgeController(deps: BadgeControllerDeps): BadgeController {
  const { shell, hub } = deps;
  const { locks } = deps;

  // The Worker is created only once this tab holds the badge (or a
  // provision asks for it), and kept until unload. Since Milestone 7 Task 8
  // the emulator runs in a dedicated Worker (`runtime/firmware-worker.ts`);
  // this thread only fetches the bytes, paints the frames the Worker posts
  // and forwards input (`runtime/firmware-worker-client.ts`). The Worker
  // imports `./cpu/bridge` lazily, so the WASM loads only inside it.
  let firmwareRuntime: WorkerFirmwareRuntime | null = null;
  let firmwareRuntimePromise: Promise<WorkerFirmwareRuntime> | null = null;

  async function ensureFirmwareRuntime(): Promise<WorkerFirmwareRuntime> {
    if (firmwareRuntime) return firmwareRuntime;
    firmwareRuntimePromise ??= (async () => {
      const { wasm, image } = await deps.loadImages();
      const worker = deps.createWorker();
      const rt = createWorkerFirmwareRuntime(worker, shell.ctx, wasm, image, {
        onAdvertisement: (bytes) => hub.fromLocal(bytes),
        onPausedChange: (paused) => hub.setPaused(paused),
      });
      // Before any start: the firmware reads its factory MAC at boot.
      rt.setMac(deps.mac());
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
      firmwareRuntime = rt;
      return rt;
    })().catch((e: unknown) => {
      // Don't let a rejected boot attempt (e.g. a transient fetch failure)
      // stay cached forever -- reset the slot so the next attempt (Run
      // here, a provision) retries the fetch/boot instead of replaying this same rejection.
      firmwareRuntimePromise = null;
      throw e;
    });
    return firmwareRuntimePromise;
  }

  // One badge per browser (src/radio/lock.ts): the firmware runs only while
  // this tab holds the badge.
  let badgeHold: BadgeHold | null = null;
  let claiming: Promise<boolean> | null = null;
  /**
   * Whether the firmware runs in this tab, set beside each
   * `hub.setRunning`. A stopped badge posts no status, so Skip setup hides
   * on it (Milestone 13's final review, R-FR-1).
   */
  let running = false;
  const badgeNotice = shell.mountBadgeNotice(() => {
    void runHere();
  });

  /** Another tab took the badge over: stop here and offer it back. */
  function onBadgeLost(): void {
    badgeHold = null;
    firmwareRuntime?.stop();
    running = false;
    hub.setRunning(false); // the bump gate (Milestone 11 Task 11)
    refreshStatus();
    showNotice();
  }

  /**
   * Frees the badge for another tab (Milestone 11 Task 12): on a failed
   * start and unload. Idempotent: the hold is dropped before it is
   * released, so it is released once.
   */
  function releaseBadge(): void {
    const hold = badgeHold;
    badgeHold = null;
    hold?.release();
  }

  /** Shows the badge notice (it clears the canvas behind it). */
  function showNotice(): void {
    badgeNotice.setVisible(true);
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
   * shows the notice. Booting it if needed and starting it if stopped, so a
   * retry after a failed boot (Run here, a provision) leaves it running.
   */
  async function startFirmware(): Promise<void> {
    if (!(await claimBadge())) {
      showNotice();
      return;
    }
    badgeNotice.setVisible(false);
    let rt: WorkerFirmwareRuntime;
    try {
      rt = await ensureFirmwareRuntime();
    } catch (e) {
      // Nothing runs here: let another tab have the badge.
      releaseBadge();
      throw e;
    }
    // Another tab took the badge while it booted, or it already runs.
    if (!badgeHold || running) return;
    rt.start();
    running = true;
    hub.setRunning(true); // the bump gate (Milestone 11 Task 11)
  }

  /** A failed start's line in the status (cleared by the next provision). */
  function startFailed(e: unknown): void {
    console.error("Failed to start the firmware:", e);
    fetchError = START_FAILED_NOTE;
    refreshStatus();
  }

  /** "Run here": takes the badge from the other tab (which stops), then starts it here. */
  async function runHere(): Promise<void> {
    await takeOver(locks);
    try {
      await startFirmware();
    } catch (e) {
      startFailed(e);
    }
  }

  const inject: ButtonInjector = (button, kind, source) => {
    firmwareRuntime?.injectButton(button, kind, source);
  };

  let fetchError = "";
  /**
   * Which of the two the user started last: its state is the status text.
   * The runtime keeps a finished skip's state through later provisions.
   */
  let lastStarted: "provision" | "skip" | null = null;
  const provisionControls: ProvisionControls = shell.mountProvisionControls(
    deps.roles,
    async (role) => {
      fetchError = "";
      lastStarted = "provision";
      try {
        // Boots and starts the badge if it isn't running (a retry after a
        // failed first boot). Without the badge it boots nothing and shows
        // the notice: another tab runs it.
        await startFirmware();
        const rt = firmwareRuntime;
        if (!badgeHold || !rt) return;
        rt.provision(await deps.fetchIdentity(role));
      } catch (e) {
        console.error(`Could not load test identity ${role}:`, e);
        fetchError = `could not load ${role}`;
      }
    },
    () => {
      // A stopped badge types nothing, and a refused click keeps the last
      // status line (Milestone 13's final review, R-FR-1 and R-FR-3).
      const rt = firmwareRuntime;
      if (!running || !rt?.setupOnScreen()) return;
      lastStarted = "skip";
      rt.skipSetup();
      refreshStatus();
    },
  );

  /**
   * The provisioning controls from the runtime's last status. Skip setup
   * shows only while the firmware's Setup app is on screen, no skip is
   * typing, no provision is in flight (the provision's token phase runs
   * after Setup appears; Milestone 13's ruling T4-1) and the badge runs in
   * this tab (a stopped badge posts no status; R-FR-1). Provision is
   * disabled while a skip types (ruling T5-1). The speed reads
   * `rebooting…` through the reboot's pause, not a speed (ride-along 4).
   */
  function refreshStatus(): void {
    provisionControls.setVisible(true);
    const rt = firmwareRuntime;
    const st = rt?.provisionState() ?? null;
    const skip = rt?.skipState() ?? null;
    const provisionInFlight = st?.kind === "waiting" || st?.kind === "typing";
    const skipTyping = skip?.kind === "typing";
    provisionControls.setSkipVisible(
      running && rt !== null && rt.setupOnScreen() && !skipTyping && !provisionInFlight,
    );
    provisionControls.setProvisionEnabled(!skipTyping);
    let text = fetchError;
    if (st) {
      if (st.kind === "waiting") text = "waiting for the console\u2026";
      else if (st.kind === "typing") text = "provisioning\u2026";
      else if ("line" in st) text = st.line;
      else if ("reason" in st) text = `failed: ${st.reason}`;
    }
    // The skip's state shows only while it is the newer of the two (`lastStarted`);
    // `done` adds nothing (the screen shows it).
    if (lastStarted === "skip") {
      if (skip?.kind === "typing") text = "skipping setup…";
      else if (skip?.kind === "failed") text = `skip failed: ${skip.reason}`;
    }
    if (rt) {
      // The emulated/wall ratio: below 1x the step cap binds (slow motion).
      const speed = rt.speed();
      const prefix = rt.rebooting() ? "rebooting…" : speed > 0 ? `${speed.toFixed(1)}x` : "";
      provisionControls.setStatus([prefix, text].filter(Boolean).join(" · "));
    } else {
      provisionControls.setStatus(text);
    }
  }

  shell.renderLeds([]); // the firmware models no LED peripheral yet
  void startFirmware().catch(startFailed);

  return {
    firmware: () => firmwareRuntime,
    inject,
    refreshStatus,
    dispose() {
      firmwareRuntime?.dispose();
      releaseBadge();
    },
  };
}

function errorText(e: unknown, fallback: string): string {
  return e instanceof SignalError ? e.message : fallback;
}

/**
 * Mounts the link panel and runs its links into `hub`: one hosted room at
 * a time (hosting again cancels a room no one has joined yet) and any
 * number of links; a link leaves the hub when it closes. Returns the panel
 * and a function that closes every link (for unload). `signal` reaches
 * `/api/signal` and WebRTC; the browser's when omitted.
 */
export function mountLinks(
  shell: Pick<ShellHandles, "mountLinkPanel">,
  hub: Pick<RadioHub, "addLink" | "removeLink">,
  signal: SignalDeps = {},
): { panel: LinkPanel; closeAll(): void } {
  const links = new Map<string, PeerLink>();
  let hosting: HostedRoom | null = null;
  /**
   * Counts host presses and cancels: a room whose `hostRoom` returns after
   * a later press or a cancel is closed at once, not orphaned.
   */
  let hostSeq = 0;

  function track(link: PeerLink): void {
    links.set(link.id, link);
    hub.addLink(link);
    link.onStateChange((state) => {
      if (state !== "closed") return;
      links.delete(link.id);
      hub.removeLink(link.id);
    });
  }

  /**
   * The cancel wiring (Milestone 11 Task 10): stops waiting on the hosted
   * room, closes its link, and has `hostRoom`'s cancel send `close` (best
   * effort, never throws), so a later joiner hears "Room closed or
   * expired." at once. Every path that drops a hosted room comes here.
   */
  function cancelHosting(): void {
    hostSeq++;
    const h = hosting;
    hosting = null;
    h?.cancel();
  }

  async function host(): Promise<void> {
    cancelHosting();
    const seq = hostSeq;
    panel.setStatus("Creating a room\u2026");
    try {
      const h = await hostRoom(signal);
      if (seq !== hostSeq) {
        // Hosted again, or cancelled, while this room was made.
        h.cancel();
        return;
      }
      hosting = h;
      track(h.link);
      panel.setStatus(`Room ${h.room}: enter this code in the other browser\u2019s Join link, within 2 minutes.`);
      h.connected.then(
        () => {
          if (hosting === h) hosting = null;
          panel.setStatus(`Room ${h.room}: linked.`);
        },
        (e: unknown) => {
          if (hosting !== h) return; // replaced or removed: its status is stale
          hosting = null;
          panel.setStatus(errorText(e, "Linking failed."));
        },
      );
    } catch (e) {
      if (seq === hostSeq) panel.setStatus(errorText(e, "Could not host a room."));
    }
  }

  async function join(code: string): Promise<void> {
    panel.setStatus(`Joining ${code.trim()}\u2026`);
    try {
      const link = await joinRoom(code, signal);
      track(link);
      panel.setStatus("Linked.");
    } catch (e) {
      panel.setStatus(errorText(e, "Could not join the room."));
    }
  }

  const panel = shell.mountLinkPanel({
    onHost: () => void host(),
    onJoin: (code) => void join(code),
  });
  return {
    panel,
    closeAll() {
      cancelHosting();
      for (const link of links.values()) link.close();
    },
  };
}

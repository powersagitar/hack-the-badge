/**
 * The page's badge wiring, factored out of `main.ts` (Milestone 11 Task 12)
 * so `test/shell.test.ts` can drive it over a real DOM: `main.ts` runs
 * itself on import and reaches `fetch`, `Worker` and `navigator.locks`
 * directly, so it cannot be driven from a test. Everything a test must
 * replace comes in through {@link BadgeControllerDeps}; `main.ts` passes
 * the browser's.
 *
 * - {@link createBadgeController}: the mode switch, the lazily booted
 *   firmware Worker, the one-badge-per-browser lock and its notice, the
 *   provisioning controls, and the bump gate's `hub.setRunning` calls.
 * - {@link mountLinks}: the radio link panel and its links, with the
 *   hosted room's cancel.
 *
 * **The radio (Milestone 10 Task 9).** One badge per browser: the firmware
 * runs only in the tab holding the `htn-badge` Web Lock
 * (`src/radio/lock.ts`); another tab shows "Badge running in another tab —
 * Run here", whose button takes the badge over (the old tab stops). The
 * badge's factory MAC is this browser's, sent to the Worker before its
 * first start. The Bump button bumps this badge and every linked one, only
 * while the badge runs here and the page is visible: each `rt.start()` and
 * `firmwareRuntime?.stop()` below is paired with `hub.setRunning`, and the
 * hidden-page pause pauses the hub (`onPausedChange`), Milestone 11 Task 11.
 *
 * **Links (Milestone 10 Task 10).** The link panel ({@link mountLinks})
 * makes WebRTC links (`src/radio/peer-link.ts`) to other browsers: "Host
 * link" creates a room on `/api/signal` and shows its code, "Join link"
 * joins one, and "Manual" swaps the same offer and answer as copy-paste
 * codes instead. Each link joins the hub as soon as it exists; it carries
 * nothing until its channel opens, and leaves the hub when it closes.
 * Cancelling a hosted room ({@link mountLinks}'s `cancelHosting`: hosting
 * again, removing its link, unload) also closes the room on the server,
 * best effort (Milestone 11 Task 10).
 */
import type { RadioHub } from "../radio/hub";
import { acquireBadge, type BadgeHold, type LockManagerLike, takeOver } from "../radio/lock";
import { acceptOffer, createOffer, decodeManualCode, encodeManualCode, type PeerLink } from "../radio/peer-link";
import {
  type HostedRoom,
  SignalError,
  type SignalDeps,
  hostRoom,
  joinRoom,
  normalizeRoomCode,
} from "../radio/signal-client";
import {
  createWorkerFirmwareRuntime,
  type WorkerFirmwareRuntime,
  type WorkerLike,
} from "../runtime/firmware-worker-client";
import type { ButtonInjector, EmulatorMode, LinkPanel, ProvisionControls, ShellHandles } from "./shell";

/** The Worker the controller boots: a {@link WorkerLike} that also reports its own load failure. */
export interface BadgeWorker extends WorkerLike {
  addEventListener(type: "error", cb: (e: { message: string }) => void, options: { once: true }): void;
}

export interface BadgeControllerDeps {
  shell: Pick<ShellHandles, "ctx" | "renderLeds" | "mountModeToggle" | "mountBadgeNotice" | "mountProvisionControls">;
  hub: Pick<RadioHub, "fromLocal" | "setPaused" | "setRunning" | "bump">;
  /** The Lua sandbox's paint loop and input; its `on_tick` timer is not the controller's. */
  lua: { startPaint(): void; stopPaint(): void; injectButton: ButtonInjector };
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

export interface BadgeController {
  mode(): EmulatorMode;
  /** The booted firmware runtime, or `null` before the first boot. */
  firmware(): WorkerFirmwareRuntime | null;
  setMode(next: EmulatorMode): Promise<void>;
  /** The button pad's and keyboard's input, routed to the mode's runtime. */
  inject: ButtonInjector;
  /** The provisioning controls' visibility and status line (`main.ts` polls it). */
  refreshStatus(): void;
  /** Page unload: disposes the runtime and frees the badge. */
  dispose(): void;
}

/**
 * Mounts the mode toggle, the badge notice and the provisioning controls,
 * in Lua mode. The Lua paint loop is the caller's to start.
 */
export function createBadgeController(deps: BadgeControllerDeps): BadgeController {
  const { shell, hub, lua } = deps;
  const { locks } = deps;

  let mode: EmulatorMode = "lua";

  // Real-firmware mode is constructed lazily (on first switch to it) rather
  // than eagerly at startup, so a user who stays in Lua-sandbox mode never
  // pays the cost of fetching/booting firmware.bin through WASM. Once built,
  // it's kept alive (not disposed) across later mode switches so switching
  // back doesn't re-boot from scratch.
  //
  // Since Milestone 7 Task 8 the emulator runs in a dedicated Worker
  // (`runtime/firmware-worker.ts`); this thread only fetches the bytes,
  // paints the frames the Worker posts and forwards input
  // (`runtime/firmware-worker-client.ts`). The Worker imports `./cpu/bridge`
  // lazily: `bridge.ts` imports the gitignored, Rust-toolchain-generated
  // `src/cpu/wasm-pkg/`, and the Lua-only path never creates the Worker, so
  // it never loads `src/cpu/`.
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
      // stay cached forever -- reset the slot so the next toggle attempt
      // retries the fetch/boot instead of replaying this same rejection.
      firmwareRuntimePromise = null;
      throw e;
    });
    return firmwareRuntimePromise;
  }

  // One badge per browser (src/radio/lock.ts): the firmware runs only while
  // this tab holds the badge.
  let badgeHold: BadgeHold | null = null;
  let claiming: Promise<boolean> | null = null;
  const badgeNotice = shell.mountBadgeNotice(() => {
    void runHere();
  });

  /** Another tab took the badge over: stop here and offer it back. */
  function onBadgeLost(): void {
    badgeHold = null;
    firmwareRuntime?.stop();
    hub.setRunning(false); // the bump gate (Milestone 11 Task 11)
    showNotice();
  }

  /**
   * Frees the badge for another tab (Milestone 11 Task 12): on a switch to
   * Lua mode, a failed start and unload. Idempotent: the hold is dropped
   * before it is released, so it is released once.
   */
  function releaseBadge(): void {
    const hold = badgeHold;
    badgeHold = null;
    hold?.release();
  }

  /** Shows the badge notice, in firmware mode only (it clears the canvas behind it). */
  function showNotice(): void {
    if (mode === "firmware") badgeNotice.setVisible(true);
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

  /** Starts the firmware if this tab holds (or can take) the badge, else shows the notice. */
  async function startFirmware(): Promise<void> {
    if (!(await claimBadge())) {
      showNotice();
      return;
    }
    // Switched back to Lua mode while the claim was in flight.
    if (mode !== "firmware") {
      releaseBadge();
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
    // The user switched away, or another tab took the badge, while it booted.
    if (mode !== "firmware" || !badgeHold) return;
    rt.start();
    hub.setRunning(true); // the bump gate (Milestone 11 Task 11)
  }

  /** "Run here": takes the badge from the other tab (which stops), then starts it here. */
  async function runHere(): Promise<void> {
    await takeOver(locks);
    if (mode !== "firmware") return;
    try {
      await startFirmware();
    } catch (e) {
      console.error("Failed to start real-firmware mode:", e);
    }
  }

  const inject: ButtonInjector = (button, kind) => {
    if (mode === "lua") {
      lua.injectButton(button, kind);
    } else {
      firmwareRuntime?.injectButton(button, kind);
    }
  };

  function refreshModeToggle(): void {
    shell.mountModeToggle(mode, (next) => {
      void setMode(next);
    });
  }

  async function setMode(next: EmulatorMode): Promise<void> {
    if (next === mode) return;
    const previous = mode;

    if (previous === "lua") {
      lua.stopPaint();
    } else {
      firmwareRuntime?.stop();
      hub.setRunning(false); // the bump gate (Milestone 11 Task 11)
    }

    mode = next;
    refreshModeToggle();

    if (next === "lua") {
      releaseBadge();
      badgeNotice.setVisible(false);
      lua.startPaint();
      return;
    }

    // next === "firmware"
    shell.renderLeds([]); // real-firmware mode models no LED peripheral yet
    try {
      await startFirmware();
    } catch (e) {
      console.error("Failed to start real-firmware mode:", e);
      // Fall back to Lua-sandbox mode rather than leaving the canvas frozen
      // with nothing driving it.
      mode = "lua";
      refreshModeToggle();
      lua.startPaint();
    }
  }

  let fetchError = "";
  const provisionControls: ProvisionControls = shell.mountProvisionControls(
    deps.roles,
    async (role) => {
      fetchError = "";
      try {
        // Without the badge, boot nothing: another tab runs it.
        if (!(await claimBadge())) {
          showNotice();
          return;
        }
        const rt = await ensureFirmwareRuntime();
        rt.provision(await deps.fetchIdentity(role));
      } catch (e) {
        console.error(`Could not load test identity ${role}:`, e);
        fetchError = `could not load ${role}`;
      }
    },
    () => firmwareRuntime?.shake(),
    () => hub.bump(),
  );

  function refreshStatus(): void {
    provisionControls.setVisible(mode === "firmware");
    const st = firmwareRuntime?.provisionState() ?? null;
    let text = fetchError;
    if (st) {
      if (st.kind === "waiting") text = "waiting for the console\u2026";
      else if (st.kind === "typing") text = "provisioning\u2026";
      else if ("line" in st) text = st.line;
      else if ("reason" in st) text = `failed: ${st.reason}`;
    }
    if (mode === "firmware" && firmwareRuntime) {
      // The emulated/wall ratio: below 1x the step cap binds (slow motion).
      const speed = firmwareRuntime.speed();
      const prefix = speed > 0 ? `${speed.toFixed(1)}x` : "";
      provisionControls.setStatus([prefix, text].filter(Boolean).join(" · "));
    } else {
      provisionControls.setStatus(text);
    }
  }

  refreshModeToggle();

  return {
    mode: () => mode,
    firmware: () => firmwareRuntime,
    setMode,
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
 * a time (hosting again cancels a room no one has joined yet), any number
 * of links, and the manual codes. Returns the panel and a function that
 * closes every link (for unload). `signal` reaches `/api/signal` and
 * WebRTC; the browser's when omitted.
 */
export function mountLinks(
  shell: Pick<ShellHandles, "mountLinkPanel">,
  hub: Pick<RadioHub, "addLink" | "removeLink">,
  signal: SignalDeps = {},
): { panel: LinkPanel; closeAll(): void } {
  const links = new Map<string, { link: PeerLink; label: string }>();
  let made = 0;
  let hosting: HostedRoom | null = null;
  /**
   * Counts host presses and cancels: a room whose `hostRoom` returns after
   * a later press or a cancel is closed at once, not orphaned.
   */
  let hostSeq = 0;
  /** This browser's manual offer, waiting for the peer's answer code. */
  let manualOffer: PeerLink | null = null;
  // The manual codes' WebRTC: `signal`'s factory, or the browser's.
  const rtc = { factory: signal.factory };

  const refresh = () =>
    panel.setLinks([...links.values()].map(({ link, label }) => ({ id: link.id, label, state: link.state })));

  function track(link: PeerLink, how: string): void {
    links.set(link.id, { link, label: `Link ${++made} (${how})` });
    hub.addLink(link);
    link.onStateChange((state) => {
      if (state === "closed") hub.removeLink(link.id);
      refresh();
    });
    refresh();
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
      track(h.link, `room ${h.room}`);
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
      track(link, `room ${normalizeRoomCode(code) ?? code.trim()}`);
      panel.setStatus("Linked.");
    } catch (e) {
      panel.setStatus(errorText(e, "Could not join the room."));
    }
  }

  async function manualCreate(): Promise<void> {
    try {
      if (manualOffer && manualOffer.state === "connecting") remove(manualOffer.id);
      const { link, offer } = await createOffer(rtc);
      manualOffer = link;
      track(link, "manual");
      panel.setManualOutput(await encodeManualCode("offer", offer));
      panel.setStatus("Copy this offer code to the other browser, then paste its answer code here and press Use code.");
    } catch {
      panel.setStatus("This browser could not start a WebRTC link.");
    }
  }

  async function manualUse(code: string): Promise<void> {
    const decoded = await decodeManualCode(code);
    if (!decoded) {
      panel.setStatus("That is not a link code.");
      return;
    }
    try {
      if (decoded.kind === "offer") {
        const { link, answer } = await acceptOffer(decoded.sdp, rtc);
        track(link, "manual");
        panel.setManualOutput(await encodeManualCode("answer", answer));
        panel.setStatus("Copy this answer code back to the other browser.");
        return;
      }
      const offerer = manualOffer;
      if (!offerer || offerer.state !== "connecting") {
        panel.setStatus("An answer code completes this browser\u2019s offer: press Create code first.");
        return;
      }
      await offerer.complete(decoded.sdp);
      manualOffer = null;
      panel.setStatus("Answer applied: connecting\u2026");
    } catch {
      panel.setStatus("That code could not be used.");
    }
  }

  function remove(id: string): void {
    const entry = links.get(id);
    if (!entry) return;
    if (hosting?.link === entry.link) cancelHosting();
    if (manualOffer === entry.link) manualOffer = null;
    links.delete(id);
    hub.removeLink(id);
    entry.link.close();
    refresh();
  }

  const panel = shell.mountLinkPanel({
    onHost: () => void host(),
    onJoin: (code) => void join(code),
    onManualCreate: () => void manualCreate(),
    onManualUse: (code) => void manualUse(code),
    onRemove: remove,
  });
  return {
    panel,
    closeAll() {
      cancelHosting();
      for (const { link } of links.values()) link.close();
    },
  };
}

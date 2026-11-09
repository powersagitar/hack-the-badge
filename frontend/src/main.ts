/**
 * Entry point: mounts the shell, and owns both runtimes named in
 * `CLAUDE.md`'s architecture section —
 *
 *   "Will grow to own both the Lua AppRuntime and the CPU FirmwareRuntime
 *    once real-firmware mode lands ... currently Lua-only."
 *
 * That's this file, now. It starts in Lua-sandbox mode (the smoke-test app,
 * as before), and the mode toggle `shell.ts` mounts (`#mode-toggle`) lets the
 * user switch to real-firmware mode, which lazily boots
 * `public/firmware/firmware.bin` through the WASM CPU emulator in a Web
 * Worker (`src/runtime/firmware-worker.ts`, driven from here through
 * `src/runtime/firmware-worker-client.ts`) the first time
 * it's selected, then reuses that same booted instance on every later
 * switch back to it (switching away just pauses its frame loop — it isn't
 * re-booted or disposed until the page unloads).
 *
 * Both modes share the same `<canvas>`, the same on-screen button pad and
 * the same keyboard bindings (`shell.ts`'s `mountButtons`/`mountKeyboard`,
 * wired once, below, to a single `inject` function that routes to whichever
 * runtime is currently active) — per the plan's "toggled via a mode switch
 * in `src/ui/shell.ts`" note.
 *
 * **What mode-switching actually gates:** only *painting*. `setMode` stops/
 * starts the Lua rAF paint loop (`startLuaPaintLoop`/`stopLuaPaintLoop`) and
 * the firmware runtime's own step+paint loop (`FirmwareRuntime.start`/
 * `.stop`) depending on which mode is now selected — but `luaRuntime`'s
 * `on_tick` timer (`lifecycle.ts`'s `setInterval`) is started once, here,
 * and never stopped except on page unload. So the Lua app keeps running and
 * mutating its own state in the background even while real-firmware mode is
 * the one being displayed; only its canvas output is suspended. This is
 * intentional (switching back to Lua mode resumes it mid-state instead of
 * restarting `main.lua` from `on_enter`), not a bug.
 *
 * **The radio (Milestone 10 Task 9).** One badge per browser: the firmware
 * runs only in the tab holding the `htn-badge` Web Lock
 * (`src/radio/lock.ts`); another tab shows "Badge running in another tab —
 * Run here", whose button takes the badge over (the old tab stops). The
 * badge's factory MAC is this browser's (`loadOrCreateMac`), sent to the
 * Worker before its first start. `src/radio/hub.ts`'s `RadioHub` carries
 * the badge's advertisements to linked peers and theirs back, and the
 * Bump button bumps this badge and every linked one, only while the badge
 * runs here and the page is visible: each `rt.start()` and
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
import { renderFrame } from "./render/canvas";
import { RadioHub } from "./radio/hub";
import { acceptOffer, createOffer, decodeManualCode, encodeManualCode, type PeerLink } from "./radio/peer-link";
import { type HostedRoom, SignalError, hostRoom, joinRoom, normalizeRoomCode } from "./radio/signal-client";
import { acquireBadge, type BadgeHold, loadOrCreateMac, takeOver } from "./radio/lock";
import {
  createWorkerFirmwareRuntime,
  type WorkerFirmwareRuntime,
  type WorkerLike,
} from "./runtime/firmware-worker-client";
import { createHttpFileLoader, loadApp } from "./runtime/lifecycle";
import { TEST_IDENTITY_ROLES, testIdentityUrl } from "./runtime/test-identities";
import { mountShell, type ButtonInjector, type EmulatorMode, type LinkPanel, type ShellHandles } from "./ui/shell";

const APP_DIR = "smoke-test";
const FIRMWARE_IMAGE_URL = "/firmware/firmware.bin";
/**
 * The WASM module the Worker boots, at the URL the generated glue
 * (`src/cpu/wasm-pkg/emulator_wasm.js`) resolves for itself: the `.wasm`
 * next to it. Vite rewrites this to the hashed asset in a build.
 */
const CPU_WASM_URL = new URL("./cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url);

async function fetchBytes(url: string | URL): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`failed to fetch ${url}: HTTP ${res.status}`);
  return res.arrayBuffer();
}

/** Mirrors `console.log` into the on-page log panel, so `badge.sys.log` calls are visible. */
function installLogPanel(): void {
  const out = document.getElementById("log-output");
  if (!out) return;
  const original = console.log.bind(console);
  console.log = (...args: unknown[]) => {
    original(...args);
    const line = args
      .map((a) => (typeof a === "string" ? a : safeStringify(a)))
      .join(" ");
    out.textContent += line + "\n";
    out.scrollTop = out.scrollHeight;
  };
}

/** `localStorage`, or `null` where reading it throws (blocked storage, sandboxed frames). */
function browserStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function safeStringify(v: unknown): string {
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

function errorText(e: unknown, fallback: string): string {
  return e instanceof SignalError ? e.message : fallback;
}

/**
 * Mounts the link panel and runs its links into `hub`: one hosted room at
 * a time (hosting again cancels a room no one has joined yet), any number
 * of links, and the manual codes. Returns the panel and a function that
 * closes every link (for unload).
 */
function mountLinks(shell: ShellHandles, hub: RadioHub): { panel: LinkPanel; closeAll(): void } {
  const links = new Map<string, { link: PeerLink; label: string }>();
  let made = 0;
  let hosting: HostedRoom | null = null;
  /** This browser's manual offer, waiting for the peer's answer code. */
  let manualOffer: PeerLink | null = null;

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
    const h = hosting;
    hosting = null;
    h?.cancel();
  }

  async function host(): Promise<void> {
    cancelHosting();
    panel.setStatus("Creating a room\u2026");
    try {
      const h = await hostRoom();
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
      panel.setStatus(errorText(e, "Could not host a room."));
    }
  }

  async function join(code: string): Promise<void> {
    panel.setStatus(`Joining ${code.trim()}\u2026`);
    try {
      const link = await joinRoom(code);
      track(link, `room ${normalizeRoomCode(code) ?? code.trim()}`);
      panel.setStatus("Linked.");
    } catch (e) {
      panel.setStatus(errorText(e, "Could not join the room."));
    }
  }

  async function manualCreate(): Promise<void> {
    try {
      if (manualOffer && manualOffer.state === "connecting") remove(manualOffer.id);
      const { link, offer } = await createOffer();
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
        const { link, answer } = await acceptOffer(decoded.sdp);
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

function main(): void {
  installLogPanel();

  const shell = mountShell();
  const fileLoader = createHttpFileLoader(`/apps/${APP_DIR}/`);
  const luaRuntime = loadApp(APP_DIR, fileLoader);

  let mode: EmulatorMode = "lua";
  let luaFrameHandle: number | null = null;

  function startLuaPaintLoop(): void {
    function frame(): void {
      if (mode !== "lua") return; // stopLuaPaintLoop() lost the race to an in-flight rAF callback
      renderFrame(shell.ctx, luaRuntime.ui.root);
      shell.renderLeds(luaRuntime.led.getColors());
      luaFrameHandle = requestAnimationFrame(frame);
    }
    if (luaFrameHandle === null) luaFrameHandle = requestAnimationFrame(frame);
  }

  function stopLuaPaintLoop(): void {
    if (luaFrameHandle !== null) {
      cancelAnimationFrame(luaFrameHandle);
      luaFrameHandle = null;
    }
  }

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
  // lazily, as this file did before: `bridge.ts` imports the gitignored,
  // Rust-toolchain-generated `src/cpu/wasm-pkg/`, and the Lua-only path
  // never creates the Worker, so it never loads `src/cpu/`.
  let firmwareRuntime: WorkerFirmwareRuntime | null = null;
  let firmwareRuntimePromise: Promise<WorkerFirmwareRuntime> | null = null;

  const hub = new RadioHub({
    deliver: (bytes, rssi) => firmwareRuntime?.deliverAdvertisement(bytes, rssi),
    bumpLocal: () => firmwareRuntime?.bump(),
  });
  const radioLinks = mountLinks(shell, hub);

  async function ensureFirmwareRuntime(): Promise<WorkerFirmwareRuntime> {
    if (firmwareRuntime) return firmwareRuntime;
    firmwareRuntimePromise ??= (async () => {
      const [wasm, image] = await Promise.all([fetchBytes(CPU_WASM_URL), fetchBytes(FIRMWARE_IMAGE_URL)]);
      const worker = new Worker(new URL("./runtime/firmware-worker.ts", import.meta.url), { type: "module" });
      // A real Worker's handlers take DOM event types; the client needs only `data`.
      const rt = createWorkerFirmwareRuntime(worker as unknown as WorkerLike, shell.ctx, wasm, image, {
        onAdvertisement: (bytes) => hub.fromLocal(bytes),
        onPausedChange: (paused) => hub.setPaused(paused),
      });
      // Before any start: the firmware reads its factory MAC at boot.
      rt.setMac(loadOrCreateMac(browserStorage()));
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
    if (mode === "firmware") badgeNotice.setVisible(true);
  }

  /** Takes the badge if no other tab holds it; `true` if this tab holds it. */
  function claimBadge(): Promise<boolean> {
    if (badgeHold) return Promise.resolve(true);
    claiming ??= acquireBadge(onBadgeLost).then((hold) => {
      claiming = null;
      badgeHold = hold;
      return hold !== null;
    });
    return claiming;
  }

  /** Starts the firmware if this tab holds (or can take) the badge, else shows the notice. */
  async function startFirmware(): Promise<void> {
    if (!(await claimBadge())) {
      if (mode === "firmware") badgeNotice.setVisible(true);
      return;
    }
    badgeNotice.setVisible(false);
    const rt = await ensureFirmwareRuntime();
    // The user switched away, or another tab took the badge, while it booted.
    if (mode !== "firmware" || !badgeHold) return;
    rt.start();
    hub.setRunning(true); // the bump gate (Milestone 11 Task 11)
  }

  /** "Run here": takes the badge from the other tab (which stops), then starts it here. */
  async function runHere(): Promise<void> {
    await takeOver();
    if (mode !== "firmware") return;
    try {
      await startFirmware();
    } catch (e) {
      console.error("Failed to start real-firmware mode:", e);
    }
  }

  const inject: ButtonInjector = (button, kind) => {
    if (mode === "lua") {
      luaRuntime.injectButton(button, kind);
    } else {
      firmwareRuntime?.injectButton(button, kind);
    }
  };
  shell.mountButtons(inject);
  shell.mountKeyboard(inject);

  function refreshModeToggle(): void {
    shell.mountModeToggle(mode, (next) => {
      void setMode(next);
    });
  }

  async function setMode(next: EmulatorMode): Promise<void> {
    if (next === mode) return;
    const previous = mode;

    if (previous === "lua") {
      stopLuaPaintLoop();
    } else {
      firmwareRuntime?.stop();
      hub.setRunning(false); // the bump gate (Milestone 11 Task 11)
    }

    mode = next;
    refreshModeToggle();

    if (next === "lua") {
      badgeNotice.setVisible(false);
      startLuaPaintLoop();
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
      startLuaPaintLoop();
    }
  }

  let fetchError = "";
  const provisionControls = shell.mountProvisionControls(
    TEST_IDENTITY_ROLES,
    async (role) => {
      fetchError = "";
      try {
        const rt = await ensureFirmwareRuntime();
        const res = await fetch(testIdentityUrl(role));
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        rt.provision(await res.text());
      } catch (e) {
        console.error(`Could not load test identity ${role}:`, e);
        fetchError = `could not load ${role}`;
      }
    },
    () => firmwareRuntime?.shake(),
    () => hub.bump(),
  );
  setInterval(() => {
    provisionControls.setVisible(mode === "firmware");
    radioLinks.panel.setVisible(mode === "firmware");
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
  }, 250);

  refreshModeToggle();
  luaRuntime.start();
  startLuaPaintLoop();

  window.addEventListener("beforeunload", () => {
    luaRuntime.stop();
    firmwareRuntime?.dispose();
    badgeHold?.release();
    radioLinks.closeAll();
  });
}

try {
  main();
} catch (e) {
  console.error("Failed to start the badge emulator:", e);
  const el = document.getElementById("app");
  if (el) {
    const pre = document.createElement("pre");
    pre.style.color = "#ff6666";
    pre.style.whiteSpace = "pre-wrap";
    pre.textContent = `Failed to start: ${e instanceof Error ? e.message : String(e)}`;
    el.appendChild(pre);
  }
}

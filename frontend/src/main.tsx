/**
 * Entry point (Milestone 14): builds the radio hub and the controller store
 * (`ui/badge-controller.ts`) over the browser (`fetch`, the firmware
 * `Worker`, `navigator.locks`, `localStorage`), and renders the React page,
 * {@link App}, into `index.html`'s `#root`, themed by `ui/theme.css`.
 *
 * - The controller boots the real firmware, `public/firmware/firmware.bin`,
 *   through the WASM CPU emulator in a Web Worker
 *   (`runtime/firmware-worker.ts`). This file polls its status every
 *   {@link STATUS_POLL_MS}, as the old `main.ts` did.
 * - The hub delivers linked peers' advertisements to the firmware, and a
 *   peer's bump to the controller's `peerBump` (the active motion source:
 *   the lanyard, or the phone; ruling R3). The controller sends the
 *   local impacts (`hub.broadcastBump`).
 * - Construction is guarded: if the hub, the controller or the first
 *   render throws, the page shows a plain "Failed to start" message.
 * - `pagehide` stops the status poll and disposes the controller (it
 *   frees the badge for another tab and closes the links). A page
 *   restored from the back/forward cache reloads, since its badge is
 *   gone.
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RadioHub } from "./radio/hub";
import { loadOrCreateMac } from "./radio/lock";
import { testIdentityUrl } from "./runtime/test-identities";
import { App } from "./ui/App";
import { type BadgeController, type BadgeWorker, createBadgeController } from "./ui/badge-controller";
import "./ui/theme.css";

const FIRMWARE_IMAGE_URL = "/firmware/firmware.bin";
/**
 * The WASM module the Worker boots, at the URL the generated glue
 * (`src/cpu/wasm-pkg/emulator_wasm.js`) resolves for itself: the `.wasm`
 * next to it. Vite rewrites this to the hashed asset in a build.
 */
const CPU_WASM_URL = new URL("./cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url);

/** How often the page reads the Worker's last status (it posts one every 250 ms at most). */
const STATUS_POLL_MS = 250;

async function fetchBytes(url: string | URL): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`failed to fetch ${url}: HTTP ${res.status}`);
  return res.arrayBuffer();
}

/** `localStorage`, or `null` where reading it throws (blocked storage, sandboxed frames). */
function browserStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function main(root: HTMLElement): void {
  let controller: BadgeController | null = null;
  const hub = new RadioHub({
    deliver: (bytes, rssi) => controller?.firmware()?.deliverAdvertisement(bytes, rssi),
    bumpLocal: () => controller?.peerBump(),
  });

  const c = createBadgeController({
    hub,
    loadImages: async () => {
      const [wasm, image] = await Promise.all([fetchBytes(CPU_WASM_URL), fetchBytes(FIRMWARE_IMAGE_URL)]);
      return { wasm, image };
    },
    // A real Worker's handlers take DOM event types; the client needs only `data`.
    createWorker: () =>
      new Worker(new URL("./runtime/firmware-worker.ts", import.meta.url), {
        type: "module",
      }) as unknown as BadgeWorker,
    mac: () => loadOrCreateMac(browserStorage()),
    fetchIdentity: async (role) => {
      const res = await fetch(testIdentityUrl(role));
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return res.text();
    },
  });
  controller = c;

  const poll = setInterval(() => c.refreshStatus(), STATUS_POLL_MS);
  window.addEventListener("pagehide", () => {
    clearInterval(poll);
    c.dispose();
  });
  window.addEventListener("pageshow", (e) => {
    if (e.persisted) window.location.reload();
  });

  createRoot(root).render(
    <StrictMode>
      <App controller={c} />
    </StrictMode>,
  );
}

/** A plain-DOM message when the page cannot start at all (as the old `main.ts` showed). */
function showStartFailure(root: HTMLElement | null, e: unknown): void {
  console.error("Failed to start the badge emulator:", e);
  const pre = document.createElement("pre");
  pre.style.cssText = "color:#ff6666;white-space:pre-wrap;padding:16px;font-family:monospace";
  pre.textContent = `Failed to start: ${e instanceof Error ? e.message : String(e)}`;
  (root ?? document.body).replaceChildren(pre);
}

const root = document.getElementById("root");
try {
  if (!root) throw new Error("index.html has no #root");
  main(root);
} catch (e) {
  showStartFailure(root, e);
}

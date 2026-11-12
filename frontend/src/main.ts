/**
 * Entry point: mounts the shell and boots the real firmware,
 * `public/firmware/firmware.bin`, through the WASM CPU emulator in a Web
 * Worker (`src/runtime/firmware-worker.ts`, driven from here through
 * `src/runtime/firmware-worker-client.ts`). The canvas, the on-screen
 * button pad and the keyboard bindings (`shell.ts`'s `mountButtons` /
 * `mountKeyboard`) all feed the one badge.
 *
 * **The badge and the radio** are wired in `src/ui/badge-controller.ts`
 * (Milestone 11 Task 12): {@link createBadgeController} owns the firmware
 * Worker, the one-badge-per-browser lock, the provisioning controls and the
 * bump gate; {@link mountLinks} owns the
 * link panel and the hosted room's cancel. This file passes them the
 * browser: `fetch`, `Worker`, `navigator.locks` and `localStorage`.
 */
import { RadioHub } from "./radio/hub";
import { loadOrCreateMac } from "./radio/lock";
import { TEST_IDENTITY_ROLES, testIdentityUrl } from "./runtime/test-identities";
import { type BadgeController, type BadgeWorker, createBadgeController, mountLinks } from "./ui/badge-controller";
import { mountShell } from "./ui/shell";

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

/** Mirrors `console.log` into the on-page log panel. */
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

function main(): void {
  installLogPanel();

  const shell = mountShell();
  const hub = new RadioHub({
    deliver: (bytes, rssi) => controller.firmware()?.deliverAdvertisement(bytes, rssi),
    // The Worker no longer plays a canned bump (Milestone 14 Task 6): a
    // peer's bump is dropped until Task 13 rewires it to the active motion
    // source's peerBump.
    bumpLocal: () => {},
  });
  const radioLinks = mountLinks(shell, hub);

  const controller: BadgeController = createBadgeController({
    shell,
    hub,
    roles: TEST_IDENTITY_ROLES,
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

  shell.mountButtons(controller.inject);
  shell.mountKeyboard(controller.inject);

  setInterval(() => {
    controller.refreshStatus();
  }, 250);
  radioLinks.panel.setVisible(true);

  window.addEventListener("beforeunload", () => {
    controller.dispose();
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

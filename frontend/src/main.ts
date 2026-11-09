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
 * **The badge and the radio** are wired in `src/ui/badge-controller.ts`
 * (Milestone 11 Task 12): {@link createBadgeController} owns the mode
 * switch, the firmware Worker, the one-badge-per-browser lock, the
 * provisioning controls and the bump gate; {@link mountLinks} owns the
 * link panel and the hosted room's cancel. This file passes them the
 * browser: `fetch`, `Worker`, `navigator.locks` and `localStorage`.
 */
import { renderFrame } from "./render/canvas";
import { RadioHub } from "./radio/hub";
import { loadOrCreateMac } from "./radio/lock";
import { createHttpFileLoader, loadApp } from "./runtime/lifecycle";
import { TEST_IDENTITY_ROLES, testIdentityUrl } from "./runtime/test-identities";
import { type BadgeController, type BadgeWorker, createBadgeController, mountLinks } from "./ui/badge-controller";
import { mountShell } from "./ui/shell";

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

function main(): void {
  installLogPanel();

  const shell = mountShell();
  const fileLoader = createHttpFileLoader(`/apps/${APP_DIR}/`);
  const luaRuntime = loadApp(APP_DIR, fileLoader);

  let luaFrameHandle: number | null = null;

  function startLuaPaintLoop(): void {
    function frame(): void {
      if (controller.mode() !== "lua") return; // stopLuaPaintLoop() lost the race to an in-flight rAF callback
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

  const hub = new RadioHub({
    deliver: (bytes, rssi) => controller.firmware()?.deliverAdvertisement(bytes, rssi),
    bumpLocal: () => controller.firmware()?.bump(),
  });
  const radioLinks = mountLinks(shell, hub);

  const controller: BadgeController = createBadgeController({
    shell,
    hub,
    lua: {
      startPaint: startLuaPaintLoop,
      stopPaint: stopLuaPaintLoop,
      injectButton: (button, kind) => luaRuntime.injectButton(button, kind),
    },
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
    radioLinks.panel.setVisible(controller.mode() === "firmware");
  }, 250);

  luaRuntime.start();
  startLuaPaintLoop();

  window.addEventListener("beforeunload", () => {
    luaRuntime.stop();
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

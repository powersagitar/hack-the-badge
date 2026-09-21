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
 * `public/firmware/factory.bin` through the WASM CPU emulator
 * (`src/cpu/bridge.ts` + `src/runtime/firmware-runtime.ts`) the first time
 * it's selected, then reuses that same booted instance on every later
 * switch back to it (switching away just pauses its frame loop — it isn't
 * re-booted or disposed until the page unloads).
 *
 * Both modes share the same `<canvas>`, the same on-screen button pad and
 * the same keyboard bindings (`shell.ts`'s `mountButtons`/`mountKeyboard`,
 * wired once, below, to a single `inject` function that routes to whichever
 * runtime is currently active) — per the plan's "toggled via a mode switch
 * in `src/ui/shell.ts`" note.
 */
import { createFirmwareEmulator, initCpuWasm } from "./cpu/bridge";
import { renderFrame } from "./render/canvas";
import { createFirmwareRuntime, type FirmwareRuntime } from "./runtime/firmware-runtime";
import { createHttpFileLoader, loadApp } from "./runtime/lifecycle";
import { mountShell, type ButtonInjector, type EmulatorMode } from "./ui/shell";

const APP_DIR = "smoke-test";
const FIRMWARE_IMAGE_URL = "/firmware/factory.bin";

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
  // pays the cost of fetching/booting factory.bin through WASM. Once built,
  // it's kept alive (not disposed) across later mode switches so switching
  // back doesn't re-boot from scratch.
  let firmwareRuntime: FirmwareRuntime | null = null;
  let firmwareRuntimePromise: Promise<FirmwareRuntime> | null = null;

  async function ensureFirmwareRuntime(): Promise<FirmwareRuntime> {
    if (firmwareRuntime) return firmwareRuntime;
    firmwareRuntimePromise ??= (async () => {
      await initCpuWasm();
      const res = await fetch(FIRMWARE_IMAGE_URL);
      if (!res.ok) {
        throw new Error(`failed to fetch ${FIRMWARE_IMAGE_URL}: HTTP ${res.status}`);
      }
      const image = new Uint8Array(await res.arrayBuffer());
      const handle = createFirmwareEmulator(image);
      const rt = createFirmwareRuntime(handle, shell.ctx);
      firmwareRuntime = rt;
      return rt;
    })();
    return firmwareRuntimePromise;
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
    }

    mode = next;
    refreshModeToggle();

    if (next === "lua") {
      startLuaPaintLoop();
      return;
    }

    // next === "firmware"
    shell.renderLeds([]); // real-firmware mode models no LED peripheral yet
    try {
      const rt = await ensureFirmwareRuntime();
      if (mode !== "firmware") return; // user switched away again before boot finished
      rt.start();
    } catch (e) {
      console.error("Failed to start real-firmware mode:", e);
      // Fall back to Lua-sandbox mode rather than leaving the canvas frozen
      // with nothing driving it.
      mode = "lua";
      refreshModeToggle();
      startLuaPaintLoop();
    }
  }

  refreshModeToggle();
  luaRuntime.start();
  startLuaPaintLoop();

  window.addEventListener("beforeunload", () => {
    luaRuntime.stop();
    firmwareRuntime?.dispose();
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

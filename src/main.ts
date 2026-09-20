/**
 * Entry point: mounts the shell, loads the smoke-test app, starts its
 * lifecycle, and drives a `requestAnimationFrame` paint loop over
 * `render/canvas.ts`. This is the only file that wires the pieces
 * (lua/vm, badge/*, runtime/lifecycle, render/canvas, ui/shell) together.
 */
import { renderFrame } from "./render/canvas";
import { createHttpFileLoader, loadApp } from "./runtime/lifecycle";
import { mountShell } from "./ui/shell";

const APP_DIR = "smoke-test";

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

  const runtime = loadApp(APP_DIR, fileLoader);

  shell.mountButtons(runtime.injectButton);
  shell.mountKeyboard(runtime.injectButton);

  runtime.start();

  function frame(): void {
    renderFrame(shell.ctx, runtime.ui.root);
    shell.renderLeds(runtime.led.getColors());
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  window.addEventListener("beforeunload", () => runtime.stop());
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

/**
 * Page chrome: mounts the on-screen button pad (mouse) and keyboard
 * bindings that both inject `badge.input` events, plus a tiny LED HUD.
 *
 * Keyboard mapping (documented per spec):
 *   ArrowUp/Down/Left/Right -> UP/DOWN/LEFT/RIGHT
 *   Z / X                   -> A / B
 *   Enter                   -> START
 *   Escape                  -> HOME
 *   Space                   -> AUX1
 * (unmapped: none needed for the smoke-test app; extend KEY_TO_BUTTON as needed.)
 */
import { BUTTON_NAMES, type ButtonName } from "../badge/input";
import type { Rgb } from "../badge/led";

export type ButtonInjector = (button: ButtonName, kind: "pressed" | "released") => void;

const KEY_TO_BUTTON: Record<string, ButtonName> = {
  ArrowUp: "UP",
  ArrowDown: "DOWN",
  ArrowLeft: "LEFT",
  ArrowRight: "RIGHT",
  KeyZ: "A",
  KeyX: "B",
  Enter: "START",
  Escape: "HOME",
  Space: "AUX1",
};

export interface ShellHandles {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  mountButtons(inject: ButtonInjector): void;
  mountKeyboard(inject: ButtonInjector): void;
  renderLeds(colors: Rgb[]): void;
}

export function mountShell(): ShellHandles {
  const canvas = document.getElementById("badge-screen") as HTMLCanvasElement | null;
  if (!canvas) throw new Error("shell: #badge-screen canvas not found");
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("shell: 2D canvas context unavailable");

  function mountButtons(inject: ButtonInjector): void {
    const pad = document.getElementById("button-pad");
    if (!pad) return;
    pad.innerHTML = "";
    for (const name of BUTTON_NAMES) {
      const btn = document.createElement("button");
      btn.textContent = name;
      btn.className = "badge-btn";
      btn.addEventListener("mousedown", (e) => {
        e.preventDefault();
        inject(name, "pressed");
      });
      btn.addEventListener("mouseup", (e) => {
        e.preventDefault();
        inject(name, "released");
      });
      // Release if the mouse is dragged off the button while held, so a
      // button can't get stuck "down" forever.
      btn.addEventListener("mouseleave", () => inject(name, "released"));
      pad.appendChild(btn);
    }
  }

  function mountKeyboard(inject: ButtonInjector): void {
    const heldKeys = new Set<string>();
    window.addEventListener("keydown", (e) => {
      const button = KEY_TO_BUTTON[e.code];
      if (!button) return;
      e.preventDefault();
      if (heldKeys.has(e.code)) return; // ignore OS auto-repeat while held
      heldKeys.add(e.code);
      inject(button, "pressed");
    });
    window.addEventListener("keyup", (e) => {
      const button = KEY_TO_BUTTON[e.code];
      if (!button) return;
      e.preventDefault();
      heldKeys.delete(e.code);
      inject(button, "released");
    });
  }

  function renderLeds(colors: Rgb[]): void {
    const row = document.getElementById("led-row");
    if (!row) return;
    if (row.children.length !== colors.length) {
      row.innerHTML = "";
      for (let i = 0; i < colors.length; i++) {
        const dot = document.createElement("span");
        dot.className = "led-dot";
        row.appendChild(dot);
      }
    }
    colors.forEach((c, i) => {
      const dot = row.children[i] as HTMLElement | undefined;
      if (dot) dot.style.background = `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
    });
  }

  return { canvas, ctx, mountButtons, mountKeyboard, renderLeds };
}

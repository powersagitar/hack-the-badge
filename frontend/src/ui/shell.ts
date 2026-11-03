/**
 * Page chrome: mounts the on-screen button pad (mouse) and keyboard
 * bindings that both inject `badge.input` events, a tiny LED HUD, and the
 * Lua-sandbox-vs-real-firmware mode toggle (`CLAUDE.md`'s architecture
 * section names this file as where that toggle lives).
 *
 * Keyboard mapping (documented per spec):
 *   ArrowUp/Down/Left/Right -> UP/DOWN/LEFT/RIGHT
 *   Z / X                   -> A / B
 *   Enter                   -> START
 *   Escape                  -> HOME
 *   Space                   -> AUX1
 * (unmapped: none needed for the smoke-test app; extend KEY_TO_BUTTON as needed.)
 *
 * The radio link panel ({@link buildLinkPanel}, Milestone 10 Task 10)
 * hosts or joins a WebRTC link, or swaps codes by hand; `main.ts` does the
 * linking.
 *
 * The button pad, keyboard bindings and canvas are shared between modes —
 * `shell.ts` itself has no notion of "which runtime"; `src/main.ts` owns
 * both runtimes and passes this module a single `ButtonInjector` that it
 * internally routes to whichever one is currently active. This module only
 * reports *which* mode was picked, via `mountModeToggle`'s callback.
 */
import { BUTTON_NAMES, type ButtonName } from "../badge/input";
import type { Rgb } from "../badge/led";

export type ButtonInjector = (button: ButtonName, kind: "pressed" | "released") => void;

/** The two emulator modes `CLAUDE.md`'s architecture section describes. */
export type EmulatorMode = "lua" | "firmware";

const MODE_OPTIONS: Array<{ value: EmulatorMode; label: string }> = [
  { value: "lua", label: "Lua Sandbox" },
  { value: "firmware", label: "Real Firmware" },
];

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

/** A firmware role string (`workshop_lead`) as a menu label (`workshop lead`). */
export function roleLabel(role: string): string {
  return role.replace(/_/g, " ");
}

export interface ProvisionControls {
  setStatus(text: string): void;
  setVisible(visible: boolean): void;
}

/** The "Badge running in another tab" notice (Milestone 10 Task 9). */
export interface BadgeNotice {
  setVisible(visible: boolean): void;
}

export interface ShellHandles {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  mountButtons(inject: ButtonInjector): void;
  mountKeyboard(inject: ButtonInjector): void;
  renderLeds(colors: Rgb[]): void;
  /**
   * Mounts (or re-renders) the mode-select control into `#mode-toggle`
   * (a no-op, not an error, if that element isn't present in the page —
   * same defensive style `mountButtons`/`renderLeds` already use for
   * `#button-pad`/`#led-row`). `current` controls which option starts
   * highlighted; `onSelect` fires once per user click on a *different*
   * option than the one currently highlighted (clicking the already-active
   * option is a no-op, not a redundant callback). The caller (`main.ts`)
   * owns actually switching runtimes; this function only reports the choice
   * and updates which button looks active.
   */
  mountModeToggle(current: EmulatorMode, onSelect: (mode: EmulatorMode) => void): void;
  /**
   * Mounts the role picker, "Provision test badge", "Shake" and "Bump"
   * buttons, a status span and a hint into `#provision-controls` (no-op
   * handles if the element is missing). `onProvision` gets the selected
   * role string.
   */
  mountProvisionControls(
    roles: readonly string[],
    onProvision: (role: string) => void,
    onShake: () => void,
    onBump: () => void,
  ): ProvisionControls;
  /**
   * Mounts the "Badge running in another tab — Run here" notice into
   * `#badge-notice`, hidden (no-op handles if the element is missing).
   */
  mountBadgeNotice(onTakeOver: () => void): BadgeNotice;
  /**
   * Mounts the radio link panel into `#link-panel` (no-op handles if the
   * element is missing); see {@link buildLinkPanel}.
   */
  mountLinkPanel(handlers: LinkPanelHandlers): LinkPanel;
}

/**
 * Builds the provisioning controls into `container` (see
 * `ShellHandles.mountProvisionControls`). Separate from {@link mountShell}
 * so it is testable with a fake `doc`.
 */
export function buildProvisionControls(
  container: HTMLElement,
  doc: Pick<Document, "createElement">,
  roles: readonly string[],
  onProvision: (role: string) => void,
  onShake: () => void,
  onBump: () => void,
): ProvisionControls {
  container.innerHTML = "";
  const select = doc.createElement("select");
  select.className = "provision-role";
  for (const role of roles) {
    const opt = doc.createElement("option");
    opt.value = role;
    opt.textContent = roleLabel(role);
    select.appendChild(opt);
  }
  const button = (label: string, onClick: () => void) => {
    const btn = doc.createElement("button");
    btn.type = "button";
    btn.className = "mode-btn";
    btn.textContent = label;
    btn.addEventListener("click", () => onClick());
    return btn;
  };
  const provisionBtn = button("Provision test badge", () => onProvision(select.value));
  const shakeBtn = button("Shake", onShake);
  // Connect's bump: pairs with every linked badge that is bumped too.
  const bumpBtn = button("Bump", onBump);
  const status = doc.createElement("span");
  status.className = "provision-status";
  const hint = doc.createElement("p");
  hint.className = "provision-hint";
  hint.textContent =
    "After PROV OK, follow the on-badge setup with the button pad; on \u201CShake it!\u201D press Shake. In Connect, press Bump.";
  container.append(select, provisionBtn, shakeBtn, bumpBtn, status, hint);
  return {
    setStatus: (text) => {
      status.textContent = text;
    },
    setVisible: (visible) => {
      container.style.display = visible ? "" : "none";
    },
  };
}

/**
 * Builds the "Badge running in another tab — Run here" notice into
 * `container`, hidden until `setVisible(true)`. "Run here" calls
 * `onTakeOver`.
 */
export function buildBadgeNotice(
  container: HTMLElement,
  doc: Pick<Document, "createElement">,
  onTakeOver: () => void,
): BadgeNotice {
  container.innerHTML = "";
  const text = doc.createElement("span");
  text.className = "badge-notice-text";
  text.textContent = "Badge running in another tab \u2014 ";
  const run = doc.createElement("button");
  run.type = "button";
  run.className = "mode-btn";
  run.textContent = "Run here";
  run.addEventListener("click", () => onTakeOver());
  container.append(text, run);
  const setVisible = (visible: boolean) => {
    container.style.display = visible ? "" : "none";
  };
  setVisible(false);
  return { setVisible };
}

/** One row of the link list: a radio link and its state. */
export interface LinkRow {
  id: string;
  label: string;
  state: "connecting" | "open" | "closed";
}

export interface LinkPanelHandlers {
  /** "Host link": create a room. */
  onHost(): void;
  /** "Join link", with the code input's text as typed. */
  onJoin(code: string): void;
  /** Manual "Create code": an offer code to copy to the peer. */
  onManualCreate(): void;
  /** Manual "Use code", with the pasted code (an offer or an answer). */
  onManualUse(code: string): void;
  /** A row's "Remove". */
  onRemove(id: string): void;
}

/** The radio link panel (Milestone 10 Task 10). */
export interface LinkPanel {
  setStatus(text: string): void;
  /** Shows a code for the user to copy, in the manual section. */
  setManualOutput(code: string): void;
  setLinks(links: readonly LinkRow[]): void;
  setVisible(visible: boolean): void;
}

const LINK_STATE_LABEL: Record<LinkRow["state"], string> = {
  connecting: "connecting…",
  open: "connected",
  closed: "disconnected",
};

/**
 * Builds the radio link panel into `container`, in this order: "Host link",
 * the room code input, "Join link", the "Manual" toggle, a status line,
 * the manual section (hidden until "Manual" is on: "Create code", a
 * read-only code to copy, a code to paste, "Use code"), and the link list,
 * each row with "Remove".
 */
export function buildLinkPanel(
  container: HTMLElement,
  doc: Pick<Document, "createElement">,
  handlers: LinkPanelHandlers,
): LinkPanel {
  container.innerHTML = "";
  const button = (label: string, onClick: () => void) => {
    const btn = doc.createElement("button");
    btn.type = "button";
    btn.className = "mode-btn";
    btn.textContent = label;
    btn.addEventListener("click", () => onClick());
    return btn;
  };

  const row = doc.createElement("div");
  row.className = "link-row";
  const code = doc.createElement("input");
  code.className = "link-code";
  code.type = "text";
  code.placeholder = "room code";
  code.maxLength = 16;
  code.autocomplete = "off";
  code.spellcheck = false;
  const host = button("Host link", () => handlers.onHost());
  const join = button("Join link", () => handlers.onJoin(code.value));
  let manualOn = false;
  const manualToggle = button("Manual", () => {
    manualOn = !manualOn;
    manualToggle.className = "mode-btn" + (manualOn ? " active" : "");
    manual.style.display = manualOn ? "" : "none";
  });
  row.append(host, code, join, manualToggle);

  const status = doc.createElement("span");
  status.className = "link-status";

  const manual = doc.createElement("div");
  manual.className = "link-manual";
  manual.style.display = "none";
  const out = doc.createElement("textarea");
  out.className = "link-manual-out";
  out.readOnly = true;
  out.rows = 3;
  out.placeholder = "your code appears here: copy it to the other browser";
  const paste = doc.createElement("textarea");
  paste.className = "link-manual-in";
  paste.rows = 3;
  paste.placeholder = "paste the other browser's code here";
  manual.append(
    button("Create code", () => handlers.onManualCreate()),
    out,
    paste,
    button("Use code", () => handlers.onManualUse(paste.value)),
  );

  const list = doc.createElement("ul");
  list.className = "link-list";

  container.append(row, status, manual, list);
  return {
    setStatus: (text) => {
      status.textContent = text;
    },
    setManualOutput: (text) => {
      out.value = text;
    },
    setLinks: (links) => {
      list.innerHTML = "";
      for (const link of links) {
        const item = doc.createElement("li");
        item.className = `link-item link-${link.state}`;
        const label = doc.createElement("span");
        label.textContent = `${link.label} — ${LINK_STATE_LABEL[link.state]} `;
        item.append(label, button("Remove", () => handlers.onRemove(link.id)));
        list.appendChild(item);
      }
    },
    setVisible: (visible) => {
      container.style.display = visible ? "" : "none";
    },
  };
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

  function mountModeToggle(current: EmulatorMode, onSelect: (mode: EmulatorMode) => void): void {
    const container = document.getElementById("mode-toggle");
    if (!container) return;
    container.innerHTML = "";
    for (const { value, label } of MODE_OPTIONS) {
      const btn = document.createElement("button");
      btn.textContent = label;
      btn.type = "button";
      btn.className = "mode-btn" + (value === current ? " active" : "");
      btn.addEventListener("click", () => {
        if (value === current) return; // already active, nothing to do
        onSelect(value);
      });
      container.appendChild(btn);
    }
  }

  function mountProvisionControls(
    roles: readonly string[],
    onProvision: (role: string) => void,
    onShake: () => void,
    onBump: () => void,
  ): ProvisionControls {
    const container = document.getElementById("provision-controls");
    if (!container) return { setStatus: () => {}, setVisible: () => {} };
    return buildProvisionControls(container, document, roles, onProvision, onShake, onBump);
  }

  function mountBadgeNotice(onTakeOver: () => void): BadgeNotice {
    const container = document.getElementById("badge-notice");
    if (!container) return { setVisible: () => {} };
    return buildBadgeNotice(container, document, onTakeOver);
  }

  function mountLinkPanel(handlers: LinkPanelHandlers): LinkPanel {
    const container = document.getElementById("link-panel");
    if (!container) return { setStatus() {}, setManualOutput() {}, setLinks() {}, setVisible() {} };
    return buildLinkPanel(container, document, handlers);
  }

  return {
    canvas,
    ctx,
    mountButtons,
    mountKeyboard,
    renderLeds,
    mountModeToggle,
    mountProvisionControls,
    mountBadgeNotice,
    mountLinkPanel,
  };
}

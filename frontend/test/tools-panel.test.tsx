import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { BadgeSnapshot } from "../src/ui/badge-controller";
import type { ToolsActions } from "../src/ui/ToolsPanel";

/**
 * FIRMWARE TOOLS and the tab notice on a happy-dom `Window`, installed for
 * this file only, as test/app-shell.test.tsx does (see its comment). The
 * panel renders from a snapshot and calls the controller's actions; these
 * tests hand it snapshots and record the calls. Ported from the old
 * `test/shell.test.ts`'s provision-controls, badge-notice and link-panel
 * cases (Milestone 14 Task 12).
 *
 * Radix's Select also reaches the DOM through globals (`instanceof
 * HTMLFormElement`, `DocumentFragment`, `getComputedStyle`, ...), so every
 * function the window has and Bun's global lacks is installed too, and
 * removed after.
 */
const BASE = ["window", "document", "navigator", "IS_REACT_ACT_ENVIRONMENT"];

let win: Window;
const saved = new Map<string, PropertyDescriptor | undefined>();
let rtl: typeof import("@testing-library/react/pure");
let tools: typeof import("../src/ui/ToolsPanel");

beforeAll(async () => {
  win = new Window({ url: "http://localhost:5173/" });
  const values: Record<string, unknown> = {
    window: win,
    document: win.document,
    navigator: win.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  };
  const w = win as unknown as Record<string, unknown>;
  for (const name of Object.getOwnPropertyNames(win)) {
    // Events too, Bun's own included: Radix dispatches `new CustomEvent` on happy-dom nodes.
    if (typeof w[name] === "function" && (!(name in globalThis) || /Event$/.test(name))) values[name] = w[name];
  }
  for (const name of [...BASE, ...Object.keys(values).filter((n) => !BASE.includes(n))]) {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value: values[name], configurable: true, writable: true });
  }
  rtl = await import("@testing-library/react/pure");
  tools = await import("../src/ui/ToolsPanel");
});

afterEach(() => {
  rtl.cleanup();
});

afterAll(async () => {
  for (const [name, d] of saved) {
    if (d) Object.defineProperty(globalThis, name, d);
    else delete (globalThis as Record<string, unknown>)[name];
  }
  await win.happyDOM.close();
});

const IDLE: BadgeSnapshot = {
  speed: 1,
  rebooting: false,
  provision: null,
  skip: null,
  setupOnScreen: false,
  tabNotice: false,
  linkStatus: "",
  hostingRoom: null,
  motion: "physics",
  phoneMotionAvailable: false,
};

function actions() {
  const calls: string[] = [];
  const a: ToolsActions = {
    provision: async (role) => void calls.push(`provision ${role}`),
    skipSetup: () => void calls.push("skip"),
    host: async () => {
      calls.push("host");
      return "";
    },
    cancelHost: () => void calls.push("cancelHost"),
    join: async (code) => void calls.push(`join ${code}`),
    usePhoneMotion: async (on) => void calls.push(`phone ${on}`),
  };
  return { a, calls };
}

function render(snapshot: Partial<BadgeSnapshot> = {}) {
  const { a, calls } = actions();
  const view = rtl.render(<tools.ToolsPanel snapshot={{ ...IDLE, ...snapshot }} actions={a} />);
  const rerender = (s: Partial<BadgeSnapshot>) =>
    view.rerender(<tools.ToolsPanel snapshot={{ ...IDLE, ...s }} actions={a} />);
  const button = (name: string | RegExp) => view.getByRole("button", { name }) as HTMLButtonElement;
  return { view, calls, rerender, button };
}

describe("ToolsPanel", () => {
  test("roleLabel shows a firmware role string as words", () => {
    expect(tools.roleLabel("hacker")).toBe("hacker");
    expect(tools.roleLabel("workshop_lead")).toBe("workshop lead");
  });

  test("shows the speed index and the sections, with no Shake or Bump", () => {
    const { view } = render({ speed: 0.84 });
    expect(view.getByText("FIRMWARE TOOLS")).toBeTruthy();
    expect(view.getByText("0.8x")).toBeTruthy();
    expect(view.container.textContent).toContain("SPEED INDEX");
    expect(view.getByRole("group", { name: "Provisioning" })).toBeTruthy();
    expect(view.getByRole("group", { name: "Badge link" })).toBeTruthy();
    expect(view.container.textContent).not.toMatch(/Shake|Bump/);
    expect(view.container.querySelector("textarea, ul")).toBeNull();
  });

  test("skip enabled only when setup on screen", () => {
    const r = render();
    expect(r.button("Skip setup").disabled).toBe(true);
    r.rerender({ setupOnScreen: true });
    expect(r.button("Skip setup").disabled).toBe(false);
    r.button("Skip setup").click();
    expect(r.calls).toEqual(["skip"]);
    // Not while a skip types, nor while a provision is in flight (its token phase runs on Setup).
    r.rerender({ setupOnScreen: true, skip: { kind: "typing" } });
    expect(r.button("Skip setup").disabled).toBe(true);
    r.rerender({ setupOnScreen: true, provision: { kind: "typing" } });
    expect(r.button("Skip setup").disabled).toBe(true);
    r.rerender({ setupOnScreen: true, provision: { kind: "waiting" } });
    expect(r.button("Skip setup").disabled).toBe(true);
    r.rerender({ setupOnScreen: true, provision: { kind: "ok", line: "PROV OK" } });
    expect(r.button("Skip setup").disabled).toBe(false);
    r.rerender({ setupOnScreen: true, skip: { kind: "failed", reason: "timed out" } });
    expect(r.button("Skip setup").disabled).toBe(false);
  });

  test("PROVISION is disabled while a provision or skip runs, and provisions the chosen role", () => {
    const r = render();
    expect(r.button("PROVISION").disabled).toBe(false);
    r.button("PROVISION").click();
    expect(r.calls).toEqual(["provision hacker"]);
    for (const s of [
      { provision: { kind: "waiting" } },
      { provision: { kind: "typing" } },
      { skip: { kind: "typing" } },
    ] as Partial<BadgeSnapshot>[]) {
      r.rerender(s);
      expect(r.button("PROVISION").disabled).toBe(true);
    }
    r.rerender({ provision: { kind: "failed", reason: "x" }, skip: { kind: "done" } });
    expect(r.button("PROVISION").disabled).toBe(false);
  });

  test("the ROLE select lists every role, labelled", () => {
    const r = render();
    const trigger = r.view.getByRole("combobox", { name: "Role" });
    expect(trigger.textContent).toContain("hacker");
  });

  test("keys in the open ROLE select never reach the badge, and Escape closes it", async () => {
    const { keyAction } = await import("../src/ui/keyboard");
    const r = render();
    const actionsSeen: unknown[] = [];
    const onKey = (e: KeyboardEvent) => void actionsSeen.push(keyAction(e.code, e.target));
    window.addEventListener("keydown", onKey);
    try {
      const trigger = r.view.getByRole("combobox", { name: "Role" });
      trigger.focus();
      rtl.fireEvent.keyDown(trigger, { key: "ArrowDown", code: "ArrowDown" });
      const options = await r.view.findAllByRole("option");
      expect(options.map((o) => o.textContent)).toContain("workshop lead");
      const target = (document.activeElement ?? options[0]!) as HTMLElement;
      rtl.fireEvent.keyDown(target, { key: "ArrowDown", code: "ArrowDown" });
      rtl.fireEvent.keyDown(document.activeElement ?? target, { key: "Escape", code: "Escape" });
      await rtl.waitFor(() => expect(r.view.queryAllByRole("option")).toHaveLength(0));
      expect(actionsSeen.length).toBeGreaterThanOrEqual(3);
      expect(actionsSeen.every((a) => a === null)).toBe(true);
    } finally {
      window.removeEventListener("keydown", onKey);
    }
  });

  test("phone motion button hidden when unavailable", () => {
    const r = render({ phoneMotionAvailable: false });
    expect(r.view.queryByRole("button", { name: /phone motion/i })).toBeNull();
    r.rerender({ phoneMotionAvailable: true });
    const b = r.button(/phone motion/i);
    expect(b.getAttribute("aria-pressed")).toBe("false");
    b.click();
    r.rerender({ phoneMotionAvailable: true, motion: "phone" });
    expect(r.button(/phone motion/i).getAttribute("aria-pressed")).toBe("true");
    r.button(/phone motion/i).click();
    expect(r.calls).toEqual(["phone true", "phone false"]);
  });

  test("BADGE LINK: the code is uppercased, Join needs a code, and the status shows", () => {
    const r = render({ linkStatus: "Linked to 2 badges" });
    expect(r.view.getByText("Linked to 2 badges")).toBeTruthy();
    const input = r.view.getByRole("textbox", { name: "Room code" }) as HTMLInputElement;
    expect(input.maxLength).toBe(16);
    expect(r.button("Join link").disabled).toBe(true);
    rtl.fireEvent.change(input, { target: { value: "k3x9qf" } });
    expect(input.value).toBe("K3X9QF");
    expect(r.button("Join link").disabled).toBe(false);
    r.button("Join link").click();
    r.button("Host link").click();
    expect(r.calls).toEqual(["join K3X9QF", "host"]);
  });

  test("a hosted room can be cancelled from the Host button", () => {
    const r = render({ hostingRoom: "K3X9QF", linkStatus: "Hosting K3X9QF" });
    expect(r.view.queryByRole("button", { name: "Host link" })).toBeNull();
    r.button("Cancel host").click();
    expect(r.calls).toEqual(["cancelHost"]);
  });
});

describe("TabNotice", () => {
  test("says the badge runs in another tab, and Run here takes it over", () => {
    let runs = 0;
    const view = rtl.render(<tools.TabNotice onRunHere={() => runs++} />);
    expect(view.getByRole("status").textContent).toContain("Badge running in another tab");
    (view.getByRole("button", { name: "Run here" }) as HTMLButtonElement).click();
    expect(runs).toBe(1);
  });
});

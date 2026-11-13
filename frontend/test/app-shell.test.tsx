import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { AppController } from "../src/ui/App";
import type { BadgeSnapshot } from "../src/ui/badge-controller";

/**
 * The React page's tests run on a happy-dom `Window`, installed as the
 * global `window` and `document` for this file only (no global
 * registrator; the previous values are restored after), so other test
 * files see no DOM. React and Testing Library are imported once the DOM
 * is in place, because react-dom looks for `window` when it loads.
 * Testing Library comes in through its `pure` entry, which registers no
 * hooks of its own (its default entry calls `beforeAll` at import, which
 * Bun refuses there): each test cleans up itself.
 *
 * The page now renders FIRMWARE TOOLS, whose Radix Select reaches the DOM
 * through globals, so every function the window has and Bun's global
 * lacks (and the window's event classes) is installed too, as
 * test/tools-panel.test.tsx does.
 */
const BASE = ["window", "document", "navigator", "IS_REACT_ACT_ENVIRONMENT"];

let win: Window;
const saved = new Map<string, PropertyDescriptor | undefined>();
let rtl: typeof import("@testing-library/react/pure");
let App: typeof import("../src/ui/App").App;
let consoleLog: typeof import("../src/ui/console-log");

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
    if (typeof w[name] === "function" && (!(name in globalThis) || /Event$/.test(name))) values[name] = w[name];
  }
  for (const name of [...BASE, ...Object.keys(values).filter((n) => !BASE.includes(n))]) {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value: values[name], configurable: true, writable: true });
  }
  rtl = await import("@testing-library/react/pure");
  ({ App } = await import("../src/ui/App"));
  consoleLog = await import("../src/ui/console-log");
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

/** A controller with a fixed snapshot (and `set` to change it), recording Run here. */
function fakeController(fields: Partial<BadgeSnapshot> = {}) {
  let snapshot: BadgeSnapshot = {
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
    ...fields,
  };
  const listeners = new Set<() => void>();
  const calls: string[] = [];
  const log = consoleLog.createConsoleLog();
  const sink = () => {};
  const controller: AppController = {
    subscribe: (cb) => {
      listeners.add(cb);
      return () => void listeners.delete(cb);
    },
    getSnapshot: () => snapshot,
    client: { injectButton() {}, powerCycle() {}, rebooting: () => false, onLeds: () => () => {} },
    console: log,
    setCanvas() {},
    motionSink: () => sink,
    runHere: async () => void calls.push("runHere"),
    provision: async () => {},
    skipSetup() {},
    host: async () => "",
    cancelHost() {},
    join: async () => {},
    usePhoneMotion: async () => {},
  };
  const set = (next: Partial<BadgeSnapshot>) => {
    snapshot = { ...snapshot, ...next };
    for (const cb of [...listeners]) cb();
  };
  return { controller, calls, log, set };
}

describe("App", () => {
  test("App renders header and footer", () => {
    const view = rtl.render(<App controller={fakeController().controller} />);
    expect(view.getByText("HACK THE NORTH 2026")).toBeTruthy();
    expect(view.getByRole("heading", { level: 1 }).textContent).toBe("Badge Emulator.");
    expect(view.getByText("Source on GitHub").closest("a")?.getAttribute("href")).toBe(
      "https://github.com/powersagitar/hack-the-badge",
    );
    expect(view.getByText("GPL-3.0").getAttribute("href")).toBe("https://www.gnu.org/licenses/gpl-3.0.html");
    expect(view.queryByText(/Lua Sandbox/)).toBeNull();
    expect(view.queryByText(/Real Firmware/)).toBeNull();
    expect(view.queryByRole("group", { name: "Emulator mode" })).toBeNull();
  });

  test("App lays out the stage, then FIRMWARE TOOLS and CONSOLE.LOG in the aside", () => {
    const { controller, log } = fakeController();
    log.appendPage("hello from the page");
    const view = rtl.render(<App controller={controller} />);
    const stage = view.container.querySelector('[data-slot="stage"]');
    const aside = view.container.querySelector("aside");
    expect(stage).not.toBeNull();
    expect(stage!.querySelector('img[alt="Hack the North 2026 hacker badge, front"]')).not.toBeNull();
    const sections = [...aside!.querySelectorAll(":scope > section")].map((s) => s.textContent ?? "");
    expect(sections).toHaveLength(2);
    expect(sections[0]).toContain("FIRMWARE TOOLS");
    expect(sections[1]).toContain("CONSOLE.LOG");
    expect(sections[1]).toContain("[page] hello from the page");
    // The stage comes before the aside, and the footer after both.
    const order = [...view.container.querySelectorAll("header, [data-slot=stage], aside, footer")].map((el) =>
      el.getAttribute("data-slot") ?? el.tagName.toLowerCase(),
    );
    expect(order).toEqual(["header", "stage", "aside", "footer"]);
  });

  test("the tab notice shows with the snapshot, and Run here takes the badge", () => {
    const { controller, calls, set } = fakeController();
    const view = rtl.render(<App controller={controller} />);
    expect(view.queryByText("Badge running in another tab")).toBeNull();
    rtl.act(() => set({ tabNotice: true }));
    expect(view.getByText("Badge running in another tab")).toBeTruthy();
    (view.getByRole("button", { name: "Run here" }) as HTMLButtonElement).click();
    expect(calls).toEqual(["runHere"]);
    rtl.act(() => set({ tabNotice: false }));
    expect(view.queryByText("Badge running in another tab")).toBeNull();
  });
});

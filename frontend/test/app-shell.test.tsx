import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { AppController } from "../src/ui/App";

/**
 * The React shell's tests run on a happy-dom `Window`, installed as the
 * global `window` and `document` for this file only (no global
 * registrator; the previous values are restored after), so other test
 * files see no DOM. React and Testing Library are imported once the DOM
 * is in place, because react-dom looks for `window` when it loads.
 * Testing Library comes in through its `pure` entry, which registers no
 * hooks of its own (its default entry calls `beforeAll` at import, which
 * Bun refuses there): each test cleans up itself.
 */
const GLOBALS = ["window", "document", "navigator", "IS_REACT_ACT_ENVIRONMENT"] as const;

let win: Window;
const saved = new Map<string, PropertyDescriptor | undefined>();
let rtl: typeof import("@testing-library/react/pure");
let App: typeof import("../src/ui/App").App;

/** The shell reads nothing from its controller yet (Task 12 brings the store). */
const fakeController: AppController = {};

beforeAll(async () => {
  win = new Window({ url: "http://localhost:5173/" });
  const values: Record<(typeof GLOBALS)[number], unknown> = {
    window: win,
    document: win.document,
    navigator: win.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  };
  for (const name of GLOBALS) {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value: values[name], configurable: true, writable: true });
  }
  rtl = await import("@testing-library/react/pure");
  ({ App } = await import("../src/ui/App"));
});

afterEach(() => {
  rtl.cleanup();
});

afterAll(async () => {
  for (const name of GLOBALS) {
    const d = saved.get(name);
    if (d) Object.defineProperty(globalThis, name, d);
    else delete (globalThis as Record<string, unknown>)[name];
  }
  await win.happyDOM.close();
});

describe("App", () => {
  test("App renders header and footer", () => {
    const view = rtl.render(<App controller={fakeController} />);
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

  test("App lays out the stage with the badge and an empty aside slot", () => {
    const view = rtl.render(<App controller={fakeController} />);
    const stage = view.container.querySelector('[data-slot="stage"]');
    const aside = view.container.querySelector("aside");
    expect(stage).not.toBeNull();
    expect(aside).not.toBeNull();
    expect(stage!.querySelector('img[alt="Hack the North 2026 hacker badge, front"]')).not.toBeNull();
    expect(aside!.childElementCount).toBe(0);
    // The stage comes before the aside, and the footer after both.
    const order = [...view.container.querySelectorAll("header, [data-slot=stage], aside, footer")].map((el) =>
      el.getAttribute("data-slot") ?? el.tagName.toLowerCase(),
    );
    expect(order).toEqual(["header", "stage", "aside", "footer"]);
  });
});

import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";

/**
 * The CONSOLE.LOG panel on a happy-dom `Window`, installed for this file
 * only, as test/app-shell.test.tsx does (see its comment). happy-dom does
 * no layout, so the tail-following rule is tested through its pure
 * function, and the DOM test checks the rows and their styling.
 */
const GLOBALS = ["window", "document", "navigator", "IS_REACT_ACT_ENVIRONMENT"] as const;

let win: Window;
const saved = new Map<string, PropertyDescriptor | undefined>();
let rtl: typeof import("@testing-library/react/pure");
let panel: typeof import("../src/ui/ConsolePanel");
let model: typeof import("../src/ui/console-log");

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
  panel = await import("../src/ui/ConsolePanel");
  model = await import("../src/ui/console-log");
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

describe("ConsolePanel", () => {
  test("renders entries newest at the bottom with muted page lines", () => {
    const log = model.createConsoleLog();
    log.appendSerial("I (1) boot: hello\n", 0);
    const view = rtl.render(<panel.ConsolePanel log={log} />);
    expect(view.getByText("CONSOLE.LOG")).toBeTruthy();

    rtl.act(() => {
      log.appendPage("provisioning…");
      log.appendSerial("I (2) app: world\n", 0);
    });
    const rows = view.container.querySelectorAll("[data-source]");
    expect([...rows].map((r) => r.textContent)).toEqual([
      "I (1) boot: hello",
      "[page] provisioning…",
      "I (2) app: world",
    ]);
    const page = rows[1]!;
    expect(page.getAttribute("data-source")).toBe("page");
    expect(page.className).toContain("text-muted-foreground");
    expect(rows[0]!.className).not.toContain("text-muted-foreground");
  });

  test("footer slot renders under the log", () => {
    const log = model.createConsoleLog();
    const view = rtl.render(<panel.ConsolePanel log={log} footer={<input aria-label="console input" />} />);
    expect(view.getByLabelText("console input")).toBeTruthy();
  });

  test("follows the tail unless scrolled up more than 24 px", () => {
    expect(panel.TAIL_SLACK_PX).toBe(24);
    // scrollHeight, scrollTop, clientHeight
    expect(panel.nearTail(1000, 640, 360)).toBe(true);
    expect(panel.nearTail(1000, 616, 360)).toBe(true);
    expect(panel.nearTail(1000, 615, 360)).toBe(false);
    expect(panel.nearTail(100, 0, 360)).toBe(true);
  });
});

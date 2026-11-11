import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { Window } from "happy-dom";
import { BADGE_LOCK_NAME } from "../src/radio/lock";
import { MemoryStore, handleSignal, memoryDeps } from "../src/radio/signal-handler";
import type { SignalDeps } from "../src/radio/signal-client";
import type { FromWorker, ToWorker } from "../src/runtime/firmware-worker-protocol";
import { type BadgeWorker, createBadgeController, mountLinks } from "../src/ui/badge-controller";
import { buildBadgeNotice, buildLinkPanel, buildProvisionControls, mountShell, roleLabel } from "../src/ui/shell";
import { FakeLockManager } from "./fake-locks";
import { FakeNetwork } from "./fake-rtc";

/**
 * The shell's tests run on a real DOM: a happy-dom `Window` per test, with
 * `index.html`'s body (its script dropped) and `shell.css` inline, set as
 * `globalThis.document` for the test and the previous value restored after.
 * No global registrator: other test files see no DOM.
 */
const INDEX_HTML = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const SHELL_CSS = readFileSync(new URL("../src/ui/shell.css", import.meta.url), "utf8");
/** `shell.css`'s `--bg`: the page's background. */
const PAGE_BG = /--bg:\s*([^;]+);/.exec(SHELL_CSS)![1]!.trim();

let win: Window;
let savedDocument: PropertyDescriptor | undefined;

beforeEach(() => {
  savedDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  win = new Window({ url: "http://localhost:5173/" });
  const body = /<body>([\s\S]*)<\/body>/.exec(INDEX_HTML)![1]!.replace(/<script[\s\S]*?<\/script>/g, "");
  win.document.write(`<!doctype html><html><head><style>${SHELL_CSS}</style></head><body>${body}</body></html>`);
  Object.defineProperty(globalThis, "document", { value: win.document, configurable: true, writable: true });
});

afterEach(async () => {
  if (savedDocument) Object.defineProperty(globalThis, "document", savedDocument);
  else delete (globalThis as { document?: unknown }).document;
  await win.happyDOM.close();
});

const doc = () => globalThis.document;
const settle = (ms = 0) => new Promise<void>((r) => setTimeout(r, ms));
const buttons = (root: ParentNode) => [...root.querySelectorAll("button")];
const button = (root: ParentNode, label: string) => buttons(root).find((b) => b.textContent === label)!;
const container = () => doc().createElement("div");
/**
 * A happy-dom mouse event, typed as the DOM lib's for `dispatchEvent` on a
 * DOM-typed element: happy-dom's typings lack only the legacy `isTrusted`,
 * `returnValue` and `srcElement` fields, which no listener here reads.
 */
const mouse = (type: string) => new win.MouseEvent(type) as unknown as MouseEvent;

/** A 2D context that records its fills: the colour and the rectangle. */
function fakeCtx() {
  const fills: Array<{ style: string; x: number; y: number; w: number; h: number }> = [];
  const ctx = {
    fillStyle: "",
    fillRect(x: number, y: number, w: number, h: number) {
      fills.push({ style: String(ctx.fillStyle), x, y, w, h });
    },
    clearRect() {},
    putImageData() {},
  };
  return { ctx, fills };
}

/** The page's real shell, over a fake 2D context on `#badge-screen` (happy-dom has no canvas). */
function pageShell() {
  const canvas = doc().getElementById("badge-screen") as HTMLCanvasElement;
  const { ctx, fills } = fakeCtx();
  (canvas as unknown as { getContext: () => unknown }).getContext = () => ctx;
  return { shell: mountShell(), canvas, fills };
}

describe("roleLabel", () => {
  test("shows a firmware role string as words", () => {
    expect(roleLabel("hacker")).toBe("hacker");
    expect(roleLabel("workshop_lead")).toBe("workshop lead");
  });
});

describe("provision controls", () => {
  test("a Bump button sits next to Shake, and each calls its own handler", () => {
    const root = container();
    const calls: string[] = [];
    buildProvisionControls(
      root,
      doc(),
      ["hacker"],
      (role) => calls.push(`provision ${role}`),
      () => calls.push("shake"),
      () => calls.push("bump"),
    );
    const labels = buttons(root).map((b) => b.textContent);
    expect(labels).toContain("Shake");
    expect(labels.indexOf("Bump")).toBe(labels.indexOf("Shake") + 1);
    button(root, "Bump").click();
    button(root, "Shake").click();
    expect(calls).toEqual(["bump", "shake"]);
  });
});

describe("badge notice", () => {
  test("says the badge runs in another tab, and Run here takes it over", () => {
    const root = container();
    let takeOvers = 0;
    const notice = buildBadgeNotice(root, doc(), () => takeOvers++);
    expect(root.textContent).toContain("Badge running in another tab");
    const run = button(root, "Run here");
    expect(run).toBeDefined();
    // Hidden until the badge is found taken.
    expect(root.style.display).toBe("none");
    notice.setVisible(true);
    expect(root.style.display).toBe("");
    run.click();
    expect(takeOvers).toBe(1);
    notice.setVisible(false);
    expect(root.style.display).toBe("none");
  });
});

describe("link panel", () => {
  function build() {
    const root = container();
    const calls: string[] = [];
    const panel = buildLinkPanel(root, doc(), {
      onHost: () => calls.push("host"),
      onJoin: (code) => calls.push(`join ${code}`),
      onManualCreate: () => calls.push("manual create"),
      onManualUse: (code) => calls.push(`manual use ${code}`),
      onRemove: (id) => calls.push(`remove ${id}`),
    });
    const byClass = <T extends HTMLElement = HTMLElement>(cls: string) => root.querySelector<T>(`.${cls}`)!;
    return { root, calls, panel, byClass };
  }

  test("Host link, then the join code input and Join link, then Manual, in that order", () => {
    const { root, calls, byClass } = build();
    const labels = buttons(root).map((b) => b.textContent);
    expect(labels.slice(0, 3)).toEqual(["Host link", "Join link", "Manual"]);
    button(root, "Host link").click();
    const input = byClass<HTMLInputElement>("link-code");
    expect(input.tagName).toBe("INPUT");
    input.value = "abcd-efgh";
    button(root, "Join link").click();
    expect(calls).toEqual(["host", "join abcd-efgh"]);
  });

  test("the status shows a hosted room code", async () => {
    // The real path: the page's #link-panel, Host link pressed, a room
    // created through the real signaling handler, and its code read back
    // from the DOM.
    const signal = memoryDeps(undefined, new MemoryStore());
    const net = new FakeNetwork();
    const rooms: string[] = [];
    const deps: SignalDeps = {
      factory: net.factory,
      fetch: async (input, init) => {
        const res = await handleSignal(new Request(new URL(String(input), "http://localhost"), init), signal);
        const reply = (await res.clone().json().catch(() => null)) as { room?: string } | null;
        if (reply?.room) rooms.push(reply.room);
        return res;
      },
      // Nobody joins: the host waits.
      sleep: () => new Promise<void>(() => {}),
    };
    const hub = { addLink() {}, removeLink() {} };
    const links = mountLinks(pageShell().shell, hub, deps);
    const panelEl = doc().getElementById("link-panel")!;
    button(panelEl, "Host link").click();
    await settle(5);
    expect(rooms).toHaveLength(1);
    const status = panelEl.querySelector(".link-status")!.textContent!;
    expect(status).toContain(`Room ${rooms[0]}`);
    expect(panelEl.querySelector(".link-list")!.textContent).toContain(`room ${rooms[0]}`);
    links.closeAll();
  });

  test("hosting twice before the first room is made closes the first room", async () => {
    const signal = memoryDeps(undefined, new MemoryStore());
    const net = new FakeNetwork();
    const ops: string[] = [];
    const deps: SignalDeps = {
      factory: net.factory,
      fetch: async (input, init) => {
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as { op?: string; room?: string }) : {};
        if (body.op) ops.push(body.op === "close" ? `close ${body.room}` : body.op);
        const res = await handleSignal(new Request(new URL(String(input), "http://localhost"), init), signal);
        const reply = (await res.clone().json().catch(() => null)) as { room?: string } | null;
        if (reply?.room) ops.push(`room ${reply.room}`);
        return res;
      },
      sleep: () => new Promise<void>(() => {}),
    };
    const links = mountLinks(pageShell().shell, { addLink() {}, removeLink() {} }, deps);
    const panelEl = doc().getElementById("link-panel")!;
    button(panelEl, "Host link").click();
    button(panelEl, "Host link").click();
    await settle(5);
    const rooms = ops.filter((o) => o.startsWith("room ")).map((o) => o.slice(5));
    expect(rooms).toHaveLength(2);
    // The first press's room is closed as soon as it is made; the second's is shown.
    expect(ops).toContain(`close ${rooms[0]}`);
    expect(ops).not.toContain(`close ${rooms[1]}`);
    expect(panelEl.querySelector(".link-status")!.textContent).toContain(`Room ${rooms[1]}`);
    expect(panelEl.querySelector(".link-list")!.children.length).toBe(1);
    links.closeAll();
    await settle(5);
    expect(ops).toContain(`close ${rooms[1]}`);
  });

  test("the Manual toggle shows the copy-paste codes", () => {
    const { root, calls, panel, byClass } = build();
    const manual = byClass("link-manual");
    expect(manual.style.display).toBe("none");
    const toggle = button(root, "Manual");
    toggle.click();
    expect(manual.style.display).toBe("");
    expect(toggle.className).toContain("active");
    button(root, "Create code").click();
    panel.setManualOutput("offer:abc");
    expect(byClass<HTMLTextAreaElement>("link-manual-out").value).toBe("offer:abc");
    byClass<HTMLTextAreaElement>("link-manual-in").value = "answer:xyz";
    button(root, "Use code").click();
    expect(calls).toEqual(["manual create", "manual use answer:xyz"]);
    toggle.click();
    expect(manual.style.display).toBe("none");
    expect(toggle.className).not.toContain("active");
  });

  test("the link list shows each link's state, with Remove", () => {
    const { root, calls, panel, byClass } = build();
    panel.setLinks([
      { id: "link-1", label: "Link 1 (room ABCDEFGH)", state: "open" },
      { id: "link-2", label: "Link 2 (manual)", state: "connecting" },
    ]);
    const list = byClass("link-list");
    expect(list.children.length).toBe(2);
    expect(list.children[0]!.textContent).toContain("Link 1 (room ABCDEFGH)");
    expect(list.children[0]!.textContent).toContain("connected");
    expect(list.children[1]!.textContent).toContain("connecting");
    button(list.children[1]!, "Remove").click();
    expect(calls).toEqual(["remove link-2"]);
    panel.setLinks([]);
    expect(list.children.length).toBe(0);
    expect(root.textContent).not.toContain("Link 1");
  });

  test("hides with the provisioning controls", () => {
    const { root, panel } = build();
    panel.setVisible(false);
    expect(root.style.display).toBe("none");
    panel.setVisible(true);
    expect(root.style.display).toBe("");
  });
});

/** A fake firmware Worker: answers `init` with `ready` (or `error`), and records what it was sent. */
function fakeWorker(boot: "ready" | "error") {
  const sent: ToWorker[] = [];
  const w: BadgeWorker = {
    postMessage(m) {
      sent.push(m);
      if (m.kind !== "init") return;
      const reply: FromWorker = boot === "ready" ? { kind: "ready" } : { kind: "error", message: "boot failed" };
      queueMicrotask(() => w.onmessage?.({ data: reply }));
    },
    onmessage: null,
    terminate() {},
    addEventListener() {},
  };
  return { w, sent };
}

/**
 * The badge controller over the page's real shell (happy-dom, a fake 2D
 * context on `#badge-screen`), a fake lock manager, and fake Workers. With
 * `otherTab`, another tab holds the badge first.
 */
function badgeRig(opts: { otherTab?: boolean; boot?: "ready" | "error" } = {}) {
  const { shell, canvas, fills } = pageShell();
  const locks = new FakeLockManager();
  if (opts.otherTab) void locks.request(BADGE_LOCK_NAME, {}, () => new Promise(() => {}));
  const workers: Array<ReturnType<typeof fakeWorker>> = [];
  const running: boolean[] = [];
  const controller = createBadgeController({
    shell,
    hub: { fromLocal() {}, setPaused() {}, setRunning: (r) => void running.push(r), bump: () => false },
    lua: { startPaint() {}, stopPaint() {}, injectButton() {} },
    roles: ["hacker"],
    loadImages: async () => ({ wasm: new ArrayBuffer(8), image: new ArrayBuffer(8) }),
    createWorker: () => {
      const fw = fakeWorker(opts.boot ?? "ready");
      workers.push(fw);
      return fw.w;
    },
    mac: () => new Uint8Array([0x02, 0, 0, 0, 0x10, 0x01]),
    fetchIdentity: async () => "{}",
    locks,
  });
  const el = (id: string) => doc().getElementById(id)!;
  const pick = (label: string) => button(el("mode-toggle"), label).click();
  const noticeShown = () => el("badge-notice").style.display !== "none";
  return { controller, shell, locks, workers, running, fills, el, pick, noticeShown, canvas };
}

/** Runs `body` with `console.error` silenced, returning what it was called with. */
async function quietErrors(body: () => Promise<void>): Promise<unknown[][]> {
  const original = console.error;
  const calls: unknown[][] = [];
  console.error = (...args: unknown[]) => void calls.push(args);
  try {
    await body();
  } finally {
    console.error = original;
  }
  return calls;
}

describe("badge controller", () => {
  test("starts in Lua mode, and Real Firmware boots and starts the badge", async () => {
    const r = badgeRig();
    expect(r.controller.mode()).toBe("lua");
    expect(r.el("mode-toggle").querySelector(".active")!.textContent).toBe("Lua Sandbox");
    r.pick("Real Firmware");
    await settle();
    expect(r.controller.mode()).toBe("firmware");
    expect(r.workers).toHaveLength(1);
    expect(r.workers[0]!.sent.map((m) => m.kind)).toEqual(["init", "setMac", "start"]);
    expect(r.locks.held()).toBe(true);
    expect(r.running).toEqual([true]);
    expect(r.noticeShown()).toBe(false);
  });

  test("switching_to_lua_releases_the_badge", async () => {
    const r = badgeRig();
    r.pick("Real Firmware");
    await settle();
    expect(r.locks.held()).toBe(true);
    r.pick("Lua Sandbox");
    await settle();
    expect(r.controller.mode()).toBe("lua");
    expect(r.locks.held()).toBe(false);
    expect(r.running).toEqual([true, false]);
    // Back to the firmware: the badge is taken again, the same Worker resumes.
    r.pick("Real Firmware");
    await settle();
    expect(r.locks.held()).toBe(true);
    expect(r.workers).toHaveLength(1);
    expect(r.running).toEqual([true, false, true]);
    // Unload frees it, and a second free does nothing.
    r.controller.dispose();
    r.controller.dispose();
    await settle();
    expect(r.locks.held()).toBe(false);
  });

  test("a_failed_firmware_start_releases_the_badge", async () => {
    const r = badgeRig({ boot: "error" });
    const errors = await quietErrors(async () => {
      r.pick("Real Firmware");
      await settle();
    });
    expect(errors.length).toBeGreaterThan(0);
    expect(r.controller.mode()).toBe("lua");
    expect(r.el("mode-toggle").querySelector(".active")!.textContent).toBe("Lua Sandbox");
    expect(r.locks.held()).toBe(false);
    expect(r.running).toEqual([]);
  });

  test("provision_without_the_badge_boots_no_worker_and_shows_the_notice", async () => {
    const r = badgeRig({ otherTab: true });
    r.pick("Real Firmware");
    await settle();
    expect(r.noticeShown()).toBe(true);
    expect(r.workers).toHaveLength(0);
    r.controller.refreshStatus();
    button(r.el("provision-controls"), "Provision test badge").click();
    await settle();
    expect(r.workers).toHaveLength(0);
    expect(r.noticeShown()).toBe(true);
    expect(r.locks.held()).toBe(true); // still the other tab's
  });

  test("a_failed_boot_from_provision_releases_the_badge", async () => {
    const r = badgeRig({ boot: "error" });
    // Another tab holds the badge while this one enters firmware mode, then frees it.
    let free = () => {};
    void r.locks.request(BADGE_LOCK_NAME, {}, () => new Promise<void>((resolve) => (free = resolve)));
    await settle();
    r.pick("Real Firmware");
    await settle();
    expect(r.controller.mode()).toBe("firmware");
    expect(r.workers).toHaveLength(0);
    free();
    await settle();
    expect(r.locks.held()).toBe(false);

    const errors = await quietErrors(async () => {
      button(r.el("provision-controls"), "Provision test badge").click();
      await settle();
    });
    expect(r.workers).toHaveLength(1); // the claim succeeded and the boot was tried
    expect(errors.length).toBeGreaterThan(0);
    expect(r.locks.held()).toBe(false);
    r.controller.refreshStatus();
    expect(r.el("provision-controls").textContent).toContain("could not load hacker");
  });

  test("the keyboard's buttons reach the Worker as the key source, the pad's as the pad", async () => {
    const r = badgeRig();
    // mountKeyboard listens on the global window; happy-dom's, for this test.
    const savedWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
    Object.defineProperty(globalThis, "window", { value: win, configurable: true, writable: true });
    try {
      r.shell.mountButtons(r.controller.inject);
      r.shell.mountKeyboard(r.controller.inject);
      r.pick("Real Firmware");
      await settle();
      const sent = r.workers[0]!.sent;
      win.dispatchEvent(new win.KeyboardEvent("keydown", { code: "KeyZ" }));
      button(r.el("button-pad"), "A").dispatchEvent(mouse("mousedown"));
      button(r.el("button-pad"), "A").dispatchEvent(mouse("mouseup"));
      win.dispatchEvent(new win.KeyboardEvent("keyup", { code: "KeyZ" }));
      expect(sent.filter((m) => m.kind === "button")).toEqual([
        { kind: "button", button: "A", pressed: true, source: "key" },
        { kind: "button", button: "A", pressed: true, source: "pad" },
        { kind: "button", button: "A", pressed: false, source: "pad" },
        { kind: "button", button: "A", pressed: false, source: "key" },
      ]);
    } finally {
      if (savedWindow) Object.defineProperty(globalThis, "window", savedWindow);
      else delete (globalThis as { window?: unknown }).window;
    }
  });

  test("notice_clears_the_canvas", async () => {
    const r = badgeRig({ otherTab: true });
    r.pick("Real Firmware");
    await settle();
    expect(r.noticeShown()).toBe(true);
    // The whole canvas, in the page's background colour (shell.css's --bg).
    expect(r.fills.at(-1)).toEqual({ style: PAGE_BG, x: 0, y: 0, w: r.canvas.width, h: r.canvas.height });
  });
});

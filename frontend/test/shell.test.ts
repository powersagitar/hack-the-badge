import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { Window } from "happy-dom";
import { BADGE_LOCK_NAME } from "../src/radio/lock";
import { MemoryStore, handleSignal, memoryDeps } from "../src/radio/signal-handler";
import type { SignalDeps } from "../src/radio/signal-client";
import type { FromWorker, ToWorker } from "../src/runtime/firmware-worker-protocol";
import { type BadgeWorker, START_FAILED_NOTE, createBadgeController, mountLinks } from "../src/ui/badge-controller";
import { buildBadgeNotice, buildLinkPanel, buildProvisionControls, mountShell, roleLabel } from "../src/ui/shell";
import { FakeLockManager } from "./fake-locks";
import { FakeNetwork } from "./fake-rtc";

/**
 * The shell's tests run on a real DOM: a happy-dom `Window` per test, with
 * the old page's body ({@link LEGACY_BODY}) and `shell.css` inline, set as
 * `globalThis.document` for the test and the previous value restored after.
 * No global registrator: other test files see no DOM.
 */
/**
 * The body `index.html` had before the React page (Milestone 14 Task 7),
 * which `shell.ts` builds into. `index.html` now mounts the React app;
 * Task 12 deletes `shell.ts`, `shell.css` and these tests.
 */
const LEGACY_BODY = `
    <div id="app">
      <h1>Hack the North 2026 &mdash; Badge Emulator</h1>
      <div id="provision-controls" style="display: none"></div>
      <div id="badge-notice" style="display: none"></div>
      <div id="link-panel" style="display: none"></div>
      <div id="badge-frame">
        <canvas id="badge-screen" width="320" height="240"></canvas>
        <div id="led-row"></div>
      </div>
      <div id="button-pad"></div>
      <div id="log-panel">
        <div id="log-header">console.log</div>
        <pre id="log-output"></pre>
      </div>
    </div>
`;
const SHELL_CSS = readFileSync(new URL("../src/ui/shell.css", import.meta.url), "utf8");
/** `shell.css`'s `--bg`: the page's background. */
const PAGE_BG = /--bg:\s*([^;]+);/.exec(SHELL_CSS)![1]!.trim();

let win: Window;
let savedDocument: PropertyDescriptor | undefined;

beforeEach(() => {
  savedDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  win = new Window({ url: "http://localhost:5173/" });
  const body = LEGACY_BODY;
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
 * A mouse event from the test's document's own window (happy-dom's at run
 * time; the DOM lib's `MouseEvent` by type), so `dispatchEvent` on a
 * DOM-typed element takes it without a cast.
 */
const mouse = (type: string) => new (doc().defaultView!.MouseEvent)(type);

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
  test("has no Shake or Bump button (Milestone 14: motion comes from the badge)", () => {
    const root = container();
    buildProvisionControls(root, doc(), ["hacker"], () => {}, () => {});
    const labels = buttons(root).map((b) => b.textContent);
    expect(labels).toEqual(["Provision test badge", "Skip setup"]);
    expect(root.textContent).not.toContain("Shake");
    expect(root.textContent).not.toContain("Bump");
  });

  test("Skip setup sits after Provision test badge, hidden until shown, and calls its handler", () => {
    const root = container();
    const calls: string[] = [];
    const controls = buildProvisionControls(
      root,
      doc(),
      ["hacker"],
      () => calls.push("provision"),
      () => calls.push("skip"),
    );
    const labels = buttons(root).map((b) => b.textContent);
    expect(labels.indexOf("Skip setup")).toBe(labels.indexOf("Provision test badge") + 1);
    const skip = button(root, "Skip setup");
    expect(skip.hidden).toBe(true);
    controls.setSkipVisible(true);
    expect(skip.hidden).toBe(false);
    skip.click();
    expect(calls).toEqual(["skip"]);
    controls.setSkipVisible(false);
    expect(skip.hidden).toBe(true);
    // The Provision button can be disabled, and enabled again.
    const provision = button(root, "Provision test badge");
    controls.setProvisionEnabled(false);
    expect(provision.disabled).toBe(true);
    controls.setProvisionEnabled(true);
    expect(provision.disabled).toBe(false);
    expect(root.querySelector(".provision-hint")!.textContent).toContain(
      "or press Skip setup to jump past the setup.",
    );
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
    });
    const byClass = <T extends HTMLElement = HTMLElement>(cls: string) => root.querySelector<T>(`.${cls}`)!;
    return { root, calls, panel, byClass };
  }

  test("Host link, then the join code input and Join link, and nothing else", () => {
    const { root, calls, byClass } = build();
    const labels = buttons(root).map((b) => b.textContent);
    // No copy-paste codes and no link list with Remove (Milestone 14).
    expect(labels).toEqual(["Host link", "Join link"]);
    expect(root.querySelector("textarea, ul")).toBeNull();
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
    links.closeAll();
    await settle(5);
    expect(ops).toContain(`close ${rooms[1]}`);
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
 * `otherTab`, another tab holds the badge first, until `freeOtherTab()`.
 * The controller starts the firmware as it is created.
 */
function badgeRig(
  opts: {
    otherTab?: boolean;
    /** Each Worker's boot outcome, in creation order (`"ready"` past the end), or one for all. */
    boot?: "ready" | "error" | ReadonlyArray<"ready" | "error">;
  } = {},
) {
  const { shell, canvas, fills } = pageShell();
  const locks = new FakeLockManager();
  let freeOtherTab = () => {};
  if (opts.otherTab) {
    void locks.request(BADGE_LOCK_NAME, {}, () => new Promise<void>((resolve) => (freeOtherTab = resolve)));
  }
  const workers: Array<ReturnType<typeof fakeWorker>> = [];
  const running: boolean[] = [];
  const controller = createBadgeController({
    shell,
    hub: {
      fromLocal() {},
      setPaused() {},
      setRunning: (r) => void running.push(r),
    },
    roles: ["hacker"],
    loadImages: async () => ({ wasm: new ArrayBuffer(8), image: new ArrayBuffer(8) }),
    createWorker: () => {
      const { boot } = opts;
      const fw = fakeWorker(typeof boot === "string" ? boot : (boot?.[workers.length] ?? "ready"));
      workers.push(fw);
      return fw.w;
    },
    mac: () => new Uint8Array([0x02, 0, 0, 0, 0x10, 0x01]),
    fetchIdentity: async () => "{}",
    locks,
  });
  const el = (id: string) => doc().getElementById(id)!;
  const noticeShown = () => el("badge-notice").style.display !== "none";
  /** The first Worker's `status`, with these fields over an idle default, then a refresh. */
  const status = (fields: Partial<Extract<FromWorker, { kind: "status" }>>) => {
    const data: FromWorker = {
      kind: "status",
      speed: 1,
      rebooting: false,
      provision: null,
      skip: null,
      setupOnScreen: false,
      ...fields,
    };
    workers[0]!.w.onmessage?.({ data });
    controller.refreshStatus();
  };
  const controls = () => el("provision-controls");
  const skipButton = () => button(controls(), "Skip setup");
  const skipShown = () => !skipButton().hidden;
  const statusText = () => controls().querySelector(".provision-status")!.textContent!;
  return {
    controller,
    shell,
    locks,
    workers,
    running,
    fills,
    el,
    freeOtherTab: () => freeOtherTab(),
    noticeShown,
    canvas,
    status,
    controls,
    skipButton,
    skipShown,
    statusText,
  };
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
  test("boots and starts the badge as it is created", async () => {
    const r = badgeRig();
    await settle();
    expect(r.workers).toHaveLength(1);
    expect(r.workers[0]!.sent.map((m) => m.kind)).toEqual(["init", "setMac", "start"]);
    expect(r.locks.held()).toBe(true);
    expect(r.running).toEqual([true]);
    expect(r.noticeShown()).toBe(false);
  });

  test("unload_releases_the_badge", async () => {
    const r = badgeRig();
    await settle();
    expect(r.locks.held()).toBe(true);
    // Unload frees it, and a second free does nothing.
    r.controller.dispose();
    r.controller.dispose();
    await settle();
    expect(r.locks.held()).toBe(false);
  });

  test("a_failed_firmware_start_releases_the_badge", async () => {
    let r!: ReturnType<typeof badgeRig>;
    const errors = await quietErrors(async () => {
      r = badgeRig({ boot: "error" });
      await settle();
    });
    expect(errors.length).toBeGreaterThan(0);
    expect(r.workers).toHaveLength(1);
    expect(r.locks.held()).toBe(false);
    expect(r.running).toEqual([]);
    // The page says so, not only the console.
    r.controller.refreshStatus();
    expect(r.statusText()).toContain(START_FAILED_NOTE);
  });

  test("provision after a failed first boot boots and starts the badge", async () => {
    let r!: ReturnType<typeof badgeRig>;
    await quietErrors(async () => {
      r = badgeRig({ boot: ["error", "ready"] });
      await settle();
    });
    expect(r.running).toEqual([]);
    button(r.controls(), "Provision test badge").click();
    await settle();
    expect(r.workers).toHaveLength(2);
    const kinds = r.workers[1]!.sent.map((m) => m.kind);
    expect(kinds).toContain("start");
    expect(kinds.indexOf("start")).toBeLessThan(kinds.indexOf("provision"));
    expect(r.running).toEqual([true]);
    expect(r.locks.held()).toBe(true);
    r.controller.refreshStatus();
    expect(r.statusText()).not.toContain(START_FAILED_NOTE);
  });

  test("provision on a running badge starts nothing again", async () => {
    const r = badgeRig();
    await settle();
    button(r.controls(), "Provision test badge").click();
    await settle();
    expect(r.workers).toHaveLength(1);
    expect(r.workers[0]!.sent.filter((m) => m.kind === "start")).toHaveLength(1);
    expect(r.workers[0]!.sent.filter((m) => m.kind === "provision")).toHaveLength(1);
    expect(r.running).toEqual([true]);
  });

  test("provision_without_the_badge_boots_no_worker_and_shows_the_notice", async () => {
    const r = badgeRig({ otherTab: true });
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
    // Another tab holds the badge while this one starts, then frees it.
    const r = badgeRig({ boot: "error", otherTab: true });
    await settle();
    expect(r.noticeShown()).toBe(true);
    expect(r.workers).toHaveLength(0);
    r.freeOtherTab();
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

  test("Skip setup shows only while Setup is on screen", async () => {
    const r = badgeRig();
    r.controller.refreshStatus();
    expect(r.skipShown()).toBe(false); // not booted yet, no runtime
    await settle();
    r.status({ setupOnScreen: true });
    expect(r.skipShown()).toBe(true);
    r.status({ setupOnScreen: false });
    expect(r.skipShown()).toBe(false);
    r.status({ setupOnScreen: true });
    expect(r.skipShown()).toBe(true);
  });

  test("Skip setup hides while a provision is in flight", async () => {
    const r = badgeRig();
    await settle();
    // The token phase runs after Setup appears: Setup is on screen while the provision types.
    r.status({ setupOnScreen: true, provision: { kind: "typing" } });
    expect(r.skipShown()).toBe(false);
    r.status({ setupOnScreen: true, provision: { kind: "waiting" } });
    expect(r.skipShown()).toBe(false);
    r.status({ setupOnScreen: true, provision: { kind: "ok", line: "PROV OK" } });
    expect(r.skipShown()).toBe(true);
    r.status({ setupOnScreen: true, provision: { kind: "failed", reason: "timed out" } });
    expect(r.skipShown()).toBe(true);
  });

  test("Skip setup hides while a skip runs, and a click calls skipSetup once", async () => {
    const r = badgeRig();
    await settle();
    r.status({ setupOnScreen: true });
    expect(r.skipShown()).toBe(true);
    const provisionBtn = button(r.controls(), "Provision test badge");
    expect(provisionBtn.disabled).toBe(false);
    r.skipButton().click();
    r.skipButton().click(); // a double click
    const skips = () => r.workers[0]!.sent.filter((m) => m.kind === "skipSetup").length;
    expect(skips()).toBe(1);
    r.controller.refreshStatus();
    expect(r.skipShown()).toBe(false);
    expect(r.statusText()).toContain("skipping setup…");
    expect(provisionBtn.disabled).toBe(true);
    // The Worker still reports Setup on screen while the skip types: still hidden.
    r.status({ setupOnScreen: true, skip: { kind: "typing" } });
    expect(r.skipShown()).toBe(false);
    expect(provisionBtn.disabled).toBe(true);
    r.status({ setupOnScreen: false, skip: { kind: "done" } });
    expect(r.skipShown()).toBe(false);
    expect(r.statusText()).not.toContain("skip");
    expect(provisionBtn.disabled).toBe(false);
    expect(skips()).toBe(1);
    // A failed skip says why, and Setup still on screen offers it again.
    r.status({ setupOnScreen: true, skip: { kind: "failed", reason: "timed out" } });
    expect(r.statusText()).toContain("skip failed: timed out");
    expect(r.skipShown()).toBe(true);
    expect(provisionBtn.disabled).toBe(false);
  });

  test("a provision started after a failed skip shows its own status", async () => {
    const r = badgeRig();
    await settle();
    r.status({ setupOnScreen: true });
    r.skipButton().click();
    const failed = { kind: "failed", reason: "timed out" } as const;
    r.status({ setupOnScreen: true, skip: failed });
    expect(r.statusText()).toContain("skip failed: timed out");
    // The user provisions again; the Worker still reports the old skip failure.
    button(r.controls(), "Provision test badge").click();
    await settle();
    r.status({ setupOnScreen: true, provision: { kind: "typing" }, skip: failed });
    expect(r.statusText()).toContain("provisioning…");
    expect(r.statusText()).not.toContain("skip failed");
    // It finishes: its own result shows, not the old skip failure.
    r.status({ setupOnScreen: true, provision: { kind: "ok", line: "PROV OK id=x" }, skip: failed });
    expect(r.statusText()).toContain("PROV OK id=x");
    expect(r.statusText()).not.toContain("skip failed");
    r.status({ setupOnScreen: true, provision: { kind: "failed", reason: "no prompt" }, skip: failed });
    expect(r.statusText()).toContain("failed: no prompt");
    expect(r.statusText()).not.toContain("skip failed");
    // A skip started after that provision wins again.
    r.skipButton().click();
    r.status({ setupOnScreen: true, provision: { kind: "ok", line: "PROV OK id=x" }, skip: { kind: "typing" } });
    expect(r.statusText()).toContain("skipping setup…");
  });

  test("Skip setup hides on a badge lost to another tab, and a click sends nothing", async () => {
    const r = badgeRig();
    await settle();
    r.status({ setupOnScreen: true });
    expect(r.skipShown()).toBe(true);
    // Another tab takes the badge over: this one stops and posts no more statuses.
    void r.locks.request(BADGE_LOCK_NAME, { steal: true }, () => new Promise(() => {}));
    await settle();
    expect(r.noticeShown()).toBe(true);
    expect(r.running).toEqual([true, false]);
    expect(r.skipShown()).toBe(false);
    r.controller.refreshStatus();
    expect(r.skipShown()).toBe(false);
    r.skipButton().click(); // a click on the hidden button all the same
    r.controller.refreshStatus();
    expect(r.workers[0]!.sent.filter((m) => m.kind === "skipSetup")).toHaveLength(0);
    expect(r.statusText()).not.toContain("skipping setup");
    expect(button(r.controls(), "Provision test badge").disabled).toBe(false);
  });

  test("a refused Skip setup click keeps the newer provision's status", async () => {
    const r = badgeRig();
    await settle();
    r.status({ setupOnScreen: true });
    r.skipButton().click();
    const failed = { kind: "failed", reason: "timed out" } as const;
    r.status({ setupOnScreen: true, skip: failed });
    button(r.controls(), "Provision test badge").click();
    await settle();
    r.status({ setupOnScreen: false, provision: { kind: "ok", line: "PROV OK id=x" }, skip: failed });
    expect(r.statusText()).toContain("PROV OK id=x");
    // Setup is off screen: the client refuses the click, and the old skip failure stays hidden.
    r.skipButton().click();
    r.controller.refreshStatus();
    expect(r.statusText()).toContain("PROV OK id=x");
    expect(r.statusText()).not.toContain("skip failed");
    expect(r.workers[0]!.sent.filter((m) => m.kind === "skipSetup")).toHaveLength(1);
  });

  test("the speed reads rebooting… during the reboot's pause", async () => {
    const r = badgeRig();
    await settle();
    r.status({ speed: 1, rebooting: true });
    expect(r.statusText()).toContain("rebooting…");
    expect(r.statusText()).not.toContain("1.0x");
    r.status({ speed: 1, rebooting: false });
    expect(r.statusText()).toContain("1.0x");
    expect(r.statusText()).not.toContain("rebooting");
  });

  test("notice_clears_the_canvas", async () => {
    const r = badgeRig({ otherTab: true });
    await settle();
    expect(r.noticeShown()).toBe(true);
    // The whole canvas, in the page's background colour (shell.css's --bg).
    expect(r.fills.at(-1)).toEqual({ style: PAGE_BG, x: 0, y: 0, w: r.canvas.width, h: r.canvas.height });
  });
});

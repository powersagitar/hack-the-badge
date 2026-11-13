import { describe, expect, test } from "bun:test";
import { BADGE_LOCK_NAME } from "../src/radio/lock";
import type { Mg, MotionSource } from "../src/motion/source";
import type { FromWorker, ToWorker } from "../src/runtime/firmware-worker-protocol";
import {
  type BadgeController,
  type BadgeWorker,
  HANGING_REST_MG,
  START_FAILED_NOTE,
  createBadgeController,
} from "../src/ui/badge-controller";
import { FakeLockManager } from "./fake-locks";

/**
 * The controller store without a DOM: fake Workers, a fake lock manager, a
 * hand-driven visibility and animation-frame queue, a recording hub and
 * fake phone motion. Ported from the old `test/shell.test.ts`'s badge
 * controller cases (Milestone 14 Task 12), with the page's status line
 * now the console's `[page]` lines and the snapshot.
 */

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

function fakeVisibility() {
  let hidden = false;
  const cbs = new Set<() => void>();
  return {
    source: {
      hidden: () => hidden,
      subscribe(cb: () => void) {
        cbs.add(cb);
        return () => void cbs.delete(cb);
      },
    },
    set(h: boolean) {
      hidden = h;
      for (const cb of [...cbs]) cb();
    },
  };
}

function fakePhone() {
  let sink: ((mg: Mg) => void) | null = null;
  const source: MotionSource = {
    start: (s) => void (sink = s),
    stop: () => void (sink = null),
    peerBump() {},
    onImpact: () => () => {},
  };
  return { source, emit: (mg: Mg) => sink?.(mg), started: () => sink !== null };
}

function rig(
  opts: {
    otherTab?: boolean;
    boot?: "ready" | "error" | ReadonlyArray<"ready" | "error">;
    hidden?: boolean;
    permission?: "granted" | "denied" | "unsupported";
    motionAvailable?: boolean;
    identity?: "ok" | "fail";
  } = {},
) {
  const locks = new FakeLockManager();
  let freeOtherTab = () => {};
  if (opts.otherTab) {
    // Run here steals it: the other tab's request then rejects (AbortError).
    locks
      .request(BADGE_LOCK_NAME, {}, () => new Promise<void>((resolve) => (freeOtherTab = resolve)))
      .catch(() => {});
  }
  const workers: Array<ReturnType<typeof fakeWorker>> = [];
  const running: boolean[] = [];
  const visibility = fakeVisibility();
  if (opts.hidden) visibility.set(true);
  const frames: Array<() => void> = [];
  const phone = fakePhone();
  const controller: BadgeController = createBadgeController({
    hub: {
      fromLocal() {},
      setPaused() {},
      setRunning: (r) => void running.push(r),
      addLink() {},
      removeLink() {},
    },
    loadImages: async () => ({ wasm: new ArrayBuffer(8), image: new ArrayBuffer(8) }),
    createWorker: () => {
      const { boot } = opts;
      const fw = fakeWorker(typeof boot === "string" ? boot : (boot?.[workers.length] ?? "ready"));
      workers.push(fw);
      return fw.w;
    },
    mac: () => new Uint8Array([0x02, 0, 0, 0, 0x10, 0x01]),
    fetchIdentity: async () => {
      if (opts.identity === "fail") throw new Error("HTTP 404");
      return "{}";
    },
    locks,
    pageEvents: new EventTarget(),
    clientOptions: {
      visibility: visibility.source,
      scheduleFrame: (cb) => frames.push(() => cb(0)),
      cancelFrame: () => {},
    },
    motion: {
      available: () => opts.motionAvailable ?? true,
      request: async () => opts.permission ?? "granted",
      create: () => phone.source,
    },
  });
  /** Runs the queued animation frames. */
  const flushFrames = () => {
    for (const f of frames.splice(0)) f();
  };
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
  const pageLines = () => controller.console.entries().filter((e) => e.source === "page").map((e) => e.text);
  const sent = (i = 0) => workers[i]!.sent;
  const snap = () => controller.getSnapshot();
  return {
    controller,
    locks,
    workers,
    running,
    visibility,
    flushFrames,
    status,
    pageLines,
    sent,
    snap,
    phone,
    freeOtherTab: () => freeOtherTab(),
  };
}

const settle = (ms = 0) => new Promise<void>((r) => setTimeout(r, ms));

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
    const r = rig();
    await settle();
    expect(r.workers).toHaveLength(1);
    expect(r.sent().map((m) => m.kind)).toEqual(["init", "setMac", "start"]);
    expect(r.locks.held()).toBe(true);
    expect(r.running).toEqual([true]);
    expect(r.snap().tabNotice).toBe(false);
  });

  test("hidden page sends the rest reading", async () => {
    const r = rig();
    await settle();
    const sink = r.controller.motionSink();
    expect(r.controller.motionSink()).toBe(sink); // stable for the stage's effect
    sink([300, -1500, 0]);
    r.flushFrames();
    const accels = () => r.sent().filter((m) => m.kind === "accel").map((m) => (m as { mg: Mg }).mg);
    expect(accels()).toEqual([[300, -1500, 0]]);

    r.visibility.set(true);
    // Samples while hidden are dropped; the rest reading is held for the
    // Worker (no animation frame runs on a hidden page).
    sink([1999, 1999, 0]);
    expect(HANGING_REST_MG).toEqual([0, 1000, 0]);

    const before = r.sent().length;
    r.visibility.set(false);
    // The rest reading lands before the badge runs again, never the hidden sample.
    const after = r.sent().slice(before).map((m) => m.kind);
    expect(after.slice(0, 2)).toEqual(["accel", "start"]);
    expect(accels()).toEqual([[300, -1500, 0], HANGING_REST_MG]);

    // Visible again: samples flow.
    sink([5, 6, 7]);
    r.flushFrames();
    expect(accels().at(-1)).toEqual([5, 6, 7]);
  });

  test("a badge booted on a hidden page starts from the rest reading", async () => {
    const r = rig({ hidden: true });
    await settle();
    r.controller.motionSink()([1500, 0, 0]);
    r.visibility.set(false);
    const kinds = r.sent().map((m) => m.kind);
    expect(kinds.slice(-2)).toEqual(["accel", "start"]);
    expect(r.sent().find((m) => m.kind === "accel")).toEqual({ kind: "accel", mg: HANGING_REST_MG });
  });

  test("provision and skip states become page console lines", async () => {
    const r = rig();
    await settle();
    await r.controller.provision("hacker");
    expect(r.sent().filter((m) => m.kind === "provision")).toHaveLength(1);
    expect(r.pageLines()).toEqual(["waiting for the console…"]);
    r.status({ provision: { kind: "typing" } });
    r.status({ provision: { kind: "typing" } }); // unchanged: no new line
    r.status({ provision: { kind: "ok", line: "PROV OK id=x" }, setupOnScreen: true });
    expect(r.snap().provision).toEqual({ kind: "ok", line: "PROV OK id=x" });
    expect(r.snap().setupOnScreen).toBe(true);

    r.controller.skipSetup();
    r.controller.skipSetup(); // a double click
    expect(r.sent().filter((m) => m.kind === "skipSetup")).toHaveLength(1);
    r.status({ provision: { kind: "ok", line: "PROV OK id=x" }, skip: { kind: "typing" }, setupOnScreen: true });
    r.status({ rebooting: true, provision: { kind: "ok", line: "PROV OK id=x" }, skip: { kind: "typing" } });
    expect(r.snap().rebooting).toBe(true);
    r.status({ provision: { kind: "ok", line: "PROV OK id=x" }, skip: { kind: "done" } });
    r.status({ provision: { kind: "ok", line: "PROV OK id=x" }, skip: { kind: "failed", reason: "timed out" } });
    expect(r.pageLines()).toEqual([
      "waiting for the console…",
      "provisioning…",
      "PROV OK id=x",
      "skipping setup…",
      "rebooting…",
      // `done` adds nothing (the screen shows it).
      "skip failed: timed out",
    ]);
  });

  test("a provision started after a failed skip shows its own lines", async () => {
    const r = rig();
    await settle();
    r.status({ setupOnScreen: true });
    r.controller.skipSetup();
    const failed = { kind: "failed", reason: "timed out" } as const;
    r.status({ setupOnScreen: true, skip: failed });
    await r.controller.provision("hacker");
    r.status({ setupOnScreen: true, provision: { kind: "typing" }, skip: failed });
    r.status({ setupOnScreen: true, provision: { kind: "failed", reason: "no prompt" }, skip: failed });
    expect(r.pageLines()).toEqual([
      "skipping setup…",
      "skip failed: timed out",
      "waiting for the console…",
      "provisioning…",
      "failed: no prompt",
    ]);
    // A refused skip click (Setup off screen) keeps the provision's line.
    r.status({ setupOnScreen: false, provision: { kind: "failed", reason: "no prompt" }, skip: failed });
    r.controller.skipSetup();
    r.controller.refreshStatus();
    expect(r.pageLines().at(-1)).toBe("failed: no prompt");
    expect(r.sent().filter((m) => m.kind === "skipSetup")).toHaveLength(1);
  });

  test("power is passed to the running badge, and a provision in flight fails on its reboot", async () => {
    const r = rig();
    await settle();
    await r.controller.provision("hacker");
    r.controller.powerCycle();
    r.controller.client.powerCycle(); // the board's switch, pressed again
    // The Worker's runtime ignores the second press while it reboots (Task 7's test).
    expect(r.sent().filter((m) => m.kind === "powerCycle")).toHaveLength(2);
    r.status({ rebooting: true, provision: { kind: "failed", reason: "the badge rebooted during provisioning" } });
    expect(r.controller.client.rebooting()).toBe(true);
    expect(r.pageLines().slice(-2)).toEqual(["rebooting…", "failed: the badge rebooted during provisioning"]);
  });

  test("the snapshot is stable between changes and notifies on change", async () => {
    const r = rig();
    await settle();
    let calls = 0;
    const off = r.controller.subscribe(() => calls++);
    r.status({ speed: 1 });
    const a = r.snap();
    r.controller.refreshStatus();
    expect(r.snap()).toBe(a);
    const before = calls;
    r.status({ speed: 0.5 });
    expect(r.snap()).not.toBe(a);
    expect(r.snap().speed).toBe(0.5);
    expect(calls).toBe(before + 1);
    off();
  });

  test("serial output and LED frames reach the console and the board", async () => {
    const r = rig();
    await settle();
    const leds: Uint8Array[] = [];
    r.controller.client.onLeds((rgb) => leds.push(rgb));
    r.workers[0]!.w.onmessage?.({ data: { kind: "console", text: "boot ok\nI (1) ", at: 0, boot: 0 } });
    r.workers[0]!.w.onmessage?.({ data: { kind: "leds", rgb: new Uint8Array([1, 2, 3]) } });
    expect(r.controller.console.entries().map((e) => [e.source, e.text])).toEqual([["serial", "boot ok"]]);
    expect(leds).toEqual([new Uint8Array([1, 2, 3])]);
  });

  test("the board's buttons reach the Worker with their source", async () => {
    const r = rig();
    await settle();
    r.controller.client.injectButton("A", "pressed", "key");
    r.controller.client.injectButton("A", "released", "pad");
    expect(r.sent().filter((m) => m.kind === "button")).toEqual([
      { kind: "button", button: "A", pressed: true, source: "key" },
      { kind: "button", button: "A", pressed: false, source: "pad" },
    ]);
  });

  test("unload releases the badge", async () => {
    const r = rig();
    await settle();
    expect(r.locks.held()).toBe(true);
    r.controller.dispose();
    r.controller.dispose();
    await settle();
    expect(r.locks.held()).toBe(false);
  });

  test("a failed firmware start releases the badge and says so", async () => {
    let r!: ReturnType<typeof rig>;
    const errors = await quietErrors(async () => {
      r = rig({ boot: "error" });
      await settle();
    });
    expect(errors.length).toBeGreaterThan(0);
    expect(r.locks.held()).toBe(false);
    expect(r.running).toEqual([]);
    expect(r.pageLines()).toEqual([START_FAILED_NOTE]);
  });

  test("provision after a failed first boot boots and starts the badge", async () => {
    let r!: ReturnType<typeof rig>;
    await quietErrors(async () => {
      r = rig({ boot: ["error", "ready"] });
      await settle();
    });
    await r.controller.provision("hacker");
    expect(r.workers).toHaveLength(2);
    const kinds = r.sent(1).map((m) => m.kind);
    expect(kinds.indexOf("start")).toBeLessThan(kinds.indexOf("provision"));
    expect(r.running).toEqual([true]);
    expect(r.locks.held()).toBe(true);
  });

  test("provision on a running badge starts nothing again", async () => {
    const r = rig();
    await settle();
    await r.controller.provision("hacker");
    expect(r.workers).toHaveLength(1);
    expect(r.sent().filter((m) => m.kind === "start")).toHaveLength(1);
    expect(r.sent().filter((m) => m.kind === "provision")).toHaveLength(1);
  });

  test("provision without the badge boots no Worker and shows the notice", async () => {
    const r = rig({ otherTab: true });
    await settle();
    expect(r.snap().tabNotice).toBe(true);
    await r.controller.provision("hacker");
    expect(r.workers).toHaveLength(0);
    expect(r.snap().tabNotice).toBe(true);
    expect(r.locks.held()).toBe(true); // still the other tab's
  });

  test("a failed boot from provision says it is a boot failure and releases the badge", async () => {
    const r = rig({ boot: "error", otherTab: true });
    await settle();
    r.freeOtherTab();
    await settle();
    const errors = await quietErrors(() => r.controller.provision("hacker"));
    expect(r.workers).toHaveLength(1);
    expect(errors.length).toBeGreaterThan(0);
    expect(r.locks.held()).toBe(false);
    expect(r.pageLines()).toEqual([START_FAILED_NOTE]);
    expect(r.pageLines().join()).not.toContain("could not load");
  });

  test("an identity that cannot be fetched says so, and keeps the badge", async () => {
    const r = rig({ identity: "fail" });
    await settle();
    const errors = await quietErrors(() => r.controller.provision("workshop_lead"));
    expect(errors.length).toBeGreaterThan(0);
    expect(r.sent().filter((m) => m.kind === "provision")).toHaveLength(0);
    expect(r.pageLines()).toEqual(["could not load workshop_lead"]);
    expect(r.locks.held()).toBe(true);
    expect(r.running).toEqual([true]);
  });

  test("Run here takes the badge from the other tab and starts it", async () => {
    const r = rig({ otherTab: true });
    await settle();
    expect(r.snap().tabNotice).toBe(true);
    await r.controller.runHere();
    await settle();
    expect(r.snap().tabNotice).toBe(false);
    expect(r.running).toEqual([true]);
  });

  test("a badge lost to another tab stops, hides Skip setup and shows the notice", async () => {
    const r = rig();
    await settle();
    r.status({ setupOnScreen: true });
    expect(r.snap().setupOnScreen).toBe(true);
    void r.locks.request(BADGE_LOCK_NAME, { steal: true }, () => new Promise(() => {}));
    await settle();
    expect(r.snap().tabNotice).toBe(true);
    expect(r.running).toEqual([true, false]);
    expect(r.snap().setupOnScreen).toBe(false);
    r.controller.skipSetup();
    expect(r.sent().filter((m) => m.kind === "skipSetup")).toHaveLength(0);
    // Power does nothing on a stopped badge.
    r.controller.powerCycle();
    expect(r.sent().filter((m) => m.kind === "powerCycle")).toHaveLength(0);
  });

  test("phone motion, granted, drives the sensor in place of the swing", async () => {
    const r = rig();
    await settle();
    expect(r.snap().phoneMotionAvailable).toBe(true);
    expect(r.snap().motion).toBe("physics");
    await r.controller.usePhoneMotion(true);
    expect(r.snap().motion).toBe("phone");
    expect(r.phone.started()).toBe(true);
    r.phone.emit([10, 20, 30]);
    r.flushFrames();
    expect(r.sent().filter((m) => m.kind === "accel").at(-1)).toEqual({ kind: "accel", mg: [10, 20, 30] });
    await r.controller.usePhoneMotion(false);
    expect(r.snap().motion).toBe("physics");
    expect(r.phone.started()).toBe(false);
  });

  test("denied phone motion keeps physics and explains", async () => {
    const r = rig({ permission: "denied" });
    await settle();
    await r.controller.usePhoneMotion(true);
    expect(r.snap().motion).toBe("physics");
    expect(r.phone.started()).toBe(false);
    expect(r.pageLines().at(-1)).toMatch(/phone motion was denied/);
  });

  test("phone motion unavailable is reported in the snapshot", async () => {
    const r = rig({ motionAvailable: false, permission: "unsupported" });
    await settle();
    expect(r.snap().phoneMotionAvailable).toBe(false);
    await r.controller.usePhoneMotion(true);
    expect(r.snap().motion).toBe("physics");
    expect(r.pageLines().at(-1)).toMatch(/phone motion is not available/);
  });
});

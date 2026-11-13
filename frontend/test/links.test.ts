import { describe, expect, test } from "bun:test";
import { createLinks } from "../src/radio/links";
import type { RadioLink } from "../src/radio/hub";
import { MemoryStore, handleSignal, memoryDeps } from "../src/radio/signal-handler";
import type { SignalDeps } from "../src/radio/signal-client";
import { FakeNetwork } from "./fake-rtc";

/**
 * Two browsers' links over the real signaling handler (a `MemoryStore`)
 * and a fake WebRTC network, with a fake clock for the host's polls.
 * `ops` records each request's `op` (and `close <room>`).
 */
function network() {
  const signal = memoryDeps(undefined, new MemoryStore());
  const net = new FakeNetwork();
  let now = 0;
  const ops: string[] = [];
  const deps: SignalDeps = {
    factory: net.factory,
    fetch: async (input, init) => {
      const body = typeof init?.body === "string" ? (JSON.parse(init.body) as { op?: string; room?: string }) : {};
      if (body.op) ops.push(body.op === "close" ? `close ${body.room}` : body.op);
      return handleSignal(new Request(new URL(String(input), "http://localhost"), init), signal);
    },
    // As test/signal-client.test.ts: a long wait passes in steps of at most
    // 1 s, so a link that opens meanwhile wins the race.
    sleep: async (ms, abort) => {
      let left = ms;
      do {
        if (abort?.aborted) return new Promise<void>(() => {});
        const step = Math.min(left, 1000);
        now += step;
        left -= step;
        await new Promise((r) => setTimeout(r, 2));
      } while (left > 0);
    },
    now: () => now,
  };
  return { net, deps, ops };
}

/** A hub that records its links. */
function fakeHub() {
  const links = new Map<string, RadioLink>();
  return {
    links,
    addLink: (l: RadioLink) => void links.set(l.id, l),
    removeLink: (id: string) => void links.delete(id),
  };
}

function side(deps: SignalDeps, pageEvents: EventTarget = new EventTarget()) {
  const hub = fakeHub();
  const notes: string[] = [];
  let changes = 0;
  const links = createLinks(hub, deps, { note: (t) => notes.push(t), pageEvents });
  links.subscribe(() => changes++);
  return { hub, notes, links, pageEvents, changes: () => changes };
}

const settle = (ms = 30) => new Promise((r) => setTimeout(r, ms));

describe("links", () => {
  test("summary counts open links", async () => {
    const { deps } = network();
    const a = side(deps);
    const b = side(deps);
    const c = side(deps);
    expect(a.links.summary()).toBe("");

    const room = await a.links.host();
    expect(room).toMatch(/^[0-9A-Z]+$/);
    expect(a.links.summary()).toBe(`Hosting ${room}`);
    expect(a.links.hosting()).toBe(room);
    expect(a.changes()).toBeGreaterThan(0);

    await b.links.join(room.toLowerCase());
    await settle();
    expect(a.links.summary()).toBe("Linked to 1 badge");
    expect(a.links.hosting()).toBeNull();
    expect(b.links.summary()).toBe("Linked to 1 badge");

    // A second room: a has two links.
    const room2 = await a.links.host();
    await c.links.join(room2);
    await settle();
    expect(a.links.summary()).toBe("Linked to 2 badges");
    expect(a.hub.links.size).toBe(2);
  });

  test("closed link drops out with a console reason", async () => {
    const { deps } = network();
    const a = side(deps);
    const b = side(deps);
    const room = await a.links.host();
    await b.links.join(room);
    await settle();
    expect(a.links.summary()).toBe("Linked to 1 badge");

    // The peer leaves: its end closes, and ours follows.
    for (const l of b.hub.links.values()) (l as unknown as { close(): void }).close();
    await settle();
    expect(a.links.summary()).toBe("");
    expect(a.hub.links.size).toBe(0);
    expect(a.notes.at(-1)).toBe("A badge link closed.");
    expect(b.notes.at(-1)).toBe("A badge link closed.");
  });

  test("a failed join says why, and the summary shows it", async () => {
    const { deps } = network();
    const b = side(deps);
    await b.links.join("zzzz-zzzz");
    expect(b.links.summary()).toBe("Room closed or expired.");
    expect(b.notes.at(-1)).toBe("Room closed or expired.");
    expect(b.hub.links.size).toBe(0);
  });

  test("cancelling a hosted room closes it on the server", async () => {
    const { deps, ops } = network();
    const a = side(deps);
    const room = await a.links.host();
    a.links.cancelHost();
    await settle();
    expect(ops).toContain(`close ${room}`);
    expect(a.links.hosting()).toBeNull();
    expect(a.links.summary()).toBe("");
    expect(a.hub.links.size).toBe(0);
    expect(a.notes.at(-1)).toBe(`Stopped hosting room ${room}.`);
  });

  test("hosting twice before the first room is made closes the first room", async () => {
    const { deps, ops } = network();
    const a = side(deps);
    const first = a.links.host();
    const second = a.links.host();
    expect(await first).toBe("");
    const room = await second;
    await settle();
    const created = ops.filter((o) => o === "create").length;
    expect(created).toBe(2);
    expect(ops.filter((o) => o.startsWith("close "))).toHaveLength(1);
    expect(ops).not.toContain(`close ${room}`);
    expect(a.links.summary()).toBe(`Hosting ${room}`);
  });

  test("every link closes on pagehide", async () => {
    const { deps, ops } = network();
    const a = side(deps);
    const b = side(deps);
    const c = side(deps);
    const room = await a.links.host();
    await b.links.join(room);
    await settle();
    const waiting = await c.links.host();
    expect(c.links.summary()).toBe(`Hosting ${waiting}`);

    a.pageEvents.dispatchEvent(new Event("pagehide"));
    c.pageEvents.dispatchEvent(new Event("pagehide"));
    await settle();
    expect(a.hub.links.size).toBe(0);
    expect(a.links.summary()).toBe("");
    expect(b.links.summary()).toBe(""); // the peer saw it close
    expect(ops).toContain(`close ${waiting}`);
    expect(c.hub.links.size).toBe(0);
  });

  test("dispose closes the links and stops listening for pagehide", async () => {
    const { deps, ops } = network();
    const pageEvents = new EventTarget();
    let added = 0;
    let removed = 0;
    const events = {
      addEventListener: (t: string, cb: EventListenerOrEventListenerObject) => {
        added++;
        pageEvents.addEventListener(t, cb);
      },
      removeEventListener: (t: string, cb: EventListenerOrEventListenerObject) => {
        removed++;
        pageEvents.removeEventListener(t, cb);
      },
    };
    const a = createLinks(fakeHub(), deps, { pageEvents: events });
    const room = await a.host();
    a.dispose();
    await settle();
    expect([added, removed]).toEqual([1, 1]);
    expect(ops).toContain(`close ${room}`);
    expect(a.hosting()).toBeNull();
  });
});

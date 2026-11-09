import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  type HubMessage,
  LINKED_PEER_RSSI,
  MAX_ADVERTISEMENT_BYTES,
  RadioHub,
  type RadioLink,
  decodeBase64,
  encodeBase64,
} from "../src/radio/hub";

/** A link whose sends are recorded and whose incoming messages the test injects. */
function fakeLink(id: string, opts: { throwOnSend?: boolean } = {}) {
  const sent: HubMessage[] = [];
  let cb: ((msg: unknown) => void) | null = null;
  const link: RadioLink = {
    id,
    send(msg) {
      if (opts.throwOnSend) throw new Error("channel closed");
      sent.push(msg);
    },
    onMessage(f) {
      cb = f;
    },
  };
  return { link, sent, receive: (msg: unknown) => cb?.(msg) };
}

function hubRig() {
  const delivered: Array<{ bytes: number[]; rssi: number }> = [];
  let localBumps = 0;
  const hub = new RadioHub({
    deliver: (bytes, rssi) => delivered.push({ bytes: [...bytes], rssi }),
    bumpLocal: () => localBumps++,
  });
  return { hub, delivered, localBumps: () => localBumps };
}

/** Runs `f` with `console.error` silenced: the hub logs the failures these tests inject. */
function quietly(f: () => void): void {
  const original = console.error;
  console.error = () => {};
  try {
    f();
  } finally {
    console.error = original;
  }
}

/** A well-formed advertisement: addr(6) ‖ addr_type(1) ‖ data. */
const ADV = new Uint8Array([0x01, 0x10, 0x00, 0x00, 0x00, 0xc2, 0x01, 0x02, 0x01, 0x06]);
const ADV2 = new Uint8Array([0x02, 0x10, 0x00, 0x00, 0x00, 0xc2, 0x01, 0x03, 0x09, 0x42, 0x4d]);

describe("LINKED_PEER_RSSI", () => {
  test("matches Task 8's ruling and the Rust constant", () => {
    const root = join(import.meta.dir, "..", "..");
    const doc = readFileSync(join(root, "docs", "milestone-10-decisions.md"), "utf8");
    const ruled = /`LINKED_PEER_RSSI` is `(-?\d+)` dBm/.exec(doc);
    expect(ruled).not.toBeNull();
    expect(LINKED_PEER_RSSI).toBe(Number(ruled![1]));
    const rust = readFileSync(join(root, "emulator-core", "src", "radio", "mod.rs"), "utf8");
    const declared = /pub const LINKED_PEER_RSSI: i8 = (-?\d+);/.exec(rust);
    expect(declared).not.toBeNull();
    expect(LINKED_PEER_RSSI).toBe(Number(declared![1]));
  });

  test("the size bound is the core decoder's: 7 header bytes and 229 data bytes", () => {
    expect(MAX_ADVERTISEMENT_BYTES).toBe(7 + 229);
  });
});

describe("base64", () => {
  test("round-trips bytes", () => {
    const all = new Uint8Array(256).map((_, i) => i);
    expect([...decodeBase64(encodeBase64(all))!]).toEqual([...all]);
    expect([...decodeBase64(encodeBase64(new Uint8Array(0)))!]).toEqual([]);
  });

  test("rejects malformed input without throwing", () => {
    for (const bad of ["@@@@", "abc", "a===", "QQ=A", " QQ==", "QQ==\n"]) {
      expect(decodeBase64(bad)).toBeNull();
    }
  });
});

describe("RadioHub", () => {
  test("hub_forwards_local_adverts_to_every_link", () => {
    const { hub } = hubRig();
    const a = fakeLink("a");
    const b = fakeLink("b");
    hub.addLink(a.link);
    hub.addLink(b.link);
    hub.fromLocal(ADV);
    const expected: HubMessage = { t: "adv", b: encodeBase64(ADV) };
    expect(a.sent).toEqual([expected]);
    expect(b.sent).toEqual([expected]);
    // An empty advertisement (not advertising) sends nothing.
    hub.fromLocal(new Uint8Array(0));
    expect(a.sent).toHaveLength(1);
  });

  test("a link that throws on send does not stop the others", () => {
    const { hub } = hubRig();
    const bad = fakeLink("bad", { throwOnSend: true });
    const good = fakeLink("good");
    hub.addLink(bad.link);
    hub.addLink(good.link);
    hub.setRunning(true);
    quietly(() => {
      expect(() => hub.fromLocal(ADV)).not.toThrow();
      expect(() => hub.bump()).not.toThrow();
    });
    expect(good.sent.map((m) => m.t)).toEqual(["adv", "bump"]);
  });

  test("a peer's advert is delivered at LINKED_PEER_RSSI", () => {
    const { hub, delivered } = hubRig();
    const a = fakeLink("a");
    hub.addLink(a.link);
    a.receive({ t: "adv", b: encodeBase64(ADV) });
    expect(delivered).toEqual([{ bytes: [...ADV], rssi: LINKED_PEER_RSSI }]);
  });

  test("hub_does_not_forward_between_links", () => {
    const { hub, delivered } = hubRig();
    const a = fakeLink("a");
    const b = fakeLink("b");
    hub.addLink(a.link);
    hub.addLink(b.link);
    a.receive({ t: "adv", b: encodeBase64(ADV) });
    a.receive({ t: "bump" });
    // Radio does not hop: b hears nothing from a, through us.
    expect(b.sent).toEqual([]);
    expect(a.sent).toEqual([]);
    expect(delivered).toHaveLength(1);
  });

  test("hub_drops_malformed_peer_messages", () => {
    const { hub, delivered, localBumps } = hubRig();
    const a = fakeLink("a");
    hub.addLink(a.link);
    const tooLong = new Uint8Array(MAX_ADVERTISEMENT_BYTES + 1);
    const tooShort = new Uint8Array(6);
    const bad: unknown[] = [
      null,
      undefined,
      42,
      "adv",
      [],
      ["adv", "AAAA"],
      {},
      { t: "nope" },
      { t: "ADV", b: encodeBase64(ADV) },
      { t: "adv" },
      { t: "adv", b: 7 },
      { t: "adv", b: "!!not base64!!" },
      { t: "adv", b: "QQ=" },
      { t: "adv", b: encodeBase64(tooLong) },
      { t: "adv", b: encodeBase64(tooShort) },
      { t: "adv", b: "A".repeat(1_000_000) },
      { t: "toString" },
      Object.create(null),
      { get t() { throw new Error("hostile getter"); } },
    ];
    for (const m of bad) expect(() => a.receive(m)).not.toThrow();
    expect(delivered).toEqual([]);
    expect(localBumps()).toBe(0);
    // A good message still gets through afterwards.
    a.receive({ t: "adv", b: encodeBase64(ADV) });
    expect(delivered).toHaveLength(1);
  });

  test("bump_hits_local_and_every_link", () => {
    const { hub, localBumps } = hubRig();
    const a = fakeLink("a");
    const b = fakeLink("b");
    hub.addLink(a.link);
    hub.addLink(b.link);
    hub.setRunning(true);
    expect(hub.bump()).toBe(true);
    expect(localBumps()).toBe(1);
    expect(a.sent).toEqual([{ t: "bump" }]);
    expect(b.sent).toEqual([{ t: "bump" }]);
  });

  test("bump_while_stopped_sends_nothing", () => {
    const { hub, localBumps } = hubRig();
    const a = fakeLink("a");
    hub.addLink(a.link);
    // A new hub's badge is not running yet.
    expect(hub.bump()).toBe(false);
    hub.setRunning(true);
    hub.setRunning(false);
    expect(hub.bump()).toBe(false);
    expect(a.sent).toEqual([]);
    expect(localBumps()).toBe(0);
    hub.setRunning(true);
    expect(hub.bump()).toBe(true);
    expect(a.sent).toEqual([{ t: "bump" }]);
    expect(localBumps()).toBe(1);
  });

  test("bump_while_hidden_sends_nothing", () => {
    const { hub, localBumps } = hubRig();
    const a = fakeLink("a");
    hub.addLink(a.link);
    hub.setRunning(true);
    hub.setPaused(true);
    expect(hub.bump()).toBe(false);
    expect(a.sent).toEqual([]);
    expect(localBumps()).toBe(0);
    // Not replayed when the page is visible again.
    hub.setPaused(false);
    expect(a.sent).toEqual([]);
    expect(hub.bump()).toBe(true);
    expect(a.sent).toEqual([{ t: "bump" }]);
    expect(localBumps()).toBe(1);
  });

  test("a peer's bump bumps only the local badge", () => {
    const { hub, localBumps } = hubRig();
    const a = fakeLink("a");
    const b = fakeLink("b");
    hub.addLink(a.link);
    hub.addLink(b.link);
    a.receive({ t: "bump" });
    expect(localBumps()).toBe(1);
    expect(a.sent).toEqual([]);
    expect(b.sent).toEqual([]);
  });

  test("paused_hub_keeps_only_the_latest_advertisement_per_peer", () => {
    const { hub, delivered, localBumps } = hubRig();
    const a = fakeLink("a");
    const b = fakeLink("b");
    hub.addLink(a.link);
    hub.addLink(b.link);
    hub.setPaused(true);
    for (let i = 0; i < 100; i++) a.receive({ t: "adv", b: encodeBase64(ADV) });
    a.receive({ t: "adv", b: encodeBase64(ADV2) });
    b.receive({ t: "adv", b: encodeBase64(ADV) });
    a.receive({ t: "bump" });
    expect(delivered).toEqual([]);
    hub.setPaused(false);
    expect(delivered).toEqual([
      { bytes: [...ADV2], rssi: LINKED_PEER_RSSI },
      { bytes: [...ADV], rssi: LINKED_PEER_RSSI },
    ]);
    // A bump to a paused badge is dropped, not replayed (ruling R-T9-3).
    expect(localBumps()).toBe(0);
    // Nothing more is held.
    hub.setPaused(true);
    hub.setPaused(false);
    expect(delivered).toHaveLength(2);
  });

  test("a removed link is heard no more, and its held advert is dropped", () => {
    const { hub, delivered } = hubRig();
    const a = fakeLink("a");
    hub.addLink(a.link);
    hub.setPaused(true);
    a.receive({ t: "adv", b: encodeBase64(ADV) });
    hub.removeLink("a");
    hub.setPaused(false);
    a.receive({ t: "adv", b: encodeBase64(ADV) });
    hub.fromLocal(ADV);
    expect(delivered).toEqual([]);
    expect(a.sent).toEqual([]);
    expect(() => hub.removeLink("missing")).not.toThrow();
  });

  test("a link replaced under the same id silences the old one", () => {
    const { hub, delivered } = hubRig();
    const old = fakeLink("a");
    const fresh = fakeLink("a");
    hub.addLink(old.link);
    hub.addLink(fresh.link);
    old.receive({ t: "adv", b: encodeBase64(ADV) });
    expect(delivered).toEqual([]);
    fresh.receive({ t: "adv", b: encodeBase64(ADV) });
    expect(delivered).toHaveLength(1);
    hub.fromLocal(ADV);
    expect(old.sent).toEqual([]);
    expect(fresh.sent).toHaveLength(1);
  });

  test("a throwing deliver or bumpLocal does not escape the hub", () => {
    const hub = new RadioHub({
      deliver: () => {
        throw new Error("worker gone");
      },
      bumpLocal: () => {
        throw new Error("worker gone");
      },
    });
    const a = fakeLink("a");
    hub.addLink(a.link);
    hub.setRunning(true);
    quietly(() => {
      expect(() => a.receive({ t: "adv", b: encodeBase64(ADV) })).not.toThrow();
      expect(() => a.receive({ t: "bump" })).not.toThrow();
      expect(() => hub.bump()).not.toThrow();
    });
    expect(a.sent).toEqual([{ t: "bump" }]);
  });
});

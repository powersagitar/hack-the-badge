import { describe, expect, test } from "bun:test";
import {
  acquireBadge,
  BADGE_LOCK_NAME,
  type LockManagerLike,
  loadOrCreateMac,
  MAC_STORAGE_KEY,
  takeOver,
} from "../src/radio/lock";

/**
 * A fake of the Web Locks API's exclusive mode for one lock name at a
 * time, enough for `ifAvailable` and `steal`: a stolen holder's request
 * promise rejects with an `AbortError`, as the spec says, and the lock
 * is held until the callback's promise settles.
 */
class FakeLockManager implements LockManagerLike {
  readonly requests: Array<{ name: string; options: { ifAvailable?: boolean; steal?: boolean } }> = [];
  private holder: { reject: (e: unknown) => void; token: object } | null = null;
  private waiting: Array<() => void> = [];

  request(
    name: string,
    options: { ifAvailable?: boolean; steal?: boolean },
    cb: (lock: { name: string } | null) => unknown,
  ): Promise<unknown> {
    this.requests.push({ name, options });
    return new Promise((resolve, reject) => {
      const grant = () => {
        const token = {};
        this.holder = { reject, token };
        Promise.resolve()
          .then(() => cb({ name }))
          .then(
            (v) => {
              if (this.holder?.token === token) this.release();
              resolve(v);
            },
            (e) => {
              if (this.holder?.token === token) this.release();
              reject(e);
            },
          );
      };
      if (options.steal) {
        if (this.holder) {
          const old = this.holder;
          this.holder = null;
          old.reject(new DOMException("stolen", "AbortError"));
        }
        grant();
        return;
      }
      if (this.holder) {
        if (options.ifAvailable) {
          Promise.resolve()
            .then(() => cb(null))
            .then(resolve, reject);
          return;
        }
        this.waiting.push(grant);
        return;
      }
      grant();
    });
  }

  held(): boolean {
    return this.holder !== null;
  }

  private release(): void {
    this.holder = null;
    this.waiting.shift()?.();
  }
}

const settle = () => new Promise<void>((r) => setTimeout(r, 0));

describe("acquireBadge", () => {
  test("the first tab gets the badge; a second gets null", async () => {
    const locks = new FakeLockManager();
    const a = await acquireBadge(() => {}, locks);
    expect(a).not.toBeNull();
    expect(locks.requests[0]).toEqual({ name: "htn-badge", options: { ifAvailable: true } });
    expect(BADGE_LOCK_NAME).toBe("htn-badge");
    const b = await acquireBadge(() => {}, locks);
    expect(b).toBeNull();
  });

  test("release frees the badge without calling onLost", async () => {
    const locks = new FakeLockManager();
    let lost = 0;
    const a = await acquireBadge(() => lost++, locks);
    a!.release();
    a!.release();
    await settle();
    expect(locks.held()).toBe(false);
    expect(lost).toBe(0);
    expect(await acquireBadge(() => {}, locks)).not.toBeNull();
  });

  test("takeover_stops_the_previous_holder", async () => {
    const locks = new FakeLockManager();
    const events: string[] = [];
    // Tab A runs its badge.
    const runtimeA = { running: true, stop: () => ((runtimeA.running = false), events.push("A stopped")) };
    const holderA = await acquireBadge(() => runtimeA.stop(), locks);
    expect(holderA).not.toBeNull();
    // Tab B finds it taken, then takes it over.
    let lostB = 0;
    expect(await acquireBadge(() => lostB++, locks)).toBeNull();
    await takeOver(locks);
    expect(locks.requests.at(-1)).toEqual({ name: "htn-badge", options: { steal: true } });
    const holderB = await acquireBadge(() => lostB++, locks);
    expect(holderB).not.toBeNull();
    events.push("B started");
    expect(events).toEqual(["A stopped", "B started"]);
    expect(runtimeA.running).toBe(false);
    expect(lostB).toBe(0);
    // A's late release does not free B's badge.
    holderA!.release();
    await settle();
    expect(await acquireBadge(() => {}, locks)).toBeNull();
  });

  test("onLost runs once, and not after the holder released", async () => {
    const locks = new FakeLockManager();
    let lost = 0;
    await acquireBadge(() => lost++, locks);
    await takeOver(locks);
    await takeOver(locks);
    await settle();
    expect(lost).toBe(1);
    const c = await acquireBadge(() => lost++, locks);
    c!.release();
    await settle();
    await takeOver(locks);
    await settle();
    expect(lost).toBe(1);
  });

  test("with no Web Locks the badge always runs here", async () => {
    const h = await acquireBadge(() => {}, null);
    expect(h).not.toBeNull();
    expect(() => h!.release()).not.toThrow();
    await takeOver(null);
  });

  test("a lock manager that throws or rejects does not reject", async () => {
    const throwing: LockManagerLike = {
      request: () => {
        throw new Error("SecurityError");
      },
    };
    const rejecting: LockManagerLike = {
      request: () => Promise.reject(new Error("SecurityError")),
    };
    for (const locks of [throwing, rejecting]) {
      const h = await acquireBadge(() => {}, locks);
      expect(h).not.toBeNull();
      await takeOver(locks);
    }
  });
});

/** A `Storage` over a Map, optionally throwing on every call. */
function fakeStorage(throws = false): Storage & { map: Map<string, string> } {
  const map = new Map<string, string>();
  const guard = () => {
    if (throws) throw new DOMException("denied", "SecurityError");
  };
  return {
    map,
    get length() {
      guard();
      return map.size;
    },
    clear: () => (guard(), map.clear()),
    getItem: (k) => (guard(), map.get(k) ?? null),
    key: (i) => (guard(), [...map.keys()][i] ?? null),
    removeItem: (k) => (guard(), void map.delete(k)),
    setItem: (k, v) => (guard(), void map.set(k, String(v))),
  };
}

const isLocalUnicast = (mac: Uint8Array) => mac.length === 6 && (mac[0]! & 0x03) === 0x02;

describe("loadOrCreateMac", () => {
  test("is stable across calls with storage, and locally administered unicast", () => {
    const s = fakeStorage();
    const a = loadOrCreateMac(s);
    const b = loadOrCreateMac(s);
    expect(isLocalUnicast(a)).toBe(true);
    expect([...b]).toEqual([...a]);
    expect(s.map.get(MAC_STORAGE_KEY)).toMatch(/^([0-9a-f]{2}:){5}[0-9a-f]{2}$/);
    expect(MAC_STORAGE_KEY).toBe("htn-badge-mac");
  });

  test("keeps a stored MAC", () => {
    const s = fakeStorage();
    s.map.set(MAC_STORAGE_KEY, "02:00:00:00:10:01");
    expect([...loadOrCreateMac(s)]).toEqual([0x02, 0, 0, 0, 0x10, 0x01]);
  });

  test("replaces a stored value that is malformed or not locally administered unicast", () => {
    for (const bad of ["nonsense", "02:00:00:00:10", "00:11:22:33:44:55", "03:00:00:00:10:01", "02:00:00:00:10:zz"]) {
      const s = fakeStorage();
      s.map.set(MAC_STORAGE_KEY, bad);
      const mac = loadOrCreateMac(s);
      expect(isLocalUnicast(mac)).toBe(true);
      expect(s.map.get(MAC_STORAGE_KEY)).not.toBe(bad);
    }
  });

  test("works with storage throwing, and with none", () => {
    for (const s of [fakeStorage(true), null]) {
      let mac: Uint8Array | null = null;
      expect(() => (mac = loadOrCreateMac(s))).not.toThrow();
      expect(isLocalUnicast(mac!)).toBe(true);
    }
  });

  test("random MACs differ", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 20; i++) seen.add([...loadOrCreateMac(null)].join(","));
    expect(seen.size).toBeGreaterThan(1);
  });
});

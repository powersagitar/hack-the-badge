import { describe, expect, test } from "bun:test";
import {
  MAX_SDP_CHARS,
  MemoryStore,
  RATE_CLIENT_REQUESTS,
  RATE_CLIENT_REQUESTS_WINDOW_SECONDS,
  RATE_CLIENT_WRITES,
  RATE_CLIENT_WRITES_WINDOW_SECONDS,
  RATE_GLOBAL_REQUESTS,
  RATE_GLOBAL_WINDOW_SECONDS,
  RATE_ROOM_POLLS,
  RATE_ROOM_POLLS_WINDOW_SECONDS,
  ROOM_TTL_SECONDS,
  type SignalStore,
  UpstashStore,
  handleSignal,
  isRoomCode,
} from "../src/radio/signal-handler";

const BASE = "http://localhost/api/signal";
const SDP = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\n";

const post = (body: unknown, raw = false) =>
  new Request(BASE, { method: "POST", body: raw ? (body as string) : JSON.stringify(body) });
const get = (query: string) => new Request(`${BASE}?${query}`);

async function create(store: SignalStore, offer = SDP): Promise<string> {
  const res = await handleSignal(post({ op: "create", offer }), store);
  expect(res.status).toBe(200);
  const { room } = (await res.json()) as { room: string };
  return room;
}

/** A `MemoryStore` whose counters never pass 1: no rate limit. */
class UnlimitedStore extends MemoryStore {
  override async incr(): Promise<number> {
    return 1;
  }
}

describe("signal handler", () => {
  test("create, fetch the offer, answer, fetch the answer", async () => {
    const store = new MemoryStore();
    const room = await create(store);
    expect(isRoomCode(room)).toBe(true);

    const pending = await handleSignal(get(`room=${room}&want=answer`), store);
    expect(pending.status).toBe(204);

    const offer = await handleSignal(get(`room=${room}&want=offer`), store);
    expect(offer.status).toBe(200);
    expect(await offer.json()).toEqual({ offer: SDP });

    const answered = await handleSignal(post({ op: "answer", room, answer: "answer-sdp" }), store);
    expect(answered.status).toBe(204);

    const answer = await handleSignal(get(`room=${room}&want=answer`), store);
    expect(answer.status).toBe(200);
    expect(await answer.json()).toEqual({ answer: "answer-sdp" });
  });

  test("signal_rejects_bad_requests", async () => {
    const store = new MemoryStore();
    const room = await create(store);
    const status = async (req: Request) => (await handleSignal(req, store)).status;

    // Bad JSON, a non-object, an unknown op, a missing or non-string SDP.
    expect(await status(post("{not json", true))).toBe(400);
    expect(await status(post("[1,2]", true))).toBe(400);
    expect(await status(post("null", true))).toBe(400);
    expect(await status(post({ op: "delete", room }))).toBe(400);
    expect(await status(post({ op: "create" }))).toBe(400);
    expect(await status(post({ op: "create", offer: 7 }))).toBe(400);
    expect(await status(post({ op: "create", offer: "" }))).toBe(400);
    // A bad room format, in a body and in a query.
    expect(await status(post({ op: "answer", room: "abc", answer: SDP }))).toBe(400);
    expect(await status(post({ op: "answer", room: "ILOU0000", answer: SDP }))).toBe(400);
    expect(await status(post({ op: "answer", room: 12345678, answer: SDP }))).toBe(400);
    expect(await status(get("room=short&want=offer"))).toBe(400);
    expect(await status(get(`room=${room}&want=everything`))).toBe(400);
    expect(await status(get("want=offer"))).toBe(400);
    // An oversized SDP, and an oversized body.
    expect(await status(post({ op: "create", offer: "x".repeat(MAX_SDP_CHARS + 1) }))).toBe(400);
    expect(await status(post({ op: "create", offer: "x".repeat(MAX_SDP_CHARS) }))).toBe(200);
    expect(await status(post({ op: "answer", room, answer: "x".repeat(MAX_SDP_CHARS + 1) }))).toBe(400);
    // Valid but for its size: the body cap, not the op check, refuses it.
    expect(await status(post({ op: "create", offer: SDP, pad: "x".repeat(200_000) }))).toBe(400);
    // An unknown room.
    expect(await status(get("room=ZZZZZZZZ&want=offer"))).toBe(404);
    expect(await status(post({ op: "answer", room: "ZZZZZZZZ", answer: SDP }))).toBe(404);
    // Another method.
    expect(await status(new Request(BASE, { method: "PUT", body: "{}" }))).toBe(405);
    // A second answer.
    expect(await status(post({ op: "answer", room, answer: "first" }))).toBe(204);
    expect(await status(post({ op: "answer", room, answer: "second" }))).toBe(409);
    const answer = await handleSignal(get(`room=${room}&want=answer`), store);
    expect(await answer.json()).toEqual({ answer: "first" });
  });

  test("a store that fails gives a bare 500", async () => {
    const broken = {
      set: () => Promise.reject(new Error("secret detail")),
      get: () => Promise.reject(new Error("secret detail")),
    };
    const res = await handleSignal(post({ op: "create", offer: SDP }), broken);
    expect(res.status).toBe(500);
    expect(await res.text()).not.toContain("secret");
  });

  test("rooms expire after the TTL", async () => {
    let now = 1_000_000;
    const store = new MemoryStore(() => now);
    const room = await create(store);
    now += (ROOM_TTL_SECONDS - 1) * 1000;
    expect((await handleSignal(get(`room=${room}&want=offer`), store)).status).toBe(200);
    now += 2000;
    expect((await handleSignal(get(`room=${room}&want=offer`), store)).status).toBe(404);
    expect((await handleSignal(get(`room=${room}&want=answer`), store)).status).toBe(404);
    expect((await handleSignal(post({ op: "answer", room, answer: SDP }), store)).status).toBe(404);
  });

  test("MemoryStore honours onlyIfAbsent and the TTL", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    expect(await store.set("k", "a", 10, true)).toBe(true);
    expect(await store.set("k", "b", 10, true)).toBe(false);
    expect(await store.get("k")).toBe("a");
    now = 10_000;
    expect(await store.get("k")).toBeNull();
    expect(await store.set("k", "c", 10, true)).toBe(true);
  });

  test("room codes are unique over 10,000 creates", async () => {
    // Far over the rate caps; this test is about the codes.
    const store = new UnlimitedStore();
    const rooms = new Set<string>();
    for (let i = 0; i < 10_000; i++) {
      const res = await handleSignal(post({ op: "create", offer: SDP }), store);
      const { room } = (await res.json()) as { room: string };
      expect(isRoomCode(room)).toBe(true);
      rooms.add(room);
    }
    expect(rooms.size).toBe(10_000);
  });
});

describe("UpstashStore", () => {
  test("sends SET with EX and NX, and GET, as REST command arrays", async () => {
    const sent: Array<{ url: string; auth: string | null; body: unknown }> = [];
    const replies: unknown[] = [{ result: "OK" }, { result: null }, { result: "v" }, { result: null }];
    const fakeFetch = (async (url: string, init: RequestInit) => {
      sent.push({
        url,
        auth: new Headers(init.headers).get("authorization"),
        body: JSON.parse(String(init.body)),
      });
      return Response.json(replies.shift());
    }) as unknown as typeof fetch;
    const store = new UpstashStore("https://example.upstash.io", "tok", fakeFetch);
    expect(await store.set("k", "v", 300, true)).toBe(true);
    expect(await store.set("k", "w", 300, true)).toBe(false);
    expect(await store.get("k")).toBe("v");
    expect(await store.get("missing")).toBeNull();
    expect(sent.map((s) => s.body)).toEqual([
      ["SET", "k", "v", "EX", "300", "NX"],
      ["SET", "k", "w", "EX", "300", "NX"],
      ["GET", "k"],
      ["GET", "missing"],
    ]);
    expect(sent.every((s) => s.url === "https://example.upstash.io" && s.auth === "Bearer tok")).toBe(true);
  });

  test("an Upstash error rejects, and the handler turns it into a bare 500", async () => {
    const fakeFetch = (async () => Response.json({ error: "WRONGPASS" }, { status: 401 })) as unknown as typeof fetch;
    const store = new UpstashStore("https://example.upstash.io", "tok", fakeFetch);
    await expect(store.get("k")).rejects.toThrow();
    const res = await handleSignal(post({ op: "create", offer: SDP }), store);
    expect(res.status).toBe(500);
    expect(await res.text()).toBe("");
  });
});

describe("the Vercel entry", () => {
  test("without its env vars it answers a bare 500", async () => {
    const saved = { url: process.env.KV_REST_API_URL, token: process.env.KV_REST_API_TOKEN };
    delete process.env.KV_REST_API_URL;
    delete process.env.KV_REST_API_TOKEN;
    try {
      const { default: entry } = await import("../api/signal");
      const res = await entry.fetch(post({ op: "create", offer: SDP }));
      expect(res.status).toBe(500);
      expect(await res.text()).toBe("");
    } finally {
      if (saved.url !== undefined) process.env.KV_REST_API_URL = saved.url;
      if (saved.token !== undefined) process.env.KV_REST_API_TOKEN = saved.token;
    }
  });
});

describe("rate limit", () => {
  const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  /** The `i`th room code: distinct per `i`, so no room cap binds. */
  const roomCode = (i: number) => {
    let code = "";
    for (let k = 0; k < 8; k++) code = ALPHABET[Math.floor(i / 32 ** k) % 32] + code;
    return code;
  };
  const headersFor = (ip: string | null, extra: Record<string, string> = {}) => {
    const headers: Record<string, string> = { ...extra };
    if (ip !== null) headers["x-forwarded-for"] = ip;
    return headers;
  };
  /** A poll of `room` (default an unknown room: 404) from `ip`. */
  const poll = (ip: string | null, room = "ZZZZZZZZ", extra: Record<string, string> = {}) =>
    new Request(`${BASE}?room=${room}&want=offer`, { headers: headersFor(ip, extra) });
  const write = (ip: string | null, body: unknown = { op: "create", offer: SDP }) =>
    new Request(BASE, { method: "POST", headers: headersFor(ip), body: JSON.stringify(body) });
  const status = async (req: Request, store: SignalStore) => (await handleSignal(req, store)).status;

  /** A spy over a `MemoryStore` on the clock `now`: every call, and every `incr` key. */
  function spyStore(now: () => number) {
    const inner = new MemoryStore(now);
    const calls: string[] = [];
    const keys: string[] = [];
    const store: SignalStore = {
      set: (...a) => (calls.push("set"), inner.set(...a)),
      get: (k) => (calls.push("get"), inner.get(k)),
      incr: (k, t) => (calls.push("incr"), keys.push(k), inner.incr(k, t)),
    };
    return { inner, store, calls, keys };
  }

  async function expect429(req: Request, store: SignalStore, windowSeconds: number) {
    const res = await handleSignal(req, store);
    expect(res.status).toBe(429);
    expect(res.headers.get("retry-after")).toBe(String(windowSeconds));
    expect(await res.text()).toBe("");
  }

  test("the windows are the ruled ones", () => {
    expect([RATE_CLIENT_WRITES, RATE_CLIENT_WRITES_WINDOW_SECONDS]).toEqual([30, 60]);
    expect([RATE_ROOM_POLLS, RATE_ROOM_POLLS_WINDOW_SECONDS]).toEqual([200, ROOM_TTL_SECONDS]);
    expect([RATE_CLIENT_REQUESTS, RATE_CLIENT_REQUESTS_WINDOW_SECONDS]).toEqual([600, 600]);
    expect([RATE_GLOBAL_REQUESTS, RATE_GLOBAL_WINDOW_SECONDS]).toEqual([5000, 600]);
  });

  test("writes per client: a 429 at write 31, reset after 60 s; answers count too", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    for (let i = 0; i < RATE_CLIENT_WRITES - 1; i++) expect(await status(write("203.0.113.7"), store)).toBe(200);
    // The 30th write is an answer (to an unknown room: 404, still a write).
    expect(await status(write("203.0.113.7", { op: "answer", room: "ZZZZZZZZ", answer: SDP }), store)).toBe(404);
    await expect429(write("203.0.113.7"), store, RATE_CLIENT_WRITES_WINDOW_SECONDS);
    // Polls are not writes.
    expect(await status(poll("203.0.113.7"), store)).toBe(404);
    now += 59_000;
    await expect429(write("203.0.113.7"), store, RATE_CLIENT_WRITES_WINDOW_SECONDS);
    now += 1_000;
    expect(await status(write("203.0.113.7"), store)).toBe(200);
  });

  test("polls per room: a 429 at poll 201, reset after the room's 300 s", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    // From many clients behind no single cap: the room's own counter binds.
    for (let i = 0; i < RATE_ROOM_POLLS; i++) expect(await status(poll(`10.0.0.${i % 250}`), store)).toBe(404);
    await expect429(poll("10.0.1.1"), store, RATE_ROOM_POLLS_WINDOW_SECONDS);
    now += 299_000;
    await expect429(poll("10.0.1.2"), store, RATE_ROOM_POLLS_WINDOW_SECONDS);
    now += 1_000;
    expect(await status(poll("10.0.1.3"), store)).toBe(404);
  });

  test("a room polled to its cap blocks neither a second room nor another client's writes", async () => {
    const store = new MemoryStore(() => 0);
    const busy = await create(store);
    const res = await handleSignal(write("198.51.100.1"), store);
    const { room: other } = (await res.json()) as { room: string };
    for (let i = 0; i < RATE_ROOM_POLLS; i++) {
      expect(await status(new Request(`${BASE}?room=${busy}&want=answer`), store)).toBe(204);
    }
    expect(await status(new Request(`${BASE}?room=${busy}&want=answer`), store)).toBe(429);
    // The same NAT ("unknown") polls another room, and writes.
    expect(await status(new Request(`${BASE}?room=${other}&want=answer`), store)).toBe(204);
    expect(await status(post({ op: "answer", room: other, answer: SDP }), store)).toBe(204);
    expect(await status(write("198.51.100.1"), store)).toBe(200);
  });

  test("all requests per client: a 429 at request 601, reset after 10 min", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    for (let i = 0; i < RATE_CLIENT_REQUESTS; i++) {
      expect(await status(poll("203.0.113.7", roomCode(i)), store)).toBe(404);
    }
    await expect429(poll("203.0.113.7", roomCode(9999)), store, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    // Another client is unaffected.
    expect(await status(poll("203.0.113.8", roomCode(9999)), store)).toBe(404);
    now += 599_000;
    await expect429(poll("203.0.113.7", roomCode(9998)), store, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    now += 1_000;
    expect(await status(poll("203.0.113.7", roomCode(9997)), store)).toBe(404);
  });

  test("all requests, global: a 429 at request 5001, reset after 10 min", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    for (let i = 0; i < RATE_GLOBAL_REQUESTS; i++) {
      expect(await status(poll(`10.${i >> 16}.${(i >> 8) & 255}.${i & 255}`, roomCode(i)), store)).toBe(404);
    }
    await expect429(poll("192.0.2.1", roomCode(99_999)), store, RATE_GLOBAL_WINDOW_SECONDS);
    now += 599_000;
    await expect429(poll("192.0.2.2", roomCode(99_998)), store, RATE_GLOBAL_WINDOW_SECONDS);
    now += 1_000;
    expect(await status(poll("192.0.2.3", roomCode(99_997)), store)).toBe(404);
  });

  test("one client is at most 12% of the global window", () => {
    expect(RATE_CLIENT_REQUESTS / RATE_GLOBAL_REQUESTS).toBeLessThanOrEqual(0.12);
    expect(RATE_CLIENT_REQUESTS_WINDOW_SECONDS).toBe(RATE_GLOBAL_WINDOW_SECONDS);
  });

  test("a client over its own cap costs one counter call and never moves the global counter", async () => {
    const { inner, store, calls, keys } = spyStore(() => 0);
    for (let i = 0; i < RATE_CLIENT_REQUESTS; i++) await handleSignal(poll("203.0.113.7", roomCode(i)), store);
    const global = await inner.get("rl:global");
    calls.length = 0;
    keys.length = 0;
    await expect429(write("203.0.113.7"), store, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    await expect429(poll("203.0.113.7"), store, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    expect(calls).toEqual(["incr", "incr"]);
    expect(keys.every((k) => k.startsWith("rl:client:"))).toBe(true);
    expect(await inner.get("rl:global")).toBe(global);
  });

  test("a write over its cap costs two counter calls and never moves the global counter", async () => {
    const { inner, store, calls } = spyStore(() => 0);
    for (let i = 0; i < RATE_CLIENT_WRITES; i++) await handleSignal(write("203.0.113.7"), store);
    const global = await inner.get("rl:global");
    calls.length = 0;
    await expect429(write("203.0.113.7"), store, RATE_CLIENT_WRITES_WINDOW_SECONDS);
    expect(calls).toEqual(["incr", "incr"]);
    expect(await inner.get("rl:global")).toBe(global);
  });

  test("a bad request is a 400 that makes no counter key", async () => {
    const { store, calls } = spyStore(() => 0);
    expect(await status(poll("203.0.113.7", "short"), store)).toBe(400);
    expect(await status(poll("203.0.113.7", "ILOU0000"), store)).toBe(400);
    expect(await status(new Request(`${BASE}?room=ZZZZZZZZ&want=all`), store)).toBe(400);
    expect(await status(write("203.0.113.7", { op: "nope" }), store)).toBe(400);
    expect(await status(write("203.0.113.7", { op: "answer", room: "bad", answer: SDP }), store)).toBe(400);
    expect(calls).toEqual([]);
  });

  test("two clients get independent windows; the first forwarded entry names the client", async () => {
    const store = new MemoryStore(() => 0);
    for (let i = 0; i < RATE_CLIENT_WRITES; i++) await handleSignal(write("203.0.113.7, 10.0.0.1"), store);
    expect(await status(write("203.0.113.7"), store)).toBe(429);
    expect(await status(write("198.51.100.9, 203.0.113.7"), store)).toBe(200);
    expect(await status(write("2001:db8::1"), store)).toBe(200);
    // x-real-ip when there is no x-forwarded-for.
    const real = new Request(BASE, {
      method: "POST",
      headers: { "x-real-ip": "198.51.100.10" },
      body: JSON.stringify({ op: "create", offer: SDP }),
    });
    expect(await status(real, store)).toBe(200);
  });

  test("garbage forwarded headers neither throw nor make big keys", async () => {
    const { store, keys } = spyStore(() => 0);
    const garbage = [
      "x".repeat(100_000),
      "\u00ff\u00fe; DROP TABLE",
      ",,,",
      " ",
      "1.2.3.4".repeat(1000),
      "::::::::::::::::::::::::::::::::::::::::::::::::::::::::::::::",
      "rl:global",
    ];
    for (const g of garbage) {
      expect(await status(poll(g, "ZZZZZZZZ", { "x-real-ip": g }), store)).toBe(404);
    }
    expect(keys.length).toBeGreaterThan(0);
    for (const k of keys) {
      expect(k.length).toBeLessThanOrEqual(64);
      expect(k.startsWith("rl:")).toBe(true);
      expect(/^[A-Za-z0-9:._-]+$/.test(k)).toBe(true);
    }
    expect(new Set(keys)).toEqual(new Set(["rl:client:unknown", "rl:room:ZZZZZZZZ", "rl:global"]));
  });

  test("the dev two-device flow stays far under every cap", async () => {
    // Two devices on one key ("unknown"), one room each way, the host
    // polling 1/s for the full 120 s before the answer.
    const { inner, store } = spyStore(() => 0);
    let requests = 0;
    for (let way = 0; way < 2; way++) {
      const room = await create(store);
      requests++;
      for (let s = 0; s < 120; s++, requests++) await handleSignal(get(`room=${room}&want=answer`), store);
      expect(await status(get(`room=${room}&want=offer`), store)).toBe(200);
      expect(await status(post({ op: "answer", room, answer: SDP }), store)).toBe(204);
      requests += 2;
      expect(await status(get(`room=${room}&want=answer`), store)).toBe(200);
      requests++;
      expect(Number(await inner.get(`rl:room:${room}`))).toBe(122);
    }
    expect(requests).toBe(248);
    expect(Number(await inner.get("rl:client:unknown"))).toBe(248);
    expect(Number(await inner.get("rl:global"))).toBe(248);
    expect(Number(await inner.get("rl:writes:unknown"))).toBe(4);
  });

  test("UpstashStore.incr sends INCR and EXPIRE NX in one pipeline request", async () => {
    const sent: Array<{ url: string; auth: string | null; body: unknown }> = [];
    const fakeFetch = (async (url: string, init: RequestInit) => {
      sent.push({ url, auth: new Headers(init.headers).get("authorization"), body: JSON.parse(String(init.body)) });
      return Response.json([{ result: 3 }, { result: 0 }]);
    }) as unknown as typeof fetch;
    const store = new UpstashStore("https://example.upstash.io", "tok", fakeFetch);
    expect(await store.incr("rl:global", 3600)).toBe(3);
    expect(sent).toEqual([
      {
        url: "https://example.upstash.io/pipeline",
        auth: "Bearer tok",
        body: [
          ["INCR", "rl:global"],
          ["EXPIRE", "rl:global", "3600", "NX"],
        ],
      },
    ]);
  });

  test("UpstashStore.incr rejects on a pipeline error, and the handler answers a bare 500", async () => {
    const fakeFetch = (async () => Response.json([{ error: "ERR" }, { result: 0 }])) as unknown as typeof fetch;
    const store = new UpstashStore("https://example.upstash.io", "tok", fakeFetch);
    await expect(store.incr("k", 60)).rejects.toThrow();
    const res = await handleSignal(post({ op: "create", offer: SDP }), store);
    expect(res.status).toBe(500);
    expect(await res.text()).toBe("");
  });

  test("UpstashStore's default fetch is called unbound-safe", async () => {
    const original = globalThis.fetch;
    let receiver: unknown = "unset";
    globalThis.fetch = function (this: unknown) {
      receiver = this;
      return Promise.resolve(Response.json({ result: "OK" }));
    } as typeof fetch;
    try {
      const store = new UpstashStore("https://example.upstash.io", "tok");
      expect(await store.set("k", "v", 10, false)).toBe(true);
      // Called as a plain function, never as a method of the store.
      expect(receiver).not.toBe(store);
    } finally {
      globalThis.fetch = original;
    }
  });
});

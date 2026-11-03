import { describe, expect, test } from "bun:test";
import {
  MAX_SDP_CHARS,
  MemoryStore,
  RATE_GLOBAL_PER_HOUR,
  RATE_PER_CLIENT_PER_MIN,
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

async function create(store: MemoryStore, offer = SDP): Promise<string> {
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
  const from = (ip: string | null, extra: Record<string, string> = {}) => {
    const headers: Record<string, string> = { ...extra };
    if (ip !== null) headers["x-forwarded-for"] = ip;
    return new Request(`${BASE}?room=ZZZZZZZZ&want=offer`, { headers });
  };

  test("the per-client cap gives a 429 on request 241 of a window, and the window resets after 60 s", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    for (let i = 0; i < RATE_PER_CLIENT_PER_MIN; i++) {
      expect((await handleSignal(from("203.0.113.7"), store)).status).toBe(404);
    }
    const limited = await handleSignal(from("203.0.113.7"), store);
    expect(limited.status).toBe(429);
    expect(limited.headers.get("retry-after")).toMatch(/^[1-9][0-9]*$/);
    expect((await limited.text()).length).toBeLessThan(64);
    now += 59_000;
    expect((await handleSignal(from("203.0.113.7"), store)).status).toBe(429);
    now += 1_000;
    expect((await handleSignal(from("203.0.113.7"), store)).status).toBe(404);
  });

  test("two clients get independent windows; the first forwarded entry names the client", async () => {
    const store = new MemoryStore(() => 0);
    for (let i = 0; i < RATE_PER_CLIENT_PER_MIN; i++) await handleSignal(from("203.0.113.7, 10.0.0.1"), store);
    expect((await handleSignal(from("203.0.113.7"), store)).status).toBe(429);
    expect((await handleSignal(from("198.51.100.9, 203.0.113.7"), store)).status).toBe(404);
    expect((await handleSignal(from("2001:db8::1"), store)).status).toBe(404);
    // x-real-ip when there is no x-forwarded-for.
    expect((await handleSignal(from(null, { "x-real-ip": "198.51.100.10" }), store)).status).toBe(404);
  });

  test("the global cap gives a 429 across many clients, for an hour", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    for (let i = 0; i < RATE_GLOBAL_PER_HOUR; i++) {
      const res = await handleSignal(from(`10.0.${i >> 8}.${i & 255}`), store);
      expect(res.status).toBe(404);
    }
    const limited = await handleSignal(from("192.0.2.1"), store);
    expect(limited.status).toBe(429);
    expect(limited.headers.get("retry-after")).toMatch(/^[1-9][0-9]*$/);
    now += 3_599_000;
    expect((await handleSignal(from("192.0.2.2"), store)).status).toBe(429);
    now += 1_000;
    expect((await handleSignal(from("192.0.2.3"), store)).status).toBe(404);
  });

  test("a rejected request costs only counter calls", async () => {
    const calls: string[] = [];
    const inner = new MemoryStore(() => 0);
    const spy: SignalStore = {
      set: (...a) => (calls.push("set"), inner.set(...a)),
      get: (k) => (calls.push("get"), inner.get(k)),
      incr: (k, t) => (calls.push("incr"), inner.incr(k, t)),
    };
    for (let i = 0; i < RATE_PER_CLIENT_PER_MIN; i++) await handleSignal(from("203.0.113.7"), spy);
    calls.length = 0;
    expect((await handleSignal(post({ op: "create", offer: SDP }), spy)).status).toBe(200);
    calls.length = 0;
    const req = new Request(BASE, {
      method: "POST",
      headers: { "x-forwarded-for": "203.0.113.7" },
      body: JSON.stringify({ op: "create", offer: SDP }),
    });
    expect((await handleSignal(req, spy)).status).toBe(429);
    expect(calls.every((c) => c === "incr")).toBe(true);
  });

  test("garbage forwarded headers neither throw nor make big keys", async () => {
    const keys: string[] = [];
    const inner = new MemoryStore(() => 0);
    const spy: SignalStore = {
      set: (...a) => inner.set(...a),
      get: (k) => inner.get(k),
      incr: (k, t) => (keys.push(k), inner.incr(k, t)),
    };
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
      const res = await handleSignal(from(g, { "x-real-ip": g }), spy);
      expect(res.status).toBe(404);
    }
    expect(keys.length).toBeGreaterThan(0);
    for (const k of keys) {
      expect(k.length).toBeLessThanOrEqual(64);
      expect(k.startsWith("rl:")).toBe(true);
      expect(/^[A-Za-z0-9:._-]+$/.test(k)).toBe(true);
    }
    // Garbage never reaches the global counter's key.
    expect(new Set(keys.filter((k) => !k.startsWith("rl:client:")))).toEqual(new Set(["rl:global"]));
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

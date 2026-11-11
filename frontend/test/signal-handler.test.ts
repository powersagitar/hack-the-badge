import { describe, expect, test } from "bun:test";
import { Redis } from "@upstash/redis";
import { REDIS_OPTIONS, redisRoomStore, upstashLimiter } from "../api/signal";
import {
  type Limiter,
  MAX_BODY_BYTES,
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
  type RoomStore,
  type SignalDeps,
  TOKEN_LENGTH,
  clientKey,
  handleSignal,
  isRoomCode,
  memoryDeps,
  memoryLimiter,
} from "../src/radio/signal-handler";

const BASE = "http://localhost/api/signal";
const SDP = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\n";

const post = (body: unknown, raw = false) =>
  new Request(BASE, { method: "POST", body: raw ? (body as string) : JSON.stringify(body) });
const get = (query: string) => new Request(`${BASE}?${query}`);

async function createRoom(deps: SignalDeps, offer = SDP): Promise<{ room: string; token: string }> {
  const res = await handleSignal(post({ op: "create", offer }), deps);
  expect(res.status).toBe(200);
  return (await res.json()) as { room: string; token: string };
}
const create = async (deps: SignalDeps, offer = SDP) => (await createRoom(deps, offer)).room;

/** A limiter that always lets a request through. */
const unlimited: Limiter = { limit: async () => ({ success: true, reset: 0 }) };
const unlimitedDeps = (rooms: RoomStore = new MemoryStore()): SignalDeps => ({
  rooms,
  limiters: { client: unlimited, roomPolls: unlimited, clientWrites: unlimited, global: unlimited },
});

describe("signal handler", () => {
  test("create, fetch the offer, answer, fetch the answer", async () => {
    const deps = memoryDeps();
    const room = await create(deps);
    expect(isRoomCode(room)).toBe(true);

    const pending = await handleSignal(get(`room=${room}&want=answer`), deps);
    expect(pending.status).toBe(204);

    const offer = await handleSignal(get(`room=${room}&want=offer`), deps);
    expect(offer.status).toBe(200);
    expect(await offer.json()).toEqual({ offer: SDP });

    const answered = await handleSignal(post({ op: "answer", room, answer: "answer-sdp" }), deps);
    expect(answered.status).toBe(204);

    const answer = await handleSignal(get(`room=${room}&want=answer`), deps);
    expect(answer.status).toBe(200);
    expect(await answer.json()).toEqual({ answer: "answer-sdp" });
  });

  test("create_returns_a_token", async () => {
    const rooms = new MemoryStore();
    const deps = memoryDeps(undefined, rooms);
    const a = await createRoom(deps);
    const b = await createRoom(deps);
    expect(Object.keys(a).sort()).toEqual(["room", "token"]);
    expect(TOKEN_LENGTH).toBe(22);
    for (const { token } of [a, b]) expect(token).toMatch(/^[A-Za-z0-9_-]{22}$/);
    expect(a.token).not.toBe(b.token);
    // Stored with the offer, under the room's own token key.
    expect(await rooms.get(`htn-signal:${a.room}:token`)).toBe(a.token);
  });

  test("the token is stored with the offer's TTL", async () => {
    let now = 1_000_000;
    const rooms = new MemoryStore(() => now);
    const { room } = await createRoom(memoryDeps(() => now, rooms));
    now += (ROOM_TTL_SECONDS - 1) * 1000;
    expect(await rooms.get(`htn-signal:${room}:token`)).not.toBeNull();
    now += 2000;
    expect(await rooms.get(`htn-signal:${room}:token`)).toBeNull();
  });

  test("close_deletes_the_room", async () => {
    const rooms = new MemoryStore();
    const deps = memoryDeps(undefined, rooms);
    const { room, token } = await createRoom(deps);
    expect((await handleSignal(post({ op: "answer", room, answer: SDP }), deps)).status).toBe(204);
    const closed = await handleSignal(post({ op: "close", room, token }), deps);
    expect(closed.status).toBe(204);
    expect(await closed.text()).toBe("");
    expect((await handleSignal(get(`room=${room}&want=offer`), deps)).status).toBe(404);
    expect((await handleSignal(get(`room=${room}&want=answer`), deps)).status).toBe(404);
    expect((await handleSignal(post({ op: "answer", room, answer: SDP }), deps)).status).toBe(404);
    for (const part of ["offer", "answer", "token"]) expect(await rooms.get(`htn-signal:${room}:${part}`)).toBeNull();
    // A second close finds nothing.
    expect((await handleSignal(post({ op: "close", room, token }), deps)).status).toBe(404);
  });

  test("close_with_a_wrong_token_is_404_and_keeps_the_room", async () => {
    const deps = memoryDeps();
    const { room, token } = await createRoom(deps);
    const other = await createRoom(deps);
    const wrong = token.startsWith("A") ? `B${token.slice(1)}` : `A${token.slice(1)}`;
    expect((await handleSignal(post({ op: "close", room, token: wrong }), deps)).status).toBe(404);
    // Another room's token does not close this one.
    expect((await handleSignal(post({ op: "close", room, token: other.token }), deps)).status).toBe(404);
    // An unknown room.
    expect((await handleSignal(post({ op: "close", room: "ZZZZZZZZ", token }), deps)).status).toBe(404);
    const offer = await handleSignal(get(`room=${room}&want=offer`), deps);
    expect(offer.status).toBe(200);
    expect(await offer.json()).toEqual({ offer: SDP });
  });

  test("valibot_rejections_are_400", async () => {
    const deps = memoryDeps();
    const { room, token } = await createRoom(deps);
    const status = async (req: Request) => (await handleSignal(req, deps)).status;

    // Bad JSON, a non-object, an unknown op, a missing or non-string SDP.
    expect(await status(post("{not json", true))).toBe(400);
    expect(await status(post("[1,2]", true))).toBe(400);
    expect(await status(post("null", true))).toBe(400);
    expect(await status(post("", true))).toBe(400);
    expect(await status(post({ op: "delete", room }))).toBe(400);
    expect(await status(post({ room }))).toBe(400);
    expect(await status(post({ op: "create" }))).toBe(400);
    expect(await status(post({ op: "create", offer: 7 }))).toBe(400);
    expect(await status(post({ op: "create", offer: "" }))).toBe(400);
    // A bad room format, in a body and in a query.
    expect(await status(post({ op: "answer", room: "abc", answer: SDP }))).toBe(400);
    expect(await status(post({ op: "answer", room: "ILOU0000", answer: SDP }))).toBe(400);
    expect(await status(post({ op: "answer", room: 12345678, answer: SDP }))).toBe(400);
    expect(await status(post({ op: "answer", answer: SDP }))).toBe(400);
    expect(await status(get("room=short&want=offer"))).toBe(400);
    expect(await status(get(`room=${room}&want=everything`))).toBe(400);
    expect(await status(get("want=offer"))).toBe(400);
    expect(await status(get(`room=${room}`))).toBe(400);
    // An oversized SDP, and an oversized body.
    expect(await status(post({ op: "create", offer: "x".repeat(MAX_SDP_CHARS + 1) }))).toBe(400);
    expect(await status(post({ op: "create", offer: "x".repeat(MAX_SDP_CHARS) }))).toBe(200);
    expect(await status(post({ op: "answer", room, answer: "x".repeat(MAX_SDP_CHARS + 1) }))).toBe(400);
    // Valid but for its size: the body cap, not the op check, refuses it.
    expect(await status(post({ op: "create", offer: SDP, pad: "x".repeat(200_000) }))).toBe(400);
    // A close without a token, or with a malformed one, or a bad room.
    expect(await status(post({ op: "close", room }))).toBe(400);
    expect(await status(post({ op: "close", room, token: 7 }))).toBe(400);
    expect(await status(post({ op: "close", room, token: token.slice(1) }))).toBe(400);
    expect(await status(post({ op: "close", room, token: `${token}A` }))).toBe(400);
    expect(await status(post({ op: "close", room, token: `${token.slice(1)}+` }))).toBe(400);
    expect(await status(post({ op: "close", room: "bad", token }))).toBe(400);
    expect(await status(post({ op: "close", token }))).toBe(400);
    // An unknown room.
    expect(await status(get("room=ZZZZZZZZ&want=offer"))).toBe(404);
    expect(await status(post({ op: "answer", room: "ZZZZZZZZ", answer: SDP }))).toBe(404);
    // Another method.
    expect(await status(new Request(BASE, { method: "PUT", body: "{}" }))).toBe(405);
    // A second answer.
    expect(await status(post({ op: "answer", room, answer: "first" }))).toBe(204);
    expect(await status(post({ op: "answer", room, answer: "second" }))).toBe(409);
    const answer = await handleSignal(get(`room=${room}&want=answer`), deps);
    expect(await answer.json()).toEqual({ answer: "first" });
  });

  test("a store that fails gives a bare 500", async () => {
    const broken: RoomStore = {
      set: () => Promise.reject(new Error("secret detail")),
      get: () => Promise.reject(new Error("secret detail")),
      del: () => Promise.reject(new Error("secret detail")),
    };
    const res = await handleSignal(post({ op: "create", offer: SDP }), memoryDeps(undefined, broken));
    expect(res.status).toBe(500);
    expect(await res.text()).not.toContain("secret");
  });

  test("store_failure_in_each_op_is_a_bare_500", async () => {
    // Review Focus 4: a store or limiter failing on any call of create,
    // poll, answer or close is a bare 500, and never a throw.
    const reject = () => Promise.reject(new Error("quota exceeded: secret"));
    const thrown = () => {
      throw new Error("sync secret");
    };
    type Fault = { rooms?: "set" | "get" | "del"; limiter?: keyof SignalDeps["limiters"]; fail: () => never | Promise<never> };
    /** Each op on a fresh room (offer and token, no answer yet), in order: create, poll offer, poll answer, answer, close. */
    async function statuses(fault: Fault): Promise<number[]> {
      const good = new MemoryStore();
      const { room, token } = await createRoom(unlimitedDeps(good));
      const rooms: RoomStore = {
        set: fault.rooms === "set" ? fault.fail : (...a) => good.set(...a),
        get: fault.rooms === "get" ? fault.fail : (k) => good.get(k),
        del: fault.rooms === "del" ? fault.fail : (k) => good.del(k),
      };
      const deps = unlimitedDeps(rooms);
      if (fault.limiter) deps.limiters = { ...deps.limiters, [fault.limiter]: { limit: fault.fail } };
      const out: number[] = [];
      for (const req of [
        post({ op: "create", offer: SDP }),
        get(`room=${room}&want=offer`),
        get(`room=${room}&want=answer`),
        post({ op: "answer", room, answer: SDP }),
        post({ op: "close", room, token }),
      ]) {
        const res = await handleSignal(req, deps);
        if (res.status === 500) {
          expect(await res.text()).toBe("");
          expect(res.headers.get("content-type")).toBeNull();
        }
        out.push(res.status);
      }
      return out;
    }
    for (const fail of [reject, thrown]) {
      // The store: each op fails exactly where it calls the failing method.
      expect(await statuses({ rooms: "set", fail })).toEqual([500, 200, 204, 500, 204]);
      expect(await statuses({ rooms: "get", fail })).toEqual([200, 500, 500, 500, 500]);
      expect(await statuses({ rooms: "del", fail })).toEqual([200, 200, 204, 204, 500]);
      // The limiters: each fails every op it counts.
      expect(await statuses({ limiter: "client", fail })).toEqual([500, 500, 500, 500, 500]);
      expect(await statuses({ limiter: "global", fail })).toEqual([500, 500, 500, 500, 500]);
      expect(await statuses({ limiter: "roomPolls", fail })).toEqual([200, 500, 500, 204, 204]);
      expect(await statuses({ limiter: "clientWrites", fail })).toEqual([500, 200, 204, 500, 500]);
    }
  });
  test("rooms expire after the TTL", async () => {
    let now = 1_000_000;
    const deps = memoryDeps(() => now);
    const room = await create(deps);
    now += (ROOM_TTL_SECONDS - 1) * 1000;
    expect((await handleSignal(get(`room=${room}&want=offer`), deps)).status).toBe(200);
    now += 2000;
    expect((await handleSignal(get(`room=${room}&want=offer`), deps)).status).toBe(404);
    expect((await handleSignal(get(`room=${room}&want=answer`), deps)).status).toBe(404);
    expect((await handleSignal(post({ op: "answer", room, answer: SDP }), deps)).status).toBe(404);
  });

  test("MemoryStore honours onlyIfAbsent, the TTL and del", async () => {
    let now = 0;
    const store = new MemoryStore(() => now);
    expect(await store.set("k", "a", 10, true)).toBe(true);
    expect(await store.set("k", "b", 10, true)).toBe(false);
    expect(await store.get("k")).toBe("a");
    now = 10_000;
    expect(await store.get("k")).toBeNull();
    expect(await store.set("k", "c", 10, true)).toBe(true);
    await store.set("j", "d", 10, false);
    await store.del(["k", "j", "missing"]);
    expect(await store.get("k")).toBeNull();
    expect(await store.get("j")).toBeNull();
  });

  test("room codes are unique over 10,000 creates", async () => {
    // Far over the rate caps; this test is about the codes.
    const deps = unlimitedDeps();
    const rooms = new Set<string>();
    for (let i = 0; i < 10_000; i++) {
      const res = await handleSignal(post({ op: "create", offer: SDP }), deps);
      const { room } = (await res.json()) as { room: string };
      expect(isRoomCode(room)).toBe(true);
      rooms.add(room);
    }
    expect(rooms.size).toBe(10_000);
  });
});

describe("clientKey", () => {
  const req = (headers: Record<string, string>) => new Request(BASE, { headers });
  const xff = (value: string) => clientKey(req({ "x-forwarded-for": value }));

  test("ipv6_clients_share_a_64_prefix_key", async () => {
    // Two addresses in one /64 are one key; one in another /64 is not.
    const a = xff("2001:db8:1:2:aaaa::1");
    const b = xff("2001:0DB8:0001:0002:ffff:eeee:dddd:cccc");
    const c = xff("2001:db8:1:3::1");
    expect(a).toBe("2001:0db8:0001:0002");
    expect(b).toBe(a);
    expect(c).toBe("2001:0db8:0001:0003");
    // Compressed forms expand.
    expect(xff("::1")).toBe("0000:0000:0000:0000");
    expect(xff("2001:db8::")).toBe("2001:0db8:0000:0000");
    expect(xff("fe80::1:2:3:4")).toBe("fe80:0000:0000:0000");
    expect(xff("1:2:3:4:5:6:7:8, 10.0.0.1")).toBe("0001:0002:0003:0004");
    // An IPv4-mapped address is its IPv4 address, not the shared ::ffff:0:0/96 prefix.
    expect(xff("::ffff:203.0.113.7")).toBe("203.0.113.7");
    expect(xff("::FFFF:cb00:7107")).toBe("203.0.113.7");

    // And through the limiter: one /64 spends one client window.
    let now = 0;
    const deps = memoryDeps(() => now);
    const write = (ip: string) =>
      new Request(BASE, { method: "POST", headers: { "x-forwarded-for": ip }, body: JSON.stringify({ op: "create", offer: SDP }) });
    for (let i = 0; i < RATE_CLIENT_WRITES; i++) {
      const ip = `2001:db8:1:2:${(i + 1).toString(16)}::1`;
      expect((await handleSignal(write(ip), deps)).status).toBe(200);
    }
    expect((await handleSignal(write("2001:db8:1:2::ffff"), deps)).status).toBe(429);
    expect((await handleSignal(write("2001:db8:1:3::1"), deps)).status).toBe(200);
  });

  test("IPv4 is the whole address; garbage is unknown", () => {
    expect(xff("203.0.113.7")).toBe("203.0.113.7");
    expect(xff(" 203.0.113.7 , 10.0.0.1")).toBe("203.0.113.7");
    expect(xff("203.0.113.8")).not.toBe(xff("203.0.113.7"));
    expect(clientKey(req({ "x-real-ip": "198.51.100.10" }))).toBe("198.51.100.10");
    expect(clientKey(req({}))).toBe("unknown");
    for (const g of [
      "256.1.1.1",
      "1.2.3",
      "1.2.3.4.5",
      "1:2:3:4:5:6:7:8:9",
      "1::2::3",
      "12345::1",
      "fe80::1%eth0",
      "::::",
      ":::",
      "x".repeat(100_000),
      "",
    ]) {
      expect(xff(g)).toBe("unknown");
    }
  });
});

describe("the Vercel entry", () => {
  const ENV_NAMES = ["KV_REST_API_URL", "KV_REST_API_TOKEN", "UPSTASH_REDIS_REST_URL", "UPSTASH_REDIS_REST_TOKEN"];

  async function withEnv(values: Record<string, string>, body: () => Promise<void>) {
    const saved = Object.fromEntries(ENV_NAMES.map((n) => [n, process.env[n]]));
    for (const n of ENV_NAMES) delete process.env[n];
    Object.assign(process.env, values);
    try {
      await body();
    } finally {
      for (const n of ENV_NAMES) {
        if (saved[n] === undefined) delete process.env[n];
        else process.env[n] = saved[n];
      }
    }
  }

  test("without its env vars, or with a malformed one, it answers a bare 500 and logs nothing", async () => {
    const { default: entry } = await import("../api/signal");
    const original = { warn: console.warn, log: console.log, error: console.error };
    const logged: unknown[] = [];
    console.warn = console.log = console.error = (...a: unknown[]) => void logged.push(a);
    try {
      const envs: Record<string, string>[] = [
        {},
        { KV_REST_API_URL: "https://example.upstash.io" },
        { KV_REST_API_TOKEN: "tok" },
        { KV_REST_API_URL: "https://example.upstash.io", KV_REST_API_TOKEN: "tok\n" },
        { KV_REST_API_URL: " https://example.upstash.io", KV_REST_API_TOKEN: "tok" },
      ];
      for (const env of envs) {
        await withEnv(env, async () => {
          const res = await entry.fetch(post({ op: "create", offer: SDP }));
          expect(res.status).toBe(500);
          expect(await res.text()).toBe("");
        });
      }
    } finally {
      Object.assign(console, original);
    }
    expect(logged).toEqual([]);
  });
});

/**
 * A fake Upstash REST server behind a stubbed global `fetch`: SET (EX, NX),
 * GET, DEL, and the fixed-window script by EVALSHA (INCRBY, then PEXPIRE on
 * the first count), which is all the function sends. Records each command.
 */
function fakeUpstash() {
  const data = new Map<string, string>();
  const commands: string[][] = [];
  const original = globalThis.fetch;
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    expect(url).toBe("https://example.upstash.io");
    const cmd = (JSON.parse(String(init.body)) as unknown[]).map(String);
    commands.push(cmd);
    const [name, ...args] = cmd;
    const reply = (result: unknown) => Response.json({ result });
    switch (name!.toLowerCase()) {
      case "set": {
        const [key, value, ...opts] = args as [string, string, ...string[]];
        if (opts.map((o) => o.toLowerCase()).includes("nx") && data.has(key)) return reply(null);
        data.set(key, value);
        return reply("OK");
      }
      case "get":
        return reply(data.get(args[0]!) ?? null);
      case "del": {
        let n = 0;
        for (const k of args) if (data.delete(k)) n++;
        return reply(n);
      }
      case "evalsha": {
        const [, numKeys, key, tokens] = args as [string, string, string, string];
        expect(numKeys).toBe("1");
        const next = Number(data.get(key) ?? 0) + 1;
        data.set(key, String(next));
        return reply([next, Number(tokens)]);
      }
      default:
        return Response.json({ error: `ERR unknown command ${name}` }, { status: 400 });
    }
  }) as typeof fetch;
  const redis = new Redis({ url: "https://example.upstash.io", token: "tok", ...REDIS_OPTIONS });
  return { redis, data, commands, restore: () => void (globalThis.fetch = original) };
}

describe("the Upstash adapters", () => {
  test("redisRoomStore sends SET with EX (and NX), GET and DEL, and keeps values verbatim", async () => {
    const fake = fakeUpstash();
    try {
      const store = redisRoomStore(fake.redis);
      expect(await store.set("k", "v", 300, true)).toBe(true);
      expect(await store.set("k", "w", 300, true)).toBe(false);
      expect(await store.set("n", "12345", 300, false)).toBe(true);
      expect(await store.get("k")).toBe("v");
      // A value that looks like JSON comes back as the same string.
      expect(await store.get("n")).toBe("12345");
      expect(await store.get("missing")).toBeNull();
      await store.del(["k", "n"]);
      expect(await store.get("k")).toBeNull();
      expect(fake.commands.map((c) => c[0]!.toLowerCase())).toEqual(["set", "set", "set", "get", "get", "get", "del", "get"]);
      expect(fake.commands[0]!.map((s) => s.toLowerCase())).toEqual(["set", "k", "v", "nx", "ex", "300"]);
      expect(fake.commands[2]!.map((s) => s.toLowerCase())).toEqual(["set", "n", "12345", "ex", "300"]);
      expect(fake.commands[6]).toEqual(["del", "k", "n"]);
    } finally {
      fake.restore();
    }
  });

  test("upstashLimiter makes one script call per count, in a fixed window under its prefix", async () => {
    const fake = fakeUpstash();
    try {
      const limiter = upstashLimiter(fake.redis, 2, 60, "rl:test");
      const t0 = Date.now();
      const results = [await limiter.limit("a"), await limiter.limit("a"), await limiter.limit("a")];
      expect(results.map((r) => r.success)).toEqual([true, true, false]);
      const bucket = Math.floor(t0 / 60_000);
      // The reset is the window's end (unless the test crossed a boundary).
      expect([(bucket + 1) * 60_000, (bucket + 2) * 60_000]).toContain(results[2]!.reset);
      expect(fake.commands.length).toBe(3);
      expect(fake.commands.every((c) => c[0]!.toLowerCase() === "evalsha")).toBe(true);
      expect(fake.commands[0]![3]).toMatch(/^rl:test:a:\d+$/);
      // Once refused, the instance's cache refuses without a call until the reset.
      expect((await limiter.limit("a")).success).toBe(false);
      expect(fake.commands.length).toBe(3);
      // Another identifier has its own window.
      expect((await limiter.limit("b")).success).toBe(true);
    } finally {
      fake.restore();
    }
  });

  test("redis_options_disable_retries", async () => {
    // A retried EVALSHA after a lost reply would count one limiter hit twice.
    expect(REDIS_OPTIONS.retry).toEqual({ retries: 0 });
    // And it reaches the client: a network failure is tried once, not retried.
    const original = globalThis.fetch;
    let fetches = 0;
    globalThis.fetch = (async () => {
      fetches++;
      throw new TypeError("Failed to fetch");
    }) as unknown as typeof fetch;
    try {
      const redis = new Redis({ url: "https://example.upstash.io", token: "tok", ...REDIS_OPTIONS });
      await expect(redisRoomStore(redis).get("k")).rejects.toThrow();
      expect(fetches).toBe(1);
      await expect(upstashLimiter(redis, 2, 60, "rl:z").limit("a")).rejects.toThrow();
      expect(fetches).toBe(2);
      // Why not `retry: false`: this library version reads it as one retry (ruling R-T5-2).
      fetches = 0;
      const once = new Redis({ url: "https://example.upstash.io", token: "tok", ...REDIS_OPTIONS, retry: false });
      await expect(redisRoomStore(once).get("k")).rejects.toThrow();
      expect(fetches).toBe(2);
    } finally {
      globalThis.fetch = original;
    }
  });

  test("a failing Upstash rejects; the handler answers a bare 500", async () => {
    const original = globalThis.fetch;
    globalThis.fetch = (async () => Response.json({ error: "ERR max requests limit exceeded" }, { status: 400 })) as unknown as typeof fetch;
    try {
      const redis = new Redis({ url: "https://example.upstash.io", token: "tok", ...REDIS_OPTIONS });
      await expect(redisRoomStore(redis).get("k")).rejects.toThrow();
      await expect(upstashLimiter(redis, 2, 60, "rl:x").limit("a")).rejects.toThrow();
      const limiter = upstashLimiter(redis, 2, 60, "rl:y");
      const deps: SignalDeps = {
        rooms: redisRoomStore(redis),
        limiters: { client: limiter, roomPolls: limiter, clientWrites: limiter, global: limiter },
      };
      const res = await handleSignal(post({ op: "create", offer: SDP }), deps);
      expect(res.status).toBe(500);
      expect(await res.text()).toBe("");
    } finally {
      globalThis.fetch = original;
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
  const status = async (req: Request, deps: SignalDeps) => (await handleSignal(req, deps)).status;

  type Name = keyof SignalDeps["limiters"];
  /** Memory deps on the clock `now`, recording every limiter call and every store call. */
  function spyDeps(now: () => number) {
    const inner = memoryDeps(now);
    const counts: Array<[Name, string]> = [];
    const calls: string[] = [];
    const limiters = {} as SignalDeps["limiters"];
    for (const name of Object.keys(inner.limiters) as Name[]) {
      limiters[name] = { limit: (k) => (counts.push([name, k]), calls.push("limit"), inner.limiters[name].limit(k)) };
    }
    const rooms: RoomStore = {
      set: (...a) => (calls.push("set"), inner.rooms.set(...a)),
      get: (k) => (calls.push("get"), inner.rooms.get(k)),
      del: (k) => (calls.push("del"), inner.rooms.del(k)),
    };
    const globalCount = () => counts.filter(([n]) => n === "global").length;
    return { deps: { rooms, limiters, now }, counts, calls, globalCount };
  }

  async function expect429(req: Request, deps: SignalDeps, retryAfterSeconds: number) {
    const res = await handleSignal(req, deps);
    expect(res.status).toBe(429);
    expect(res.headers.get("retry-after")).toBe(String(retryAfterSeconds));
    expect(await res.text()).toBe("");
  }

  test("the windows are the ruled ones", () => {
    expect([RATE_CLIENT_WRITES, RATE_CLIENT_WRITES_WINDOW_SECONDS]).toEqual([30, 60]);
    expect([RATE_ROOM_POLLS, RATE_ROOM_POLLS_WINDOW_SECONDS]).toEqual([200, ROOM_TTL_SECONDS]);
    expect([RATE_CLIENT_REQUESTS, RATE_CLIENT_REQUESTS_WINDOW_SECONDS]).toEqual([600, 600]);
    expect([RATE_GLOBAL_REQUESTS, RATE_GLOBAL_WINDOW_SECONDS]).toEqual([5000, 600]);
  });

  test("writes per client: a 429 at write 31, reset after 60 s; answers and closes count too", async () => {
    let now = 0;
    const deps = memoryDeps(() => now);
    for (let i = 0; i < RATE_CLIENT_WRITES - 2; i++) expect(await status(write("203.0.113.7"), deps)).toBe(200);
    // The 29th write is an answer, the 30th a close (unknown room: 404, still writes).
    expect(await status(write("203.0.113.7", { op: "answer", room: "ZZZZZZZZ", answer: SDP }), deps)).toBe(404);
    expect(await status(write("203.0.113.7", { op: "close", room: "ZZZZZZZZ", token: "A".repeat(22) }), deps)).toBe(404);
    await expect429(write("203.0.113.7"), deps, RATE_CLIENT_WRITES_WINDOW_SECONDS);
    // Polls are not writes.
    expect(await status(poll("203.0.113.7"), deps)).toBe(404);
    now += 59_000;
    // Retry-After is what is left of the window.
    await expect429(write("203.0.113.7"), deps, 1);
    now += 1_000;
    expect(await status(write("203.0.113.7"), deps)).toBe(200);
  });

  test("polls per room: a 429 at poll 201, reset after the room's 300 s", async () => {
    let now = 0;
    const deps = memoryDeps(() => now);
    // From many clients behind no single cap: the room's own counter binds.
    for (let i = 0; i < RATE_ROOM_POLLS; i++) expect(await status(poll(`10.0.0.${i % 250}`), deps)).toBe(404);
    await expect429(poll("10.0.1.1"), deps, RATE_ROOM_POLLS_WINDOW_SECONDS);
    now += 299_000;
    await expect429(poll("10.0.1.2"), deps, 1);
    now += 1_000;
    expect(await status(poll("10.0.1.3"), deps)).toBe(404);
  });

  test("windows are fixed to the clock: one opened mid-window ends at the boundary", async () => {
    // Upstash's fixed window: bucket = floor(now / window), as memoryLimiter.
    let now = 30_000;
    const deps = memoryDeps(() => now);
    for (let i = 0; i < RATE_CLIENT_WRITES; i++) expect(await status(write("203.0.113.7"), deps)).toBe(200);
    await expect429(write("203.0.113.7"), deps, 30);
    now = 59_999;
    await expect429(write("203.0.113.7"), deps, 1);
    now = 60_000;
    expect(await status(write("203.0.113.7"), deps)).toBe(200);
  });

  test("memoryLimiter: the cap, the reset, and per-key windows", async () => {
    let now = 120_500;
    const limiter = memoryLimiter(2, 60, () => now);
    expect(await limiter.limit("a")).toEqual({ success: true, reset: 180_000 });
    expect(await limiter.limit("a")).toEqual({ success: true, reset: 180_000 });
    expect(await limiter.limit("a")).toEqual({ success: false, reset: 180_000 });
    expect((await limiter.limit("b")).success).toBe(true);
    now = 180_000;
    expect(await limiter.limit("a")).toEqual({ success: true, reset: 240_000 });
  });

  test("a room polled to its cap blocks neither a second room nor another client's writes", async () => {
    const deps = memoryDeps(() => 0);
    const busy = await create(deps);
    const res = await handleSignal(write("198.51.100.1"), deps);
    const { room: other } = (await res.json()) as { room: string };
    for (let i = 0; i < RATE_ROOM_POLLS; i++) {
      expect(await status(new Request(`${BASE}?room=${busy}&want=answer`), deps)).toBe(204);
    }
    expect(await status(new Request(`${BASE}?room=${busy}&want=answer`), deps)).toBe(429);
    // The same NAT ("unknown") polls another room, and writes.
    expect(await status(new Request(`${BASE}?room=${other}&want=answer`), deps)).toBe(204);
    expect(await status(post({ op: "answer", room: other, answer: SDP }), deps)).toBe(204);
    expect(await status(write("198.51.100.1"), deps)).toBe(200);
  });

  test("all requests per client: a 429 at request 601, reset after 10 min", async () => {
    let now = 0;
    const deps = memoryDeps(() => now);
    for (let i = 0; i < RATE_CLIENT_REQUESTS; i++) {
      expect(await status(poll("203.0.113.7", roomCode(i)), deps)).toBe(404);
    }
    await expect429(poll("203.0.113.7", roomCode(9999)), deps, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    // Another client is unaffected.
    expect(await status(poll("203.0.113.8", roomCode(9999)), deps)).toBe(404);
    now += 599_000;
    await expect429(poll("203.0.113.7", roomCode(9998)), deps, 1);
    now += 1_000;
    expect(await status(poll("203.0.113.7", roomCode(9997)), deps)).toBe(404);
  });

  test("all requests, global: a 429 at request 5001, reset after 10 min", async () => {
    let now = 0;
    const deps = memoryDeps(() => now);
    for (let i = 0; i < RATE_GLOBAL_REQUESTS; i++) {
      expect(await status(poll(`10.${i >> 16}.${(i >> 8) & 255}.${i & 255}`, roomCode(i)), deps)).toBe(404);
    }
    await expect429(poll("192.0.2.1", roomCode(99_999)), deps, RATE_GLOBAL_WINDOW_SECONDS);
    now += 599_000;
    await expect429(poll("192.0.2.2", roomCode(99_998)), deps, 1);
    now += 1_000;
    expect(await status(poll("192.0.2.3", roomCode(99_997)), deps)).toBe(404);
  });

  test("one client is at most 12% of the global window", () => {
    expect(RATE_CLIENT_REQUESTS / RATE_GLOBAL_REQUESTS).toBeLessThanOrEqual(0.12);
    expect(RATE_CLIENT_REQUESTS_WINDOW_SECONDS).toBe(RATE_GLOBAL_WINDOW_SECONDS);
  });

  test("a client over its own cap costs one limiter call and never moves the global counter", async () => {
    const { deps, counts, calls, globalCount } = spyDeps(() => 0);
    for (let i = 0; i < RATE_CLIENT_REQUESTS; i++) await handleSignal(poll("203.0.113.7", roomCode(i)), deps);
    const global = globalCount();
    calls.length = 0;
    counts.length = 0;
    await expect429(write("203.0.113.7"), deps, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    await expect429(poll("203.0.113.7"), deps, RATE_CLIENT_REQUESTS_WINDOW_SECONDS);
    expect(calls).toEqual(["limit", "limit"]);
    expect(counts).toEqual([
      ["client", "203.0.113.7"],
      ["client", "203.0.113.7"],
    ]);
    expect(globalCount()).toBe(0);
    expect(global).toBe(RATE_CLIENT_REQUESTS);
  });

  test("a write over its cap costs two limiter calls and never moves the global counter", async () => {
    const { deps, calls, counts } = spyDeps(() => 0);
    for (let i = 0; i < RATE_CLIENT_WRITES; i++) await handleSignal(write("203.0.113.7"), deps);
    calls.length = 0;
    counts.length = 0;
    await expect429(write("203.0.113.7"), deps, RATE_CLIENT_WRITES_WINDOW_SECONDS);
    expect(calls).toEqual(["limit", "limit"]);
    expect(counts.map(([n]) => n)).toEqual(["client", "clientWrites"]);
  });

  test("a bad request is a 400 that makes no limiter or store call", async () => {
    const { deps, calls } = spyDeps(() => 0);
    expect(await status(poll("203.0.113.7", "short"), deps)).toBe(400);
    expect(await status(poll("203.0.113.7", "ILOU0000"), deps)).toBe(400);
    expect(await status(new Request(`${BASE}?room=ZZZZZZZZ&want=all`), deps)).toBe(400);
    expect(await status(write("203.0.113.7", { op: "nope" }), deps)).toBe(400);
    expect(await status(write("203.0.113.7", { op: "answer", room: "bad", answer: SDP }), deps)).toBe(400);
    expect(await status(write("203.0.113.7", { op: "close", room: "ZZZZZZZZ" }), deps)).toBe(400);
    expect(calls).toEqual([]);
  });

  test("two clients get independent windows; the first forwarded entry names the client", async () => {
    const deps = memoryDeps(() => 0);
    for (let i = 0; i < RATE_CLIENT_WRITES; i++) await handleSignal(write("203.0.113.7, 10.0.0.1"), deps);
    expect(await status(write("203.0.113.7"), deps)).toBe(429);
    expect(await status(write("198.51.100.9, 203.0.113.7"), deps)).toBe(200);
    expect(await status(write("2001:db8::1"), deps)).toBe(200);
    // x-real-ip when there is no x-forwarded-for.
    const real = new Request(BASE, {
      method: "POST",
      headers: { "x-real-ip": "198.51.100.10" },
      body: JSON.stringify({ op: "create", offer: SDP }),
    });
    expect(await status(real, deps)).toBe(200);
  });

  test("garbage forwarded headers neither throw nor make big keys", async () => {
    const { deps, counts } = spyDeps(() => 0);
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
      expect(await status(poll(g, "ZZZZZZZZ", { "x-real-ip": g }), deps)).toBe(404);
    }
    expect(counts.length).toBeGreaterThan(0);
    for (const [, k] of counts) {
      expect(k.length).toBeLessThanOrEqual(39);
      expect(/^[A-Za-z0-9:.]+$/.test(k)).toBe(true);
    }
    expect(new Set(counts.map(([n, k]) => `${n}:${k}`))).toEqual(
      new Set(["client:unknown", "roomPolls:ZZZZZZZZ", "global:all"]),
    );
  });

  test("the dev two-device flow stays far under every cap", async () => {
    // Two devices on one key ("unknown"), one room each way, the host
    // polling 1/s for the full 120 s before the answer, then closing.
    const { deps, counts } = spyDeps(() => 0);
    let requests = 0;
    for (let way = 0; way < 2; way++) {
      const { room, token } = await createRoom(deps);
      requests++;
      for (let s = 0; s < 120; s++, requests++) await handleSignal(get(`room=${room}&want=answer`), deps);
      expect(await status(get(`room=${room}&want=offer`), deps)).toBe(200);
      expect(await status(post({ op: "answer", room, answer: SDP }), deps)).toBe(204);
      requests += 2;
      expect(await status(get(`room=${room}&want=answer`), deps)).toBe(200);
      requests++;
      expect(await status(post({ op: "close", room, token }), deps)).toBe(204);
      requests++;
      expect(counts.filter(([n, k]) => n === "roomPolls" && k === room).length).toBe(122);
    }
    const per = (name: Name) => counts.filter(([n]) => n === name).length;
    expect(requests).toBe(250);
    expect(per("client")).toBe(250);
    expect(per("global")).toBe(250);
    expect(per("clientWrites")).toBe(6);
  });
});

describe("the dev middleware's body read", () => {
  test("stops reading past MAX_BODY_BYTES", async () => {
    const { PassThrough } = await import("node:stream");
    const { readCapped } = await import("../vite.config.ts");
    const small = new PassThrough();
    const pending = readCapped(small as never);
    small.end(Buffer.from("{}"));
    expect(String(await pending)).toBe("{}");

    const big = new PassThrough();
    const capped = readCapped(big as never);
    big.write(Buffer.alloc(MAX_BODY_BYTES));
    big.write(Buffer.alloc(1));
    expect(await capped).toBeNull();
    // Nothing reads on: the stream is paused with no data listener left.
    expect(big.isPaused()).toBe(true);
    expect(big.listenerCount("data")).toBe(0);
  });
});

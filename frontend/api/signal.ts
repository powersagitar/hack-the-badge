/**
 * The Vercel function serving `/api/signal` (Milestone 10 Task 10;
 * Milestone 11 Task 9): the WebRTC signaling mailbox of
 * `../src/radio/signal-handler.ts`, over the project's Upstash Redis
 * (connected through the Vercel Marketplace, which sets `KV_REST_API_URL`
 * and `KV_REST_API_TOKEN`), through `@upstash/redis` and its fixed-window
 * limiter `@upstash/ratelimit`. Both live here, not in the handler, so
 * the browser bundle never carries them.
 *
 * The entry format is the `fetch` Web Standard export, which Vercel's
 * Node.js runtime calls for a file under `/api` in a project without a
 * framework router (here Vite, whose project root is `frontend/`): a
 * default export `{ fetch(request: Request) }` returning a `Response`, for
 * every method. See "Functions API Reference", section "`fetch` Web
 * Standard": https://vercel.com/docs/functions/functions-api-reference
 * (checked 2026-11-02).
 *
 * The clients are built once per instance, on the first request, by
 * `Redis.fromEnv`, which reads `UPSTASH_REDIS_REST_URL` or else
 * `KV_REST_API_URL`, and the same for the token. The same two values are
 * checked first: if either is missing, or has the leading or trailing
 * whitespace or line break that `@upstash/redis` would warn about on the
 * console, the answer is a bare 500 and nothing is built. Nothing is ever
 * logged: the clients' telemetry, analytics and latency logging are off.
 */
import { Ratelimit } from "@upstash/ratelimit";
import { Redis } from "@upstash/redis";
import {
  type Limiter,
  RATE_CLIENT_REQUESTS,
  RATE_CLIENT_REQUESTS_WINDOW_SECONDS,
  RATE_CLIENT_WRITES,
  RATE_CLIENT_WRITES_WINDOW_SECONDS,
  RATE_GLOBAL_REQUESTS,
  RATE_GLOBAL_WINDOW_SECONDS,
  RATE_ROOM_POLLS,
  RATE_ROOM_POLLS_WINDOW_SECONDS,
  type RoomStore,
  type SignalDeps,
  handleSignal,
} from "../src/radio/signal-handler.js";

/** The rooms over Upstash: `SET … EX ttl [NX]`, `GET`, `DEL`. Values are stored and read back verbatim. */
export function redisRoomStore(redis: Redis): RoomStore {
  return {
    async set(key, value, ttlSeconds, onlyIfAbsent) {
      const reply = onlyIfAbsent
        ? await redis.set(key, value, { nx: true, ex: ttlSeconds })
        : await redis.set(key, value, { ex: ttlSeconds });
      // "OK" when set; null when NX found the key.
      return reply === "OK";
    },
    async get(key) {
      const value = await redis.get<unknown>(key);
      return typeof value === "string" ? value : null;
    },
    async del(keys) {
      await redis.del(...keys);
    },
  };
}

/**
 * One `Ratelimit.fixedWindow` of `tokens` per `windowSeconds`, keyed
 * `<prefix>:<key>:<window number>`: one script call (`EVALSHA`) per
 * count, none for a key this instance already saw refused in the current
 * window (the library's in-memory cache). `timeout: 0`: a slow store is
 * waited for, never waved through (ruling R-T9-2).
 */
export function upstashLimiter(redis: Redis, tokens: number, windowSeconds: number, prefix: string): Limiter {
  const ratelimit = new Ratelimit({
    redis,
    limiter: Ratelimit.fixedWindow(tokens, `${windowSeconds} s`),
    prefix,
    timeout: 0,
    analytics: false,
    enableTelemetry: false,
  });
  return {
    async limit(key) {
      const { success, reset } = await ratelimit.limit(key);
      return { success, reset };
    },
  };
}

/** The four limiters at the ruled caps (R-T10-8), and the rooms, over one Redis client. */
export function upstashDeps(redis: Redis): SignalDeps {
  return {
    rooms: redisRoomStore(redis),
    limiters: {
      client: upstashLimiter(redis, RATE_CLIENT_REQUESTS, RATE_CLIENT_REQUESTS_WINDOW_SECONDS, "rl:client"),
      roomPolls: upstashLimiter(redis, RATE_ROOM_POLLS, RATE_ROOM_POLLS_WINDOW_SECONDS, "rl:room"),
      clientWrites: upstashLimiter(redis, RATE_CLIENT_WRITES, RATE_CLIENT_WRITES_WINDOW_SECONDS, "rl:writes"),
      global: upstashLimiter(redis, RATE_GLOBAL_REQUESTS, RATE_GLOBAL_WINDOW_SECONDS, "rl:global"),
    },
  };
}

type Env = Record<string, string | undefined>;

function env(): Env {
  // No Node types in this project: reach `process.env` without them.
  return (globalThis as { process?: { env?: Env } }).process?.env ?? {};
}

/** A credential `Redis` would take without a console warning: present, with no surrounding whitespace or line break. */
const usable = (s: string | undefined): s is string => !!s && s.trim() === s && !/[\r\n]/.test(s);

/**
 * Values verbatim (no JSON parsing of what `GET` returns), no telemetry
 * headers, and one REST request per command: the handler's calls are
 * sequential, so auto-pipelining would batch nothing.
 */
export const REDIS_OPTIONS = { automaticDeserialization: false, enableTelemetry: false, enableAutoPipelining: false };

let deps: SignalDeps | null = null;

const serverError = () => new Response(null, { status: 500, headers: { "Cache-Control": "no-store" } });

export default {
  async fetch(request: Request): Promise<Response> {
    if (!deps) {
      const e = env();
      // What `Redis.fromEnv` will read, in its order.
      if (!usable(e.UPSTASH_REDIS_REST_URL || e.KV_REST_API_URL)) return serverError();
      if (!usable(e.UPSTASH_REDIS_REST_TOKEN || e.KV_REST_API_TOKEN)) return serverError();
      try {
        deps = upstashDeps(Redis.fromEnv(REDIS_OPTIONS));
      } catch {
        return serverError();
      }
    }
    return handleSignal(request, deps);
  },
};

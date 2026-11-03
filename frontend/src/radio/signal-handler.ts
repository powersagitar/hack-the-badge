/**
 * The signaling handler (Milestone 10 Task 10): a two-step mailbox through
 * which two browsers swap one WebRTC offer and one answer, and nothing
 * else. Radio traffic never passes through it: once the data channel is
 * open, the server is out of the loop. It is pure (a Web `Request` in, a
 * `Response` out, over an injected {@link SignalStore}), so `bun test`, the
 * Vite dev middleware (`vite.config.ts`, a {@link MemoryStore}) and the
 * Vercel function (`api/signal.ts`, Upstash Redis) share it.
 *
 * Protocol (JSON):
 * - `POST {op:"create", offer}` → 200 `{room}`: a new room holding `offer`.
 * - `GET ?room=R&want=offer` → 200 `{offer}`, or 404.
 * - `POST {op:"answer", room, answer}` → 204; 409 if already answered,
 *   404 if the room is unknown or expired.
 * - `GET ?room=R&want=answer` → 200 `{answer}`, 204 while pending, or 404
 *   if the room is unknown or expired.
 *
 * Room codes are {@link ROOM_CODE_LENGTH} characters of Crockford base32,
 * from `crypto.getRandomValues`. Every key expires {@link ROOM_TTL_SECONDS}
 * after it is written. An SDP is at most {@link MAX_SDP_CHARS}; a body over
 * {@link MAX_BODY_BYTES} is refused unread past the cap. Bad JSON, an
 * unknown op, a bad room format or an oversized SDP or body give 400; a
 * method other than GET and POST gives 405; a store failure gives a bare
 * 500. Nothing is logged, ever: SDPs carry the peers' addresses.
 *
 * Rate limit (ruling R-T12-RL; R-T10-8..10): before any other store
 * work, each request counts against its client's fixed 60 s window (at
 * most {@link RATE_PER_CLIENT_PER_MIN}) and then a global fixed 1 h
 * window (at most {@link RATE_GLOBAL_PER_HOUR}), through
 * {@link SignalStore.incr} under the `rl:` namespace. Over either cap:
 * 429 with a `Retry-After`, and no further store call. A window starts at
 * its first request and ends when its counter key expires.
 *
 * This module imports nothing, so the Vercel function, which runs it as
 * Node ESM, needs no bundling to resolve it.
 */

/** Crockford base32: digits and capitals without I, L, O and U. */
export const CROCKFORD_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export const ROOM_CODE_LENGTH = 8;
export const ROOM_TTL_SECONDS = 300;
/** The longest SDP accepted (16 KiB). A data-channel-only SDP is about 1 KiB. */
export const MAX_SDP_CHARS = 16 * 1024;
/** The longest request body: one SDP, JSON-escaped (CRLFs double), plus the envelope. */
export const MAX_BODY_BYTES = 2 * MAX_SDP_CHARS + 1024;

/**
 * Requests per client per 60 s window: a hackathon NAT puts many badges
 * behind one IP, and each live pairing polls about once a second.
 */
export const RATE_PER_CLIENT_PER_MIN = 240;
export const RATE_CLIENT_WINDOW_SECONDS = 60;
/** Requests per 1 h window over all clients: the Upstash allowance's guard. */
export const RATE_GLOBAL_PER_HOUR = 6000;
export const RATE_GLOBAL_WINDOW_SECONDS = 3600;
/** The global counter's key; client keys are `rl:client:<ip>`. */
const GLOBAL_RATE_KEY = "rl:global";
/** An IPv4 or IPv6 address's characters (IPv4-mapped IPv6 is at most 45). */
const CLIENT_IP = /^[0-9A-Fa-f:.]{1,45}$/;

const ROOM_CODE = new RegExp(`^[${CROCKFORD_ALPHABET}]{${ROOM_CODE_LENGTH}}$`);
/** Tries at a free room code; 32^8 codes make a second try already rare. */
const CREATE_ATTEMPTS = 8;

/** The key-value store behind the handler. */
export interface SignalStore {
  /** `SET key value EX ttl [NX]`: `false` when `onlyIfAbsent` and the key exists. */
  set(key: string, value: string, ttlSeconds: number, onlyIfAbsent: boolean): Promise<boolean>;
  get(key: string): Promise<string | null>;
  /**
   * Increments the integer at `key` atomically (from 0 if absent) and
   * returns the new value; a new key expires after `ttlSeconds`, an
   * existing one keeps its expiry.
   */
  incr(key: string, ttlSeconds: number): Promise<number>;
}

/** An in-memory store, for tests and the dev server. `now` is injectable (ms). */
export class MemoryStore implements SignalStore {
  private readonly entries = new Map<string, { value: string; expiresAt: number }>();

  constructor(private readonly now: () => number = () => Date.now()) {}

  async set(key: string, value: string, ttlSeconds: number, onlyIfAbsent: boolean): Promise<boolean> {
    this.prune();
    if (onlyIfAbsent && this.entries.has(key)) return false;
    this.entries.set(key, { value, expiresAt: this.now() + ttlSeconds * 1000 });
    return true;
  }

  async get(key: string): Promise<string | null> {
    this.prune();
    return this.entries.get(key)?.value ?? null;
  }

  async incr(key: string, ttlSeconds: number): Promise<number> {
    this.prune();
    const entry = this.entries.get(key);
    if (!entry) {
      this.entries.set(key, { value: "1", expiresAt: this.now() + ttlSeconds * 1000 });
      return 1;
    }
    const next = Number(entry.value) + 1;
    entry.value = String(next);
    return next;
  }

  private prune(): void {
    const now = this.now();
    for (const [key, { expiresAt }] of this.entries) if (expiresAt <= now) this.entries.delete(key);
  }
}

type FetchFn = (input: string, init?: RequestInit) => Promise<Response>;

/**
 * Upstash Redis through its REST API, over `fetch`, so no client library
 * is needed (https://upstash.com/docs/redis/features/restapi, checked
 * 2026-11-02): `POST <url>` with a JSON command array and a bearer token
 * answers `{result}` or `{error}`; `POST <url>/pipeline` with a
 * two-dimensional array of commands answers an array of those, in order.
 */
export class UpstashStore implements SignalStore {
  constructor(
    private readonly url: string,
    private readonly token: string,
    // Wrapped: a bare `fetch` called as this object's method throws "Illegal invocation" in some runtimes.
    private readonly fetchFn: FetchFn = (input, init) => fetch(input, init),
  ) {}

  /**
   * `INCR` then `EXPIRE key ttl NX` in one pipeline request. `INCR` is
   * atomic on its own; `NX` sets the expiry only on the key's first
   * count, so the window stays fixed.
   */
  async incr(key: string, ttlSeconds: number): Promise<number> {
    const res = await this.post(`${this.url}/pipeline`, [
      ["INCR", key],
      ["EXPIRE", key, String(ttlSeconds), "NX"],
    ]);
    if (!Array.isArray(res) || res.length !== 2) throw new Error("store error");
    for (const r of res) {
      if (r === null || typeof r !== "object" || "error" in r) throw new Error("store error");
    }
    const count = (res[0] as { result?: unknown }).result;
    if (typeof count !== "number" || !Number.isSafeInteger(count)) throw new Error("store error");
    return count;
  }

  async set(key: string, value: string, ttlSeconds: number, onlyIfAbsent: boolean): Promise<boolean> {
    const command = ["SET", key, value, "EX", String(ttlSeconds)];
    if (onlyIfAbsent) command.push("NX");
    // "OK" when set; null when NX found the key.
    return (await this.command(command)) === "OK";
  }

  async get(key: string): Promise<string | null> {
    const result = await this.command(["GET", key]);
    return typeof result === "string" ? result : null;
  }

  private async command(command: string[]): Promise<unknown> {
    const body = (await this.post(this.url, command)) as { result?: unknown; error?: unknown } | null;
    if (body === null || typeof body !== "object" || "error" in body) throw new Error("store error");
    return body.result ?? null;
  }

  private async post(url: string, payload: unknown): Promise<unknown> {
    const res = await this.fetchFn(url, {
      method: "POST",
      headers: { Authorization: `Bearer ${this.token}`, "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error("store unavailable");
    return res.json();
  }
}

export function isRoomCode(s: unknown): s is string {
  return typeof s === "string" && ROOM_CODE.test(s);
}

/** A new room code. 256 is a multiple of 32, so `byte & 31` is unbiased. */
export function newRoomCode(): string {
  const bytes = new Uint8Array(ROOM_CODE_LENGTH);
  crypto.getRandomValues(bytes);
  let code = "";
  for (const b of bytes) code += CROCKFORD_ALPHABET[b & 31];
  return code;
}

const offerKey = (room: string) => `htn-signal:${room}:offer`;
const answerKey = (room: string) => `htn-signal:${room}:answer`;

const NO_STORE = { "Cache-Control": "no-store" };

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { ...NO_STORE, "Content-Type": "application/json" } });
const empty = (status: number) => new Response(null, { status, headers: NO_STORE });

class BadRequest extends Error {}

function isSdp(s: unknown): s is string {
  return typeof s === "string" && s.length > 0 && s.length <= MAX_SDP_CHARS;
}

/** The body as text, refusing (400) past {@link MAX_BODY_BYTES} without reading the rest. */
async function readBody(req: Request): Promise<string> {
  const declared = Number(req.headers.get("content-length"));
  if (Number.isFinite(declared) && declared > MAX_BODY_BYTES) throw new BadRequest();
  if (!req.body) return "";
  const reader = req.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > MAX_BODY_BYTES) {
      void reader.cancel().catch(() => {});
      throw new BadRequest();
    }
    chunks.push(value);
  }
  const all = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    all.set(c, at);
    at += c.byteLength;
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(all);
  } catch {
    throw new BadRequest();
  }
}

async function handleGet(url: URL, store: SignalStore): Promise<Response> {
  const room = url.searchParams.get("room");
  const want = url.searchParams.get("want");
  if (!isRoomCode(room) || (want !== "offer" && want !== "answer")) return empty(400);
  if (want === "offer") {
    const offer = await store.get(offerKey(room));
    return offer === null ? empty(404) : json(200, { offer });
  }
  const answer = await store.get(answerKey(room));
  if (answer !== null) return json(200, { answer });
  // Pending only while the room exists; an expired room is gone for good.
  return (await store.get(offerKey(room))) === null ? empty(404) : empty(204);
}

async function handlePost(req: Request, store: SignalStore): Promise<Response> {
  let body: unknown;
  try {
    body = JSON.parse(await readBody(req));
  } catch {
    return empty(400);
  }
  if (typeof body !== "object" || body === null || Array.isArray(body)) return empty(400);
  const { op, offer, room, answer } = body as Record<string, unknown>;
  if (op === "create") {
    if (!isSdp(offer)) return empty(400);
    for (let i = 0; i < CREATE_ATTEMPTS; i++) {
      const code = newRoomCode();
      if (await store.set(offerKey(code), offer, ROOM_TTL_SECONDS, true)) return json(200, { room: code });
    }
    return empty(503);
  }
  if (op === "answer") {
    if (!isRoomCode(room) || !isSdp(answer)) return empty(400);
    if ((await store.get(offerKey(room))) === null) return empty(404);
    const stored = await store.set(answerKey(room), answer, ROOM_TTL_SECONDS, true);
    return stored ? empty(204) : empty(409);
  }
  return empty(400);
}

/**
 * The client's rate key: the first `x-forwarded-for` entry, else
 * `x-real-ip`, else `unknown`. A value that is not an IP address's
 * characters (at most 45) counts as absent, so header garbage never
 * reaches a key.
 */
export function clientRateKey(req: Request): string {
  for (const header of ["x-forwarded-for", "x-real-ip"]) {
    const value = req.headers.get(header);
    if (value === null) continue;
    const first = value.slice(0, 64).split(",")[0]!.trim();
    if (CLIENT_IP.test(first)) return `rl:client:${first}`;
  }
  return "rl:client:unknown";
}

function tooMany(retryAfterSeconds: number): Response {
  return new Response(null, {
    status: 429,
    headers: { ...NO_STORE, "Retry-After": String(retryAfterSeconds) },
  });
}

/**
 * Counts the request against both caps: a 429 if either is over, else
 * `null`. Over the client cap, the global counter is not touched. The
 * `Retry-After` is the window's length, an upper bound on its rest.
 */
async function rateLimit(req: Request, store: SignalStore): Promise<Response | null> {
  if ((await store.incr(clientRateKey(req), RATE_CLIENT_WINDOW_SECONDS)) > RATE_PER_CLIENT_PER_MIN) {
    return tooMany(RATE_CLIENT_WINDOW_SECONDS);
  }
  if ((await store.incr(GLOBAL_RATE_KEY, RATE_GLOBAL_WINDOW_SECONDS)) > RATE_GLOBAL_PER_HOUR) {
    return tooMany(RATE_GLOBAL_WINDOW_SECONDS);
  }
  return null;
}

/** Handles one signaling request. Never throws, and never logs. */
export async function handleSignal(req: Request, store: SignalStore): Promise<Response> {
  try {
    const limited = await rateLimit(req, store);
    if (limited) return limited;
    if (req.method === "GET") return await handleGet(new URL(req.url), store);
    if (req.method === "POST") return await handlePost(req, store);
    return new Response(null, { status: 405, headers: { ...NO_STORE, Allow: "GET, POST" } });
  } catch (e) {
    if (e instanceof BadRequest) return empty(400);
    return empty(500);
  }
}

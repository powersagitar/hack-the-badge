/**
 * The signaling handler (Milestone 10 Task 10; Milestone 11 Task 9): a
 * two-step mailbox through which two browsers swap one WebRTC offer and
 * one answer, and nothing else. Radio traffic never passes through it:
 * once the data channel is open, the server is out of the loop. It is
 * pure (a Web `Request` in, a `Response` out, over injected
 * {@link SignalDeps}), so `bun test`, the Vite dev middleware
 * (`vite.config.ts`, {@link memoryDeps}) and the Vercel function
 * (`api/signal.ts`, Upstash Redis and `@upstash/ratelimit`) share it.
 *
 * Protocol (JSON):
 * - `POST {op:"create", offer}` → 200 `{room, token}`: a new room holding
 *   `offer`; `token` ({@link TOKEN_LENGTH} base64url characters) is the
 *   host's proof of ownership, for `close`.
 * - `GET ?room=R&want=offer` → 200 `{offer}`, or 404.
 * - `POST {op:"answer", room, answer}` → 204; 409 if already answered,
 *   404 if the room is unknown or expired.
 * - `GET ?room=R&want=answer` → 200 `{answer}`, 204 while pending, or 404
 *   if the room is unknown or expired.
 * - `POST {op:"close", room, token}` → 204: the room's keys are deleted;
 *   404 if the room is unknown or expired, or the token is not its own.
 *
 * Room codes are {@link ROOM_CODE_LENGTH} characters of Crockford base32,
 * from `crypto.getRandomValues`. Every key expires {@link ROOM_TTL_SECONDS}
 * after it is written. An SDP is at most {@link MAX_SDP_CHARS}; a body over
 * {@link MAX_BODY_BYTES} is refused unread past the cap. Requests are
 * checked with `valibot` schemas: bad JSON, an unknown op, a bad room or
 * token format, or an oversized SDP or body give 400; a method other than
 * GET and POST gives 405; a store or limiter failure (a thrown error or a
 * rejection, a spent quota included) gives a bare 500. Nothing is logged,
 * ever: SDPs carry the peers' addresses.
 *
 * Rate limit (rulings R-T12-RL4; R-T10-8..10; R-T9-1, R-T9-2): a request
 * is validated first, with no store or limiter call, so a 400 counts
 * nothing. A valid one then counts, before any store work, against four
 * fixed windows ({@link Limiter}s), in the order {@link rateLimit} gives:
 * all requests per client (600 / 10 min); polls per room (200 / 300 s) or
 * writes per client (30 / 60 s); all requests (5000 / 10 min). Over any
 * cap: 429, empty, with `Retry-After` the whole seconds left until that
 * limiter's `reset`, and no further call. A window is fixed to the clock
 * (`floor(now / window)`, as `Ratelimit.fixedWindow` buckets it), not to
 * its first request.
 *
 * This module imports only `valibot`; the Upstash clients stay in
 * `api/signal.ts`, so the browser bundle (which takes the room-code
 * helpers from here) never carries them.
 */
import * as v from "valibot";

/** Crockford base32: digits and capitals without I, L, O and U. */
export const CROCKFORD_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export const ROOM_CODE_LENGTH = 8;
export const ROOM_TTL_SECONDS = 300;
/** The longest SDP accepted (16 KiB). A data-channel-only SDP is about 1 KiB. */
export const MAX_SDP_CHARS = 16 * 1024;
/** The longest request body: one SDP, JSON-escaped (CRLFs double), plus the envelope. */
export const MAX_BODY_BYTES = 2 * MAX_SDP_CHARS + 1024;
/** A room's token: 16 random bytes (128 bits) in unpadded base64url. */
export const TOKEN_LENGTH = 22;

/**
 * Rate caps (ruling R-T12-RL4, R-T10-8), each a fixed window counted by
 * one {@link Limiter}. A venue NAT puts many browsers behind one IP, each
 * waiting one polling about once a second, so polls are capped per room,
 * not per IP.
 *
 * Writes (`create`, `answer`, `close`) per client per 60 s.
 */
export const RATE_CLIENT_WRITES = 30;
export const RATE_CLIENT_WRITES_WINDOW_SECONDS = 60;
/** Polls (`GET want=offer|answer`) per room, per 300 s window (a room's life). */
export const RATE_ROOM_POLLS = 200;
export const RATE_ROOM_POLLS_WINDOW_SECONDS = 300;
/** All requests per client per 10 min: 12% of the global window. */
export const RATE_CLIENT_REQUESTS = 600;
export const RATE_CLIENT_REQUESTS_WINDOW_SECONDS = 600;
/** All requests per 10 min over all clients: the Upstash allowance's guard. */
export const RATE_GLOBAL_REQUESTS = 5000;
export const RATE_GLOBAL_WINDOW_SECONDS = 600;
/** The global limiter's one identifier. */
const GLOBAL_KEY = "all";

const ROOM_CODE = new RegExp(`^[${CROCKFORD_ALPHABET}]{${ROOM_CODE_LENGTH}}$`);
const TOKEN = new RegExp(`^[A-Za-z0-9_-]{${TOKEN_LENGTH}}$`);
/** Tries at a free room code; 32^8 codes make a second try already rare. */
const CREATE_ATTEMPTS = 8;

/** The rooms' key-value store. */
export interface RoomStore {
  /** `SET key value EX ttl [NX]`: `false` when `onlyIfAbsent` and the key exists. */
  set(key: string, value: string, ttlSeconds: number, onlyIfAbsent: boolean): Promise<boolean>;
  get(key: string): Promise<string | null>;
  /** `DEL keys...`; absent keys are ignored. */
  del(keys: string[]): Promise<void>;
}

/** One fixed-window rate limit. `reset` is when the key's window ends (ms since the epoch). */
export interface Limiter {
  limit(key: string): Promise<{ success: boolean; reset: number }>;
}

/** What {@link handleSignal} runs over. `now` (ms) is for `Retry-After`; it defaults to `Date.now`. */
export interface SignalDeps {
  rooms: RoomStore;
  limiters: { client: Limiter; roomPolls: Limiter; clientWrites: Limiter; global: Limiter };
  now?: () => number;
}

/** An in-memory store, for tests and the dev server. `now` is injectable (ms). */
export class MemoryStore implements RoomStore {
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

  async del(keys: string[]): Promise<void> {
    for (const key of keys) this.entries.delete(key);
  }

  private prune(): void {
    const now = this.now();
    for (const [key, { expiresAt }] of this.entries) if (expiresAt <= now) this.entries.delete(key);
  }
}

/**
 * An in-memory fixed-window limiter, for tests and the dev server, with
 * `Ratelimit.fixedWindow`'s windows: the bucket is `floor(now / window)`
 * and `reset` its end. `now` is injectable (ms).
 */
export function memoryLimiter(tokens: number, windowSeconds: number, now: () => number = () => Date.now()): Limiter {
  const windowMs = windowSeconds * 1000;
  const counts = new Map<string, { bucket: number; count: number }>();
  return {
    async limit(key) {
      const bucket = Math.floor(now() / windowMs);
      for (const [k, entry] of counts) if (entry.bucket !== bucket) counts.delete(k);
      const entry = counts.get(key) ?? { bucket, count: 0 };
      entry.count++;
      counts.set(key, entry);
      return { success: entry.count <= tokens, reset: (bucket + 1) * windowMs };
    },
  };
}

/** The dev server's and the tests' deps: a {@link MemoryStore} and four {@link memoryLimiter}s at the ruled caps. */
export function memoryDeps(now: () => number = () => Date.now(), rooms: RoomStore = new MemoryStore(now)): SignalDeps {
  return {
    rooms,
    limiters: {
      client: memoryLimiter(RATE_CLIENT_REQUESTS, RATE_CLIENT_REQUESTS_WINDOW_SECONDS, now),
      roomPolls: memoryLimiter(RATE_ROOM_POLLS, RATE_ROOM_POLLS_WINDOW_SECONDS, now),
      clientWrites: memoryLimiter(RATE_CLIENT_WRITES, RATE_CLIENT_WRITES_WINDOW_SECONDS, now),
      global: memoryLimiter(RATE_GLOBAL_REQUESTS, RATE_GLOBAL_WINDOW_SECONDS, now),
    },
    now,
  };
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

/** A new room token: 16 random bytes as {@link TOKEN_LENGTH} base64url characters, unpadded. */
export function newToken(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/** Equal strings, compared in time that depends only on their lengths. */
function sameToken(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

const offerKey = (room: string) => `htn-signal:${room}:offer`;
const answerKey = (room: string) => `htn-signal:${room}:answer`;
const tokenKey = (room: string) => `htn-signal:${room}:token`;

const NO_STORE = { "Cache-Control": "no-store" };

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { ...NO_STORE, "Content-Type": "application/json" } });
const empty = (status: number) => new Response(null, { status, headers: NO_STORE });

class BadRequest extends Error {}

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

const RoomSchema = v.pipe(v.string(), v.regex(ROOM_CODE));
const SdpSchema = v.pipe(v.string(), v.minLength(1), v.maxLength(MAX_SDP_CHARS));
const PollSchema = v.object({ room: RoomSchema, want: v.picklist(["offer", "answer"]) });
/** A POST body; unknown fields are ignored (the body cap bounds them). */
const BodySchema = v.variant("op", [
  v.object({ op: v.literal("create"), offer: SdpSchema }),
  v.object({ op: v.literal("answer"), room: RoomSchema, answer: SdpSchema }),
  v.object({ op: v.literal("close"), room: RoomSchema, token: v.pipe(v.string(), v.regex(TOKEN)) }),
]);

/** A request that passed validation: what the rate limit counts and the handler serves. */
type Parsed = ({ op: "poll" } & v.InferOutput<typeof PollSchema>) | v.InferOutput<typeof BodySchema>;

/** Validates a request without touching the store: the parsed request, or a 400/405. */
async function parse(req: Request): Promise<Parsed | Response> {
  if (req.method === "GET") {
    const url = new URL(req.url);
    const query = v.safeParse(PollSchema, { room: url.searchParams.get("room"), want: url.searchParams.get("want") });
    return query.success ? { op: "poll", ...query.output } : empty(400);
  }
  if (req.method !== "POST") return new Response(null, { status: 405, headers: { ...NO_STORE, Allow: "GET, POST" } });
  let body: unknown;
  try {
    body = JSON.parse(await readBody(req));
  } catch {
    return empty(400);
  }
  const parsed = v.safeParse(BodySchema, body);
  return parsed.success ? parsed.output : empty(400);
}

async function serve(p: Parsed, rooms: RoomStore): Promise<Response> {
  switch (p.op) {
    case "poll": {
      if (p.want === "offer") {
        const offer = await rooms.get(offerKey(p.room));
        return offer === null ? empty(404) : json(200, { offer });
      }
      const answer = await rooms.get(answerKey(p.room));
      if (answer !== null) return json(200, { answer });
      // Pending only while the room exists; an expired room is gone for good.
      return (await rooms.get(offerKey(p.room))) === null ? empty(404) : empty(204);
    }
    case "create": {
      for (let i = 0; i < CREATE_ATTEMPTS; i++) {
        const room = newRoomCode();
        if (await rooms.set(offerKey(room), p.offer, ROOM_TTL_SECONDS, true)) {
          const token = newToken();
          await rooms.set(tokenKey(room), token, ROOM_TTL_SECONDS, false);
          return json(200, { room, token });
        }
      }
      return empty(503);
    }
    case "answer": {
      if ((await rooms.get(offerKey(p.room))) === null) return empty(404);
      const stored = await rooms.set(answerKey(p.room), p.answer, ROOM_TTL_SECONDS, true);
      return stored ? empty(204) : empty(409);
    }
    case "close": {
      const token = await rooms.get(tokenKey(p.room));
      if (token === null || !sameToken(token, p.token)) return empty(404);
      await rooms.del([offerKey(p.room), answerKey(p.room), tokenKey(p.room)]);
      return empty(204);
    }
  }
}

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;
const HEXTET = /^[0-9A-Fa-f]{1,4}$/;

/** An IPv4 address's four bytes, or null. */
function ipv4Bytes(s: string): number[] | null {
  const m = IPV4.exec(s);
  if (!m) return null;
  const bytes = m.slice(1).map(Number);
  return bytes.every((b) => b <= 255) ? bytes : null;
}

/** Colon-separated groups as hextets, the last one optionally an IPv4 tail (two hextets), or null. */
function hextets(groups: string[], v4Tail: boolean): number[] | null {
  const out: number[] = [];
  for (let i = 0; i < groups.length; i++) {
    const v4 = v4Tail && i === groups.length - 1 ? ipv4Bytes(groups[i]!) : null;
    if (v4) out.push((v4[0]! << 8) | v4[1]!, (v4[2]! << 8) | v4[3]!);
    else if (HEXTET.test(groups[i]!)) out.push(parseInt(groups[i]!, 16));
    else return null;
  }
  return out;
}

/** An IPv6 address's eight hextets (an embedded IPv4 tail allowed), or null. */
function ipv6Hextets(s: string): number[] | null {
  const halves = s.split("::");
  if (halves.length > 2) return null;
  const groups = (half: string) => (half === "" ? [] : half.split(":"));
  const head = hextets(groups(halves[0]!), halves.length === 1);
  if (halves.length === 1) return head?.length === 8 ? head : null;
  const tail = hextets(groups(halves[1]!), true);
  if (!head || !tail) return null;
  // "::" stands for at least one zero hextet.
  const fill = 8 - head.length - tail.length;
  return fill < 1 ? null : [...head, ...new Array<number>(fill).fill(0), ...tail];
}

/** An address's rate key: IPv4 whole, IPv6 its /64 (IPv4-mapped IPv6 as its IPv4), or null if it is not an address. */
function addressKey(s: string): string | null {
  const v4 = ipv4Bytes(s);
  if (v4) return v4.join(".");
  if (s.length > 45 || !s.includes(":")) return null;
  const h = ipv6Hextets(s);
  if (!h) return null;
  // ::ffff:a.b.c.d is an IPv4 client.
  if (h.slice(0, 5).every((x) => x === 0) && h[5] === 0xffff) {
    return [h[6]! >> 8, h[6]! & 255, h[7]! >> 8, h[7]! & 255].join(".");
  }
  return h
    .slice(0, 4)
    .map((x) => x.toString(16).padStart(4, "0"))
    .join(":");
}

/**
 * The client, for its rate keys: the first `x-forwarded-for` entry, else
 * `x-real-ip`, else `unknown`. An IPv4 address is the whole address; an
 * IPv6 one is its /64 prefix (the first four hextets, expanded, lower
 * case), since one host may hold a whole /64. A value that is not an
 * address counts as absent, so header garbage never reaches a key.
 */
export function clientKey(req: Request): string {
  for (const header of ["x-forwarded-for", "x-real-ip"]) {
    const value = req.headers.get(header);
    if (value === null) continue;
    const key = addressKey(value.slice(0, 64).split(",")[0]!.trim());
    if (key !== null) return key;
  }
  return "unknown";
}

/** 429 with `Retry-After` the whole seconds left until `reset` (at least 1). */
function tooMany(reset: number, now: number): Response {
  const seconds = Math.max(1, Math.ceil((reset - now) / 1000));
  return new Response(null, { status: 429, headers: { ...NO_STORE, "Retry-After": String(seconds) } });
}

/**
 * Counts a valid request against its caps, in this order, stopping at
 * the first one over (a 429 whose `Retry-After` is what is left of that
 * limiter's window):
 *
 * 1. all requests per client: a client over its own cap costs one
 *    limiter call and touches no other limiter, the global one included;
 * 2. polls per room (a poll) or writes per client (a create, answer or
 *    close): a request refused here costs two calls, and still never
 *    moves the global counter;
 * 3. all requests, global: counted only for a request every narrower cap
 *    let through, so one client or one room cannot spend more of it than
 *    its own cap allows.
 */
async function rateLimit(req: Request, p: Parsed, deps: SignalDeps): Promise<Response | null> {
  const { limiters } = deps;
  const client = clientKey(req);
  const steps: Array<[Limiter, string]> = [
    [limiters.client, client],
    p.op === "poll" ? [limiters.roomPolls, p.room] : [limiters.clientWrites, client],
    [limiters.global, GLOBAL_KEY],
  ];
  for (const [limiter, key] of steps) {
    const { success, reset } = await limiter.limit(key);
    if (!success) return tooMany(reset, (deps.now ?? Date.now)());
  }
  return null;
}

/**
 * Handles one signaling request: validate (no store or limiter call; a
 * 400 or 405 counts nothing), rate-limit, serve. Never throws, and never
 * logs.
 */
export async function handleSignal(req: Request, deps: SignalDeps): Promise<Response> {
  try {
    const parsed = await parse(req);
    if (parsed instanceof Response) return parsed;
    const limited = await rateLimit(req, parsed, deps);
    if (limited) return limited;
    return await serve(parsed, deps.rooms);
  } catch (e) {
    if (e instanceof BadRequest) return empty(400);
    return empty(500);
  }
}

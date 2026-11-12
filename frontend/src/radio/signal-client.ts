/**
 * The browser side of signaling (Milestone 10 Task 10): hosts or joins a
 * room on `/api/signal` (`./signal-handler.ts`'s protocol), the only
 * server URL this page calls. The server carries one offer and one answer;
 * the radio traffic then flows over the direct WebRTC link
 * (`./peer-link.ts`).
 *
 * - {@link hostRoom} makes an offer, creates a room for it and returns
 *   the room code at once; its `connected` promise polls for the answer
 *   every {@link POLL_INTERVAL_MS}, applies it and waits for the channel
 *   to open, all within {@link SIGNAL_TIMEOUT_MS} of the start. It keeps
 *   the room's token from `create` to itself, and sends `close` with it,
 *   best effort, exactly once: on its `cancel` (Milestone 11 Task 10) or
 *   when `connected` fails, a timeout included (Milestone 12 Task 5).
 * - {@link joinRoom} fetches the room's offer, answers it, and waits for
 *   the channel to open within the same budget.
 *
 * Waiting for the channel is event-driven: {@link PeerLink.opened} raced
 * against the deadline and the cancel, with no polling.
 *
 * Every request aborts after {@link SIGNAL_FETCH_TIMEOUT_MS} (and a host's
 * polls also on its cancel), its reply's body read included, so a hung
 * fetch or a stalled body ends as "Could not reach the signaling server"
 * rather than holding the side forever.
 *
 * Every failure is a {@link SignalError} with a message meant for the
 * page's status line, and ends that side's requests; a link that fails is
 * closed. A 429 says how long to wait ({@link busyMessage}), any 5xx is
 * "Signaling unavailable." (a store failure is a bare 500), and a 404 on
 * join is "Room closed or expired.". Nothing is logged.
 */
import { acceptOffer, createOffer, type PeerConnectionFactory, type PeerLink } from "./peer-link";
import { CROCKFORD_ALPHABET, ROOM_CODE_LENGTH, TOKEN_LENGTH, isRoomCode } from "./signal-handler";

export const SIGNAL_URL = "/api/signal";
export const POLL_INTERVAL_MS = 1000;
export const SIGNAL_TIMEOUT_MS = 120_000;
/** How long one request to the signaling server may take before it is aborted. */
export const SIGNAL_FETCH_TIMEOUT_MS = 10_000;

export const SIGNALING_UNAVAILABLE = "Signaling unavailable.";
export const ROOM_CLOSED = "Room closed or expired.";

export class SignalError extends Error {
  override name = "SignalError";
}

/** Injectable for tests; the defaults are the browser's. */
export interface SignalDeps {
  fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  /** Waits `ms`; `signal`'s abort may cut it short (the promise then never settles). */
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
  now?: () => number;
  factory?: PeerConnectionFactory;
  /** Aborts each request after this many ms; default {@link SIGNAL_FETCH_TIMEOUT_MS}. */
  fetchTimeoutMs?: number;
}

export interface HostedRoom {
  /** The code to give the peer. */
  room: string;
  link: PeerLink;
  /** Resolves when the link is open; rejects with a {@link SignalError}. */
  connected: Promise<void>;
  /** Stops waiting and closes the link. */
  cancel(): void;
}

/**
 * A typed room code in canonical form, or `null`: case-insensitive, with
 * spaces and hyphens ignored and I/L read as 1 and O as 0 (Crockford's
 * decoding rules).
 */
export function normalizeRoomCode(input: string): string | null {
  if (typeof input !== "string") return null;
  const code = input
    .toUpperCase()
    .replace(/[\s-]/g, "")
    .replace(/[IL]/g, "1")
    .replace(/O/g, "0");
  return isRoomCode(code) ? code : null;
}

/** The browser's sleep: a timer, cleared if `signal` aborts so it does not outlive the wait. */
function timerSleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => clearTimeout(timer), { once: true });
  });
}

function resolveDeps(deps: SignalDeps) {
  return {
    fetch: deps.fetch ?? ((input: string, init?: RequestInit) => fetch(input, init)),
    sleep: deps.sleep ?? timerSleep,
    now: deps.now ?? (() => Date.now()),
    factory: deps.factory,
    fetchTimeoutMs: deps.fetchTimeoutMs ?? SIGNAL_FETCH_TIMEOUT_MS,
  };
}
type Deps = ReturnType<typeof resolveDeps>;

/**
 * One request to the server, and its reply's body as JSON (`null` when it
 * is not JSON, an empty reply included). It aborts on `signal` or after
 * `d.fetchTimeoutMs`, whichever comes first (a hand-made controller and
 * timer, cleared once the body is read: ruling R-T5-1, extended to the
 * body in Milestone 13 Task 7); an abort, during the fetch or the body
 * read, or any throw of the fetch is a {@link SignalError}.
 */
async function call(
  d: Deps,
  init: RequestInit,
  query = "",
  signal?: AbortSignal,
): Promise<{ res: Response; body: unknown }> {
  const abort = new AbortController();
  const onAbort = () => abort.abort();
  if (signal?.aborted) abort.abort();
  else signal?.addEventListener("abort", onAbort, { once: true });
  const timer = setTimeout(onAbort, d.fetchTimeoutMs);
  try {
    const res = await d.fetch(SIGNAL_URL + query, { cache: "no-store", ...init, signal: abort.signal });
    let body: unknown = null;
    try {
      // Raced against the abort too, so a body that ignores the signal still ends.
      body = await Promise.race([res.json(), whenAborted(abort.signal)]);
    } catch (e) {
      if (abort.signal.aborted) throw e;
      // Not JSON: `null`.
    }
    return { res, body };
  } catch {
    throw new SignalError("Could not reach the signaling server; check the connection.");
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener("abort", onAbort);
  }
}

const RETRY_AFTER_SECONDS = /^\s*(\d{1,9})\s*$/;

/**
 * A 429's message: `Retry-After` (whole seconds) rounded up to minutes,
 * at least 1. A missing or malformed header (an HTTP date included, which
 * this server never sends) says "later" (ruling R-T10-1, Milestone 11).
 */
export function busyMessage(retryAfter: string | null): string {
  const m = retryAfter === null ? null : RETRY_AFTER_SECONDS.exec(retryAfter);
  if (!m) return "Signaling server busy, try again later.";
  const minutes = Math.max(1, Math.ceil(Number(m[1]) / 60));
  return `Signaling server busy, try again in ${minutes} min.`;
}

/** A reply that ends this side's signaling: busy, unavailable, or another status. */
function serverError(res: Response): SignalError {
  if (res.status === 429) return new SignalError(busyMessage(res.headers.get("Retry-After")));
  if (res.status >= 500) return new SignalError(SIGNALING_UNAVAILABLE);
  return new SignalError(`Signaling server error (HTTP ${res.status}).`);
}

const TOKEN = new RegExp(`^[A-Za-z0-9_-]{${TOKEN_LENGTH}}$`);

function isToken(s: unknown): s is string {
  return typeof s === "string" && TOKEN.test(s);
}

/** A JSON reply's string field, or a {@link SignalError}. */
function field(body: unknown, name: string): string {
  const value = (body as Record<string, unknown> | null)?.[name];
  if (typeof value === "string" && value.length > 0) return value;
  throw new SignalError("Unexpected reply from the signaling server.");
}

const CANCELLED = "Cancelled.";
const NO_PATH =
  "Could not connect to the peer: no direct path between the two browsers (are they on the same network?).";

/** Rejects with "Cancelled." when `signal` aborts; never settles otherwise, and never goes unhandled. */
function whenAborted(signal: AbortSignal): Promise<never> {
  const p = new Promise<never>((_, reject) => {
    const fail = () => reject(new SignalError(CANCELLED));
    if (signal.aborted) fail();
    else signal.addEventListener("abort", fail, { once: true });
  });
  p.catch(() => {});
  return p;
}

/**
 * Waits for `link` to open: {@link PeerLink.opened} raced against
 * `deadline` and `signal`, with no polling. Closes the link and throws on
 * a close, the deadline or a cancel.
 */
async function waitOpen(d: Deps, link: PeerLink, deadline: number, signal: AbortSignal): Promise<void> {
  // Ends the deadline timer once this wait is over, however it ends.
  const done = new AbortController();
  const onCancel = () => done.abort();
  signal.addEventListener("abort", onCancel, { once: true });
  try {
    const outcome = await Promise.race([
      link.opened.then(
        () => "open" as const,
        () => "closed" as const,
      ),
      d.sleep(Math.max(0, deadline - d.now()), done.signal).then(() => "deadline" as const),
      whenAborted(signal),
    ]);
    if (outcome === "open") return;
  } catch (e) {
    link.close();
    throw e;
  } finally {
    signal.removeEventListener("abort", onCancel);
    done.abort();
  }
  link.close();
  throw new SignalError(signal.aborted ? CANCELLED : NO_PATH);
}

/** Asks the server to close `room`; best effort: never throws or rejects, and its reply is ignored. */
function sendClose(d: Deps, room: string, token: string): void {
  call(d, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ op: "close", room, token }),
    // Let it finish if the page is unloading.
    keepalive: true,
  }).then(
    () => {},
    () => {},
  );
}

/** Hosts a room: returns its code at once, and a promise for the open link. */
export async function hostRoom(deps: SignalDeps = {}): Promise<HostedRoom> {
  const d = resolveDeps(deps);
  const deadline = d.now() + SIGNAL_TIMEOUT_MS;
  let link: PeerLink;
  let offer: string;
  try {
    ({ link, offer } = await createOffer({ factory: d.factory }));
  } catch {
    throw new SignalError("This browser could not start a WebRTC link.");
  }
  let room: string;
  let token: string;
  try {
    const { res, body } = await call(d, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ op: "create", offer }),
    });
    if (res.status !== 200) throw serverError(res);
    const reply = body as Record<string, unknown> | null;
    room = reply?.room as string;
    token = reply?.token as string;
    if (!isRoomCode(room) || !isToken(token)) throw new SignalError("Unexpected reply from the signaling server.");
  } catch (e) {
    link.close();
    throw e;
  }

  const cancel = new AbortController();
  const cancelled = whenAborted(cancel.signal);
  // Shared by `cancel()` and `connected`'s failure, whichever comes first: one `close`.
  let closeSent = false;
  const closeOnce = () => {
    if (closeSent) return;
    closeSent = true;
    sendClose(d, room, token);
  };
  const connected = (async () => {
    try {
      for (;;) {
        if (cancel.signal.aborted) throw new SignalError(CANCELLED);
        const poll = call(d, { method: "GET" }, `?room=${room}&want=answer`, cancel.signal);
        const { res, body } = await Promise.race([poll, cancelled]);
        if (res.status === 200) {
          const answer = field(body, "answer");
          try {
            await link.complete(answer);
          } catch {
            throw new SignalError("The peer's answer could not be used.");
          }
          break;
        }
        if (res.status === 404) throw new SignalError(`Room ${room} expired before anyone joined.`);
        if (res.status !== 204) throw serverError(res);
        if (d.now() >= deadline) throw new SignalError(`No one joined room ${room} within 2 minutes.`);
        await Promise.race([d.sleep(POLL_INTERVAL_MS, cancel.signal), cancelled]);
      }
      await waitOpen(d, link, deadline, cancel.signal);
    } catch (e) {
      link.close();
      // The room is of no use to anyone now: free it rather than leave it to its TTL.
      closeOnce();
      throw e instanceof SignalError ? e : new SignalError("Linking failed.");
    }
  })();
  // The caller may never await it (a cancelled host); keep it from going unhandled.
  connected.catch(() => {});
  return {
    room,
    link,
    connected,
    cancel() {
      if (cancel.signal.aborted) return;
      cancel.abort();
      link.close();
      closeOnce();
    },
  };
}

/** Joins the room `code` (as typed): resolves with the open link, or rejects with a {@link SignalError}. */
export async function joinRoom(code: string, deps: SignalDeps = {}): Promise<PeerLink> {
  const d = resolveDeps(deps);
  const deadline = d.now() + SIGNAL_TIMEOUT_MS;
  const room = normalizeRoomCode(code);
  if (!room) {
    throw new SignalError(
      `A room code is ${ROOM_CODE_LENGTH} characters from ${CROCKFORD_ALPHABET.slice(0, 4)}…${CROCKFORD_ALPHABET.slice(-4)}.`,
    );
  }
  const { res, body } = await call(d, { method: "GET" }, `?room=${room}&want=offer`);
  if (res.status === 404) throw new SignalError(ROOM_CLOSED);
  if (res.status !== 200) throw serverError(res);
  const offer = field(body, "offer");

  let link: PeerLink;
  let answer: string;
  try {
    ({ link, answer } = await acceptOffer(offer, { factory: d.factory }));
  } catch {
    throw new SignalError("The host's offer could not be used.");
  }
  try {
    const { res: posted } = await call(d, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ op: "answer", room, answer }),
    });
    if (posted.status === 409) throw new SignalError(`Room ${room} was already joined by someone else.`);
    if (posted.status === 404) throw new SignalError(ROOM_CLOSED);
    if (posted.status !== 204) throw serverError(posted);
    await waitOpen(d, link, deadline, new AbortController().signal);
  } catch (e) {
    link.close();
    throw e instanceof SignalError ? e : new SignalError("Linking failed.");
  }
  return link;
}

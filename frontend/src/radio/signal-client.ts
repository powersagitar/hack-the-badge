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
 *   to open, all within {@link SIGNAL_TIMEOUT_MS} of the start.
 * - {@link joinRoom} fetches the room's offer, answers it, and waits for
 *   the channel to open within the same budget.
 *
 * Every failure is a {@link SignalError} with a message meant for the
 * page's status line; a link that fails is closed.
 */
import { acceptOffer, createOffer, type PeerConnectionFactory, type PeerLink } from "./peer-link";
import { CROCKFORD_ALPHABET, ROOM_CODE_LENGTH, isRoomCode } from "./signal-handler";

export const SIGNAL_URL = "/api/signal";
export const POLL_INTERVAL_MS = 1000;
export const SIGNAL_TIMEOUT_MS = 120_000;

export class SignalError extends Error {
  override name = "SignalError";
}

/** Injectable for tests; the defaults are the browser's. */
export interface SignalDeps {
  fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
  factory?: PeerConnectionFactory;
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

function resolveDeps(deps: SignalDeps) {
  return {
    fetch: deps.fetch ?? ((input: string, init?: RequestInit) => fetch(input, init)),
    sleep: deps.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms))),
    now: deps.now ?? (() => Date.now()),
    factory: deps.factory,
  };
}
type Deps = ReturnType<typeof resolveDeps>;

async function call(d: Deps, init: RequestInit, query = ""): Promise<Response> {
  try {
    return await d.fetch(SIGNAL_URL + query, { cache: "no-store", ...init });
  } catch {
    throw new SignalError("Could not reach the signaling server; check the connection.");
  }
}

function serverError(res: Response): SignalError {
  return new SignalError(`Signaling server error (HTTP ${res.status}).`);
}

/** A JSON reply's string field, or a {@link SignalError}. */
async function field(res: Response, name: string): Promise<string> {
  try {
    const body = (await res.json()) as Record<string, unknown>;
    const value = body?.[name];
    if (typeof value === "string" && value.length > 0) return value;
  } catch {
    // Not JSON.
  }
  throw new SignalError("Unexpected reply from the signaling server.");
}

/** Waits until `link` is open, polling its state; closes it and throws on failure or at `deadline`. */
async function waitOpen(d: Deps, link: PeerLink, deadline: number, cancelled: () => boolean): Promise<void> {
  for (;;) {
    if (link.state === "open") return;
    if (cancelled()) throw new SignalError("Cancelled.");
    if (link.state === "closed" || d.now() >= deadline) {
      link.close();
      throw new SignalError(
        "Could not connect to the peer: no direct path between the two browsers (are they on the same network?).",
      );
    }
    await d.sleep(Math.min(POLL_INTERVAL_MS, Math.max(0, deadline - d.now())));
  }
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
  try {
    const res = await call(d, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ op: "create", offer }),
    });
    if (res.status !== 200) throw serverError(res);
    room = await field(res, "room");
    if (!isRoomCode(room)) throw new SignalError("Unexpected reply from the signaling server.");
  } catch (e) {
    link.close();
    throw e;
  }

  let cancelled = false;
  const connected = (async () => {
    try {
      for (;;) {
        if (cancelled) throw new SignalError("Cancelled.");
        const res = await call(d, { method: "GET" }, `?room=${room}&want=answer`);
        if (res.status === 200) {
          const answer = await field(res, "answer");
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
        await d.sleep(POLL_INTERVAL_MS);
      }
      await waitOpen(d, link, deadline, () => cancelled);
    } catch (e) {
      link.close();
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
      cancelled = true;
      link.close();
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
  const res = await call(d, { method: "GET" }, `?room=${room}&want=offer`);
  if (res.status === 404) throw new SignalError(`No room ${room}: check the code, or it expired.`);
  if (res.status !== 200) throw serverError(res);
  const offer = await field(res, "offer");

  let link: PeerLink;
  let answer: string;
  try {
    ({ link, answer } = await acceptOffer(offer, { factory: d.factory }));
  } catch {
    throw new SignalError("The host's offer could not be used.");
  }
  try {
    const posted = await call(d, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ op: "answer", room, answer }),
    });
    if (posted.status === 409) throw new SignalError(`Room ${room} was already joined by someone else.`);
    if (posted.status === 404) throw new SignalError(`Room ${room} expired.`);
    if (posted.status !== 204) throw serverError(posted);
    await waitOpen(d, link, deadline, () => false);
  } catch (e) {
    link.close();
    throw e instanceof SignalError ? e : new SignalError("Linking failed.");
  }
  return link;
}

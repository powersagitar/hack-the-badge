/**
 * The browser radio hub (Milestone 10 Task 9): carries advertisements and
 * bumps between this browser's one badge and its linked peers. It lives on
 * the main thread, between the firmware Worker (through
 * `../runtime/firmware-worker-client.ts`) and the links, which Task 10's
 * WebRTC data channels provide. With no link, the badge is alone in the
 * room.
 *
 * - **Out.** {@link RadioHub.fromLocal} takes the badge's advertisement
 *   (the Worker's `advOut`) and sends it to every link as
 *   `{ t: "adv", b: <base64> }`. An empty one (the badge stopped
 *   advertising) is not sent: a peer's controller keeps no report state,
 *   so silence is enough.
 * - **In.** A link's message is validated here, before it reaches the
 *   Worker, by a `valibot` schema (Milestone 11 Task 11): an object whose
 *   `t` is `"adv"` or `"bump"`, and for `"adv"` a string `b` of at most
 *   `MAX_BASE64_CHARS`, which must then be canonical base64 decoding to 7
 *   to `MAX_ADVERTISEMENT_BYTES` bytes (the core's decoder bounds, ruling
 *   R-T8-1). Anything else, including a hostile getter, is dropped without
 *   a throw. An advert is delivered at {@link LINKED_PEER_RSSI}.
 * - **No hop.** A peer's message is never forwarded to another link: a
 *   radio does not relay what it hears.
 * - **Bump.** {@link RadioHub.bump} (the page's Bump button) bumps the
 *   local badge and sends `{ t: "bump" }` to every link; a peer's bump
 *   bumps only the local badge. It is gated on the local badge: while it
 *   is not running ({@link RadioHub.setRunning}, which
 *   `ui/badge-controller.ts` calls at the runtime's start and stop) or the hub is paused (the page is
 *   hidden), `bump()` sends nothing, bumps nothing and returns `false`
 *   (Milestone 11's ruling R-T11-1, revising Milestone 10's R-T10-1). A
 *   new hub is not running.
 * - **Paused** (the hidden-page pause, Milestone 7's ruling R-T8-10): the
 *   hub keeps only the latest advertisement per peer, and delivers those
 *   when it resumes. A bump heard while paused is dropped, not replayed
 *   (ruling R-T9-3).
 *
 * Every call into a link or the target is guarded: a link that throws on
 * send, or a target whose Worker is gone, does not stop the others.
 */
import * as v from "valibot";

/**
 * The RSSI (dBm) a linked peer is heard at: the same value as
 * `emulator_core::radio::LINKED_PEER_RSSI` (ruling R-T8-1), inside
 * Connect's −64 dBm gate. `test/radio-hub.test.ts` checks both.
 */
export const LINKED_PEER_RSSI = -40;

/** Bytes before the advertising data in the wire form: `addr(6) ‖ addr_type(1)`. */
export const ADVERTISEMENT_HEADER_BYTES = 7;

/**
 * The largest advertisement in the wire form: the header plus 229 data
 * bytes, the core's single-report limit (`MAX_ADV_DATA_LEN`).
 */
export const MAX_ADVERTISEMENT_BYTES = ADVERTISEMENT_HEADER_BYTES + 229;

/** The longest base64 string accepted: `MAX_ADVERTISEMENT_BYTES` encoded. */
const MAX_BASE64_CHARS = Math.ceil(MAX_ADVERTISEMENT_BYTES / 3) * 4;

/** What goes over a link. `b` is the advertisement, base64. */
export type HubMessage = { t: "adv"; b: string } | { t: "bump" };

export interface RadioLink {
  /** Unique per peer; a link added under an id already present replaces it. */
  id: string;
  send(msg: HubMessage): void;
  /** Registers the hub's handler for the peer's messages (unvalidated). */
  onMessage(cb: (msg: unknown) => void): void;
}

/** Where the hub delivers: the local badge. */
export interface RadioHubTarget {
  /** A peer's advertisement, validated, heard at `rssi` dBm. */
  deliver(bytes: Uint8Array, rssi: number): void;
  /** Bumps the local badge. */
  bumpLocal(): void;
}

export function encodeBase64(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;

/** Decodes canonical, padded base64; `null` for anything else. Never throws. */
export function decodeBase64(s: string): Uint8Array | null {
  if (!BASE64.test(s)) return null;
  try {
    const bin = atob(s);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

type Incoming = { kind: "adv"; bytes: Uint8Array } | { kind: "bump" };

/** A peer's message on the wire; unknown keys are ignored. */
const PeerMessageSchema = v.variant("t", [
  v.object({ t: v.literal("adv"), b: v.pipe(v.string(), v.maxLength(MAX_BASE64_CHARS)) }),
  v.object({ t: v.literal("bump") }),
]);

/** A peer's message, validated; `null` if malformed. May throw on a hostile object (the caller guards). */
function parse(msg: unknown): Incoming | null {
  if (Array.isArray(msg)) return null;
  const parsed = v.safeParse(PeerMessageSchema, msg);
  if (!parsed.success) return null;
  if (parsed.output.t === "bump") return { kind: "bump" };
  const bytes = decodeBase64(parsed.output.b);
  if (!bytes || bytes.length < ADVERTISEMENT_HEADER_BYTES || bytes.length > MAX_ADVERTISEMENT_BYTES) return null;
  return { kind: "adv", bytes };
}

export class RadioHub {
  private readonly links = new Map<string, RadioLink>();
  private paused = false;
  /** The local badge's runtime is started (`ui/badge-controller.ts`); a new hub's is not. */
  private running = false;
  /** While paused: the latest advertisement per peer id, in first-heard order. */
  private readonly held = new Map<string, Uint8Array>();

  constructor(private readonly target: RadioHubTarget) {}

  addLink(link: RadioLink): void {
    this.removeLink(link.id);
    this.links.set(link.id, link);
    try {
      link.onMessage((msg) => this.receive(link, msg));
    } catch (e) {
      console.error("radio hub: link refused a handler:", e);
      this.links.delete(link.id);
    }
  }

  removeLink(id: string): void {
    this.links.delete(id);
    this.held.delete(id);
  }

  /** The local badge's advertisement (the Worker's `advOut`). */
  fromLocal(bytes: Uint8Array): void {
    if (bytes.length === 0) return;
    this.broadcast({ t: "adv", b: encodeBase64(bytes) });
  }

  /**
   * The page's Bump button: bumps the local badge and every linked peer,
   * and returns `true`. While the badge is not running, or the hub is
   * paused, it does nothing and returns `false`.
   */
  bump(): boolean {
    if (!this.running || this.paused) return false;
    this.guard(() => this.target.bumpLocal());
    this.broadcast({ t: "bump" });
    return true;
  }

  /** Whether the local badge's runtime is started; gates {@link bump}. */
  setRunning(running: boolean): void {
    this.running = running;
  }

  setPaused(paused: boolean): void {
    if (paused === this.paused) return;
    this.paused = paused;
    if (paused) return;
    const held = [...this.held.values()];
    this.held.clear();
    for (const bytes of held) this.guard(() => this.target.deliver(bytes, LINKED_PEER_RSSI));
  }

  private receive(link: RadioLink, msg: unknown): void {
    // A replaced or removed link is heard no more.
    if (this.links.get(link.id) !== link) return;
    let parsed: Incoming | null = null;
    try {
      parsed = parse(msg);
    } catch {
      return;
    }
    if (!parsed) return;
    if (parsed.kind === "bump") {
      if (!this.paused) this.guard(() => this.target.bumpLocal());
      return;
    }
    if (this.paused) {
      this.held.set(link.id, parsed.bytes);
      return;
    }
    const bytes = parsed.bytes;
    this.guard(() => this.target.deliver(bytes, LINKED_PEER_RSSI));
  }

  private broadcast(msg: HubMessage): void {
    for (const link of [...this.links.values()]) this.guard(() => link.send(msg));
  }

  private guard(f: () => void): void {
    try {
      f();
    } catch (e) {
      console.error("radio hub:", e);
    }
  }
}

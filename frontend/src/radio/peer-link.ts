/**
 * A WebRTC radio link to one peer browser (Milestone 10 Task 10): a
 * {@link RadioLink} for `./hub.ts`'s `RadioHub`, over one ordered data
 * channel named `"radio"`.
 *
 * - **No servers.** Every peer connection is made with `iceServers: []`:
 *   no STUN, no TURN, no relay. The two browsers must reach each other
 *   directly (host candidates, which Chrome hides behind mDNS names), so
 *   in practice they share a network.
 * - **No trickle.** {@link createOffer} and {@link acceptOffer} return the
 *   SDP only once `iceGatheringState` is `"complete"`, so one offer and
 *   one answer carry every candidate. They are swapped through the
 *   signaling function (`./signal-client.ts`).
 * - **Nothing before open.** {@link PeerLink.send} drops a message unless
 *   the channel is open. {@link PeerLink.opened} settles on the channel's
 *   own events (Milestone 11 Task 10), so a waiter needs no polling.
 * - **Peer input is untrusted.** A message is passed on only if it is a
 *   string of at most {@link MAX_PEER_MESSAGE_CHARS} that parses as JSON;
 *   the hub validates its shape. Nothing a peer sends throws out of a
 *   channel callback.
 *
 * `RTCPeerConnection` is reached only through a {@link PeerConnectionFactory}
 * (the browser's by default), so `bun test`, which has none, injects a fake.
 */
import type { HubMessage, RadioLink } from "./hub";
import { MAX_SDP_CHARS } from "./signal-handler";

/**
 * The longest peer message passed to the hub. The largest real one, an
 * advertisement of `MAX_ADVERTISEMENT_BYTES` in base64 inside
 * `{"t":"adv","b":"…"}`, is about 330 characters.
 */
export const MAX_PEER_MESSAGE_CHARS = 1024;
/** Longest wait for ICE gathering; with no ICE servers it takes milliseconds. */
export const ICE_GATHERING_TIMEOUT_MS = 10_000;
/** The data channel's label. */
export const RADIO_CHANNEL = "radio";

export interface SessionDescription {
  type: "offer" | "answer";
  sdp: string;
}

/** The part of `RTCDataChannel` used here. */
export interface DataChannelLike {
  readonly label: string;
  readonly readyState: string;
  onopen: (() => void) | null;
  onclose: (() => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  send(data: string): void;
  close(): void;
}

/** The part of `RTCPeerConnection` used here. */
export interface PeerConnectionLike {
  readonly iceGatheringState: string;
  readonly connectionState?: string;
  readonly localDescription: { sdp: string } | null;
  onicegatheringstatechange: (() => void) | null;
  ondatachannel: ((ev: { channel: DataChannelLike }) => void) | null;
  onconnectionstatechange: (() => void) | null;
  createDataChannel(label: string, init?: { ordered?: boolean }): DataChannelLike;
  createOffer(): Promise<SessionDescription>;
  createAnswer(): Promise<SessionDescription>;
  setLocalDescription(d: SessionDescription): Promise<void>;
  setRemoteDescription(d: SessionDescription): Promise<void>;
  close(): void;
}

/** Always called with `{ iceServers: [] }`. */
export type PeerConnectionFactory = (config: { iceServers: [] }) => PeerConnectionLike;

export interface PeerLinkOptions {
  factory?: PeerConnectionFactory;
}

export type PeerLinkState = "connecting" | "open" | "closed";

const browserFactory: PeerConnectionFactory = (config) =>
  new RTCPeerConnection(config) as unknown as PeerConnectionLike;

let nextLinkId = 1;

export class PeerLink implements RadioLink {
  readonly id = `link-${nextLinkId++}`;
  private channel: DataChannelLike | null = null;
  private handler: ((msg: unknown) => void) | null = null;
  private readonly stateHandlers: Array<(s: PeerLinkState) => void> = [];
  private current: PeerLinkState = "connecting";
  private settleOpened!: (open: boolean) => void;

  /**
   * Resolves when the data channel opens; rejects if the link closes
   * first. It settles once, and never goes unhandled: nobody need await it.
   */
  readonly opened: Promise<void> = new Promise<void>((resolve, reject) => {
    this.settleOpened = (open) => (open ? resolve() : reject(new Error("closed before open")));
  });

  /** @internal Use {@link createOffer} or {@link acceptOffer}. */
  constructor(private readonly pc: PeerConnectionLike) {
    this.opened.catch(() => {});
    pc.onconnectionstatechange = () => {
      const s = pc.connectionState;
      // A failed connection does not recover without new signaling: free it.
      if (s === "failed" || s === "closed") this.close();
    };
  }

  get state(): PeerLinkState {
    return this.current;
  }

  /** The offerer's last step: applies the peer's answer. Rejects on a bad one. */
  async complete(answer: string): Promise<void> {
    if (typeof answer !== "string" || answer.length > MAX_SDP_CHARS) throw new Error("bad answer");
    await this.pc.setRemoteDescription({ type: "answer", sdp: answer });
  }

  send(msg: HubMessage): void {
    const ch = this.channel;
    if (!ch || this.current !== "open" || ch.readyState !== "open") return;
    try {
      ch.send(JSON.stringify(msg));
    } catch {
      // Closing under us: the close handler updates the state.
    }
  }

  onMessage(cb: (msg: unknown) => void): void {
    this.handler = cb;
  }

  /** Runs `cb` on each state change after this call. */
  onStateChange(cb: (s: PeerLinkState) => void): void {
    this.stateHandlers.push(cb);
  }

  close(): void {
    this.setState("closed");
    try {
      this.channel?.close();
    } catch {
      // Already closed.
    }
    try {
      this.pc.close();
    } catch {
      // Already closed.
    }
  }

  /** @internal Binds the `"radio"` channel (created here, or the peer's). */
  attach(ch: DataChannelLike): void {
    if (this.channel || this.current === "closed") return;
    this.channel = ch;
    ch.onopen = () => this.setState("open");
    ch.onclose = () => this.setState("closed");
    ch.onmessage = (ev) => this.receive(ev);
    if (ch.readyState === "open") this.setState("open");
  }

  private receive(ev: { data: unknown } | null | undefined): void {
    try {
      const data = ev?.data;
      if (typeof data !== "string" || data.length > MAX_PEER_MESSAGE_CHARS) return;
      const msg: unknown = JSON.parse(data);
      this.handler?.(msg);
    } catch {
      // Not JSON, or the handler failed: the message is dropped.
    }
  }

  private setState(s: PeerLinkState): void {
    if (s === this.current || this.current === "closed") return;
    this.current = s;
    // The first change settles `opened`: "open" resolves it, "closed" (before open) rejects it.
    this.settleOpened(s === "open");
    for (const cb of this.stateHandlers) {
      try {
        cb(s);
      } catch (e) {
        console.error("peer link: state handler failed:", e);
      }
    }
  }
}

/** Resolves once ICE gathering is complete (or after the timeout, with what was gathered). */
function gathered(pc: PeerConnectionLike): Promise<void> {
  if (pc.iceGatheringState === "complete") return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(done, ICE_GATHERING_TIMEOUT_MS);
    function done(): void {
      clearTimeout(timer);
      pc.onicegatheringstatechange = null;
      resolve();
    }
    pc.onicegatheringstatechange = () => {
      if (pc.iceGatheringState === "complete") done();
    };
  });
}

/** The local SDP, after gathering. */
async function localSdp(pc: PeerConnectionLike): Promise<string> {
  await gathered(pc);
  const sdp = pc.localDescription?.sdp;
  if (!sdp || sdp.length > MAX_SDP_CHARS) throw new Error("no local description");
  return sdp;
}

/** Starts a link as the offerer: the link (connecting) and the offer SDP to send. */
export async function createOffer(opts: PeerLinkOptions = {}): Promise<{ link: PeerLink; offer: string }> {
  const pc = (opts.factory ?? browserFactory)({ iceServers: [] });
  const link = new PeerLink(pc);
  try {
    link.attach(pc.createDataChannel(RADIO_CHANNEL, { ordered: true }));
    await pc.setLocalDescription(await pc.createOffer());
    return { link, offer: await localSdp(pc) };
  } catch (e) {
    link.close();
    throw e;
  }
}

/** Answers a peer's offer: the link (connecting) and the answer SDP to send back. */
export async function acceptOffer(
  offer: string,
  opts: PeerLinkOptions = {},
): Promise<{ link: PeerLink; answer: string }> {
  if (typeof offer !== "string" || offer.length === 0 || offer.length > MAX_SDP_CHARS) throw new Error("bad offer");
  const pc = (opts.factory ?? browserFactory)({ iceServers: [] });
  const link = new PeerLink(pc);
  pc.ondatachannel = (ev) => {
    try {
      if (ev?.channel?.label === RADIO_CHANNEL) link.attach(ev.channel);
    } catch {
      // A channel we cannot bind is ignored.
    }
  };
  try {
    await pc.setRemoteDescription({ type: "offer", sdp: offer });
    await pc.setLocalDescription(await pc.createAnswer());
    return { link, answer: await localSdp(pc) };
  } catch (e) {
    link.close();
    throw e;
  }
}

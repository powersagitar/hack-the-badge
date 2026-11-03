/**
 * A fake WebRTC for `bun test`, which has no `RTCPeerConnection`: just
 * enough of `PeerConnectionLike` for `src/radio/peer-link.ts`. Offers and
 * answers are `fake-offer-<n>` / `fake-answer-<n>`, looked up in this
 * network's registry; when the offerer applies the answer, the answerer
 * gets the `"radio"` channel through `ondatachannel` and both ends open.
 * A message sent on one end arrives at the other on a microtask.
 */
import type {
  DataChannelLike,
  PeerConnectionFactory,
  PeerConnectionLike,
  SessionDescription,
} from "../src/radio/peer-link";

export class FakeChannel implements DataChannelLike {
  readyState: "connecting" | "open" | "closing" | "closed" = "connecting";
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  readonly sent: string[] = [];
  peer: FakeChannel | null = null;
  constructor(
    readonly label: string,
    readonly ordered: boolean,
  ) {}
  send(data: string): void {
    if (this.readyState !== "open") throw new Error("InvalidStateError");
    this.sent.push(data);
    const peer = this.peer;
    if (peer) queueMicrotask(() => peer.onmessage?.({ data }));
  }
  close(): void {
    if (this.readyState === "closed") return;
    this.readyState = "closed";
    this.onclose?.();
    this.peer?.close();
  }
  open(): void {
    this.readyState = "open";
    this.onopen?.();
  }
}

export class FakePeerConnection implements PeerConnectionLike {
  iceGatheringState: "new" | "gathering" | "complete" = "new";
  connectionState = "new";
  localDescription: SessionDescription | null = null;
  remoteDescription: SessionDescription | null = null;
  onicegatheringstatechange: (() => void) | null = null;
  ondatachannel: ((ev: { channel: DataChannelLike }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  channel: FakeChannel | null = null;
  closed = false;
  constructor(
    readonly net: FakeNetwork,
    readonly id: number,
    readonly config: unknown,
  ) {}
  createDataChannel(label: string, init?: { ordered?: boolean }): DataChannelLike {
    this.channel = new FakeChannel(label, init?.ordered ?? true);
    return this.channel;
  }
  async createOffer(): Promise<SessionDescription> {
    return { type: "offer", sdp: `fake-offer-${this.id}` };
  }
  async createAnswer(): Promise<SessionDescription> {
    return { type: "answer", sdp: `fake-answer-${this.id}` };
  }
  async setLocalDescription(d: SessionDescription): Promise<void> {
    this.localDescription = d;
    this.iceGatheringState = "gathering";
    // Gathering finishes later, as in a browser.
    setTimeout(() => {
      this.iceGatheringState = "complete";
      this.onicegatheringstatechange?.();
    }, 1);
  }
  async setRemoteDescription(d: SessionDescription): Promise<void> {
    const m = /^fake-(offer|answer)-(\d+)$/.exec(d.sdp);
    if (!m || m[1] !== d.type) throw new Error("bad sdp");
    const other = this.net.pcs.get(Number(m[2]));
    if (!other) throw new Error("unknown peer");
    this.remoteDescription = d;
    if (d.type === "answer" && this.net.connectOnAnswer) {
      const mine = this.channel!;
      const theirs = new FakeChannel(mine.label, mine.ordered);
      mine.peer = theirs;
      theirs.peer = mine;
      other.ondatachannel?.({ channel: theirs });
      setTimeout(() => {
        mine.open();
        theirs.open();
      }, 1);
    }
  }
  close(): void {
    this.closed = true;
    this.channel?.close();
  }
}

export class FakeNetwork {
  readonly pcs = new Map<number, FakePeerConnection>();
  /** Off: the answer is applied but no path is found (the channel never opens). */
  connectOnAnswer = true;
  private next = 1;
  readonly factory: PeerConnectionFactory = (config) => {
    const pc = new FakePeerConnection(this, this.next++, config);
    this.pcs.set(pc.id, pc);
    return pc;
  };
}

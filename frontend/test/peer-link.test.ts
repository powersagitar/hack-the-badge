import { describe, expect, test } from "bun:test";
import { RadioHub } from "../src/radio/hub";
import {
  MAX_PEER_MESSAGE_CHARS,
  acceptOffer,
  createOffer,
  decodeManualCode,
  encodeManualCode,
  type PeerLink,
} from "../src/radio/peer-link";
import { MAX_SDP_CHARS } from "../src/radio/signal-handler";
import { FakeNetwork, type FakeChannel, type FakePeerConnection } from "./fake-rtc";

const tick = (ms = 5) => new Promise((r) => setTimeout(r, ms));

/** A connected pair of links over a fake network. */
async function pair(): Promise<{ net: FakeNetwork; a: PeerLink; b: PeerLink }> {
  const net = new FakeNetwork();
  const { link: a, offer } = await createOffer({ factory: net.factory });
  const { link: b, answer } = await acceptOffer(offer, { factory: net.factory });
  await a.complete(answer);
  await tick();
  return { net, a, b };
}

describe("peer link", () => {
  test("uses no ICE servers and one ordered channel named radio, and waits for gathering", async () => {
    const net = new FakeNetwork();
    const { link, offer } = await createOffer({ factory: net.factory });
    const pc = net.pcs.get(1)!;
    expect(pc.config).toEqual({ iceServers: [] });
    expect(pc.channel!.label).toBe("radio");
    expect(pc.channel!.ordered).toBe(true);
    // No trickle: the SDP comes back only after gathering completed.
    expect(pc.iceGatheringState).toBe("complete");
    expect(offer).toBe("fake-offer-1");
    expect(link.state).toBe("connecting");

    const { answer } = await acceptOffer(offer, { factory: net.factory });
    expect(net.pcs.get(2)!.config).toEqual({ iceServers: [] });
    expect(net.pcs.get(2)!.iceGatheringState).toBe("complete");
    expect(answer).toBe("fake-answer-2");
  });

  test("sends nothing until the channel is open", async () => {
    const net = new FakeNetwork();
    const { link: a, offer } = await createOffer({ factory: net.factory });
    a.send({ t: "bump" });
    const ch = net.pcs.get(1)!.channel as FakeChannel;
    expect(ch.sent).toEqual([]);
    const { answer } = await acceptOffer(offer, { factory: net.factory });
    await a.complete(answer);
    await tick();
    expect(a.state).toBe("open");
    a.send({ t: "bump" });
    expect(ch.sent).toEqual([JSON.stringify({ t: "bump" })]);
  });

  test("onMessage passes data-channel messages through, and drops garbage without throwing", async () => {
    const { net, a, b } = await pair();
    const got: unknown[] = [];
    b.onMessage((m) => got.push(m));
    a.send({ t: "adv", b: "AAAAAAAAAA==" });
    a.send({ t: "bump" });
    await tick();
    expect(got).toEqual([{ t: "adv", b: "AAAAAAAAAA==" }, { t: "bump" }]);

    // Garbage straight onto b's channel: not JSON, binary, oversized, a hostile object.
    const theirs = (net.pcs.get(1)!.channel as FakeChannel).peer!;
    got.length = 0;
    expect(() => {
      theirs.onmessage!({ data: "{not json" });
      theirs.onmessage!({ data: new ArrayBuffer(8) });
      theirs.onmessage!({ data: JSON.stringify({ t: "bump", pad: "x".repeat(MAX_PEER_MESSAGE_CHARS) }) });
      theirs.onmessage!({ data: null });
      theirs.onmessage!(null as unknown as { data: unknown });
    }).not.toThrow();
    expect(got).toEqual([]);
    // Valid JSON of any shape passes: the hub validates it.
    theirs.onmessage!({ data: "42" });
    expect(got).toEqual([42]);
  });

  test("a handler that throws does not escape the channel callback", async () => {
    const { a, b } = await pair();
    b.onMessage(() => {
      throw new Error("handler bug");
    });
    a.send({ t: "bump" });
    await tick();
  });

  test("a link plugs into the hub: a peer's bump bumps the local badge", async () => {
    const { a, b } = await pair();
    let bumps = 0;
    const hubB = new RadioHub({ deliver: () => {}, bumpLocal: () => bumps++ });
    hubB.addLink(b);
    const hubA = new RadioHub({ deliver: () => {}, bumpLocal: () => {} });
    hubA.addLink(a);
    hubA.bump();
    await tick();
    expect(bumps).toBe(1);
  });

  test("state follows the channel, and close() closes both", async () => {
    const { net, a, b } = await pair();
    const states: string[] = [];
    b.onStateChange((s) => states.push(s));
    expect(a.state).toBe("open");
    expect(b.state).toBe("open");
    a.close();
    expect(a.state).toBe("closed");
    expect(b.state).toBe("closed");
    expect(states).toEqual(["closed"]);
    expect((net.pcs.get(1) as FakePeerConnection).closed).toBe(true);
    // Sending on a closed link is a no-op.
    expect(() => a.send({ t: "bump" })).not.toThrow();
  });

  test("a bad answer or offer rejects (the caller reports it), never throws synchronously", async () => {
    const net = new FakeNetwork();
    const { link } = await createOffer({ factory: net.factory });
    await expect(link.complete("garbage")).rejects.toThrow();
    await expect(acceptOffer("garbage", { factory: net.factory })).rejects.toThrow();
    await expect(acceptOffer("x".repeat(MAX_SDP_CHARS + 1), { factory: net.factory })).rejects.toThrow();
  });
});

describe("manual codes", () => {
  const SDP = "v=0\r\no=- 4611731400430051336 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0\r\n".repeat(8);

  test("round-trip, in both directions", async () => {
    for (const kind of ["offer", "answer"] as const) {
      const code = await encodeManualCode(kind, SDP);
      expect(code.startsWith(`${kind}:`)).toBe(true);
      expect(/^[a-z]+:[A-Za-z0-9_-]+$/.test(code)).toBe(true);
      // Compressed: shorter than the SDP.
      expect(code.length).toBeLessThan(SDP.length);
      const decoded = await decodeManualCode(code);
      expect(decoded).toEqual({ kind, sdp: SDP });
      expect(await encodeManualCode(decoded!.kind, decoded!.sdp)).toBe(code);
      // Pasting adds whitespace and line breaks; they are ignored.
      expect(await decodeManualCode(`  ${code.slice(0, 20)}\n${code.slice(20)}  \n`)).toEqual({ kind, sdp: SDP });
    }
  });

  test("garbage is rejected without throwing", async () => {
    const valid = await encodeManualCode("offer", SDP);
    const bomb = new Uint8Array(
      await new Response(
        new Blob([new Uint8Array(1 << 20)]).stream().pipeThrough(new CompressionStream("deflate-raw")),
      ).arrayBuffer(),
    );
    let bin = "";
    for (const b of bomb) bin += String.fromCharCode(b);
    const bombCode = "offer:" + btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

    const garbage = [
      "",
      "offer:",
      "offer",
      "hello world",
      "offer:!!!!",
      "offer:AAAA",
      "answer:" + "A".repeat(100),
      "bogus:" + valid.slice("offer:".length),
      valid.slice(0, valid.length - 7),
      "offer:" + "A".repeat(200_000),
      bombCode,
      "offer:" + valid.slice("offer:".length) + "%",
    ];
    for (const g of garbage) expect(await decodeManualCode(g)).toBeNull();
    // Not a string at all (a hostile caller).
    expect(await decodeManualCode(42 as unknown as string)).toBeNull();
  });
});

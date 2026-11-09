import { describe, expect, test } from "bun:test";
import { SignalError, hostRoom, joinRoom, normalizeRoomCode, type SignalDeps } from "../src/radio/signal-client";
import { MemoryStore, type RoomStore, handleSignal, memoryDeps } from "../src/radio/signal-handler";
import { FakeNetwork } from "./fake-rtc";

/** Signaling through the real handler over a `MemoryStore`, with a fake clock for the polls. */
function harness(rooms: RoomStore = new MemoryStore()) {
  const signal = memoryDeps(undefined, rooms);
  const net = new FakeNetwork();
  let now = 0;
  const urls: string[] = [];
  const deps: SignalDeps = {
    factory: net.factory,
    fetch: async (input, init) => {
      const url = new URL(String(input), "http://localhost");
      urls.push(url.pathname);
      return handleSignal(new Request(url, init), signal);
    },
    // Advance the fake clock, and give the fake network a real moment.
    sleep: async (ms) => {
      now += ms;
      await new Promise((r) => setTimeout(r, 2));
    },
    now: () => now,
  };
  return { net, deps, urls, elapsed: () => now };
}

describe("signal client", () => {
  test("host and join a room, and the link opens on both sides", async () => {
    const { deps, urls } = harness();
    const hosted = await hostRoom(deps);
    expect(normalizeRoomCode(hosted.room)).toBe(hosted.room);
    const joined = await joinRoom(hosted.room.toLowerCase(), deps);
    await hosted.connected;
    expect(hosted.link.state).toBe("open");
    expect(joined.state).toBe("open");
    // Only /api/signal is ever called.
    expect(new Set(urls)).toEqual(new Set(["/api/signal"]));
  });

  test("normalizes typed codes the Crockford way", () => {
    expect(normalizeRoomCode(" abcd-efgh ")).toBe("ABCDEFGH");
    expect(normalizeRoomCode("ilo0ILO0")).toBe("11001100");
    expect(normalizeRoomCode("ABCDEFG")).toBeNull();
    expect(normalizeRoomCode("ABCDEFGU")).toBeNull();
    expect(normalizeRoomCode(7 as unknown as string)).toBeNull();
  });

  test("a wrong code gives a clear message", async () => {
    const { deps } = harness();
    await expect(joinRoom("ZZZZZZZZ", deps)).rejects.toThrow(/no room ZZZZZZZZ/i);
    await expect(joinRoom("bad", deps)).rejects.toThrow(/8 characters/);
  });

  test("a second join of one room is refused", async () => {
    const { deps } = harness();
    const hosted = await hostRoom(deps);
    await joinRoom(hosted.room, deps);
    await expect(joinRoom(hosted.room, deps)).rejects.toThrow(/already joined/i);
  });

  test("the host gives up after 120 s without an answer", async () => {
    const { deps, elapsed } = harness();
    const hosted = await hostRoom(deps);
    await expect(hosted.connected).rejects.toThrow(/no one joined/i);
    expect(elapsed()).toBeGreaterThanOrEqual(120_000);
    expect(elapsed()).toBeLessThan(122_000);
    expect(hosted.link.state).toBe("closed");
  });

  test("no direct path: both sides give up after 120 s with a message", async () => {
    const { net, deps } = harness();
    net.connectOnAnswer = false;
    const hosted = await hostRoom(deps);
    const join = joinRoom(hosted.room, deps);
    await expect(join).rejects.toThrow(/could not connect/i);
    await expect(hosted.connected).rejects.toThrow(/could not connect/i);
  });

  test("cancel stops the host's polling", async () => {
    const { deps } = harness();
    const hosted = await hostRoom(deps);
    hosted.cancel();
    await expect(hosted.connected).rejects.toThrow(/cancelled/i);
    expect(hosted.link.state).toBe("closed");
  });

  test("server errors and network failures are SignalErrors, never raw", async () => {
    const fail = () => Promise.reject(new Error("x"));
    const broken: RoomStore = { set: fail, get: fail, del: fail };
    const { deps } = harness(broken);
    await expect(hostRoom(deps)).rejects.toBeInstanceOf(SignalError);
    await expect(hostRoom(deps)).rejects.toThrow(/signaling server error \(HTTP 500\)/i);

    const offline = harness();
    offline.deps.fetch = () => Promise.reject(new TypeError("Failed to fetch"));
    await expect(joinRoom("ABCDEFGH", offline.deps)).rejects.toThrow(/could not reach/i);

    const junk = harness();
    junk.deps.fetch = async () => new Response("<html>", { status: 200 });
    await expect(hostRoom(junk.deps)).rejects.toThrow(/unexpected reply/i);
  });
});

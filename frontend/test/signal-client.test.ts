import { describe, expect, test } from "bun:test";
import {
  SIGNAL_TIMEOUT_MS,
  SignalError,
  hostRoom,
  joinRoom,
  normalizeRoomCode,
  type SignalDeps,
} from "../src/radio/signal-client";
import { MemoryStore, type RoomStore, handleSignal, memoryDeps } from "../src/radio/signal-handler";
import { FakeNetwork } from "./fake-rtc";

/** One request the client made, and the handler's status for it. */
interface Call {
  method: string;
  url: string;
  body: Record<string, unknown> | null;
  status: number;
  reply: Record<string, unknown> | null;
}

/** Signaling through the real handler over a `MemoryStore`, with a fake clock for the polls. */
function harness(rooms: RoomStore = new MemoryStore()) {
  const signal = memoryDeps(undefined, rooms);
  const net = new FakeNetwork();
  let now = 0;
  const urls: string[] = [];
  const calls: Call[] = [];
  const deps: SignalDeps = {
    factory: net.factory,
    fetch: async (input, init) => {
      const url = new URL(String(input), "http://localhost");
      urls.push(url.pathname);
      const body = typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : null;
      const res = await handleSignal(new Request(url, init), signal);
      let reply: Record<string, unknown> | null = null;
      try {
        reply = (await res.clone().json()) as Record<string, unknown>;
      } catch {
        // An empty reply.
      }
      calls.push({ method: init?.method ?? "GET", url: url.pathname + url.search, body, status: res.status, reply });
      return res;
    },
    // Advance the fake clock, and give the fake network a real moment.
    // A long wait (the open deadline) passes in steps of at most 1 s, so a link that opens
    // meanwhile wins the race, and stops once its abort signal fires.
    sleep: async (ms, abort) => {
      let left = ms;
      do {
        if (abort?.aborted) return new Promise<void>(() => {});
        const step = Math.min(left, 1000);
        now += step;
        left -= step;
        await new Promise((r) => setTimeout(r, 2));
      } while (left > 0);
    },
    now: () => now,
  };
  return { net, deps, urls, calls, elapsed: () => now };
}

const tick = (ms = 5) => new Promise((r) => setTimeout(r, ms));

/** Runs `body`, failing if any promise rejection goes unhandled meanwhile (or just after). */
async function noUnhandledRejections(body: () => Promise<void>): Promise<void> {
  const unhandled: unknown[] = [];
  const onRejection = (reason: unknown) => unhandled.push(reason);
  process.on("unhandledRejection", onRejection);
  try {
    await body();
    await tick(20);
  } finally {
    process.off("unhandledRejection", onRejection);
  }
  expect(unhandled).toEqual([]);
}

/** A fetch that answers `status` (with `headers`) to every call, counting them. */
function always(status: number, headers: Record<string, string> = {}) {
  const fetched = { count: 0 };
  const fetch = async () => {
    fetched.count++;
    return new Response(null, { status, headers });
  };
  return { fetch, fetched };
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
    await expect(joinRoom("ZZZZZZZZ", deps)).rejects.toThrow("Room closed or expired.");
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
    await expect(hostRoom(deps)).rejects.toThrow("Signaling unavailable.");

    const offline = harness();
    offline.deps.fetch = () => Promise.reject(new TypeError("Failed to fetch"));
    await expect(joinRoom("ABCDEFGH", offline.deps)).rejects.toThrow(/could not reach/i);

    const junk = harness();
    junk.deps.fetch = async () => new Response("<html>", { status: 200 });
    await expect(hostRoom(junk.deps)).rejects.toThrow(/unexpected reply/i);
  });

  test("429_maps_to_busy_with_minutes", async () => {
    for (const [retryAfter, minutes] of [
      ["1", 1],
      ["60", 1],
      ["61", 2],
      ["600", 10],
    ] as const) {
      const busy = harness();
      busy.deps.fetch = always(429, { "Retry-After": retryAfter }).fetch;
      const text = `Signaling server busy, try again in ${minutes} min.`;
      await expect(hostRoom(busy.deps)).rejects.toThrow(text);
      await expect(joinRoom("ABCDEFGH", busy.deps)).rejects.toThrow(text);
    }
    // A missing or malformed Retry-After still gives a busy message (ruling R-T10-1, Milestone 11).
    for (const headers of [
      {},
      { "Retry-After": "soon" },
      { "Retry-After": "Wed, 21 Oct 2015 07:28:00 GMT" },
      { "Retry-After": "-5" },
    ]) {
      const busy = harness();
      busy.deps.fetch = always(429, headers).fetch;
      await expect(hostRoom(busy.deps)).rejects.toThrow("Signaling server busy, try again later.");
    }
  });

  test("a 429 while the host polls stops it with the busy message", async () => {
    const h = harness();
    const real = h.deps.fetch!;
    let polls = 0;
    h.deps.fetch = async (input, init) => {
      if (String(input).includes("want=answer")) {
        polls++;
        return new Response(null, { status: 429, headers: { "Retry-After": "120" } });
      }
      return real(input, init);
    };
    await noUnhandledRejections(async () => {
      const hosted = await hostRoom(h.deps);
      await expect(hosted.connected).rejects.toThrow("Signaling server busy, try again in 2 min.");
    });
    expect(polls).toBe(1);
  });

  test("client_stops_on_500", async () => {
    // Review Focus 4: a store that fails on any call. The server answers a bare 500; the
    // client says "Signaling unavailable." and makes no further request.
    const opOf = (input: string, init?: RequestInit): string => {
      if (init?.method === "POST") return String((JSON.parse(String(init.body)) as { op: string }).op);
      return new URL(input, "http://localhost").searchParams.get("want") === "offer" ? "offer-poll" : "answer-poll";
    };
    for (const failing of ["create", "answer-poll", "offer-poll", "answer"]) {
      const h = harness();
      const real = h.deps.fetch!;
      // The failing side's requests after its 500 (the host's, or for a join's ops the joiner's).
      let after = -1;
      const failingFetch: SignalDeps["fetch"] = async (input, init) => {
        if (after >= 0) after++;
        else if (opOf(input, init) === failing) {
          after = 0;
          return new Response(null, { status: 500 });
        }
        return real(input, init);
      };
      await noUnhandledRejections(async () => {
        if (failing === "create" || failing === "answer-poll") {
          h.deps.fetch = failingFetch;
          let hosted;
          try {
            hosted = await hostRoom(h.deps);
          } catch (e) {
            expect(failing).toBe("create");
            expect(e).toBeInstanceOf(SignalError);
            expect((e as Error).message).toBe("Signaling unavailable.");
            return;
          }
          await expect(hosted.connected).rejects.toThrow("Signaling unavailable.");
          expect(hosted.link.state).toBe("closed");
          return;
        }
        const hosted = await hostRoom(h.deps);
        await expect(joinRoom(hosted.room, { ...h.deps, fetch: failingFetch })).rejects.toThrow(
          "Signaling unavailable.",
        );
        hosted.cancel();
        await expect(hosted.connected).rejects.toThrow(/cancelled/i);
      });
      expect(after).toBe(0);
    }
    // A 500 on `close` itself is swallowed by cancel.
    const h = harness();
    const real = h.deps.fetch!;
    let closes = 0;
    h.deps.fetch = async (input, init) => {
      if (opOf(input, init) === "close") {
        closes++;
        return new Response(null, { status: 500 });
      }
      return real(input, init);
    };
    await noUnhandledRejections(async () => {
      const hosted = await hostRoom(h.deps);
      expect(() => hosted.cancel()).not.toThrow();
      await expect(hosted.connected).rejects.toThrow(/cancelled/i);
    });
    expect(closes).toBe(1);
    // So is a network failure, or a fetch that throws synchronously.
    for (const broken of [
      () => Promise.reject(new TypeError("Failed to fetch")),
      () => {
        throw new TypeError("sync");
      },
    ]) {
      const n = harness();
      const realFetch = n.deps.fetch!;
      n.deps.fetch = (input, init) => (opOf(input, init) === "close" ? broken() : realFetch(input, init));
      await noUnhandledRejections(async () => {
        const hosted = await hostRoom(n.deps);
        expect(() => hosted.cancel()).not.toThrow();
        await expect(hosted.connected).rejects.toThrow(/cancelled/i);
      });
    }
  });

  test("cancel_sends_close_with_the_token", async () => {
    const { deps, calls } = harness();
    const hosted = await hostRoom(deps);
    const created = calls.find((c) => c.body?.op === "create")!;
    const token = created.reply!.token as string;
    expect(token).toMatch(/^[A-Za-z0-9_-]{22}$/);
    // The token is not part of what the caller sees.
    expect(JSON.stringify(Object.keys(hosted))).not.toContain("token");
    hosted.cancel();
    hosted.cancel(); // twice: one close
    await tick();
    const closes = calls.filter((c) => c.body?.op === "close");
    expect(closes.map((c) => c.body)).toEqual([{ op: "close", room: hosted.room, token }]);
    expect(closes[0]!.method).toBe("POST");
    expect(closes[0]!.status).toBe(204);
    // Never in a URL.
    for (const c of calls) expect(c.url).not.toContain(token);
  });

  test("join_of_a_closed_room_says_closed", async () => {
    const { deps } = harness();
    const hosted = await hostRoom(deps);
    hosted.cancel();
    await tick();
    await expect(joinRoom(hosted.room, deps)).rejects.toThrow("Room closed or expired.");
  });

  test("waitOpen_resolves_on_the_open_event", async () => {
    const h = harness();
    const hosted = await hostRoom(h.deps);
    // The joiner's clock never moves, and its sleeps never end: only the open event can finish the join.
    const sleeps: number[] = [];
    const joiner: SignalDeps = {
      ...h.deps,
      now: () => 0,
      sleep: (ms) => {
        sleeps.push(ms);
        return new Promise(() => {});
      },
    };
    const link = await joinRoom(hosted.room, joiner);
    expect(link.state).toBe("open");
    await hosted.connected;
    // No polling: at most the one deadline timer, never a poll interval.
    expect(sleeps.every((ms) => ms === SIGNAL_TIMEOUT_MS)).toBe(true);
    expect(sleeps.length).toBeLessThanOrEqual(1);
  });
});

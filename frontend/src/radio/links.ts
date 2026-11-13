/**
 * The page's badge links (Milestone 14 Task 12, from Milestone 10 Task
 * 10's `mountLinks`): WebRTC links to other browsers (`./peer-link.ts`)
 * made through `/api/signal` (`./signal-client.ts`), run into the radio
 * hub. There is no list and no per-link Remove: BADGE LINK shows one
 * summary line.
 *
 * - **Host** creates a room and resolves with its code (or "" when it
 *   failed, or a later host or a cancel replaced it). One hosted room
 *   waits at a time: hosting again, or {@link Links.cancelHost}, cancels
 *   the waiting room, which also closes it on the server, best effort
 *   (Milestone 11 Task 10). A room whose `hostRoom` returns after a later
 *   press or a cancel is closed at once, not orphaned.
 * - **Join** joins a room by code (normalised by the signal client).
 * - Any number of links can be open. Each joins the hub as soon as it
 *   exists, and leaves it when it closes.
 * - **{@link Links.summary}**: "" with no links, `Hosting <room>` while a
 *   hosted room waits, `Linked to N badge(s)` for the open links (both,
 *   joined by " · ", when a room waits beside open links), or else the
 *   signal client's last message ("Creating a room…", "Joining …", a
 *   failure such as "busy N min").
 * - **Notes** (the console's `[page]` lines): a hosted room's code, a
 *   link that opens, a failure's reason, a cancelled room, and an open
 *   link that closes ("A badge link closed."). A link that never opened
 *   says why through its failure instead.
 * - **pagehide** closes every link and cancels a waiting room.
 */
import type { RadioHub } from "./hub";
import type { PeerLink } from "./peer-link";
import { type HostedRoom, SignalError, type SignalDeps, hostRoom, joinRoom } from "./signal-client";

export interface Links {
  /** Hosts a room; resolves with its code, or "" when it failed or was replaced. */
  host(): Promise<string>;
  /** Joins the room `code`; resolves when linked or failed (the failure is a note). */
  join(code: string): Promise<void>;
  /** Stops waiting on the hosted room and closes it. */
  cancelHost(): void;
  /** The waiting hosted room's code, or null. */
  hosting(): string | null;
  summary(): string;
  subscribe(cb: () => void): () => void;
  /** Closes every link and cancels a waiting room (what pagehide does). */
  closeAll(): void;
}

export interface LinksOptions {
  /** Receives the page lines (the console's `appendPage`). */
  note?: (text: string) => void;
  /** Where `pagehide` comes from; the browser's `window` when omitted, none if null. */
  pageEvents?: Pick<EventTarget, "addEventListener" | "removeEventListener"> | null;
}

export const LINK_CLOSED = "A badge link closed.";

function errorText(e: unknown, fallback: string): string {
  return e instanceof SignalError ? e.message : fallback;
}

function defaultPageEvents(): LinksOptions["pageEvents"] {
  return typeof window !== "undefined" ? window : null;
}

export function createLinks(
  hub: Pick<RadioHub, "addLink" | "removeLink">,
  signal: SignalDeps = {},
  options: LinksOptions = {},
): Links {
  const note = options.note ?? (() => {});
  const links = new Map<string, PeerLink>();
  const listeners = new Set<() => void>();
  let hosting: HostedRoom | null = null;
  /** Counts host presses and cancels, so a stale `hostRoom` is closed. */
  let hostSeq = 0;
  /** The signal client's last message, shown when nothing else is. */
  let message = "";
  /** Links closed on purpose (cancel, pagehide): no "closed" note. */
  const quiet = new WeakSet<PeerLink>();

  function changed(): void {
    for (const cb of [...listeners]) {
      try {
        cb();
      } catch (e) {
        console.error("links: listener failed:", e);
      }
    }
  }

  function openCount(): number {
    let n = 0;
    for (const l of links.values()) if (l.state === "open") n++;
    return n;
  }

  function track(link: PeerLink): void {
    links.set(link.id, link);
    hub.addLink(link);
    link.onStateChange((state) => {
      if (state === "open") {
        message = "";
      } else if (state === "closed") {
        links.delete(link.id);
        hub.removeLink(link.id);
      }
      changed();
    });
    // Closed before (or while) it was tracked: it never reports the change.
    if (link.state === "closed") {
      links.delete(link.id);
      hub.removeLink(link.id);
    }
  }

  /** Notes a closed link once it was open; set up after `opened` so a never-opened link stays quiet. */
  function noteWhenClosed(link: PeerLink): void {
    link.onStateChange((state) => {
      if (state === "closed" && !quiet.has(link)) note(LINK_CLOSED);
    });
  }

  function fail(text: string): void {
    message = text;
    note(text);
    changed();
  }

  function cancelHosting(): HostedRoom | null {
    hostSeq++;
    const h = hosting;
    hosting = null;
    if (h) {
      quiet.add(h.link);
      h.cancel();
    }
    return h;
  }

  async function host(): Promise<string> {
    cancelHosting();
    const seq = hostSeq;
    message = "Creating a room…";
    changed();
    let h: HostedRoom;
    try {
      h = await hostRoom(signal);
    } catch (e) {
      if (seq === hostSeq) fail(errorText(e, "Could not host a room."));
      return "";
    }
    if (seq !== hostSeq) {
      // Hosted again, or cancelled, while this room was made.
      quiet.add(h.link);
      h.cancel();
      return "";
    }
    hosting = h;
    message = "";
    track(h.link);
    note(`Hosting room ${h.room}: enter this code in the other browser’s Join link within 2 minutes.`);
    changed();
    h.connected.then(
      () => {
        if (hosting === h) hosting = null;
        note(`Linked to a badge (room ${h.room}).`);
        noteWhenClosed(h.link);
        changed();
      },
      (e: unknown) => {
        if (hosting !== h) return; // replaced or cancelled: its failure is stale
        hosting = null;
        fail(errorText(e, "Linking failed."));
      },
    );
    return h.room;
  }

  async function join(code: string): Promise<void> {
    message = `Joining ${code.trim()}…`;
    changed();
    try {
      const link = await joinRoom(code, signal);
      track(link);
      message = "";
      note("Linked to a badge.");
      noteWhenClosed(link);
      changed();
    } catch (e) {
      fail(errorText(e, "Could not join the room."));
    }
  }

  function closeAll(): void {
    cancelHosting();
    for (const link of [...links.values()]) {
      quiet.add(link);
      link.close();
    }
  }

  const pageEvents = options.pageEvents === undefined ? defaultPageEvents() : options.pageEvents;
  pageEvents?.addEventListener("pagehide", closeAll);

  return {
    host,
    join,
    cancelHost() {
      const h = cancelHosting();
      if (!h) return;
      message = "";
      note(`Stopped hosting room ${h.room}.`);
      changed();
    },
    hosting: () => hosting?.room ?? null,
    summary() {
      const n = openCount();
      const parts: string[] = [];
      if (hosting) parts.push(`Hosting ${hosting.room}`);
      if (n > 0) parts.push(`Linked to ${n} badge${n === 1 ? "" : "s"}`);
      return parts.length > 0 ? parts.join(" · ") : message;
    },
    subscribe(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    closeAll,
  };
}

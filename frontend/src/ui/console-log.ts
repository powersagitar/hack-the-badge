/**
 * The CONSOLE.LOG model (Milestone 14): a bounded list of entries, each a
 * line from one source.
 *
 * - `serial`: the badge's USB serial output, already decoded to text (the
 *   Worker slices it byte-exactly, as `consoleSince` does). It arrives in
 *   arbitrary chunks, so `appendSerial` splits on `\n`, drops every `\r`
 *   (a `\r\n` may straddle two chunks) and holds a trailing partial line
 *   until more text arrives. A held tail longer than `MAX_LINE_CHARS` is
 *   flushed as a line, so a flood with no newline stays bounded.
 * - `page`: the page's own status lines (provisioning, skip, rebooting,
 *   link changes, failures), and the reboot marker. `appendSerial` pushes
 *   `REBOOT_MARKER` when its `boot` argument changes, after flushing the
 *   old boot's held tail as a line of its own.
 * - `event`: reserved for a later "what the visitor did" stream; it only
 *   needs a producer (the spec's "Console sources" seam).
 *
 * Text is never altered beyond the line split: U+FFFD, lone surrogates and
 * control or escape bytes pass through as they came.
 *
 * Bounds and notification: the log keeps the newest `maxLines` entries
 * (2,000 by default, the builder's call within the spec's "thousands").
 * Each append call that adds entries notifies subscribers once, however
 * many lines it adds, and an append that adds none (a partial line)
 * notifies no one. `getSnapshot()` returns a frozen copy, rebuilt lazily
 * at most once per change and otherwise the same array, as
 * `useSyncExternalStore` requires.
 */

export type ConsoleSource = "serial" | "page" | "event";

export interface ConsoleEntry {
  id: number;
  source: ConsoleSource;
  text: string;
}

export interface ConsoleLog {
  /** Appends serial text from boot number `boot` (the soft-reset count). */
  appendSerial(text: string, boot: number): void;
  /** Appends one page status line. */
  appendPage(text: string): void;
  /** The current entries, oldest first. The same array as `getSnapshot()`. */
  entries(): readonly ConsoleEntry[];
  subscribe(cb: () => void): () => void;
  getSnapshot(): readonly ConsoleEntry[];
}

/** The page line that marks a reboot in the serial stream. */
export const REBOOT_MARKER = "— badge rebooted —";

/** A held partial line longer than this is flushed as a line. */
export const MAX_LINE_CHARS = 4096;

export const DEFAULT_MAX_LINES = 2000;

export function createConsoleLog(maxLines = DEFAULT_MAX_LINES): ConsoleLog {
  const cap = Math.max(1, Math.floor(maxLines));
  let lines: ConsoleEntry[] = [];
  let nextId = 0;
  let held = "";
  let lastBoot: number | undefined;
  let snapshot: readonly ConsoleEntry[] = Object.freeze([]);
  let dirty = false;
  const listeners = new Set<() => void>();

  const push = (source: ConsoleSource, text: string) => {
    lines.push({ id: nextId++, source, text });
  };

  /** Trims to the cap and notifies once, if anything was added. */
  const commit = (before: number) => {
    if (nextId === before) return;
    if (lines.length > cap) lines = lines.slice(lines.length - cap);
    dirty = true;
    for (const cb of [...listeners]) cb();
  };

  const getSnapshot = () => {
    if (dirty) {
      snapshot = Object.freeze(lines.slice());
      dirty = false;
    }
    return snapshot;
  };

  return {
    appendSerial(text, boot) {
      const before = nextId;
      if (lastBoot !== undefined && boot !== lastBoot) {
        if (held !== "") push("serial", held);
        held = "";
        push("page", REBOOT_MARKER);
      }
      lastBoot = boot;

      const parts = (held + text.replace(/\r/g, "")).split("\n");
      held = parts.pop() ?? "";
      // Only the newest `cap` complete lines can survive the trim.
      for (let i = Math.max(0, parts.length - cap); i < parts.length; i++) {
        push("serial", parts[i]!);
      }
      if (held.length > MAX_LINE_CHARS) {
        let at = 0;
        for (; held.length - at > MAX_LINE_CHARS; at += MAX_LINE_CHARS) {
          push("serial", held.slice(at, at + MAX_LINE_CHARS));
        }
        held = held.slice(at);
      }
      commit(before);
    },
    appendPage(text) {
      const before = nextId;
      push("page", text);
      commit(before);
    },
    entries: getSnapshot,
    subscribe(cb) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    getSnapshot,
  };
}

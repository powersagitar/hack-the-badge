/**
 * Console helpers shared by the hosts that type into the emulated badge's
 * USB console: the provisioner (`provisioner.ts`) and the Skip-setup typing
 * (Milestone 13).
 *
 * Offsets are absolute byte offsets: a `consoleWritten()` value, the count
 * of bytes the console has ever written. The console keeps only its newest
 * 256 KiB (`CONSOLE_CAPACITY`, emulator-core `console.rs`) but counts on
 * past a wrap and across a soft reset (only a cold `reset()` starts it at
 * 0 again), so an absolute offset stays meaningful where an index into
 * `consoleOutput()` would shift once the ring is full.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";

/** The REPL's prompt. */
export const PROMPT = "badge> ";
/** What ends a REPL line: a single `\n` (`\r\n` would also submit an empty line). */
export const LINE_END = "\n";

const encoder = new TextEncoder();

/**
 * The console's text from absolute byte offset `at` on: the core slices the
 * ring's bytes exactly, then decodes them (lossily). An `at` older than the
 * ring gives the whole ring; one at or past `consoleWritten()` gives "".
 * A slice that starts inside a multi-byte character, or at the ring's start
 * past a wrap, can begin with U+FFFD.
 */
export function consoleSince(handle: FirmwareEmulatorHandle, at: number): string {
  return handle.consoleSince(at);
}

/**
 * The absolute byte offset of `text[i]`, for a `text` just read with
 * {@link consoleSince} (no run in between): counted back from
 * `consoleWritten()` by the UTF-8 length of `text.slice(i)`. Counting from
 * the end stays exact when the slice was the whole ring (an `at` older
 * than it) or began with U+FFFD; `i` is where an ASCII needle was found,
 * so the tail decodes byte for byte.
 */
export function consoleOffsetOf(handle: FirmwareEmulatorHandle, text: string, i: number): number {
  return handle.consoleWritten() - encoder.encode(text.slice(i)).length;
}

/** The complete line of `text` holding a needle at `at`, or `undefined` while it is unterminated. */
export function completeLine(text: string, at: number): string | undefined {
  const start = text.lastIndexOf("\n", at) + 1;
  const end = text.indexOf("\n", at);
  return end < 0 ? undefined : text.slice(start, end).trim();
}

/**
 * Types `line` and [`LINE_END`] into the console. Returns the absolute
 * offset before the line was sent (`consoleWritten()`), where its output
 * starts, or `null` if the serial queue refused any byte.
 */
export function typeLine(handle: FirmwareEmulatorHandle, line: string): number | null {
  const at = handle.consoleWritten();
  const bytes = encoder.encode(`${line}${LINE_END}`);
  return handle.serialInput(bytes) === bytes.length ? at : null;
}

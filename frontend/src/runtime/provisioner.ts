/**
 * Types the registration-desk provisioning flow into the emulated badge's
 * USB console, as a USB host would: `put /littlefs/identity.json <len>`,
 * the JSON bytes once `put` prints `READY`, then `prov apply` — the
 * firmware's own commands (docs/milestone-5-decisions.md, "Trace facts":
 * `REPL_LINE_END`, `PUT_READ`, `APPLY_EFFECTS`).
 *
 * A deliberate copy of `emulator-core/tests/common/provision.rs`'s
 * `provision()` and `set_upload_token()` (a short wire protocol that
 * belongs to the firmware); keep the constants equal to the Rust
 * helper's. It is a state machine polled
 * between `run()` calls: once per frame by the browser's firmware runtime,
 * every few thousand steps by the WASM twin test. It never runs the CPU
 * itself, and it only provisions: walking the onboarding app that follows
 * is the user's job.
 *
 * Payload pacing (decisions file, R-D1-1): the firmware's USB-Serial-JTAG
 * driver drops a packet that does not fit its 256-byte RX ring, and `put`
 * drains about one 64-byte packet per 20-24k steps, so the payload goes
 * out in [`PUT_CHUNK_BYTES`] chunks at least [`PUT_CHUNK_GAP_STEPS`] steps
 * apart (measured with `handle.totalSteps()`), at most one per poll. `put`
 * blocks until all bytes arrive, so a slower poller only takes longer.
 *
 * The first prompt (Milestone 11 Task 13): the console keeps its text
 * across a soft reset, so the old boot's idle `badge> ` is still in it
 * after a reboot, and that REPL is gone. The first `put` is typed only
 * once a prompt appears at or after `bootConsoleStart()`, the console
 * offset where the current boot began (the firmware runtime records it at
 * each soft reset; 0 without one). A prompt the current boot printed
 * before `provision()` still counts: its REPL is idle at it, and prints no
 * new one until it gets a line.
 *
 * The upload credential (Milestone 12): after `PROV OK`, the registration
 * desk's second step types `badge_token set`, waits for
 * `BADGE_TOKEN READY`, sends the 64-byte token in one write (one USB
 * packet, under the RX ring), and waits for `BADGE_TOKEN OK` and the
 * REPL's next prompt; only then is the state `ok`, still carrying the
 * `PROV OK …` line. `BADGE_TOKEN FAIL` or a `usage:` line fails the
 * provision with that line; no answer at all is the caller's timeout
 * (`PROVISION_TIMEOUT_STEPS` in the firmware runtime). A token is valid
 * only for the identity it was set against, so every provision sets it
 * again, re-provisioning from the launcher included. The token waits for
 * a prompt printed after the `PROV OK` line and at or after
 * `bootConsoleStart()`, whichever is later: never `put`'s prompt or the
 * old boot's. On v392 `prov apply` does not reboot, on the first-run
 * screen or on a registered badge (docs/milestone-12-decisions.md,
 * "Traces"); the boot rule only keeps the first prompt's guarantee.
 * Without `uploadToken` the token is [`FAKE_UPLOAD_TOKEN`] (the browser
 * and the WASM twin); `uploadToken: null` skips the token phase and
 * reports `ok` at `PROV OK`, as before Milestone 12. The WASM redraw
 * benchmark skips it, so its rows stay like-for-like with Milestones 7,
 * 9 and 11.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";

/** The REPL's prompt. */
export const PROMPT = "badge> ";
export const IDENTITY_PATH = "/littlefs/identity.json";
/** What ends a REPL line: a single `\n` (`\r\n` would also submit an empty line). */
export const LINE_END = "\n";
/** What `put` prints once it is about to read the payload. */
export const PUT_READY = "READY";
/** The host's chunk size: one USB full-speed bulk packet. */
export const PUT_CHUNK_BYTES = 64;
/**
 * Steps between payload chunks: 2x the measured 24,000-step drain threshold
 * (re-measured under Milestone 6's clock: 24,000 still passes, so unchanged;
 * `PUT_CHUNK_GAP_STEPS` in `emulator-core/tests/common/provision.rs`).
 */
export const PUT_CHUNK_GAP_STEPS = 48_000;

/**
 * The fake upload credential `badge_token set` stores: 64 lowercase hex
 * characters, obviously fake (`0123456789abcdef` four times), one constant
 * for every role. Committed with the organizers' permission (2026-11-10),
 * like the fake identities; it never goes into the identity files.
 * Mirrors `FAKE_UPLOAD_TOKEN` in `emulator-core/tests/common/provision.rs`
 * (test-identities.test.ts checks the two are equal).
 */
export const FAKE_UPLOAD_TOKEN = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/** What `badge_token set` prints before it reads the token's bytes. */
export const BADGE_TOKEN_READY = "BADGE_TOKEN READY";
/** What `badge_token set` prints once the credential is stored. */
export const BADGE_TOKEN_OK = "BADGE_TOKEN OK";
/** What `badge_token set` prints when it refuses the token or cannot store it. */
export const BADGE_TOKEN_FAIL = "BADGE_TOKEN FAIL";
/** The REPL's answer to a malformed command (`usage: badge_token <set|get>`). */
const USAGE = "usage:";

export interface ProvisionOptions {
  /**
   * The upload credential typed after `prov apply`: left out, it is
   * [`FAKE_UPLOAD_TOKEN`]; `null` skips the token phase (the benchmark).
   */
  uploadToken?: string | null;
}

export type ProvisionState =
  | { kind: "waiting" | "typing" }
  | { kind: "ok"; line: string }
  | { kind: "failed"; reason: string };

export interface Provisioner {
  poll(): ProvisionState;
}

/** `put`'s failure messages (decisions file, `PUT_READ`). */
const PUT_ERRORS = ["usage: put", "path too long", "bad size", "write error", "short read"];

/** The complete line of `text` holding `needle` at `at`, or `undefined` while it is unterminated. */
function completeLine(text: string, at: number): string | undefined {
  const start = text.lastIndexOf("\n", at) + 1;
  const end = text.indexOf("\n", at);
  return end < 0 ? undefined : text.slice(start, end).trim();
}

/**
 * `bootConsoleStart` returns the console offset where the current boot
 * began (see the module doc); it is read at every poll, so a reboot while
 * the provisioner waits for its first prompt moves it on.
 */
export function createProvisioner(
  handle: FirmwareEmulatorHandle,
  identityJson: string,
  bootConsoleStart: () => number = () => 0,
  options: ProvisionOptions = {},
): Provisioner {
  const encoder = new TextEncoder();
  const payload = encoder.encode(identityJson);
  const token = options.uploadToken === undefined ? FAKE_UPLOAD_TOKEN : options.uploadToken;
  type Phase =
    | "prompt"
    | "ready"
    | "payload"
    | "putReply"
    | "applyReply"
    | "tokenPrompt"
    | "tokenReady"
    | "tokenReply"
    | "done";
  let phase: Phase = "prompt";
  /**
   * Console offset where the current command's output starts. Offsets stay
   * valid only while the console has not wrapped: it keeps the newest 256 KiB
   * (`CONSOLE_CAPACITY`, emulator-core `console.rs`), far above what boot
   * and provisioning print (an idle badge holds about 2.3 KB). Past a wrap,
   * `since()` is a shifted window: it may re-see older output or, with the
   * buffer full, show nothing new, and the caller's timeout fires. The same
   * holds for `bootConsoleStart()`.
   */
  let marker = 0;
  let sent = 0;
  let lastChunkAt = 0;
  let result: ProvisionState = { kind: "waiting" };
  /** The `PROV OK …` line, reported as `ok` once the token is stored. */
  let provOkLine = "";
  /** Console offset just past the `PROV OK` line: the token's prompt comes after it. */
  let provOkEnd = 0;

  const since = (): string => handle.consoleOutput().slice(marker);

  function fail(reason: string): ProvisionState {
    phase = "done";
    return (result = { kind: "failed", reason });
  }

  /** Sends `bytes`; false (and the provisioner fails) if the queue refuses any. */
  function send(bytes: Uint8Array): boolean {
    if (handle.serialInput(bytes) === bytes.length) return true;
    fail("the serial queue refused the input");
    return false;
  }

  /** Types `line` and starts watching the console from here. */
  function typeLine(line: string): boolean {
    marker = handle.consoleOutput().length;
    return send(encoder.encode(`${line}${LINE_END}`));
  }

  /** The first `put` error line in `text`, once it is complete. */
  function putError(text: string): string | undefined {
    for (const e of PUT_ERRORS) {
      const at = text.indexOf(e);
      if (at >= 0) return completeLine(text, at);
    }
    return undefined;
  }

  /** The complete line holding the first of `needles` in `text`, if any is there. */
  function firstLine(text: string, needles: string[]): string | undefined {
    for (const n of needles) {
      const at = text.indexOf(n);
      if (at >= 0) return completeLine(text, at);
    }
    return undefined;
  }

  function sendChunk(): void {
    const chunk = payload.subarray(sent, sent + PUT_CHUNK_BYTES);
    if (!send(chunk)) return;
    sent += chunk.length;
    lastChunkAt = handle.totalSteps();
    if (sent >= payload.length) phase = "putReply";
  }

  return {
    poll(): ProvisionState {
      switch (phase) {
        case "prompt":
          if (handle.consoleOutput().indexOf(PROMPT, bootConsoleStart()) < 0) return result;
          result = { kind: "typing" };
          if (typeLine(`put ${IDENTITY_PATH} ${payload.length}`)) phase = "ready";
          return result;
        case "ready": {
          const text = since();
          const err = putError(text);
          if (err) return fail(err);
          if (!text.includes(PUT_READY)) return result;
          phase = "payload";
          sendChunk();
          return result;
        }
        case "payload":
          if (handle.totalSteps() - lastChunkAt >= PUT_CHUNK_GAP_STEPS) sendChunk();
          return result;
        case "putReply": {
          const text = since();
          const err = putError(text);
          if (err) return fail(err);
          const ok = text.indexOf(`OK ${payload.length}`);
          // `put` returns to the REPL, which prompts again; type only then.
          if (ok < 0 || text.indexOf(PROMPT, ok) < 0) return result;
          if (typeLine("prov apply")) phase = "applyReply";
          return result;
        }
        case "applyReply": {
          const text = since();
          const okAt = text.indexOf("PROV OK");
          const failAt = text.indexOf("PROV FAIL");
          if (okAt >= 0) {
            const line = completeLine(text, okAt);
            if (line === undefined) return result;
            if (token === null) {
              phase = "done";
              return (result = { kind: "ok", line });
            }
            provOkLine = line;
            provOkEnd = marker + text.indexOf("\n", okAt) + 1;
            phase = "tokenPrompt";
            return result;
          }
          if (failAt >= 0) {
            const line = completeLine(text, failAt);
            return line === undefined ? result : fail(line);
          }
          return result;
        }
        case "tokenPrompt": {
          // The REPL prompts after the `PROV OK` line (and its identity
          // line), in the current boot; never an earlier prompt.
          const from = Math.max(provOkEnd, bootConsoleStart());
          if (handle.consoleOutput().indexOf(PROMPT, from) < 0) return result;
          if (typeLine("badge_token set")) phase = "tokenReady";
          return result;
        }
        case "tokenReady": {
          const text = since();
          const at = text.indexOf(BADGE_TOKEN_READY);
          if (at < 0) {
            const err = firstLine(text, [BADGE_TOKEN_FAIL, USAGE]);
            return err === undefined ? result : fail(err);
          }
          // One 64-byte USB packet, under the 256-byte RX ring.
          marker += at + BADGE_TOKEN_READY.length;
          if (send(encoder.encode(token as string))) phase = "tokenReply";
          return result;
        }
        case "tokenReply": {
          const text = since();
          const failed = firstLine(text, [BADGE_TOKEN_FAIL]);
          if (failed !== undefined) return fail(failed);
          const okAt = text.indexOf(BADGE_TOKEN_OK);
          // `badge_token set` returns to the REPL, which prompts again.
          if (okAt < 0 || text.indexOf(PROMPT, okAt) < 0) return result;
          phase = "done";
          return (result = { kind: "ok", line: provOkLine });
        }
        case "done":
          return result;
      }
    },
  };
}

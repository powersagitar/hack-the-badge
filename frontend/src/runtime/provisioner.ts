/**
 * Types the registration-desk provisioning flow into the emulated badge's
 * USB console, as a USB host would: `put /littlefs/identity.json <len>`,
 * the JSON bytes once `put` prints `READY`, then `prov apply` — the
 * firmware's own commands (docs/milestone-5-decisions.md, "Trace facts":
 * `REPL_LINE_END`, `PUT_READ`, `APPLY_EFFECTS`).
 *
 * A deliberate copy of `emulator-core/tests/common/provision.rs`'s
 * `provision()` (a short wire protocol that belongs to the firmware); keep
 * the constants equal to the Rust helper's. It is a state machine polled
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
/** Steps between payload chunks: 2x the measured 24,000-step drain threshold. */
export const PUT_CHUNK_GAP_STEPS = 48_000;

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

export function createProvisioner(handle: FirmwareEmulatorHandle, identityJson: string): Provisioner {
  const encoder = new TextEncoder();
  const payload = encoder.encode(identityJson);
  type Phase = "prompt" | "ready" | "payload" | "putReply" | "applyReply" | "done";
  let phase: Phase = "prompt";
  /** Console offset where the current command's output starts. */
  let marker = 0;
  let sent = 0;
  let lastChunkAt = 0;
  let result: ProvisionState = { kind: "waiting" };

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
          if (!handle.consoleOutput().includes(PROMPT)) return result;
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
            phase = "done";
            return (result = { kind: "ok", line });
          }
          if (failAt >= 0) {
            const line = completeLine(text, failAt);
            return line === undefined ? result : fail(line);
          }
          return result;
        }
        case "done":
          return result;
      }
    },
  };
}

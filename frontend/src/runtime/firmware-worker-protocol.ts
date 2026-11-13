/**
 * The messages between the main thread (`./firmware-worker-client.ts`) and
 * the firmware Worker (`./firmware-worker.ts`), Milestone 7 Task 8.
 *
 * - `init` carries the WASM module's bytes and `firmware.bin`, both fetched
 *   on the main thread and transferred. The Worker answers `ready` once the
 *   emulator is booted, or `error`.
 * - `frame` carries one framebuffer (RGB565, `width * height` elements) in
 *   a transferred buffer. At most one is in flight: the Worker posts the
 *   next only after the client's `frameAck`, which the client sends after
 *   painting (or after dropping a frame while stopped).
 * - `status` carries the speed reading, whether the reboot's pause is
 *   still being paid (`rebooting`), the provisioning state, the setup
 *   skip's state and whether Setup is on screen (Milestone 13 Task 5; the
 *   software reset count it carried before is the Worker's own now), at
 *   most every 250 ms (`STATUS_INTERVAL_MS`), and at once when the badge
 *   soft-resets (Milestone 11 Task 8). `setupOnScreen` copies the current
 *   boot's console, so it is computed only when a `status` is built.
 * - `skipSetup` starts the setup skip (`./setup-skip.ts`). It has no
 *   fields; the runtime refuses it unless Setup is on screen and nothing
 *   is typing.
 *
 * The radio (Milestone 10 Task 9):
 *
 * - `setMac` gives the badge its per-browser factory MAC. The Worker
 *   applies it once, before the first `start` (the firmware reads it at
 *   boot), and ignores it after that.
 * - `advIn` is a linked peer's advertisement (the peer's `advOut` bytes),
 *   already validated by `../radio/hub.ts`; the Worker checks the types
 *   and bounds again and hands it to the emulator at `rssi` dBm.
 * - `advOut` is this badge's advertisement (`addr(6) ‖ addr_type(1) ‖
 *   data`, empty when it stops advertising). The Worker posts it at most
 *   every `ADV_REPOST_MS` (ruling R-PF-3's 10 Hz bound): on the tick, the
 *   latest when it changed, and a re-post while it advertises. Changes
 *   inside one window are coalesced (Milestone 11 Task 11).
 *
 * Motion, power, LEDs and the console (Milestone 14 Task 6; the canned
 * `shake` and `bump` messages are gone):
 *
 * - `accel` is the motion source's latest reading, in mg per axis. The
 *   client posts at most one per animation frame (the latest wins); the
 *   Worker keeps the latest and applies it at the start of the next
 *   slice. Each value must be a finite number; it is clamped to
 *   ±`ACCEL_LIMIT_MG` and truncated to an integer. A malformed message is
 *   ignored.
 * - `powerCycle` is the board's power switch (the runtime ignores a press
 *   while the badge boots or within 500 ms of the last).
 * - `leds` carries the LED chain's latest frame (R, G, B per LED in chain
 *   order, its buffer transferred; empty when the chain is dark after a
 *   cold power-on), posted when the firmware sent a frame since the last
 *   post, at most every `LED_INTERVAL_MS`.
 * - `console` carries the serial output since the last one: `text` is the
 *   console from absolute byte offset `at` (`consoleSince(at)`) to its
 *   written count, and `boot` the boot it belongs to (soft resets plus
 *   cold power-ons; it only rises). It goes out with a `status` when
 *   there is new text, and at once on a reset: a soft reset's frame posts
 *   the old boot's tail, then an empty chunk with the new `boot`; a power
 *   cycle or `reset` posts the old boot's tail before it, then an empty
 *   chunk with the new `boot` at `at` 0 (the cold power-on empties the
 *   console). A UTF-8 sequence cut off at the end is held for the next
 *   chunk, so a character is never split in two.
 */
import type { ButtonName, ButtonSource } from "./buttons";
import type { ProvisionState } from "./provisioner";
import type { SkipState } from "./setup-skip";

export type ToWorker =
  | { kind: "init"; wasm: ArrayBuffer; image: ArrayBuffer }
  | { kind: "start" }
  | { kind: "stop" }
  /** `source` is optional: a message without one (or with an unknown one) is the pad's. */
  | { kind: "button"; button: ButtonName; pressed: boolean; source?: ButtonSource }
  | { kind: "provision"; identityJson: string }
  | { kind: "skipSetup" }
  | { kind: "accel"; mg: [number, number, number] }
  | { kind: "powerCycle" }
  | { kind: "setMac"; mac: Uint8Array }
  | { kind: "advIn"; bytes: Uint8Array; rssi: number }
  | { kind: "reset" }
  | { kind: "frameAck" }
  | { kind: "dispose" };

export type FromWorker =
  | { kind: "ready" }
  | { kind: "frame"; pixels: ArrayBuffer; width: number; height: number }
  | {
      kind: "status";
      speed: number;
      rebooting: boolean;
      provision: ProvisionState | null;
      skip: SkipState | null;
      setupOnScreen: boolean;
    }
  | { kind: "advOut"; bytes: Uint8Array }
  | { kind: "leds"; rgb: Uint8Array }
  | { kind: "console"; text: string; at: number; boot: number }
  | { kind: "error"; message: string };

/** Least wall time (ms) between two `status` messages. */
export const STATUS_INTERVAL_MS = 250;

/** Least wall time (ms) between two `advOut` messages with the same advertisement (ruling R-PF-3). */
export const ADV_REPOST_MS = 100;

/** Least wall time (ms) between two `leds` messages. */
export const LED_INTERVAL_MS = 50;

/** The largest acceleration (mg) per axis an `accel` message applies; larger ones are clamped. */
export const ACCEL_LIMIT_MG = 2000;

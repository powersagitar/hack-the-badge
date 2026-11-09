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
 * - `status` carries the speed reading, the provisioning state and the
 *   software reset count, at most every 250 ms (`STATUS_INTERVAL_MS`),
 *   and at once when the reset count changes (Milestone 11 Task 8).
 *
 * The radio (Milestone 10 Task 9):
 *
 * - `setMac` gives the badge its per-browser factory MAC. The Worker
 *   applies it once, before the first `start` (the firmware reads it at
 *   boot), and ignores it after that.
 * - `advIn` is a linked peer's advertisement (the peer's `advOut` bytes),
 *   already validated by `../radio/hub.ts`; the Worker checks the types
 *   and bounds again and hands it to the emulator at `rssi` dBm.
 * - `bump` plays the bump pulse on the accelerometer.
 * - `advOut` is this badge's advertisement (`addr(6) ‖ addr_type(1) ‖
 *   data`, empty when it stops advertising). The Worker posts it at most
 *   every `ADV_REPOST_MS` (ruling R-PF-3's 10 Hz bound): on the tick, the
 *   latest when it changed, and a re-post while it advertises. Changes
 *   inside one window are coalesced (Milestone 11 Task 11).
 */
import type { ButtonName, ButtonSource } from "../badge/input";
import type { ProvisionState } from "./provisioner";

export type ToWorker =
  | { kind: "init"; wasm: ArrayBuffer; image: ArrayBuffer }
  | { kind: "start" }
  | { kind: "stop" }
  /** `source` is optional: a message without one (or with an unknown one) is the pad's. */
  | { kind: "button"; button: ButtonName; pressed: boolean; source?: ButtonSource }
  | { kind: "provision"; identityJson: string }
  | { kind: "shake" }
  | { kind: "bump" }
  | { kind: "setMac"; mac: Uint8Array }
  | { kind: "advIn"; bytes: Uint8Array; rssi: number }
  | { kind: "reset" }
  | { kind: "frameAck" }
  | { kind: "dispose" };

export type FromWorker =
  | { kind: "ready" }
  | { kind: "frame"; pixels: ArrayBuffer; width: number; height: number }
  | { kind: "status"; speed: number; provision: ProvisionState | null; softResets: number }
  | { kind: "advOut"; bytes: Uint8Array }
  | { kind: "error"; message: string };

/** Least wall time (ms) between two `status` messages. */
export const STATUS_INTERVAL_MS = 250;

/** Least wall time (ms) between two `advOut` messages with the same advertisement (ruling R-PF-3). */
export const ADV_REPOST_MS = 100;

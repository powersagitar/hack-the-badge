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
 * - `status` carries the speed reading and the provisioning state, at most
 *   every 250 ms (`STATUS_INTERVAL_MS`).
 */
import type { ButtonName } from "../badge/input";
import type { ProvisionState } from "./provisioner";

export type ToWorker =
  | { kind: "init"; wasm: ArrayBuffer; image: ArrayBuffer }
  | { kind: "start" }
  | { kind: "stop" }
  | { kind: "button"; button: ButtonName; pressed: boolean }
  | { kind: "provision"; identityJson: string }
  | { kind: "shake" }
  | { kind: "reset" }
  | { kind: "frameAck" }
  | { kind: "dispose" };

export type FromWorker =
  | { kind: "ready" }
  | { kind: "frame"; pixels: ArrayBuffer; width: number; height: number }
  | { kind: "status"; speed: number; provision: ProvisionState | null }
  | { kind: "error"; message: string };

/** Least wall time (ms) between two `status` messages. */
export const STATUS_INTERVAL_MS = 250;

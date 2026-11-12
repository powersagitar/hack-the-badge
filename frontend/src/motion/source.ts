/**
 * The motion-source seam (spec "Flexibility"): exactly one source drives
 * the emulated SC7A20H at a time. The matter.js lanyard (`./lanyard.ts`)
 * and the phone's DeviceMotion (Task 10) implement it; the Worker and its
 * protocol never know which one is active.
 */

/** One accelerometer sample in the chip's frame, mg per axis (x, y, z). */
export type Mg = [number, number, number];

export interface MotionSource {
  /** Starts delivering samples to `sink`; replaces any earlier sink. */
  start(sink: (mg: Mg) => void): void;
  /** Stops delivering samples. */
  stop(): void;
  /** A linked peer bumped us: show it and let the sensor feel it. */
  peerBump(): void;
  /** A wall impact (or jolt) worth telling linked peers about. */
  onImpact(cb: () => void): () => void;
}

/**
 * Task 5 (Milestone 2, real-firmware mode): a pure, WASM-independent blit
 * function that paints a reconstructed RGB565 pixel buffer onto a 2D canvas.
 *
 * This is deliberately *not* wired to `src/cpu/wasm-pkg` or any live Rust
 * output — per this task's ruling (see the plan/task brief), it takes a
 * plain `Uint16Array` + width/height and a `CanvasRenderingContext2D`,
 * mirroring `emulator-core/src/peripherals/spi.rs`'s `St7789::framebuffer()`
 * shape (RGB565, row-major, `width * height` elements, index
 * `y * width + x`) without depending on it. Task 6 is the only later phase
 * that wires the real `get_framebuffer()` output through the WASM bridge
 * into this already-built, already-tested function.
 *
 * ## RGB565 -> RGBA8888 decode
 *
 * Standard bit-replication expansion (not naive `* 255 / 31` scaling): each
 * channel's low bits are filled in by repeating its own high bits, so pure
 * white (`0xFFFF`) round-trips to `(255,255,255)` exactly and the ramp stays
 * visually even. `R:5,G:6,B:5`, high byte first (matches
 * `emulator-core/src/peripherals/spi.rs`'s documented `RAMWR` byte order —
 * though this function's input is already-assembled `u16` values, not raw
 * bytes, so no endianness decision is needed here).
 *
 * ## `ImageData` availability (judgment call, documented per the task brief)
 *
 * Real browsers construct pixel data via `new ImageData(clampedArray,
 * width, height)` before `ctx.putImageData(...)` — the standard, fast way
 * to blit a full pixel buffer (a single call, vastly cheaper than
 * `width * height` individual `fillRect` calls for a 320x240 = 76,800-pixel
 * panel). Checked via `bun -e 'console.log(typeof ImageData)'`: `ImageData`
 * is **not** a global under `bun test` (prints `undefined`). Rather than
 * degrade the production code path's performance to accommodate the test
 * environment, this module always uses `new ImageData(...)` +
 * `ctx.putImageData(...)` — the actually-correct, actually-performant
 * approach for a real browser — and `test/framebuffer.test.ts` supplies its
 * own minimal test-local `ImageData` shim (assigned onto `globalThis`
 * before the test body runs), following this codebase's existing
 * stub-first convention (see `test/canvas.test.ts`'s fake
 * `CanvasRenderingContext2D`).
 */

export function blitFramebuffer(
  ctx: CanvasRenderingContext2D,
  pixels: Uint16Array,
  width: number,
  height: number,
): void {
  const rgba = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    const px = pixels[i] ?? 0;
    const r5 = (px >> 11) & 0x1f;
    const g6 = (px >> 5) & 0x3f;
    const b5 = px & 0x1f;

    // Bit-replication expansion: fill the low bits with a copy of the high
    // bits, so 0x1f (max 5-bit) -> 255 and 0x00 -> 0 exactly.
    const r8 = (r5 << 3) | (r5 >> 2);
    const g8 = (g6 << 2) | (g6 >> 4);
    const b8 = (b5 << 3) | (b5 >> 2);

    const o = i * 4;
    rgba[o] = r8;
    rgba[o + 1] = g8;
    rgba[o + 2] = b8;
    rgba[o + 3] = 255;
  }

  const imageData = new ImageData(rgba, width, height);
  ctx.putImageData(imageData, 0, 0);
}

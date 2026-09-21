import { describe, expect, test } from "bun:test";
import { blitFramebuffer } from "../src/render/framebuffer";

/**
 * `bun test` has no global `ImageData` (`typeof ImageData === "undefined"`,
 * confirmed via `bun -e 'console.log(typeof ImageData)'`) — see
 * `src/render/framebuffer.ts`'s module doc for why the production code path
 * still uses the real `new ImageData(...)` + `ctx.putImageData(...)`
 * approach rather than degrading to it. This is a minimal test-local shim
 * (this codebase's existing stub-first convention — see
 * `test/canvas.test.ts`) that mimics just enough of the real `ImageData`
 * constructor's shape (`.data`/`.width`/`.height`) for `blitFramebuffer` to
 * construct one and for this test to inspect what it built.
 */
class FakeImageData {
  data: Uint8ClampedArray;
  width: number;
  height: number;
  constructor(data: Uint8ClampedArray, width: number, height: number) {
    this.data = data;
    this.width = width;
    this.height = height;
  }
}
(globalThis as unknown as { ImageData: typeof FakeImageData }).ImageData = FakeImageData;

/**
 * A fake `CanvasRenderingContext2D` that only implements `putImageData`,
 * capturing its argument's `.data` array for pixel-value assertions —
 * `blitFramebuffer` calls nothing else (verified by reading the source).
 */
function makeStubCtx() {
  let captured: FakeImageData | null = null;
  const ctx = {
    putImageData: (imageData: unknown) => {
      captured = imageData as FakeImageData;
    },
  };
  return { ctx: ctx as unknown as CanvasRenderingContext2D, getCaptured: () => captured };
}

function pixelAt(data: Uint8ClampedArray, width: number, x: number, y: number): [number, number, number, number] {
  const o = (y * width + x) * 4;
  return [data[o], data[o + 1], data[o + 2], data[o + 3]];
}

describe("blitFramebuffer", () => {
  test("decodes pure red/green/blue/white RGB565 values exactly", () => {
    const width = 2;
    const height = 2;
    // (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=white -- same fixture shape
    // as emulator-core's spi.rs end-to-end test.
    const pixels = new Uint16Array([0xf800, 0x07e0, 0x001f, 0xffff]);
    const { ctx, getCaptured } = makeStubCtx();

    blitFramebuffer(ctx, pixels, width, height);

    const captured = getCaptured();
    expect(captured).not.toBeNull();
    const data = captured!.data;
    expect(captured!.width).toBe(width);
    expect(captured!.height).toBe(height);

    expect(pixelAt(data, width, 0, 0)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(data, width, 1, 0)).toEqual([0, 255, 0, 255]);
    expect(pixelAt(data, width, 0, 1)).toEqual([0, 0, 255, 255]);
    expect(pixelAt(data, width, 1, 1)).toEqual([255, 255, 255, 255]);
  });

  test("decodes black exactly", () => {
    const { ctx, getCaptured } = makeStubCtx();
    blitFramebuffer(ctx, new Uint16Array([0x0000]), 1, 1);
    expect(pixelAt(getCaptured()!.data, 1, 0, 0)).toEqual([0, 0, 0, 255]);
  });

  test("bit-replication expansion fills low bits (not naive truncation)", () => {
    // A mid-range RGB565 value: R=0b10000 (16/31), G=0b100000 (32/63),
    // B=0b10000 (16/31). Bit-replication: r8 = (16<<3)|(16>>2) = 128|4=132.
    const { ctx, getCaptured } = makeStubCtx();
    const px = (0b10000 << 11) | (0b100000 << 5) | 0b10000;
    blitFramebuffer(ctx, new Uint16Array([px]), 1, 1);
    const [r, g, b, a] = pixelAt(getCaptured()!.data, 1, 0, 0);
    expect(r).toBe(132);
    expect(g).toBe(130); // (32<<2)|(32>>4) = 128|2 = 130
    expect(b).toBe(132);
    expect(a).toBe(255);
  });

  test("handles a larger buffer with known coordinates", () => {
    const width = 4;
    const height = 3;
    const pixels = new Uint16Array(width * height); // defaults to all-black
    // Set a single distinguishable pixel at (3,2) (last row, last column).
    pixels[2 * width + 3] = 0xffff;
    const { ctx, getCaptured } = makeStubCtx();

    blitFramebuffer(ctx, pixels, width, height);

    const data = getCaptured()!.data;
    expect(pixelAt(data, width, 3, 2)).toEqual([255, 255, 255, 255]);
    expect(pixelAt(data, width, 0, 0)).toEqual([0, 0, 0, 255]);
    expect(pixelAt(data, width, 2, 1)).toEqual([0, 0, 0, 255]);
  });
});

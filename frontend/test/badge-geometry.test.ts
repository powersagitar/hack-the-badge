import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { IMG_H, IMG_W } from "../src/motion/lanyard";
import { BUTTON_NAMES } from "../src/runtime/buttons";
import {
  BOARD_CORNERS,
  BUTTON_SPOTS,
  HOLES,
  LED_CHAIN_TO_SPOT,
  LED_SPOTS,
  PHOTO,
  POWER,
  SCREEN,
} from "../src/ui/badge-geometry";

/** A WebP's canvas size, from its VP8X header (the photo has alpha, so it has one). */
function webpSize(bytes: Uint8Array): { w: number; h: number } {
  const tag = (o: number) => String.fromCharCode(...bytes.subarray(o, o + 4));
  expect(tag(0)).toBe("RIFF");
  expect(tag(8)).toBe("WEBP");
  expect(tag(12)).toBe("VP8X");
  const u24 = (o: number) => bytes[o] | (bytes[o + 1] << 8) | (bytes[o + 2] << 16);
  return { w: u24(24) + 1, h: u24(27) + 1 };
}

const inside = ([x, y]: readonly [number, number]) => x > 0 && x < 100 && y > 0 && y < 100;

describe("badge geometry", () => {
  test("every button has a spot inside the photo", () => {
    expect(Object.keys(BUTTON_SPOTS).sort()).toEqual([...BUTTON_NAMES].sort());
    for (const name of BUTTON_NAMES) {
      const { x, y } = BUTTON_SPOTS[name];
      expect(x).toBeGreaterThan(0);
      expect(x).toBeLessThan(100);
      expect(y).toBeGreaterThan(0);
      expect(y).toBeLessThan(100);
    }
  });

  test("mobile hit sizes are larger", () => {
    for (const name of BUTTON_NAMES) {
      const { s } = BUTTON_SPOTS[name];
      expect(s.desktop).toBeGreaterThan(0);
      expect(s.mobile).toBeGreaterThan(s.desktop);
    }
  });

  test("PHOTO is the photo's size, and the lanyard's aspect is the photo's", () => {
    const bytes = new Uint8Array(readFileSync(join(import.meta.dir, "../public", PHOTO.src)));
    expect(webpSize(bytes)).toEqual({ w: PHOTO.w, h: PHOTO.h });
    expect([IMG_W, IMG_H]).toEqual([PHOTO.w, PHOTO.h]);
  });

  test("the LEDs, the holes and the power switch lie inside the photo", () => {
    for (const spot of [...LED_SPOTS, ...HOLES, [POWER.left, POWER.top] as const]) {
      expect(inside(spot)).toBe(true);
    }
  });

  test("SCREEN is 4:3 in the photo's pixels, inside the photo, drawn level", () => {
    const w = (SCREEN.width / 100) * PHOTO.w;
    const h = (SCREEN.height / 100) * PHOTO.h;
    expect(Math.abs(w / h / (4 / 3) - 1)).toBeLessThan(0.005);
    expect(SCREEN.left).toBeGreaterThan(0);
    expect(SCREEN.top).toBeGreaterThan(0);
    expect(SCREEN.left + SCREEN.width).toBeLessThan(100);
    expect(SCREEN.top + SCREEN.height).toBeLessThan(100);
    // Ruling R18: the screen is level on the badge; the canvas never turns.
    expect("rotateDeg" in SCREEN).toBe(false);
  });

  test("each board corner has its own radius, the bottom-right the largest", () => {
    const radii = [BOARD_CORNERS.tl, BOARD_CORNERS.tr, BOARD_CORNERS.br, BOARD_CORNERS.bl];
    for (const r of radii) {
      expect(r).toBeGreaterThan(0);
      expect(r).toBeLessThan(0.25);
    }
    expect(BOARD_CORNERS.br).toBeGreaterThan(1.5 * Math.max(BOARD_CORNERS.tl, BOARD_CORNERS.tr, BOARD_CORNERS.bl));
  });

  test("LED_CHAIN_TO_SPOT is a permutation of 0..5", () => {
    expect(LED_SPOTS.length).toBe(6);
    expect(LED_CHAIN_TO_SPOT.length).toBe(6);
    expect([...LED_CHAIN_TO_SPOT].sort()).toEqual([0, 1, 2, 3, 4, 5]);
  });
});

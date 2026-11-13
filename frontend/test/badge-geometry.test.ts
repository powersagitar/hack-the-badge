import { describe, expect, test } from "bun:test";
import { BUTTON_NAMES } from "../src/runtime/buttons";
import { BUTTON_SPOTS, LED_CHAIN_TO_SPOT, LED_SPOTS } from "../src/ui/badge-geometry";

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

  test("LED_CHAIN_TO_SPOT is a permutation of 0..5", () => {
    expect(LED_SPOTS.length).toBe(6);
    expect(LED_CHAIN_TO_SPOT.length).toBe(6);
    expect([...LED_CHAIN_TO_SPOT].sort()).toEqual([0, 1, 2, 3, 4, 5]);
  });
});

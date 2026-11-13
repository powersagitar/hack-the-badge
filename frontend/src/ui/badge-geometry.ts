/**
 * The badge photo's geometry: the design's values
 * (local/m14-design/Main.dc.html and Mobile.dc.html), in percent of the
 * 974×1513 photo (`public/badge/front.webp`), x from the left and y from
 * the top. Positions are centres; `s` is a hit target's width.
 */
import type { ButtonName } from "../runtime/buttons";

export const PHOTO = { w: 974, h: 1513, src: "/badge/front.webp" };

/** The LCD's rectangle, where the 320×240 framebuffer is drawn. */
export const SCREEN = { left: 15.39, top: 14.76, width: 45.45, height: 21.93 };

/** The power slide switch at the top edge. */
export const POWER = { left: 34.1, top: 1.5, width: 10.5, height: 4 };

/** The lanyard's two mounting holes (the split rings hang in them). */
export const HOLES = [
  [19.59, 3.08],
  [80.18, 2.83],
] as const;

/** The six WS2812s around the board (the design's `LEDS`). */
export const LED_SPOTS = [
  [10.3, 5.58],
  [85.13, 5.45],
  [17.08, 52.44],
  [79.53, 51.85],
  [17.27, 91.19],
  [77.86, 91.19],
] as const;

const DPAD = { desktop: 8.25, mobile: 9.6 };
const ROUND = { desktop: 10.31, mobile: 12.5 };
const SIDE = { desktop: 6.09, mobile: 7.5 };

/** Hit targets; mobile's are bigger (the design's Mobile artboard). AUX1 is the side switch. */
export const BUTTON_SPOTS: Record<ButtonName, { x: number; y: number; s: { desktop: number; mobile: number } }> = {
  UP: { x: 24.44, y: 62.06, s: DPAD },
  LEFT: { x: 17.57, y: 67.11, s: DPAD },
  RIGHT: { x: 31.41, y: 67.11, s: DPAD },
  DOWN: { x: 24.74, y: 72.36, s: DPAD },
  A: { x: 84.34, y: 63.44, s: ROUND },
  B: { x: 69.12, y: 68.2, s: ROUND },
  HOME: { x: 44.18, y: 81.28, s: ROUND },
  START: { x: 59.2, y: 81.28, s: ROUND },
  AUX1: { x: 4.41, y: 37.28, s: SIDE },
};

/** The order the hit targets are drawn (and tabbed through): the design's `BTNS`. */
export const BUTTON_ORDER: readonly ButtonName[] = ["UP", "LEFT", "RIGHT", "DOWN", "A", "B", "HOME", "START", "AUX1"];

/**
 * LED_SPOTS index for each chain index (Task 2's order); Task 14 confirms
 * against the physical badge.
 *
 * A BEST GUESS (Ruling R8): Task 2's evidence shows the chain follows the
 * ring (Setup's fill and chase step 0→5, the shake page lights 3 and 4
 * together) but not where it starts or which way it turns. This walks
 * clockwise from the top-left spot: TL, TR, mid-right, bottom-right,
 * bottom-left, mid-left. See docs/milestone-14-decisions.md, "Badge
 * component (Task 11)".
 */
export const LED_CHAIN_TO_SPOT: readonly number[] = [0, 1, 3, 5, 4, 2];

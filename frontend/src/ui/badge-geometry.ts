/**
 * The badge photo's geometry, in percent of the 974×1523 photo
 * (`public/badge/front.webp`), x from the left and y from the top.
 * Positions are centres; `s` is a hit target's width.
 *
 * The photo is cut from the organizers' orthographic close-up scan and
 * levelled on the board's long straight edges (Task 11c, Ruling R14).
 * Every value is measured on it by `frontend/scripts/badge-features.py`
 * (run through `badge-photo.py --geometry`); see
 * docs/milestone-14-decisions.md, "Badge photo from the scan (Task 11c)".
 */
import type { ButtonName } from "../runtime/buttons";

export const PHOTO = { w: 974, h: 1523, src: "/badge/front.webp" };

/**
 * The board outline's corner radii, as fractions of the photo's width
 * (each an arc tangent to its two edges, fitted to the scan's outline).
 * The bottom-right corner is about twice the others. The photo's top edge
 * is the power switch's actuator, 0.46% above the board's top edge.
 */
export const BOARD_CORNERS = { tl: 0.093, tr: 0.0946, br: 0.189, bl: 0.0964 } as const;

/**
 * The LCD's rectangle, where the 320×240 framebuffer is drawn: 4:3,
 * centred on the flat glass (inside the polarizer's line), as wide as the
 * design's relation to the glass (1% wider than it, 2.2 px over each side;
 * 2.9 px of glass above and below). It is drawn level, unturned: the
 * scan put the glass 0.32° off the board, but the human partner sees
 * the screen perfectly level on the badge (Ruling R18, superseding R14's
 * rotation), so the canvas no longer turns. The rectangle was fitted in
 * the glass's own frame about its centre, so it stays centred.
 */
export const SCREEN = { left: 15.02, top: 14.75, width: 45.96, height: 22.04 };

/** The power slide switch at the top edge: centred on its body; the design's hit size, scaled as the hit sizes below. */
export const POWER = { left: 34.05, top: 1.96, width: 10.57, height: 4 };

/** The lanyard's two mounting holes (the split rings hang in them). */
export const HOLES = [
  [19.47, 3.03],
  [80.46, 3.02],
] as const;

/** The six WS2812-2020s around the board (the design's `LEDS`): the packages' centres. */
export const LED_SPOTS = [
  [10.32, 5.8],
  [85.59, 5.58],
  [16.88, 52.51],
  [79.55, 52.09],
  [16.66, 90.86],
  [77.43, 91.49],
] as const;

/**
 * Hit sizes: the design's, scaled by 1.007, the factor by which the same
 * switches appear larger in this photo than in the previous one (from the
 * eight switch centres' spread), so each keeps its ratio to the real part.
 */
const DPAD = { desktop: 8.31, mobile: 9.67 };
const ROUND = { desktop: 10.38, mobile: 12.59 };
const SIDE = { desktop: 6.13, mobile: 7.55 };

/**
 * Hit targets, centred on the tactile switches' housings; mobile's are
 * bigger (the design's Mobile artboard). AUX1 is the side slide switch's body.
 */
export const BUTTON_SPOTS: Record<ButtonName, { x: number; y: number; s: { desktop: number; mobile: number } }> = {
  UP: { x: 23.72, y: 62.17, s: DPAD },
  LEFT: { x: 16.28, y: 67.19, s: DPAD },
  RIGHT: { x: 30.63, y: 67.29, s: DPAD },
  DOWN: { x: 24.08, y: 72.48, s: DPAD },
  A: { x: 84.31, y: 64.22, s: ROUND },
  B: { x: 68.99, y: 68.68, s: ROUND },
  HOME: { x: 43.77, y: 81.59, s: ROUND },
  START: { x: 59.03, y: 81.5, s: ROUND },
  AUX1: { x: 4.7, y: 37.65, s: SIDE },
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

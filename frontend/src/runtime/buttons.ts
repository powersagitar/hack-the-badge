/**
 * The badge's buttons and the small value types the firmware path and the
 * page share. Moved here from the removed Lua sandbox's `badge/input.ts`
 * and `badge/led.ts` (Milestone 14 Task 4); the tuple's order is unchanged.
 */

export const BUTTON_NAMES = [
  "A",
  "B",
  "HOME",
  "DOWN",
  "LEFT",
  "RIGHT",
  "UP",
  "AUX1",
  "START",
] as const;
export type ButtonName = (typeof BUTTON_NAMES)[number];

/**
 * Where a press or release came from: the keyboard or the on-screen pad.
 * The firmware runtime holds a button down while any source holds it
 * (Milestone 11 Task 13).
 */
export type ButtonSource = "key" | "pad";

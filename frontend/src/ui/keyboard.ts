/**
 * The page's keyboard shortcuts (spec "The badge", "Keyboard impulses"):
 * the arrows, Z/X → A/B, Enter → START and Escape → HOME press the
 * badge's buttons while held; Space toggles the AUX1 side switch; S
 * shakes and K knocks the badge on its lanyard.
 *
 * Keys typed into a field (an input, a select, a textarea or editable
 * content), or into an open list or its trigger (the role Select), are
 * the field's. Enter and Space on a focused button or link
 * are that control's own activation (a keyboard click on a board button
 * taps it), so a keyboard-only visitor can still use every control.
 */
import type { ButtonName } from "../runtime/buttons";

export type KeyAction = { button: ButtonName } | { toggle: "AUX1" } | { impulse: "shake" | "knock" };

const KEYS: Record<string, KeyAction> = {
  ArrowUp: { button: "UP" },
  ArrowDown: { button: "DOWN" },
  ArrowLeft: { button: "LEFT" },
  ArrowRight: { button: "RIGHT" },
  KeyZ: { button: "A" },
  KeyX: { button: "B" },
  Enter: { button: "START" },
  Escape: { button: "HOME" },
  Space: { toggle: "AUX1" },
  KeyS: { impulse: "shake" },
  KeyK: { impulse: "knock" },
};

const FIELDS = new Set(["INPUT", "SELECT", "TEXTAREA"]);
const ACTIVATABLE = new Set(["BUTTON", "A"]);
/**
 * Roles whose element keeps every key: an open list (the role Select's
 * listbox and its options) and its trigger (`combobox`). Radix moves focus
 * into them and uses the arrows, Enter, Space, typeahead and Escape (which
 * closes it), so none of those may also press the badge's buttons.
 */
const LIST_ROLES = new Set(["combobox", "listbox", "option"]);

/** What a key (`KeyboardEvent.code`) does, given the event's target; null for nothing. */
export function keyAction(code: string, target?: unknown): KeyAction | null {
  if (!Object.hasOwn(KEYS, code)) return null;
  const t = target as
    | { tagName?: unknown; isContentEditable?: unknown; getAttribute?: (name: string) => unknown }
    | null
    | undefined;
  const tag = typeof t?.tagName === "string" ? t.tagName.toUpperCase() : "";
  if (FIELDS.has(tag) || t?.isContentEditable === true) return null;
  const role = typeof t?.getAttribute === "function" ? t.getAttribute("role") : null;
  if (typeof role === "string" && LIST_ROLES.has(role)) return null;
  if ((code === "Enter" || code === "Space") && ACTIVATABLE.has(tag)) return null;
  return KEYS[code];
}

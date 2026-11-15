import { describe, expect, test } from "bun:test";
import { keyAction, shortcutsDescription, shortcutsHint } from "../src/ui/keyboard";

/** A stand-in for an event target: only its tag name matters. */
const el = (tagName: string, extra: Record<string, unknown> = {}) => ({ tagName, ...extra });

describe("keyAction", () => {
  test("keyAction maps the documented keys", () => {
    expect(keyAction("ArrowUp")).toEqual({ button: "UP" });
    expect(keyAction("ArrowDown")).toEqual({ button: "DOWN" });
    expect(keyAction("ArrowLeft")).toEqual({ button: "LEFT" });
    expect(keyAction("ArrowRight")).toEqual({ button: "RIGHT" });
    expect(keyAction("KeyZ")).toEqual({ button: "A" });
    expect(keyAction("KeyX")).toEqual({ button: "B" });
    expect(keyAction("Enter")).toEqual({ button: "START" });
    expect(keyAction("Escape")).toEqual({ button: "HOME" });
    expect(keyAction("Space")).toEqual({ toggle: "AUX1" });
    expect(keyAction("KeyS")).toEqual({ impulse: "shake" });
    expect(keyAction("KeyK")).toEqual({ impulse: "knock" });
    expect(keyAction("KeyQ")).toBeNull();
    expect(keyAction("")).toBeNull();
  });

  test("keyAction ignores typing in inputs", () => {
    for (const code of ["ArrowUp", "KeyZ", "Enter", "Escape", "Space", "KeyS", "KeyK"]) {
      expect(keyAction(code, el("INPUT"))).toBeNull();
      expect(keyAction(code, el("SELECT"))).toBeNull();
      expect(keyAction(code, el("TEXTAREA"))).toBeNull();
      expect(keyAction(code, el("DIV", { isContentEditable: true }))).toBeNull();
    }
    expect(keyAction("KeyZ", el("DIV"))).toEqual({ button: "A" });
    expect(keyAction("KeyZ", null)).toEqual({ button: "A" });
  });

  test("a focused button keeps Enter and Space for itself", () => {
    expect(keyAction("Enter", el("BUTTON"))).toBeNull();
    expect(keyAction("Space", el("BUTTON"))).toBeNull();
    expect(keyAction("Enter", el("A"))).toBeNull();
    expect(keyAction("ArrowUp", el("BUTTON"))).toEqual({ button: "UP" });
    expect(keyAction("Escape", el("BUTTON"))).toEqual({ button: "HOME" });
  });

  test("an open list or its trigger keeps every key (the role Select)", () => {
    const role = (tagName: string, r: string) => el(tagName, { getAttribute: (n: string) => (n === "role" ? r : null) });
    for (const code of ["Escape", "ArrowUp", "ArrowDown", "Enter", "Space", "KeyS"]) {
      expect(keyAction(code, role("DIV", "option"))).toBeNull();
      expect(keyAction(code, role("DIV", "listbox"))).toBeNull();
      expect(keyAction(code, role("BUTTON", "combobox"))).toBeNull();
    }
    expect(keyAction("Escape", role("DIV", "img"))).toEqual({ button: "HOME" });
  });
});

describe("shortcuts text", () => {
  test("the hint and the description name every key, and S/K only without phone motion", () => {
    const hint = shortcutsHint(false);
    for (const k of ["ARROWS", "Z/X", "ENTER", "ESC", "SPACE", "S SHAKE", "K KNOCK"]) expect(hint).toContain(k);
    const desc = shortcutsDescription(false);
    for (const k of ["arrow keys", "Z presses A", "X presses B", "Enter", "Escape", "Space", "S shakes", "K knocks"]) {
      expect(desc).toContain(k);
    }
    expect(shortcutsHint(true)).not.toContain("SHAKE");
    expect(shortcutsHint(true)).not.toContain("KNOCK");
    expect(shortcutsHint(true)).toContain("SPACE AUX1");
    expect(shortcutsDescription(true)).not.toContain("S shakes");
    expect(shortcutsDescription(true)).not.toContain("K knocks");
    expect(shortcutsDescription(true)).toContain("Escape presses HOME");
  });
});

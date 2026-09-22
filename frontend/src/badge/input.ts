/**
 * `badge.input` — button state exposed to Lua, plus a TS-side API
 * (`injectButton`) the UI shell uses to feed real button presses in.
 *
 * `BUTTON.*` values are bit flags so `held()` can return a bitmask, matching
 * the documented API shape (`badge.input.held()` -> integer bitmask).
 */
import { lua, to_luastring } from "fengari";
import type { LuaState } from "../lua/interop";

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

export const BUTTON_IDS: Record<ButtonName, number> = BUTTON_NAMES.reduce(
  (acc, name, i) => {
    acc[name] = 1 << i;
    return acc;
  },
  {} as Record<ButtonName, number>,
);

export const KIND = {
  PRESSED: 1,
  RELEASED: 0,
} as const;

export interface ButtonEvent {
  buttonId: number;
  kindId: number;
}

export interface InputModule {
  attach(L: LuaState): void;
  /** UI shell / keyboard bindings call this on press/release. */
  injectButton(button: ButtonName, kind: "pressed" | "released"): void;
  /** Runtime drains queued events each tick to dispatch to `on_button`. */
  drainEvents(): ButtonEvent[];
  /** Current held-button bitmask (for the LED/status HUD, tests, etc). */
  getHeld(): number;
}

export function createInputModule(): InputModule {
  let held = 0;
  const pending: ButtonEvent[] = [];

  function attach(L: LuaState): void {
    lua.lua_newtable(L);

    lua.lua_newtable(L);
    for (const name of BUTTON_NAMES) {
      lua.lua_pushinteger(L, BUTTON_IDS[name]);
      lua.lua_setfield(L, -2, to_luastring(name));
    }
    lua.lua_setfield(L, -2, to_luastring("BUTTON"));

    lua.lua_newtable(L);
    lua.lua_pushinteger(L, KIND.PRESSED);
    lua.lua_setfield(L, -2, to_luastring("PRESSED"));
    lua.lua_pushinteger(L, KIND.RELEASED);
    lua.lua_setfield(L, -2, to_luastring("RELEASED"));
    lua.lua_setfield(L, -2, to_luastring("KIND"));

    lua.lua_pushjsfunction(L, (L2: LuaState) => {
      const id = lua.lua_tointeger(L2, 1) || 0;
      lua.lua_pushboolean(L2, (held & id) !== 0 ? 1 : 0);
      return 1;
    });
    lua.lua_setfield(L, -2, to_luastring("is_down"));

    lua.lua_pushjsfunction(L, (L2: LuaState) => {
      lua.lua_pushinteger(L2, held);
      return 1;
    });
    lua.lua_setfield(L, -2, to_luastring("held"));
  }

  function injectButton(button: ButtonName, kind: "pressed" | "released"): void {
    const id = BUTTON_IDS[button];
    if (id === undefined) return;
    if (kind === "pressed") held |= id;
    else held &= ~id;
    pending.push({ buttonId: id, kindId: kind === "pressed" ? KIND.PRESSED : KIND.RELEASED });
  }

  function drainEvents(): ButtonEvent[] {
    return pending.splice(0, pending.length);
  }

  function getHeld(): number {
    return held;
  }

  return { attach, injectButton, drainEvents, getHeld };
}

/**
 * Low-level helpers for moving values between JS and the Fengari Lua state.
 *
 * These are intentionally generic (no widget-specific logic lives here —
 * see `src/badge/ui.ts` for the hand-rolled widget/userdata bindings, which
 * need identity-preserving behavior these generic converters don't provide).
 */
import { lua, lauxlib, to_luastring } from "fengari";

/** Fengari doesn't ship types, so we use a loose alias for `lua_State*`. */
export type LuaState = any;

const {
  lua_type,
  lua_absindex,
  lua_toboolean,
  lua_tonumber,
  lua_tojsstring,
  lua_rawlen,
  lua_rawgeti,
  lua_next,
  lua_pushnil,
  lua_pushboolean,
  lua_pushnumber,
  lua_pushinteger,
  lua_pushstring,
  lua_pushjsfunction,
  lua_newtable,
  lua_setfield,
  lua_pop,
  lua_gettop,
  LUA_TNIL,
  LUA_TBOOLEAN,
  LUA_TNUMBER,
  LUA_TSTRING,
  LUA_TTABLE,
} = lua;

/** Marker wrapper: a bridged JS function returning this pushes multiple Lua return values. */
export class LuaMulti {
  values: unknown[];
  constructor(...values: unknown[]) {
    this.values = values;
  }
}
export function multi(...values: unknown[]): LuaMulti {
  return new LuaMulti(...values);
}

/** Convert the Lua value at `idx` into a plain JS value (recursively for tables). */
export function luaToJs(L: LuaState, idx: number): any {
  idx = lua_absindex(L, idx);
  const t = lua_type(L, idx);
  switch (t) {
    case LUA_TNIL:
      return undefined;
    case LUA_TBOOLEAN:
      return lua_toboolean(L, idx) !== 0;
    case LUA_TNUMBER:
      return lua_tonumber(L, idx);
    case LUA_TSTRING:
      return lua_tojsstring(L, idx);
    case LUA_TTABLE:
      return luaTableToJs(L, idx);
    default:
      // functions/userdata/threads aren't generically convertible.
      return undefined;
  }
}

function luaTableToJs(L: LuaState, idx: number): any {
  idx = lua_absindex(L, idx);
  const len = lua_rawlen(L, idx);
  if (len > 0) {
    const arr: unknown[] = [];
    for (let i = 1; i <= len; i++) {
      lua_rawgeti(L, idx, i);
      arr.push(luaToJs(L, -1));
      lua_pop(L, 1);
    }
    return arr;
  }
  const obj: Record<string, unknown> = {};
  lua_pushnil(L);
  while (lua_next(L, idx) !== 0) {
    // key at -2, value at -1
    const keyType = lua_type(L, -2);
    const key =
      keyType === LUA_TSTRING
        ? lua_tojsstring(L, -2)
        : String(luaToJs(L, -2));
    obj[key] = luaToJs(L, -1);
    lua_pop(L, 1); // pop value, keep key for lua_next
  }
  return obj;
}

/** Push a JS value onto the Lua stack, converting functions/arrays/objects recursively. */
export function pushJsValue(L: LuaState, val: unknown): void {
  if (val === undefined || val === null) {
    lua_pushnil(L);
  } else if (typeof val === "boolean") {
    lua_pushboolean(L, val ? 1 : 0);
  } else if (typeof val === "number") {
    if (Number.isInteger(val)) lua_pushinteger(L, val);
    else lua_pushnumber(L, val);
  } else if (typeof val === "string") {
    lua_pushstring(L, to_luastring(val));
  } else if (typeof val === "function") {
    lua_pushjsfunction(L, (L2: LuaState) => callBridgedFunction(L2, val as BridgedFn));
  } else if (Array.isArray(val)) {
    lua_newtable(L);
    val.forEach((v, i) => {
      pushJsValue(L, v);
      lua.lua_rawseti(L, -2, i + 1);
    });
  } else if (typeof val === "object") {
    pushNamespace(L, val as Record<string, unknown>);
  } else {
    lua_pushnil(L);
  }
}

/** Push a plain JS object as a fresh Lua table (used for `badge.<namespace>` tables). */
export function pushNamespace(L: LuaState, obj: Record<string, unknown>): void {
  lua_newtable(L);
  for (const key of Object.keys(obj)) {
    pushJsValue(L, obj[key]);
    lua_setfield(L, -2, to_luastring(key));
  }
}

type BridgedFn = (...args: any[]) => unknown;

/**
 * Adapts a plain JS function into a Lua C-function calling convention:
 * reads all Lua arguments off the stack (converted via `luaToJs`), calls the
 * JS function, and pushes back its result. Returning a `LuaMulti` pushes
 * multiple values; returning `undefined` pushes nothing (void call);
 * returning `null` pushes a single `nil`.
 */
function callBridgedFunction(L: LuaState, fn: BridgedFn): number {
  const argc = lua_gettop(L);
  const args: unknown[] = [];
  for (let i = 1; i <= argc; i++) args.push(luaToJs(L, i));

  let result: unknown;
  try {
    result = fn(...args);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return lauxlib.luaL_error(L, to_luastring("%s"), to_luastring(msg));
  }

  if (result instanceof LuaMulti) {
    for (const v of result.values) pushJsValue(L, v);
    return result.values.length;
  }
  if (result === undefined) return 0;
  pushJsValue(L, result);
  return 1;
}

export function pushString(L: LuaState, s: string): void {
  lua_pushstring(L, to_luastring(s));
}

export function toJsString(L: LuaState, idx: number): string {
  return lua_tojsstring(L, idx) ?? "";
}

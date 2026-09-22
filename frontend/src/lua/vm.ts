/**
 * Sandboxed Fengari Lua environment construction, per the badge's documented
 * app sandbox:
 *
 *   Available:  base (minus dofile/loadfile/load/require/pcall/xpcall/
 *               setmetatable) + table, string, math, utf8, plus our own
 *               `require`.
 *   Blocked:    os, io, package, debug, coroutine — never opened, so these
 *               globals simply don't exist.
 *
 * `require(modname)` resolves `<pkg>.<mod>` to `<appdir>/<pkg>/<mod>.lua`
 * (relative to the app directory passed into `createSandboxedEnv`), is
 * cycle-safe, caps require depth at 8, caps distinct modules loaded per app
 * at 16, and caches loaded modules (re-`require`ing returns the same value
 * without re-executing the module body) — standard Lua `require` semantics.
 */
import { lua, lauxlib, lualib, to_luastring } from "fengari";
import type { LuaState } from "./interop";
import { pushJsValue } from "./interop";

export type { LuaState };

/** Synchronous file loader: returns file contents relative to the app dir, or null if missing. */
export type FileLoader = (path: string) => string | null;

export const MAX_MAIN_SOURCE_BYTES = 64 * 1024;
export const MAX_REQUIRE_DEPTH = 8;
export const MAX_REQUIRE_MODULES = 16;

const MODNAME_RE = /^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$/;

interface RequireState {
  /** Stack of module names currently mid-load (cycle detection). */
  loading: string[];
  /** modname -> registry ref of its cached return value. */
  cache: Map<string, number>;
  /** Distinct module names loaded so far (for the 16-module cap). */
  loadOrder: string[];
}

export interface LuaEnv {
  L: LuaState;
  appDir: string;
  fileLoader: FileLoader;
  close(): void;
}

/** Resolve a dotted Lua module name to a `.lua` path, e.g. "foo.bar" -> "foo/bar.lua". */
export function modNameToPath(modname: string): string {
  return modname.split(".").join("/") + ".lua";
}

/** Pure validator, independently testable without a Lua VM. */
export function isValidModName(modname: string): boolean {
  return MODNAME_RE.test(modname);
}

export function createSandboxedEnv(appDir: string, fileLoader: FileLoader): LuaEnv {
  const L = lauxlib.luaL_newstate();
  if (!L) throw new Error("Failed to create Lua state (out of memory?)");

  // Open exactly the libraries the sandbox allows, and set them as globals.
  openAsGlobal(L, "_G", lualib.luaopen_base);
  openAsGlobal(L, "table", lualib.luaopen_table);
  openAsGlobal(L, "string", lualib.luaopen_string);
  openAsGlobal(L, "math", lualib.luaopen_math);
  openAsGlobal(L, "utf8", lualib.luaopen_utf8);
  // os, io, package, debug, coroutine are intentionally never opened.

  // Strip the base-library entry points the sandbox forbids.
  for (const name of ["dofile", "loadfile", "load", "pcall", "xpcall", "setmetatable"]) {
    lua.lua_pushnil(L);
    lua.lua_setglobal(L, to_luastring(name));
  }

  const requireState: RequireState = { loading: [], cache: new Map(), loadOrder: [] };
  lua.lua_pushjsfunction(L, (L2: LuaState) => luaRequire(L2, fileLoader, requireState));
  lua.lua_setglobal(L, to_luastring("require"));

  return {
    L,
    appDir,
    fileLoader,
    close() {
      // Fengari has no explicit lua_close in the JS port's public surface we
      // rely on; dropping all references lets the JS GC reclaim the state.
    },
  };
}

function openAsGlobal(L: LuaState, name: string, openf: (L: LuaState) => number): void {
  lauxlib.luaL_requiref(L, to_luastring(name), openf, 1);
  lua.lua_pop(L, 1);
}

function luaRequire(L: LuaState, fileLoader: FileLoader, state: RequireState): number {
  if (lua.lua_type(L, 1) !== lua.LUA_TSTRING) {
    return lauxlib.luaL_error(L, to_luastring("require: module name must be a string"));
  }
  const modname = lua.lua_tojsstring(L, 1);

  if (!isValidModName(modname)) {
    return lauxlib.luaL_error(
      L,
      to_luastring("require: invalid module name '%s'"),
      to_luastring(modname),
    );
  }

  const cachedRef = state.cache.get(modname);
  if (cachedRef !== undefined) {
    lua.lua_rawgeti(L, lua.LUA_REGISTRYINDEX, cachedRef);
    return 1;
  }

  if (state.loading.includes(modname)) {
    return lauxlib.luaL_error(
      L,
      to_luastring("require: cycle detected ('%s' requires itself transitively: %s)"),
      to_luastring(modname),
      to_luastring([...state.loading, modname].join(" -> ")),
    );
  }

  if (state.loading.length >= MAX_REQUIRE_DEPTH) {
    return lauxlib.luaL_error(
      L,
      to_luastring("require: max require depth of %d exceeded"),
      MAX_REQUIRE_DEPTH,
    );
  }

  if (state.loadOrder.length >= MAX_REQUIRE_MODULES) {
    return lauxlib.luaL_error(
      L,
      to_luastring("require: too many modules loaded (max %d per app)"),
      MAX_REQUIRE_MODULES,
    );
  }

  const relPath = modNameToPath(modname);
  const source = fileLoader(relPath);
  if (source === null || source === undefined) {
    return lauxlib.luaL_error(
      L,
      to_luastring("module '%s' not found (looked for %s)"),
      to_luastring(modname),
      to_luastring(relPath),
    );
  }

  state.loading.push(modname);
  state.loadOrder.push(modname);

  const bytes = to_luastring(source);
  const loadStatus = lauxlib.luaL_loadbuffer(L, bytes, bytes.length, to_luastring("@" + relPath));
  if (loadStatus !== lua.LUA_OK) {
    state.loading.pop();
    return lua.lua_error(L); // error message is already on the stack
  }

  const callStatus = lua.lua_pcall(L, 0, 1, 0);
  state.loading.pop();
  if (callStatus !== lua.LUA_OK) {
    return lua.lua_error(L);
  }

  // Standard Lua convention: a module that returns nothing loads as `true`.
  if (lua.lua_isnil(L, -1)) {
    lua.lua_pop(L, 1);
    lua.lua_pushboolean(L, 1);
  }

  const ref = lauxlib.luaL_ref(L, lua.LUA_REGISTRYINDEX); // pops the value, stores a ref
  state.cache.set(modname, ref);
  lua.lua_rawgeti(L, lua.LUA_REGISTRYINDEX, ref);
  return 1;
}

/** Compile + run `main.lua`'s top level (defines on_enter/on_tick/on_button/on_exit as globals). */
export function runMainScript(env: LuaEnv, source: string): void {
  const bytes = to_luastring(source);
  if (bytes.length > MAX_MAIN_SOURCE_BYTES) {
    throw new Error(
      `main.lua is ${bytes.length} bytes, exceeding the ${MAX_MAIN_SOURCE_BYTES}-byte sandbox cap`,
    );
  }
  const { L } = env;
  const loadStatus = lauxlib.luaL_loadbuffer(L, bytes, bytes.length, to_luastring("@main.lua"));
  if (loadStatus !== lua.LUA_OK) {
    throw new Error(`main.lua failed to compile: ${popError(L)}`);
  }
  const callStatus = lua.lua_pcall(L, 0, 0, 0);
  if (callStatus !== lua.LUA_OK) {
    throw new Error(`main.lua raised an error during top-level execution: ${popError(L)}`);
  }
}

function popError(L: LuaState): string {
  const msg = lua.lua_tojsstring(L, -1) ?? "<non-string error>";
  lua.lua_pop(L, 1);
  return msg;
}

/** Push a JS value (or table via `pushJsValue`) as global `name`. */
export function setGlobal(env: LuaEnv, name: string, value: unknown): void {
  pushJsValue(env.L, value);
  lua.lua_setglobal(env.L, to_luastring(name));
}

/** Push whatever `push(L)` puts on the stack top as global `name`. */
export function setGlobalRaw(env: LuaEnv, name: string, push: (L: LuaState) => void): void {
  push(env.L);
  lua.lua_setglobal(env.L, to_luastring(name));
}

export interface CallResult {
  /** Whether the global was actually a function (and thus was invoked). */
  called: boolean;
  /** Error message, if the call raised one (caught, never thrown). */
  error?: string;
}

/**
 * Calls global Lua function `name(...)` if it's defined, catching any Lua
 * error so a misbehaving callback can't crash the emulator. `pushArgs`, if
 * given, pushes arguments onto the stack and returns how many were pushed.
 */
export function callGlobalIfDefined(
  env: LuaEnv,
  name: string,
  pushArgs?: (L: LuaState) => number,
): CallResult {
  const { L } = env;
  const t = lua.lua_getglobal(L, to_luastring(name));
  if (t !== lua.LUA_TFUNCTION) {
    lua.lua_pop(L, 1);
    return { called: false };
  }
  let nargs = 0;
  try {
    nargs = pushArgs ? pushArgs(L) : 0;
  } catch (e) {
    // Failed while preparing arguments; pop the function and bail cleanly.
    lua.lua_pop(L, 1);
    return { called: false, error: e instanceof Error ? e.message : String(e) };
  }
  const status = lua.lua_pcall(L, nargs, 0, 0);
  if (status !== lua.LUA_OK) {
    return { called: true, error: popError(L) };
  }
  return { called: true };
}

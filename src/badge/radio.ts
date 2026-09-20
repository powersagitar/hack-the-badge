/**
 * `badge.radio` — no multi-instance simulation this milestone, so
 * `send()`/`on_recv()` don't actually talk to anything. `on_recv(fn)` still
 * *stores* the callback properly (via a Lua registry ref) rather than
 * silently dropping it — it's just never invoked yet.
 *
 * Implemented by hand (not via the generic namespace bridge in
 * `lua/interop.ts`) specifically for `on_recv`: the generic bridge converts
 * Lua arguments to plain JS values via `luaToJs`, which has no
 * representation for a Lua function value, so a Lua-function argument would
 * be silently lost. Grabbing it with raw Fengari calls avoids that.
 */
import { lua, lauxlib, to_luastring } from "fengari";
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";

const FAKE_MAC = "DE:AD:BE:EF:13:37";

export function createRadioModule() {
  let callbackRef: number | null = null;

  function attach(L: LuaState): void {
    pushNamespace(L, {
      enable() {
        return true;
      },
      disable() {
        // No-op.
      },
      send(_payload: unknown) {
        return true;
      },
      mac() {
        return FAKE_MAC;
      },
      dropped() {
        return 0;
      },
    });

    // `pushNamespace` leaves the freshly built table on the stack top; add
    // `on_recv` onto it directly so it's part of the same `badge.radio` table.
    lua.lua_pushjsfunction(L, (L2: LuaState) => {
      if (lua.lua_type(L2, 1) === lua.LUA_TFUNCTION) {
        if (callbackRef !== null) {
          lauxlib.luaL_unref(L2, lua.LUA_REGISTRYINDEX, callbackRef);
        }
        lua.lua_pushvalue(L2, 1);
        callbackRef = lauxlib.luaL_ref(L2, lua.LUA_REGISTRYINDEX);
      }
      return 0;
    });
    lua.lua_setfield(L, -2, to_luastring("on_recv"));
  }

  return {
    attach,
    hasReceiveCallback(): boolean {
      return callbackRef !== null;
    },
  };
}

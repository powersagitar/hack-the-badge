/**
 * `badge.sys` — clock, logging, RNG, and made-up-but-plausible resource
 * stats. `ms()`/`uptime()` are measured from module construction time,
 * i.e. app load time (matches "since app start" in the spec).
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";

const FIXED_LUA_LIMIT_BYTES = 131072; // 128 KiB, plausible heap_kb-ish budget
const FIXED_FREE_HEAP_BYTES = 196608; // 192 KiB, plausible ESP32-C3 free heap

export interface SysModuleOptions {
  getWidgetCount?: () => number;
  /** Overrides `stats().lua_limit`; defaults to `FIXED_LUA_LIMIT_BYTES`. */
  luaLimitBytes?: number;
}

export function createSysModule(opts: SysModuleOptions = {}) {
  const startTime = now();
  const luaLimitBytes = opts.luaLimitBytes ?? FIXED_LUA_LIMIT_BYTES;

  function elapsedMs(): number {
    return Math.max(0, Math.floor(now() - startTime));
  }

  const api = {
    ms() {
      return elapsedMs();
    },
    uptime() {
      return elapsedMs() / 1000;
    },
    log(...args: unknown[]) {
      console.log("[lua]", ...args);
    },
    random(n?: number) {
      if (n === undefined || n === null) return Math.random();
      const max = Math.max(1, Math.trunc(Number(n)));
      return 1 + Math.floor(Math.random() * max);
    },
    heap() {
      return FIXED_FREE_HEAP_BYTES;
    },
    gc_step() {
      // No-op: fengari's JS GC isn't user-steppable.
    },
    version() {
      return "emu-0.1.0";
    },
    wake_lock(_locked?: boolean) {
      // No-op: no real display/power management to hold a wake lock against.
    },
    stats() {
      return {
        lua_used: 32768,
        lua_peak: 40960,
        lua_limit: luaLimitBytes,
        widgets: opts.getWidgetCount ? opts.getWidgetCount() : 0,
        uptime_ms: elapsedMs(),
        free_heap: FIXED_FREE_HEAP_BYTES,
      };
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

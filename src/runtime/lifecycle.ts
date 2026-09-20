/**
 * App runner: parses `manifest.cfg`, builds the sandboxed Lua env, wires up
 * the `badge.*` global table from `src/badge/*`, creates the root widget,
 * runs `main.lua`, calls `on_enter(root)`, and then drives a tick loop.
 *
 * Tick cadence: `setInterval(tickOnce, ~20ms)`. Chosen over a
 * `requestAnimationFrame` accumulator for simplicity — it doesn't need an
 * accumulator/delta-time dance, and this emulator doesn't need rAF's
 * vsync-alignment. Known tradeoff: browsers throttle `setInterval` in
 * background tabs (to ~1s), so a backgrounded emulator tab will visibly
 * slow down; acceptable for a dev tool.
 *
 * Execution-budget enforcement: the real badge's per-callback ms budgets are
 * NOT implemented (see spec note — this would require preempting a
 * synchronous Lua call mid-flight, which isn't feasible without running the
 * VM off-thread/in a worker; out of scope for this milestone). What IS
 * implemented: every callback invocation is wrapped in Lua's own `pcall`
 * boundary (see `callGlobalIfDefined` in `lua/vm.ts`) so a thrown Lua error
 * never crashes the emulator, plus the documented failure-streak
 * suspension: 3 consecutive `on_tick` failures suspend further ticks, 1
 * `on_button` failure suspends further button dispatch.
 */
import { lua, to_luastring } from "fengari";
import type { LuaState } from "../lua/interop";
import { pushJsValue } from "../lua/interop";
import {
  callGlobalIfDefined,
  createSandboxedEnv,
  runMainScript,
  setGlobalRaw,
  type FileLoader,
  type LuaEnv,
} from "../lua/vm";

import { createAppModule } from "../badge/app";
import { createContactsModule } from "../badge/contacts";
import { createFsModule } from "../badge/fs";
import { type ButtonName, createInputModule } from "../badge/input";
import { createLedModule } from "../badge/led";
import { createMeModule } from "../badge/me";
import { createNfcModule } from "../badge/nfc";
import { createRadioModule } from "../badge/radio";
import { createSensorModule } from "../badge/sensor";
import { createStoreModule } from "../badge/store";
import { createSysModule } from "../badge/sys";
import { createUiModule, type Widget } from "../badge/ui";

export const DEFAULT_TICK_INTERVAL_MS = 20;

export interface AppManifest {
  slug: string;
  name: string;
  icon?: string;
  api?: string;
  heap_kb?: number;
  wake_lock?: boolean;
  home_button?: string;
  confirm_home?: boolean;
  version?: string;
  author?: string;
}

/** Pure INI-like `key=value` parser — independently testable, no Lua VM needed. */
export function parseManifest(text: string): AppManifest {
  const raw: Record<string, string> = {};
  for (const lineRaw of text.split(/\r?\n/)) {
    const line = lineRaw.trim();
    if (!line || line.startsWith("#") || line.startsWith(";")) continue;
    const eq = line.indexOf("=");
    if (eq === -1) continue;
    const key = line.slice(0, eq).trim();
    const value = line.slice(eq + 1).trim();
    if (key) raw[key] = value;
  }
  if (!raw.slug) throw new Error("manifest.cfg: missing required key 'slug'");
  if (!raw.name) throw new Error("manifest.cfg: missing required key 'name'");

  const toBool = (v: string | undefined) => v !== undefined && /^(1|true|yes)$/i.test(v);
  const toIntOpt = (v: string | undefined) =>
    v !== undefined && v !== "" ? parseInt(v, 10) : undefined;

  return {
    slug: raw.slug,
    name: raw.name,
    icon: raw.icon,
    api: raw.api,
    heap_kb: toIntOpt(raw.heap_kb),
    wake_lock: raw.wake_lock !== undefined ? toBool(raw.wake_lock) : undefined,
    home_button: raw.home_button,
    confirm_home: raw.confirm_home !== undefined ? toBool(raw.confirm_home) : undefined,
    version: raw.version,
    author: raw.author,
  };
}

/**
 * Synchronous file loader over HTTP (via a blocking XHR). This is required
 * because Lua's `require()` must return synchronously mid-script — there's
 * no way to `await` inside a Fengari C-function callback. Vite serves
 * `public/` at the site root, so `baseUrl` is e.g. `/apps/smoke-test/`.
 * Deprecated-but-functional; acceptable tradeoff for a local dev emulator.
 */
export function createHttpFileLoader(baseUrl: string): FileLoader {
  const base = baseUrl.endsWith("/") ? baseUrl : baseUrl + "/";
  return (path: string): string | null => {
    try {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", base + path.replace(/^\/+/, ""), false);
      xhr.send(null);
      if (xhr.status >= 200 && xhr.status < 300) return xhr.responseText;
      return null;
    } catch {
      return null;
    }
  };
}

function countWidgets(w: Widget): number {
  let n = 1;
  for (const c of w.children) n += countWidgets(c);
  return n;
}

export interface AppRuntime {
  manifest: AppManifest;
  env: LuaEnv;
  ui: ReturnType<typeof createUiModule>;
  input: ReturnType<typeof createInputModule>;
  led: ReturnType<typeof createLedModule>;
  /** Begins the tick loop (calling `on_enter` first, once). */
  start(intervalMs?: number): void;
  /** Stops the tick loop and calls `on_exit` if defined. */
  stop(): void;
  injectButton(button: ButtonName, kind: "pressed" | "released"): void;
  isExitRequested(): boolean;
}

export function createAppRuntime(manifest: AppManifest, appDir: string, fileLoader: FileLoader): AppRuntime {
  const env = createSandboxedEnv(appDir, fileLoader);

  const ui = createUiModule();
  const input = createInputModule();
  const led = createLedModule();
  const sensor = createSensorModule();
  const store = createStoreModule(manifest.slug);
  const me = createMeModule();
  const contacts = createContactsModule();
  const fs = createFsModule(manifest.slug);
  const nfc = createNfcModule();
  const radio = createRadioModule();

  let exitRequested = false;
  const appApi = createAppModule({ slug: manifest.slug, name: manifest.name }, () => {
    exitRequested = true;
    console.log(`[badge.app.exit] '${manifest.slug}' requested exit (no launcher in this milestone)`);
  });
  const sys = createSysModule({ getWidgetCount: () => countWidgets(ui.root) });

  setGlobalRaw(env, "badge", (L: LuaState) => {
    lua.lua_newtable(L);
    const modules: Array<[string, { attach(L: LuaState): void }]> = [
      ["ui", ui],
      ["input", input],
      ["led", led],
      ["sensor", sensor],
      ["sys", sys],
      ["store", store],
      ["me", me],
      ["contacts", contacts],
      ["app", appApi],
      ["fs", fs],
      ["nfc", nfc],
      ["radio", radio],
    ];
    for (const [name, mod] of modules) {
      mod.attach(L);
      lua.lua_setfield(L, -2, to_luastring(name));
    }
  });

  const mainSource = fileLoader("main.lua");
  if (mainSource === null) {
    throw new Error(`main.lua not found for app '${manifest.slug}' (dir '${appDir}')`);
  }
  runMainScript(env, mainSource);

  let timer: ReturnType<typeof setInterval> | null = null;
  let entered = false;
  let tickFailureStreak = 0;
  let tickSuspended = false;
  let buttonSuspended = false;

  function enter(): void {
    if (entered) return;
    entered = true;
    const res = callGlobalIfDefined(env, "on_enter", (L) => {
      ui.pushWidget(L, ui.root);
      return 1;
    });
    if (res.error) console.error(`[${manifest.slug}] on_enter error:`, res.error);
  }

  function dispatchButtons(): void {
    for (const evt of input.drainEvents()) {
      if (buttonSuspended) continue;
      const res = callGlobalIfDefined(env, "on_button", (L) => {
        pushJsValue(L, evt.buttonId);
        pushJsValue(L, evt.kindId);
        return 2;
      });
      if (res.error) {
        buttonSuspended = true;
        console.error(`[${manifest.slug}] on_button error (button callbacks now suspended):`, res.error);
      }
    }
  }

  function dispatchTick(): void {
    if (tickSuspended) return;
    const res = callGlobalIfDefined(env, "on_tick");
    if (res.error) {
      tickFailureStreak++;
      console.error(`[${manifest.slug}] on_tick error (${tickFailureStreak}/3):`, res.error);
      if (tickFailureStreak >= 3) {
        tickSuspended = true;
        console.error(`[${manifest.slug}] on_tick suspended after 3 consecutive failures`);
      }
    } else if (res.called) {
      tickFailureStreak = 0;
    }
  }

  function tickOnce(): void {
    dispatchButtons();
    dispatchTick();
  }

  function start(intervalMs: number = DEFAULT_TICK_INTERVAL_MS): void {
    enter();
    if (timer !== null) return;
    timer = setInterval(tickOnce, intervalMs);
  }

  function stop(): void {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
    const res = callGlobalIfDefined(env, "on_exit");
    if (res.error) console.error(`[${manifest.slug}] on_exit error:`, res.error);
  }

  return {
    manifest,
    env,
    ui,
    input,
    led,
    start,
    stop,
    injectButton: input.injectButton,
    isExitRequested: () => exitRequested,
  };
}

/** Convenience entry point: fetches+parses `manifest.cfg`, then builds the runtime. */
export function loadApp(appDir: string, fileLoader: FileLoader): AppRuntime {
  const manifestSrc = fileLoader("manifest.cfg");
  if (manifestSrc === null) {
    throw new Error(`manifest.cfg not found for app dir '${appDir}'`);
  }
  const manifest = parseManifest(manifestSrc);
  return createAppRuntime(manifest, appDir, fileLoader);
}

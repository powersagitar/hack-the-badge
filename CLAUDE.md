# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A browser-based emulator for the "Hack the North 2026" hacker badge (an
ESP32-C3 device). The real badge runs hacker-written apps as sandboxed Lua
scripts (`main.lua` + `manifest.cfg`) against a documented `badge.*` API,
rendering to a 320x240 LVGL-based screen. This repo emulates that: a
Fengari (pure-JS Lua 5.3) VM sandbox, a JS implementation of the `badge.*`
API surface, and a `<canvas>` renderer for the LVGL-ish widget tree.

## Commands

Use **Bun** for everything — do not use npm/yarn/pnpm/node directly.

- `bun install` — install dependencies
- `bun run dev` (or `bunx vite`) — start the dev server
- `bun run build` (or `bunx vite build`) — production build to `dist/`
- `bun run preview` — preview a production build
- `bun test` — run unit tests (`bun test path/to/file.test.ts` for a single file)
- `bunx tsc --noEmit` — type-check without emitting

## Architecture

```
src/lua/vm.ts        Sandboxed Fengari Lua env: base+table+string+math+utf8
                      only; os/io/package/debug/coroutine never opened; custom
                      cycle-safe require() (max depth 8, max 16 modules/app);
                      64 KiB cap on main.lua.
src/lua/interop.ts    Generic JS<->Lua value bridge (luaToJs, pushJsValue,
                      pushNamespace) used by the simple badge.* modules.
src/badge/ui.ts       badge.ui: retained-mode widget tree + Lua bindings.
                      Widgets are pushed to Lua as fresh table-of-closures
                      per instance (not metatable userdata) — see the
                      file-level comment for why.
src/badge/*.ts        Other badge.* namespaces (input, led, sensor, sys,
                      store, me, contacts, app, fs, nfc, radio). Most use
                      lua/interop.ts's generic namespace bridge; radio.ts
                      hand-rolls on_recv() to retain a real Lua function ref.
src/runtime/lifecycle.ts
                      Parses manifest.cfg, assembles the `badge` global
                      table, runs main.lua, drives the ~20ms tick loop,
                      dispatches on_enter/on_tick/on_button/on_exit.
src/render/canvas.ts  Pure function: walks a Widget tree, paints it to a
                      2D canvas context. No Lua/DOM coupling — testable
                      with a fake CanvasRenderingContext2D.
src/ui/shell.ts       Page chrome: on-screen button pad + keyboard bindings
                      that call runtime.injectButton(); LED HUD.
src/main.ts           Wires it all together; loads public/apps/smoke-test.
public/apps/<slug>/   Static app bundles (main.lua + manifest.cfg), served
                      by Vite's public/ dir and fetched via a *synchronous*
                      XHR-based fileLoader (required because Lua require()
                      must return synchronously).
```

Module boundaries are intentionally pure where feasible (manifest parsing,
require() path/cycle logic, sandbox global-table construction, widget-tree
mutations, canvas painting) so they're unit-testable without a live browser.
`src/badge/storage.ts` falls back to an in-memory `Storage` shim when
`localStorage` isn't available (e.g. under `bun test`), so `store.ts`/`fs.ts`
stay testable outside a browser too.

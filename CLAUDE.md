# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A browser-based emulator for the "Hack the North 2026" hacker badge (an
ESP32-C3 device), in two complementary modes:

1. **Lua sandbox mode** (Milestone 1): the real badge runs hacker-sideloaded
   apps as sandboxed Lua scripts (`main.lua` + `manifest.cfg`) against a
   documented `badge.*` API. This repo emulates that with a Fengari
   (pure-JS Lua 5.3) VM sandbox, a JS implementation of the `badge.*` API
   surface, and a `<canvas>` renderer for the LVGL-ish widget tree.
2. **Real-firmware mode** (Milestone 2, in progress): the badge's *built-in*
   apps (Snake, Dice, etc.) are native RISC-V machine code baked into one
   monolithic ESP-IDF app image — not extractable as Lua files (confirmed
   by flash-dump forensics; see `emulator-core/`'s doc comments). This mode
   runs the actual dumped firmware (`public/firmware/factory.bin`) against
   a from-scratch ESP32-C3 processor emulator (RV32IMC RISC-V core + a
   minimal peripheral set) written in Rust and compiled to WebAssembly.

Both modes are designed to share the same on-screen button pad/keyboard
input and the same `<canvas>` element, toggled via a mode switch in
`src/ui/shell.ts` — that wiring lands in the final phase of Milestone 2.

## Commands

Use **Bun** for everything JS/TS — do not use npm/yarn/pnpm/node directly.

- `bun install` — install dependencies
- `bun run dev` (or `bunx vite`) — start the dev server
- `bun run build` (or `bunx vite build`) — production build to `dist/`
- `bun run preview` — preview a production build
- `bun test` — run unit tests (`bun test path/to/file.test.ts` for a single file)
- `bunx tsc --noEmit` — type-check without emitting

For the Rust/WASM CPU emulator (`emulator-core/`, `emulator-wasm/`):

- Requires a Rust toolchain (`rustup`) with the `wasm32-unknown-unknown`
  target, plus `wasm-pack` (`cargo install wasm-pack`).
- `cargo test -p emulator-core` — fast native unit tests for the CPU
  core/peripherals (no WASM round-trip; this is the primary iteration loop).
- `bun run build:wasm` — rebuilds `src/cpu/wasm-pkg/` (gitignored generated
  glue + `.wasm` binary) from `emulator-wasm/`. **Must be re-run after any
  change under `emulator-core/`/`emulator-wasm/`**, before `bun run dev`,
  `bun run build`, or `bun test` — nothing else regenerates it automatically.

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
                      Will grow to own both the Lua AppRuntime and the CPU
                      FirmwareRuntime once real-firmware mode lands (see
                      emulator-core/ below) — currently Lua-only.
public/apps/<slug>/   Static app bundles (main.lua + manifest.cfg), served
                      by Vite's public/ dir and fetched via a *synchronous*
                      XHR-based fileLoader (required because Lua require()
                      must return synchronously).

emulator-core/        Pure Rust (no wasm-bindgen deps) — cargo-testable
                      natively. Will hold the RV32IMC RISC-V decode/execute
                      core (src/cpu/), the memory map + flash-image boot
                      loader (src/mem/, boot.rs), and a minimal ESP32-C3
                      peripheral set (src/peripherals/: timer, interrupt
                      controller, GPIO, SPI->ST7789 framebuffer
                      reconstruction) as later phases land; currently just
                      scaffolding + a round-trip smoke test (`add()`).
emulator-wasm/        Thin wasm-bindgen shim over emulator-core, built via
                      `bun run build:wasm` into src/cpu/wasm-pkg/ (gitignored).
                      TS-side consumers (src/cpu/bridge.ts,
                      src/runtime/firmware-runtime.ts, src/render/framebuffer.ts
                      — the CPU-emulator analogs of the Lua-mode files above)
                      land alongside the peripheral phases that need them.
public/firmware/      The dumped real badge firmware: factory.bin (the
                      app partition the CPU emulator boots) and
                      full_flash_dump.bin (the complete 4MB flash, kept for
                      future bootloader/OTA work). Captured read-only via
                      esptool from a physical badge; publication approved
                      by Hack the North organizers ahead of their own
                      open-sourcing of this firmware.
```

Module boundaries are intentionally pure where feasible (manifest parsing,
require() path/cycle logic, sandbox global-table construction, widget-tree
mutations, canvas painting) so they're unit-testable without a live browser.
`src/badge/storage.ts` falls back to an in-memory `Storage` shim when
`localStorage` isn't available (e.g. under `bun test`), so `store.ts`/`fs.ts`
stay testable outside a browser too.

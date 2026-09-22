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
2. **Real-firmware mode** (Milestone 2): the badge's *built-in* apps (Snake,
   Dice, etc.) are native RISC-V machine code baked into one monolithic
   ESP-IDF app image — not extractable as Lua files (confirmed by
   flash-dump forensics; see `docs/firmware-emulator-notes.md`). This mode
   runs the actual dumped firmware (`frontend/public/firmware/factory.bin`) against
   a from-scratch ESP32-C3 processor emulator (RV32IMC RISC-V core + a
   minimal peripheral set) written in Rust and compiled to WebAssembly.
   Real-firmware boot currently runs 2,000,000+ instructions cleanly but
   stalls before reaching any built-in app (a known, documented gap — see
   `docs/firmware-emulator-notes.md`'s "Known limitations" section before
   assuming a built-in app is reachable in this mode).

Both modes share the same on-screen button pad/keyboard input and the same
`<canvas>` element, toggled via a mode switch in `frontend/src/ui/shell.ts` — that
wiring has landed (see `frontend/src/main.ts`).

## Repository layout

The repo root holds three sibling modules plus shared docs:

- `frontend/` — the Vite/TypeScript web app (its own `package.json`,
  `src/`, `public/`, `test/`, `index.html`, configs).
- `emulator-core/` — the pure-Rust CPU/peripheral emulator crate.
- `emulator-wasm/` — the wasm-bindgen shim crate over `emulator-core`.

The root `Cargo.toml` is the Cargo workspace for the two Rust crates.

## Commands

Use **Bun** for everything JS/TS — do not use npm/yarn/pnpm/node directly.
The real package lives in `frontend/`; the root `package.json` is a thin
proxy whose scripts forward to it (`bun run --cwd frontend ...`), so every
command below works from the repo root or from `frontend/`. Cargo commands
run from the repo root.

- `bun install` (in `frontend/`) or `bun run install:frontend` (from root) —
  install dependencies into `frontend/node_modules/`
- `bun run dev` — start the dev server
- `bun run build` — production build to `frontend/dist/`
- `bun run preview` — preview a production build
- `bun test` — run unit tests (`bun test path/to/file.test.ts` for a single file)
- `bun run typecheck` — type-check without emitting (`tsc --noEmit`)

For the Rust/WASM CPU emulator (`emulator-core/`, `emulator-wasm/`):

- Requires a Rust toolchain (`rustup`) with the `wasm32-unknown-unknown`
  target, plus `wasm-pack` (`cargo install wasm-pack`).
- `cargo test -p emulator-core` — fast native unit tests for the CPU
  core/peripherals (no WASM round-trip; this is the primary iteration loop).
- `bun run build:wasm` — rebuilds `frontend/src/cpu/wasm-pkg/` (gitignored
  generated glue + `.wasm` binary) from `emulator-wasm/`. **Must be re-run after any
  change under `emulator-core/`/`emulator-wasm/`**, before `bun run dev`,
  `bun run build`, or `bun test` — nothing else regenerates it automatically.

## Architecture

```
frontend/             Vite + TypeScript web app (own package.json).
  src/lua/vm.ts       Sandboxed Fengari Lua env: base+table+string+math+utf8
                      only; os/io/package/debug/coroutine never opened; custom
                      cycle-safe require() (max depth 8, max 16 modules/app);
                      64 KiB cap on main.lua.
  src/lua/interop.ts  Generic JS<->Lua value bridge (luaToJs, pushJsValue,
                      pushNamespace) used by the simple badge.* modules.
  src/badge/ui.ts     badge.ui: retained-mode widget tree + Lua bindings.
                      Widgets are pushed to Lua as fresh table-of-closures
                      per instance (not metatable userdata) — see the
                      file-level comment for why.
  src/badge/*.ts      Other badge.* namespaces (input, led, sensor, sys,
                      store, me, contacts, app, fs, nfc, radio). Most use
                      lua/interop.ts's generic namespace bridge; radio.ts
                      hand-rolls on_recv() to retain a real Lua function ref.
  src/runtime/lifecycle.ts
                      Parses manifest.cfg, assembles the `badge` global
                      table, runs main.lua, drives the ~20ms tick loop,
                      dispatches on_enter/on_tick/on_button/on_exit.
  src/render/canvas.ts
                      Pure function: walks a Widget tree, paints it to a
                      2D canvas context. No Lua/DOM coupling — testable
                      with a fake CanvasRenderingContext2D.
  src/ui/shell.ts     Page chrome: on-screen button pad + keyboard bindings
                      that call runtime.injectButton(); LED HUD.
  src/main.ts         Wires it all together; loads public/apps/smoke-test.
                      Owns both the Lua AppRuntime and the CPU
                      FirmwareRuntime (see emulator-core/ below), with a
                      mode toggle (src/ui/shell.ts) that switches which one
                      drives the shared canvas/button pad. The `./cpu/bridge`
                      import is dynamic, loaded lazily on first switch to
                      firmware mode, so Lua-only usage has no static
                      dependency on the Rust/WASM toolchain output.
  public/apps/<slug>/ Static app bundles (main.lua + manifest.cfg), served
                      by Vite's public/ dir and fetched via a *synchronous*
                      XHR-based fileLoader (required because Lua require()
                      must return synchronously).
  public/firmware/    The dumped real badge firmware: factory.bin, the app
                      partition the CPU emulator boots. Captured read-only
                      via esptool from a physical badge; publication
                      approved by Hack the North organizers ahead of their
                      own open-sourcing of this firmware. Do not add a full
                      flash dump here — see docs/firmware-emulator-notes.md's
                      "Data-handling note."

emulator-core/        Pure Rust (no wasm-bindgen deps) — cargo-testable
                      natively; the primary iteration loop for this half of
                      the codebase.
  src/cpu/            Generic RV32IMC decode/execute core (registers, CSRs,
                      M-mode trap entry/mret). Deliberately knows nothing
                      about ESP32-C3 specifics — see cpu/mod.rs's module
                      doc. cpu/rom_stubs.rs is the chip-agnostic *mechanism*
                      for intercepting fetches to fixed ROM addresses
                      (paired with the chip-specific data in src/rom.rs,
                      below).
  src/mem/            mem/mod.rs defines the Bus trait the CPU core is
                      generic over. mem/bus.rs's FirmwareBus is the real
                      ESP32-C3 memory map: an ordered sequence of named
                      regions (XIP flash, RAM-copied segments, then one
                      concrete named field per peripheral, then a
                      never-panics catch-all) checked in order on every
                      access — no trait-object dispatch table (a deliberate
                      choice; SPI needs a direct cross-peripheral read of
                      GPIO's D/C pin state, which a trait object would
                      fight). Unmapped *data* access never panics (reads 0,
                      writes drop); unmapped *instruction fetches* always
                      trap — this asymmetry is load-bearing, not an
                      oversight (see docs/firmware-emulator-notes.md).
                      mem/image.rs parses the ESP-IDF app-image format;
                      mem/soc.rs holds the ESP32-C3 address-space ranges.
  src/peripherals/    SYSTIMER + the ESP32-C3 interrupt matrix (not a
                      standard PLIC), GPIO + an emulated 74HC165 button
                      shift register, and SPI2/GPSPI2 + an ST7789
                      command/pixel-stream interpreter that reconstructs a
                      framebuffer. Each module's doc comment cites the
                      exact ESP-IDF v5.5.3 header its register layout came
                      from.
  src/rom.rs          The ESP32-C3-specific mask-ROM HLE stub table (which
                      fixed addresses to intercept + what each pretends to
                      have done), paired with cpu/rom_stubs.rs's generic
                      mechanism above. Addresses sourced from ESP-IDF's own
                      linker scripts, not guessed — see
                      docs/firmware-emulator-notes.md.
  src/boot.rs         "Shortcut boot": loads factory.bin directly into a
                      Cpu/FirmwareBus pair via the app image's own header,
                      skipping mask-ROM/2nd-stage-bootloader emulation
                      entirely (see docs/firmware-emulator-notes.md for
                      why this is safe to skip).
  src/runtime.rs      FirmwareRuntime: the whole emulator as one owned,
                      driveable object. Buttons are addressed by raw slot
                      index here (0 = direct-GPIO START, 1..=8 = shift-
                      register bits) — the human button-name mapping is a
                      UI-layer concern that lives in
                      frontend/src/runtime/firmware-runtime.ts, not here.
emulator-wasm/        Thin wasm-bindgen shim over emulator-core — logic-free
                      by design (see its module doc re: why framebuffer()
                      returns an owned Vec, not a borrowed slice, to avoid a
                      use-after-free across WASM-heap-growing calls). Built
                      via `bun run build:wasm` into
                      frontend/src/cpu/wasm-pkg/ (gitignored). Consumed by
                      frontend/'s src/cpu/bridge.ts,
                      src/runtime/firmware-runtime.ts, and
                      src/render/framebuffer.ts (the CPU-emulator analogs of
                      the Lua-mode files above), wired up by src/main.ts's
                      mode toggle.
```

See `docs/firmware-emulator-notes.md` for the forensic findings behind this
design (why built-in apps can't be extracted as Lua, the firmware's segment
layout, the physical button/display pin map and its sourcing), the
mask-ROM HLE stub strategy in more detail, and the emulator's current known
limitations (where real-firmware boot stalls today, and what's next).

Module boundaries are intentionally pure where feasible (manifest parsing,
require() path/cycle logic, sandbox global-table construction, widget-tree
mutations, canvas painting) so they're unit-testable without a live browser.
`frontend/src/badge/storage.ts` falls back to an in-memory `Storage` shim when
`localStorage` isn't available (e.g. under `bun test`), so `store.ts`/`fs.ts`
stay testable outside a browser too.

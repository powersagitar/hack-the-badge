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
2. **Real-firmware mode** (Milestones 2–3): the badge's *built-in* apps (Snake,
   Dice, etc.) are native RISC-V machine code baked into one monolithic
   ESP-IDF app image — not extractable as Lua files (confirmed by
   flash-dump forensics; see `docs/firmware-emulator-notes.md`). This mode
   runs the actual dumped firmware (`frontend/public/firmware/factory.bin`) against
   a from-scratch ESP32-C3 processor emulator (RV32IMC RISC-V core + a
   minimal peripheral set) written in Rust and compiled to WebAssembly.
   **Current state (end of Milestone 3):** boot runs through the ESP-IDF
   startup log, FreeRTOS and `app_main`, and draws the firmware's **boot
   splash** to the emulated ST7789 framebuffer (final from step 5,535,126;
   pinned by `boots_to_first_real_frame` in
   `emulator-core/tests/boot_progress.rs`). It then stalls without a
   fault: `load_partitions()` reads the partition table through the flash
   MMU, which is not modeled, gets zeros, and never reaches the app
   launcher or any built-in app. The flash MMU is the next blocker
   (Milestone 4). Read `docs/firmware-emulator-notes.md`'s "Known
   limitations" (current state, open limitations, and the stall-by-stall
   history) before assuming a built-in app is reachable in this mode.

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
- `cargo test -p emulator-core --release --test boot_progress` — the
  real-firmware boot ratchet: each test boots `factory.bin` and asserts a
  console line, a no-fault point, or a framebuffer state (the finish line
  is `boots_to_first_real_frame`). It also passes in a debug build, but the
  multi-million-step boots are much faster with `--release`. When boot
  moves, update the rung whose budget it affects and the module doc's
  per-task status.
- `cargo run -p emulator-core --release --example boot-probe -- [--steps N] [--window W] [--dump-frame PATH]`
  — the "why is boot stuck" diagnostic: console output, run summary, hot
  PCs, unmapped accesses, framebuffer diversity; `--dump-frame` writes a
  PNG under `local/` (relative paths are confined there).

Local-only data (`local/`, gitignored): the full flash dump, the real
serial boot log and the esptool venv (`local/.venv/bin/python`; use it for
any Python, never the system `python3`). Never commit anything from
`local/`, and never quote the boot log's identity lines in code, tests,
docs or commits. Committed tests depend only on `factory.bin`; code that
needs the full dump reads `BADGE_FULL_DUMP` and skips when it is unset. See
the notes' "Data-handling note".

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
                      M-mode trap entry/mret, WFI wait state). Deliberately
                      knows nothing about ESP32-C3 specifics — see
                      cpu/mod.rs's module doc. cpu/rom_stubs.rs is the
                      chip-agnostic *mechanism*
                      for intercepting fetches to fixed ROM addresses and
                      running a stub's effect in place of the real
                      instruction — a return value, a real memcpy/memset,
                      or (as of Milestone 3's Task D3 fix round) a
                      runtime-computed peripheral-register write via
                      `RomStubEffect::BusRegisterWrite` (or a sequence of
                      them, `BusRegisterWrites`, Task D11) — paired with the
                      chip-specific data in src/rom.rs, below. A stub
                      can't run guest code, so ROM routines that call
                      back into firmware (qsort's comparator) are real
                      RV32 code instead; cpu/encode.rs is the const-fn
                      RV32IM encoder those code blobs are assembled with.
  src/md5.rs          Chip-agnostic MD5 in Colin Plumb's context shape
                      (the ROM's md5_context_t layout), shared by the ROM
                      MD5 stubs (RomStubEffect::Md5, Task D13) and
                      peripherals/flash.rs's partition-table MD5 check.
  src/mem/            mem/mod.rs defines the Bus trait the CPU core is
                      generic over. mem/bus.rs's FirmwareBus is the real
                      ESP32-C3 memory map: an ordered sequence of named
                      regions (XIP flash, RAM-copied segments, ROM code
                      and data blobs, then one concrete named field per
                      peripheral, then a
                      never-panics catch-all) checked in order on every
                      access — no trait-object dispatch table (a deliberate
                      choice; SPI needs a direct cross-peripheral read of
                      GPIO's D/C pin state, which a trait object would
                      fight). Unmapped *data* access never panics (reads 0,
                      writes drop); unmapped *instruction fetches* always
                      trap — this asymmetry is load-bearing, not an
                      oversight (see docs/firmware-emulator-notes.md).
                      A `RomCodeBlob` region (Milestone 3 Task D6,
                      `FirmwareBus::map_rom_code`) maps small read-only,
                      executable code blobs at fixed ROM addresses; only
                      the blob's own bytes become fetchable. A
                      `RomDataBlob` region (Task D7,
                      `FirmwareBus::map_rom_data`) maps read-only ROM
                      *data* the same way, but is never fetchable. Each
                      XIP (DROM/IROM) segment is widened to its containing
                      64 KiB flash-cache MMU page (Milestone 3 Task D5,
                      `xip_page_window`), not just its own declared
                      `[load_addr, load_addr+len)`, matching what the real
                      2nd-stage bootloader's page-granular MMU setup
                      exposes — load-bearing for `cpu_start`'s
                      app-image-header check, which reads bytes just before
                      the DROM segment's own `load_addr`.
                      mem/image.rs parses the ESP-IDF app-image format;
                      mem/soc.rs holds the ESP32-C3 address-space ranges
                      plus the ESP32-C3's fixed 64 KiB MMU page size.
  src/peripherals/    SYSTIMER (2 counters, 3 comparators, HAL-faithful
                      alarm sequencing; advance_by/ticks_until_next_alarm
                      for fast-forward, Task 5) + the ESP32-C3 interrupt
                      matrix (not a standard PLIC: 64 source MAP registers
                      onto 32 CPU lines, gated by enable and priority >=
                      threshold, priority 0 = disabled, delivered as levels
                      sampled every step by boot::step_with_interrupts; the
                      core takes the highest-priority pending line, ties to
                      the lowest; while the core waits in WFI that loop
                      fast-forwards SYSTIMER to its next alarm, Task 6),
                      SYSTEM's FROM_CPU software-interrupt registers
                      (system.rs; the rest of SYSTEM is unmapped), GPIO
                      (including the GPIO matrix's FUNCn_IN/OUT_SEL_CFG
                      routing registers, stored but not yet consulted,
                      Task D11) + an emulated
                      74HC165 button shift register, SPI2/GPSPI2 (UPDATE
                      self-clear, TRANS_DONE interrupt, Task 9) + an ST7789
                      command/pixel-stream interpreter that reconstructs a
                      framebuffer, and GDMA (gdma.rs, Task 10: the TX
                      out-link that feeds SPI2 when SPI_DMA_TX_ENA is set;
                      the descriptor walk reads RAM, so it is
                      FirmwareBus::gdma_pull), TIMG0/TIMG1 (timg.rs: RTC
                      slow-clock calibration, inert watchdog storage) and
                      RTC_CNTL (rtc_cntl.rs: the RTC timer and the
                      RTC_XTAL_FREQ_REG store). console.rs is the capped
                      byte sink for firmware output. Its one feed is the
                      USB-Serial-JTAG EP1 TX FIFO (usb_serial_jtag.rs, the
                      badge's real console), written both by firmware
                      directly and by the ROM ets_printf HLE stub. UART0
                      and the ROM putc functions are not modeled.
                      The flash MMU is not modeled (the current stall).
                      Each module's doc comment cites the
                      exact ESP-IDF v5.5.3 header its register layout came
                      from.
  src/peripherals/flash.rs
                      EmulatedFlash (Milestone 3 Task 8): the badge's 4 MiB
                      flash chip as an in-memory array. Synthetic: blank
                      (0xFF) except a partition table at 0x8000 built from
                      committed constants, plus factory.bin at 0x10000.
                      Erase/program use NOR semantics and stay in memory.
                      Never a copy of the physical chip (see the notes'
                      "Emulated flash chip" section). Also Spimem1, the
                      SPI1 flash controller (0x6000_2000): user-command
                      transactions fire on the SPI_MEM_USR byte of CMD and
                      self-clear; only RDID (the badge's JEDEC ID) has an
                      effect so far. The bus holds both as named fields
                      (spimem1, flash_chip); XIP still reads the app image
                      via the D5 page mapping, not through flash_chip.
  src/rom.rs          The ESP32-C3-specific mask-ROM HLE stub table (which
                      fixed addresses to intercept + what each pretends to
                      have done — including, for the six interrupt-matrix/
                      interrupt-controller ROM calls, a real read/write of
                      the `peripherals::intc` registers those calls target,
                      and for ROM `gpio_matrix_out`/`gpio_matrix_in`, of
                      the `peripherals::gpio` matrix registers, with indexed
                      calls bounds-checked),
                      paired with cpu/rom_stubs.rs's generic mechanism
                      above. Also the guest-executed ROM code blobs
                      (`ESP32C3_ROM_CODE`: qsort's one-`jal` jump-table
                      slot plus its insertion-sort body in a ROM range no
                      linker-script symbol points into) and the ROM data
                      tables (`ESP32C3_ROM_DATA`: the `ets_rom_layout_p`
                      layout table, values from Espressif's ROM ELF),
                      and the ROM's writable `.data` boot reads
                      (`esp32c3_rom_ram_initializers`: the SPI-flash legacy
                      data, `chip_size` from the image header, plus the
                      bootloader's `RTC_XTAL_FREQ_REG` store, Task D12), all
                      installed by boot.rs alongside the stub table. Addresses
                      sourced from ESP-IDF's own linker scripts, not
                      guessed — see docs/firmware-emulator-notes.md.
  src/boot.rs         "Shortcut boot": loads factory.bin directly into a
                      Cpu/FirmwareBus pair via the app image's own header,
                      skipping mask-ROM/2nd-stage-bootloader emulation
                      entirely (see docs/firmware-emulator-notes.md for
                      why this is safe to skip). What those skipped steps leave
                      in RAM for the app is re-created by a generic list
                      of boot-time (address, bytes) writes
                      (`apply_ram_initializers`, Task D10); the ESP32-C3
                      data (the ROM's SPI-flash legacy data, and the
                      RTC_CNTL register the bootloader stores the XTAL
                      frequency in, written through the bus) lives in
                      rom.rs.
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

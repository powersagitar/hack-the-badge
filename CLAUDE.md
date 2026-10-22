# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A browser-based emulator for the "Hack the North 2026" hacker badge (an
ESP32-C3 device), in two modes that share one `<canvas>`, one on-screen
button pad/keyboard input, and a mode switch (`frontend/src/ui/shell.ts`,
wired in `frontend/src/main.ts`):

1. **Lua sandbox mode:** runs hacker-sideloaded apps (`main.lua` +
   `manifest.cfg`) against a JS implementation of the badge's `badge.*`
   API, in a Fengari (pure-JS Lua 5.3) sandbox.
2. **Real-firmware mode:** runs the badge's actual dumped firmware
   (`frontend/public/firmware/factory.bin`) on a from-scratch ESP32-C3
   emulator (RV32IMC core + a minimal peripheral set) written in Rust and
   compiled to WebAssembly. The built-in apps (Snake, Dice, …) are native
   RISC-V code in one ESP-IDF image and cannot be extracted as Lua.

**Current state:** the firmware boots to My Badge's first-run screen,
provisions a fake test identity through its own USB console, walks the
11-page onboarding app, and reaches the launcher. Dice and Snake work, and
their key frames were compared with the physical badge. In the browser the
emulator runs in a Web Worker, paced to wall-clock time. Snake plays at
about 1.0x, but the launcher's HOME redraw plays at about 0.6x, because the
WASM core does about 40M steps/s against the badge's 80M. Before assuming
anything else works, read `docs/firmware-emulator-notes.md`, its "Timing
model" and "Known limitations" sections in particular. Design decisions are
in `docs/milestone-3-decisions.md` through `docs/milestone-7-decisions.md`;
the last one has the benchmark results and the Milestone 8 backlog, which a
block cache leads.

## Repository layout

- `frontend/` — the Vite/TypeScript web app (its own `package.json`).
- `emulator-core/` — the pure-Rust CPU/peripheral emulator crate.
- `emulator-wasm/` — the wasm-bindgen shim crate over `emulator-core`.
- Root `Cargo.toml` — the Cargo workspace for the two crates, and the
  build profiles: release at `opt-level = 3`, and the dev profile builds
  `emulator-core` at `opt-level = 3` with debug assertions, so debug runs
  its per-step cross-checks.

## Commands

Use **Bun** for everything JS/TS, never npm/yarn/pnpm/node. The root
`package.json` forwards to `frontend/` (`bun run --cwd frontend ...`), so
every command below works from the repo root or from `frontend/`. Run Cargo
commands from the repo root.

- `bun run install:frontend` (from root) or `bun install` (in `frontend/`).
- `bun run dev`: dev server at http://localhost:5173. It also serves the
  dev-only bench page `/bench.html`.
- `bun run build`: production build to `frontend/dist/`. `bun run preview`
  serves that build on port 4173.
- `bun test`: unit tests, about 11 s. Most of that is the WASM twin that
  provisions and walks to the launcher, `test/cpu-wasm.test.ts`. Run one
  file with `bun test path/to/file.test.ts`.
- `bun run typecheck`: `tsc --noEmit`.

Rust/WASM. These need `rustup` with the `wasm32-unknown-unknown` target,
plus `wasm-pack`.

- `cargo test -p emulator-core`: fast native unit tests, the primary
  iteration loop.
- `bun run build:wasm`: rebuilds `frontend/src/cpu/wasm-pkg/` (gitignored)
  from `emulator-wasm/`. **Re-run it after any change under
  `emulator-core/` or `emulator-wasm/`**, before `bun run dev`,
  `bun run build` or `bun test`. Nothing else regenerates it.
- `cargo test -p emulator-core --release --test boot_progress`: the
  real-firmware boot ratchet, about 8 s.
  - Each rung boots `factory.bin` and asserts a console line, a no-fault
    point or a framebuffer hash.
  - The rungs share cloned `FirmwareRuntime` checkpoints in `OnceLock`s.
  - When boot moves, update the rung whose budget it affects, and put the
    history in the notes, not the test docs.
  - `cargo test --workspace` runs the same rungs in debug, about 12 s.
- `cargo clippy --workspace --all-targets` has 3 known warnings (two in
  `boot.rs`, one in a `console.rs` test).
- Benchmarks are never part of a test run; results go in
  `docs/milestone-7-decisions.md`:
  - `cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture`
    (native);
  - `bun run bench:wasm` (Bun);
  - `bun run bench:safari` (needs `bun run dev` running and Safari's
    remote automation enabled);
  - any browser at `/bench.html`.

  A factor of 1.0x is the badge's speed. Compare WASM rows only with WASM
  rows.
- `cargo run -p emulator-core --release --example boot-probe -- [--steps N] [--window W] [--dump-frame PATH]`:
  the "why is boot stuck" diagnostic. It prints console output, hot PCs,
  unmapped accesses and framebuffer diversity. `--dump-frame` writes a PNG
  under `local/`.

## Rules

- **Performance work must not change emulated behaviour.** Every step
  count, console needle, emulated time and frame hash in the suites stays
  byte-identical. A pin that moves is a bug in the change, never something
  to re-pin. The equivalence rungs `execution_digest_is_unchanged` and
  `hacker_walk_totals_are_unchanged` guard this.
- **Keep the core chip-agnostic.** `emulator-core/src/cpu/` and
  `clock.rs` hold no ESP32-C3 constant. Chip data lives in `rom.rs`,
  `mem/soc.rs` and `peripherals/`.
- `FirmwareBus` uses ordered, concrete named fields, with no `dyn` dispatch.
  SPI reads GPIO's D/C pin directly.
- **Unmapped *data* access never panics** (it reads 0 and drops writes,
  logged). An **unmapped instruction fetch always traps**. This asymmetry
  is load-bearing. Browser input never panics the emulator.
- `emulator-core` has no wasm-bindgen dependency. `emulator-wasm` stays
  logic-free: one-line passthroughs only. `framebuffer()` returns an owned
  `Vec`, to avoid a use-after-free across WASM heap growth.
- Recorded decisions in `docs/milestone-*-decisions.md` stand unless the
  human partner agrees to override one.
- Register layouts come from ESP-IDF v5.5.3 headers, and ROM addresses from
  ESP-IDF's linker scripts. Cite them in the module doc, and never guess.

## Local data

`local/` is gitignored. It holds:
- the full flash dump;
- the real serial boot log;
- badge recordings;
- the esptool venv. Use `local/.venv/bin/python` for any Python, never the
  system `python3`.

The rules for this data:
- Never commit anything from `local/`.
- Never quote the boot log's identity lines anywhere.
- Committed tests depend only on `factory.bin` and committed synthetic data.
  Code that needs the full dump reads `BADGE_FULL_DUMP` and skips when it is
  unset.
- The only committed identity data is the eleven obviously fake identities
  in `frontend/public/firmware/test-identities/`, committed with the
  organizers' permission. A real attendee's identity never goes there.
- Never add a full flash dump to `public/firmware/`.

See the notes' "Data-handling note".

## Architecture

```
frontend/src/
  lua/vm.ts             Sandboxed Fengari env: base, table, string, math and
                        utf8 only; a cycle-safe require() (depth 8, 16
                        modules); 64 KiB main.lua cap.
  lua/interop.ts        Generic JS<->Lua value bridge for badge.* modules.
  badge/*.ts            badge.* namespaces. ui.ts holds the retained widget
                        tree, with each widget a fresh table of closures (see
                        its file comment). radio.ts keeps a Lua function ref.
                        storage.ts falls back to an in-memory shim under
                        `bun test`.
  runtime/lifecycle.ts  Lua mode: manifest parse, the `badge` global, the
                        ~20 ms tick loop, on_enter/tick/button/exit.
  runtime/firmware-runtime.ts
                        Firmware runtime over the WASM handle: wall-clock
                        pacing (runUntil by the wall delta, clamped and
                        step-capped; a capped frame drops its debt), the
                        button name -> raw slot table, the minimum press
                        hold, provision() and shake(). It runs inside the
                        Worker.
  runtime/firmware-worker.ts
                        Web Worker that owns the runtime and runs it in
                        capped slices: one frame in flight, a throttled
                        `status`, and messages applied between slices.
  runtime/firmware-worker-client.ts
                        Main-thread proxy with the same API. It queues
                        messages until ready, paints frames on rAF and acks
                        them, and pauses while the page is hidden.
  runtime/firmware-worker-protocol.ts
                        The message types.
  runtime/provisioner.ts
                        Types `put /littlefs/identity.json` and `prov apply`
                        into the emulated USB console, paced in 64-byte
                        chunks. Mirrors emulator-core's
                        tests/common/provision.rs.
  render/canvas.ts      Pure widget-tree painter, testable with a fake 2D
                        context.
  render/framebuffer.ts Blits the firmware's framebuffer.
  cpu/bridge.ts         Loads the wasm-pkg. Only the Worker imports it, and
                        lazily, so Lua-only use never loads the WASM.
  bench/                The WASM redraw benchmark and the dev-only bench page.
  ui/shell.ts, main.ts  Page chrome, input bindings, the mode toggle and the
                        provisioning controls; main.ts wires it together.
frontend/public/
  apps/<slug>/          Lua app bundles, loaded through a *synchronous* XHR
                        because require() must return synchronously.
  firmware/             factory.bin (publication approved by the organizers)
                        and test-identities/.

emulator-core/src/
  cpu/                  Generic RV32IMC core: CSRs, traps, WFI, the cycle
                        counter. Also rom_stubs.rs (the mechanism that
                        intercepts ROM-address fetches), encode.rs (a const
                        RV32IM encoder for ROM code blobs) and
                        decode_cache.rs (a direct-mapped table validated by
                        the raw bits on every fetch, so it needs no
                        invalidation; 256 KiB per clone).
  clock.rs              Converts steps to 16 MHz SYSTIMER ticks at the
                        firmware-selected CPU frequency, without drift. Every
                        timer reads it.
  md5.rs                MD5 shared by the ROM stubs and the partition table.
  mem/bus.rs            FirmwareBus, the ESP32-C3 memory map: ordered regions
                        (XIP through the flash MMU, RAM segments, ROM code and
                        data blobs, the peripherals, a catch-all). A word fast
                        path tries XIP within one MMU page, then a RAM span
                        map, and otherwise takes the byte path.
  mem/image.rs, soc.rs  App-image parser; the address ranges.
  peripherals/          SYSTIMER, the interrupt matrix (not a PLIC: 64
                        sources onto 32 lines, priority >= threshold), SYSTEM
                        (clock registers, software interrupts), APB_CTRL's
                        RNG (fixed seed), GPIO plus the 74HC165 button shift
                        register, SPI2 plus the ST7789 framebuffer, GDMA,
                        TIMG, RTC_CNTL, USB-Serial-JTAG (the console, in both
                        directions), I2C0 plus the SC7A20H accelerometer, RMT,
                        the flash MMU, and flash.rs (a synthetic 4 MiB flash
                        and the SPI1 controller).
  rom.rs                The ESP32-C3 mask-ROM HLE stub table, the
                        guest-executed ROM code blobs (qsort, strdup, the
                        CPU-frequency pair), the ROM data tables and the
                        boot-time RAM initializers.
  boot.rs               "Shortcut boot": loads factory.bin straight from its
                        header, skipping ROM and bootloader emulation.
                        step_with_interrupts runs one step, with WFI
                        fast-forward.
  runtime.rs            FirmwareRuntime: the whole emulator as one Clone-able
                        object. Buttons by raw slot; serial input; the
                        acceleration; run_until and emulated_time_us.
emulator-wasm/          The logic-free wasm-bindgen shim.
```

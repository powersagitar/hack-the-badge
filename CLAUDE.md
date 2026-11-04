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
2. **Real-firmware mode:** runs the badge's official firmware, v392
   (`v0.1.2-392-gd3089c4`, `frontend/public/firmware/firmware.bin`; the
   physical badge still runs the dumped v335) on a from-scratch ESP32-C3
   emulator (RV32IMC core + a minimal peripheral set) written in Rust and
   compiled to WebAssembly. The built-in apps (Snake, Dice, …) are native
   RISC-V code in one ESP-IDF image and cannot be extracted as Lua.

**Current state:** the firmware boots to My Badge's first-run screen,
provisions a fake test identity through its own USB console, walks the
11-page onboarding app, and reaches the launcher. All 16 of the
launcher's apps open, each pinned by a rung. v392 enters five apps
(Scanner, Sync, Share, Sponsors, Workshop Quest) by rebooting, through
the emulated software reset. Connect and Sync run on a fake BLE
controller that replaces the mask-ROM one at bt.c's VHCI seam, through
app-image hooks at v392's addresses. Two emulated badges pair in Connect
("SYNCED" on both after a bump), in a rung and in two browsers linked
by WebRTC on the same network (checked by the human partner on two
devices). Sync shows "Badge not ready to sync": the fake identities have
no upload credential. No browser can advertise BLE, so emulated badges
hear only each other. Every frame
compared with the physical badge (boot, onboarding, the registered
screen, the launcher, Dice, Snake) was compared on v335; v392's changed
frames and its other apps were checked by the human partner's eye. In
the browser the emulator runs in a Web Worker, paced to wall-clock time.
Snake plays at about 1.0x, and every tap of a quick burst registers (a
per-button tap queue in the browser). On v392, in Bun's WASM bench, a
launcher move (DOWN) plays at about 0.85x and entering the launcher
(the HOME redraw) at about 0.63x, because the WASM core does about 41M
steps/s against the badge's 80M; desktop browsers were not re-measured
on v392, and mobile Safari runs far slower (about 0.0x to 0.2x). Before
assuming anything else works, read
`docs/firmware-emulator-notes.md`, its "Timing model" and "Known
limitations" sections in particular. Design decisions are in
`docs/milestone-3-decisions.md` through `docs/milestone-10-decisions.md`.
Milestone 7's has the benchmark results and profiles. Milestone 9's has
the v335 → v392 re-pins. Milestone 10's has the radio's rulings and the
Milestone 11 backlog.

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
  dev-only bench page `/bench.html` and `/api/signal` over an in-memory
  store. `bun run dev -- --host` serves the LAN, for two-device radio
  links.
- `bun run build`: production build to `frontend/dist/`. `bun run preview`
  serves that build on port 4173.
- `bun test`: unit tests, 275 of them, about 11 s. Most of that is the
  WASM twin that provisions and walks to the launcher,
  `test/cpu-wasm.test.ts`. Run one file with
  `bun test path/to/file.test.ts`.
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
  real-firmware boot ratchet, 87 rungs plus 1 ignored benchmark, about
  16 s.
  - Each rung boots `firmware.bin` and asserts a console line, a no-fault
    point or a framebuffer hash.
  - The rungs share cloned `FirmwareRuntime` checkpoints in `OnceLock`s.
  - When boot moves, update the rung whose budget it affects, and put the
    history in the notes, not the test docs.
  - `cargo test --workspace` runs the same rungs in debug, about 23 s.
- `cargo clippy --workspace --all-targets` has 3 known warnings (two in
  `boot.rs`, one in a `console.rs` test).
- Benchmarks are never part of a test run; results go in
  the current milestone's decisions doc (Milestone 7's has the profiles):
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

- **New dependencies are allowed** (npm or Cargo) wherever a library fits
  better than hand-written code; no need to ask first. Tell the human
  partner about each one you add (package, exact version, why), and pin
  the exact version. The structural rules below still hold: for example,
  `emulator-core` takes no wasm-bindgen dependency.
- **Performance work must not change emulated behaviour.** Every step
  count, console needle, emulated time and frame hash in the suites stays
  byte-identical. A pin that moves is a bug in the change, never something
  to re-pin. The equivalence rungs `execution_digest_is_unchanged` and
  `hacker_walk_totals_are_unchanged` guard this.
- **Keep the core chip-agnostic.** `emulator-core/src/cpu/` and
  `clock.rs` hold no ESP32-C3 or v392 constant. Chip data lives in
  `rom.rs`, `mem/soc.rs` and `peripherals/`; app-image addresses live in
  `radio/v392.rs`, each cited with its disassembly evidence.
- `FirmwareBus` uses ordered, concrete named fields, with no `dyn` dispatch.
  SPI reads GPIO's D/C pin directly.
- **Unmapped *data* access never panics** (it reads 0 and drops writes,
  logged). An **unmapped instruction fetch always traps**. This asymmetry
  is load-bearing. Browser, peer and network input never panics the
  emulator; a malformed peer packet never crashes a tab.
- The radio links use `iceServers: []` (no STUN, TURN or relay), and the
  only server endpoint is `/api/signal`.
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
- Committed tests depend only on `firmware.bin` and committed synthetic data.
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
                        button name -> raw slot table, the per-button tap
                        queue (press hold, release gap), provision() and
                        shake() and bump() (one motion schedule). It runs
                        inside the Worker, which also posts the badge's
                        advertisement (advOut) and takes peers' (advIn).
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
  radio/hub.ts          The radio hub on the main thread: sends the badge's
                        advertisement and bumps to every link, validates
                        peers' messages, holds the latest per peer while
                        paused (hidden page).
  radio/lock.ts         One badge per browser (Web Lock `htn-badge`,
                        take-over), and the per-browser locally
                        administered MAC in localStorage.
  radio/peer-link.ts    A WebRTC link: one ordered data channel,
                        iceServers: [], no trickle; the manual copy-paste
                        codes (deflate-raw, base64url).
  radio/signal-client.ts
                        Hosts or joins a room on /api/signal (1 s polls,
                        120 s budget).
  radio/signal-handler.ts
                        Pure signaling handler (Request in, Response out)
                        over a SignalStore: offer/answer mailbox, 300 s
                        TTL, four rate-limit counters. MemoryStore for dev
                        and tests, UpstashStore (REST) for Vercel.
  render/canvas.ts      Pure widget-tree painter, testable with a fake 2D
                        context.
  render/framebuffer.ts Blits the firmware's framebuffer.
  cpu/bridge.ts         Loads the wasm-pkg. Only the Worker imports it, and
                        lazily, so Lua-only use never loads the WASM.
  bench/                The WASM redraw benchmark and the dev-only bench page.
  ui/shell.ts, main.ts  Page chrome, input bindings, the mode toggle, the
                        provisioning controls, the badge notice and the
                        link panel; main.ts wires it together.
frontend/api/signal.ts  The Vercel function for /api/signal: the handler
                        over Upstash Redis (KV_REST_API_URL/_TOKEN; a
                        missing one is a bare 500). Vite's dev middleware
                        serves the same handler.
frontend/public/
  apps/<slug>/          Lua app bundles, loaded through a *synchronous* XHR
                        because require() must return synchronously.
  firmware/             firmware.bin (official v0.1.2-392-gd3089c4,
                        unpadded; public; publication approved by the
                        organizers) and test-identities/.

emulator-core/src/
  cpu/                  Generic RV32IMC core: CSRs, traps, WFI, the cycle
                        counter. Also rom_stubs.rs (the mechanism that
                        intercepts ROM-address fetches), encode.rs (a const
                        RV32IM encoder for ROM code blobs) and
                        decode_cache.rs (a direct-mapped table validated by
                        the raw bits on every fetch, so it needs no
                        invalidation; 256 KiB per clone). app_hooks.rs:
                        app-image hooks; a step at a hooked pc records a
                        HookHit and raises the attention flag, and the
                        driver returns, passes through or redirects. The
                        per-step check is two ranges learned from the
                        table.
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
                        TIMG, RTC_CNTL (the RTC timer; STORE0..7, kept
                        across a software reset), USB-Serial-JTAG (the
                        console, in both directions), I2C0 plus the SC7A20H
                        accelerometer, RMT, efuse.rs (only the factory MAC,
                        opt-in; set it at power-on),
                        the flash MMU, and flash.rs (a synthetic 4 MiB flash
                        and the SPI1 controller).
  radio/                The fake BLE controller. hci.rs: a pure LE-only HCI
                        model in H4 framing (extended advertising and
                        scanning, event masks, duplicate filter, a bounded
                        event queue; connectionless). mod.rs: Radio, which
                        hooks bt.c's controller init/enable and VHCI
                        functions and feeds NimBLE through a guest
                        trampoline in free mask-ROM space (reports from the
                        idle hook); the peers' wire form and
                        LINKED_PEER_RSSI (-40 dBm). v392.rs: the hook
                        addresses, each with its evidence.
  rom.rs                The ESP32-C3 mask-ROM HLE stub table (including
                        the software_reset/software_reset_cpu stubs that
                        request a reset, and the reset reason
                        rtc_get_reset_reason reports), the guest-executed
                        ROM code blobs (qsort, strdup, the CPU-frequency
                        pair), the ROM data tables and the boot-time RAM
                        initializers.
  boot.rs               "Shortcut boot": loads firmware.bin straight from its
                        header, skipping ROM and bootloader emulation.
                        step_with_interrupts runs one step, with WFI
                        fast-forward.
  runtime.rs            FirmwareRuntime: the whole emulator as one Clone-able
                        object. Buttons by raw slot; serial input; the
                        acceleration; run_until and emulated_time_us. The
                        software reset: after a reset stub's step it
                        re-boots the image, keeping the flash, RTC_CNTL's
                        STORE registers, RTC memory, the panel, the console
                        text, held input and host time (soft_reset_count).
                        reset() stays a cold power-on. attend(), the cold
                        path behind the attention flag, takes resets and
                        hook hits. The radio host API: set_factory_mac,
                        radio_advertisement(_bytes),
                        deliver_advertisement(_bytes).
emulator-wasm/          The logic-free wasm-bindgen shim.
```

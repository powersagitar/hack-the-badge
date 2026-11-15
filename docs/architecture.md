# Architecture

How the emulator and the page fit together, as of the end of Milestone
14. This is the overview; each section names where the full account is.
`CLAUDE.md` has the file-by-file map, and `firmware-emulator-notes.md`
is the long-form technical record.

## The idea

The badge's built-in apps are native RISC-V code linked into one
ESP-IDF image, so they cannot be lifted out (notes, "Why real firmware
is emulated instead of extracted"). The project emulates the chip
instead: an ESP32-C3 written from scratch in Rust, booting the official
firmware `v0.1.2-392-gd3089c4` unmodified. Everything on the screen is
drawn by the firmware. The page only supplies input (buttons, motion,
serial text, a peer's radio packets) and shows output (the panel, the
LEDs, the console).

```
 browser main thread                        Web Worker
 ┌──────────────────────────────────┐       ┌───────────────────────────────┐
 │ React page (ui/)                 │ msgs  │ firmware-runtime.ts           │
 │  Badge: photo, hit buttons,      │──────▶│  wall-clock pacing, tap queue │
 │   screen canvas, LED glows       │ press │  provisioner, setup skipper   │
 │  Stage: matter.js lanyard ──mg──▶│ accel │  reboot pause, power cycle    │
 │  ToolsPanel, ConsolePanel        │ power │        │ wasm-bindgen          │
 │ badge-controller (store)         │ serial│        ▼                       │
 │ firmware-worker-client ◀─────────│◀──────│ emulator-wasm (passthroughs)  │
 │  paints frames on rAF            │ frame │        │                       │
 │ radio hub ◀──▶ WebRTC peers      │ leds  │ emulator-core::FirmwareRuntime│
 │   (advIn/advOut, bumps)          │consol │  CPU, bus, peripherals, ROM   │
 └──────────────────────────────────┘ advOut│  stubs, fake BLE controller   │
          │ /api/signal (offer/answer)      └───────────────────────────────┘
          ▼
   Vercel function + Upstash Redis (dev: in-memory)
```

## The crates

- **`emulator-core`**: pure Rust, no wasm-bindgen. Everything that
  executes or models hardware.
- **`emulator-wasm`**: a logic-free shim of one-line passthroughs.
  `framebuffer()` returns an owned `Vec`, so a view never outlives a
  WASM heap growth.
- The root `Cargo.toml` builds `emulator-core` at `opt-level = 3` even
  in the dev profile, with debug assertions on, so debug test runs keep
  their per-step cross-checks at usable speed.

## Booting: the shortcut boot

The real chip runs its mask ROM, then the 2nd-stage bootloader, which
reads the partition table, sets up the flash MMU and jumps into the app.
The emulator skips both. `boot.rs` parses the app image's header and
segments, loads DRAM, IRAM and RTC memory, seeds the flash MMU as the
bootloader would, seeds the clock registers the bootloader leaves
(PLL, 80 MHz), sets SP and PC, and starts at `entry_addr`. What an app
expects at its entry point is documented in ESP-IDF's `startup.c`; the
bootloader's internals are not (notes, "Boot strategy").

The flash is synthetic: 4 MiB, `firmware.bin` at the `factory` offset,
the partition table the badge has, and blank (0xFF) elsewhere. The
firmware formats littlefs on `storage` itself on first boot (notes,
"Emulated flash chip").

## The CPU

`cpu/` is a generic RV32IMC core with no chip constants: decode,
execute, CSRs, traps, WFI, and the cycle counter (`mpccr`, one cycle per
working step). Every instruction costs one cycle (CPI 1).
`decode_cache.rs` is a direct-mapped decoded-instruction cache that
checks the raw bits on every fetch, so it never needs invalidating.

Two rules are load-bearing:

- An **unmapped data access never panics**: it reads 0 and drops writes,
  and is logged. Real firmware touches registers nobody modeled.
- An **unmapped instruction fetch always traps**. Otherwise the CPU could
  run away through empty memory in a way that looks like progress.

## The mask ROM: high-level stubs

The firmware calls fixed-address mask-ROM functions from its first
instructions, and the ROM's bytes are not in flash, so nobody has them.
`cpu/rom_stubs.rs` intercepts a fetch at a known ROM address and applies
an effect instead (`RomStubEffect`): return a value, a real
`memcpy`/`memset`/`strlen`/`itoa`/`printf` through the bus, a libgcc
64-bit integer helper, a register write, a reset request, and so on.
`rom.rs` holds the ESP32-C3's table, each address from ESP-IDF's linker
scripts. A few ROM routines must call back into firmware code (`qsort`
calls `compar`), so they are real RV32 machine code assembled at
compile time (`cpu/encode.rs`) and placed in the emulated ROM space.
ROM data tables and boot-time RAM initializers live in `rom.rs` too
(notes, "The mask-ROM problem").

## The bus and the peripherals

`mem/bus.rs`'s `FirmwareBus` is the ESP32-C3 memory map as ordered,
concretely typed fields (no `dyn`): XIP through the flash MMU, the RAM
segments, ROM code and data, each peripheral, and a logging catch-all. A
word fast path tries XIP within one MMU page, then a RAM span map.

Each peripheral in `peripherals/` models only what the firmware uses,
from ESP-IDF v5.5.3's register headers:

| Peripheral | What it does for the badge |
| ---------- | -------------------------- |
| SYSTIMER | FreeRTOS's tick and alarms; the clock everything reads |
| Interrupt matrix (`intc.rs`) | 64 sources onto 32 CPU lines, priority ≥ threshold (not a PLIC) |
| SYSTEM | CPU clock selection (40/80 MHz switching), software interrupts |
| APB_CTRL | The RNG, with a fixed seed (deterministic dice and food) |
| GPIO + 74HC165 | The buttons, through the shift register |
| SPI2 + ST7789 | The display; SPI reads GPIO's D/C pin directly and rebuilds the framebuffer |
| GDMA | SPI's DMA |
| TIMG | RTC calibration; the watchdogs are inert |
| RTC_CNTL | The RTC timer (counts through a reboot), STORE0..7 |
| USB-Serial-JTAG + `console.rs` | The console both ways; a 256 KiB ring with an absolute byte count |
| I2C0 + SC7A20H | The accelerometer |
| RMT + `ws2812.rs` | The LED chain, decoded passively into six colours |
| eFuse | Only the factory MAC, opt-in, set at power-on |
| Flash MMU, SPI1 (`flash.rs`, `mmu.rs`) | XIP mapping, flash reads and writes (littlefs, NVS) |

## Time

One notion of emulated time (notes, "Timing model"; Milestone 6). The
chip-agnostic `clock.rs` turns executed steps into 16 MHz SYSTIMER
ticks at the CPU frequency the firmware selects, without drift; the
firmware switches between 40 and 80 MHz around every idle. When the core
is in WFI with nothing pending, it fast-forwards to the next event
(a SYSTIMER alarm or a USB packet) in one step. Snake's CPU is idle
about 96% of the time, so most emulated time costs almost nothing.
`FirmwareRuntime::run_until(target_us, max_steps)` is the host's entry
point and ends idle runs exactly on target.

## Resets and power

- **Software reset** (Milestones 9 and 11). v392 enters Scanner, Sync,
  Share, Sponsors and Workshop Quest by rebooting. The ROM's
  `software_reset_cpu` stub asks the runtime to reset; it re-boots the
  image, adds the badge's measured 466 ms (`REBOOT_US`) to host time and
  the RTC timer, and keeps what a CPU reset keeps on the chip: the flash,
  RTC memory, the STORE registers, the panel, and (for a CPU reset) the
  peripherals the TRM says survive. The table, row by row with
  citations, is in `milestone-11-decisions.md`, "What a CPU reset keeps".
- **`reset()`** is a cold power-on at 0 µs with blank flash.
- **`power_cycle()`** (Milestone 14) is the page's power switch: a cold
  power-on that keeps the flash, so a provisioned badge boots back to
  its registered screen.

## The radio

The real BLE controller lives in mask ROM. `radio/` replaces it at the
seam where ESP-IDF's `bt.c` talks to it (VHCI): app-image hooks
(`cpu/app_hooks.rs`) at v392's addresses (`radio/v392.rs`, each cited
from the disassembly) catch controller init and enable and the three
`esp_vhci_host_*` functions. `radio/hci.rs` is a pure LE-only HCI model
in H4 framing: extended advertising and scanning, event masks, a
duplicate filter and a bounded event queue. Replies and peers'
advertising reports reach NimBLE through a guest trampoline in free ROM
space. The model is connectionless, which is enough: Connect's pairing,
Sync's station request and Share's transfer are all advertisement-only
in v392.

Between browsers, the Worker posts the badge's advertisement (at most
10 Hz) to the main thread's radio hub (`radio/hub.ts`), which sends it
over every WebRTC data channel and validates what peers send (valibot).
Links use `iceServers: []` (same network only) and are set up through
`/api/signal`, an offer/answer mailbox with a 300 s TTL and rate limits
(Upstash Redis on Vercel, in memory under `bun run dev`). A peer's
advertisement enters the core at −40 dBm. A bump is an impact of the
local motion source; it is felt locally and sent to the peers, whose
badges feel it through their own motion source.

## The browser runtime

- **The Worker** (`firmware-worker.ts`) owns `firmware-runtime.ts` and
  runs it in slices of at most 400,000 steps, each targeting the
  previous emulated time plus the wall-clock delta (clamped to 50 ms; a
  capped slice drops its debt, so the badge runs in slow motion rather
  than freezing). It posts one frame in flight, a throttled `status`,
  `leds` on change, byte-exact `console` chunks, and `advOut`.
- **Taps** go through a per-button queue: 100 ms of emulated hold and a
  40 ms release gap, so a quick double tap is never lost (Milestone 8).
- **A reboot's pause**: the wall clock pays the 466 ms jump, so the
  screen holds as the badge's does.
- **The provisioner** types `put /littlefs/identity.json`, the JSON in
  64-byte chunks, `prov apply`, then `badge_token set` with the fake
  token, into the firmware's own console. **Skip setup** types
  `config system onboard_build 2147483647` and `reboot`. Both match
  prompts by absolute console byte offsets, because v392's console is
  not ASCII and the ring wraps.
- **The client** (`firmware-worker-client.ts`) paints frames on
  `requestAnimationFrame`, sends at most one acceleration per frame, and
  pauses everything while the page is hidden.
- **One badge per browser**: a Web Lock (`htn-badge`) and a per-browser
  locally administered MAC in `localStorage`.

## Motion

Exactly one source feeds the SC7A20H (`motion/source.ts`):

- **The lanyard** (`motion/lanyard.ts`): a headless matter.js rope of 10
  segments holding the badge body, with walls and a ceiling, stepped at
  1/120 s. `motion/sensor.ts` turns the board's pose into what the chip
  would read: a true 1 g through the tilt plus the chip spot's
  acceleration times 0.2 mg per px/s², clamped to ±2000 mg. A hanging
  badge reads (0, +1000, 0) mg. A wall impact over 840 px/s is a bump.
- **The phone** (`motion/device-motion.ts`): DeviceMotion's
  acceleration including gravity, sign-corrected on iOS and iPadOS
  (which invert every axis, R21). The drawn badge follows the phone's
  tilt physically. A jolt over 1500 mg is a bump.

`motion/axes.ts`'s `BOARD_AXES` (right +x, up +y, out +z) is an
assumption, not verified on a physical badge.

## The page

React with shadcn/ui on Tailwind v4, dark only (`ui/theme.css`). The
board is a photo cut from the organizers' public orthographic scan
(`frontend/scripts/badge-photo.py`), with its geometry measured in
percent (`ui/badge-geometry.ts`): screen, buttons, LEDs, holes and
per-corner radii. `Badge.tsx` lays the screen canvas, the LED glows (a
gamma-2.2 boost with a 0.35 floor, for the eye only), invisible hit
buttons and the power switch over it. `Stage.tsx` runs one frame loop
for the physics, the pose and the glows. `ToolsPanel.tsx` is FIRMWARE
TOOLS; `ConsolePanel.tsx` is CONSOLE.LOG (2,000 lines, a reboot marker,
`[page]` lines merged in). `badge-controller.ts` is the store tying
them to the Worker and the hub.

## Deployment

Vercel builds `frontend/` (its project root) from
`frontend/vercel.json`: `frontend/scripts/vercel-build.sh`
installs a minimal Rust toolchain, a prebuilt wasm-pack 0.15.0 and Bun
if missing, runs `bun run build:wasm` and `bun run build`, and serves
`frontend/dist/`. `frontend/api/signal.ts` is the only server code: the
signaling handler over `@upstash/redis` and `@upstash/ratelimit`
(`KV_REST_API_URL` and `KV_REST_API_TOKEN`; a missing one is a bare
500). Vite's dev middleware serves the same handler over an in-memory
store.

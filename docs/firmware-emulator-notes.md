# Real-firmware emulator: forensic findings & known limitations

This records findings from reverse-engineering the physical badge's dumped
flash that fed the design of `emulator-core`/`emulator-wasm` (Milestone 2),
plus the emulator's current known gaps. Most of this isn't derivable by
reading the code alone — it's either forensic work against the real device
or research against ESP-IDF sources that happened outside this repo. Kept
here (not just in commit messages) so it survives independently of any one
commit.

## Why real firmware is emulated instead of extracted

The original plan assumed the badge's built-in apps (Snake, Dice, etc.)
could be extracted as `main.lua`/`manifest.cfg` pairs, like user-sideloaded
apps. A read-only `esptool` dump of a physical badge's full 4MB SPI flash
and forensic analysis disproved this:

- **Partition table** (confirmed by manual parse): `nvs` @0x9000/0x4000,
  `phy_init` @0xd000/0x1000, `factory` (app) @0x10000/0x2a0000, `storage`
  (LittleFS, only `/config/*.cfg` + `/identity.json` on the dumped device —
  no user apps installed at dump time) @0x2b0000/0x140000.
- **`factory` partition** is a single ESP-IDF app image for ESP32-C3
  (single-core RV32IMC, no FPU): `esp_image_header_t` magic `0xE9` at
  0x10000, `esp_app_desc_t` magic `0xABCD5432` at 0x10020, ESP-IDF v5.5.3,
  project version `v0.1.2-335-gbf0bee6`.
- **No Lua bytecode anywhere in `factory`** — the only `\x1bLua`-adjacent
  hits are the Lua loader's own `lundump.c` error-string literals, not a
  real chunk. All 42 `"littlefs"` string hits are the vendored
  `esp_littlefs` VFS driver's own log/path strings — no second embedded
  filesystem exists.
- **Built-in apps are native RISC-V machine code**, statically linked into
  one monolithic image: Snake/Dice/Pong/Sokoban/Invaders/Flappy/Hangman/
  Mines9/Museum/Scanner/Share/Slots and dozens more, each a
  `DisplayName\0ShortCode\0slug\0` triplet followed by that app's own UI
  strings, packed contiguously in rodata — the classic footprint of
  statically-linked translation units, not separable files.
- The firmware genuinely does contain a working embedded Lua VM, but it's
  used only for user-sideloaded apps received via Bluetooth "bump" sharing
  — exactly what Milestone 1's Lua sandbox already replicates. That code
  path needed no rework.

Given this, Milestone 2 built a real ESP32-C3 processor emulator (RV32IMC
core + a minimal peripheral set) that boots `frontend/public/firmware/factory.bin`
unmodified, rather than trying to hand-extract or recreate any built-in app.

### `factory.bin` segment table

File offsets are relative to 0x10000 (the partition's flash offset).

| # | kind | load addr | file offset | size |
|---|------|-----------|--------------|------|
| 0 | DROM (XIP rodata) | 0x3c130020 | 0x10020–0x147ea8 | 1,277,576 B |
| 1 | DRAM | 0x3fc99c00 | 0x147eb0–0x150018 | 33,128 B |
| 2 | IROM (XIP code) | 0x42000020 | 0x150020–0x279b6c | 1,219,404 B |
| 3 | DRAM | 0x3fca1d68 | 0x279b74–0x2805f0 | 27,260 B |
| 4 | IRAM | 0x40380000 | 0x2805f8–0x29a164 | 105,324 B |
| 5 | RTC/LP | 0x50000000 | 0x29a16c–0x29a18c | 32 B |

Entry point is `0x403803fc` (bytes 4-7 of the image header, confirmed by
directly reading `factory.bin`'s raw bytes). `emulator-core/src/mem/image.rs`
parses this format from scratch (shape only, not a copy of ESP-IDF source);
`emulator-core/src/mem/soc.rs` categorizes each segment as XIP vs. RAM-copied
by address range, matching how the real 2nd-stage bootloader does it.

### Boot strategy: "shortcut boot," not literal bootloader execution

Real hardware boot is mask-ROM bootloader → 2nd-stage bootloader (reads the
partition table, decides which app to run) → jumps into the app's
`entry_addr` with flash cache/MMU already configured and a valid stack set
up. The emulator does not execute the mask ROM or 2nd-stage bootloader at
all — literally emulating them would require modeling an even
less-documented surface (efuse reads, the ROM SPI-flash driver, early
UART/console, RNG) without removing any guesswork, just relocating it.

Instead (`emulator-core/src/boot.rs`), the emulator parses the app image
itself, populates DRAM/IRAM/RTC at their load addresses, registers
DROM/IROM as flash-backed XIP regions, sets SP/PC per the header, and jumps
straight to `entry_addr` — starting emulated execution at exactly the point
real hardware would be at when the 2nd-stage bootloader hands off. What the
app expects true at its own entry point is public and documented (ESP-IDF's
`components/esp_system/startup.c`), unlike the mask ROM/bootloader's own
internals.

**Milestone 3 Task D5**: those DROM/IROM XIP regions are page-granular, not
just `[load_addr, load_addr+len)` (`emulator-core/src/mem/bus.rs`'s
`FirmwareBus::from_segments`/`xip_page_window`) — matching what the real
2nd-stage bootloader's `set_cache_and_start_app()` +
`mmu_hal_map_region()` actually expose (see the "Known limitations" entry
below for the full citation chain and why it matters: `cpu_start`'s
app-image-header check reads bytes that live in exactly this leading
per-segment page gap).

**Not yet done:** cross-checking computed segment/entry addresses against a
real-hardware serial boot log transcript (the real bootloader's own boot log
prints over USB-Serial-JTAG before the app's console takes over — passively
observable, no firmware modification needed). Judged lower priority since
the header-byte ground truth above is stronger evidence for "does the parser
read the header correctly" than a serial log would add on top — but it
remains a cheap live-hardware check if boot ever behaves unexpectedly in a
way only that comparison would explain.

## The mask-ROM problem, and how it's solved

Real ESP-IDF firmware calls directly into fixed-address, silicon-resident
mask-ROM functions within its first few instructions (e.g.
`rtc_get_reset_reason` at `0x40000018`) — normal, common ESP-IDF behavior,
not unique to the bootloader stage. The emulator has the badge's *flash*
contents but never has and never will have the mask ROM's bytes (it isn't
in flash at all; flash-dumping cannot recover it).

Without any accommodation, the first such call runs `pc` into unmapped
space. `emulator-core` deliberately makes that trap loudly
(`INSTRUCTION_ACCESS_FAULT`) rather than silently decode a zero word as a
no-op — unmapped **instruction fetches** always trap even though unmapped
**data** MMIO reads/writes never panic (return 0 / drop silently instead).
This asymmetry is load-bearing: without it, the CPU could silently run away
through unmapped memory in a way that looks like progress.

The fix is high-level emulation (HLE): intercept a fetch to a known ROM
address and simulate just enough of that function's observable effect
instead of trapping. The *default* effect is simple ("set `a0`, jump to
`ra`"), but a growing minority of stubs need a real effect the caller's
next instruction actually depends on, because a fabricated return value
would silently corrupt the caller rather than unblock it. Every effect a
stub can have is one of `RomStubEffect`'s variants
(`emulator-core/src/cpu/rom_stubs.rs`):

- `Return(value)` — the default: write a fixed value into `a0`, jump to
  `ra`.
- `Void` — jump to `ra` without touching `a0` (for a real ROM function
  whose C signature is `void`, so a stray value the caller kept alive in
  `a0` must survive the call).
- `Memset`/`Memcpy` — a real byte-for-byte fill/copy through the bus (ROM
  libc's `memset`/`memcpy`; their whole point is the bytes they write, so a
  generic status return would corrupt the caller rather than unblock it).
- `Int64` — one of libgcc's 64-bit integer helpers (`__udivdi3` and
  siblings), computed for real in registers, no bus access.
- `BusRegisterWrite` — a runtime-computed peripheral-register write through
  the bus (added in Milestone 3's Task D3 fix round, for the five
  interrupt-matrix/interrupt-controller ROM calls below): the target
  address is a fixed or argument-register-indexed base, and the write
  itself is one of three chip-agnostic shapes (`BusRegisterOp`) — overwrite
  the whole word (`Store`), OR-in/AND-out a mask (`UpdateMask`), or set/clear
  one bit chosen by another argument register (`SetOrClearBit`).
- `Itoa`/`Strcat` (added in Milestone 3's Task D4) — real byte-for-byte
  reimplementations of ROM libc's `itoa`/`strcat`, mirroring newlib's own
  `itoa.c`/`utoa.c`/`strcat.c` source, through the bus — same "the caller
  uses the output, so a status stub would corrupt it" reasoning as
  `Memset`/`Memcpy`.
- `Printf { sink_addr }` (added in Milestone 3's Task 7) — a real C-printf
  formatter for ROM's `ets_printf` (`%d %i %u %x %X %c %s %p %%`, the `l`/
  `ll` length modifiers, `-`/`0` flags, field width, `%s` precision — see
  `emulator-core/src/cpu/rom_stubs.rs`'s `compute_printf` for the exact
  RV32 ILP32 vararg-slot convention cited from the RISC-V calling-
  convention spec). Formats `a0`'s C string against `a1..a7`/the stack,
  then writes each output byte through `bus.write8(sink_addr, ..)` — for
  the ESP32-C3, `rom.rs` supplies `sink_addr =
  USB_SERIAL_JTAG_RANGE.start`, the same MMIO byte real FIFO TX output
  writes through, so console output, its cap, and the WASM passthrough all
  apply unchanged. Output and format-string-scan length are both capped
  (1 KiB) so a garbage format-string pointer can't hang the emulator.

- `LoadStoreWords` (added in Milestone 3's Task D9) — for a `void` ROM
  function whose whole effect is copying the words its pointer arguments
  point at to fixed ROM-internal addresses (`esp_rom_newlib_init_common_mutexes`),
  done through the bus so later ROM code sees the state.
- `Strlen`/`Memcmp`/`Strncmp`/`DivT` (Task D9) — real ROM libc reads and
  computations mirroring the ROM ELF's own code (`DivT` returns `div_t` in
  `a0`/`a1`).

**Guest-executed ROM routines (Milestone 3 Task D6).** A stub's effect runs
atomically inside one CPU step, so it cannot run guest code. ROM libc
`qsort` has to call its `compar` argument, which is firmware code, over and
over in the middle of the sort, so it can't be a stub. Computing the
comparison in Rust would mean PC-intercepting an ESP-IDF function, which
this emulator never does. So `qsort` is real RV32 machine code in the
emulated ROM address space, and the CPU runs it like any other code:

- `qsort`'s ROM address (`0x4000_0434`) is a 4-byte jump-table slot. It
  holds one `jal x0, <body>`, so execution can't run on into `rand_r`'s slot
  at `0x4000_0438`.
- The body is a byte-swapping insertion sort at `0x4003_f000`
  (`emulator-core/src/rom.rs`'s `QSORT_BODY`). It is assembled at compile
  time from `emulator-core/src/cpu/encode.rs`'s `const fn` encoders, and a
  test decodes every word through the crate's own decoder. It saves `ra`
  and `s0..s5` in a 16-byte-aligned frame, per the RISC-V psABI.
- `0x4003_f000..0x4004_0000` is free by the linker scripts. Across all 21
  `components/esp_rom/esp32c3/ld/esp32c3.rom*.ld` scripts (v5.5.3), every
  IROM-mask symbol is at or below `0x4000_4680`
  (`r_lld_res_list_rem`, `esp32c3.rom.eco7_bt_funcs.ld`). The only other
  ROM-aperture symbols are DROM-mask data at `0x3ff1_ee3c..=0x3ff1_fffc`.
  The range also ends below `0x4004_0000`, so it stays clear of the top
  128 KiB of the IROM mask either way.
- Mechanism and data are split the same way as for stubs.
  `emulator-core/src/mem/bus.rs`'s `RomCodeBlob`/`FirmwareBus::map_rom_code`
  is a generic read-only, executable region in the bus's ordered region
  list. `rom.rs`'s `ESP32C3_ROM_CODE` names the addresses and words.
  `boot_from_factory_image_with_rom_stubs` installs both. Every other ROM
  address still traps on fetch, exactly as before.
- Equal elements may end up in a different order than on real silicon
  (`qsort` isn't stable either way). The one observed caller has no equal
  elements, so this doesn't affect it.

**ROM data (Milestone 3 Task D7).** Some of what firmware takes from the
mask ROM is *data*, not code: tables and pointers at fixed DROM-mask
addresses (`SOC_DROM_MASK_LOW..SOC_DROM_MASK_HIGH` = `0x3ff0_0000..
0x3ff2_0000`, `components/soc/esp32c3/include/soc/soc.h`). These are
backed the same split way:

- `emulator-core/src/mem/bus.rs`'s `RomDataBlob`/`FirmwareBus::map_rom_data`
  is a generic read-only region, checked right after ROM code. Unlike ROM
  code it is **never fetchable**: a jump into ROM data traps like unmapped
  space. Only each blob's own bytes are mapped; the rest of the DROM mask
  still reads `0` and is logged. ROM code and data blobs can't overlap.
- `rom.rs`'s `ESP32C3_ROM_DATA` holds the ESP32-C3 values:
  `ets_rom_layout_p` (`0x3ff1_fffc`, `esp32c3.rom.ld`) holds `0x3ff1_be3c`,
  and the 40-word `ets_rom_layout_t` there
  (`components/esp_rom/esp32c3/include/esp32c3/rom/rom_layout.h`) has
  `dram0_rtos_reserved_start = 0x3fcd_f060`.
- The **values** come from the mask ROM, so IDF source can't supply them.
  They were read from Espressif's published ROM ELF: `esp32c3_rev3_rom.elf`
  in `espressif/esp-rom-elfs` release `20241011` (the release ESP-IDF
  v5.5.3's `tools/tools.json` pins; tarball SHA-256 `921f0001…e9a9`, ELF
  SHA-256 `19ac22e0…fc3c`, full hashes in `rom.rs`'s entry 15). Rev3 matches
  the badge's v0.4 (ECO3) chip; the rev0 ELF has different addresses.
  The ELF is not committed.
- Only fields the firmware actually reads get real values. Disassembly of
  `factory.bin` shows exactly one reader of `ets_rom_layout_p`, and it
  reads only `dram0_rtos_reserved_start` (offset 4). The other 39 fields
  are `0`. A later consumer of another field must back it from the same
  ELF, not rely on the `0`.

This is split across two files on purpose:

- `emulator-core/src/cpu/rom_stubs.rs` — the **generic mechanism** (a stub
  table keyed by address, checked before each fetch, plus the
  `RomStubEffect` vocabulary above), deliberately chip-agnostic, consistent
  with `cpu/`'s rule that it knows nothing about ESP32-C3 specifics. An
  empty/unpopulated stub table leaves every other test byte-for-byte
  unaffected — verified by re-running the full pre-HLE test suite as part
  of adding this mechanism.
- `emulator-core/src/rom.rs` — the **chip-specific data**: which of the
  ~1200 ROM addresses ESP-IDF's own `components/esp_rom/esp32c3/ld/*.ld`
  linker scripts (fetched at tag `v5.5.3`) name, and which `RomStubEffect`
  each one gets (plus, since Task D6, which ROM addresses hold
  guest-executed code blobs: currently just `qsort`, see above). 80 stub
  addresses total: 17 individually named/justified —
  `rtc_get_reset_reason`, `ets_printf`, `ets_delay_us`, ROM libc
  `memset`/`memcpy`/`itoa`/`strcat`, the CPU-frequency getter/setter pair,
  the two eFuse queries, the UART-flush call, and the five interrupt-matrix/
  interrupt-controller calls below (each researched against a specific
  ESP-IDF header/linker-script citation) — plus the
  `Cache_*`/`rom_i2c_*`/libgcc-64-bit-helper families (63 more) added by
  symbol range as a generic default (`Return(0)` for `Cache_*`/read-side
  `rom_i2c_*`, `Void` for write-side `rom_i2c_*`, real `Int64` arithmetic
  for the libgcc family), not each individually verified necessary.

Five of the seventeen individually-named stubs are the ESP32-C3 interrupt
matrix's own bring-up calls, and are worth calling out on their own because
they're the one case where "the wrong stub type would corrupt boot" bit
concretely: `intr_matrix_set`, `esprv_intc_int_disable`,
`esprv_intc_int_enable`, `esprv_intc_int_set_type`, and
`esprv_intc_int_set_priority` (`0x4000_05e0`–`0x4000_05f4`,
`esp32c3.rom.ld`/`components/riscv/include/esp_private/interrupt_deprecated.h`)
all use `BusRegisterWrite` to perform a real read/write of
`emulator-core/src/peripherals/intc.rs`'s `InterruptController` registers
(the MAP region, `CPU_INT_ENABLE_REG`, `CPU_INT_TYPE_REG`,
`CPU_INT_PRI_<n>_REG`) — an earlier revision of this table left four of
these five as `void` no-ops (safe only because the one boot run observed
happened to write values those registers already held) and left the fifth
unstubbed entirely; see "Known limitations" item 1 below for the fix and
citations.

Result: real-firmware boot went from faulting ~20 instructions in to
running past the mask-ROM wall entirely and into the app image's own
runtime/logging code — see "Known limitations" below for exactly where it
stalls today.

## Physical pin map (buttons, display) — third-party sourced

The 8 physical buttons are **not** individually wired to 8 GPIO pins. Per a
community member's own working custom ESP-IDF firmware for this exact badge
(`github.com/abigail-liang/hack-the-north-2026-badge`, `firmware/main.c` —
third-party, reverse-engineered against real hardware by its author, not
official Hack the North documentation, but high-confidence since it's a real
working firmware): 7 of 8 buttons go through a **74HC165
parallel-in/serial-out shift register**, bit-banged over 3 GPIO pins; only
`START` is a direct GPIO pin (GPIO9, which doubles as the chip's BOOT
strapping pin).

| Signal | GPIO | Role |
|---|---|---|
| `LCD_DC` | 0 | ST7789 D/C (command vs. data) |
| `LCD_CLK` | 1 | ST7789 SPI SCLK |
| `LCD_CS` | 2 | ST7789 SPI CS (not modeled — see Known Limitations) |
| `LED_DIN` | 3 | WS2812 chain data (RMT) — unmodeled in v1 |
| `LCD_RST` | 4 | ST7789 reset |
| `I2C_SDA` | 5 | accelerometer — unmodeled in v1 |
| `I2C_SCL` | 6 | accelerometer — unmodeled in v1 |
| `HC165_DATA` | 7 | shift register serial data out → CPU (GPIO input) |
| `PIN_START` | 9 | START button, direct GPIO, also BOOT strap |
| `LCD_MOSI` | 10 | ST7789 SPI MOSI |
| `HC165_LOAD` | 20 | shift register parallel-load/latch (GPIO output) |
| `HC165_CLK` | 21 | shift register shift clock (GPIO output) |

Human button name → raw shift-register/GPIO slot (composed from
`emulator-core/src/runtime.rs`'s slot numbering and
`emulator-core/src/peripherals/gpio.rs`'s `Hc165` slot→button table, both
sourced from the same third-party firmware; see
`frontend/src/runtime/firmware-runtime.ts`'s `RAW_SLOT_BY_BUTTON` for the composed
table used by the TS side):

| Button | Raw slot | Source |
|---|---|---|
| START | 0 | direct GPIO9 |
| A | 1 | shift-register bit 0 |
| B | 2 | shift-register bit 1 |
| HOME | 3 | shift-register bit 2 |
| DOWN | 4 | shift-register bit 3 |
| LEFT | 5 | shift-register bit 4 |
| RIGHT | 6 | shift-register bit 5 |
| UP | 7 | shift-register bit 6 |
| AUX1 | 8 | shift-register bit 7 |

**Not yet visually confirmed against the real physical badge** — sourced
from the third-party firmware and independently cross-checked against this
repo's own `gpio.rs`/`runtime.rs` code twice, but never against actual
button presses on real hardware. Do this live, with the badge in hand, not
via a subagent — it's exactly the kind of check that needs a human looking
at the actual device.

## Known limitations / where boot currently stalls

As of the Milestone 2 merge, real-firmware boot runs 2,000,000+ instructions
with zero traps, then stalls inside `rtc_clk_cal()`, polling an unmodeled
`TIMG_RTCCALICFG_REG` (TIMERGROUP0, the `RTC_CALI_RDY` bit) — before
reaching C-runtime init, `app_main`, the app launcher, or any built-in app
including Snake. This is the plan's own accepted "best-effort boot
progress" bar (Tier 3), not a bug — Tier 4 (fully playable Snake,
physical-hardware visual cross-check) was always out of scope for Milestone
2.

**Milestone 3 candidate work**, in the order a whole-branch code review
predicted these blockers would surface once TIMG unblocks further boot:

1. ~~**Model `TIMG_RTCCALICFG_REG`**~~ — **Resolved in Milestone 3, Task 3**
   (`emulator-core/src/peripherals/timg.rs`). `RTCCALICFG_REG`/
   `RTCCALICFG1_REG`/`RTCCALICFG2_REG` on both TIMG0 and TIMG1 are now
   modeled per `components/soc/esp32c3/register/soc/timer_group_reg.h` and
   `components/esp_hw_support/port/esp32c3/rtc_time.c`'s
   `rtc_clk_cal_internal()`: a write setting `RTC_CALI_START` completes the
   calibration instantly (`RDY := 1`, `VALUE := MAX * 40_000_000 /
   slow_hz(CLK_SEL)`); the six `WDTCONFIG*`/`WDTFEED`/`WDTWPROTECT`
   watchdog registers are plain inert storage (the watchdog never fires).
   This unblocked the exact pre-Task-3 stall: hot PCs are no longer
   confined to `0x4038c80c`-`0x4038c822` (confirmed by
   `emulator-core/tests/boot_progress.rs`'s
   `timg_calibration_escapes_the_pre_fix_rtc_clk_cal_spin_loop`, which
   asserts no single PC in that range is hit 1,000+ times in a
   3,000,000-step trace — pre-fix it was ~22,222 per address; post-fix each
   address in that range is hit exactly once, i.e. the same poll loop still
   legitimately executes once per boot, it just no longer spins).
   **Where boot now stalls** (measured via `cargo run -p emulator-core
   --release --example boot-probe -- --steps 20000000`): still **zero**
   console output after 20,000,000 steps (`rom_stub_calls: 181005`, `traps:
   0` — still executing real, non-faulting code, not stuck on a fault).
   Hot PCs (last 200,000-step window) are now a *different*, wider pair of
   loops at `0x4038bbbe`-`0x4038bbe4` and `0x4038f8f6`-`0x4038f8fe`
   (~1,810/~3,620 hits respectively — smaller counts than the old
   fully-blocking spin, consistent with *some* forward progress happening
   between visits, not a hard stall — but still not advancing past this
   pair over the full 20M-step run). The dominant unmapped accesses are now
   `0x6000_800c`/`0x6000_8010`/`0x6000_8014` (16 reads/writes each in the
   trailing-window tail) — all three fall inside `RTCCNTL`
   (`DR_REG_RTCCNTL_BASE == 0x6000_8000`, confirmed via
   `components/soc/esp32c3/register/soc/reg_base.h`) at offsets `0x0c`
   (`RTC_CNTL_TIME_UPDATE_REG`, W — triggers an RTC-timer snapshot),
   `0x10`/`0x14` (`RTC_CNTL_TIME_LOW0_REG`/`RTC_CNTL_TIME_HIGH0_REG`, R —
   the latched snapshot), per
   `components/soc/esp32c3/register/soc/rtc_cntl_reg.h` — a trigger/poll/
   read-latch pattern structurally identical to SYSTIMER's `OP_REG`/
   `UNIT0_VALUE_HI/LO_REG` this codebase already knows how to model.
   **Resolved in Milestone 3, Task D1** (`emulator-core/src/peripherals/rtc_cntl.rs`).
   `RTC_CNTL_TIME_UPDATE_REG`/`TIME_LOW0_REG`/`TIME_HIGH0_REG` are now
   modeled per `components/hal/esp32c3/include/hal/rtc_cntl_ll.h`'s
   `rtc_cntl_ll_get_rtc_time()` (which this task confirmed has **no**
   ready/valid bit on the ESP32-C3, unlike TIMG's calibration — the trigger
   write latches synchronously): a write setting `TIME_UPDATE` (bit 31)
   latches a 48-bit snapshot derived from the emulator's own elapsed step
   count (SYSTIMER's live `unit0_counter`, the same "notion of elapsed
   time" SYSTIMER itself uses), scaled by `RC_SLOW_HZ / XTAL_HZ` (reused
   from `crate::peripherals::timg`, not duplicated). This unblocked the
   pre-Task-D1 stall (confirmed via a throwaway experiment: the specific
   scaling ratio doesn't matter — even a 1:1 ratio reaches the exact same
   next fault, just sooner — so this is a deterministic *next* boundary in
   the firmware, not an artifact of the tick-rate placeholder).
   **Where boot now stalls**: no longer a spin at all — boot runs past this
   point and, for the first time, prints real console output: a full
   ESP-IDF "Guru Meditation Error" panic dump (generic text, confirmed via
   `emulator-core/tests/boot_progress.rs`'s
   `first_console_output_is_the_firmware_s_own_panic_report`). The panic was
   a genuine `INSTRUCTION_ACCESS_FAULT` at a fixed call target,
   `0x4000_0358` (step 401,761 from a cold boot), which did **not**
   correspond to any named symbol in ESP-IDF v5.5.3's esp32c3 ROM linker
   scripts — flagged rather than guessed at, per Task D1's brief.
   **Resolved in Milestone 3, Task D2**
   (`emulator-core/src/cpu/rom_stubs.rs`'s `RomStubEffect::Memcpy`,
   `emulator-core/src/rom.rs`'s `MEMCPY` entry). The orchestrator identified
   `0x4000_0358` as ROM libc `memcpy`, defined not in the "obvious"
   `esp32c3.rom.libc.ld` but in the sibling
   `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld` (the variant ESP-IDF's
   `esp_rom` `CMakeLists.txt` links whenever
   `CONFIG_ESP_ROM_HAS_SUBOPTIMAL_NEWLIB_ON_MISALIGNED_MEMORY` is set and
   `CONFIG_LIBC_OPTIMIZED_MISALIGNED_ACCESS` is not — the ESP32-C3's default),
   which is why Task D1's exhaustive linker-script search missed it. Like
   `memset`, `memcpy` is stubbed with a real byte-for-byte implementation,
   not a generic status return, since a fabricated return value would
   silently corrupt the copy's destination rather than unblock the caller.
   **Where boot now stalls**: still not a spin — the same "Guru Meditation
   Error" panic text is still the first console output (no new line to
   ratchet on), but the fault has moved forward, from the old `memcpy`
   call to a **new** `INSTRUCTION_ACCESS_FAULT` at `0x4000_071c` (step
   402,113), which this time **does** resolve to a named symbol:
   `ets_efuse_get_spiconfig`, in the main `esp32c3.rom.ld` table (not a
   libc/string function, so out of Task D2's scope per its brief's explicit
   ruling). As before, the firmware's own panic handler catches it, prints
   the dump, and tries to reboot via the same unstubbed `software_reset_cpu`
   (`0x4000_0094`) as before, which also faults and loops. Pinned down
   exactly in `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_stalls_on_the_unstubbed_ets_efuse_get_spiconfig_rom_call`.
   **Resolved in Milestone 3, Task D3** (`emulator-core/src/rom.rs`'s module
   doc, entry 10). `ets_efuse_get_spiconfig` returns `0` ("default SPI
   pins" — the documented sentinel from `esp32c3/rom/efuse.h`, and correct
   for the badge's flash, which sits on the default SPI pads; the eFuse
   block itself remains unmodeled). That unblocked a chain of six more
   observed ROM calls in turn, each stubbed per the same task: eFuse's WP
   pad accessor (`ets_efuse_get_wp_pad`, returns the header's documented
   `0x3f` "invalid" sentinel), a UART TX-flush busy-wait
   (`uart_tx_wait_idle`, `void` no-op, same reasoning as `ets_delay_us`),
   and four ESP-IDF interrupt-controller bring-up calls
   (`intr_matrix_set`, `esprv_intc_int_disable`, `esprv_intc_int_set_type`,
   `esprv_intc_int_set_priority`, originally landed as `void` no-ops — a
   throwaway boot-probe confirmed each one's real target register was still
   at its power-on value for every observed call, so a real write and a
   no-op were byte-identical for what boot actually asked of them *at the
   time*). **Boot/panic ordering, corrected**: all of this — the eFuse
   queries, the UART flush, the whole interrupt-controller chain — is
   *normal pre-panic boot code*; it runs with **zero** traps of any kind
   (confirmed: 0 traps through step 402,113, exactly 1 trap by step
   500,000). An earlier revision of this doc and of `rom.rs`'s module doc
   described this chain as running "straight through" an already-printed
   panic dump; that had the order backwards — no panic has happened yet at
   this point in boot. The panic dump described below is the *direct
   result* of the very next fault, not something the chain ran through
   after the fact.
   **Where boot then stalled**: still not a spin, still the same "Guru
   Meditation Error" panic text as the first console output (no new line to
   ratchet on), but the fault had moved forward again, into the middle of
   ESP-IDF's interrupt-controller bring-up: a **new**
   `INSTRUCTION_ACCESS_FAULT` at `0x4000_05e8` (step 405,806),
   `esprv_intc_int_enable`. Unlike its four `esprv_intc_int_*`-*and*-`intr_matrix_set`
   siblings above (`intr_matrix_set` itself is from `esp32c3/rom/ets_sys.h`,
   not the `esprv_intc_int_*` family, despite this task grouping all four
   together), this one *sets* a bit in `CPU_INT_ENABLE_REG` — a register
   `emulator-core/src/peripherals/intc.rs`'s `InterruptController::poll`
   genuinely consults to decide whether a pending, routed interrupt source
   reaches the CPU — so treating it as an inert no-op wasn't provably safe
   the way its siblings were, and a real fix meant reaching into that
   already-modeled peripheral's register from a ROM stub, new
   stub-mechanism plumbing Task D3's brief scoped out. Pinned down exactly
   in `emulator-core/tests/rom_stub_boot.rs` (superseded below).
   **Resolved in Milestone 3, Task D3 Fix round 1**
   (`emulator-core/src/rom.rs`'s module doc, entry 11;
   `emulator-core/src/cpu/rom_stubs.rs`'s new
   `RomStubEffect::BusRegisterWrite`). A review of Task D3 raised two
   findings: (1) the four `void` no-ops above were justified only by the
   *one* call each observed in that boot run — ESP-IDF's real
   `esp_intr_alloc` path calls the same `intr_matrix_set` with non-zero
   arguments once a hacker app (or a later milestone's built-in app)
   actually registers an interrupt, and a `void` no-op would silently drop
   that; and (2) `esprv_intc_int_enable` itself was still unstubbed — the
   stall directly above. The fix adds a chip-agnostic
   `RomStubEffect::BusRegisterWrite` (an address computed from a base plus
   an optional argument-register-indexed offset, then a whole-word store, a
   mask OR/AND-out, or a single set/clear bit — see `cpu/rom_stubs.rs`'s
   `BusRegisterOp`) and rewires all **five** interrupt-controller calls onto
   it, with addresses computed from `INTERRUPT_CORE0_RANGE`
   (`emulator-core/src/mem/soc.rs`) plus `crate::peripherals::intc`'s own
   register-offset constants (`CPU_INT_ENABLE_REG`, `CPU_INT_TYPE_REG`,
   `CPU_INT_PRI_BASE_REG`, all already cited there against
   `components/soc/esp32c3/register/soc/interrupt_core0_reg.h`):
   `intr_matrix_set(cpu_no, model_num, intr_num)` now really writes
   `intr_num` into the MAP register at offset `model_num * 4`;
   `esprv_intc_int_disable`/`esprv_intc_int_enable` really AND-out/OR-in
   their mask into `CPU_INT_ENABLE_REG`; `esprv_intc_int_set_type` really
   sets or clears `intr_num`'s bit in `CPU_INT_TYPE_REG` depending on
   whether `type` is `INTR_TYPE_EDGE` (`1`) or `INTR_TYPE_LEVEL` (`0`,
   `components/riscv/include/riscv/interrupt.h`); `esprv_intc_int_set_priority`
   really writes `priority` into the indexed `CPU_INT_PRI_<n>_REG`. All five
   remain `void` C functions (per
   `components/riscv/include/esp_private/interrupt_deprecated.h`), so `a0`
   is still never touched.
   **Where boot then stalled**: with `esprv_intc_int_enable` performing a
   real write, boot ran straight past the old `0x4000_05e8` fault and hit a
   **new, genuinely different** unstubbed ROM call: `itoa` (`0x4000_0448`,
   `esp32c3.rom.libc.ld`, step 407,471). Boot-probe evidence:
   `itoa(value = 0x4200_1011, buf = 0x3fcd_c694, base = 0x10)` — a real
   int-to-hex-string conversion whose *output* the caller uses, in the same
   "needs a real implementation, not a guessed status return" category as
   `memset`/`memcpy` (see `rom.rs`'s "What is NOT stubbed, on purpose"
   section).

   **Task D4** gave `itoa` a real HLE implementation
   (`emulator-core/src/cpu/rom_stubs.rs`'s `RomStubEffect::Itoa`, mirroring
   newlib's `itoa.c`/`utoa.c` byte-for-byte). With `itoa` unblocked, boot
   immediately hit the very next unstubbed ROM libc call, `strcat`
   (`0x4000_03d8`, also `esp32c3.rom.libc.ld`) — the same task's own
   iteration ruling authorized fixing it in the same task, so it also got a
   real HLE implementation (`RomStubEffect::Strcat`, mirroring newlib's
   `strcat.c`). Both stubs are correct and needed; this section's
   *narrative* about what they're called for was originally wrong and is
   corrected below.

   **Corrected in Task D4 fix round 1** (a review traced the actual call
   chain through `factory.bin`'s own bytes and found the original
   conclusion — "this is the normal, pre-panic boot path, formatting its
   first log line" — wrong): `itoa`/`strcat` are called from newlib's
   `abort()` (`components/newlib/abort.c`, ESP-IDF's override), which
   unconditionally formats the fixed message `"abort() was called at PC
   0x<addr> on core <n>"` (confirmed by reading the literal string bytes
   directly out of the image) before calling `esp_system_abort` →
   `panic_abort`. `abort()`'s own caller, at `0x42001004`–`0x42001010`
   (confirmed via `auipc`+`jalr` target computation against the same image
   bytes), is `ets_printf` called with the format string `"E (%lu) %s:
   Invalid app image header\n"` and the tag `"cpu_start"` (both read
   directly out of the image at their literal addresses). **So `cpu_start`
   (ESP-IDF's early startup) rejects this image's header and aborts** —
   and has been doing so since at least Task D3 fix round 1 (whichever fix
   first let execution reach this check); Task D4 did not move this wall,
   it only let this pre-existing `abort()` call finish formatting its
   message instead of faulting mid-format on an unstubbed `itoa`/`strcat`.
   The console showed nothing at the time of the original `itoa` call not
   because nothing had been logged, but because `ets_printf`'s stub is
   `Return(0)` and never reaches `Console` (`emulator-core/src/rom.rs`'s
   entry 7 caveat, added in this same fix round) — the `cpu_start` error
   line was logged and silently dropped, not skipped. **Console emptiness
   at any point during this boot run is therefore not evidence that
   nothing was logged** — implementing `ets_printf` for real is left to a
   later task (plan Task 7), not attempted here.

   **Where boot now stalls**: no longer on an unmapped ROM-address fetch at
   all. With `itoa`/`strcat` real, the pre-existing `abort()` call above
   runs to completion and reaches the trap it was always going to reach: a
   real `ILLEGAL_INSTRUCTION` exception (RISC-V cause 2, step 407,549) at
   `0x4038e4fa`, inside the app's own IRAM code — a compiler-emitted
   `c.unimp` (the RVC extension's all-zero 16-bit encoding, architecturally
   reserved to always trap), reached right after storing `g_panic_abort =
   true` and `g_panic_abort_details = <this abort's message>` to two fixed
   addresses (`components/esp_system/panic.c`) — a deliberate,
   hardware-standard trap, not a decode gap. Per Task D4's own stop
   condition ("stop as soon as the stall is anything other than an
   unstubbed ROM libc/string call"), that task correctly stopped here,
   since the actual blocker (`cpu_start`'s header check) isn't a ROM stub
   gap at all.

   What happens next is all real, hardware-faithful behavior, not an
   emulator gap: the trap delivers correctly to the firmware's own
   exception handler, which runs `esp_panic_handler` to completion for the
   first time (`itoa`/`strcat` actually built this abort's message this
   time, instead of faulting mid-format). Per `panic.c`, the abort path
   leaves `info->reason == NULL`, so the "Guru Meditation Error" header is
   *skipped* on this first pass; ESP-IDF's generic pre-restart text prints
   unconditionally — **panic output for `cpu_start`'s abort, not evidence
   of boot progress past it** — then `panic_restart()` calls into the
   still-unstubbed ROM `software_reset_cpu` (`0x4000_0094`) to actually
   reboot — which faults for real (step 645,410), re-entering the panic
   handler through its *exception* path (where `info->reason` is finally
   non-`NULL`, so "Guru Meditation Error" prints for the first time, by
   step 648,000), which again reaches the same unstubbed
   `software_reset_cpu` call and loops. `software_reset_cpu` remains
   unstubbed — it's only reached via the panic path, and stubbing it
   wouldn't address `cpu_start`'s header-check failure, the actual
   blocker. Pinned down exactly in `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_aborts_reaching_the_panic_handlers_reboot_message` —
   named and documented to make clear it pins today's *abort* path, and is
   expected to break, deliberately, once a later task fixes the header
   check. This is the natural next Milestone 3 candidate: find out *why*
   `cpu_start` considers this image's header invalid (a plausible,
   **unverified** hypothesis: `cpu_start` reads the header through
   `SOC_DROM_LOW`, `0x3c00_0000`, via the flash cache/`memcpy`, and
   `crate::boot`'s shortcut-boot mapping may not expose the header bytes
   there the way real flash-cache bring-up would — not confirmed, just a
   plausible next investigation), since that — not another ROM stub — is
   what stands between here and a built-in app / the first real pixels.

   **Milestone 3 Task 7** implemented `ets_printf` for real (see the
   `RomStubEffect` vocabulary's `Printf` bullet above), closing the
   "console emptiness is not evidence" caveat two paragraphs up: boot-probe
   confirmed the firmware calls `ets_printf` directly rather than through
   an IDF-side formatter that calls a ROM putc, so the plan's original
   "don't implement printf in the stub" clause didn't hold and was
   overridden by that task's orchestrator ruling. Every early-boot log
   line up to and including `cpu_start`'s own header-check error now
   reaches the console for real, e.g. (full text in that task's report,
   not reproduced here beyond this one line, which is generic ESP-IDF
   text, not identity data): `E (0) cpu_start: Invalid app image header`.
   The spec's original ladder rungs (`cpu_start: Pro cpu start user code`,
   `cpu_start: cpu freq:`) were checked for and do **not** appear —
   `cpu_start`'s header check runs and fails before either would print —
   so `emulator-core/tests/boot_progress.rs`'s new
   `boot_reaches_cpu_starts_own_header_check_error_line` rung asserts the
   line boot *does* reach instead, per that task's own contingency for
   this case. **This does not move the header-check wall** — it's the same
   blocker as before, just now diagnosable by reading the actual firmware
   log instead of only trap addresses and register dumps.

   **Resolved in Milestone 3, Task D5** (`emulator-core/src/mem/bus.rs`'s
   `FirmwareBus::from_segments`, `xip_page_window`;
   `emulator-core/src/mem/soc.rs`'s `MMU_PAGE_SIZE`). Task 7's own
   orchestrator hypothesis (quoted above: `cpu_start` reads the header
   through `SOC_DROM_LOW`, `0x3c00_0000`) turned out to be **refuted by the
   real ESP-IDF source**: `components/esp_system/port/cpu_start.c` reads
   the header via `hal_memcpy(&fhdr, (void*)fhdr_src_addr, sizeof(fhdr))`
   where `fhdr_src_addr = (uint32_t)&_rodata_reserved_start -
   sizeof(esp_image_header_t) - sizeof(esp_image_segment_header_t)` — a
   **linker symbol relative to the DROM segment's own `load_addr`**, not a
   fixed SoC-wide address at all. `_rodata_reserved_start` resolves to
   `factory.bin`'s DROM segment 0's `load_addr`, `0x3c13_0020` (see the
   segment table above), so `fhdr_src_addr = 0x3c13_0020 - 24 - 8 =
   0x3c13_0000` — 32 bytes *before* `load_addr`, and (not a coincidence)
   exactly that address's containing 64 KiB flash-cache MMU page start
   (`0x3c13_0020 & !0xFFFF`). Confirmed two ways, not just read off the
   source: (1) disassembling `factory.bin`'s own bytes around the check
   (`emulator-core`'s decoder, ad hoc) shows compiler-emitted `lui`
   instructions loading the literal constant `0x3c13_0000` as a base for
   nearby rodata string addresses; (2) instrumenting a real boot run
   confirmed **exactly 24 consecutive byte reads** (`sizeof(esp_image_header_t)`)
   at addresses `0x3c13_0000`..`0x3c13_0017`, immediately before the
   pre-fix `abort()` — a direct, empirical trace of `hal_memcpy`'s own read
   pattern, not an inference.

   Real hardware exposes those bytes because the 2nd-stage bootloader's
   `set_cache_and_start_app()` (`bootloader_support/src/bootloader_utility.c`)
   page-aligns each XIP segment's `load_addr` *down* and widens its mapped
   size by the leading gap before programming the flash-cache MMU, and
   `hal/mmu_hal.c`'s `mmu_hal_map_region()` rounds the total mapped length
   *up* to a whole number of 64 KiB pages (`MMU_PAGE_SIZE`, confirmed fixed
   at 64 KiB on ESP32-C3 by `hal/esp32c3/include/hal/mmu_ll.h`'s
   `mmu_ll_get_page_size()`) — so whatever else shares that physical flash
   page as a segment (before its `load_addr` or after `load_addr + len`)
   becomes readable at the matching virtual address too, not just the
   segment's own declared bytes. `crate::boot`'s shortcut boot previously
   mapped each XIP segment over only its own exact `[load_addr,
   load_addr+len)`, so `[0x3c13_0000, 0x3c13_0020)` fell through to the
   bus's never-panic catch-all (reads `0`), making the magic-byte check
   fail. `FirmwareBus::from_segments` now widens every XIP segment to its
   containing page(s) the same way, so this read now returns the image's
   real header bytes (magic `0xE9` at offset 0).

   Only the magic byte is actually validated by `cpu_start` itself (a
   second `abort()` in the same function is gated behind
   `CONFIG_SPI_FLASH_SIZE_OVERRIDE`, which this build doesn't define, and
   `chip_id`/`min_chip_rev`/`max_chip_rev_full` are parsed into `fhdr` but
   never compared against anything in this function) — so fixing the page
   mapping was sufficient on its own; no second header-field check needed
   fixing in the same task.

   **Where boot now stalls**: the header check passes for real —
   `cpu_start: Invalid app image header` no longer appears anywhere in the
   console — and boot runs much further, printing several lines this
   emulator had never reached before: `cpu_start: Pro cpu start user code`
   (step 407,528), `cpu_start: cpu freq: 160000000 Hz` (407,586), then a
   full `app_init`/`efuse_init` block (project name, app version, compile
   time, ELF SHA256, ESP-IDF version, min/max/actual chip revision — all
   generic ESP-IDF build metadata, not identity data), ending at
   `efuse_init: Chip rev: v0.0` (408,344). It then hits a **new, unrelated**
   stall: an unstubbed ROM `qsort` call (`0x4000_0434`, confirmed against
   `esp32c3.rom.libc.ld`'s `qsort = 0x40000434;`) — a plain missing-ROM-stub
   gap, not another header-field check, so per this task's own scope ruling
   it's left for the next task rather than fixed here. The register dump at
   the fault (`nmemb`/`size`-shaped args `A1=5, A2=8`; return address inside
   the app's own `esp_system` startup code) is consistent with ESP-IDF's
   `do_system_init_fn()` sorting its init-function array before running it,
   though that caller isn't independently confirmed. The resulting
   `INSTRUCTION_ACCESS_FAULT` (step 408,481) is a genuine hardware
   exception, not an `abort()`, so ESP-IDF's panic handler takes its
   *exception* path immediately (`info->reason` non-`NULL` from the first
   pass) and prints "Guru Meditation Error" right away (step 410,197),
   then `panic_restart()` calls into the still-unstubbed ROM
   `software_reset_cpu` (`0x4000_0094`) to actually reboot — which faults
   again (console shows "Rebooting..." at step 648,457), re-entering the
   panic handler and looping, the same downstream shape as the old
   cpu_start-abort scenario purely by coincidence (both are a first
   `INSTRUCTION_ACCESS_FAULT`-class fault immediately followed by the same
   unstubbed reboot-retry fault), not because the cause is related. Pinned
   down at the time in `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_faults_on_the_unstubbed_qsort_call_and_reaches_the_panic_handlers_reboot_message`
   (historical name: since Task D6 that test is
   `boot_currently_aborts_on_the_unbacked_rom_layout_reserved_region_overlap_and_reaches_the_panic_handlers_reboot_message`).
   This is the natural next Milestone 3 candidate: stub `qsort` (a
   self-contained ROM libc algorithm, same "needs a real implementation"
   category as `memcpy`/`memset`/`itoa`/`strcat` — a fabricated return
   would leave the array unsorted rather than unblock the caller) and
   re-probe.

   **Task D6 (`qsort`) and the current stall.** `qsort` is now real
   guest-executed code (see "Guest-executed ROM routines" above), so the
   step-408,481 fault is gone. Task D6's Step 1 captured the call:
   `a0 = 0x3fcdc4d0` (a stack array), `nmemb = 5`, `size = 8`,
   `compar = 0x420029bc`, `ra = 0x42002a18`. The comparator's bytes in
   `factory.bin` are `c.lw a0,0(a0); c.lw a5,0(a1); c.sub a0,a5; c.ret`.
   That is ESP-IDF v5.5.3's `s_prepare_reserved_regions()`
   (`components/heap/port/memory_layout_utils.c`) sorting its
   `soc_reserved_region_t {start, end}` array with
   `s_compare_reserved_regions`. **The `do_system_init_fn()` guess above
   was wrong.** The sort returns at step 408,906, correctly sorted.

   The firmware's own validity check on the sorted array then fails. Entry 0
   is `{ets_rom_layout_p->dram0_rtos_reserved_start, SOC_DIRAM_DRAM_HIGH}`
   (`ESP_ROM_HAS_LAYOUT_TABLE` is set for the ESP32-C3). `ets_rom_layout_p`
   is a ROM **data** pointer (`esp32c3.rom.ld`: `ets_rom_layout_p =
   0x3ff1fffc;`) that the emulator doesn't back. It reads `0` from the bus
   catch-all, and so does the field load at `NULL + 4`. The region becomes
   `0x00000000 - 0x3fce0000`, so the firmware logs `E (0) memory_layout:
   SOC_RESERVE_MEMORY_REGION region range 0x00000000 - 0x3fce0000 overlaps
   with 0x3fc80000 - 0x3fc99c00` (step 408,970) and calls `abort()`. That
   reaches `panic_abort()`'s `ILLEGAL_INSTRUCTION` at `0x4038e4fa` (step
   409,071). The panic handler prints `abort() was called at PC 0x42002acf
   on core 0` and "Rebooting...", faults on the unstubbed
   `software_reset_cpu` (step 647,238), prints "Guru Meditation Error" and
   loops. Pinned at the time in `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_aborts_on_the_unbacked_rom_layout_reserved_region_overlap_and_reaches_the_panic_handlers_reboot_message`
   (historical name: since Task D7 that test is
   `boot_currently_faults_on_the_unstubbed_clzsi2_call_and_reaches_the_panic_handlers_reboot_message`).

   **Task D7 (ROM layout data) and the current stall.** The ROM layout
   table is now backed (see "ROM data" above). The only reader,
   `s_prepare_reserved_regions()` at `0x4200_29d6`, loads
   `ets_rom_layout_p` and then the field at offset 4, now `0x3fcdf060`. So
   entry 0 is `0x3fcdf060 - 0x3fce0000`. It sorts last, nothing overlaps,
   and the `memory_layout` error and its `abort()` are gone. `qsort` now
   returns at step 409,036 (its input changed). Boot prints ESP-IDF's
   normal `I (0) heap_init: Initializing. RAM available for dynamic
   allocation:`.

   **The stall at the end of Task D7** (now fixed, see Task D8 below): on
   the 409,759th step, boot fetches from
   `0x4000_079c`, which is the unstubbed libgcc `__clzsi2`
   (`esp32c3.rom.libgcc.ld`). The caller (`0x4212_9480`) computes
   `32 - __clzsi2(size)` (`a0 = 0x2e6c`), a TLSF "find last set" while
   registering a heap region. The `INSTRUCTION_ACCESS_FAULT` is a hardware
   exception, so the panic handler prints "Guru Meditation Error" right
   away (step ~411,500), then "Rebooting..." (step ~649,700), faults on the
   unstubbed `software_reset_cpu` (650,332nd step) and loops. Pinned in
   `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_faults_on_the_unstubbed_newlib_init_common_mutexes_call_and_reaches_the_panic_handlers_reboot_message`
   (historical name: it pinned this `__clzsi2` fault until Task D8).

   **Task D8 (libgcc unary helpers) and the current stall.** New
   `RomStubEffect::Int32Unary(Int32UnaryOp)`: a real value in `a0`, a real
   value out in `a0`, `pc = ra`. Semantics come from the GCC internals
   manual ("Integer library routines"). `__clzsi2` (`0x4000_079c`) returns
   the number of leading 0-bits; the manual leaves `a == 0` undefined and
   the stub returns 32 (`u32::leading_zeros`), so TLSF's `32 - clz(x)` idiom
   gives 0 for zero. It was observed with `a0 = 0x2e6c` (result 18). The
   next call, `__ffssi2` (`0x4000_07d4`, `a0 = 0x200`, result 10, caller
   `0x4039_5c08`), returns one plus the index of the lowest set bit, or 0
   for 0 (defined). Both are in `esp32c3.rom.libgcc.ld`. The rest of the
   bit-count family (`__clzdi2`, `__ctzsi2`, `__popcountsi2`, …) is left to
   fault until observed. `heap_init` now prints all four `heap_init: At
   ...` lines (the last, `RTCRAM`, at step ~415,621).

   **The stall at the end of Task D8** (now fixed, see Task D9 below): on
   the 417,992nd step boot fetched from `0x4000_0350`, ROM
   `esp_rom_newlib_init_common_mutexes` (RA `0x4200_6a22`), which mutates
   ROM-owned data (two `_lock_t` words). Its pinned test was re-pointed in
   Task D9.

   **Task D9 (newlib-init hooks) and the current stall.** New
   `RomStubEffect::LoadStoreWords`: for each `(ptr_reg, dst)` pair, load the
   word `a[ptr_reg]` points at and store it to the fixed address `dst`,
   through the bus; `a0` is untouched (`void` functions).
   `esp_rom_newlib_init_common_mutexes` (`0x4000_0350`,
   `esp32c3.rom.libc.ld`) is called by `esp_newlib_locks_init()` (IDF
   v5.5.3 `components/newlib/src/locks.c`) with two copies of a magic
   `_LOCK_T`. The ROM ELF's body (`0x4005260e`) is `lw a4,0(a0); sw
   a4,0x660(0x3fcdf000); lw a4,0(a1); sw a4,0x65c(0x3fcdf000); ret`, so it
   stores `*a0` at `0x3fcd_f660` (`common_recursive_mutex`) and `*a1` at
   `0x3fcd_f65c` (`common_mutex`), the pointed-at words rather than the
   pointers. Boot then made four atomic ROM libc calls, each now a real
   stub: `strlen` (`0x4000_0374`), `memcmp` (`0x4000_0360`), `strncmp`
   (`0x4000_0370`) and `div` (`0x4000_0428`, returning `div_t` in
   `a0`/`a1`). All are in `esp32c3.rom.libc.ld`; the rest of the string
   family (`strcpy`, `strncpy`, `strcmp`, `strstr`, `bzero`, `memmove`,
   `ldiv`) stays unstubbed until observed. The stub count is 87.

   **The stall at the end of Task D9** (now fixed, see Task 8 below): boot
   ran fault-free to step 442,140, when ESP-IDF's flash-chip detection
   (`memspi_host_read_id_hs`, `components/spi_flash/memspi_host_driver.c`)
   read the JEDEC ID through the then-unmodeled SPI1 flash controller, got
   0, logged `E (0) memspi: no response` (step 441,439), failed an `assert`
   and `abort()`ed (the 442,141st step).

   **Task 8 (emulated flash + SPI1 flash controller) and the current
   stall.** `emulator-core/src/peripherals/flash.rs` adds `EmulatedFlash`
   (the 4 MiB chip; see "Emulated flash chip" below) and `Spimem1`, the
   SPI1 flash controller at `DR_REG_SPI1_BASE = 0x6000_2000`, wired into
   `FirmwareBus` as a named field. Step 1's trace showed the firmware
   issuing exactly one flash command, RDID (`0x9F`), three times: once from
   the app's own `bootloader_flash_update_id()`
   (`bootloader_flash_execute_command_common`) and twice from
   `spi_flash_hal_common_command`. So only user-command (`SPI_MEM_USR`)
   transactions with a command and MISO phase are modeled, and only RDID
   has an effect: it returns `0x46 0x40 0x16`. The trigger fires on the
   byte of `SPI_MEM_CMD_REG` holding `SPI_MEM_USR` and self-clears. Other
   commands complete without effect and are listed by `boot-probe`
   ("unmodeled commands"; none so far). Boot now prints `I (0) spi_flash:
   detected chip: generic` (step ~446,991), `flash io: dio`, a `W (0)
   spi_flash: Detected size(4096k) larger than the size in the binary image
   header(0k)` warning (not in the real badge's log; see limitation 8), and
   both `sleep_gpio:` lines. The unmapped `0x600c_4000` read D9 noted is
   `EXTMEM_ICACHE_CTRL_REG` (`soc/extmem_reg.h`), read by the `assert`
   message's `<cached disabled>` check on the panic path, not flash
   machinery; the other EXTMEM accesses on the way (`+0x04`, `+0x08`,
   `+0x40`, `+0x78..+0x88`, `+0xAC`) are cache control and cache-error
   interrupt enables, also not flash-MMU table writes. The flash MMU
   sub-unit is therefore not needed yet: Task D5's page-granular XIP
   mapping still serves every IROM/DROM read, and SPIMEM1's `flash_chip`
   is separate from XIP's buffer.

   Boot then faulted on two atomic ROM libc calls, both now real HLE stubs
   (`emulator-core/src/rom.rs` entry 18): `memchr` (`0x4000_03c8`,
   `esp32c3.rom.libc.ld`; step 490,128, caller `0x4211_8854`, `a1 = '\n'`,
   `a2 = 3`) and `memmove` (`0x4000_035c`,
   `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`; step 490,143).
   Their semantics follow the ROM ELF's newlib code (`memchr` compares
   `(unsigned char)c`; `memmove` copies backward when `src < dst < src +
   n`). The stub count is 89.

   **The current stall**: on the 493,861st step boot fetches from
   `0x4000_0060`, the unstubbed ROM `ets_apb_backup_init_lock_func`
   (`esp32c3.rom.ld`; caller RA `0x4200_155c`; the ROM ELF's body at
   `0x40045fe8` stores its two arguments into ROM statics). It is neither
   flash nor libc, so Task 8 stopped there. The fault prints "Guru
   Meditation Error" (step ~494,744), then "Rebooting..." (~733,938), then
   the unstubbed `software_reset_cpu` faults on step 734,553. Pinned in
   `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_faults_on_the_unstubbed_ets_apb_backup_init_lock_func_call_and_reaches_the_panic_handlers_reboot_message`.
2. **SYSTIMER doesn't match real ESP-IDF v5.5.3 driver behavior.**
   `emulator-core/src/peripherals/systimer.rs` only models unit 0/target 0
   with real behavior, but ESP-IDF's `vSystimerSetup`
   (`components/freertos/port_systick.c`) actually connects the FreeRTOS
   tick alarm to **counter 1**, not counter 0. The model's `COMP0_LOAD_REG`
   sequencing assumption is also contradicted by the real HAL's call order
   (`systimer_hal_set_alarm_period` pulses the load while still in oneshot
   mode, switching to period mode afterward). Also: SYSTIMER/INTERRUPT_CORE0
   registers with no named field silently no-op instead of logging through
   the shared `unmapped_log` ring buffer other regions use — worth fixing
   first, since the next stalls will otherwise be invisible to the same
   trap-count-based diagnosis method that found the TIMG stall.
3. **SPI2 can't be driven by ESP-IDF's `spi_master` driver as-is.** Real
   `spi_ll_apply_config` sets `SPI_CMD_REG`'s `UPDATE` bit and spins on it
   clearing — `emulator-core/src/peripherals/spi.rs` currently stores it as
   plain read/write storage with no self-clear, so the first real SPI
   configuration would spin forever. Real completion detection also uses a
   different flag (`dma_int_raw.trans_done`) than what's modeled, and pixel
   transfers over 64 bytes use GDMA, which is entirely unmodeled.
4. **WFI decodes as `Illegal`** rather than as a real wait-for-interrupt —
   the FreeRTOS idle task's `wfi` would trap once boot reaches it.
5. Two flagged guesses worth re-examining once boot progresses further:
   `rom_i2c_readReg*` stubbed to return 0 (currently steers boot down an
   "assume 40MHz" crystal-frequency fallback that happens to match the
   badge's real crystal — correct today, but a guess, not a verified
   value); `Cache_Get_*` accessor family stubbed to 0 (never actually called
   during the observed boot path — unverified whether that generalizes to a
   boot path that gets further).
6. **`TICKS_PER_STEP = 1`** (the emulator's steps-to-real-cycles ratio) is
   roughly 10× the real SYSTIMER/CPU clock ratio — harmless while boot never
   reaches timing-sensitive code, but will need recalibrating once it does.
7. **The real bootloader's extra "boot partition lookup" DROM page isn't
   modeled** (Task D5 fix round 1, M4). Beyond widening each XIP segment to
   its own containing page (item 1 above), `set_cache_and_start_app()`
   (`bootloader_support/src/bootloader_utility.c:1084-1086`, v5.5.3) also
   maps one *extra*, unrelated MMU entry: `MMU_DROM_END_ENTRY_VADDR`
   (`hal/esp32c3/include/hal/mmu_ll.h`: `SOC_DRAM_FLASH_ADDRESS_HIGH -
   0x10000`, i.e. the very last page of the whole DROM flash-cache
   aperture) mapped to the *same physical page* as the DROM segment's own
   aligned start (`drom_addr_aligned`) — the source comment says this is
   "for app to find the boot partition." `crate::mem::bus::FirmwareBus`
   doesn't add this extra mapping at all today; nothing in the observed
   boot trace through Task D7's stall (item 1)
   has touched that fixed high address, so it's not yet a confirmed
   blocker — but it's a plausible **candidate cause of a later
   partition-table/`esp_partition_find`-style stall**, worth checking first
   if boot gets past heap init and stalls again on an unmapped read inside
   the DROM aperture near its top end.

8. **The ROM's SPI-flash legacy data is not initialized** (found in Task
   8). ESP-IDF reads the flash chip's size and ID from `g_rom_flashchip`,
   i.e. `rom_spiflash_legacy_data->chip` (`rom_spiflash_legacy_data` is the
   ROM data word at `0x3fcd_fff0`, `esp32c3.rom.ld`). On real hardware the
   mask ROM's startup sets that pointer to its own
   `rom_default_spiflash_legacy_data` (`0x3fcd_f5c0` per the ROM ELF) and
   the 2nd-stage bootloader writes the chip size from the image header. The
   shortcut boot does neither, so the pointer reads 0: `esp_flash` sees a
   0-byte chip (the `Detected size(4096k) larger than ... (0k)` warning)
   and the ID/size accesses land at addresses `0x0`/`0x4`. Nothing has
   failed on it yet, but `esp_flash_default_chip->size` is 0.

None of the above are correctness bugs *today* — they're dormant because
boot doesn't reach the code paths that would exercise them. They're
recorded here so Milestone 3 starts from a known list instead of
rediscovering each one by stepping through a debugger again.

## Emulated flash chip: what it contains

Milestone 3 Task 8 gives the emulator a model of the badge's whole 4 MiB
flash chip (`emulator-core/src/peripherals/flash.rs`, `EmulatedFlash`), for
the firmware's own flash driver to read through the SPI1 flash controller.
It is **synthetic**, never a copy of the physical chip:

- Everything is blank (`0xFF`, NOR flash's erased state) except:
- a partition table at `0x8000`, synthesized from committed constants: the
  badge's four entries (`nvs` 0x9000/0x4000, `phy_init` 0xd000/0x1000,
  `factory` 0x10000/0x2a0000, `storage` 0x2b0000/0x140000) followed by the
  `0xEBEB` MD5 entry (`CONFIG_PARTITION_TABLE_MD5`); and
- `factory.bin` at `0x10000`.

So `nvs`, `phy_init` and `storage` start blank, as on a freshly erased
chip, and no personal data from the physical badge is involved. The
partition layout itself is not personal data. Erase and program work in
memory only (program ANDs, so bits only go 1 -> 0); nothing is written back
to any file. `emulator-core/tests/flash_partition_table.rs` checks the
synthesized table byte-for-byte against the real chip's, but only when
`BADGE_FULL_DUMP` points at the local dump; without it the test skips.

## Data-handling note: what NOT to re-add

An earlier commit on the Milestone 2 branch briefly included
`frontend/public/firmware/full_flash_dump.bin` (the complete 4MB flash dump, as
opposed to `factory.bin`, just the app partition). That file was found to
contain the dumping device owner's personal identity data, a likely
credential, and other people's contact information (received via badge
"bump" exchanges) living in the `nvs`/`storage` partitions — none of which
organizer approval for firmware publication actually covered. It was
removed from the branch's git history entirely before merge (verified never
pushed to any remote). **Do not commit a full flash dump to this
repository** — only `factory.bin` (the app partition alone, confirmed clean
of personal data) belongs here. If a full dump is ever genuinely needed for
future bootloader/OTA work, it must have the `nvs` (0x9000–0xD000) and
`storage` (0x2b0000–0x3f0000) partition ranges zeroed/redacted first.

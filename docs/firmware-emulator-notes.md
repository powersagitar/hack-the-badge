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
(typically: set `a0`, jump to `ra`) instead of trapping. This is split
across two files on purpose:

- `emulator-core/src/cpu/rom_stubs.rs` — the **generic mechanism** (a stub
  table keyed by address, checked before each fetch), deliberately
  chip-agnostic, consistent with `cpu/`'s rule that it knows nothing about
  ESP32-C3 specifics. An empty/unpopulated stub table leaves every other
  test byte-for-byte unaffected — verified by re-running the full pre-HLE
  test suite as part of adding this mechanism.
- `emulator-core/src/rom.rs` — the **chip-specific data**: which of the
  ~1200 ROM addresses ESP-IDF's own `components/esp_rom/esp32c3/ld/*.ld`
  linker scripts (fetched at tag `v5.5.3`) name, and what each stub pretends
  to have done. 69 addresses total (6 individually named/justified —
  `rtc_get_reset_reason`, the `ets_printf` family, etc. — plus the
  `Cache_*`/`rom_i2c_*`/libgcc-helper families added by symbol range as a
  generic no-op default, not each individually verified necessary). Default
  behavior for any stubbed address with no specific semantics: return 0 in
  `a0`, jump to `ra`.

Result: real-firmware boot went from faulting ~20 instructions in to running
2,000,000+ clean instructions with zero traps.

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
   block itself remains unmodeled). That unblocked a chain of five more
   observed ROM calls in turn, each stubbed per the same task: eFuse's WP
   pad accessor (`ets_efuse_get_wp_pad`, returns the header's documented
   `0x3f` "invalid" sentinel), a UART TX-flush busy-wait
   (`uart_tx_wait_idle`, `void` no-op, same reasoning as `ets_delay_us`),
   and four ESP-IDF interrupt-controller bring-up calls
   (`intr_matrix_set`, `esprv_intc_int_disable`, `esprv_intc_int_set_type`,
   `esprv_intc_int_set_priority`, all `void` no-ops — a throwaway boot-probe
   confirmed each one's real target register was still at its power-on
   value for every observed call, so a real write and a no-op are
   byte-identical for what boot actually asks of them).
   **Where boot now stalls**: still not a spin, still the same "Guru
   Meditation Error" panic text as the first console output (no new line to
   ratchet on), but the fault has moved forward again, into the middle of
   ESP-IDF's interrupt-controller bring-up: a **new**
   `INSTRUCTION_ACCESS_FAULT` at `0x4000_05e8` (step 405,806),
   `esprv_intc_int_enable`. Unlike its four `esprv_intc_int_*` siblings
   above, this one *sets* a bit in `CPU_INT_ENABLE_REG` — a register
   `emulator-core/src/peripherals/intc.rs`'s `InterruptController::poll`
   genuinely consults to decide whether a pending, routed interrupt source
   reaches the CPU — so treating it as an inert no-op isn't provably safe
   the way its siblings were, and a real fix means reaching into that
   already-modeled peripheral's register from a ROM stub, new
   stub-mechanism plumbing Task D3's brief scoped out. Pinned down exactly
   in `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call`.
   This is the natural next Milestone 3 candidate: decide whether
   `esprv_intc_int_enable` needs a real register-write HLE (and, if so, what
   `RomStubEffect` shape lets a stub reach a specific peripheral's register
   without hard-coding SoC addresses into the chip-agnostic
   `cpu::rom_stubs` mechanism), or whether it's still safe to no-op given
   how little of the interrupt-delivery path is otherwise wired up yet.
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

None of the above are correctness bugs *today* — they're dormant because
boot doesn't reach the code paths that would exercise them. They're
recorded here so Milestone 3 starts from a known list instead of
rediscovering each one by stepping through a debugger again.

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

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

**Milestone 3 Task D5, superseded by the flash MMU (Milestone 4 Task 2)**:
the bus now models the flash MMU (`emulator-core/src/peripherals/mmu.rs`) and
`FirmwareBus::from_segments` seeds it like the bootloader, so XIP is
page-granular by construction (see `docs/milestone-4-decisions.md`); the
D5 text that follows describes the replaced mechanism. D5 made those
DROM/IROM XIP regions page-granular, not just `[load_addr, load_addr+len)`
(`xip_page_window`, now deleted) — matching what the real
2nd-stage bootloader's `set_cache_and_start_app()` +
`mmu_hal_map_region()` actually expose (see history item 1's "Resolved
in Milestone 3, Task D5" paragraph, under "History: the stall-by-stall
log" below, for the full citation chain and why it matters: `cpu_start`'s
app-image-header check reads bytes that live in exactly this leading
per-segment page gap).

A real-hardware serial boot log has since been captured (kept local-only,
never committed; see the "Data-handling note") and is the source of
"Ground truth from the physical badge" below. The header-byte evidence
above remains the primary check that the parser reads the header
correctly.

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

- `StoreWords` (added in Milestone 3's Task D9 as `LoadStoreWords`,
  generalized in Task D10) — for a `void` ROM function whose whole effect
  is storing words taken from its argument registers to fixed ROM-internal
  addresses, done through the bus so later ROM code sees the state. Each
  word is either the one a register points at (`WordSource::Pointee`:
  `esp_rom_newlib_init_common_mutexes` copies `*a0`/`*a1`) or the
  register's own value (`WordSource::Register`:
  `ets_apb_backup_init_lock_func` stores its two function pointers).
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
- Task D10 added a second consumer: the coexistence library's version
  string `"9387209"` at `0x3ff1_b74c` (ROM `.rodata`), which the stub for
  `esp_coex_rom_version_get` (`0x4000_18ac`) returns a pointer to, because
  the caller formats it (`rom.rs` entry 21).

**ROM writable `.data` (Milestone 3 Task D10).** Some mask-ROM state lives
in ordinary *writable* DRAM, in the ROM-reserved window `0x3fcd_f060..
0x3fce_0000`: the ROM's reset code copies its `.data` initializers there,
and the 2nd-stage bootloader updates some of it through ROM calls. The
shortcut boot skips both, so the app would read zeros. That is ordinary RAM,
so a read-only `RomDataBlob` doesn't fit. Instead:

- `emulator-core/src/boot.rs`'s `RamInitializer`/`apply_ram_initializers`
  is a generic list of `(address, bytes)` writes applied, in order, to the
  already-backed RAM before the first instruction.
- `rom.rs`'s `esp32c3_rom_ram_initializers(header)` holds the ESP32-C3
  data, which `boot_from_factory_image_with_rom_stubs` applies. Currently
  it holds only the SPI-flash legacy data, the one piece of ROM `.data` boot
  is observed to read: `rom_spiflash_legacy_data` (`0x3fcd_fff0`) pointing
  at `rom_default_spiflash_legacy_data` (`0x3fcd_f5c0`, 28 bytes, the ROM
  ELF's initializer), then the bootloader's
  `esp_rom_spiflash_config_param(device_id 0x464016, chip_size, 0x10000,
  0x1000, 0x100, 0xffff)` over its `chip` fields. `chip_size` is decoded
  from factory.bin's own header flash-size nibble (`0x2f` → 4 MB,
  `esp_app_format.h`), falling back to 2 MB as the bootloader's
  `update_flash_config()` does. `device_id` is the badge's JEDEC ID
  byte-swapped the way `bootloader_read_flash_id()` does it.
- No other ROM `.data` is seeded. A temporary read-before-write trace of the whole
  ROM-reserved window, run to the current stall, found no other read of
  ROM `.data` (the only other read was all-zero `.bss`, which zeroed RAM
  already matches). Every value is cited in `rom.rs` entry 20.
- **Task D12** added one entry that is a peripheral register, not RAM: the
  2nd-stage bootloader's XTAL-frequency store. `RTC_XTAL_FREQ_REG` is
  `RTC_CNTL_STORE4_REG` (`0x6000_80b8`; `esp_rom/esp32c3/include/esp32c3/
  rom/rtc.h`, `soc/rtc_cntl_reg.h`). The bootloader's
  `bootloader_clock_configure()` -> `rtc_clk_init()` ->
  `rtc_clk_xtal_freq_update()` -> `clk_ll_xtal_store_freq_mhz(40)`
  (`hal/esp32c3/include/hal/clk_tree_ll.h`) writes the MHz value into both
  16-bit halves: `0x0028_0028`. `clk_ll_xtal_load_freq_mhz()` rejects the
  reset value 0, so without the seed ESP-IDF logged an "invalid
  RTC_XTAL_FREQ_REG value, assume 40MHz" warning 17 times in early boot
  (`rtc_clk` tag) and once more from the SPI clock setup (`clk_hal` tag).
  The real badge's log has none of them. The write goes through the bus
  into the RTC_CNTL model's plain storage. Citations are in `rom.rs`
  entry 23.


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
unstubbed entirely; see history item 1 under "History: the stall-by-stall
log" below (the paragraph that rewires all five interrupt-controller calls
onto `BusRegisterWrite`) for the fix and citations.

Task D11 adds two more register-writing stubs, ROM `gpio_matrix_out` and
`gpio_matrix_in`, which program the GPIO matrix's
`GPIO_FUNCn_OUT_SEL_CFG_REG`/`GPIO_ENABLE_W1TS_REG` and
`GPIO_FUNCn_IN_SEL_CFG_REG` in `emulator-core/src/peripherals/gpio.rs`.
They use two new `BusRegisterOp` shapes, `StoreComposed` (a register plus
conditional flag bits) and `StoreBit` (`1 << reg`, for a write-1-to-set
register), and `gpio_matrix_out`'s two stores run as one
`RomStubEffect::BusRegisterWrites` sequence, which drops the whole call if
any index is out of range, as the ROM's own `gpio > 25` guard does.

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

Milestone 4 Task 7 confirmed the mapping from the firmware's side: in the
hardware self-test, each raw slot lights the on-screen label of the
button in this table (slot 8 is the self-test's "SideSwitch"). That is
the firmware agreeing with the table, still not a press on real hardware.

## Known limitations / where boot currently stalls

### Current state (end of Milestone 4)

Real-firmware boot runs from the shortcut boot of `factory.bin` on blank
synthetic flash through the whole of `app_main` to the first app's
screen, and that screen responds to buttons. From a cold boot:

- The boot splash is final from step 5,535,126 (`boots_to_first_real_frame`,
  hash `0x5599c270ab0429fa`, 2,340 distinct RGB565 values; unchanged by
  the flash MMU).
- `load_partitions()` accepts the synthesized partition table through the
  flash MMU, `esp_littlefs` formats and mounts the blank `storage`
  partition, the hardware abstraction layer starts (buttons, the absent
  accelerometer, the RMT LED driver, the sleep manager), and the app
  registry launches its first app, `app_reg: launched My Badge` (step
  ~12.29M).
- My Badge finds no identity (`hal_identity: no identity at
  /littlefs/identity.json — badge is unprovisioned`) and draws its
  first-run screen: "Not registered yet", "This badge hasn't been linked
  to an attendee. Bring it to a registration desk.", "Press START for
  hardware self-test". The framebuffer leaves the splash at ~13.36M,
  changes 9 more times as the screen draws in, and is stable from the
  15,500,000-step sample on (45 distinct values, hash
  `0x8c5027cec0f79490`; unchanged with no input to at least step
  46,500,000). `boots_to_first_run_screen` pins it (250,000-step samples,
  stable for 1,000,000 steps, found at step 16,500,000; cap 17,500,000).
- Pressing START (slot 0, held 100,000 steps) starts My Badge's hardware
  self-test. The firmware computes for about 6,000,000 steps without
  drawing (hot PCs `0x420b_ead4`, `0x420b_b6a4`; not traced), then shows
  the self-test's "Press every button" screen: stable from the 22,350,000-step
  sample, 48 distinct values, hash `0x7f325d838835baaa`.
  `first_run_screen_responds_to_start` pins it with an 8,000,000-step
  stability window (a 1,000,000-step window mistakes the busy phase for
  "START did nothing").
- The WASM twins in `frontend/test/cpu-wasm.test.ts` reach the same splash
  and first-run hashes through the wasm-bindgen build.
- No fault, no panic text and no unmodeled SPIMEM1 command on the way.
  The idle hot PCs are the FreeRTOS idle set (IRAM `0x4038_f930..`, the
  idle hooks, the 74HC165 scan): the firmware is waiting for input. The
  unmapped-access tail is ASSIST_DEBUG (`0x600c_e0xx`), SYSTEM
  `0x600c_0058`/`+0x08` and RTC_CNTL `0x6000_80bc`.
- **Human comparison, 2026-10-07:** the human partner compared the dumped
  first-run frame and the self-test frames with the physical badge after a
  factory reset (spec Part 3: the emulated flash holds no identity, so
  the reference is a badge without one) and confirmed they match, in content
  and in orientation. So the frames pinned are what blank-flash boot shows
  and what a factory-reset badge shows.
- To look at the frame:
  `cargo run -p emulator-core --release --example boot-probe -- --steps 16500000 --dump-frame first-run.png`
  writes a PNG under the gitignored `local/`.

**Why the app launcher is not reached.** It is firmware logic, not an
emulator gap. On a HOME press the app manager first asks the current app
whether it handles HOME itself; My Badge always says yes, and its button
handler checks the provisioned flag. Unprovisioned (as on blank flash), it
reacts only to START (the self-test); provisioned, HOME takes it to the
launcher. Every other button on the first-run screen does nothing, on the
emulator and on the badge. The input path was traced end to end on a HOME
hold (scan, debouncer, event queue, timer callback, app manager). So the
launcher needs a provisioned identity in emulated flash, which is
synthetic data and needs its own ruling (Milestone 5 backlog,
`milestone-4-decisions.md`). The self-test also confirms the button slot
mapping from the firmware's side: each raw slot lights the matching
on-screen label (see "Physical pin map").

**What the self-test shows that the emulator does not model:** the
accelerometer reads "0 mg" on every axis (no I2C device; the badge shows
live values), the lights test drives the LEDs over RMT but the emulator
does not show them, and the NFC test waits forever for a card (the summary
lists NFC as skipped). "Hold HOME" exits the self-test back to the
first-run screen, as A does after the summary.

**Resolved in Milestone 4** (details in the history below and in
[`milestone-4-decisions.md`](milestone-4-decisions.md)):

- The flash MMU (Tasks 1 and 2), replacing Milestone 3's D5 page windows:
  XIP reads `flash_chip` through the MMU table, so `load_partitions()`
  works (Task 3).
- Console output after the scheduler starts (Task 4: the USB-Serial-JTAG
  model reports an attached host).
- SPIMEM1's dedicated flash commands, `RDSR` and the DIO fast read (Task
  5), so littlefs formats and mounts `storage`.
- I2C0 with no device on the bus (Task D-M4-1) and RMT as a zero-latency
  TX engine (Task D-M4-2).
- ROM stubs: `strncpy`, `strcmp`, `strlcat`, `strspn`, `strcspn`, the
  libgcc soft-double helpers, `strchr`, `strcpy`, and guest-executed
  `strdup` (109 stubs in all).

**Resolved in Milestone 3** (details in the history below):

- TIMG0/TIMG1 RTC calibration (Task 3) and the RTC_CNTL RTC timer (Task D1).
- The ROM HLE stub table, grown task by task (Tasks D2 to D13 and 7): libc
  and libgcc helpers, `ets_printf` as a real formatter, the interrupt-matrix
  and GPIO-matrix ROM calls, ROM MD5, and ROM data/`.data` the shortcut
  boot seeds (layout table, SPI-flash legacy data, `RTC_XTAL_FREQ_REG`).
- Page-granular XIP mapping (Task D5), replaced by the real flash MMU
  model in Milestone 4 Task 2.
- The SPI1 flash controller and a synthetic 4 MiB flash chip (Task 8; see
  "Emulated flash chip" below).
- The interrupt matrix with priority/threshold and the SYSTEM FROM_CPU
  software interrupts (Task 4); SYSTIMER (Task 5, item 2); WFI with
  SYSTIMER fast-forward (Task 6, item 4).
- SPI2 `SPI_UPDATE`/`TRANS_DONE` (Task 9) and the GDMA TX out-link feeding
  SPI2 (Task 10, item 3), which is what puts pixels on screen.

The design decisions behind these fixes (and where they override the
plan text), plus the full Milestone 4 backlog, are in
[`docs/milestone-3-decisions.md`](milestone-3-decisions.md).

**Open limitations (Milestone 5 candidates)**, the first one being the
next blocker (the Milestone 5 backlog in `milestone-4-decisions.md` is the
complete list):

1. **No identity in emulated flash**, so My Badge stays on its first-run
   screen and the app launcher (and every built-in app behind it) is out
   of reach; see "Why the app launcher is not reached" above.
2. **ST7789 `MADCTL` is not modeled.** The firmware sends `MADCTL` `0x20`
   and then `0x60` (row/column exchange, then also column-address
   mirroring). `emulator-core/src/peripherals/spi.rs` ignores `MADCTL` and
   hardcodes the unrotated 320x240 order. The pinned hashes are of *this*
   framebuffer order. The splash, the first-run screen and the self-test
   frames were compared with the physical badge and match in orientation
   (the screen test's corner fiducials are all fully lit, so nothing is
   clipped), so the conditional Milestone 4 MADCTL task was skipped.
3. **No I2C devices, no LED output, no NFC.** The accelerometer probe is
   NACKed (the self-test reads 0 mg), RMT transmissions are consumed but
   not shown, and NFC is not modeled.
4. The dormant items from the history below: flagged ROM-stub guesses
   (item 5, and `CPU_FREQ_MHZ` = 160 against the badge's real 80 MHz; see
   "Ground truth from the physical badge"), `TICKS_PER_STEP = 1` (item 6)
   and simplified edge interrupts (item 9). Item 7 (the bootloader's
   extra DROM page) is resolved: the MMU seeds entry 127.
5. **The eFuse block is not modeled**, so the emulated boot logs
   `efuse_init: Chip rev: v0.0` where the real badge reports v0.4 (see
   "Ground truth from the physical badge"). Nothing has stalled on it.
6. **Every log timestamp up to `main_task: Calling app_main()` reads
   `I (0)`** (likewise `W (0)`/`E (0)`). Later lines carry non-zero
   timestamps (`I (120)`, `E (470)`, ...), which on ESP-IDF come from the
   FreeRTOS tick count once the scheduler runs. On real hardware the log
   timestamp is milliseconds since boot and advances line by line. The
   likely cause (not traced) is that the early timestamp comes from the
   CPU cycle counter, and the ESP32-C3's performance-counter CSR is not
   modeled. Cosmetic; no test depends on it.

### Ground truth from the physical badge

Read from the real badge's serial boot log and partition table (neither is
committed; see the "Data-handling note"). None of these facts is personal
data:

- ESP32-C3, chip revision v0.4; 40 MHz crystal; CPU at 80 MHz (the
  emulator's `CPU_FREQ_MHZ` stub returns 160, a flagged guess left alone
  because nothing has stalled on it).
- 4 MiB flash, JEDEC ID `0x46 0x40 0x16`.
- Console: USB-Serial-JTAG.
- Partition table: `nvs` 0x9000/0x4000, `phy_init` 0xd000/0x1000,
  `factory` 0x10000/0x2a0000, `storage` 0x2b0000/0x140000.

### History: the stall-by-stall log (Milestones 2 and 3)

As of the Milestone 2 merge, real-firmware boot runs 2,000,000+ instructions
with zero traps, then stalls inside `rtc_clk_cal()`, polling an unmodeled
`TIMG_RTCCALICFG_REG` (TIMERGROUP0, the `RTC_CALI_RDY` bit) — before
reaching C-runtime init, `app_main`, the app launcher, or any built-in app
including Snake. This is the plan's own accepted "best-effort boot
progress" bar (Tier 3), not a bug — Tier 4 (fully playable Snake,
physical-hardware visual cross-check) was always out of scope for Milestone
2.

**Milestone 3 candidate work**, in the order a whole-branch code review
predicted these blockers would surface once TIMG unblocks further boot
(item 1 grew into the step-by-step record of every Milestone 3 stall):

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
   time" SYSTIMER itself uses; *superseded in Task 5*: the RTC timer now
   reads SYSTIMER's monotonic `elapsed_ticks()`, which firmware cannot stop
   or reload, see item 2), scaled by `RC_SLOW_HZ / XTAL_HZ` (reused
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

   **Task D9 (newlib-init hooks) and the then-current stall.** New
   `RomStubEffect::LoadStoreWords` (since Task D10 `StoreWords` with
   `WordSource::Pointee`): for each `(ptr_reg, dst)` pair, load the
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

   **The stall at the end of Task 8** (now fixed, see Task D10 below): on
   the 493,861st step boot fetched from `0x4000_0060`, the unstubbed ROM
   `ets_apb_backup_init_lock_func` (`esp32c3.rom.ld`; caller RA
   `0x4200_155c`), printed "Guru Meditation Error" (step ~494,744) and
   "Rebooting..." (~733,938), and looped on the unstubbed
   `software_reset_cpu`.

   **Task D10 (APB lock hooks, ROM `.data`, scheduler start) and the
   current stall.** `ets_apb_backup_init_lock_func` is called by
   `esp_apb_backup_dma_lock_init()`
   (`components/esp_system/port/soc/esp32c3/apb_backup_dma.c`, via
   `init_apb_dma` in `startup_funcs.c`). Its ROM body stores the two
   function-pointer *values* into the ROM statics `0x3fcd_f654`/
   `0x3fcd_f658`, so D9's effect was generalized to `StoreWords` with a
   per-word `WordSource` (`Pointee` for D9's copy, `Register` here). The
   ROM's SPI-flash legacy data is now seeded at boot (limitation 8 below,
   resolved; "ROM writable `.data`" above), so the `(0k)` flash-size
   warning is gone and no access lands at address `0x0`..`0x7` any more.
   Two more atomic ROM calls follow, both now stubbed (`rom.rs` entry 21):
   `esp_coex_rom_version_get` (`0x4000_18ac`, step 498,924, from the
   coexistence library's `coex_pre_init()`; returns the ROM version string,
   backed as ROM data) and `esprv_intc_int_set_threshold` (`0x4000_05e4`,
   step 528,694, from FreeRTOS's `xPortStartScheduler()`; stores `a0` = 1
   to `CPU_INT_THRESH_REG`). The stub count is 92.

   **Task D10's stall (history) was a peripheral, with zero traps.**
   `xPortStartScheduler()` (`components/freertos/FreeRTOS-Kernel/portable/
   riscv/port.c`) enables interrupts and calls `vPortYield()`, which asks
   for the first context switch through the cross-core software interrupt:
   `esp_crosscore_int_send_yield()` writes `SYSTEM_CPU_INTR_FROM_CPU_0_REG`
   (`0x600c_0028`, `soc/system_reg.h`; `crosscore_int_ll_trigger_interrupt`
   in `hal/esp32c3/include/hal/crosscore_int_ll.h`) on the 528,777th step.
   The SYSTEM peripheral was unmodeled, so the write landed in the unmapped
   catch-all and no interrupt fired. The yield returned,
   `xPortStartScheduler()` returned, `vTaskStartScheduler()` returned to
   `esp_startup_start_app()` (`components/freertos/app_startup.c`, which
   had created the `main` task), and from step 528,805 the CPU spun on a
   `j .` at `0x4200_0cd2`. The interrupt controller also ignored
   `CPU_INT_THRESH_REG` (and every `CPU_INT_PRI_<n>_REG`), which D10's
   `esprv_intc_int_set_threshold` stub had just started writing.

   **Task 4 (interrupt matrix + SYSTEM software interrupts).** Both gaps
   are resolved:
   - `emulator-core/src/peripherals/system.rs` models the four
     `SYSTEM_CPU_INTR_FROM_CPU_<n>_REG` registers (`DR_REG_SYSTEM_BASE` +
     `0x028`/`0x02C`/`0x030`/`0x034`, `soc/system_reg.h`; bit 0, R/W) as
     level interrupt sources `ETS_FROM_CPU_INTR0..3_SOURCE` = 50..=53
     (`soc/interrupts.h`). The rest of the SYSTEM page stays in the logged
     catch-all.
   - `emulator-core/src/peripherals/intc.rs` is source-indexed: 64 uniform
     MAP registers (source `n` = MAP offset `n * 4`, e.g. SPI2 = 19,
     SYSTIMER target0 = 37, DMA ch0 = 44, FROM_CPU0 = 50; `emulator-core/src/mem/soc.rs`
     has the `SRC_*` constants). `poll()` takes a `u64` of asserted sources
     and returns a mask of lines that are routed (line 0 = unrouted),
     enabled, and have **priority `>=` threshold**. That is the legacy
     INTC rule: `components/riscv/include/esp_private/interrupt_intc.h`
     says "all interrupt priority levels strictly less than the threshold
     level are masked", `components/riscv/vectors.S` raises the threshold
     to `PRI[mcause] + 1` on ISR entry, and Espressif's QEMU model
     (`hw/riscv/esp32c3_intmatrix.c`) uses `irq_prio >= irq_thres`. (The
     plan's text said "strictly greater"; that would have masked every
     priority-1 line under FreeRTOS's threshold of 1, including this
     yield.)
   - Delivery is level-based. `crate::boot::step_with_interrupts` samples
     `FirmwareBus::asserted_lines()` into `Cpu::set_pending_interrupts`
     (which replaces the set, never ORs it) at the start of every step, so
     a source the ISR clears, or a raised threshold, stops the interrupt
     at once.
   - The indexed intc ROM stubs (`intr_matrix_set`,
     `esprv_intc_int_set_priority`) drop an out-of-range index (>= 64 /
     >= 32) instead of writing a neighbouring register
     (`BusRegisterWrite::index_limit`).

   The firmware routes FROM_CPU0 to CPU line 4 at priority 1 (threshold
   1), so the yield on step 528,777 is taken on step 528,778 (`mcause` =
   `0x8000_0004`; the first trap of the boot). The port switches to the
   first task, and the console prints `I (0) main_task: Started on CPU0`
   (step ~536,400) and `I (0) main_task: Calling app_main()` (step
   ~555,650), both also in the real badge's boot log.

   **Task 4's stall** (history) was a ROM call inside `app_main`: the
   unstubbed ROM `gpio_matrix_out` (`0x4000_05a4`, `esp32c3.rom.ld`), with
   `a0` = 10 and `a1` = `0x41`, faulted on the 571,713th step; the panic
   handler printed "Guru Meditation Error" and "Rebooting...", and its
   reboot faulted on the unstubbed ROM `software_reset_cpu`.

   **Task D11 (ROM GPIO-matrix routing).** The caller is ESP-IDF's
   `spicommon_bus_initialize_io()` (`components/esp_driver_spi/src/gpspi/
   spi_common.c:650-705`), routing SPI2 onto the display pads with
   `esp_rom_gpio_connect_out_signal()`/`esp_rom_gpio_connect_in_signal()`,
   aliased by `esp32c3.rom.api.ld` to ROM `gpio_matrix_out`/
   `gpio_matrix_in`. Both are now real stubs that do what their ROM
   bodies (disassembled from the rev3 ROM ELF) do, through the bus:
   `gpio_matrix_out` stores `signal | out_inv<<8 | oen_inv<<10` to
   `GPIO_FUNCn_OUT_SEL_CFG_REG` and sets the pad's `GPIO_ENABLE_W1TS_REG`
   bit, skipping both for `gpio > 25`; `gpio_matrix_in` stores
   `gpio | inv<<5 | SIG_IN_SEL` to `GPIO_FUNCn_IN_SEL_CFG_REG` (see
   `emulator-core/src/rom.rs`'s module doc, entry 22). The GPIO model
   stores both register arrays (store-only; nothing consults the routing
   yet). The firmware routes MOSI (`FSPID`, 65) to GPIO10, SCLK
   (`FSPICLK`, 63) to GPIO1, and `FSPIWP`/`FSPIHD` (67/66) to GPIO0.
   (GPIO0 is the pin the SPI/ST7789 model reads as D/C. That fits a bus
   config that leaves `quadwp_io_num`/`quadhd_io_num` at 0 instead of -1,
   but this is an inference from the observed calls, not confirmed.)

   **Task D11's stall** (history) was SPI2, not a ROM call.
   `spi_bus_initialize()` -> `spi_master_init_driver()` (`spi_master.c:341`)
   -> `spi_hal_init()` (`components/hal/spi_hal.c:13-28`, `0x420f_d64c` in
   factory.bin) ends with `spi_ll_apply_config()`
   (`hal/esp32c3/include/hal/spi_ll.h:264-267`): `hw->cmd.update = 1;
   while (hw->cmd.update);`. The SPI2 model kept `SPI_UPDATE` (`SPI_CMD_REG`
   bit 23, `soc/spi_reg.h`) as inert storage, so from step 584,618 on the
   CPU spun forever on the poll loop at `0x420f_d6fc..=0x420f_d702`.

   **Task 9 (SPI2 register fidelity).** `SPI_UPDATE` now reads back 0 at
   once, and transaction completion is signalled the way `spi_ll.h` reads
   it (item 3). The poll exits on its first check, and boot printed an
   (emulator-only) `clk_hal` "invalid RTC_XTAL_FREQ_REG" warning from the
   SPI clock setup; Task D12 removed it (below).

   **Task 9's stall** (history) was a ROM call again. On the 602,868th
   step boot fetched from `0x4000_0788`, the then-unstubbed libgcc ROM
   helper `__bswapsi2` (`esp32c3.rom.libgcc.ld`), called (RA `0x4039_45fa`)
   from `spi_ll_set_command()` (`spi_ll.h:1018-1030`, `0x4039_45c0` in
   factory.bin, disassembled). Its MSB-first branch writes
   `HAL_SPI_SWAP_DATA_TX(cmd, cmdlen)` to `SPI_USER2_REG`'s command value;
   that macro is `HAL_SWAP32` = `__builtin_bswap32` (`hal/misc.h:15`), a
   libcall on RV32IMC without Zbb.

   **Task D12.** `__bswapsi2` is a real stub in the libgcc `Int32Unary`
   family (`rom.rs` entry 23; observed once, step 593,018, `a0 = 0`). And
   the shortcut boot seeds `RTC_XTAL_FREQ_REG` as the skipped bootloader
   leaves it ("ROM writable `.data`" above), so neither XTAL warning prints.
   Those warnings were the first callers of `ets_get_cpu_frequency`/
   `ets_printf`, so without them the whole timeline is earlier (296 steps
   at ROM `qsort`'s return, 629 steps by the first yield, now taken on step
   528,149). Step numbers in the history paragraphs above are as measured
   in their own tasks. Boot then runs with no exception through one SPI2
   transaction (`TRANS_DONE` raw and enabled; the SPI2 source is routed to
   CPU line 6, which is not enabled yet), and `app_main`'s task blocks.

   **Task D12's stall** (history) was `wfi` (item 4). On the 596,609th
   step the FreeRTOS IDLE task's `esp_cpu_wait_for_intr()`
   (`components/esp_hw_support/cpu.c:52-64`, called from
   `esp_vApplicationIdleHook()`, `components/esp_system/freertos_hooks.c`)
   executed `wfi` (`0x1050_0073`) at `0x4038_b8bc`, which the core decoded
   as illegal, so it took an `ILLEGAL_INSTRUCTION` exception and the panic
   handler reboot-looped. By then `vSystimerSetup` had armed alarm 0 in
   period mode on counter 1 with a 160,000-tick period (10 ms at 16 MHz,
   `CONFIG_FREERTOS_HZ` 100), with `INT_ENA` bits 0 and 2 set.

   **Task 6 (WFI with SYSTIMER fast-forward).** `wfi` is now a real
   instruction (item 4). It retires and parks the core until an interrupt
   line is pending (`Cpu::is_waiting`); waking ignores `mstatus.MIE`
   (privileged spec 3.3.3), then the interrupt is taken if `MIE` is set,
   else execution resumes after the `wfi`. While the core waits with no
   line asserted, `boot::step_with_interrupts` advances SYSTIMER by
   `ticks_until_next_alarm().unwrap_or(0).max(TICKS_PER_STEP)` instead of
   one tick, then samples the lines again, so the alarm that ends the wait
   is taken in the same step. Every waiting step is still exactly one step
   of `FirmwareRuntime::run`'s budget, so a `wfi` that never wakes cannot
   hang the host. A priority-0 interrupt line is now disabled whatever the
   threshold (item 9), so it can't wake a `wfi` either.

   On the real boot the idle `wfi` on step 596,609 retires, and step
   596,610 jumps SYSTIMER 91,439 ticks to the FreeRTOS tick (routed to CPU
   line 5 at priority 1) and takes it. The idle task waits and is woken 11
   more times in the next ~25,000 steps (each jump 154,855 to 158,163
   ticks), then `app_main`'s work keeps the CPU busy: LVGL renders (the hot
   PCs are fill, `memcpy` and `memset` loops) and flushes frames to the
   ST7789 over SPI2. SPI2 raises 72 `TRANS_DONE` interrupts (then routed to
   CPU line 8), for transfers alternating 1 byte and 12,800 bytes (20 rows
   of 320 RGB565 pixels), all with `SPI_DMA_TX_ENA` set. Those pixels come
   from GDMA, which was not modeled until Task 10 (item 3), so the
   framebuffer stayed blank. By the next fault 42 FreeRTOS ticks have
   fired, 12 of them reached by fast-forward (1,827,031 ticks ahead of the
   step count in all). No new console line prints.

   **Task 10 (GDMA out-link feeding SPI2).** GDMA's TX out-link is modeled
   (item 3), and every SPI2 transaction with `SPI_DMA_TX_ENA` set now takes
   its bytes from the GDMA channel connected to SPI2. On a DMA-enabled bus
   that is *every* transaction (`spi_master.c`'s `spi_new_trans`), so
   before Task 10 the panel-init commands and their parameters were also
   being read from the stale `W0..W15` buffer, not just the pixels.
   Measured with `boot-probe` (a temporary trace, not committed): by the
   stall, 234 SPI2 transactions, all on GDMA channel 0, each delivered in
   full: the ST7789 init sequence (`DISPOFF`, `SLPOUT`, `MADCTL`, `COLMOD
   0x55`, `INVON`, ...), then `CASET`/`RASET`/`RAMWR` plus a 12,800-byte
   band (4 descriptors: 3 x 4,092 + 524, per `spicommon_dma_desc_setup_link`)
   36 times, i.e. three full 320x240 frames of 12 bands each. The D/C line
   (GPIO0's output level) was right for every one: the D11 GPIO-matrix
   routing of GPIO0 as SPI2 `FSPIWP`/`FSPIHD` did not matter: the emulator
   reads GPIO0 from `GPIO_OUT`/`GPIO_ENABLE` regardless of routing, and
   `esp_lcd_new_panel_io_spi()` itself calls `gpio_func_sel(dc,
   PIN_FUNC_GPIO)` and `gpio_output_enable(dc)` (`esp_lcd/spi/
   esp_lcd_panel_io_spi.c`), then drives the level from its `pre_cb`. The
   framebuffer is blank through step 1,128,604 and non-blank from step
   1,128,605 (a light-grey band first, then a black clear); by the stall it
   holds the firmware's boot splash, 2,340 distinct RGB565 values, laid
   out correctly in the unrotated 320x240 orientation. The timeline is
   unchanged step for step (the DMA path costs no guest instructions), so
   the stall below is the same.

   **Task 10's stall (history) was ROM `MD5Init`.** On the 5,555,258th
   step the CPU fetched from `0x4000_0614`, the unstubbed ROM `MD5Init`
   (`esp32c3.rom.ld`: `MD5Init = 0x40000614`), and took an
   `INSTRUCTION_ACCESS_FAULT` (RA `0x420f_a6ea`). The caller is ESP-IDF's
   `load_partitions()` (`components/esp_partition/partition.c:107-250`,
   `0x420f_a6ce` in factory.bin), which starts with
   `esp_rom_md5_init(&context)` before it maps and reads the partition
   table. The panic handler printed "Guru Meditation Error", and its
   reboot faulted on the unstubbed ROM `software_reset_cpu` (step
   5,795,474).

   **Task D13 (ROM MD5).** `MD5Init`/`MD5Update`/`MD5Final`
   (`0x4000_0614`/`0x4000_0618`/`0x4000_061c`, `esp32c3.rom.ld`; aliased
   as `esp_rom_md5_*` by `esp32c3.rom.api.ld`) are real stubs over the
   guest's `md5_context_t` (`emulator-core/src/rom.rs` entry 24). The ROM's
   code is Colin Plumb's public-domain MD5 (checked against its
   disassembly in Espressif's ROM ELF, including `MD5Final`'s closing
   88-byte `memset`), and `emulator-core/src/md5.rs` mirrors it, so the
   context bytes in guest memory end up as the ROM would leave them. The
   same module now also recomputes the synthesized partition table's MD5
   (it used to be a test-only copy in `flash.rs`), and a unit test drives
   `load_partitions()`'s exact call pattern through the stubs over that
   table and gets the stored digest. The stub count is 98.

   **The current stall is the flash MMU (plan Task 8, sub-unit 3), and it
   is not a fault.** The `MD5Init` call on step 5,555,258 returns. Then
   `load_partitions()` calls `spi_flash_mmap(0, 0x1000,
   SPI_FLASH_MMAP_DATA, ..)` (`0x420f_babc`), which returns `ESP_OK` with
   a window at `0x3c27_0000`. To make it, the firmware wrote MMU table
   entry 39 (`0x600c_509c`: `DR_REG_MMU_TABLE` `0x600c_5000` + 39 * 4;
   virtual page 39 of the `0x3c00_0000` DROM aperture). The bus has no MMU
   table, so that write is dropped (logged unmapped), and the reads of the
   table at `p_start + 0x8000 = 0x3c27_8000` are unmapped and return 0.
   The first entry's magic is then neither `0x50AA` nor `0xEBEB`, the loop
   ends before any `esp_rom_md5_update`, and the function takes the "No
   MD5 found in partition table" path and returns `ESP_ERR_NOT_FOUND`
   (`0x105`; `pc` is on its epilogue, `0x420f_a878`, before step
   5,566,584). `MD5Update`/`MD5Final` are never reached on the real boot
   yet. The partition list is not cached on failure, so the next lookup
   (through `ensure_partitions_loaded()`) calls it again: `MD5Init` on step
   5,583,744, the same `0x105` before step 5,591,869. This is not item 7's
   fixed top-of-aperture page: the window is an ordinary
   `spi_flash_mmap` mapping, so the fix is an MMU table model that the
   DROM/IROM read path consults.

   After that the firmware keeps running with **no exception** (checked to
   step 7,600,000): FreeRTOS ticks (CPU line 5), cross-core yields (line
   4), an interrupt on line 3, and repeated CPU-frequency switches
   (`ets_update_cpu_frequency`, `SYSTEM_CPU_PER_CONF_REG`/
   `SYSTEM_SYSCLK_CONF_REG` writes, interrupt-threshold critical
   sections). The CPU is busy, mostly in IRAM, and almost never waits in
   `wfi`. No new console line prints, and nothing new is drawn: sampled
   every 1,000 steps from step 5,555,000 to 7,600,000, the framebuffer is
   unchanged, the boot splash with 2,340 distinct RGB565 values. Pinned in
   `emulator-core/tests/rom_stub_boot.rs`'s
   `boot_stubs_reach_load_partitions_md5init_after_drawing_the_splash` (renamed and truncated in Milestone 4 Task 2; was `boot_stubs_rom_md5init_then_load_partitions_reads_zeros_through_the_unmapped_flash_mmu_window`).

   One loose end: `load_partitions()`'s `ESP_LOGE` does run
   (`esp_log_write` from `0x420f_a826`, for about 2,700 steps), but no byte
   reaches the console. Every console line so far was printed before or
   just after the scheduler started; how log output leaves the chip after
   `app_main` starts (the stdio/VFS path, and whether it waits on a
   USB-Serial-JTAG interrupt the emulator does not raise) is not yet
   checked. Until it is, a missing console line is not proof the firmware
   did not log it.

   **Task 11 (Milestone 3 finish line).** No emulator change. The
   splash is final from step 5,535,126 on, and
   `boots_to_first_real_frame` pins it by hash (see "Milestone 3's end
   state" below). The flash MMU above is the next blocker,
   left for Milestone 4.
2. **SYSTIMER: resolved (Milestone 3 Task 5).** Milestone 2 modeled only
   unit 0/target 0, with a `COMP0_LOAD` rule that contradicted the real
   HAL order. `emulator-core/src/peripherals/systimer.rs` now models both
   counters and all three comparators, register-faithful to
   `soc/systimer_reg.h` (including `CONF`'s reset value `0x4600_0000`), and
   one comparator model (documented and cited in its module doc) that
   satisfies both real call sequences: FreeRTOS's `vSystimerSetup` (alarm
   0 on counter 1, load pulsed while still oneshot, then period mode) and
   esp_timer's `systimer_hal_set_alarm_target` (alarm 2 on counter 0,
   oneshot, `MISS_COMPENSATE`). Each target drives its own source
   (`SRC_SYSTIMER_TARGET0..2`, 37..39) as a level. `advance_by(ticks)` and
   `ticks_until_next_alarm()` drive WFI fast-forward (Task 6, item 4);
   a jump across several periods fires once and re-arms to the next future
   period. Every modeled offset is in `SysTimer::handles`, so the rest
   (only `DATE`) is logged as unmapped; the RTC timer now reads
   `SysTimer::elapsed_ticks()` (monotonic) rather than a unit counter that
   firmware can stop or reload. Tick rate is still the 1-tick-per-step
   placeholder (real: 16 MHz), so a 10 ms FreeRTOS tick is 160,000 steps.
3. **SPI2 DMA transmit: resolved (Milestone 3 Task 10).** Task 9 made SPI2's registers
   match what `spi_ll.h` expects (`emulator-core/src/peripherals/spi.rs`'s
   module doc): `SPI_UPDATE` (`WT`) reads back 0 at once
   (`spi_ll_apply_config`); `SPI_USR` (`R/W/SC`) clears and
   `SPI_TRANS_DONE_INT_RAW` (`SPI_DMA_INT_RAW_REG` bit 12) sets when a
   transaction completes, which is what `spi_ll_usr_is_done` reads;
   `SPI_DMA_INT_ENA`/`_CLR`/`_ST` behave per their access types, and
   `TRANS_DONE` RAW & ENA drives `ETS_SPI2_INTR_SOURCE` (19) as a level.
   Until Task 10, `SPI_DMA_TX_ENA` was stored but unused, so every
   transaction sent the `SPI_W0..W15` buffer. Task 10 added
   `emulator-core/src/peripherals/gdma.rs` (register layout from v5.5.3
   `soc/gdma_reg.h`; the plan's offsets were not the ESP32-C3's, see that
   module doc's correction table): three channels' interrupt registers
   (`RAW`/`ST`/`ENA`/`CLR`, driving `ETS_DMA_CH0..2_INTR_SOURCE` as
   levels), the TX out-link (`OUT_LINK` `START`/`STOP`/`RESTART` firing on
   the byte that carries them, `OUT_RST`, `OUT_PERI_SEL`, EOF descriptor
   addresses) and plain-storage RX registers. When `SPI_USR` fires with
   `SPI_DMA_TX_ENA` set and a TX channel's `OUT_PERI_SEL` is SPI2 (0),
   `FirmwareBus::gdma_pull` walks that channel's `dma_descriptor_t` link in
   RAM for `SPI_MS_DATA_BITLEN` bytes, raising `OUT_DONE` per descriptor
   and `OUT_EOF`/`OUT_TOTAL_EOF` at `suc_eof`, and clearing owner bits only
   if `OUT_AUTO_WRBACK` is set. The walk reads and writes internal SRAM
   only (`SOC_DRAM_LOW..HIGH`); a descriptor or buffer anywhere else
   raises `OUT_DSCR_ERR` and stops the channel, so a bad `next` pointer
   cannot re-enter a peripheral (final-review fix; the SRAM-only rule is a
   reconstruction, see `milestone-3-decisions.md`). The SPI driver waits on SPI2's own
   `TRANS_DONE` (the `spi_intr` ISR for queued transfers,
   `spi_device_polling_end`'s `spi_hal_usr_is_done` poll for polling ones,
   `spi_master.c`), never on a GDMA interrupt, so completion needed no new
   signalling. Not modeled: the RX in-link walk, CPU FIFO push/pop, the
   `OUT_DSCR*` pre-fetch registers (read 0, logged), and timing (a
   transfer is instant at `SPI_USR`).
4. **WFI: resolved (Milestone 3 Task 6).** `wfi` used to decode as
   `Illegal`, and the FreeRTOS idle task's `wfi` trapped on step 596,609
   (Task D12's stall). It is now a real wait-for-interrupt with SYSTIMER
   fast-forward while idle (item 1's "Task 6" paragraph): the idle task
   sleeps until the FreeRTOS tick, which costs one step instead of up to
   160,000.
5. Two flagged guesses worth re-examining once boot progresses further:
   `rom_i2c_readReg*` stubbed to return 0 (currently steers boot down an
   "assume 40MHz" crystal-frequency fallback that happens to match the
   badge's real crystal — correct today, but a guess, not a verified
   value. Task D12 note: the "invalid RTC_XTAL_FREQ_REG ... assume 40MHz"
   warnings came from the unseeded `RTC_XTAL_FREQ_REG`, not from these
   stubs, and are gone now that it is seeded); `Cache_Get_*` accessor
   family stubbed to 0 (never actually called
   during the observed boot path — unverified whether that generalizes to a
   boot path that gets further).
6. **`TICKS_PER_STEP = 1`** (the emulator's steps-to-real-cycles ratio) is
   roughly 10× the real SYSTIMER/CPU clock ratio — harmless while boot never
   reaches timing-sensitive code, but will need recalibrating once it does.
7. **Resolved in Milestone 4 Task 2: entry 127 is seeded.** (Originally: the real bootloader's extra "boot partition lookup" DROM page wasn't
   modeled.) (Task D5 fix round 1, M4). Beyond widening each XIP segment to
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
   the DROM aperture near its top end. (Task D13: the partition-table
   stall that did come is a plain `spi_flash_mmap` window at `0x3c27_0000`,
   not this page; see item 1.)

8. **Resolved in Task D10: the ROM's SPI-flash legacy data was not
   initialized** (found in Task 8). The shortcut boot now seeds it (see
   "ROM writable `.data`" above). The original finding: ESP-IDF reads the
   flash chip's size and ID from `g_rom_flashchip`, i.e.
   `rom_spiflash_legacy_data->chip` (`rom_spiflash_legacy_data` is the
   ROM data word at `0x3fcd_fff0`, `esp32c3.rom.ld`). On real hardware the
   mask ROM's startup sets that pointer to its own
   `rom_default_spiflash_legacy_data` (`0x3fcd_f5c0` per the ROM ELF) and
   the 2nd-stage bootloader writes the chip size from the image header. The
   shortcut boot did neither, so the pointer read 0: `esp_flash` saw a
   0-byte chip (the `Detected size(4096k) larger than ... (0k)` warning)
   and the ID/size accesses landed at addresses `0x0`/`0x4`. Nothing had
   failed on it, but `esp_flash_default_chip->size` was 0.

9. **Edge interrupts are simplified** (Task 4). Arbitration is no longer
   simplified: since Task 5, when several enabled lines at or above the
   threshold are pending at once, the core takes the highest-priority one,
   ties to the lowest line number (ESP32-C3 TRM v1.4 section 1.5.2;
   `cpu::select_interrupt_line`). `CPU_INT_TYPE_REG` (edge vs. level) and
   `CPU_INT_CLEAR_REG` are plain storage: every source wired so far
   (SYSTIMER targets 0..2, SYSTEM FROM_CPU, SPI2 TRANS_DONE) is level-type,
   and the SYSTIMER's own latched `INT_RAW` stands in for its edge behavior. Neither has
   mattered in the observed boot. `FirmwareBus::pending_sources()` only
   includes SYSTIMER targets 0..2 (Task 5), SYSTEM FROM_CPU0..3, SPI2
   (Task 9) and GDMA channels 0..2 (Task 10) so far. Since Task 6, a line whose 4-bit priority is 0 is
   disabled (never asserted to the core, whatever the threshold; ESP32-C3
   TRM v1.4 section 1.5.2, and ESP-IDF's `esprv_int_set_priority` takes
   levels 1 to 7). Espressif's QEMU model admits a priority-0 line under
   threshold 0; this emulator follows the TRM.

Item 8 was a live divergence, not a dormant one: it printed a warning the
real badge doesn't and sent real accesses to address 0, until Task D10
fixed it. Items 2, 3 and 4 are resolved (Tasks 5, 10 and 6): boot now
draws the firmware's boot splash into the framebuffer, and it stays on
screen through the flash-MMU stall (item 1). Items 5 to 7 and 9 are not
correctness bugs *today*; they're dormant because boot doesn't reach the
code paths that would exercise them. They were recorded so Milestone 3
started from a known list instead of rediscovering each one by stepping
through a debugger, and they carry over to Milestone 4 in "Open
limitations" at the top of this section.

### Milestone 3's end state, with Milestone 4's interim updates

Superseded by "Current state (end of Milestone 4)" above; kept as the
record of where Milestone 3 ended, the updates each Milestone 4 task made
to it, and Milestone 3's open-limitations list.

Real-firmware boot reaches its **first real ST7789 frame**: the firmware's
boot splash ("Hack the North × SOLANA" and the FW version line; no
personal data). From a cold shortcut boot of `factory.bin`:

- The framebuffer is blank through step 1,128,604. It is first non-blank
  on step 1,128,605 (2 distinct RGB565 values, not yet the finished
  splash).
- It changes 33 times (sampled every 1,000 steps) and is final from step
  5,535,126 on: 2,340 distinct values, unchanged at every 1,000-step sample
  to step 10,000,000.
- `emulator-core/tests/boot_progress.rs`'s `boots_to_first_real_frame`
  pins that stable frame by an FNV-1a hash of the framebuffer
  (`0x5599c270ab0429fa`). It samples every 250,000 steps and stops once
  the hash has held for 1,000,000 steps (at step 6,750,000). It takes
  about 0.5 s with `--release` and about 6.5 s in a debug build.
- 2026-10-06: the human partner compared this frame by eye with the
  physical badge's first screen and confirmed they match.
- To look at the frame:
  `cargo run -p emulator-core --release --example boot-probe -- --steps 6600000 --dump-frame first-frame.png`
  writes a PNG under the gitignored `local/`.

After the frame, boot takes no instruction-access fault and prints no
panic text (checked to step 10,000,000; history item 1 checked for no
exception at all to step 7,600,000), but it never reaches the app launcher or a built-in app. Once the splash is
drawn, `load_partitions()` reads the partition table through a
`spi_flash_mmap` window, the flash MMU behind it is not modeled, the
table reads as zeros, and partition loading fails with
`ESP_ERR_NOT_FOUND` (history item 1's "Task D13" paragraphs have the
details). The firmware keeps scheduling with the splash on screen.

**Update (Milestone 4 Task 2):** the flash MMU is now modeled, so that stall
is gone; boot proceeds past `load_partitions()`'s read (ROM `strncpy` and
`strcmp` are now real stubs) with no fault through step 10,000,000, but no
new console line or frame yet (see the history entry at the end of the stall
log).

**Update (Milestone 4 Task 3):** partition loading succeeds: `load_partitions()`
returns `ESP_OK` (observed 4 `MD5Update` calls and 1 `MD5Final`), pinned by
`boot_progress.rs`'s
`load_partitions_accepts_the_synthesized_table_through_the_flash_mmu` (which
asserts `ESP_OK`, exactly 1 `MD5Final` and at least 1 `MD5Update`). The
next stall is a poll of SPIMEM1's `CMD` register for `SPI_MEM_FLASH_WREN` to
clear (see the "Milestone 4 Task 3" history entry).

**Update (Milestone 4 Task 4):** log output after the scheduler starts now
reaches the emulated console (open limitation 2 below is resolved). After
`main_task: Calling app_main()` the console shows `LVGL: Starting LVGL task`
and then `esp_littlefs`'s mount failure on the blank `storage` partition
(`Corrupted dir pair at {0x0, 0x1}`, then `mount failed, (-84).
formatting...`). The format is what issues the `SPI_MEM_FLASH_WREN` poll
above, so the stall itself is unchanged (see the "Milestone 4 Task 4"
history entry).

**Update (Milestone 4 Task 5):** SPIMEM1 now runs the flash driver's
dedicated write-enable, sector-erase and page-program commands, `RDSR` and
the DIO fast read, so that poll is gone and `esp_littlefs` formats the blank
`storage` partition (the littlefs superblock lands in its blocks 0 and 1).
ROM `strlcat`, `strspn` and `strcspn`, which the littlefs VFS registration
and path walk call next, are real stubs, so the console continues with
`hal_fs: littlefs mounted at /littlefs (used 8192 / 1310720 bytes)`,
`hal_identity`'s "badge is unprovisioned" warning (blank flash) and
`hal_buttons: buttons ready`. There is no fault through step 30,000,000.
Boot then stops printing: the physical badge's next line is `hal_accel`'s
accelerometer detection, and the firmware is driving the unmodeled I2C0
controller at that point (see the "Milestone 4 Task 5" history entry).

**Update (Milestone 4 Task D-M4-1):** I2C0 is modeled as a master with no
device on the bus, so the accelerometer probe's address byte is NACKed
and the driver's ISR reports it; the console continues with
`E (470) hal_accel: accelerometer setup failed: ESP_ERR_INVALID_STATE`
(non-fatal; the physical badge, which has the accelerometer, prints its
detection line instead). The next function converts a clock frequency
with ROM libgcc's soft-double helpers, now real stubs. There is no fault
through step 30,000,000. Boot then stops printing while the firmware
drives the unmodeled RMT controller (see the "Milestone 4 Task D-M4-1"
history entry); the physical badge's next line is `hal_sleep`'s.

**Update (Milestone 4 Task D-M4-2):** RMT is modeled as a TX engine that
completes each transmission in zero emulated time
(`milestone-4-decisions.md`), so the LED driver's transmission ends and
the console continues with `hal_sleep: sleep manager ready (...)`. ROM
`strdup` (guest code calling the firmware's `_malloc_r`), `strchr` and
`strcpy` are now real, so the app registry enters and launches its first
app, `I (890) app_reg: launched My Badge` (step ~12.29M). On the blank
emulated flash that app shows its unregistered-badge screen: the
framebuffer leaves the splash from step ~13.36M and settles (45 colors)
by step ~15.48M. There is no fault through step 30,000,000; the firmware
then idles waiting for input (see the "Milestone 4 Task D-M4-2" history
entry).

**Open limitations at the end of Milestone 3 (Milestone 4 candidates)**, the first one being the
current blocker (the backlog in `milestone-3-decisions.md` is the complete
list):

1. **Flash MMU (plan Task 8, sub-unit 3): modeled in Milestone 4 Task 2.**
   *(The text below describes the state at the end of Milestone 3; the MMU
   now exists and boot proceeds past it, see the "Milestone 4 Task 2" history
   entry.)* The bus has
   no MMU table: the firmware's entry writes (`DR_REG_MMU_TABLE`,
   `0x600c_5000`) are dropped and reads through a `spi_flash_mmap` window
   return 0. The fix is an MMU table model that the DROM/IROM read path
   consults, backed by the emulated flash chip. Milestone 3 deliberately
   stopped short of it: the spec builds flash/partition support only if
   boot reaches it before the first frame, and the first frame comes
   first.
2. **Console output after the scheduler starts: resolved in Milestone 4
   Task 4** (the USB-Serial-JTAG model now reports SOF frames from an
   attached host; see the "Milestone 4 Task 4" history entry). *(The text
   below describes the state before the fix.)*
   `load_partitions()`'s `ESP_LOGE` runs (`esp_log_write`), but no byte
   reaches the emulated console. Every console line so far was printed
   before or just after the scheduler started. How later log output
   leaves the chip (newlib stdout through the VFS, and whether it waits on
   a USB-Serial-JTAG interrupt the emulator does not raise) is not traced.
   Until it is, a missing console line is not proof the firmware did not
   log it.
3. **ST7789 `MADCTL` is not modeled.** The firmware sends `MADCTL` `0x20`
   and then `0x60` (row/column exchange, then also column-address
   mirroring). `emulator-core/src/peripherals/spi.rs` ignores `MADCTL` and
   hardcodes the unrotated 320x240 order. The splash comes out right only
   because the firmware's address window fits that default order. So
   `boots_to_first_real_frame`'s pinned hash is of *this* framebuffer
   orientation; modeling `MADCTL` may change the hash without the image
   being wrong.
4. The dormant items from the history below: flagged ROM-stub guesses
   (item 5, and `CPU_FREQ_MHZ` = 160 against the badge's real 80 MHz; see
   "Ground truth from the physical badge"), `TICKS_PER_STEP = 1` (item 6),
   the bootloader's extra DROM page (item 7) and simplified edge
   interrupts (item 9).
5. **The eFuse block is not modeled**, so the emulated boot logs
   `efuse_init: Chip rev: v0.0` where the real badge reports v0.4 (see
   "Ground truth from the physical badge"). Nothing has stalled on it.
6. **Every log timestamp up to `main_task: Calling app_main()` reads
   `I (0)`** (likewise `W (0)`/`E (0)`). The lines printed after it
   (Milestone 4 Task 4) carry non-zero timestamps (`I (120)`, `E (430)`,
   ...), which on ESP-IDF come from the FreeRTOS tick count once the
   scheduler runs. On
   real hardware ESP-IDF's log timestamp is milliseconds since boot and
   advances line by line. The likely cause (not traced) is that the early
   timestamp comes from the CPU cycle counter, and the ESP32-C3's
   performance-counter CSR is not modeled. Cosmetic; no test depends on
   it.

### Milestone 4 Task 2: flash MMU

XIP now goes through the flash MMU instead of D5's page windows
(`emulator-core/src/peripherals/mmu.rs`, `FirmwareBus::mmu`; decisions and
what-if-wrong notes in [`milestone-4-decisions.md`](milestone-4-decisions.md)).
Reads in the DBUS/IBUS cache apertures translate through the table to
`flash_chip`; `FirmwareBus::from_segments` seeds the table by replaying the
bootloader's `set_cache_and_start_app()` mapping (including entry 127, which
resolves history item 7). `XipRegion`, `xip_page_window` and the bus's own
copy of the image are deleted; a SPIMEM1 flash program is now visible through
XIP.

- **Splash unchanged.** The framebuffer hash `0x5599c270ab0429fa` is reached
  at the same 5,750,000-step sample as before, and every earlier boot-ladder
  rung still passes.
- **Boot moves on.** `load_partitions()` now reads the real
  synthesized table, so boot proceeds past the Milestone 3 stall. Just
  after `MD5Init` it called ROM `strncpy` (`0x4000_0368`), then `strcmp`
  (`0x4000_036c`); both are now real HLE stubs
  (`RomStubEffect::Strncpy`/`Strcmp`).
  `boots_to_first_real_frame` passes again with hash `0x5599c270ab0429fa`
  unchanged, no fault through step 6.75M. The pinned-stall test in
  `rom_stub_boot.rs` was truncated to its Phases 1 to 10.
- **Performance.** `boot-probe --steps 5750000` (release), median of three:
  0.40 s before, 0.39 s after; no translation cache was needed.

### Milestone 4 Task 3: `load_partitions()` succeeds; the next stall

The rung `load_partitions_accepts_the_synthesized_table_through_the_flash_mmu`
(`boot_progress.rs`) single-steps from the first `MD5Init` to
`load_partitions()`'s epilogue (`0x420f_a878`): 4 `MD5Update` calls (one per
partition entry), 1 `MD5Final`, `s4` = `ESP_OK`, no DBUS read through an
invalid MMU entry, no fault. Observed 4 `MD5Update` calls, one per table entry
(the rung asserts at least 1).

Next stall, from `boot-probe --steps 12000000 --window 500000` (release):

- **Fault:** none (303 traps, all interrupts; 8,136 ROM stub calls).
- **Console:** unchanged, last line `I (0) main_task: Calling app_main()`.
- **Framebuffer:** still the splash (2,340 distinct values).
- **Hot PCs (last 500,000 steps):** `0x4039_3916`, `0x4039_3918`,
  `0x4039_391a` (about 166,000 hits each, i.e. nearly all of the time the
  CPU is not in the scheduler/idle path). Disassembly of the IRAM segment
  (`0x4038_0000`) shows `c.lw a5,4(a0); c.lw a5,0(a5); c.bnez a5,-4`: a
  loop that loads a register pointer from `a0+4` and re-reads it until it
  reads 0. At step 11,000,000 `a0` = `0x3fca_6b48`, the pointer is
  `0x6000_2000` (SPIMEM1 `SPI_MEM_CMD_REG`) and the value read is
  `0x4000_0000`, i.e. `SPI_MEM_FLASH_WREN` (bit 30, self-clearing "SC" in
  `soc/spi_mem_reg.h`). Return address `0x4039_3e42`. The SPIMEM1 model
  stores the dedicated `SPI_MEM_FLASH_*` command bits but never fires or
  clears them, so the poll never ends. The other hot PCs
  (`0x4038_e4c2..0x4038_e4dc`, `0x4038_f930..`) are short and not identified (probably the tick/idle path).
- **SPIMEM1:** 26 user transactions; recent unmodeled user commands:
  `0xbb` (fifteen times, a dual-I/O read) then `0x05` (`RDSR`).
- **Unmapped-access tail:** `0x600c_e000..0x600c_e03c` (the
  ASSIST_DEBUG block, `DR_REG_ASSIST_DEBUG_BASE`): reads and writes of
  `+0x000`, writes of `+0x038` and `+0x03c` (12 each), `+0x000` 20 each.
- **Resolved in Milestone 4 Task 5** (see that entry): the bits below,
  `RDSR` and the `0xbb` reads are modeled.
- **Next task:** model SPIMEM1's dedicated `SPI_MEM_FLASH_*` command bits in
  `CMD` (at least `WREN`, and the ones `esp_flash` issues next: `RDSR`,
  `PP`, `SE`/`BE`, `READ`), with their self-clear and the effect on the
  emulated flash chip; the `0xbb` reads and `RDSR` as user commands are
  related. ASSIST_DEBUG accesses are probably a separate, benign later item.

### Milestone 4 Task 4: console output after the scheduler starts

**Found.** Boot entered `esp_log_write` (`0x4039_708e` in factory.bin) eight
times in the first 12,000,000 steps, but only the two `main_task` calls
(steps ~529,200 and ~549,400) produced bytes. The first silent one after
them was `LVGL: Starting LVGL task` (step ~663,700); the last two were
`esp_littlefs` errors (steps ~5,585,500 and ~5,591,700). Diffing the
control flow of the `Calling app_main()` call against the `LVGL` one
showed both reach `vprintf` and newlib's write path identically, and part
at the VFS write (`0x4200_8160`): `usb_serial_jtag_write()` in
`esp_driver_usb_serial_jtag/src/usb_serial_jtag_vfs.c` (v5.5.3) starts with
`if (!usb_serial_jtag_is_connected()) return -1;`.
`usb_serial_jtag_is_connected()` (`0x4200_7ae6`) returns
`s_usb_serial_jtag_conn_status`. In
`usb_serial_jtag_connection_monitor.c`, `usb_serial_jtag_conn_status_init`
sets it true (step ~501,800) and registers
`usb_serial_jtag_sof_tick_hook` (`0x4038_1b16`), a FreeRTOS tick hook that
reads `USB_SERIAL_JTAG_INT_RAW_REG`, clears `SOF_INT` through
`USB_SERIAL_JTAG_INT_CLR_REG`, and sets the flag false once a tick passes
without `SOF_INT_RAW` (bit 1) and the tolerance is used up
(`ALLOWED_NO_SOF_TICKS` is 0 in this build: the init stores 0 into
`remaining_allowed_no_sof_ticks`, `0x4200_7acc`). The model never set `SOF_INT_RAW`, so the first
tick hook (step 597,330) marked the port disconnected, and from then on
every `stdout` write returned -1 with no FIFO access. Not the cause:
the interrupt-driven `usb_serial_jtag` driver (the VFS write never got as
far as its `tx_func`, and the physical badge starts its console REPL, the
usual installer of that driver, only after the launcher), newlib locks, and
log level or line buffering (the bytes never reached the VFS write's loop).

The `pm` and `coexist` `esp_log_write` calls before step ~501,800 are also
dropped, because the flag starts false until the init function runs. That is
real behavior: the physical badge's serial log has no such lines either.

**Resolved.** `crate::peripherals::usb_serial_jtag` forces `SOF_INT_RAW` to
read 1, like `SERIAL_IN_EMPTY_INT_RAW`: the emulated badge has a host
attached that sends a start-of-frame every 1 ms, so the bit is set again by
the next read whatever `INT_CLR` did (decision and what-if-wrong in
`milestone-4-decisions.md`). Unit test
`sof_int_raw_reads_set_even_after_a_byte_split_int_clr` (failed before the
fix on its first assertion: the bit read 0).

**After the fix**, from `boot-probe --steps 12000000 --window 500000`
(release):

- **Console:** the tail after `main_task: Calling app_main()` is
  `I (120) LVGL: Starting LVGL task`, then
  `E (430) esp_littlefs: ./managed_components/joltwallet__littlefs/src/littlefs/lfs.c:1383:error: Corrupted dir pair at {0x0, 0x1}`
  and `W (430) esp_littlefs: mount failed,  (-84). formatting...`. The
  littlefs lines come from the blank (`0xFF`) emulated `storage`
  partition; the physical badge mounts its filesystem here instead.
  Rungs: `boot_prints_console_output_after_the_scheduler_starts`
  (`LVGL: Starting LVGL task` within 1,000,000 steps) and
  `boot_reaches_littlefs_formatting_the_blank_storage_partition`
  (`esp_littlefs: mount failed` and `formatting...` within 6,000,000).
- **Timeline:** printing the `LVGL` line through the VFS costs 3,736 guest
  steps, so `load_partitions()`'s `MD5Init` moves from step 5,555,258 to
  5,558,994 (`tests/rom_stub_boot.rs`'s pinned-stall test was updated; its
  console check is now containment, not "the last line"). Every earlier
  step count is unchanged. `boots_to_first_real_frame`'s hash is unchanged.
- **Stall:** unchanged, the `SPI_MEM_FLASH_WREN` poll at
  `0x4039_3916..0x4039_391a` (no fault, 303 traps, framebuffer still the
  splash with 2,340 values). It is littlefs's format, after the mount
  failure, that writes to flash.

### Milestone 4 Task 5: flash writes through SPIMEM1; littlefs formats `storage`

**Found.** Disassembly of factory.bin's IRAM segment around the Task 3/4
poll identifies the ESP-IDF v5.5.3 HAL functions (`spi_flash_hal_iram.c`,
`spi_flash_hal_common.inc`, LL in `hal/esp32c3/include/hal/spimem_flash_ll.h`,
bits from `soc/esp32c3/register/soc/spi_mem_reg.h`):

- `0x4039_3916` is `spi_flash_hal_poll_cmd_done` (`while (dev->cmd.val != 0)`).
- `0x4039_3e2a` is `spi_flash_hal_set_write_protect`: `CMD |= 0x4000_0000`
  (`SPI_MEM_FLASH_WREN`, bit 30) or `0x2000_0000` (`FLASH_WRDI`, bit 29),
  then the poll; the poll's return address `0x4039_3e42` is the one Task 3
  recorded.
- `0x4039_3d4a` is `spi_flash_hal_erase_sector`: `USER1.usr_addr_bitlen =
  23`, `USER.usr_addr = 1`, `ADDR = start & 0xFFFFFF`, `CTRL = 0`,
  `CMD |= 0x0100_0000` (`FLASH_SE`, bit 24).
- `0x4039_3ddc` is `spi_flash_hal_program_page`: the same address setup with
  `ADDR = (address & 0xFFFFFF) | length << 24`, then
  `spimem_flash_ll_program_page` (`0x4039_3896`: clear `USER.usr_dummy`,
  copy the data into `W0..`, `CMD |= 0x0200_0000`, `FLASH_PP`, bit 25).
- Erase chip (`FLASH_CE`, bit 22) and erase block (`FLASH_BE`, bit 23)
  sit next to them but are not called during boot.

These are the SPI1 host driver's functions (`ESP_FLASH_DEFAULT_HOST_DRIVER()`
in `spi_flash/include/memspi_host_driver.h`). Status polls go through the
user command `RDSR` (0x05, `memspi_host_read_status_hs`) and reads through
the user command `CMD_FASTRD_DIO` (0xBB, `spi_flash_hal_read`; the chip runs
in DIO mode). Both had been completing with no effect (the `0xbb` reads
returned stale buffer bytes). The generic chip driver
(`spi_flash_chip_generic.c`) also reads the write enable latch back
(`SR_WREN`, status bit 1) after `WREN` and after every erase or program,
so the latch has to be modeled for any write to succeed.

**Resolved.** `crate::peripherals::flash::Spimem1` runs `FLASH_WREN`,
`FLASH_WRDI`, `FLASH_SE` and `FLASH_PP` on the byte write that sets their
bit and clears it, keeps the chip's write enable latch (set by `WREN`,
cleared by `WRDI` and by an accepted erase or program; erase and program
are ignored without it), answers `RDSR` with `WIP` = 0 and the latch, and
serves the 24-bit-address reads (0x03, 0x0B, 0x3B, 0x6B, 0xBB, 0xEB) from
the emulated chip. Other dedicated bits complete with no effect and are
logged. Ten unit tests in `flash.rs` replay each HAL function's register
writes as byte-split words (all failed before the change). Decisions are in
`milestone-4-decisions.md`.

**After the fix** (release `boot-probe`):

- `esp_littlefs`'s format runs: 83 SPIMEM1 transactions, no unmodeled
  command. The littlefs superblock is written to `storage` blocks 0
  (`0x2b_0000`, by step ~5,654,200) and 1 (`0x2b_1000`, by step
  ~5,690,100); `nvs` is still blank. New rung:
  `boot_formats_the_blank_storage_partition_with_littlefs` (both
  superblocks within 8,000,000 steps, no fault, nothing unmodeled, write
  enable latch clear afterwards). The Task 4 rung's budget is widened to
  8,000,000.
- **Console:** unchanged up to `W (430) esp_littlefs: mount failed,  (-84).
  formatting...`; the `Corrupted dir pair` error before it now comes from
  reading the real blank partition instead of stale buffer bytes. Then
  `Guru Meditation Error: Core  0 panic'ed (Instruction access fault)`,
  `MEPC 0x400003ec`, `RA 0x420f2776`, a register dump, `Rebooting...`, and
  `Panic handler entered multiple times` repeating.
- **Next stall, then fixed in the same task (fix round 1):** an
  instruction fetch fault at ROM `strlcat` (`0x4000_03ec`,
  `esp32c3.rom.libc.ld`; ROM ELF `__call_strlcat`) at step ~5,728,800.
  The caller (`0x420f_2740`, return address `0x420f_2776`) calls a
  function that returns 0, then
  `strlcat(_efs[index] + 0xc, conf->base_path, 16)` with `_efs` at
  `0x3fcb_c2a8`. That matches `esp_vfs_littlefs_register` in
  `joltwallet/esp_littlefs` (`esp_littlefs_init`, which mounts and formats,
  returned `ESP_OK`; then `strlcat` into `base_path`,
  `ESP_VFS_LITTLEFS_PATH_MAX + 1` = 16). The panic handler's reboot then
  faulted on the unstubbed `software_reset_cpu` (`0x4000_0094`). With a
  real `strlcat` stub, the next fault was ROM `strspn` (`0x4000_0410`,
  return address `0x420f_721c`), in a littlefs path-walk loop that calls
  `strcspn` (`0x4000_03e4`, from `0x420f_71ea`) right after. Both are now
  real stubs too; first calls at steps 5,728,808 (`strlcat`, once),
  5,872,938 (`strspn`, 15 calls to step 12M) and 5,872,946 (`strcspn`, 16).
  Unit tests: `strlcat_stub_appends_within_siz_terminates_and_returns_the_tried_length`,
  `strspn_stub_counts_the_leading_bytes_in_the_set`,
  `strcspn_stub_counts_the_leading_bytes_not_in_the_set` (each failed
  first with a trap). `boots_to_first_real_frame` and its WASM twin keep
  their full fault and panic window (an earlier draft had narrowed it; see
  `milestone-4-decisions.md`).
- **Console now:** after `formatting...`,
  `I (440) hal_fs: littlefs mounted at /littlefs (used 8192 / 1310720 bytes)`
  (the physical badge prints the same line with its own usage),
  `W (450) hal_identity: no identity at /littlefs/identity.json — badge is
  unprovisioned` (blank flash; the physical badge has an identity file),
  `I (470) hal_buttons: buttons ready`. New rungs
  `boot_reaches_hal_fs_littlefs_mounted` (8,000,000) and
  `boot_reaches_hal_buttons_ready` (10,000,000).
- **Current stall** (release `boot-probe` and scratch tracing to step
  30,000,000): no fault and no panic. The console gets no new line after
  `hal_buttons: buttons ready` (printed between steps 6M and 7M). The
  physical badge's next line is `hal_accel: SC7A20H detected (0x11)`.
  What it is: the firmware programs I2C0 (`0x6001_3000`, not modeled,
  every access logged as unmapped) from step ~6,040,000 on: `+0x04`,
  `+0x18`, `+0x1c`, `+0x24`, `+0x28`, `+0x50`, `+0x54` and the command
  registers `+0x58..+0x6c`, presumably the accelerometer probe, and gets
  zeros back (no I2C model, no I2C interrupt). What it is not: a spin or a
  flash problem. SPIMEM1 shows 471 transactions and nothing unmodeled. The
  IROM hot PCs are the idle task's hook loop (`0x4212_81d2..`, 8 hook
  slots) and `gpio_set_level` (`0x420f_917c`) on GPIO20/GPIO21: the
  74HC165 button scan (LOAD/CLK, 8 clocks per latch), so `hal_buttons` is
  polling normally. The IRAM hot PCs `0x4038_f930..` are FreeRTOS critical
  sections (writes to `INTERRUPT_CORE0_CPU_INT_THRESH`, `0x600c_2194`).
  The framebuffer is still the splash (2,340 values, hash unchanged).
  Other unmapped accesses: SPIMEM1 `+0xA4` (`SPI_MEM_SUS_STATUS_REG`, read
  by `spi_flash_hal_check_status`; now named in `Spimem1::handles`),
  IO_MUX `0x6000_9000`, SYSTEM `0x600c_0000` (`+0x08`, `+0x58`), RTC_CNTL
  `0x6000_80bc`, and ASSIST_DEBUG `0x600c_e0xx`. Next task: model I2C0
  enough for the accelerometer probe, or trace why `hal_accel` waits.

### Milestone 4 Task D-M4-1: I2C0 with no device; the RMT stall

- **Found:** the Task 5 stall above. The firmware uses ESP-IDF v5.5.3's
  `esp_driver_i2c` master driver: `s_i2c_send_commands()` writes the
  command list and TX FIFO, sets `CTR.TRANS_START`, then blocks on
  `event_queue`, which only the I2C ISR (`ETS_I2C_EXT0_INTR_SOURCE`,
  source 29) feeds. With I2C0 unmodeled, no interrupt ever came.
- **Resolved:** `peripherals/i2c.rs` models I2C0 (`0x6001_3000`) as a
  controller with **no device attached** (`milestone-4-decisions.md`).
  The command list runs inside the byte write that sets `TRANS_START`;
  every ACK slot reads 1, so the address byte of the `hal_accel` probe is
  NACKed: `NACK_INT`, then a STOP (bus released, `TRANS_COMPLETE_INT`).
  `INT_STATUS` (`INT_RAW & INT_ENA`) drives `SRC_I2C_EXT0`; the ISR clears
  it through `INT_CLR` and posts `I2C_EVENT_NACK`, and the transaction
  returns `ESP_ERR_INVALID_STATE`. Offsets, access types and every
  non-zero reset value come from `i2c_reg.h`/`i2c_struct.h`; accessors from
  `i2c_ll.h`. Unit tests in `i2c.rs` (12 failed against the stub first,
  then a bus-routing test `i2c0_probe_through_the_bus_nacks_and_asserts_its_source`).
- **Console now:** after `hal_buttons: buttons ready`,
  `E (470) hal_accel: accelerometer setup failed: ESP_ERR_INVALID_STATE`
  (step ~6.09M). The physical badge prints `hal_accel`'s detection line
  instead. New rung `boot_reports_the_absent_accelerometer_after_i2c0_nacks`
  (10,000,000, plus 1,000,000 fault-free steps after the line).
- **Then a ROM fault, fixed in this task (ruling R10):** the next function
  (`0x420e_d7f0` onward; `a0 = 10_000_000`, a clock frequency) faulted at
  `0x4000_080c`, ROM `__floatunsidf` (`esp32c3.rom.libgcc.ld`), and calls
  `__muldf3` (`0x4000_0848`), `__divdf3` (`0x4000_07b0`) and
  `__fixunsdfsi` (`0x4000_07e8`) in sequence after it. All four are real
  stubs (`RomStubEffect::SoftDouble`, libgcc soft-fp semantics:
  round-to-nearest, canonical quiet NaN `0x7FF8_0000_0000_0000`,
  `_FP_TO_INT`'s unsigned saturation). The fault fell inside
  `boots_to_first_real_frame`'s fault window, so the stubs are what keeps
  the finish line green. Unit tests
  `soft_double_ops_match_libgcc_soft_fp` and
  `soft_double_rom_stubs_use_the_rv32_register_pairs` (both failed first
  against an `apply` that returned 0).
- **Current stall** (release `boot-probe` and scratch tracing to step
  30,000,000): no fault and no panic; no console line after the
  `hal_accel` one. The framebuffer is still the splash (2,340 values, hash
  unchanged). From step ~6.10M the firmware programs the **RMT**
  controller (`DR_REG_RMT_BASE = 0x6001_6000`, `RMTMEM = 0x6001_6400`;
  not modeled, logged as unmapped): `RMT_CH0CONF0_REG` (`+0x10`),
  `RMT_INT_ENA_REG` (`+0x40`), `RMT_INT_CLR_REG` (`+0x44`),
  `RMT_CH0_TX_LIM_REG` (`+0x58`), `RMT_SYS_CONF_REG` (`+0x68`),
  `RMT_TX_SIM_REG` (`+0x6c`), `RMT_REF_CNT_RST_REG` (`+0x70`), and from
  step ~6.12M writes the channel RAM words `+0x42c..+0x528`, then nothing
  more. Presumably a TX (the 10 MHz resolution computed just before)
  waiting for an RMT interrupt (`ETS_RMT_INTR_SOURCE`, source 28) that
  never comes; the waiting task is not traced yet. The hot PCs are the
  same as at the Task 5 stall: IRAM `0x4038_f930..` (FreeRTOS critical
  sections) plus the idle hooks and the 74HC165 button scan, so the rest
  of the system runs normally. Other unmapped accesses are unchanged
  (ASSIST_DEBUG `0x600c_e0xx`, SYSTEM `0x600c_0058`). SPIMEM1: 471
  transactions, nothing unmodeled. Next task: model RMT TX.

### Milestone 4 Task D-M4-2: RMT TX; the first app launches

- **Found:** the D-M4-1 stall above, traced: the firmware creates TX
  channel 0 with two RAM blocks (`RMT_MEM_SIZE_CH0` = 2, 96 words),
  `TX_LIM` 48, wrap mode on, enables `CH0_TX_THR_EVENT` and `CH0_TX_END`,
  fills all 96 words with WS2812-style symbols (`0x0009_8003`: level 1 for
  3 ticks, level 0 for 9), sets `TX_START`, and blocks in
  `rmt_tx_wait_all_done()` (ESP-IDF v5.5.3 `esp_driver_rmt/src/rmt_tx.c`,
  non-DMA ping-pong: each threshold interrupt refills the half just sent).
  No interrupt ever came.
- **Resolved:** `peripherals/rmt.rs` models RMT (`0x6001_6000`, RAM at
  `+0x400..+0x700`) as a TX engine that consumes symbols inside the
  `TX_START` write, stops at an end-marker (`TX_END`), pauses after each
  *enabled* `TX_THR_EVENT`/`TX_LOOP` until the ISR clears or disables it,
  wraps in wrap/continuous mode, and raises `ERR` + `MEM_EMPTY` on a
  non-wrap overrun. `INT_ST` drives `SRC_RMT` (source 28,
  `INTERRUPT_CORE0_RMT_INTR_MAP_REG` = `0x070`). Offsets, access types and
  reset values from `rmt_reg.h`/`rmt_struct.h`, accessors from
  `rmt_ll.h`, behavior from the ESP32-C3 TRM v1.4 chapter 33. Unit tests
  in `rmt.rs` (9 of 12 failed first: 8 against an empty transmitter, 1 on
  a `MEM_RADDR_EX` reset-value bug; the `ping_pong_...` test replays the
  observed transaction) and a bus test
  `rmt_transmission_through_the_bus_asserts_its_source` (failed before the
  routing). Decisions (zero latency, pause-until-ack) in
  `milestone-4-decisions.md`.
- **Console now:** `I (470) hal_sleep: sleep manager ready (idle 300s,
  wake on START)` (step ~6.14M). New rung
  `boot_reaches_hal_sleep_after_the_rmt_transmission_completes`
  (8,000,000; channel 0 not left running).
- **Then three ROM faults, fixed in this task (ruling R10):** ROM newlib
  `strdup` (`0x4000_03dc`, return address `0x420f_1214`), then ROM libc
  `strchr` (`0x4000_03e0`, return address `0x420f_7ee4`), then, after
  `app_reg: heap after exit ?: ...` (step ~8.23M) and `app_reg: heap after
  enter My Badge: ...`, ROM libc `strcpy` (`0x4000_0364`, return address
  `0x4206_1f20`). `strchr`/`strcpy` are atomic real stubs; `strdup` is
  guest-executed ROM code like `qsort`, because it calls the firmware's
  `_malloc_r` through ROM newlib's `syscall_table_ptr` (`rom.rs` module
  doc entry 25). Each test failed first (a trap at the ROM address, or
  a missing effect).
- **Console now:** `I (890) app_reg: launched My Badge` (step ~12.29M).
  New rung `boot_launches_the_first_app_and_leaves_the_splash_without_faulting`
  (16,000,000 to the line, then fault- and panic-free to 20,000,000 with
  the framebuffer changed). `boots_to_first_real_frame` still passes
  unchanged: the splash is stable from 5,535,126 until the app draws at
  ~13.36M.
- **Current state** (release `boot-probe` to step 30,000,000): no fault, no
  panic, 9,599 traps (interrupts), 73,359 ROM stub calls. The
  framebuffer shows the "My Badge" app's unregistered-badge screen (45
  values; frame dumped to `local/m4-task-d2-frame.png`, which shows no
  personal data: the emulated flash is blank). The last console line is
  `app_reg: launched My Badge`; the hot PCs are the same idle set as
  before (IRAM `0x4038_f930..0x4038_f97c`, FreeRTOS critical sections,
  plus the idle hooks and the 74HC165 button scan), i.e. the firmware is
  waiting for input, not stalled on a peripheral. Unmapped tail:
  ASSIST_DEBUG `0x600c_e000`/`0x600c_e038`/`0x600c_e03c` only. SPIMEM1:
  1,735 transactions, nothing unmodeled. The physical badge prints the
  same `hal_sleep` and three `app_reg` lines (with its own heap figures
  and timestamps). Task 7 found that My Badge branches on whether the
  badge is provisioned (an identity at `/littlefs/identity.json`); a
  factory-reset physical badge shows this same first-run screen
  (human-confirmed, see "Current state"). What a provisioned badge
  shows was not compared.

### Milestone 4 Task 7: the finish line (first-run screen)

- **Found** (release scratch runs over `FirmwareRuntime`, frames dumped
  under `local/`): the first stable non-splash frame is My Badge's
  first-run screen (see "Current state" for steps and hashes). No input
  path or emulator gap stops boot: the firmware waits for input.
- **Buttons on the first-run screen:** HOME, B, A, DOWN, LEFT, RIGHT, UP
  and AUX1, each held 100,000 steps (HOME, B and A also for 1M, 4M and
  30M), leave the frame and the console unchanged, with no fault. A
  single-stepped HOME hold showed every stage working: the button task's
  scan returns the pressed mask, the debouncer posts the event after two
  equal scans, the 10-tick timer callback hands it to the app manager,
  and My Badge claims it and ignores it while unprovisioned (see "Why the
  app launcher is not reached").
- **START:** the self-test (no console lines, no app switch). Walking it
  with A presses gives, in order, the button screen (each slot lights its
  label: 0 Start, 1 A, 2 B, 3 Home, 4 DOWN, 5 LEFT, 6 RIGHT, 7 UP, 8
  SideSwitch), a screen test with lit 8x8 fiducials in all four corners,
  the accelerometer (0 mg), the lights test, NFC ("Waiting for card..."),
  the "Hardware test complete" summary, and back to the first-run screen
  (same hash). Hold HOME also exits to the first-run screen. No path leads
  to the launcher.
- **Human gate (2026-10-07):** match after a factory reset, content and
  orientation (see "Current state"). The MADCTL task was therefore
  skipped.
- **Rungs:** `boots_to_first_run_screen` and
  `first_run_screen_responds_to_start` in `boot_progress.rs`, sharing
  `run_until_stable_frame` with `boots_to_first_real_frame`; the WASM twin
  "boots factory.bin to the same first-run screen as the native
  finish-line test". The per-rung task history that used to live in
  `boot_progress.rs`'s and `rom_stub_boot.rs`'s docs is in this log; the
  test docs now say only what each rung asserts.

### Milestone 5 Task 1: the provisioning path

A trace of `factory.bin` only (no emulator change): segments disassembled
from the image, ROM names from Espressif's ROM ELF, and single-stepped
scratch runs of the blank-flash boot. Every fact, with its firmware
address, is in `milestone-5-decisions.md`'s "Trace facts"; in short:

- **The provisioned flag** (`0x3fca_92b2`, byte +2 of the 0x1B4-byte
  identity record at `0x3fca_92b0`) is set only by `hal_identity`'s
  parser (`0x4200_e64e`), when `/littlefs/identity.json` passes every
  check, and reaches RAM through the loader (`0x4200_e8a6`), which runs
  at boot and from `prov apply`. NVS `badge_upload/credential` plays no
  part in it.
- **Validation:** 1..4096 bytes of JSON; `badge_id` and `display_name`
  required non-empty; ten string fields with fixed maximum lengths (63,
  39, 23, 19 or 31 bytes); `version`, `attendee_id` and
  `provisioned_unix` accepted as numbers or decimal strings with no range
  check (`version` is never compared); `role` matched case-insensitively
  against an 11-entry table (`hacker` .. `visitor`), falling back to
  `hacker`; optional `role_color`.
- **The console does start in the emulator.** `app_main` calls
  `hal_console_start()` unconditionally after the first app launch, and
  it completes (REPL up, `app_main` returns at step 16,552,541). Its
  output, and everything printed after the USB-Serial-JTAG driver is
  installed, sits in the driver's 256-byte TX ring buffer: only the
  driver's ISR moves it to the FIFO, and that interrupt source is not
  modeled. Nothing blocks; once the buffer is full, writes are refused at
  once. Task 2's interrupt source should release it.
- **The REPL** runs in linenoise's dumb mode (its 500 ms terminal probe
  gets no answer and discards input received meanwhile); `\r` or `\n`
  ends a line. `put <path> <size>` prints `READY`, then `read()`s exactly
  `<size>` bytes from stdin in 256-byte chunks; bytes beyond the driver's
  256-byte RX ring buffer are dropped by its ISR, so the payload is sent
  after `READY`.
- **`prov apply`** loads and validates the file, resets onboarding
  (`system.onboard_build = -1`, the `tutorial=reset` in its output),
  prints `PROV OK id=..` (with the BLE address) and a summary line, and
  sets the LEDs; it neither switches apps nor restarts. My Badge's tick
  notices the new identity, redraws, and hands over to the onboarding app
  ("Setup") while onboarding is pending.
- **R-M5-3:** no signature, HMAC or hash check and no MAC/eFuse binding
  on the path; the only MAC read is the printed BLE address.
- **Local fixtures** (one per role, `local/m5-identities/`, gitignored
  until the R-M5-2 gate) were written by hand from the traced schema and
  cross-checked against the structure of a registered badge's record.

### Milestone 5 Task 2: USB-Serial-JTAG receive and interrupt source

USB-Serial-JTAG gained a receive direction (host queue, paced 64-byte OUT
packets, real INT_ST/INT_ENA/INT_CLR) and interrupt source 26; the idle
fast-forward now also stops at the next host packet
(`FirmwareBus::ticks_until_next_event`). `FirmwareRuntime::serial_input`
and `serial_pending` expose it. Console outcome (boot-probe, 20M steps):
the driver's ISR now drains the TX ring buffer, so `hal_console: console
started`, `main_task: Returned from app_main()` and the `badge> ` prompt
(after the REPL banner) all appear by 20M steps; no interrupt storm, no
fault. Side effect: `first_run_screen_responds_to_start`'s 100,000-step
press was no longer registered (button polling is busy while the console
drains); the hold is now `PRESS_HOLD_STEPS` = 1,600,000 (100 ms emulated),
ruling R-T2-1 in `milestone-5-decisions.md`; no hash changed. Console
input is no longer an open limitation.

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
to any file. Since Milestone 4 Task 5 the firmware writes it through SPIMEM1
(sector erase and page program), so e.g. `storage` holds a fresh littlefs
after boot formats it. `emulator-core/tests/flash_partition_table.rs` checks the
synthesized table byte-for-byte against the real chip's, but only when
`BADGE_FULL_DUMP` points at the local dump; without it the test skips.
A committed unit test (Task D10) also recomputes the MD5 constant from the
committed entries, so editing a partition without updating the digest
fails. It uses `emulator-core/src/md5.rs`, the crate's one MD5 (shared
with the ROM MD5 stubs since Task D13; it was a test-only copy inside
`flash.rs` before).

The firmware learns the chip's size and ID not from the chip but from the
ROM's SPI-flash legacy data (`g_rom_flashchip`), which the shortcut boot
seeds (Task D10, "ROM writable `.data`" above): `device_id` `0x464016`
(this chip's JEDEC ID `0x46 0x40 0x16`, as the bootloader byte-swaps it) and
`chip_size` 4 MiB (from factory.bin's header), which matches this model's
size, so `esp_flash` no longer warns about a size mismatch.

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

**Local-only artifacts (`local/`).** The full dump, the real serial boot
log, the esptool Python venv (`local/.venv/`) and any frames dumped with
`boot-probe --dump-frame` live in the repo-root `local/` directory, which
is gitignored (as is `*flash*dump*.bin`). Never commit them, and never
quote the boot log's identity lines anywhere committed (code, tests, docs
or commit messages). Committed tests depend only on `factory.bin`. Code
that needs the full dump reads its path from the `BADGE_FULL_DUMP` env var
and skips when it is unset (`emulator-core/tests/flash_partition_table.rs`,
and `boot-probe`, which prints only the dump's partition table). Only two
kinds of thing come from the boot log: generic ESP-IDF log lines (the
`boot_progress` tests' needles) and the facts in "Ground truth from the
physical badge" above.

# Milestone 3: design decisions, remarks, and the Milestone 4 backlog

Milestone 3 took the real-firmware emulator from a stall inside
`rtc_clk_cal()` to the firmware's **first real ST7789 frame** (the boot
splash). `docs/firmware-emulator-notes.md` holds the forensic findings,
the current state, and the stall-by-stall history. This file records what
that history does not make obvious: the design decisions taken along the
way (with what breaks if each one is wrong), a few remarks worth keeping,
and everything consciously deferred to Milestone 4.

Some decisions below override the plan text in
`docs/superpowers/plans/2026-09-22-milestone3-first-pixels.md`. The plan
and spec are kept as historical records. Where they disagree with this
file or the code, this file and the code win.

## Design decisions (Milestone 3)

### Interrupts

- **Threshold comparison is `>=`: a line whose priority is at or above
  the threshold is delivered.** The plan said `>` ("<= threshold must not
  fire"); that was wrong. ESP-IDF v5.5.3 (`interrupt_intc.h`, `vectors.S`)
  and Espressif's QEMU mask only priorities *below* the threshold, and
  with `>` the FreeRTOS yield (priority 1, threshold 1) is never
  delivered. Do not "fix" this back to `>`. If wrong: an interrupt at
  exactly the threshold is taken when it should be masked.
- **Priority 0 is never delivered**, whatever the threshold (ESP32-C3 TRM
  v1.4 section 1.5.2: priority 0 disables a line; `esprv_int_set_priority`
  takes 1 to 7). Espressif's QEMU admits a priority-0 line at threshold 0;
  this emulator follows the TRM. It matters because WFI wakes on any
  pending line, so a deliverable priority-0 line would wake or trap
  spuriously. If wrong: a priority-0 line the firmware relies on is
  silently ignored.
- **Arbitration**: among pending enabled lines at or above the threshold,
  the highest priority wins, ties to the lowest line number
  (`cpu::select_interrupt_line`).
- **Interrupt lines are sampled at the start of each step**
  (`step_with_interrupts`), not set from `tick_peripherals()` after the
  step as the plan described. An ISR that clears its source through
  `INT_CLR` only behaves correctly this way; guest-initiated timing is
  unchanged. If wrong: a one-step latency difference for
  peripheral-raised lines.
- **SYSTEM `FROM_CPU_INTR0..3` software interrupts are interrupt-matrix
  sources** handled with the rest of `pending_sources()` (FreeRTOS
  `vPortYield` depends on them), and out-of-range `intr_matrix_set` /
  priority indices are dropped rather than written past the arrays.

### ROM high-level emulation (HLE)

- **`ets_printf` is a Rust formatting HLE that writes through the bus to
  the USB-Serial-JTAG TX FIFO.** The plan said not to implement printf in
  the stub, assuming formatting happened IDF-side; the binary calls ROM
  `ets_printf` directly, and the boot-ladder tests are built from those
  lines. Unsupported specifiers echo `%` and the conversion character
  only. If wrong: an unsupported specifier misrenders a console line
  (and, for size-modifier specifiers, later arguments can misalign).
- **The console's only feed is the USB-Serial-JTAG TX FIFO** (firmware
  writes and the `ets_printf` HLE). There is no UART0 or ROM `putc` feed.
- **`qsort` runs as guest-executed RV32 code in an emulated ROM region**
  (a `jal` in the `0x4000_0434` jump-table slot to a body placed in ROM
  space no `rom*.ld` symbol uses), not as an atomic Rust stub. `qsort`
  must call the firmware's comparator, and doing that from Rust would mean
  intercepting the PC of ESP-IDF code, which this emulator never does.
  Cost: a hand-encoded machine-code body (`cpu/encode.rs`) to maintain,
  to be dropped if a real ROM image is ever loaded.
- **ROM stubs that have side effects perform them**, not `Void` no-ops:
  the interrupt-matrix / `esprv_intc_*` calls and `gpio_matrix_out`/`_in`
  write the real registers through the bus, and
  `esp_rom_newlib_init_common_mutexes` copies its two words into the ROM
  statics (addresses from the ROM ELF). Dropped state was a review finding
  each time it was tried.
- **`ets_efuse_get_spiconfig` returns 0** ("default SPI pins",
  `rom/efuse.h`); the eFuse block stays unmodeled. The badge boots from the
  default flash pads. If wrong: the flash driver picks the wrong pad
  configuration, which would surface as a flash-init stall.
- **One MD5 implementation** (`emulator-core/src/md5.rs`) is shared by the
  ROM `MD5Init`/`MD5Update`/`MD5Final` stubs and the synthetic flash's
  partition-table MD5 entry, so `load_partitions()` accepting the
  synthesized table is a cross-check rather than a coincidence (see the
  backlog: that acceptance is not yet proven end to end).
- ROM libc / libgcc helpers were added only as boot observed them, each
  with header- or GCC-documented semantics, consistent with `rom.rs`'s
  "only when observed" policy.

### Shortcut boot and memory map

- **XIP is mapped in 64 KiB pages** (Task D5) mirroring what the 2nd-stage
  bootloader programs into the MMU, instead of building a flash MMU model.
  It fixed `cpu_start`'s "Invalid app image header" check, which reads the
  header back through `SOC_DROM_LOW`. It assumes each segment's file
  offset and load address agree modulo 64 KiB, and bytes widened past the
  end of the image read `0xFF` (blank flash). If wrong / when outgrown:
  replaced by the real MMU model (Milestone 4).
- **Boot-time RAM initializers** (`boot.rs`'s `RamInitializer` list, data
  in `rom.rs`) replay exactly the effects the shortcut boot skips: the
  mask ROM's `.data` init (`rom_spiflash_legacy_data` pointing at the ROM's
  default struct), the bootloader's `esp_rom_spiflash_config_param` effect
  (chip size from `factory.bin`'s header), and the bootloader's
  `RTC_XTAL_FREQ_REG` store (whose absence printed an "invalid
  RTC_XTAL_FREQ_REG" warning the real badge never prints). Only state the
  firmware actually consumes is seeded, with values cited from the ROM ELF
  and bootloader source. If wrong: flash size/ID or crystal-frequency
  divergence, visible as warnings the real log lacks.
- **The flash MMU was ruled out of Milestone 3's scope.** The spec builds
  flash/partition support only if boot reaches it before the first frame;
  the stable splash comes first, so the MMU is Milestone 4's first task.

### Peripherals

- **ESP-IDF v5.5.3 headers are the register authority.** The plan's GDMA
  register table was not the ESP32-C3's; `gdma.rs`'s correction table uses
  `soc/gdma_reg.h`. The same rule applied everywhere a plan offset
  disagreed with a header.
- **The GDMA descriptor/buffer walk only touches internal SRAM**
  (`SOC_DRAM_LOW..HIGH`) through a RAM-only accessor. Any other address
  raises `OUT_DSCR_ERR` (bit 6, confirmed by `gdma_reg.h`) and stops the
  channel. Without this, a `next` pointer aimed at SPI2's command register
  re-triggered SPI2 and recursed until the host stack overflowed (a trap
  in WASM). Two parts are reconstruction, not header fact: the fixed upper
  address bits (`0x3FC0_0000 | OUTLINK_ADDR`), and the rule that non-SRAM
  addresses are errors. Spans that cross from one RAM region into an
  adjacent one are rejected.
- **SYSTIMER** uses one comparator model that satisfies both real call
  sequences (FreeRTOS `vSystimerSetup` and esp_timer's
  `systimer_hal_set_alarm_target`). A time jump that crosses several
  periods fires once and re-arms to the next future period.
- **WFI is a real wait** that fast-forwards SYSTIMER to the next alarm
  when nothing is pending; a waiting step still costs one step of the run
  budget, so a WFI that never wakes cannot hang the runtime.

## Remarks

- **Finish-line tests**: native `boots_to_first_real_frame`
  (`emulator-core/tests/boot_progress.rs`) and its WASM-path twin in
  `frontend/test/cpu-wasm.test.ts` ("boots factory.bin to the same first
  real frame as the native finish-line test"). Both run 6,750,000 steps in
  250,000-step chunks and pin the FNV-1a hash `0x5599c270ab0429fa`.
- **The hash is of the emulator's framebuffer orientation**, not the
  panel's: `MADCTL` is unmodeled (see backlog), so modeling it may change
  the hash without the image being wrong.
- The human comparison of the frame with the physical badge
  (2026-10-06, a match) is recorded in the notes ("Milestone 3's end
  state", under the history).
- **Idle time is compressed.** WFI fast-forward skips straight to the next
  SYSTIMER alarm, and one step is one SYSTIMER tick. The browser can
  therefore run the firmware faster (or slower) than real time. Timing
  calibration was out of Milestone 3's scope.

## Milestone 4 backlog

### Flash and boot

- **Flash MMU: the first blocker.** MMU table writes (`0x600c_5000`) are
  dropped, so the partition table read through a `spi_flash_mmap` window
  returns zeros and `load_partitions()` fails with `ESP_ERR_NOT_FOUND`.
- Real `load_partitions()` MD5 acceptance of the synthesized table is
  unproven until the MMU exists (a stand-in test drives the same call
  pattern over `EmulatedFlash`'s table).
- Replace the D5 page-granular XIP mapping with the MMU model; check the
  bootloader's extra `MMU_DROM_END_ENTRY` page (notes, history item 7).
- SPIMEM1 `CMD` bits 19..31 (dedicated `SPI_MEM_FLASH_*` commands) are
  stored but never self-cleared; flash erase/write will spin on them.
  *(Resolved in Milestone 4 Task 5.)*

### Console and logging

- Console output after the scheduler starts never arrives:
  `load_partitions()`'s `ESP_LOGE` runs but no byte reaches the console.
  The newlib stdout / VFS path (and whether it waits on a USB-Serial-JTAG
  interrupt) is untraced.
- Every log timestamp reads `I (0)` (likely the unmodeled performance
  counter CSR).

### Display and GPIO

- ST7789 `MADCTL` (`0x20`, then `0x60`) is unmodeled; the splash's
  orientation is right by coincidence of its address window.
- GPIO0 (the display D/C pin) is routed as SPI2 `FSPIWP`/`FSPIHD`
  (probably the firmware leaving the quad pins at 0); routing is stored
  only. Revisit if D/C misbehaves.

### Chip identity and timing

- eFuse block unmodeled: boot logs chip revision v0.0, the badge is v0.4.
- `CPU_FREQ_MHZ` stub returns 160; the real CPU runs at 80 MHz.
- `TICKS_PER_STEP = 1` is about 10x the real SYSTIMER/CPU ratio; no
  real-time calibration.
- Flagged ROM-stub guesses: `rom_i2c_readReg*` returning 0,
  `Cache_Get_*` returning 0 (notes, history item 5).
- Edge-type interrupts are simplified: `CPU_INT_TYPE_REG` and
  `CPU_INT_CLEAR_REG` are plain storage (every source so far is level).

### GDMA and SPI

- DMA spans that cross into an adjacent RAM region are rejected; the
  SRAM-only rule is a reconstruction (see decisions above).
- Not modeled: the RX in-link walk, CPU FIFO push/pop, the `OUT_DSCR*`
  pre-fetch registers, transfer timing. A trailing zero-length `suc_eof`
  descriptor is never visited when a transaction ends at the previous
  descriptor's end; `RESTART` with nothing to restart leaves the channel
  active.

### Tests and docs hygiene

- Condense the task-history prose in `tests/boot_progress.rs` and
  `tests/rom_stub_boot.rs`, and replace the step-exact pinned-stall
  characterization tests once the MMU moves the stall (they pin exact step
  counts and PCs and must be rewritten anyway).
- `boot_until_console_contains` checks in 250,000-step chunks, so rung
  budgets are looser than they read; several rungs have thin margins.
- The boot-ladder suite repeats the first ~570k steps in several tests;
  suite time keeps growing.
- `MAX_STUB_MEMORY_BYTES` clamps in the `strlen`/`memcmp`/`MD5Update`
  stubs truncate silently.
- Small duplications: the `memcpy`/`memset` and `memcmp`/`strncmp` stub
  arms, and the `header_with_speed_size` test helper in `image.rs` and
  `rom.rs`.
- The first-non-blank check compares pixels to `fb[0]`, and the stable
  step is quantized to the 250,000-step sampling.

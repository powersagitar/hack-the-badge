# Milestone 3 — Real-firmware boot to first pixels

Date: 2026-09-22
Status: approved design (brainstorming), pending written-spec review

> **Outcome (2026-10-06):** done. Real firmware boots to its first real
> ST7789 frame (the boot splash), pinned by `boots_to_first_real_frame`.
> The flash MMU was ruled out of scope and is Milestone 4's first
> blocker. This spec is kept as a historical record; decisions that
> departed from it and the Milestone 4 backlog are in
> `docs/milestone-3-decisions.md`.

## Goal

Boot the real dumped badge firmware (`frontend/public/firmware/factory.bin`)
in the Rust ESP32-C3 emulator (`emulator-core`) past the Milestone-2 stall in
`rtc_clk_cal()` and onward until the firmware renders its **first real frame**
into the emulated ST7789 framebuffer.

### Success criteria

1. A native `cargo test -p emulator-core` integration test boots `factory.bin`
   alone (no full flash dump required) and reaches a framebuffer containing
   more than one distinct color within a fixed step budget, then asserts a
   pinned checksum of that frame.
2. `boot_progress.rs` contains a ratchet of console-log assertions (see
   "Boot ladder") covering every milestone crossed on the way.
3. `cargo run -p emulator-core --example boot-probe -- --dump-frame <path>`
   writes the frame as a PNG for by-eye comparison against the physical
   badge. (Frame output goes under the gitignored `local/`.)
4. Bonus, not gating: the frame is visible in the browser via `bun run dev`
   in firmware mode.
5. All existing tests (`cargo test`, `bun test`, `bun run typecheck`) still
   pass.

### Out of scope (deferred to Milestone 4+)

Snake / any built-in app being playable; button-navigation verification on
real hardware; `TICKS_PER_STEP` / timing calibration beyond what first-frame
needs; I2C/accelerometer beyond what is needed not to block; WiFi/BLE/radio;
persisting emulated flash writes.

## Constraints

- **Register-faithful modeling.** Every new/changed peripheral behavior is
  modeled at the register level per ESP-IDF v5.5.3 headers/HAL, with the
  exact source cited in the module doc comment (Milestone-2 convention).
  No PC-based interception of ESP-IDF driver functions. (Mask-ROM HLE stubs
  in `src/rom.rs` remain the mechanism for ROM functions only.)
- **Full flash dump is local-only.** The full dump
  (`local/full_flash_dump.bin`, 4 MiB, read-only via esptool on 2026-09-22)
  and the real boot log (`local/boot_log.txt`) contain personal data (owner
  identity, contacts in `nvs`/`storage`). They live only under the
  gitignored `local/` directory. `.gitignore` also ignores
  `*flash*dump*.bin`. Code and tests never hard-code a dump path; anything
  that uses the dump reads the `BADGE_FULL_DUMP` env var and **skips** when
  it is unset. No committed test, fixture, doc, or commit message may quote
  identity-bearing log lines (e.g. `hal_identity`).
- Python tooling (esptool etc.) runs from the venv at `local/.venv`.
- Existing architecture rules hold: no trait-object dispatch in
  `FirmwareBus`; `emulator-core` stays wasm-bindgen-free; `emulator-wasm`
  stays logic-free.

## Ground truth from the physical badge

Captured 2026-09-22 (details in `local/`, not committed):

- Chip ESP32-C3 rev v0.4, 40 MHz crystal, 4 MiB flash, **console =
  USB-Serial/JTAG**, CPU at 80 MHz.
- Partition table: `nvs` @0x9000 (0x4000), `phy_init` @0xd000 (0x1000),
  `factory` @0x10000 (0x2a0000), `storage` @0x2b0000 (0x140000, littlefs).
- `factory.bin` is byte-identical to the dump's factory partition.
- ESP-IDF v5.5.3, LVGL-based UI.

### Boot ladder (real console lines, in order)

These generic lines are the progress-test rungs (timestamps stripped):

1. `cpu_start: Pro cpu start user code`
2. `cpu_start: cpu freq: 80000000 Hz`
3. `heap_init: Initializing. RAM available for dynamic allocation:`
4. `spi_flash: detected chip: generic`
5. `main_task: Calling app_main()`
6. `LVGL: Starting LVGL task`
7. `hal_fs: littlefs mounted` (may differ under blank flash — see Storage)
8. `hal_buttons: buttons ready`
9. `app_reg: launched My Badge`

The emulator's shortcut boot skips the ROM and 2nd-stage bootloader, so
lines before `cpu_start` are never expected.

## Design

### Part 1 — Diagnostics foundation (built before any blocker fix)

1. **Console capture peripheral** (`peripherals/usb_serial_jtag.rs`).
   Models the USB-Serial/JTAG EP1 TX FIFO registers: bytes written to the
   FIFO data register append to an in-memory console buffer; status
   registers always report "FIFO writable" / "idle" so writes never block;
   flush/`WR_DONE` accepted as no-op. UART0 TX FIFO gets the same treatment
   only if boot is observed writing there. `FirmwareRuntime` exposes
   `console_output() -> &str` (lossy UTF-8 of the buffer, bounded length).
   The WASM shim passes it through; the frontend may show it in a
   collapsible log pane (optional).
2. **No silent no-ops.** SYSTIMER and INTERRUPT_CORE0 offsets without a
   named field are recorded in `FirmwareBus::unmapped_log` like every other
   unmapped region, instead of silently dropping.
3. **`boot-probe` example** (`emulator-core/examples/boot-probe.rs`): runs
   N steps (arg), prints console output, the top-N hottest PCs over the final
   window (stall signature), the tail of `unmapped_log`, and trap/fault
   state. `--dump-frame <path>` writes the framebuffer as PNG (dev-dependency
   PNG encoder, example-only). With `BADGE_FULL_DUMP` set it may use the dump
   for reference (e.g. partition table), never as emulated flash contents.
4. **Progress ratchet** (`emulator-core/tests/boot_progress.rs`): each test
   asserts that within a step budget the console contains a given ladder
   line. Every blocker fix appends the assertion for the newest line reached.

### Part 2 — Blocker-fix protocol

Repeated for every stall:

1. Run `boot-probe`; localize via last console line, hot PCs, unmapped log.
2. Identify the registers involved; cite the exact ESP-IDF v5.5.3 header /
   HAL function whose behavior is being modeled.
3. Failing unit test for the register behavior → register-faithful
   implementation.
4. Append a `boot_progress.rs` assertion for the new furthest point.
5. Update `docs/firmware-emulator-notes.md` "Known limitations" (predicted →
   resolved; record new findings).

### Part 3 — Known blockers (seeded order; real order decided by boot-probe)

1. **TIMG0/TIMG1** (`peripherals/timg.rs`): `RTCCALICFG` — a write setting
   `RTC_CALI_START` makes `RTC_CALI_RDY` read 1 and
   `RTCCALICFG1.RTC_CALI_VALUE` read a plausible value derived from the
   requested `RTC_CALI_MAX` cycles and the modeled slow-clock / XTAL ratio
   (40 MHz XTAL vs ~136 kHz RC_SLOW default); timeout bit never set. Plus
   watchdog registers (`WDTCONFIG0..5`, `WDTWPROTECT` key, `WDTFEED`) as
   plain storage — watchdogs never fire.
2. **SYSTIMER fidelity**: both counters (UNIT0/UNIT1) and all three
   comparators (TARGET0..2) with per-comparator unit selection, period/
   oneshot mode, and `COMPx_LOAD` semantics matching the real
   `systimer_hal` call order (load pulsed in oneshot mode, then period mode
   enabled). FreeRTOS tick uses counter 1 per
   `components/freertos/port_systick.c`. Existing Milestone-2 tests updated
   where they encoded the incorrect assumption.
3. **WFI**: decoded as a real instruction. The CPU enters a waiting state
   until an enabled interrupt is pending; while waiting, the runtime
   fast-forwards SYSTIMER to its next armed alarm so idle time costs no
   emulated instruction steps (bounded by the run budget).
4. **Unknown blockers** (RTC_CNTL clock tree, eFuse reads, SPI1/flash
   controller, I2C, …): each becomes its own task via the protocol.

`TICKS_PER_STEP` calibration and re-verification of the ROM-stub guesses
(`rom_i2c_readReg*`, `Cache_Get_*`) get a task only if a stall implicates
them.

### Part 4 — Display path

1. **SPI2 register fidelity** (`peripherals/spi.rs`): `SPI_CMD_REG.UPDATE`
   self-clears on write; `SPI_USR` completion sets
   `SPI_DMA_INT_RAW.TRANS_DONE` (and raises the interrupt if enabled via
   `SPI_DMA_INT_ENA` through the interrupt matrix); command/data phases feed
   the existing ST7789 interpreter with D/C sampled from GPIO as today.
2. **GDMA** (`peripherals/gdma.rs`): minimal but real. Out-link
   (memory→peripheral) channels only. On `OUT_LINK_START` with a
   peripheral selection of SPI2, walk `dma_descriptor_t` linked lists
   (size/length/owner/eof word, buffer pointer, next pointer) from emulated
   RAM via the bus, feeding bytes into SPI2's transmit path when SPI2's user
   transaction runs with DMA enabled; update owner bits, raise
   `OUT_EOF`/`OUT_DONE` raw status and interrupts. No in-links, no other
   peripherals.

### Part 5 — Emulated flash beyond the app image

`hal_fs` mounts littlefs from `storage`; `nvs` holds identity. The real
contents are personal data and are never used as emulated contents.
Policy: the emulator models the whole 4 MiB flash address space as
`factory.bin` at 0x10000 and **blank (0xFF) everywhere else** — including
the partition table region only if the firmware reads it at runtime, in
which case a synthesized table matching the real layout (above) is placed
at 0x8000 (partition layout is not personal data). Erase/write operations
(via whichever path the firmware uses — SPI1/SPIMEM1 registers or
`esp_rom_spiflash_*` ROM stubs, determined by boot-probe) are applied to an
in-memory copy and never persisted. littlefs then either formats blank
storage or fails gracefully; identity falls back to defaults. This work is
done only if boot actually reaches it before first frame.

### Part 6 — I2C

Out of scope unless it blocks first frame. If it does, the minimal
register-faithful fix is an I2C0 controller that completes transactions
with NACK, so `hal_accel` reports the accelerometer as absent.

## Testing strategy

- Every peripheral change: unit tests in its module (register semantics).
- Cross-peripheral behavior (SPI+GDMA+ST7789, WFI+SYSTIMER+INTC):
  integration tests in `emulator-core/tests/`.
- Boot ladder: `tests/boot_progress.rs` against `factory.bin` (committed).
- Finish line: first-frame test with pinned checksum in `boot_progress.rs`.
- Anything needing the full dump: gated on `BADGE_FULL_DUMP`, skipped
  otherwise.
- After any `emulator-core`/`emulator-wasm` change: `bun run build:wasm`
  then `bun test` and `bun run typecheck`.

## Execution model

The orchestrator (main session) coordinates; subagents implement, test, and
review. Per plan task: a fresh implementer subagent (TDD) → a reviewer
subagent checking spec conformance, register-layout citations, and test
quality → orchestrator integrates and runs the full suites. Discovered
blockers are added as new tasks following the Part-2 protocol.

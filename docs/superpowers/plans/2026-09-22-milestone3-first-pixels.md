# Milestone 3 — Real-Firmware Boot to First Pixels — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Boot `frontend/public/firmware/factory.bin` in the Rust ESP32-C3 emulator past the Milestone-2 `rtc_clk_cal()` stall until the firmware renders its first real frame into the emulated ST7789 framebuffer, proven by a native `cargo test`.

**Architecture:** Diagnostics first (console capture from USB-Serial-JTAG + ROM putc, a `boot-probe` example, a console-line progress ratchet), then register-faithful peripheral fixes applied in the order boot actually hits them. Every peripheral remains a concrete named field on `FirmwareBus` with byte-granular `read_byte`/`write_byte` (no trait objects). Interrupt delivery is generalized from "SYSTIMER target0 only" to a source-indexed pending bitmap feeding the interrupt matrix.

**Tech Stack:** Rust 2021 (`emulator-core`, `emulator-wasm` via wasm-bindgen/wasm-pack), Bun + TypeScript frontend, ESP-IDF v5.5.3 headers as the register-layout source of truth.

**Spec:** `docs/superpowers/specs/2026-09-22-milestone3-first-pixels-design.md` — read it before starting any task.

## Handoff state (as of 2026-09-22)

- Worktree: `.claude/worktrees/milestone-3`, branch `worktree-milestone-3`, based on `main` @ `b2419a8`. Spec committed at `7937ba5`.
- Baseline green: `cargo test -p emulator-core` (152 unit + integration), `bun test` (99), after `bun run build:wasm`.
- Local-only, gitignored (`/local/`, `*flash*dump*.bin`), present only in the original worktree's `local/` (a new worktree must copy them over manually — never via git):
  - `local/full_flash_dump.bin` — 4 MiB full dump (personal data; never commit).
  - `local/boot_log.txt` — real serial boot log (contains owner identity; never quote `hal_identity` lines anywhere committed).
  - `local/.venv/` — Python venv with esptool (`local/.venv/bin/python3 -m esptool ...`). Use it for any Python tooling.
- Hardware facts from the badge: ESP32-C3 rev v0.4, 40 MHz XTAL, CPU 80 MHz, 4 MiB flash, **flash JEDEC ID `0x46 0x40 0x16`** (manufacturer 0x46, device 0x4016), console = USB-Serial/JTAG. Partition table: `nvs` 0x9000/0x4000 (data/nvs 01/02), `phy_init` 0xd000/0x1000 (01/01), `factory` 0x10000/0x2a0000 (00/00), `storage` 0x2b0000/0x140000 (01/0x83).
- Note: `src/rom.rs` sets `CPU_FREQ_MHZ = 160`, but the real badge logs `cpu freq: 80000000 Hz`. Treat as a flagged guess (see Task D protocol); do not change it unless a stall implicates it.

## Global Constraints

- Register-faithful: every modeled register behavior cites the exact ESP-IDF v5.5.3 header / HAL / LL source file it came from, in the module doc comment (Milestone-2 convention). Fetch headers from `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`; never guess an offset without citing.
- No PC-based interception of ESP-IDF (non-ROM) functions. ROM functions only, via `src/rom.rs` + `cpu/rom_stubs.rs`, with addresses from `components/esp_rom/esp32c3/ld/esp32c3.rom*.ld`.
- `FirmwareBus` dispatch stays an ordered sequence of concrete named fields; no `dyn` peripheral dispatch. `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays logic-free.
- Unmapped data access never panics (reads 0, writes drop, logged); unmapped instruction fetch always traps.
- The full dump / real boot log are never committed, quoted in commits, or required by committed tests. Code touching them reads env var `BADGE_FULL_DUMP` and skips when unset.
- Emulated flash beyond the app image is blank (`0xFF`) plus (only if needed) a synthesized partition table; writes are in-memory only.
- Python via `local/.venv` only. JS/TS via Bun only.
- After any change under `emulator-core/` or `emulator-wasm/`: `bun run build:wasm`, then `bun test` and `bun run typecheck` must pass, in addition to `cargo test -p emulator-core` and `cargo test --workspace`.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Multi-byte register writes arrive as 4 ascending byte writes** (`FirmwareBus::write32` → `write_byte(addr+0..3)`). Any write-triggered side effect must fire on the byte containing the trigger bit and must read other fields from the *same* write's already-applied bytes — tests in every new peripheral task write whole words via a byte-splitting helper and assert the side effect fires exactly once.
2. **Level vs sticky interrupts:** after a peripheral's raw status is cleared by the ISR, the CPU must not take the interrupt again. Task 4 replaces sticky `raise_interrupt` accumulation in the boot loop with per-step level recomputation, and tests it.
3. **Interrupt priority/threshold masking:** ESP-IDF's RISC-V port masks interrupts in critical sections via `CPU_INT_THRESH_REG`, not `mstatus.MIE`. A line whose priority < threshold must not fire; priority == threshold is delivered (Task 4 test). (Corrected in Task D11: ESP-IDF v5.5.3 `components/riscv/include/esp_private/interrupt_intc.h:24` and `components/riscv/vectors.S:424-432` mask a line iff its priority is below the threshold.)
4. **WFI with nothing armed** must not hang the host: `run(budget)` always returns after at most `budget` steps even if the CPU waits forever (Task 6 test).
5. **Console buffer growth:** a firmware stuck in a print loop must not grow memory unboundedly — console buffer is capped (Task 1 test).

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `emulator-core/src/peripherals/usb_serial_jtag.rs` | Create | USB-Serial-JTAG EP1 TX FIFO → console byte sink |
| `emulator-core/src/peripherals/console.rs` | Create | Capped console buffer shared by USB-Serial-JTAG / UART0 / ROM putc |
| `emulator-core/src/peripherals/timg.rs` | Create | TIMG0/TIMG1: RTC slow-clock calibration + inert watchdog/timer storage |
| `emulator-core/src/peripherals/systimer.rs` | Modify | 2 counters, 3 comparators, 3 interrupt sources, fast-forward |
| `emulator-core/src/peripherals/intc.rs` | Modify | Source-indexed MAP regs, priority/threshold, line-mask output |
| `emulator-core/src/peripherals/spi.rs` | Modify | `UPDATE` self-clear, `DMA_INT_*` + `TRANS_DONE`, DMA TX path |
| `emulator-core/src/peripherals/gdma.rs` | Create | GDMA out-link channels walking `dma_descriptor_t` lists |
| `emulator-core/src/peripherals/mod.rs` | Modify | Register new modules |
| `emulator-core/src/mem/soc.rs` | Modify | New address ranges + interrupt source numbers |
| `emulator-core/src/mem/bus.rs` | Modify | Route new ranges; `pending_sources()`; log unhandled SYSTIMER/INTC offsets |
| `emulator-core/src/cpu/decode.rs`, `execute.rs`, `mod.rs` | Modify | WFI; level-pending interrupt setter |
| `emulator-core/src/boot.rs` | Modify | Level interrupt delivery in `step_with_interrupts` |
| `emulator-core/src/runtime.rs` | Modify | `console_output()`, WFI fast-forward in `run` |
| `emulator-core/src/rom.rs` | Modify | ROM putc stubs → console (as observed) |
| `emulator-core/examples/boot-probe.rs` | Create | Stall diagnosis CLI + PNG frame dump |
| `emulator-core/tests/boot_progress.rs` | Create | Console-line ratchet + first-frame finish-line test |
| `emulator-core/Cargo.toml` | Modify | `[dev-dependencies] png` (example-only) |
| `emulator-wasm/src/lib.rs` | Modify | `consoleOutput` getter passthrough |
| `docs/firmware-emulator-notes.md`, `CLAUDE.md` | Modify | Findings, resolved limitations, new modules |

Tasks 3–10 are listed in the *predicted* order. The orchestrator runs `boot-probe` after each task and picks the next task by what boot actually hits; unknown blockers use the **Task D template**.

---

### Task 1: Console capture (USB-Serial-JTAG + capped console buffer + runtime/WASM passthrough)

**Files:**
- Create: `emulator-core/src/peripherals/console.rs`, `emulator-core/src/peripherals/usb_serial_jtag.rs`
- Modify: `emulator-core/src/peripherals/mod.rs`, `emulator-core/src/mem/soc.rs`, `emulator-core/src/mem/bus.rs`, `emulator-core/src/runtime.rs`, `emulator-wasm/src/lib.rs`

**Interfaces:**
- Produces:
  - `peripherals::console::Console { pub fn new() -> Self; pub fn push(&mut self, b: u8); pub fn bytes(&self) -> Vec<u8>; pub fn text(&self) -> String /* lossy UTF-8 */ }`, `pub const CONSOLE_CAPACITY: usize = 256 * 1024;` (when full, drop the oldest bytes).
  - `peripherals::usb_serial_jtag::UsbSerialJtag { pub fn new() -> Self; pub fn read_byte(&mut self, offset: u32) -> u8; pub fn write_byte(&mut self, offset: u32, val: u8, console: &mut Console) }`
  - `mem::soc::USB_SERIAL_JTAG_RANGE: Range<u32> = 0x6004_3000..0x6004_4000`
  - `FirmwareBus.console: Console` and `FirmwareBus.usb_serial_jtag: UsbSerialJtag` (pub fields)
  - `FirmwareRuntime::console_output(&self) -> String`
  - WASM: `FirmwareEmulator::console_output(&self) -> String` exported as `consoleOutput` (method, not getter).

Register facts to verify against `components/soc/esp32c3/register/soc/usb_serial_jtag_reg.h` (v5.5.3) and cite: `USB_SERIAL_JTAG_EP1_REG` @ +0x00 (`RDWR_BYTE` bits 7:0 — write pushes a TX byte; read pops RX, return 0: no host input modeled), `USB_SERIAL_JTAG_EP1_CONF_REG` @ +0x04 (bit0 `WR_DONE` write-trigger flush → no-op; bit1 `SERIAL_IN_EP_DATA_FREE` → always reads 1; bit2 `SERIAL_OUT_EP_DATA_AVAIL` → always reads 0). All other offsets: plain read/write word storage so drivers configuring interrupts don't get lost, except `INT_RAW` — make `SERIAL_IN_EMPTY` (verify bit position in header) read 1 so interrupt-driven TX drivers see "FIFO empty". If the header disagrees with these offsets, the header wins; update the tests accordingly and note it in the module doc.

- [ ] **Step 1: Write failing tests** in `console.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushes_bytes_and_renders_lossy_text() {
        let mut c = Console::new();
        for b in b"I (5) boot: hi\n" { c.push(*b); }
        assert_eq!(c.text(), "I (5) boot: hi\n");
    }

    #[test]
    fn caps_at_capacity_dropping_oldest() {
        let mut c = Console::new();
        for i in 0..(CONSOLE_CAPACITY + 10) { c.push((i % 251) as u8); }
        assert_eq!(c.bytes().len(), CONSOLE_CAPACITY);
        assert_eq!(c.bytes()[0], (10 % 251) as u8, "oldest 10 bytes dropped");
    }
}
```

and in `usb_serial_jtag.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::console::Console;

    fn write_word(u: &mut UsbSerialJtag, c: &mut Console, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() { u.write_byte(off + i as u32, *b, c); }
    }
    fn read_word(u: &mut UsbSerialJtag, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| u.read_byte(off + i)))
    }

    #[test]
    fn word_write_to_ep1_pushes_exactly_one_byte() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        write_word(&mut u, &mut c, EP1_REG, 0x41);
        write_word(&mut u, &mut c, EP1_REG, 0x42);
        assert_eq!(c.bytes(), b"AB");
    }

    #[test]
    fn byte_write_to_ep1_pushes_that_byte() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        u.write_byte(EP1_REG, b'x', &mut c);
        assert_eq!(c.bytes(), b"x");
    }

    #[test]
    fn ep1_conf_always_reports_tx_free_and_no_rx() {
        let mut u = UsbSerialJtag::new();
        let conf = read_word(&mut u, EP1_CONF_REG);
        assert_ne!(conf & SERIAL_IN_EP_DATA_FREE, 0);
        assert_eq!(conf & SERIAL_OUT_EP_DATA_AVAIL, 0);
    }

    #[test]
    fn wr_done_write_is_accepted_and_emits_nothing() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        write_word(&mut u, &mut c, EP1_CONF_REG, WR_DONE);
        assert!(c.bytes().is_empty());
    }

    #[test]
    fn other_registers_are_plain_storage() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        write_word(&mut u, &mut c, 0x10, 0xDEAD_BEEF);
        assert_eq!(read_word(&mut u, 0x10), 0xDEAD_BEEF);
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p emulator-core console usb_serial_jtag` → compile errors (types missing).

- [ ] **Step 3: Implement.** `console.rs`:

```rust
//! Capped byte sink for everything the firmware prints (USB-Serial-JTAG TX
//! FIFO, UART0 TX FIFO, ROM putc stubs). Diagnostic output, not device state:
//! capped so a firmware stuck in a print loop can't grow host memory
//! without bound.
use std::collections::VecDeque;

pub const CONSOLE_CAPACITY: usize = 256 * 1024;

#[derive(Default)]
pub struct Console { buf: VecDeque<u8> }

impl Console {
    pub fn new() -> Self { Self::default() }
    pub fn push(&mut self, b: u8) {
        if self.buf.len() >= CONSOLE_CAPACITY { self.buf.pop_front(); }
        self.buf.push_back(b);
    }
    /// Owned copy (the ring may be non-contiguous); diagnostic use only.
    pub fn bytes(&self) -> Vec<u8> { self.buf.iter().copied().collect() }
    pub fn text(&self) -> String { String::from_utf8_lossy(&self.bytes()).into_owned() }
}
```

(Interface note: `bytes()` returns `Vec<u8>`, not `&[u8]` — update the Interfaces line above accordingly. Tests compare with `assert_eq!(c.bytes(), b"AB")`, which works for `Vec<u8>` vs `&[u8; N]`.)

`usb_serial_jtag.rs`: constants `EP1_REG = 0x00`, `EP1_CONF_REG = 0x04`, `WR_DONE = 1<<0`, `SERIAL_IN_EP_DATA_FREE = 1<<1`, `SERIAL_OUT_EP_DATA_AVAIL = 1<<2`; `regs: HashMap<u32, u32>` storage for other offsets; `write_byte` on `EP1_REG` byte index 0 → `console.push(val)`, other byte indices of EP1 ignored; `EP1_CONF_REG` writes ignored; reads of `EP1_CONF_REG` return `SERIAL_IN_EP_DATA_FREE`; reads of `EP1_REG` return 0. Module doc cites the header.

Bus wiring in `bus.rs`: add fields, construct in `from_segments`, add a tier after SPI2 in both `read_byte`/`write_byte`:

```rust
if USB_SERIAL_JTAG_RANGE.contains(&addr) {
    return self.usb_serial_jtag.read_byte(addr - USB_SERIAL_JTAG_RANGE.start);
}
```
```rust
if USB_SERIAL_JTAG_RANGE.contains(&addr) {
    self.usb_serial_jtag.write_byte(addr - USB_SERIAL_JTAG_RANGE.start, val, &mut self.console);
    return;
}
```
Update the module-doc tier list. Add `soc.rs` range + disjointness test mirroring the existing ones.

`runtime.rs`: `pub fn console_output(&self) -> String { self.bus.console.text() }`. WASM `lib.rs`: `#[wasm_bindgen(js_name = consoleOutput)] pub fn console_output(&self) -> String { self.inner.console_output() }` (match the existing field name for the wrapped runtime).

- [ ] **Step 4: Add a bus-level test** in `bus.rs` tests: `bus.write32(0x6004_3000, b'Z' as u32); assert_eq!(bus.console.bytes(), b"Z");`
- [ ] **Step 5: Run** `cargo test -p emulator-core` → all pass. `bun run build:wasm && bun test && bun run typecheck` → pass.
- [ ] **Step 6: Commit** `feat(emulator-core): capture firmware console via USB-Serial-JTAG TX FIFO`

---

### Task 2: Diagnostics — log unhandled SYSTIMER/INTC offsets + `boot-probe` example

**Files:**
- Modify: `emulator-core/src/peripherals/systimer.rs`, `intc.rs`, `emulator-core/src/mem/bus.rs`, `emulator-core/Cargo.toml`
- Create: `emulator-core/examples/boot-probe.rs`

**Interfaces:**
- Consumes: `FirmwareRuntime::console_output()` (Task 1).
- Produces:
  - `SysTimer::handles(offset: u32) -> bool` and `InterruptController::handles(offset: u32) -> bool` (pure functions of the word offset: `true` iff the word offset is a named register the module models). Bus calls `record_unmapped(addr, is_write)` *in addition to* dispatching when `handles` is false.
  - `FirmwareRuntime::run_traced(&mut self, budget: u32, pc_hist: &mut HashMap<u32, u64>) -> RunSummary` — same as `run` but counts each executed `pc_before`. (Keeps `run` allocation-free for WASM.)
  - `examples/boot-probe.rs` CLI: `cargo run -p emulator-core --release --example boot-probe -- [--steps N] [--window W] [--dump-frame PATH]`.

- [ ] **Step 1: Failing tests.** In `bus.rs` tests:

```rust
#[test]
fn unhandled_systimer_and_intc_offsets_are_logged() {
    let mut bus = bus_with(vec![]);
    bus.write32(0x6002_3000 + 0x0FC, 1); // not a modeled SYSTIMER register
    bus.read32(0x600c_2000 + 0x800);     // not a modeled INTC register
    let log: Vec<_> = bus.unmapped_log().iter().map(|a| (a.addr & !3, a.is_write)).collect();
    assert!(log.contains(&(0x6002_30FC, true)));
    assert!(log.contains(&(0x600c_2800, false)));
}

#[test]
fn handled_systimer_offsets_are_not_logged() {
    let mut bus = bus_with(vec![]);
    bus.read32(0x6002_3000); // CONF_REG
    assert!(bus.unmapped_log().is_empty());
}
```

- [ ] **Step 2: Run** → FAIL (no log entries).
- [ ] **Step 3: Implement** `handles()` in both modules (match on the same constants their `read_byte` matches; for INTC include the MAP region `< MAP_REGION_END` and the PRI range). In bus: `if !SysTimer::handles(off & !3) { self.record_unmapped(addr, false); }` before returning the systimer value (same for writes and INTC). Update the bus module doc's catch-all paragraph (remove the "silently no-op'd" sentence).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: `run_traced`** in `runtime.rs` (share the body with `run` via a private generic helper taking `Option<&mut HashMap<u32,u64>>`). Unit test: synthetic self-branch image, `run_traced(50, &mut h)` → `h[&0x4200_0000] == 50`.
- [ ] **Step 6: boot-probe example.** Add to `emulator-core/Cargo.toml`:

```toml
[dev-dependencies]
png = "0.17"
```

`examples/boot-probe.rs` behavior (write it fully; no external arg-parsing crate):
1. Parse `--steps N` (default 20_000_000), `--window W` (default 200_000), `--dump-frame PATH` (optional).
2. Load `../frontend/public/firmware/factory.bin` relative to `CARGO_MANIFEST_DIR`; `FirmwareRuntime::from_image`.
3. `run(N - W)` in chunks of 1_000_000, then `run_traced(W, &mut hist)`.
4. Print sections: `== console ==` (full `console_output()`), `== summary ==` (total steps, pc, traps, rom_stub_calls, last_instruction_fault), `== hot PCs (last W steps) ==` top 20 by count as `0x%08x  count`, `== unmapped (tail) ==` last 64 entries of `bus().unmapped_log()` deduplicated by `(addr & !3, is_write)` with counts, `== framebuffer ==` number of distinct pixel values.
5. If `--dump-frame`, write the RGB565 framebuffer (320×240) as 8-bit RGB PNG (expand 5/6/5 bits: `r = (px>>11)&31; r8 = (r<<3)|(r>>2)`, etc.). Refuse to write outside the repo-relative `local/` directory unless the path is absolute (print a warning either way that frames may show personal data).
6. If env `BADGE_FULL_DUMP` is set and readable, print its partition table (parse 32-byte entries at 0x8000 while magic `0xAA 0x50`) under `== full dump partition table ==`. Never print any other dump contents.

- [ ] **Step 7: Run it** `cargo run -p emulator-core --release --example boot-probe -- --steps 3000000` and paste the summary + hot PCs into the task report (expected today: empty/near-empty console, hot PCs inside `rtc_clk_cal`, unmapped log showing TIMG0 `0x6001_F068` reads).
- [ ] **Step 8: Full suite** (`cargo test --workspace`, `bun run build:wasm && bun test && bun run typecheck`) → PASS.
- [ ] **Step 9: Commit** `feat(emulator-core): boot-probe diagnostics; log unhandled SYSTIMER/INTC offsets`

---

### Task 3: TIMG0/TIMG1 — RTC slow-clock calibration + inert watchdogs; first progress ratchet

**Files:**
- Create: `emulator-core/src/peripherals/timg.rs`, `emulator-core/tests/boot_progress.rs`
- Modify: `peripherals/mod.rs`, `mem/soc.rs`, `mem/bus.rs`, `docs/firmware-emulator-notes.md`

**Interfaces:**
- Produces:
  - `mem::soc::TIMG0_RANGE = 0x6001_F000..0x6002_0000`, `TIMG1_RANGE = 0x6002_0000..0x6002_1000`
  - `peripherals::timg::Timg { pub fn new() -> Self; pub fn read_byte(&mut self, offset: u32) -> u8; pub fn write_byte(&mut self, offset: u32, val: u8) }`; `FirmwareBus.timg0`, `FirmwareBus.timg1`.
  - `tests/boot_progress.rs` helper `fn boot_until_console_contains(needle: &str, max_steps: u64) -> (FirmwareRuntime, bool)` used by all later ratchet tests.

Register facts (verify + cite `components/soc/esp32c3/register/soc/timer_group_reg.h` and the calibration algorithm in `components/esp_hw_support/port/esp32c3/rtc_time.c`, function `rtc_clk_cal_internal`):
- `RTCCALICFG_REG` @ +0x68: bit 12 `RTC_CALI_START_CYCLING`, bits 14:13 `RTC_CALI_CLK_SEL` (0 = RTC_MUX/RC_SLOW, 1 = RC_FAST_D256, 2 = XTAL32K), bit 15 `RTC_CALI_RDY` (read-only), bits 30:16 `RTC_CALI_MAX`, bit 31 `RTC_CALI_START`.
- `RTCCALICFG1_REG` @ +0x6C: bits 31:7 `RTC_CALI_VALUE` (read-only).
- `RTCCALICFG2_REG` @ +0x80: bit 0 `RTC_CALI_TIMEOUT` (read-only, always 0 here).
- Watchdog `WDTCONFIG0..5` @ +0x48..+0x5C, `WDTFEED` @ +0x60, `WDTWPROTECT` @ +0x64: plain storage; the watchdog never fires.

Calibration model: when a write sets `RTC_CALI_START` (bit 31, i.e. byte index 3, bit 7) the calibration completes instantly: `RDY := 1`, `VALUE := MAX * 40_000_000 / slow_hz(CLK_SEL)` with `slow_hz = [136_000, 17_500_000 / 256, 32_768, 136_000][clk_sel]` (constants: `SOC_CLK_RC_SLOW_FREQ_APPROX`, `SOC_CLK_RC_FAST_FREQ_APPROX / 256`, `SOC_CLK_XTAL32K_FREQ_APPROX` — cite `soc/esp32c3/include/soc/soc.h` or `clk_tree_defs.h`). A write with START clear sets `RDY := 0`. Writes never overwrite `RDY` directly.

- [ ] **Step 1: Failing unit tests** in `timg.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn w(t: &mut Timg, off: u32, v: u32) { for (i, b) in v.to_le_bytes().iter().enumerate() { t.write_byte(off + i as u32, *b); } }
    fn r(t: &mut Timg, off: u32) -> u32 { u32::from_le_bytes([0, 1, 2, 3].map(|i| t.read_byte(off + i))) }

    fn cfg(clk_sel: u32, max: u32, start: bool) -> u32 {
        (clk_sel << 13) | (max << 16) | if start { 1 << 31 } else { 0 }
    }

    #[test]
    fn calibration_is_not_ready_before_start() {
        let mut t = Timg::new();
        assert_eq!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
    }

    #[test]
    fn start_completes_rc_slow_calibration_with_xtal_ratio() {
        let mut t = Timg::new();
        w(&mut t, RTCCALICFG_REG, cfg(0, 1024, false));
        w(&mut t, RTCCALICFG_REG, cfg(0, 1024, true));
        assert_ne!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
        let value = r(&mut t, RTCCALICFG1_REG) >> 7;
        assert_eq!(value, 1024 * 40_000_000 / 136_000);
        assert_eq!(r(&mut t, RTCCALICFG2_REG) & 1, 0, "never times out");
    }

    #[test]
    fn xtal32k_calibration_uses_32768_hz() {
        let mut t = Timg::new();
        w(&mut t, RTCCALICFG_REG, cfg(2, 100, true));
        assert_eq!(r(&mut t, RTCCALICFG1_REG) >> 7, 100 * 40_000_000 / 32_768);
    }

    #[test]
    fn clearing_start_clears_ready_and_writes_cannot_forge_ready() {
        let mut t = Timg::new();
        w(&mut t, RTCCALICFG_REG, cfg(0, 10, true));
        w(&mut t, RTCCALICFG_REG, cfg(0, 10, false));
        assert_eq!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
        w(&mut t, RTCCALICFG_REG, RTC_CALI_RDY);
        assert_eq!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
    }

    #[test]
    fn watchdog_registers_are_plain_storage() {
        let mut t = Timg::new();
        w(&mut t, WDTWPROTECT_REG, 0x50D8_3AA1);
        w(&mut t, WDTCONFIG0_REG, 0);
        assert_eq!(r(&mut t, WDTWPROTECT_REG), 0x50D8_3AA1);
        assert_eq!(r(&mut t, WDTCONFIG0_REG), 0);
    }
}
```

- [ ] **Step 2: Run** → FAIL (missing module).
- [ ] **Step 3: Implement** `Timg` with `regs: [u32; 64]` (word storage for +0x00..+0x100; offsets ≥ 0x100 read 0 / drop), the calibration side effect applied when byte index 3 of `RTCCALICFG_REG` is written (after `set_byte`, so MAX's high bits from the same write are present), RDY preserved across byte writes to byte 1, and `RTCCALICFG1`/`RTCCALICFG2` reads served from computed state. Wire into bus (two fields, two tiers, doc tier list), soc ranges + disjointness test.
- [ ] **Step 4: Run unit tests** → PASS.
- [ ] **Step 5: Progress ratchet file** `tests/boot_progress.rs`:

```rust
//! Boot-progress ratchet: each test asserts the real firmware's console
//! reaches a known line (from the real badge's boot log) within a step
//! budget. Lines are generic ESP-IDF log lines only — never identity data.
use emulator_core::runtime::FirmwareRuntime;
use std::path::PathBuf;

fn factory() -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/public/firmware/factory.bin"))
        .expect("factory.bin")
}

pub fn boot_until_console_contains(needle: &str, max_steps: u64) -> (FirmwareRuntime, bool) {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    const CHUNK: u32 = 250_000;
    while rt.total_steps() < max_steps {
        rt.run(CHUNK);
        if rt.console_output().contains(needle) { return (rt, true); }
    }
    (rt, false)
}

fn assert_reaches(needle: &str, max_steps: u64) {
    let (rt, ok) = boot_until_console_contains(needle, max_steps);
    assert!(ok, "never printed {needle:?} within {max_steps} steps; pc=0x{:08x}\nconsole:\n{}",
        rt.pc(), rt.console_output());
}
```

Then run `boot-probe` to see what the console now shows past TIMG. Add one `#[test]` per newly reached ladder rung (spec "Boot ladder"), with `max_steps` = observed steps × 2 rounded up to a multiple of 1_000_000. If *no* console line is reached yet, add instead a test asserting the hot-PC set no longer sits in the pre-fix `rtc_clk_cal` loop (record the pre-fix hot PC range from Task 2's report) and let the next task add the first console rung.

- [ ] **Step 6: Run** `cargo test -p emulator-core --release --test boot_progress` → PASS. (Use `--release` for boot_progress locally; also confirm it passes in debug within a reasonable time — if debug is > 60 s, mark the heavy tests `#[cfg_attr(debug_assertions, ignore)]` and document running them with `--release`.)
- [ ] **Step 7: Update** `docs/firmware-emulator-notes.md` Known limitations: item 1 → resolved; record where boot now stalls (from boot-probe).
- [ ] **Step 8: Full suite + commit** `feat(emulator-core): model TIMG RTC calibration; boot progress ratchet`

---

### Task 4: Interrupt matrix generalization — source-indexed, priority/threshold-gated, level-delivered

**Files:**
- Modify: `emulator-core/src/peripherals/intc.rs`, `emulator-core/src/mem/bus.rs`, `emulator-core/src/mem/soc.rs`, `emulator-core/src/boot.rs`, `emulator-core/src/cpu/mod.rs`, existing tests in `emulator-core/tests/interrupt_integration.rs`

**Interfaces:**
- Produces:
  - `mem::soc` interrupt source numbers (verify + cite `components/soc/esp32c3/include/soc/interrupts.h`, enum `periph_interrupt_t`): `pub const SRC_SPI2: u32 = 19; pub const SRC_SYSTIMER_TARGET0: u32 = 37; SRC_SYSTIMER_TARGET1 = 38; SRC_SYSTIMER_TARGET2 = 39; SRC_DMA_CH0 = 44; SRC_DMA_CH1 = 45; SRC_DMA_CH2 = 46;` (37 is already confirmed: `SYSTIMER_TARGET0_INT_MAP_REG == 0x94 == 37*4`).
  - `InterruptController::poll(&self, pending_sources: u64) -> u32` — returns a **mask of CPU lines** that are asserted, enabled (`CPU_INT_ENABLE`), and whose priority (`CPU_INT_PRI_n`) is at least `CPU_INT_THRESH` (a line is masked iff priority < threshold). A source maps to line `MAP[src] & 0x1F`; line 0 means "not routed" (ESP-IDF never uses line 0).
    - Note (Task D11 correction): an earlier draft said "strictly greater than". The accepted rule follows ESP-IDF v5.5.3 `components/riscv/include/esp_private/interrupt_intc.h:24` and `components/riscv/vectors.S:424-432`. The `priority_at_or_below_threshold_is_masked` sketch below predates the ruling; the landed test admits priority == threshold.
  - `InterruptController::eip_status(&self, pending_sources: u64) -> u32` — mask of lines with any routed pending source, before enable/priority gating.
  - `FirmwareBus::pending_sources(&self) -> u64` — OR of each peripheral's asserted sources (for now only SYSTIMER target0 via the existing `target0_pending()`; later tasks extend it).
  - `FirmwareBus::tick_peripherals(&mut self) -> u32` (line mask; was `Option<u32>`). (Task D11: now returns `()`; `step_with_interrupts` samples `asserted_lines()` at the start of each step instead.)
  - `Cpu::set_pending_interrupts(&mut self, mask: u32)` — replaces (not ORs) the pending line set. `step_with_interrupts` calls it with `tick_peripherals()` each step. `raise_interrupt` stays for unit tests.
- Remove the special-case `systimer_target0_map` field: MAP registers become a uniform `[u32; 64]` indexed by `offset / 4` for `offset < MAP_REGION_END` (keep the `SYSTIMER_TARGET0_INT_MAP_REG` constant for tests).

- [ ] **Step 1: Failing tests** in `intc.rs`:

```rust
#[test]
fn routed_enabled_source_above_threshold_asserts_its_line() {
    let mut ic = InterruptController::new();
    write_word(&mut ic, 19 * 4, 5);                    // SPI2 -> line 5
    write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 5);
    write_word(&mut ic, CPU_INT_PRI_BASE_REG + 5 * 4, 3);
    write_word(&mut ic, CPU_INT_THRESH_REG, 1);
    assert_eq!(ic.poll(1u64 << 19), 1 << 5);
    assert_eq!(ic.poll(0), 0);
}

#[test]
fn priority_at_or_below_threshold_is_masked() {
    let mut ic = InterruptController::new();
    write_word(&mut ic, 37 * 4, 7);
    write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 7);
    write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 2);
    write_word(&mut ic, CPU_INT_THRESH_REG, 2);
    assert_eq!(ic.poll(1u64 << 37), 0, "pri == thresh must not fire");
    assert_eq!(ic.eip_status(1u64 << 37), 1 << 7, "still visible as pending");
}

#[test]
fn two_sources_on_two_lines_both_assert() {
    let mut ic = InterruptController::new();
    write_word(&mut ic, 37 * 4, 7);
    write_word(&mut ic, 44 * 4, 9);
    write_word(&mut ic, CPU_INT_ENABLE_REG, (1 << 7) | (1 << 9));
    write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 1);
    write_word(&mut ic, CPU_INT_PRI_BASE_REG + 9 * 4, 1);
    assert_eq!(ic.poll((1u64 << 37) | (1u64 << 44)), (1 << 7) | (1 << 9));
}

#[test]
fn unrouted_source_line_zero_never_fires() {
    let mut ic = InterruptController::new();
    write_word(&mut ic, CPU_INT_ENABLE_REG, 1);
    write_word(&mut ic, CPU_INT_PRI_BASE_REG, 7);
    assert_eq!(ic.poll(1u64 << 19), 0);
}
```

In `cpu/mod.rs` tests:

```rust
#[test]
fn set_pending_interrupts_replaces_rather_than_accumulates() {
    let mut cpu = Cpu::new();
    cpu.set_pending_interrupts(1 << 5);
    cpu.set_pending_interrupts(0);
    // With MIE set, a step must NOT take a trap: the source was de-asserted.
    cpu.csr.mstatus |= mstatus_bits::MIE;
    let mut bus = TestBus::with_program(&[0x0000_0013]); // nop
    let info = cpu.step(&mut bus);
    assert!(!info.trap_taken);
}
```

In `tests/interrupt_integration.rs`, add a level-delivery test: arm SYSTIMER target0 → routed line with priority 1 > thresh 0; run until the trap is taken; in "ISR" (test code) clear `INT_CLR`; step again with MIE re-enabled; assert no second trap before the next alarm.

- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** Existing Milestone-2 tests that relied on priority defaults: the real reset value of `CPU_INT_PRI_n` is 0 and `THRESH` is 0, which would mask everything under the new rule — update those tests to program a priority ≥ 1 (this is what ESP-IDF does via `esprv_intc_int_set_priority`), and note it in the intc module doc citing `components/riscv/interrupt_intc.c` / `esp_cpu_intr_set_priority`.
- [ ] **Step 4: Run** `cargo test -p emulator-core` → PASS; boot_progress still green.
- [ ] **Step 5: Full suite + commit** `feat(emulator-core): source-indexed interrupt matrix with priority/threshold and level delivery`

---

### Task 5: SYSTIMER fidelity — 2 counters, 3 comparators, real HAL call sequences

**Files:**
- Modify: `emulator-core/src/peripherals/systimer.rs`, `emulator-core/src/mem/bus.rs` (`pending_sources`), `emulator-core/tests/interrupt_integration.rs`

**Interfaces:**
- Consumes: `soc::SRC_SYSTIMER_TARGET{0,1,2}` (Task 4).
- Produces:
  - `SysTimer::pending_sources(&self) -> u64` (bits 37/38/39 for targets 0/1/2 when `INT_RAW & INT_ENA` bit n set). `target0_pending()` kept as a thin wrapper for existing callers or removed with callers updated.
  - `SysTimer::counter(unit: usize) -> u64`.
  - `SysTimer::advance_by(&mut self, ticks: u64)` (with `advance()` = `advance_by(TICKS_PER_STEP)`), firing any comparator crossed within the jump (period comparators re-arm correctly even if several periods elapse — fire once, re-arm to the next future multiple).
  - `SysTimer::ticks_until_next_alarm(&self) -> Option<u64>` — smallest positive tick distance to any enabled (`TARGETn_WORK_EN` in CONF and its `INT_ENA` bit) armed comparator on a working unit; `None` if none.

Register facts: verify + cite `components/soc/esp32c3/register/soc/systimer_reg.h` (offsets below match the existing module's), `components/hal/esp32c3/include/hal/systimer_ll.h`, `components/hal/systimer_hal.c`, `components/freertos/port_systick.c`, `components/esp_timer/src/esp_timer_impl_systimer.c`.
- `CONF` @0x00: bit31 `CLK_EN`, bit30 `UNIT0_WORK_EN`, bit29 `UNIT1_WORK_EN`, bits 24/23/22 `TARGET0/1/2_WORK_EN`. Reset value: verify (existing code assumes UNIT0 work-enabled at reset; check header's reset value — ESP-IDF's `systimer_hal_init` sets `CLK_EN`).
- `UNIT0_OP` @0x04, `UNIT1_OP` @0x08 (bit30 `UPDATE` write, bit29 `VALUE_VALID` read).
- `UNIT0_LOAD_HI/LO` @0x0C/0x10, `UNIT1_LOAD_HI/LO` @0x14/0x18, `UNIT0_LOAD` @0x5C, `UNIT1_LOAD` @0x60 (bit0 pulse: counter := LOAD value).
- `TARGETn_HI/LO` @0x1C+8n / 0x20+8n; `TARGETn_CONF` @0x34+4n (bits 25:0 period, bit30 period mode, bit31 unit select).
- `UNIT0_VALUE_HI/LO` @0x40/0x44, `UNIT1_VALUE_HI/LO` @0x48/0x4C.
- `COMPn_LOAD` @0x50+4n (bit0 pulse).
- `INT_ENA/RAW/CLR/ST` @0x64/0x68/0x6C/0x70, bits 0..2 = targets 0..2.

Verified facts (fetched from v5.5.3 while writing this plan): `soc_caps.h` for C3 defines `SOC_SYSTIMER_COUNTER_NUM 2`, `SOC_SYSTIMER_ALARM_NUM 3`, `SOC_SYSTIMER_FIXED_DIVIDER 1` (16 MHz), **`SOC_SYSTIMER_INT_LEVEL 1`** (level interrupt — consistent with Task 4's level delivery), **`SOC_SYSTIMER_ALARM_MISS_COMPENSATE 1`** (an alarm whose target is already ≤ the counter fires immediately). `systimer_ll_enable_alarm(id, en)` toggles `CONF` bit `24 - id`; `systimer_ll_apply_alarm_value(id)` writes 1 to `COMP{id}_LOAD`; `systimer_ll_connect_alarm_counter` writes `TARGET{id}_CONF.target_timer_unit_sel`; `enable_alarm_oneshot/period` write `target_period_mode`; `set_alarm_period` writes `target_period` (26 bits); `systimer_hal_init` sets `CONF.CLK_EN`.

**Exact FreeRTOS tick sequence** (`components/freertos/port_systick.c::vSystimerSetup`, unicore, alarm 0 / counter 1): (1) `systimer_hal_init` → CONF |= CLK_EN; (2) `UNIT1_LOAD_HI/LO := 0`, `UNIT1_LOAD := 1`; (3) `TARGET0_CONF.period_mode := 0` (oneshot); (4) `TARGET0_CONF.unit_sel := 1`; (5) `systimer_hal_set_alarm_period`: CONF &= ~bit24, `TARGET0_CONF.period := P`, `COMP0_LOAD := 1`, CONF |= bit24 — **note: load happens while still oneshot**; (6) `TARGET0_CONF.period_mode := 1`; (7) stall-by-CPU bits in CONF (inert); (8) `INT_ENA |= 1`; (9) CONF |= UNIT1_WORK_EN.

**Exact esp_timer sequence** (`systimer_hal_set_alarm_target`, MISS_COMPENSATE variant, alarm 2 / counter 0): CONF &= ~bit22, `TARGET2_HI/LO := T`, `COMP2_LOAD := 1`, CONF |= bit22.

**Comparator model that satisfies both** (implement this; document it with the citations above): on `COMPn_LOAD` pulse, latch `load_base := counter(unit_sel)` and `oneshot_target := TARGETn_HI/LO`, clear `fired_since_load`. Each tick, for an enabled comparator (`CONF` bit `24-n`) on a working unit: in **period mode** (read live from `TARGETn_CONF`), the next alarm is `load_base + k·period` for the smallest `k ≥ 1` not yet fired — fire when the counter reaches it (set `INT_RAW` bit n) and advance `k`; in **oneshot mode**, fire once when `counter ≥ oneshot_target` and `!fired_since_load` (this covers MISS_COMPENSATE: a past target fires on the next tick). Required outcomes: FreeRTOS alarm 0 fires at counter1 = P, 2P, 3P…; esp_timer alarm 2 fires exactly once at counter0 ≥ T, and again after a re-arm.

- [ ] **Step 1: Failing tests** (in `systimer.rs`), written as literal register-write sequences mirroring the HAL:

```rust
fn w(t: &mut SysTimer, off: u32, v: u32) { for (i, b) in v.to_le_bytes().iter().enumerate() { t.write_byte(off + i as u32, *b); } }

#[test]
fn freertos_tick_alarm0_on_counter1_period_mode_fires_every_period() {
    let mut t = SysTimer::new();
    // Literal vSystimerSetup order (see "Exact FreeRTOS tick sequence" above).
    let conf = |t: &mut SysTimer| u32::from_le_bytes([0, 1, 2, 3].map(|i| t.read_byte(CONF_REG + i)));
    let c = conf(&mut t); w(&mut t, CONF_REG, c | CLK_EN);                                  // (1)
    w(&mut t, UNIT1_LOAD_HI_REG, 0); w(&mut t, UNIT1_LOAD_LO_REG, 0); w(&mut t, UNIT1_LOAD_REG, 1); // (2)
    w(&mut t, TARGET0_CONF_REG, 0);                                                        // (3) oneshot
    w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL);                                    // (4) unit1
    let c = conf(&mut t); w(&mut t, CONF_REG, c & !TARGET0_WORK_EN);                       // (5) disable
    w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL | 100);                              //     period
    w(&mut t, COMP0_LOAD_REG, 1);                                                          //     load (still oneshot)
    let c = conf(&mut t); w(&mut t, CONF_REG, c | TARGET0_WORK_EN);                        //     enable
    w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL | TARGET_PERIOD_MODE | 100);         // (6) period mode
    w(&mut t, INT_ENA_REG, 1);                                                             // (8)
    let c = conf(&mut t); w(&mut t, CONF_REG, c | UNIT1_WORK_EN);                          // (9)
    let mut fired_at = vec![];
    for step in 1..=350u64 {
        t.advance_by(1);
        if t.pending_sources() & (1 << 37) != 0 { fired_at.push(step); w(&mut t, INT_CLR_REG, 1); }
    }
    assert_eq!(fired_at, vec![100, 200, 300]);
}

#[test]
fn esp_timer_alarm2_on_counter0_oneshot_fires_once_at_target() {
    let mut t = SysTimer::new();
    w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN);
    w(&mut t, TARGET2_CONF_REG, 0); // unit0, oneshot
    w(&mut t, TARGET2_HI_REG, 0);
    w(&mut t, TARGET2_LO_REG, 50);
    w(&mut t, COMP2_LOAD_REG, 1);
    w(&mut t, INT_ENA_REG, 1 << 2);
    w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN | TARGET2_WORK_EN);
    let mut fired = 0;
    for _ in 0..200 { t.advance_by(1); if t.pending_sources() & (1 << 39) != 0 { fired += 1; w(&mut t, INT_CLR_REG, 1 << 2); } }
    assert_eq!(fired, 1);
}

#[test]
fn unit_load_sets_counter_and_op_update_latches_value() {
    let mut t = SysTimer::new();
    w(&mut t, CONF_REG, CLK_EN | UNIT1_WORK_EN);
    w(&mut t, UNIT1_LOAD_HI_REG, 0x1);
    w(&mut t, UNIT1_LOAD_LO_REG, 0x2);
    w(&mut t, UNIT1_LOAD_REG, 1);
    w(&mut t, UNIT1_OP_REG, 1 << 30);
    assert_eq!(t.counter(1), (1u64 << 32) | 2);
}

#[test]
fn advance_by_large_jump_fires_period_alarm_once_and_rearms_in_future() {
    let mut t = SysTimer::new();
    // Same setup as the FreeRTOS test, condensed (load at counter1 = 0, period 100):
    w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL | 100);
    w(&mut t, COMP0_LOAD_REG, 1);
    w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL | TARGET_PERIOD_MODE | 100);
    w(&mut t, INT_ENA_REG, 1);
    w(&mut t, CONF_REG, CLK_EN | UNIT1_WORK_EN | TARGET0_WORK_EN);
    t.advance_by(1050);                       // crosses 100..=1000 (10 alarms) in one jump
    assert_ne!(t.pending_sources() & (1 << 37), 0, "fires (level, once)");
    assert_eq!(t.ticks_until_next_alarm(), Some(50), "re-armed to 1100, not replaying missed periods");
    w(&mut t, INT_CLR_REG, 1);
    assert_eq!(t.pending_sources(), 0);
}

#[test]
fn oneshot_target_already_in_the_past_fires_on_next_tick() {
    let mut t = SysTimer::new();
    w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN);
    t.advance_by(500);
    w(&mut t, TARGET2_LO_REG, 10);            // already passed (MISS_COMPENSATE)
    w(&mut t, COMP2_LOAD_REG, 1);
    w(&mut t, INT_ENA_REG, 1 << 2);
    w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN | TARGET2_WORK_EN);
    t.advance_by(1);
    assert_ne!(t.pending_sources() & (1 << 39), 0);
}

#[test]
fn ticks_until_next_alarm_is_none_when_nothing_armed() {
    assert_eq!(SysTimer::new().ticks_until_next_alarm(), None);
}
```

Constants the tests use (add to `systimer.rs`): `CLK_EN = 1<<31`, `UNIT0_WORK_EN = 1<<30`, `UNIT1_WORK_EN = 1<<29`, `TARGET0_WORK_EN = 1<<24`, `TARGET1_WORK_EN = 1<<23`, `TARGET2_WORK_EN = 1<<22`, `TARGET_PERIOD_MODE = 1<<30`, `TARGET_TIMER_UNIT_SEL = 1<<31`, and the register offsets listed above (`UNIT1_LOAD_HI_REG = 0x14`, `UNIT1_LOAD_LO_REG = 0x18`, `UNIT1_LOAD_REG = 0x60`, `UNIT1_OP_REG = 0x08`, `TARGET2_HI_REG = 0x2C`, `TARGET2_LO_REG = 0x30`, `TARGET2_CONF_REG = 0x3C`, `COMP2_LOAD_REG = 0x58`, …). Reset value of `CONF`: take it from `systimer_reg.h`'s documented reset values and cite it; tests above set `CONF` explicitly so they don't depend on it.

- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** with per-unit `{counter, work_en, load_hi, load_lo, value_hi, value_lo, value_valid}` and per-comparator `{hi, lo, conf, armed_target: Option<u64>}`; update `bus.pending_sources()` to OR in `systimer.pending_sources()`; bus's INTC EIP read uses `pending_sources()`. Update/replace Milestone-2 systimer tests that encoded the wrong single-unit assumption (keep their intent where still valid).
- [ ] **Step 4: Run** `cargo test -p emulator-core` (incl. boot_progress) → PASS.
- [ ] **Step 5: Docs:** notes item 2 → resolved. **Commit** `feat(emulator-core): SYSTIMER two counters, three comparators, HAL-faithful alarm sequencing`

---

### Task 6: WFI — real wait-for-interrupt with SYSTIMER fast-forward

**Files:**
- Modify: `emulator-core/src/cpu/decode.rs`, `execute.rs`, `mod.rs`, `emulator-core/src/runtime.rs`, `emulator-core/src/boot.rs`

**Interfaces:**
- Consumes: `SysTimer::ticks_until_next_alarm`, `advance_by` (Task 5); `Cpu::set_pending_interrupts` (Task 4).
- Produces: `Instruction::Wfi`; `Cpu::is_waiting(&self) -> bool`. While waiting, `Cpu::step` executes nothing and returns `StepInfo { trap_taken: false, instr_len: 0, .. }`; it leaves waiting as soon as `pending_interrupts != 0` (regardless of MIE — RISC-V privileged spec §3.3.3), then takes the trap if MIE is set, else resumes at the instruction after WFI.
- `FirmwareRuntime::run`: when `cpu.is_waiting()` at the top of a step, call `bus.systimer.advance_by(ticks)` with `ticks = ticks_until_next_alarm().unwrap_or(0).max(TICKS_PER_STEP)` instead of the normal one-step advance, then poll interrupts. Each such step still counts as exactly one step of the budget, so `run(budget)` always returns.

- [ ] **Step 1: Failing tests.** decode: `assert_eq!(decode_32(0x1050_0073), Instruction::Wfi);` (use the existing 32-bit decode entry point name in `decode.rs`). cpu:

```rust
#[test]
fn wfi_waits_until_an_interrupt_is_pending_then_traps_if_mie() {
    let mut cpu = Cpu::new();
    let mut bus = TestBus::with_program(&[0x1050_0073, 0x0000_0013]);
    cpu.csr.mstatus |= mstatus_bits::MIE;
    cpu.step(&mut bus);                  // executes WFI
    assert!(cpu.is_waiting());
    let pc = cpu.regs.pc;
    for _ in 0..5 { cpu.step(&mut bus); }
    assert_eq!(cpu.regs.pc, pc, "no progress while waiting");
    cpu.set_pending_interrupts(1 << 3);
    let info = cpu.step(&mut bus);
    assert!(info.trap_taken && !cpu.is_waiting());
}

#[test]
fn wfi_with_mie_clear_resumes_after_wfi_without_trapping() {
    let mut cpu = Cpu::new();
    let mut bus = TestBus::with_program(&[0x1050_0073, 0x0000_0013]);
    cpu.step(&mut bus);
    cpu.set_pending_interrupts(1 << 3);
    let info = cpu.step(&mut bus);
    assert!(!info.trap_taken && !cpu.is_waiting());
}
```

runtime (synthetic image whose code is `wfi; j .` with no timers armed):

```rust
#[test]
fn run_returns_after_budget_even_if_wfi_never_wakes() {
    let mut rt = FirmwareRuntime::from_image(&wfi_image()).unwrap();
    let s = rt.run(1000);
    assert_eq!(s.steps, 1000);
    assert!(rt.cpu().is_waiting());
}
```

Plus an integration test in `tests/interrupt_integration.rs`: arm SYSTIMER target0 period 1_000_000 routed to a line with priority 1, MIE set, `mtvec` pointing at a handler; execute WFI; assert the trap is taken within ≤ 3 `run` steps (fast-forward), and `counter(unit)` advanced ≈ 1_000_000.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** (decode `funct7=0b0001000, rs2=0b00101, rd=0, rs1=0, funct3=0` in `decode_system`).
- [ ] **Step 4: Run** → PASS; boot_progress green; add any new rung reached.
- [ ] **Step 5: Docs** (notes item 4 → resolved) + **commit** `feat(emulator-core): WFI with SYSTIMER fast-forward while idle`

---

### Task 7: ROM console output stubs (conditional — do when boot-probe shows ROM print calls with empty console)

**Trigger:** boot-probe shows `rom_stub_calls` to `ETS_PRINTF` or instruction faults / hot PCs at ROM output routines while `== console ==` is missing expected early lines (e.g. `cpu_start: Pro cpu start user code`, which ESP-IDF prints via `ESP_EARLY_LOGI` → `esp_rom_printf`).

**Files:** Modify `emulator-core/src/cpu/rom_stubs.rs`, `emulator-core/src/rom.rs`.

**Interfaces:**
- Produces: `RomStubEffect::WriteA0ByteToMmio(u32)` — chip-agnostic mechanism: writes `a0 & 0xFF` via `bus.write8(addr, …)` then returns (`Void`). `rom.rs` registers ROM putc-family entry points (look up exact names/addresses in v5.5.3 `components/esp_rom/esp32c3/ld/esp32c3.rom.ld` and `esp32c3.rom.api.ld`, e.g. the target of `esp_rom_output_putc` / `esp_rom_output_tx_one_char`) with `WriteA0ByteToMmio(USB_SERIAL_JTAG_RANGE.start)`, so ROM output lands in the same console as FIFO output.
- If the firmware calls the ROM's formatting `ets_printf` directly (vararg formatting inside ROM), do **not** implement printf in the stub; instead record the finding and check whether ESP-IDF v5.5.3's `esp_rom_printf` is IDF-side (it formats in IDF and calls a ROM putc) — if so, stubbing putc is sufficient.

- [ ] Step 1: failing test in `rom_stubs.rs`: `a0 = b'Q'`, pc at a registered stub addr → after step, bus byte written at the MMIO address == `b'Q'`, `a0` unchanged, pc == ra.
- [ ] Step 2: run → FAIL. Step 3: implement. Step 4: run → PASS; boot-probe now shows early log lines; add ratchet rungs for `cpu_start: Pro cpu start user code` and `cpu_start: cpu freq:` (assert only the prefix `cpu_start: cpu freq:` — the value may differ, see the `CPU_FREQ_MHZ = 160` flagged guess).
- [ ] Step 5: commit `feat(emulator-core): route ROM putc output into the console`

---

### Task 8: Emulated flash + SPI1/SPIMEM1 flash controller + flash MMU (conditional — expected before `spi_flash: detected chip`)

**Trigger:** boot-probe stalls with hot PCs in `esp_flash`/`spi_flash` code, or unmapped accesses in `0x6000_2000..0x6000_3000` (SPIMEM1, verify `DR_REG_SPI1_BASE`) or the MMU table / EXTMEM range (verify `DR_REG_MMU_TABLE` / `DR_REG_EXTMEM_BASE` in `soc/reg_base.h`), or the partition table load fails (`esp_partition` errors in console).

**Files:**
- Create: `emulator-core/src/peripherals/flash.rs` (4 MiB flash array + SPIMEM1 register model), possibly `emulator-core/src/peripherals/mmu.rs`
- Modify: `mem/bus.rs` (XIP reads go through the flash array + MMU instead of per-segment `file_offset`), `mem/soc.rs`, `boot.rs` (build the 4 MiB flash image; seed MMU entries matching what the 2nd-stage bootloader would have set for the app's DROM/IROM segments)

**Interfaces:**
- Produces: `peripherals::flash::EmulatedFlash { pub fn from_app_image(app: &[u8]) -> Self /* 4 MiB, 0xFF-filled, app at 0x10000, synthesized partition table at 0x8000 */; pub fn read(&self, off: u32) -> u8; pub fn erase_sector(&mut self, off: u32); pub fn program(&mut self, off: u32, data: &[u8]) /* AND semantics: bits only go 1->0 */ }`.
- Synthesized partition table: exactly the 4 entries in "Handoff state" (32-byte `esp_partition_info_t` entries, magic `0xAA50`, then an MD5 entry `0xEBEB` + 16-byte MD5 of preceding entries if the firmware's `CONFIG_PARTITION_TABLE_MD5` is enabled — check by reading the real table's layout from `BADGE_FULL_DUMP` in a gated test, never by committing its bytes; the partition layout itself is not personal data and may be committed as constants).
- SPIMEM1 model: user-command transactions sufficient for `spi_flash_hal` / `memspi_host` in v5.5.3 (`components/hal/spi_flash_hal_common.inc`, `hal/esp32c3/include/hal/spimem_flash_ll.h`): RDID (0x9F) → `0x46 0x40 0x16`; RDSR (0x05) → 0 (never busy); READ/fast-read into `W0..W15`; WREN; sector erase (0x20) / block erase (0xD8) / page program (0x02) applied to the in-memory array; `CMD` trigger bits self-clear on completion. Model only commands the probe shows being issued; list them in the module doc with citations.
- MMU: verify the C3 MMU entry format (`hal/esp32c3/include/hal/mmu_ll.h`: 64 KiB pages, 128 entries, invalid bit) and implement `mmu_ll_write_entry`-visible behavior so `spi_flash_mmap` of arbitrary flash pages works for DROM reads.

- [ ] Steps: TDD per sub-unit (flash array semantics → SPIMEM1 RDID/RDSR/READ → erase/program → MMU entry translation → bus XIP via MMU with bootloader-equivalent seeding such that all existing boot tests remain green) — each sub-unit: write failing test with literal register writes mirroring the cited LL functions, run → FAIL, implement, run → PASS, commit. Gated test (skips without `BADGE_FULL_DUMP`): the synthesized partition table is byte-identical to the real one at 0x8000..0x8000+N for the entries region.
- [ ] Ratchet rungs: `spi_flash: detected chip: generic`, and whichever of `main_task: Calling app_main()` / later lines become reachable.
- [ ] Commit(s): `feat(emulator-core): emulated 4 MiB flash, SPIMEM1 controller, flash MMU`

---

### Task 9: SPI2 register fidelity — UPDATE self-clear, DMA_INT, TRANS_DONE interrupt

**Files:** Modify `emulator-core/src/peripherals/spi.rs`, `emulator-core/src/mem/bus.rs` (`pending_sources` bit `SRC_SPI2`), `emulator-core/tests/spi_integration.rs`.

**Interfaces:**
- Produces: `Spi::pending_sources(&self) -> u64` (bit `SRC_SPI2` when `DMA_INT_RAW & DMA_INT_ENA & TRANS_DONE`); `Spi::dma_tx_enabled(&self) -> bool`; `Spi::tx_byte_len(&self) -> usize` (from `MS_DLEN`, existing rounding); `Spi::finish_transaction(&mut self)` sets `TRANS_DONE` raw and clears `USR`. `process_transaction(dc_low)` keeps today's CPU-buffer (W0..W15) path.

Register facts (verify + cite `components/soc/esp32c3/register/soc/spi_reg.h`, `components/hal/esp32c3/include/hal/spi_ll.h` `spi_ll_apply_config`, `spi_ll_user_start`, `spi_ll_usr_is_done`, `spi_ll_dma_tx_enable`): `SPI_CMD_REG` bit 23 `UPDATE` (write 1 → reads back 0 immediately), bit 24 `USR`; `SPI_DMA_CONF_REG` (+0x30, verify) with `DMA_TX_ENA`; `SPI_DMA_INT_ENA/CLR/RAW/ST` (+0x34/+0x38/+0x3C/+0x40, verify) with `TRANS_DONE` (bit 12, verify).

- [ ] Step 1: failing tests (in `spi.rs`):

```rust
#[test]
fn update_bit_self_clears() {
    let mut s = Spi::new();
    write_word(&mut s, CMD_REG, CMD_UPDATE);
    assert_eq!(read_word(&s, CMD_REG) & CMD_UPDATE, 0);
}

#[test]
fn transaction_sets_trans_done_raw_and_int_when_enabled() {
    let mut s = Spi::new();
    write_word(&mut s, DMA_INT_ENA_REG, TRANS_DONE);
    write_word(&mut s, MS_DLEN_REG, 7);
    write_word(&mut s, W0_REG, 0x2C);
    if write_word(&mut s, CMD_REG, CMD_USR) { s.process_transaction(true); }
    assert_ne!(read_word(&s, DMA_INT_RAW_REG) & TRANS_DONE, 0);
    assert_ne!(s.pending_sources() & (1u64 << crate::mem::soc::SRC_SPI2), 0);
    write_word(&mut s, DMA_INT_CLR_REG, TRANS_DONE);
    assert_eq!(s.pending_sources(), 0);
}
```
- [ ] Step 2: run → FAIL. Step 3: implement (`process_transaction` calls `finish_transaction` at the end). Step 4: run → PASS. Update `bus.pending_sources`. Step 5: docs (notes item 3, first half) + commit `feat(emulator-core): SPI2 UPDATE self-clear and TRANS_DONE interrupt`

---

### Task 10: GDMA out-link → SPI2 DMA transmit

**Files:** Create `emulator-core/src/peripherals/gdma.rs`; modify `peripherals/mod.rs`, `mem/soc.rs` (`GDMA_RANGE`, verify `DR_REG_GDMA_BASE` = 0x6003_F000), `mem/bus.rs`, `peripherals/spi.rs`, `tests/spi_integration.rs`.

**Interfaces:**
- Consumes: `Spi::dma_tx_enabled`, `Spi::tx_byte_len`, `Spi::finish_transaction`, `St7789::handle_transaction(dc_low, &[u8])` (existing, via `Spi`), `soc::SRC_DMA_CH{0,1,2}`.
- Produces: `Gdma { pub fn new() -> Self; pub fn read_byte(&mut self, off: u32) -> u8; pub fn write_byte(&mut self, off: u32, val: u8); pub fn channel_for_spi2(&self) -> Option<usize>; pub fn pending_sources(&self) -> u64 }` and a bus-level function (in `bus.rs`, since it needs RAM access) `fn gdma_pull(&mut self, ch: usize, max_len: usize) -> Vec<u8>` that walks descriptors starting at the channel's current descriptor pointer, reading RAM through `self.read_byte`, clearing each descriptor's owner bit (write back via `self.write_byte`), stopping at `suc_eof` or `max_len`, and raising `OUT_EOF`/`OUT_DONE`/`OUT_TOTAL_EOF` raw bits and `OUT_EOF_DES_ADDR`.
- SPI2 trigger path in `bus.write_byte`: if `spi.dma_tx_enabled()` and `gdma.channel_for_spi2() == Some(ch)`, bytes = `gdma_pull(ch, spi.tx_byte_len())`, then `spi.handle_dma_bytes(dc_low, &bytes)` (new `Spi` method feeding the ST7789 interpreter + `finish_transaction`); else the existing W0..W15 path.

Register facts (verify every one + cite `components/soc/esp32c3/register/soc/gdma_reg.h`, `components/hal/esp32c3/include/hal/gdma_ll.h`, `components/hal/include/hal/dma_types.h` for `dma_descriptor_t`): 3 channels, per-channel stride 0xC0; out-link block per channel: `OUT_CONF0` +0x60, `OUT_CONF1` +0x64, `OUT_INT_RAW` +0x68, `OUT_INT_ST` +0x6C, `OUT_INT_ENA` +0x70, `OUT_INT_CLR` +0x74, `OUT_LINK` +0x80 (bits 19:0 descriptor address low bits — full address = `0x3FC0_0000 | (addr & 0xFFFFF)`, verify against `gdma_ll_tx_set_desc_addr`; bit 21 `START`, bit 20 `STOP`, bit 22 `RESTART`), `OUT_EOF_DES_ADDR` +0x88, `OUT_PERI_SEL` (verify offset; SPI2 = peripheral id 0 per `soc/gdma_channel.h`). Descriptor: word0 `size[11:0] length[23:12] suc_eof bit30 owner bit31`, word1 buffer ptr, word2 next ptr (0 = end). Interrupt bits: `OUT_DONE` bit0, `OUT_EOF` bit1, `OUT_DSCR_ERR` bit2, `OUT_TOTAL_EOF` bit3 (verify). In-link registers: plain storage only.

- [ ] Step 1: failing unit tests in `gdma.rs` (register storage, `OUT_LINK` START latches descriptor address and self-clears, `channel_for_spi2` honors `OUT_PERI_SEL`, INT RAW/ENA/CLR/ST semantics, `pending_sources` bits 44..46). Failing integration test in `tests/spi_integration.rs` building a real 2-descriptor chain in DRAM holding a CASET/RASET-prepared RAMWR payload of 200 bytes (> 64, the W-buffer limit), programming GDMA ch0 → SPI2, SPI2 `DMA_TX_ENA`, `MS_DLEN = 200*8-1`, GPIO0 high (data), `USR` → assert 100 framebuffer pixels written in order, descriptor owner bits cleared, `OUT_EOF` raw set, `TRANS_DONE` raw set.

```rust
// sketch of descriptor construction for the integration test (write it fully):
fn put_desc(bus: &mut FirmwareBus, at: u32, buf: u32, len: u32, eof: bool, next: u32) {
    let w0 = (len & 0xFFF) | ((len & 0xFFF) << 12) | if eof { 1 << 30 } else { 0 } | (1 << 31);
    bus.write32(at, w0); bus.write32(at + 4, buf); bus.write32(at + 8, next);
}
```
- [ ] Step 2: run → FAIL. Step 3: implement. Step 4: run → PASS; `pending_sources` extended. Step 5: docs (notes item 3 → resolved) + commit `feat(emulator-core): GDMA out-link channels feeding SPI2 DMA transmit`

---

### Task D (template, repeat per unknown blocker)

For any stall not covered above (RTC_CNTL clock tree, eFuse, I2C0, APB_CTRL, SYSTEM clock gating, a flagged ROM-stub guess, `CPU_FREQ_MHZ`, `TICKS_PER_STEP`, IRAM/DRAM aliasing, …). The orchestrator creates a new numbered task from this template and dispatches it like any other:

**Files:** new `emulator-core/src/peripherals/<name>.rs` or the existing module that owns the registers; `mem/soc.rs`; `mem/bus.rs`; `tests/boot_progress.rs`; `docs/firmware-emulator-notes.md`.

- [ ] Step 1: Reproduce with `boot-probe`; record last console line, top hot PCs, unmapped tail in the task brief.
- [ ] Step 2: Identify the registers and the ESP-IDF v5.5.3 source (header + HAL/LL function) that reads them; if disassembly is needed, disassemble `factory.bin` around the hot PCs (e.g. `riscv64-unknown-elf-objdump -b binary -m riscv:rv32 -D --adjust-vma=<load_addr>` on the extracted segment, or a small Rust helper using this crate's decoder) — do not guess.
- [ ] Step 3: Failing unit test with literal register writes/reads mirroring the cited LL/HAL function; run → FAIL.
- [ ] Step 4: Register-faithful minimal implementation; module doc cites the sources; run → PASS.
- [ ] Step 5: Add the ratchet rung for the newly reached ladder line (or, if between rungs, a hot-PC-escape assertion).
- [ ] Step 6: Update notes (predicted → resolved / new finding). Full suite. Commit `feat(emulator-core): model <peripheral> (<what it unblocks>)`.
- I2C specifically: if `hal_accel` blocks, model I2C0 so every transaction completes with NACK (`hal_accel` then reports no accelerometer) — cite `i2c_ll.h`.

---

### Task 11: Finish line — first-frame test, PNG dump, browser check, docs

**Files:** Modify `emulator-core/tests/boot_progress.rs`, `docs/firmware-emulator-notes.md`, `CLAUDE.md`; optionally `frontend/src/ui/shell.ts` / `frontend/src/main.ts` for a collapsible console pane (only if trivial; skip otherwise).

**Interfaces:** Consumes everything above.

- [ ] **Step 1:** Run `boot-probe --steps <large> --dump-frame local/first-frame.png`; find the smallest step count at which the framebuffer has > 1 distinct color and is stable across the next 1_000_000 steps. Ask the human partner to compare `local/first-frame.png` against the physical badge's first screen (the frame may show defaults, not personal data, since flash storage is blank).
- [ ] **Step 2: Finish-line test:**

```rust
#[test]
fn boots_to_first_real_frame() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    const MAX: u64 = /* observed steps × 2, rounded */ 0;
    while rt.total_steps() < MAX {
        rt.run(250_000);
        let fb = rt.framebuffer();
        let distinct: std::collections::HashSet<u16> = fb.iter().copied().collect();
        if distinct.len() > 1 { break; }
    }
    let fb = rt.framebuffer();
    let distinct: std::collections::HashSet<u16> = fb.iter().copied().collect();
    assert!(distinct.len() > 1, "no frame within {MAX} steps; pc=0x{:08x}\n{}", rt.pc(), rt.console_output());
    // FNV-1a over the framebuffer at the first non-blank frame, then after it stabilizes:
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for px in fb { for b in px.to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100_0000_01b3); } }
    assert_eq!(h, /* pinned */ 0, "first frame changed — inspect with boot-probe --dump-frame");
}
```
Replace `MAX` and the pinned hash with observed values (the test must be deterministic; run it 3× to confirm). If the first non-blank frame is a partial flush, advance to the first stable frame (unchanged for 1_000_000 steps) and pin that instead — document the choice in the test's doc comment.
- [ ] **Step 3:** `bun run build:wasm && bun run dev`; in firmware mode confirm the frame renders in the browser; report (screenshot to `local/`). Not gating.
- [ ] **Step 4: Docs.** `docs/firmware-emulator-notes.md`: rewrite "Known limitations" to the new stall point after first frame, list resolved items, record the flash/partition policy and the ground-truth facts (no personal data). `CLAUDE.md`: add new modules (`console`, `usb_serial_jtag`, `timg`, `gdma`, `flash`/`mmu` if built), `boot-probe` command, `boot_progress` test notes, `BADGE_FULL_DUMP` + `local/` policy, and update the "stalls before reaching any built-in app" wording.
- [ ] **Step 5:** Full suite (`cargo test --workspace`, `cargo test -p emulator-core --release --test boot_progress`, `bun run build:wasm && bun test && bun run typecheck`) → PASS. Commit `feat: real firmware boots to first frame (Milestone 3)`.
- [ ] **Step 6:** Whole-branch review (fresh reviewer subagent on the full diff vs `main`, checking spec conformance, citations, Review Focus items, and that nothing from `local/` or identity data leaked into git: `grep -F -f <(grep -oE 'hal_identity: .*' local/boot_log.txt | sed 's/hal_identity: identity: //; s/ (.*) id=/\n/' | tr -d '\r') <(git log -p main..HEAD)` must print nothing, and `git log --all --stat main..HEAD | grep -E 'local/|dump'` must print nothing. The identity patterns are read from `local/` at check time and must never be written into any committed file). Then superpowers:finishing-a-development-branch.

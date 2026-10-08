//! ESP32-C3 peripheral models wired into `crate::mem::bus::FirmwareBus`.
//!
//! Interrupt sources are levels recomputed every step
//! (`FirmwareBus::pending_sources`) and delivered by
//! `crate::boot::step_with_interrupts`, which also fast-forwards
//! [`systimer`] while the core waits in `WFI` (Milestone 3 Task 6). See
//! each submodule's doc comment for register-layout citations and the
//! judgment calls their exact behavior required.
//!
//! - [`systimer`]: [`systimer::SysTimer`], two 52-bit counters and three
//!   comparators driving `SRC_SYSTIMER_TARGET0..2` (Milestone 3 Task 5): the
//!   FreeRTOS tick (alarm 0 on counter 1) and esp_timer (alarm 2 on counter 0).
//! - [`intc`]: [`intc::InterruptController`], the ESP32-C3's non-PLIC
//!   interrupt matrix (`INTERRUPT_CORE0`) — per-source MAP registers routing
//!   into 32 CPU interrupt lines, gated by `CPU_INT_ENABLE_REG` and by
//!   priority vs. `CPU_INT_THRESH_REG` (Milestone 3 Task 4); a priority-0
//!   line is disabled (Task 6).
//! - [`system`]: [`system::System`], the SYSTEM peripheral's four
//!   `FROM_CPU` software-interrupt registers (Milestone 3 Task 4) — the
//!   level sources FreeRTOS's `vPortYield` raises to request a context
//!   switch.
//! - [`rtc_cntl`]: [`rtc_cntl::RtcCntl`], the RTC_CNTL peripheral's RTC timer
//!   (`TIME_UPDATE_REG`/`TIME_LOW0_REG`/`TIME_HIGH0_REG`) that
//!   `rtc_cntl_ll_get_rtc_time()` reads at boot — derives its latched value
//!   from `systimer`'s monotonic `elapsed_ticks()` scaled to the RTC slow
//!   clock. Also
//!   `STORE4_REG` (`RTC_XTAL_FREQ_REG`), plain storage the shortcut boot
//!   seeds as the bootloader would (Task D12).
//! - [`gpio`]: [`gpio::Gpio`], the GPIO peripheral plus an emulated 74HC165
//!   shift register (`gpio::Hc165`) the real badge uses to read 7 of its 8
//!   buttons.
//! - [`spi`]: [`spi::Spi`], the SPI2 (GPSPI2) peripheral plus an ST7789
//!   command/pixel-stream interpreter (`spi::St7789`) that reconstructs a
//!   framebuffer from what real firmware writes to drive the badge's LCD.
//!   Needs a live cross-peripheral read of `gpio`'s GPIO0 level (the D/C
//!   line) at transaction-trigger time — see `spi`'s module doc.
//! - [`gdma`]: [`gdma::Gdma`], the GDMA controller's three channels
//!   (Milestone 3 Task 10): interrupt registers driving
//!   `SRC_DMA_CH0..2`, the TX out-link FSM, and plain-storage RX
//!   registers. With `SPI_DMA_TX_ENA` set, an SPI2 transaction's bytes come
//!   from the TX channel connected to SPI2; the descriptor walk reads RAM,
//!   so it lives in `FirmwareBus::gdma_pull`.
//! - [`console`]: [`console::Console`], a capped byte sink every "firmware
//!   printed something" path (USB-Serial-JTAG TX, UART0 TX, ROM putc stubs)
//!   feeds into — diagnostic output, not emulated device state.
//! - [`usb_serial_jtag`]: [`usb_serial_jtag::UsbSerialJtag`], the
//!   USB-Serial-JTAG peripheral (the badge's actual console transport) that
//!   feeds [`console::Console`] from firmware TX writes.
//! - [`timg`]: [`timg::Timg`], the TIMG0/TIMG1 timer-group peripherals —
//!   RTC slow-clock calibration (`RTCCALICFG*_REG`, what `rtc_clk_cal()`
//!   polls at boot) plus inert MWDT watchdog storage. `FirmwareBus` holds
//!   one `Timg` instance per timer group (`timg0`, `timg1`).
//! - [`flash`]: [`flash::EmulatedFlash`], the badge's 4 MiB SPI NOR flash
//!   chip as an in-memory array: blank (`0xFF`) except a synthesized
//!   partition table at `0x8000` and the app image at `0x10000`, with NOR
//!   erase/program semantics (writes are in memory only); and
//!   [`flash::Spimem1`], the SPI1 flash controller (`0x6000_2000`) whose
//!   commands reach that chip (RDID, RDSR, reads, and the dedicated
//!   write-enable, sector-erase and page-program bits). `FirmwareBus`
//!   holds them as `flash_chip` and `spimem1`.
//! - [`i2c`]: [`i2c::I2c`], the I2C0 controller in master mode (Milestone 4
//!   Task D-M4-1): the command list runs on `TRANS_START`, and `INT_STATUS`
//!   drives `SRC_I2C_EXT0` for the `esp_driver_i2c` ISR. `FirmwareBus` holds
//!   it as `i2c0`. One device answers on its bus (Milestone 5 Task D-M5-2):
//!   [`sc7a20h::Sc7a20h`], the accelerometer at `0x19`, a concrete field of
//!   the controller (`i2c0.accel`), with a host-set acceleration; every
//!   other address NACKs.
//! - [`rmt`]: [`rmt::Rmt`], the RMT controller as a TX engine that completes
//!   each transmission in zero time (Milestone 4 Task D-M4-2): `TX_START`
//!   consumes the channel's RMT RAM symbols, pausing at each enabled
//!   threshold/loop interrupt until the ISR acknowledges it, and `INT_ST`
//!   drives `SRC_RMT`. `FirmwareBus` holds it as `rmt`.
//!
//! Per this plan's pre-flight design ruling, no peripheral is behind a
//! trait object: `FirmwareBus` holds concrete, named fields for each, and
//! does its own address-range dispatch (see `mem::bus`'s module doc) —
//! mirroring how `FirmwareBus` already distinguishes XIP vs. RAM-copied
//! regions by a manual range check, not a generic abstraction.

pub mod console;
pub mod flash;
pub mod gdma;
pub mod gpio;
pub mod i2c;
pub mod intc;
pub mod mmu;
pub mod rmt;
pub mod rtc_cntl;
pub mod sc7a20h;
pub mod spi;
pub mod system;
pub mod systimer;
pub mod timg;
pub mod usb_serial_jtag;

/// Replaces byte `idx` (`0..=3`, little-endian, i.e. `idx == 0` is the
/// least-significant byte) of `word` with `val`, leaving the other three
/// bytes untouched.
///
/// Shared by both peripherals' byte-granular `read_byte`/`write_byte`
/// methods, which exist so `FirmwareBus`'s existing byte-oriented
/// `read_byte`/`write_byte` dispatch (see `mem::bus`) can route individual
/// bytes of a CPU `sb`/`sh`/`sw` straight through without FirmwareBus ever
/// needing to reconstruct a 32-bit word itself. Each peripheral applies
/// write-triggered side effects (SYSTIMER's `OP_REG`/`COMP0_LOAD_REG`
/// latches, INTC's none) by inspecting *only* the specific byte that
/// contains the relevant trigger bit, not a word reconstructed from
/// possibly-stale/fake state — see `systimer::SysTimer::write_byte`'s doc
/// comment for why that matters and is safe regardless of the order a
/// multi-byte access's individual bytes arrive in.
pub(crate) fn set_byte(word: &mut u32, idx: u32, val: u8) {
    let shift = (idx & 0b11) * 8;
    *word = (*word & !(0xFFu32 << shift)) | ((val as u32) << shift);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_byte_replaces_only_the_targeted_byte() {
        let mut word = 0xAABBCCDDu32;
        set_byte(&mut word, 0, 0x11);
        assert_eq!(word, 0xAABBCC11);
        set_byte(&mut word, 3, 0x22);
        assert_eq!(word, 0x22BBCC11);
    }
}

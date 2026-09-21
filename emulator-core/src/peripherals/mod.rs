//! ESP32-C3 peripheral models wired into `crate::mem::bus::FirmwareBus`.
//!
//! Task 3 scope: just enough of the interrupt matrix + SYSTIMER for
//! FreeRTOS's periodic tick interrupt (`ETS_SYSTIMER_TARGET0_INTR_SOURCE`,
//! per ESP-IDF v5.5.3's `components/freertos/port_systick.c`) to actually
//! reach the CPU core once something later unblocks the mask-ROM boundary.
//! See each submodule's doc comment for register-layout citations and the
//! judgment calls their exact behavior required.
//!
//! - [`systimer`]: [`systimer::SysTimer`], the free-running 52-bit counter +
//!   comparator peripheral.
//! - [`intc`]: [`intc::InterruptController`], the ESP32-C3's non-PLIC
//!   interrupt matrix (`INTERRUPT_CORE0`) — per-source MAP registers routing
//!   into 32 CPU interrupt lines, gated by `CPU_INT_ENABLE_REG`.
//! - [`gpio`]: [`gpio::Gpio`], the GPIO peripheral plus an emulated 74HC165
//!   shift register (`gpio::Hc165`) the real badge uses to read 7 of its 8
//!   buttons.
//!
//! Per this plan's pre-flight design ruling, no peripheral is behind a
//! trait object: `FirmwareBus` holds concrete, named fields for each, and
//! does its own address-range dispatch (see `mem::bus`'s module doc) —
//! mirroring how `FirmwareBus` already distinguishes XIP vs. RAM-copied
//! regions by a manual range check, not a generic abstraction.

pub mod gpio;
pub mod intc;
pub mod systimer;

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

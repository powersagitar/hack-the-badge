//! ESP32-C3 SoC address-space regions relevant to booting a real ESP-IDF app
//! image.
//!
//! ESP-IDF's own app loader (the 2nd-stage bootloader, `esp_image_format.c` /
//! `image_process`) decides whether a segment is flash-mapped
//! execute-in-place (XIP) or must be copied into RAM purely by which
//! documented SoC address region that segment's `load_addr` falls into —
//! there is no flag in the segment header itself saying "this one's XIP".
//! So categorization here is necessarily range-based too.
//!
//! The ranges below are the flash MMU cache apertures and RAM regions of the
//! ESP32-C3 memory map, as documented in ESP-IDF's `soc/soc.h` /
//! `esp32c3.peripherals.ld` for this target (chip ID 5, confirmed against
//! `factory.bin`'s own header — see `crate::mem::image`). We keep the ranges
//! generous (whole documented apertures, not "just wide enough for today's
//! six segments") on purpose: a future firmware revision can shift segment
//! boundaries within the same aperture without silently getting
//! miscategorized by an accidentally-too-tight bound.

use core::ops::Range;

/// Flash-mapped, read-only *data* aperture (DROM). The flash cache maps
/// arbitrary flash offsets into this virtual address window; segments
/// linked here are read-only rodata/strings.
pub const DROM_RANGE: Range<u32> = 0x3C00_0000..0x3E00_0000;

/// Flash-mapped, read-only *instruction* aperture (IROM). Same flash-cache
/// mechanism as [`DROM_RANGE`], but for code fetches.
pub const IROM_RANGE: Range<u32> = 0x4200_0000..0x4400_0000;

/// Internal SRAM mapped as data (DRAM): where `.data`/`.rodata`/`.bss`, the
/// heap, and the stack actually live at runtime. The app image only carries
/// initialized bytes for the segments that need them (`.data`/`.rodata`);
/// `.bss`/heap/stack occupy the rest of this same aperture without any
/// bytes present in the flash image (they don't need init data). Used by
/// `crate::boot` to place a scratch stack region right after the
/// highest-address DRAM segment the image actually carries — see
/// `boot::boot_from_factory_image`'s doc comment for the SP investigation
/// this backs.
pub const DRAM_RANGE: Range<u32> = 0x3FC8_0000..0x3FCE_0000;

/// `true` if `addr` falls inside one of the flash-mapped XIP apertures
/// ([`DROM_RANGE`] or [`IROM_RANGE`]). Everything else in the app image
/// (DRAM/IRAM/RTC segments) is RAM-copied at boot instead — see
/// `crate::mem::bus::FirmwareBus::from_segments`.
pub fn is_xip_addr(addr: u32) -> bool {
    DROM_RANGE.contains(&addr) || IROM_RANGE.contains(&addr)
}

/// SYSTIMER peripheral registers (`DR_REG_SYSTIMER_BASE`, confirmed via
/// ESP-IDF v5.5.3's `soc/reg_base.h`). One full 4 KiB register page, the
/// standard ESP32 peripheral spacing — generous on purpose (same rationale
/// as [`DROM_RANGE`]/[`IROM_RANGE`]: a later task adding more systimer
/// register support shouldn't need to touch this range), and tight enough
/// not to swallow the next peripheral's base address. See
/// `crate::peripherals::systimer` for what's actually modeled within it.
pub const SYSTIMER_RANGE: Range<u32> = 0x6002_3000..0x6002_4000;

/// `INTERRUPT_CORE0` (the ESP32-C3's non-PLIC interrupt matrix) registers
/// (`DR_REG_INTERRUPT_CORE0_BASE == DR_REG_INTERRUPT_BASE`, confirmed via
/// ESP-IDF v5.5.3's `soc/reg_base.h`). Same one-4KiB-page rationale as
/// [`SYSTIMER_RANGE`]. See `crate::peripherals::intc` for what's actually
/// modeled within it.
pub const INTERRUPT_CORE0_RANGE: Range<u32> = 0x600c_2000..0x600c_3000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drom_and_irom_addresses_are_xip() {
        assert!(is_xip_addr(0x3c130020)); // real segment 0 load_addr
        assert!(is_xip_addr(0x42000020)); // real segment 2 load_addr
    }

    #[test]
    fn dram_iram_rtc_addresses_are_not_xip() {
        assert!(!is_xip_addr(0x3fc99c00)); // DRAM
        assert!(!is_xip_addr(0x40380000)); // IRAM
        assert!(!is_xip_addr(0x50000000)); // RTC
    }

    #[test]
    fn systimer_and_interrupt_core0_ranges_are_disjoint_from_each_other_and_xip_iram() {
        assert!(SYSTIMER_RANGE.contains(&0x6002_3000));
        assert!(!SYSTIMER_RANGE.contains(&0x6002_4000)); // exclusive end
        assert!(INTERRUPT_CORE0_RANGE.contains(&0x600c_2000));
        assert!(!INTERRUPT_CORE0_RANGE.contains(&0x600c_3000)); // exclusive end
        assert!(!SYSTIMER_RANGE.contains(&INTERRUPT_CORE0_RANGE.start));
        assert!(!INTERRUPT_CORE0_RANGE.contains(&SYSTIMER_RANGE.start));
        assert!(!is_xip_addr(SYSTIMER_RANGE.start));
        assert!(!is_xip_addr(INTERRUPT_CORE0_RANGE.start));
    }
}

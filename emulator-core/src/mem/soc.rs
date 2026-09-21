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
}

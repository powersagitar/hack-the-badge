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
/// bytes present in the flash image (they don't need init data), so
/// `crate::boot` backs the whole aperture with zeroed RAM — see
/// `boot::boot_from_factory_image`'s doc comment.
///
/// Matches `SOC_DRAM_LOW`/`SOC_DRAM_HIGH` in ESP-IDF v5.5.3's
/// `components/soc/esp32c3/include/soc/soc.h` exactly.
pub const DRAM_RANGE: Range<u32> = 0x3FC8_0000..0x3FCE_0000;

/// Internal SRAM mapped as instructions (IRAM): where `.iram*` code and any
/// IRAM-resident data/bss live. `SOC_IRAM_LOW`/`SOC_IRAM_HIGH` from the same
/// `soc.h`.
///
/// **Known fidelity gap**: on real silicon DRAM and IRAM are two apertures
/// onto *the same* 400 KiB SRAM (IRAM `0x4038_0000` is the same physical word
/// as DRAM `0x3FC8_0000`), and the app image's own segment layout shows it —
/// its IRAM segment ends at SRAM offset `0x1db6c` and its first DRAM segment
/// starts at `0x1dc00`, packed contiguously by the linker. This emulator
/// models the two apertures as *separate* buffers, so firmware that writes
/// through one aperture and reads through the other would not see its own
/// write. Nothing observed so far depends on that; it's recorded here rather
/// than silently assumed away.
pub const IRAM_RANGE: Range<u32> = 0x4037_C000..0x403E_0000;

/// RTC slow memory (`SOC_RTC_IRAM_LOW`..`SOC_RTC_IRAM_HIGH`, which on the
/// ESP32-C3 is the same window as `SOC_RTC_DRAM_*`/`SOC_RTC_DATA_*` — the chip
/// has only one RTC memory). Survives deep sleep on real hardware; here it's
/// just ordinary zeroed RAM.
pub const RTC_RANGE: Range<u32> = 0x5000_0000..0x5000_2000;

/// Base of the region at the top of DRAM that the *mask ROM* uses for its own
/// stack (`SOC_ROM_STACK_START`/`SOC_ROM_STACK_SIZE` in ESP-IDF v5.5.3's
/// `soc.h`: `0x3fcd_e710`, `0x2000` — a downward-growing stack, so the
/// reserved window is `[START - SIZE, START)`). `crate::boot` seeds the app's
/// initial `sp` just below it, which is where a real 2nd-stage bootloader's
/// own stack sits when it hands control to the app.
pub const ROM_STACK_START: u32 = 0x3FCD_E710;
/// Size of the mask ROM's reserved stack window — see [`ROM_STACK_START`].
pub const ROM_STACK_SIZE: u32 = 0x2000;

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

/// GPIO peripheral registers (`DR_REG_GPIO_BASE`, confirmed via ESP-IDF
/// v5.5.3's `soc/reg_base.h`). Same one-4KiB-page rationale as
/// [`SYSTIMER_RANGE`] (the header's highest-cited `GPIO_*_REG` offset is
/// `0x6FC`, comfortably inside one page). See `crate::peripherals::gpio` for
/// what's actually modeled within it.
pub const GPIO_RANGE: Range<u32> = 0x6000_4000..0x6000_5000;

/// SPI2 (GPSPI2) peripheral registers (`DR_REG_SPI2_BASE`, confirmed via
/// ESP-IDF v5.5.3's `soc/reg_base.h`). This is the SPI instance the badge's
/// ST7789 display uses (SPI2_HOST). Same one-4KiB-page rationale as
/// [`SYSTIMER_RANGE`] (the header's highest-cited register this module
/// models, `SPI_W15_REG` at `0xD4`, is comfortably inside one page). See
/// `crate::peripherals::spi` for what's actually modeled within it.
pub const SPI2_RANGE: Range<u32> = 0x6002_4000..0x6002_5000;

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

    #[test]
    fn gpio_range_is_disjoint_from_the_other_peripheral_ranges_and_xip_iram() {
        assert!(GPIO_RANGE.contains(&0x6000_4000));
        assert!(!GPIO_RANGE.contains(&0x6000_5000)); // exclusive end
        assert!(!SYSTIMER_RANGE.contains(&GPIO_RANGE.start));
        assert!(!INTERRUPT_CORE0_RANGE.contains(&GPIO_RANGE.start));
        assert!(!GPIO_RANGE.contains(&SYSTIMER_RANGE.start));
        assert!(!GPIO_RANGE.contains(&INTERRUPT_CORE0_RANGE.start));
        assert!(!is_xip_addr(GPIO_RANGE.start));
    }

    #[test]
    fn spi2_range_is_disjoint_from_the_other_peripheral_ranges_and_xip_iram() {
        assert!(SPI2_RANGE.contains(&0x6002_4000));
        assert!(!SPI2_RANGE.contains(&0x6002_5000)); // exclusive end
        assert!(!SYSTIMER_RANGE.contains(&SPI2_RANGE.start));
        assert!(!INTERRUPT_CORE0_RANGE.contains(&SPI2_RANGE.start));
        assert!(!GPIO_RANGE.contains(&SPI2_RANGE.start));
        assert!(!SPI2_RANGE.contains(&SYSTIMER_RANGE.start));
        assert!(!SPI2_RANGE.contains(&INTERRUPT_CORE0_RANGE.start));
        assert!(!SPI2_RANGE.contains(&GPIO_RANGE.start));
        assert!(!is_xip_addr(SPI2_RANGE.start));
    }
}

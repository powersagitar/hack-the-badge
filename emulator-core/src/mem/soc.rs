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

/// The mask ROM's read-only *data* aperture (DROM mask):
/// `SOC_DROM_MASK_LOW..SOC_DROM_MASK_HIGH` in ESP-IDF v5.5.3's
/// `components/soc/esp32c3/include/soc/soc.h`. Nothing backs it wholesale:
/// only the specific ROM data words `crate::rom` installs as
/// `crate::mem::bus::RomDataBlob`s are mapped, and every other address in
/// it stays catch-all (reads `0`, logged).
pub const DROM_MASK_RANGE: Range<u32> = 0x3FF0_0000..0x3FF2_0000;

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

/// The ESP32-C3's flash-cache MMU page size: always 64 KiB, not
/// configurable on this target (unlike some other ESP32 variants) --
/// `hal/esp32c3/include/hal/mmu_ll.h`'s `mmu_ll_get_page_size()` hardcodes
/// `return MMU_PAGE_64KB` with the comment "On esp32c3, MMU Page size is
/// always 64KB"; `hal/include/hal/mmu_types.h` defines `MMU_PAGE_64KB =
/// 0x10000`. `crate::mem::bus::FirmwareBus::from_segments` uses this to
/// widen each XIP (DROM/IROM) segment to its containing MMU page, matching
/// `bootloader_support/src/bootloader_utility.c`'s `set_cache_and_start_app()`
/// (page-aligns `load_addr` down, extends the mapped size by the leading
/// gap) composed with `hal/mmu_hal.c`'s `mmu_hal_map_region()` (rounds the
/// total mapped length up to a whole number of pages) -- see that
/// function's doc comment for why this matters (`cpu_start`'s app-image-
/// header check reads bytes that live in this leading gap).
pub const MMU_PAGE_SIZE: u32 = 0x1_0000;

/// `SOC_MMU_ENTRY_NUM` (`components/soc/esp32c3/include/soc/ext_mem_defs.h`,
/// ESP-IDF v5.5.3): the number of 32-bit entries in the MMU table.
pub const MMU_ENTRY_NUM: usize = 128;
/// `SOC_MMU_INVALID` (`ext_mem_defs.h`): bit 8 set = entry unmapped.
pub const MMU_INVALID: u32 = 1 << 8;
/// `SOC_MMU_VALID_VAL_MASK` (`ext_mem_defs.h`): the physical page number.
pub const MMU_VALID_VAL_MASK: u32 = 0xff;
/// `SOC_MMU_VADDR_MASK` (`ext_mem_defs.h`); `mmu_ll_get_entry_id()`
/// (`hal/esp32c3/include/hal/mmu_ll.h`) is `(vaddr & SOC_MMU_VADDR_MASK) >> 16`.
pub const MMU_VADDR_MASK: u32 = 0x7F_FFFF;
/// `MMU_LL_END_DROM_ENTRY_ID = SOC_MMU_ENTRY_NUM - 1` (`mmu_ll.h`): the
/// entry the bootloader maps to the DROM's first page "for app to find the
/// boot partition" (`bootloader_support/src/bootloader_utility.c`,
/// `set_cache_and_start_app`).
pub const MMU_DROM_END_ENTRY_ID: usize = MMU_ENTRY_NUM - 1;

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

/// SYSTEM peripheral registers (`DR_REG_SYSTEM_BASE = 0x600c_0000`,
/// `soc/reg_base.h` v5.5.3). Only the `SYSTEM_CPU_INTR_FROM_CPU_0..3_REG`
/// software-interrupt registers are modeled (see `crate::peripherals::system`);
/// every other offset in the page keeps the bus's logged catch-all behavior.
pub const SYSTEM_RANGE: Range<u32> = 0x600c_0000..0x600c_1000;

/// Interrupt-source numbers: the value `periph_interrupt_t`
/// (`components/soc/esp32c3/include/soc/interrupts.h`, v5.5.3) gives each
/// `ETS_*_INTR_SOURCE`, equal to that source's `INTERRUPT_CORE0_*_MAP_REG`
/// offset divided by 4 (`interrupt_core0_reg.h`: `SPI_INTR_2_MAP_REG` =
/// `0x04C`, `SYSTIMER_TARGET0_INT_MAP_REG` = `0x094`,
/// `DMA_CH0_INT_MAP_REG` = `0x0B0`, `CPU_INTR_FROM_CPU_0_MAP_REG` = `0x0C8`).
pub const SRC_SPI2: u32 = 19; // ETS_SPI2_INTR_SOURCE
pub const SRC_RMT: u32 = 28; // ETS_RMT_INTR_SOURCE (INTERRUPT_CORE0_RMT_INTR_MAP_REG = 0x070)
pub const SRC_I2C_EXT0: u32 = 29; // ETS_I2C_EXT0_INTR_SOURCE
pub const SRC_USB_SERIAL_JTAG: u32 = 26; // ETS_USB_SERIAL_JTAG_INTR_SOURCE (soc/esp32c3/include/soc/interrupts.h)
pub const SRC_SYSTIMER_TARGET0: u32 = 37; // ETS_SYSTIMER_TARGET0_INTR_SOURCE
pub const SRC_SYSTIMER_TARGET1: u32 = 38;
pub const SRC_SYSTIMER_TARGET2: u32 = 39;
pub const SRC_DMA_CH0: u32 = 44; // ETS_DMA_CH0_INTR_SOURCE
pub const SRC_DMA_CH1: u32 = 45;
pub const SRC_DMA_CH2: u32 = 46;
pub const SRC_FROM_CPU_INTR0: u32 = 50; // ETS_FROM_CPU_INTR0_SOURCE
pub const SRC_FROM_CPU_INTR1: u32 = 51;
pub const SRC_FROM_CPU_INTR2: u32 = 52;
pub const SRC_FROM_CPU_INTR3: u32 = 53;

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

/// USB-Serial-JTAG peripheral registers (`DR_REG_USB_SERIAL_JTAG_BASE`,
/// confirmed via ESP-IDF v5.5.3's
/// `components/soc/esp32c3/register/soc/reg_base.h`). This is the badge's
/// actual console transport (no external UART is wired out). Same
/// one-4KiB-page rationale as [`SYSTIMER_RANGE`] (the header's
/// highest-cited register this module models, `USB_SERIAL_JTAG_DATE_REG`,
/// is at `0x80`, comfortably inside one page). See
/// `crate::peripherals::usb_serial_jtag` for what's actually modeled
/// within it.
pub const USB_SERIAL_JTAG_RANGE: Range<u32> = 0x6004_3000..0x6004_4000;

/// TIMG0 (timer group 0) peripheral registers
/// (`DR_REG_TIMERGROUP0_BASE`, confirmed via ESP-IDF v5.5.3's
/// `components/soc/esp32c3/register/soc/reg_base.h`). One full 4 KiB page,
/// same rationale as [`SYSTIMER_RANGE`]. This is the timer group
/// `rtc_clk_cal_internal()` uses for RTC slow-clock calibration at boot
/// (`TIMG_RTCCALICFG*_REG`) — see `crate::peripherals::timg` for what's
/// actually modeled within it.
pub const TIMG0_RANGE: Range<u32> = 0x6001_F000..0x6002_0000;

/// TIMG1 (timer group 1) peripheral registers. Immediately follows
/// [`TIMG0_RANGE`] (`DR_REG_TIMERGROUP1_BASE == DR_REG_TIMERGROUP0_BASE +
/// 0x1000`, same header). Same one-4KiB-page rationale; see
/// `crate::peripherals::timg`.
pub const TIMG1_RANGE: Range<u32> = 0x6002_0000..0x6002_1000;

/// RTC_CNTL peripheral registers (`DR_REG_RTCCNTL_BASE`, confirmed via
/// ESP-IDF v5.5.3's `components/soc/esp32c3/register/soc/reg_base.h`). Same
/// one-4KiB-page rationale as [`SYSTIMER_RANGE`] (the header's
/// highest-cited `RTC_CNTL_*_REG` offset is `0x1FC`, comfortably inside one
/// page). This is the peripheral `rtc_cntl_ll_get_rtc_time()` reads at boot
/// (`RTC_CNTL_TIME_UPDATE_REG`/`TIME_LOW0_REG`/`TIME_HIGH0_REG`) — see
/// `crate::peripherals::rtc_cntl` for what's actually modeled within it.
/// Not to be confused with [`RTC_RANGE`] above (`0x5000_0000`), which is RTC
/// *slow memory* — a completely different address-space region from this
/// peripheral's MMIO registers.
pub const RTC_CNTL_RANGE: Range<u32> = 0x6000_8000..0x6000_9000;

/// SPI1 / `SPIMEM1` flash-controller registers (`DR_REG_SPI1_BASE`,
/// confirmed via ESP-IDF v5.5.3's
/// `components/soc/esp32c3/register/soc/reg_base.h`; also
/// `REG_SPI_MEM_BASE(1) == DR_REG_SPI0_BASE - 0x1000`, `soc/soc.h`). One 4 KiB
/// page: `spi_mem_reg.h`'s highest register, `SPI_MEM_DATE_REG`, is at
/// `+0x3FC`. The controller ESP-IDF's `esp_flash` driver issues flash
/// commands through; see `crate::peripherals::flash::Spimem1`.
pub const SPIMEM1_RANGE: Range<u32> = 0x6000_2000..0x6000_3000;

/// GDMA registers (`DR_REG_GDMA_BASE = 0x6003_F000`, confirmed via ESP-IDF
/// v5.5.3's `components/soc/esp32c3/register/soc/reg_base.h`). One 4 KiB
/// page: `gdma_reg.h`'s highest register, `GDMA_OUT_PERI_SEL_CH2_REG`, is
/// at `+0x280`. See `crate::peripherals::gdma`.
pub const GDMA_RANGE: Range<u32> = 0x6003_F000..0x6004_0000;

/// I2C0 controller registers (`DR_REG_I2C_EXT_BASE = 0x6001_3000`, ESP-IDF
/// v5.5.3 `components/soc/esp32c3/register/soc/reg_base.h`). One 4 KiB
/// page: `i2c_reg.h`'s highest address, `I2C_RXFIFO_START_ADDR_REG` RAM, ends
/// at `+0x200`. See `crate::peripherals::i2c`.
pub const I2C0_RANGE: Range<u32> = 0x6001_3000..0x6001_4000;

/// RMT registers and RAM (`DR_REG_RMT_BASE = 0x6001_6000`, ESP-IDF v5.5.3
/// `components/soc/esp32c3/register/soc/reg_base.h`; `RMTMEM = 0x6001_6400`,
/// `components/soc/esp32c3/ld/rmt.peripherals.ld`). One 4 KiB page (TRM
/// v1.4 table 3.3-3: `0x6001_6000..=0x6001_6FFF`); the 192-word RAM ends at
/// `+0x700`. See `crate::peripherals::rmt`.
pub const RMT_RANGE: Range<u32> = 0x6001_6000..0x6001_7000;

/// The flash MMU table: `DR_REG_MMU_TABLE = 0x600c5000`
/// (`components/soc/esp32c3/register/soc/reg_base.h`, ESP-IDF v5.5.3).
/// `SOC_MMU_ENTRY_NUM` (128) 32-bit entries occupy the first `0x200`
/// bytes; the rest of this 4 KiB block is not modeled (catch-all).
pub const MMU_TABLE_RANGE: Range<u32> = 0x600c_5000..0x600c_6000;

/// The data-bus flash-cache aperture the MMU translates:
/// `SOC_DRAM0_CACHE_ADDRESS_LOW..HIGH`
/// (`components/soc/esp32c3/include/soc/ext_mem_defs.h`). Narrower than
/// [`DROM_RANGE`]: addresses in `DROM_RANGE` past this end are not
/// cache-backed at all.
pub const DBUS_CACHE_RANGE: Range<u32> = 0x3C00_0000..0x3C80_0000;

/// The instruction-bus flash-cache aperture:
/// `SOC_IRAM0_CACHE_ADDRESS_LOW..HIGH` (same header). Shares the 128 MMU
/// entries with [`DBUS_CACHE_RANGE`].
pub const IBUS_CACHE_RANGE: Range<u32> = 0x4200_0000..0x4280_0000;

/// SYSCON, formerly APB_CTRL (`DR_REG_SYSCON_BASE` = `DR_REG_APB_CTRL_BASE`
/// = `0x6002_6000`, ESP-IDF v5.5.3
/// `components/soc/esp32c3/register/soc/reg_base.h:41-42`). Only the RNG
/// data register is modeled (`crate::peripherals::apb_ctrl`); every other
/// offset keeps the bus's logged catch-all behavior.
pub const APB_CTRL_RANGE: Range<u32> = 0x6002_6000..0x6002_7000;

/// The ESP32-C3 CPU's machine performance-counter CSRs, as the core's
/// chip-agnostic cycle counter (`crate::cpu::CycleCounter`). ESP-IDF v5.5.3
/// `components/riscv/include/riscv/rv_utils.h:41-43`: `CSR_PCER_MACHINE`
/// `0x7e0`, `CSR_PCMR_MACHINE` `0x7e1`, `CSR_PCCR_MACHINE` `0x7e2`;
/// `rv_utils_get_cycle_count()` / `rv_utils_set_cycle_count()` (same file,
/// lines 107-131) read and write `CSR_PCCR_MACHINE` in M-mode on targets
/// with `SOC_CPU_HAS_CSR_PC` (esp32c3 `soc_caps.h`). This is what
/// `esp_cpu_get_cycle_count()` returns, e.g. in `esp_random()`
/// (`components/esp_hw_support/hw_random.c`) and the early log timestamp.
///
/// PCER/PCMR are stored only: nothing in ESP-IDF v5.5.3 for the ESP32-C3,
/// the app or the ESP32-C3 ROM ELF writes them, so the firmware relies on
/// the counter running from reset, and no source to cite says otherwise.
/// The counter therefore always counts (Milestone 5 Task D-M5-3, ruling
/// R-T9-1).
pub const ESP32C3_CYCLE_COUNTER: crate::cpu::CycleCounterCsrs = crate::cpu::CycleCounterCsrs {
    counter: 0x7e2,
    controls: [0x7e0, 0x7e1],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmu_page_size_is_64kib() {
        assert_eq!(MMU_PAGE_SIZE, 0x1_0000);
    }

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

    #[test]
    fn usb_serial_jtag_range_is_disjoint_from_the_other_peripheral_ranges_and_xip_iram() {
        assert!(USB_SERIAL_JTAG_RANGE.contains(&0x6004_3000));
        assert!(!USB_SERIAL_JTAG_RANGE.contains(&0x6004_4000)); // exclusive end
        assert!(!SYSTIMER_RANGE.contains(&USB_SERIAL_JTAG_RANGE.start));
        assert!(!INTERRUPT_CORE0_RANGE.contains(&USB_SERIAL_JTAG_RANGE.start));
        assert!(!GPIO_RANGE.contains(&USB_SERIAL_JTAG_RANGE.start));
        assert!(!SPI2_RANGE.contains(&USB_SERIAL_JTAG_RANGE.start));
        assert!(!USB_SERIAL_JTAG_RANGE.contains(&SYSTIMER_RANGE.start));
        assert!(!USB_SERIAL_JTAG_RANGE.contains(&INTERRUPT_CORE0_RANGE.start));
        assert!(!USB_SERIAL_JTAG_RANGE.contains(&GPIO_RANGE.start));
        assert!(!USB_SERIAL_JTAG_RANGE.contains(&SPI2_RANGE.start));
        assert!(!is_xip_addr(USB_SERIAL_JTAG_RANGE.start));
    }

    #[test]
    fn timg0_and_timg1_ranges_are_disjoint_from_each_other_and_the_other_peripheral_ranges_and_xip_iram(
    ) {
        assert!(TIMG0_RANGE.contains(&0x6001_F000));
        assert!(!TIMG0_RANGE.contains(&0x6002_0000)); // exclusive end
        assert!(TIMG1_RANGE.contains(&0x6002_0000));
        assert!(!TIMG1_RANGE.contains(&0x6002_1000)); // exclusive end
        assert!(!TIMG0_RANGE.contains(&TIMG1_RANGE.start));
        assert!(!TIMG1_RANGE.contains(&TIMG0_RANGE.start));

        for other in [
            SYSTIMER_RANGE.start,
            INTERRUPT_CORE0_RANGE.start,
            GPIO_RANGE.start,
            SPI2_RANGE.start,
            USB_SERIAL_JTAG_RANGE.start,
        ] {
            assert!(!TIMG0_RANGE.contains(&other));
            assert!(!TIMG1_RANGE.contains(&other));
        }
        assert!(!SYSTIMER_RANGE.contains(&TIMG0_RANGE.start));
        assert!(!SYSTIMER_RANGE.contains(&TIMG1_RANGE.start));

        assert!(!is_xip_addr(TIMG0_RANGE.start));
        assert!(!is_xip_addr(TIMG1_RANGE.start));
    }

    #[test]
    fn rtc_cntl_range_is_disjoint_from_every_other_peripheral_range_and_xip_iram_and_rtc_slow_memory(
    ) {
        assert!(RTC_CNTL_RANGE.contains(&0x6000_8000));
        assert!(!RTC_CNTL_RANGE.contains(&0x6000_9000)); // exclusive end

        for other in [
            SYSTIMER_RANGE.start,
            INTERRUPT_CORE0_RANGE.start,
            GPIO_RANGE.start,
            SPI2_RANGE.start,
            USB_SERIAL_JTAG_RANGE.start,
            TIMG0_RANGE.start,
            TIMG1_RANGE.start,
        ] {
            assert!(!RTC_CNTL_RANGE.contains(&other));
        }
        assert!(!GPIO_RANGE.contains(&RTC_CNTL_RANGE.start));
        assert!(!RTC_RANGE.contains(&RTC_CNTL_RANGE.start));
        assert!(!RTC_CNTL_RANGE.contains(&RTC_RANGE.start));
        assert!(!is_xip_addr(RTC_CNTL_RANGE.start));
    }

    #[test]
    fn gdma_range_is_disjoint_from_every_other_peripheral_range_and_xip_iram() {
        assert!(GDMA_RANGE.contains(&0x6003_F000));
        assert!(GDMA_RANGE.contains(&(0x6003_F000 + 0x280))); // OUT_PERI_SEL_CH2
        assert!(!GDMA_RANGE.contains(&0x6004_0000)); // exclusive end
        for other in [
            SYSTIMER_RANGE,
            INTERRUPT_CORE0_RANGE,
            SYSTEM_RANGE,
            GPIO_RANGE,
            SPI2_RANGE,
            USB_SERIAL_JTAG_RANGE,
            TIMG0_RANGE,
            TIMG1_RANGE,
            RTC_CNTL_RANGE,
            SPIMEM1_RANGE,
        ] {
            assert!(!GDMA_RANGE.contains(&other.start));
            assert!(!other.contains(&GDMA_RANGE.start));
        }
        assert!(!is_xip_addr(GDMA_RANGE.start));
    }

    #[test]
    fn mmu_constants_match_esp_idf_v5_5_3() {
        assert_eq!(MMU_TABLE_RANGE.start, 0x600c_5000); // DR_REG_MMU_TABLE
        assert_eq!(MMU_ENTRY_NUM, 128); // SOC_MMU_ENTRY_NUM
        assert_eq!(MMU_INVALID, 1 << 8); // SOC_MMU_INVALID
        assert_eq!(MMU_VALID_VAL_MASK, 0xff); // SOC_MMU_VALID_VAL_MASK
        assert_eq!(MMU_VADDR_MASK, 0x7f_ffff); // SOC_MMU_VADDR_MASK
        assert_eq!(MMU_DROM_END_ENTRY_ID, 127); // MMU_LL_END_DROM_ENTRY_ID
        assert_eq!(DBUS_CACHE_RANGE, 0x3c00_0000..0x3c80_0000); // SOC_DRAM0_CACHE_ADDRESS_LOW/HIGH
        assert_eq!(IBUS_CACHE_RANGE, 0x4200_0000..0x4280_0000); // SOC_IRAM0_CACHE_ADDRESS_LOW/HIGH

        // One entry per 64 KiB page of either aperture.
        assert_eq!(
            (DBUS_CACHE_RANGE.end - DBUS_CACHE_RANGE.start) / MMU_PAGE_SIZE,
            MMU_ENTRY_NUM as u32
        );
        // The cache apertures sit inside the coarse XIP ranges.
        let (d, db) = (DROM_RANGE, DBUS_CACHE_RANGE);
        assert!(d.start <= db.start && db.end <= d.end);
        let (i, ib) = (IROM_RANGE, IBUS_CACHE_RANGE);
        assert!(i.start <= ib.start && ib.end <= i.end);
        // The MMU block does not overlap any other modeled peripheral.
        for r in [
            &SYSTEM_RANGE,
            &INTERRUPT_CORE0_RANGE,
            &GDMA_RANGE,
            &SPIMEM1_RANGE,
        ] {
            assert!(MMU_TABLE_RANGE.end <= r.start || r.end <= MMU_TABLE_RANGE.start);
        }
    }
}

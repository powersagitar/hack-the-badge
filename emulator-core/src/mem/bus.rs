//! [`FirmwareBus`]: the real ESP32-C3 memory map, backing Task 1's [`Bus`]
//! trait with an app image's parsed segments (see `crate::mem::image`).
//!
//! An ordered sequence of named address ranges, checked in this order on
//! every access:
//! 1. **XIP** (the flash-cache apertures, [`crate::mem::soc::DBUS_CACHE_RANGE`]
//!    and [`crate::mem::soc::IBUS_CACHE_RANGE`], 8 MiB each): every access
//!    translates through the flash MMU ([`FirmwareBus::mmu`]) to a physical
//!    flash page and reads [`FirmwareBus::flash_chip`] there (the physical
//!    page wraps modulo the 4 MiB chip). An access through an *invalid*
//!    entry reads `0` and is logged into [`FirmwareBus::unmapped_log`] (no
//!    cache-error interrupt is modeled); an instruction fetch there traps.
//!    Writes are silently dropped (real hardware: read-only flash cache).
//!    The DBUS aperture is readable but **never fetchable**; only IBUS
//!    through a valid entry is. `DROM_RANGE`/`IROM_RANGE` are wider than the
//!    apertures: addresses in them but outside both apertures are not XIP at
//!    all and fall through to the catch-all. The table is seeded at
//!    construction by replaying the 2nd-stage bootloader's mapping of the
//!    app image ([`FirmwareBus::from_segments`]); after that only the
//!    firmware's own writes to the table change it.
//! 1b. **Flash MMU table** ([`crate::mem::soc::MMU_TABLE_RANGE`]): routed to
//!    [`FirmwareBus::mmu`], same concrete-field ruling -- see
//!    `crate::peripherals::mmu`. Offsets `FlashMmu::handles` rejects are
//!    logged as unmapped. Checked after the RAM/ROM tiers like the other
//!    peripherals; XIP itself is checked first.
//! 2. **RAM-copied**: a real, mutable, per-segment `Vec<u8>` that the
//!    segment's bytes were copied into at boot. Reads/writes go straight to
//!    it.
//! 3. **ROM code** ([`RomCodeBlob`]s installed via
//!    [`FirmwareBus::map_rom_code`]): small, fixed, read-only, *executable*
//!    code blobs at fixed mask-ROM addresses (Milestone 3 Task D6) -- the
//!    guest-executed ROM routines that `crate::rom` can't HLE-stub
//!    atomically because they call back into firmware (ROM `qsort`). Reads
//!    and fetches return the blob's bytes; writes are dropped silently, like
//!    XIP flash. Only each blob's own bytes are mapped: every other ROM
//!    address stays unmapped, so its fetch still traps.
//!
//!    **ROM data** ([`RomDataBlob`]s installed via
//!    [`FirmwareBus::map_rom_data`], checked right after ROM code): the same
//!    kind of small, fixed, read-only blob, but for mask-ROM *data* tables
//!    and pointers the firmware reads (Milestone 3 Task D7: the ROM layout
//!    table behind `ets_rom_layout_p`). Reads return the blob's bytes and
//!    writes are dropped, but a ROM data blob is **never fetchable**:
//!    [`Bus::fetch16`] on it returns `None`, exactly as for unmapped space.
//!    ROM code and data blobs may not overlap each other.
//! 4. **SYSTIMER** ([`crate::mem::soc::SYSTIMER_RANGE`]): routed to
//!    [`FirmwareBus::systimer`], a concrete named field per this plan's
//!    pre-flight "no trait-object peripheral dispatch" ruling — see
//!    `crate::peripherals` and `crate::peripherals::systimer`. Then
//!    **SYSTEM** ([`crate::mem::soc::SYSTEM_RANGE`], Milestone 3 Task 4):
//!    only the four `SYSTEM_CPU_INTR_FROM_CPU_<n>_REG` software-interrupt
//!    registers (`crate::peripherals::system::System::handles`) are routed
//!    to [`FirmwareBus::system`]; every other SYSTEM offset falls through to
//!    the logged catch-all below, as before.
//! 5. **INTERRUPT_CORE0** ([`crate::mem::soc::INTERRUPT_CORE0_RANGE`]):
//!    routed to [`FirmwareBus::intc`], same ruling — see
//!    `crate::peripherals::intc`. One register
//!    (`CPU_INT_EIP_STATUS_REG`) needs the live asserted interrupt sources
//!    ([`FirmwareBus::pending_sources`]: SYSTIMER, SYSTEM, SPI2, GDMA and I2C0) to answer a
//!    read, which is exactly the cross-peripheral access the ruling
//!    anticipated: [`FirmwareBus::read_byte`] reads the concrete fields
//!    directly, no trait object involved.
//! 6. **GPIO** ([`crate::mem::soc::GPIO_RANGE`]): routed to
//!    [`FirmwareBus::gpio`], same ruling — see `crate::peripherals::gpio`
//!    for the register model and the emulated 74HC165 button shift
//!    register.
//! 7. **SPI2/GPSPI2** ([`crate::mem::soc::SPI2_RANGE`]): routed to
//!    [`FirmwareBus::spi`], same ruling — see `crate::peripherals::spi` for
//!    the register model and the ST7789 command/pixel-stream interpreter.
//!    A triggering write (one that sets `SPI_CMD_REG`'s `SPI_USR` bit)
//!    needs `gpio`'s live GPIO0 level (the D/C line) to know whether the
//!    transaction is a command or data — another cross-peripheral read
//!    the "no trait-object dispatch" ruling anticipated:
//!    [`FirmwareBus::write_byte`] reads `self.gpio.pin_level(0)` directly
//!    and hands it to [`crate::peripherals::spi::Spi::process_transaction`],
//!    no trait object involved. SPI2 also contributes its
//!    `TRANS_DONE` interrupt level to [`FirmwareBus::pending_sources`]
//!    (Milestone 3 Task 9). If `SPI_DMA_TX_ENA` is set and a GDMA TX
//!    channel is connected to SPI2, the triggering write instead pulls the
//!    transaction's bytes from that channel's descriptor link in RAM
//!    ([`FirmwareBus::gdma_pull`]) and hands them to
//!    [`crate::peripherals::spi::Spi::handle_dma_bytes`] (Milestone 3 Task
//!    10): a three-way direct field access (spi, gdma, RAM), same ruling.
//! 8. **USB-Serial-JTAG** ([`crate::mem::soc::USB_SERIAL_JTAG_RANGE`]):
//!    routed to [`FirmwareBus::usb_serial_jtag`], same ruling — see
//!    `crate::peripherals::usb_serial_jtag`. TX-byte writes also need a
//!    live mutable reference to [`FirmwareBus::console`] (the capped sink
//!    that firmware console output accumulates into); `FirmwareBus::write_byte`
//!    passes `&mut self.console` straight through, another instance of the
//!    "no trait object" ruling's direct concrete-field access.
//! 9. **TIMG0** ([`crate::mem::soc::TIMG0_RANGE`]): routed to
//!    [`FirmwareBus::timg0`], same ruling — see `crate::peripherals::timg`
//!    for the RTC slow-clock calibration model `rtc_clk_cal_internal()`
//!    polls at boot, plus inert MWDT watchdog storage.
//! 10. **TIMG1** ([`crate::mem::soc::TIMG1_RANGE`]): routed to
//!    [`FirmwareBus::timg1`], same peripheral model as TIMG0 (a second,
//!    independent instance) — same ruling.
//! 11. **RTC_CNTL** ([`crate::mem::soc::RTC_CNTL_RANGE`]): routed to
//!    [`FirmwareBus::rtc_cntl`], same ruling — see
//!    `crate::peripherals::rtc_cntl` for the RTC timer latch model
//!    `rtc_cntl_ll_get_rtc_time()` polls at boot. A triggering write (one
//!    that sets `TIME_UPDATE_REG`'s `TIME_UPDATE` bit) needs `systimer`'s
//!    monotonic tick count (`SysTimer::elapsed_ticks`) as its "elapsed time" source — another
//!    cross-peripheral read the "no trait-object dispatch" ruling
//!    anticipated: [`FirmwareBus::write_byte`] reads `self.systimer.elapsed_ticks()`
//!    directly and hands it to
//!    [`crate::peripherals::rtc_cntl::RtcCntl::write_byte`], no trait object
//!    involved. Like SYSTIMER/INTC, not-yet-modeled RTC_CNTL registers are
//!    still logged into [`FirmwareBus::unmapped_log`] via
//!    [`crate::peripherals::rtc_cntl::RtcCntl::handles`], even though the
//!    peripheral itself gives them real (if inert) storage — see that
//!    module's doc.
//! 12. **SPIMEM1** ([`crate::mem::soc::SPIMEM1_RANGE`], the SPI1 flash
//!    controller): routed to [`FirmwareBus::spimem1`], same ruling — see
//!    `crate::peripherals::flash`. A write that sets one of `SPI_MEM_CMD_REG`'s
//!    command bits (`SPI_MEM_USR` or a dedicated `SPI_MEM_FLASH_*` bit) runs
//!    a flash command against
//!    [`FirmwareBus::flash_chip`] (the emulated 4 MiB chip), another direct
//!    cross-field access. Offsets `Spimem1::handles` does not name are
//!    still logged into [`FirmwareBus::unmapped_log`], like RTC_CNTL's.
//!    `flash_chip` is the only flash store: XIP reads it through the MMU, so
//!    a flash program through SPIMEM1 is visible through XIP.
//! 13. **GDMA** ([`crate::mem::soc::GDMA_RANGE`]): routed to
//!    [`FirmwareBus::gdma`], same ruling — see `crate::peripherals::gdma`.
//!    Register offsets [`crate::peripherals::gdma::Gdma::handles`] does not
//!    name (reserved gaps, the unmodeled `OUT_DSCR*` pre-fetch registers)
//!    are still logged into [`FirmwareBus::unmapped_log`]. GDMA contributes
//!    its channels' `INT_RAW & INT_ENA` levels to
//!    [`FirmwareBus::pending_sources`].
//! 14. **I2C0** ([`crate::mem::soc::I2C0_RANGE`]): routed to
//!    [`FirmwareBus::i2c0`], same ruling — see `crate::peripherals::i2c`.
//!    Offsets [`crate::peripherals::i2c::I2c::handles`] does not name are
//!    logged into [`FirmwareBus::unmapped_log`]. Its `INT_STATUS != 0` level
//!    is `SRC_I2C_EXT0` in [`FirmwareBus::pending_sources`].
//! 15. **Catch-all**: any address covered by none of the above (every
//!    genuinely not-yet-modeled ESP32-C3 peripheral MMIO register, plus
//!    truly unmapped space). Reads return `0`, writes are dropped — this
//!    must never panic, for any address, since real firmware immediately
//!    starts probing peripheral registers that later tasks haven't built
//!    yet. Every such access is recorded into a small capped ring buffer
//!    ([`FirmwareBus::unmapped_log`]) as a debugging aid for later
//!    "why is boot stuck" investigation. Addresses inside the SYSTIMER/
//!    INTERRUPT_CORE0 ranges but not backed by a named register within
//!    those peripherals are *also* logged here — one tier up from this
//!    catch-all, but the same ring buffer — via each peripheral's
//!    `handles(offset)` pure function (`crate::peripherals::systimer::handles`,
//!    `crate::peripherals::intc::handles`): [`FirmwareBus::read_byte`]/
//!    [`FirmwareBus::write_byte`] call `record_unmapped` *in addition to*
//!    dispatching into the peripheral whenever `handles` says the offset
//!    isn't one of its modeled registers, so the peripheral still returns
//!    its own (0-reading, dropped-write) default for it, but the access
//!    shows up in the same "what's the firmware probing" log as truly
//!    unmapped space.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::peripherals::console::Console;
use crate::peripherals::flash::{EmulatedFlash, Spimem1, APP_OFFSET, FLASH_SIZE};
use crate::peripherals::gdma::{self, Gdma};
use crate::peripherals::gpio::Gpio;
use crate::peripherals::i2c::I2c;
use crate::peripherals::intc::{self, InterruptController};
use crate::peripherals::mmu::FlashMmu;
use crate::peripherals::rtc_cntl::RtcCntl;
use crate::peripherals::spi::Spi;
use crate::peripherals::system::System;
use crate::peripherals::systimer::SysTimer;
use crate::peripherals::timg::Timg;
use crate::peripherals::usb_serial_jtag::UsbSerialJtag;

use super::image::SegmentDescriptor;
use super::soc::{
    is_xip_addr, DBUS_CACHE_RANGE, DRAM_RANGE, GDMA_RANGE, GPIO_RANGE, I2C0_RANGE,
    IBUS_CACHE_RANGE, INTERRUPT_CORE0_RANGE, MMU_DROM_END_ENTRY_ID, MMU_PAGE_SIZE, MMU_TABLE_RANGE,
    MMU_VALID_VAL_MASK, RTC_CNTL_RANGE, SPI2_RANGE, SPIMEM1_RANGE, SRC_FROM_CPU_INTR0,
    SYSTEM_RANGE, SYSTIMER_RANGE, TIMG0_RANGE, TIMG1_RANGE, USB_SERIAL_JTAG_RANGE,
};
use super::Bus;

/// Max number of catch-all accesses [`FirmwareBus`] remembers (oldest
/// entries are dropped once this cap is hit) — a debugging aid, not
/// behavior real firmware depends on.
pub const UNMAPPED_LOG_CAPACITY: usize = 256;

/// One access that fell through to the catch-all region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnmappedAccess {
    pub addr: u32,
    pub is_write: bool,
}

/// A RAM-copied window: `[load_addr, load_addr + data.len())` is backed by a
/// real, mutable, growable-at-construction-time buffer.
struct RamRegion {
    load_addr: u32,
    data: Vec<u8>,
}

impl RamRegion {
    fn contains(&self, addr: u32) -> bool {
        let start = self.load_addr as u64;
        let end = start + self.data.len() as u64;
        (addr as u64) >= start && (addr as u64) < end
    }
}

/// A small, fixed, read-only, **executable** code blob mapped at a fixed
/// address: `[base, base + 4 * words.len())` reads (and fetches) `words` as
/// little-endian 32-bit instruction words. Writes are dropped, like XIP flash
/// (real mask ROM is read-only).
///
/// This is the generic, chip-agnostic half of "guest-executed ROM
/// routines" (Milestone 3 Task D6): some ROM functions can't be HLE-stubbed
/// atomically by `crate::cpu::rom_stubs` because they call back into
/// firmware code mid-flight (ROM libc `qsort` calls its `compar` argument),
/// so instead the CPU really executes a small hand-assembled body placed at
/// a fixed ROM address. Which addresses and which words is chip-specific data
/// and lives in `crate::rom` (see its module doc); this type only knows how
/// to back them on the bus. The words are `&'static` because every blob is a
/// compile-time constant (assembled with `crate::cpu::encode`'s `const fn`s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomCodeBlob {
    /// Address of `words[0]`. Must be 4-byte aligned.
    pub base: u32,
    pub words: &'static [u32],
}

impl RomCodeBlob {
    fn contains(&self, addr: u32) -> bool {
        rom_words_contain(self.base, self.words, addr)
    }

    fn byte_at(&self, addr: u32) -> u8 {
        rom_word_byte(self.base, self.words, addr)
    }
}

/// `true` if `addr` is one of the `4 * words.len()` bytes a ROM blob based
/// at `base` covers. Shared by [`RomCodeBlob`] and [`RomDataBlob`].
fn rom_words_contain(base: u32, words: &[u32], addr: u32) -> bool {
    let start = base as u64;
    let end = start + 4 * words.len() as u64;
    (addr as u64) >= start && (addr as u64) < end
}

/// The little-endian byte at `addr` of a ROM blob based at `base`
/// (`addr` must be inside it).
fn rom_word_byte(base: u32, words: &[u32], addr: u32) -> u8 {
    let offset = (addr - base) as usize;
    words[offset / 4].to_le_bytes()[offset % 4]
}

/// A small, fixed, read-only **data** blob mapped at a fixed address:
/// `[base, base + 4 * words.len())` reads `words` as little-endian 32-bit
/// words. Writes are dropped (real mask ROM is read-only). Unlike
/// [`RomCodeBlob`], it is **never executable**: an instruction fetch from it
/// traps exactly as it would from unmapped space, because on real silicon
/// these bytes are ROM *data* (tables and pointers the firmware reads), not
/// code, and a jump into them is a bug worth surfacing.
///
/// The generic, chip-agnostic half of "ROM data tables" (Milestone 3 Task
/// D7): which addresses and which words is chip-specific data that lives in
/// `crate::rom` (see its module doc), like [`RomCodeBlob`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomDataBlob {
    /// Address of `words[0]`. Must be 4-byte aligned.
    pub base: u32,
    pub words: &'static [u32],
}

impl RomDataBlob {
    fn contains(&self, addr: u32) -> bool {
        rom_words_contain(self.base, self.words, addr)
    }

    fn byte_at(&self, addr: u32) -> u8 {
        rom_word_byte(self.base, self.words, addr)
    }
}

/// The concrete [`Bus`] implementation used to boot a real ESP-IDF app
/// image. See the module-level docs for the ordered-sequence-of-named-
/// regions read/write dispatch (XIP, RAM, ROM code, then one concrete field
/// per peripheral, then a never-panic catch-all).
pub struct FirmwareBus {
    /// The flash MMU table (`crate::peripherals::mmu`): every DBUS/IBUS
    /// access translates through it to [`FirmwareBus::flash_chip`].
    pub mmu: FlashMmu,
    ram_regions: Vec<RamRegion>,
    /// Read-only executable ROM code blobs ([`RomCodeBlob`]), installed by
    /// [`FirmwareBus::map_rom_code`]. Empty unless a caller installs some
    /// (`crate::boot::boot_from_factory_image_with_rom_stubs` installs
    /// `crate::rom`'s ESP32-C3 set).
    rom_code: Vec<RomCodeBlob>,
    /// Read-only, non-executable ROM data blobs ([`RomDataBlob`]),
    /// installed by [`FirmwareBus::map_rom_data`]. Empty unless a caller
    /// installs some (`crate::boot::boot_from_factory_image_with_rom_stubs`
    /// installs `crate::rom`'s ESP32-C3 set).
    rom_data: Vec<RomDataBlob>,
    /// The SYSTIMER peripheral (`crate::peripherals::systimer`), a concrete
    /// named field per this plan's pre-flight design ruling — see the
    /// module doc.
    pub systimer: SysTimer,
    /// The `INTERRUPT_CORE0` interrupt matrix (`crate::peripherals::intc`),
    /// same ruling.
    pub intc: InterruptController,
    /// The SYSTEM peripheral (`crate::peripherals::system`), same ruling —
    /// only the `FROM_CPU_0..3` software-interrupt registers are modeled.
    pub system: System,
    /// The GPIO peripheral (`crate::peripherals::gpio`), same ruling —
    /// includes the emulated 74HC165 button shift register.
    pub gpio: Gpio,
    /// The SPI2/GPSPI2 peripheral (`crate::peripherals::spi`), same ruling —
    /// includes the ST7789 command/pixel-stream interpreter and
    /// reconstructed framebuffer.
    pub spi: Spi,
    /// The GDMA controller (`crate::peripherals::gdma`), same ruling -- its
    /// TX out-link feeds SPI2 when `SPI_DMA_TX_ENA` is set (see
    /// [`FirmwareBus::gdma_pull`]).
    pub gdma: Gdma,
    /// The I2C0 controller (`crate::peripherals::i2c`), same ruling: a
    /// master with no device on the bus, so every address phase NACKs.
    pub i2c0: I2c,
    /// The USB-Serial-JTAG peripheral (`crate::peripherals::usb_serial_jtag`),
    /// same ruling — the badge's actual console transport.
    pub usb_serial_jtag: UsbSerialJtag,
    /// TIMG0 (`crate::peripherals::timg`), same ruling — RTC slow-clock
    /// calibration plus inert MWDT watchdog storage.
    pub timg0: Timg,
    /// TIMG1 (`crate::peripherals::timg`), a second independent instance of
    /// the same peripheral model, same ruling.
    pub timg1: Timg,
    /// The RTC_CNTL peripheral (`crate::peripherals::rtc_cntl`), same
    /// ruling — the RTC timer latch (`TIME_UPDATE_REG`/`TIME_LOW0_REG`/
    /// `TIME_HIGH0_REG`) `rtc_cntl_ll_get_rtc_time()` polls at boot.
    pub rtc_cntl: RtcCntl,
    /// The SPI1 flash controller (`crate::peripherals::flash::Spimem1`),
    /// same ruling — runs flash commands against [`FirmwareBus::flash_chip`].
    pub spimem1: Spimem1,
    /// The emulated 4 MiB flash chip (`crate::peripherals::flash::EmulatedFlash`):
    /// blank except a synthesized partition table and the app image at
    /// `0x10000`. The only flash store; XIP reads it through `mmu`, and
    /// SPIMEM1 commands run against it.
    pub flash_chip: EmulatedFlash,
    /// Capped sink for everything the firmware prints
    /// (`crate::peripherals::console`), fed by
    /// [`FirmwareBus::usb_serial_jtag`]'s TX-byte writes.
    pub console: Console,
    /// Ring buffer of the most recent catch-all accesses, capped at
    /// [`UNMAPPED_LOG_CAPACITY`].
    unmapped_log: VecDeque<UnmappedAccess>,
}

impl FirmwareBus {
    /// Builds a `FirmwareBus` from a flash image and its already-parsed
    /// segment table (see `crate::mem::image::parse_image`). Categorizes
    /// each segment as XIP or RAM-copied purely by `load_addr`
    /// ([`is_xip_addr`]): RAM segments get their bytes copied into a fresh
    /// owned buffer here; XIP segments are not copied at all -- the image
    /// goes into [`FirmwareBus::flash_chip`] at [`APP_OFFSET`], and the XIP
    /// segments are made visible by seeding the flash MMU the way the real
    /// 2nd-stage bootloader does.
    ///
    /// ## MMU seeding
    ///
    /// ESP-IDF v5.5.3's `set_cache_and_start_app()`
    /// (`bootloader_support/src/bootloader_utility.c`) maps each XIP segment
    /// with `hal/mmu_hal.c`'s `mmu_hal_map_region()`: the vaddr and flash
    /// paddr are both rounded *down* to a 64 KiB page, the length is
    /// extended by the vaddr's page offset and rounded *up* to whole pages
    /// (`page_num = (len + page_size - 1) / page_size`), and one MMU entry
    /// per page is written, to consecutive physical pages. It also maps the
    /// DROM's first physical page at `MMU_DROM_END_ENTRY_ID` (entry 127) so
    /// the app can find its own image.
    /// [`FirmwareBus::seed_mmu_like_bootloader`] replays exactly that for
    /// each XIP segment, with `paddr = APP_OFFSET + file_offset`, written
    /// through the bus's own MMU-table write path.
    ///
    /// The bootloader's paddr is independent of the vaddr; this emulator
    /// derives it from `file_offset`, so a segment whose flash offset and
    /// `load_addr` disagree modulo 64 KiB cannot be mapped faithfully and is
    /// left unmapped (its fetches trap). Esptool-built images -- including
    /// `factory.bin` -- always agree.
    ///
    /// Mapping whole pages matters concretely: ESP-IDF's `cpu_start`
    /// (`components/esp_system/port/cpu_start.c`) reads the running app's own
    /// `esp_image_header_t` from `&_rodata_reserved_start -
    /// sizeof(esp_image_header_t) - sizeof(esp_image_segment_header_t)`, i.e.
    /// 32 bytes *before* the DROM segment's `load_addr` (`0x3c13_0020` in
    /// `factory.bin`), in the same page. Those bytes are simply the chip's
    /// bytes at `APP_OFFSET`, visible because the whole page is mapped.
    ///
    /// All arithmetic on segment-derived values is checked: a browser-supplied
    /// image must never be able to panic this constructor.
    pub fn from_segments(flash: Arc<[u8]>, segments: &[SegmentDescriptor]) -> Self {
        let mut ram_regions = Vec::new();

        for seg in segments {
            if is_xip_addr(seg.load_addr) {
                // Mapped by `seed_mmu_like_bootloader` below.
                continue;
            } else {
                // `seg.file_offset + seg.len` used to be unchecked here. It
                // was safe when this function's only caller was
                // `boot::boot_from_factory_image`'s pre-validated internal
                // path (`parse_image` already rejects a segment whose data
                // runs past the image), but `FirmwareEmulator::new`
                // (emulator-wasm) now reaches this constructor directly with
                // a browser-supplied image, so a malformed/adversarial
                // `SegmentDescriptor` must not be able to overflow this
                // addition (wraps on wasm32, where `usize` is 32 bits) and
                // panic on the resulting bad slice bound. Same contract this
                // bus already holds every *data* access to (see the module
                // doc's catch-all tier): malformed input degrades gracefully
                // rather than panicking. Skip the segment rather than
                // constructing a broken RAM region for it.
                match seg.file_offset.checked_add(seg.len) {
                    Some(end) if end <= flash.len() => {
                        let data = flash[seg.file_offset..end].to_vec();
                        ram_regions.push(RamRegion {
                            load_addr: seg.load_addr,
                            data,
                        });
                    }
                    _ => continue,
                }
            }
        }

        let flash_chip = EmulatedFlash::from_app_image(&flash);
        let mut bus = Self {
            mmu: FlashMmu::new(),
            ram_regions,
            rom_code: Vec::new(),
            rom_data: Vec::new(),
            systimer: SysTimer::new(),
            intc: InterruptController::new(),
            system: System::new(),
            gpio: Gpio::new(),
            spi: Spi::new(),
            gdma: Gdma::new(),
            i2c0: I2c::new(),
            usb_serial_jtag: UsbSerialJtag::new(),
            timg0: Timg::new(),
            timg1: Timg::new(),
            rtc_cntl: RtcCntl::new(),
            spimem1: Spimem1::new(),
            flash_chip,
            console: Console::new(),
            unmapped_log: VecDeque::with_capacity(UNMAPPED_LOG_CAPACITY),
        };
        bus.seed_mmu_like_bootloader(segments);
        bus
    }

    /// Replays the 2nd-stage bootloader's `set_cache_and_start_app()` MMU
    /// programming (ESP-IDF v5.5.3 `bootloader_support/src/bootloader_utility.c`;
    /// page rounding from `hal/mmu_hal.c`'s `mmu_hal_map_region`) for the
    /// app at [`APP_OFFSET`]. See [`FirmwareBus::from_segments`].
    fn seed_mmu_like_bootloader(&mut self, segments: &[SegmentDescriptor]) {
        let mut drom_end_mapped = false;
        for seg in segments.iter().filter(|s| is_xip_addr(s.load_addr)) {
            let Some(first_phys) = self.map_xip_segment(seg) else {
                continue;
            };
            if DBUS_CACHE_RANGE.contains(&seg.load_addr) && !drom_end_mapped {
                self.write_mmu_entry(MMU_DROM_END_ENTRY_ID, first_phys);
                drom_end_mapped = true;
            }
        }
    }

    /// Maps one XIP segment's pages; returns its first physical page, or
    /// `None` (nothing written) if the segment cannot be mapped faithfully.
    fn map_xip_segment(&mut self, seg: &SegmentDescriptor) -> Option<u32> {
        let aperture = [&DBUS_CACHE_RANGE, &IBUS_CACHE_RANGE]
            .into_iter()
            .find(|r| r.contains(&seg.load_addr))?;
        let paddr = APP_OFFSET.checked_add(u32::try_from(seg.file_offset).ok()?)?;
        if paddr % MMU_PAGE_SIZE != seg.load_addr % MMU_PAGE_SIZE {
            return None;
        }
        let end = seg.load_addr.checked_add(u32::try_from(seg.len).ok()?)?;
        if end > aperture.end {
            return None;
        }
        let first_page = seg.load_addr / MMU_PAGE_SIZE;
        let page_count = end.div_ceil(MMU_PAGE_SIZE) - first_page;
        let first_phys = paddr / MMU_PAGE_SIZE;
        let first_id = FlashMmu::entry_id(seg.load_addr);
        for i in 0..page_count {
            self.write_mmu_entry(first_id + i as usize, first_phys + i);
        }
        Some(first_phys)
    }

    /// `mmu_ll_write_entry`: `phys_page | ACCESS_FLASH (0) | VALID (0)`,
    /// written through the bus like the bootloader's store.
    fn write_mmu_entry(&mut self, id: usize, phys_page: u32) {
        let addr = MMU_TABLE_RANGE.start + 4 * id as u32;
        Bus::write32(self, addr, phys_page & MMU_VALID_VAL_MASK);
    }

    fn in_cache_aperture(addr: u32) -> bool {
        DBUS_CACHE_RANGE.contains(&addr) || IBUS_CACHE_RANGE.contains(&addr)
    }

    /// The OR of every peripheral's currently-asserted interrupt *source*
    /// levels (bit `n` = source number `n`, `crate::mem::soc::SRC_*`). A
    /// pure recomputation from live peripheral state -- nothing latched --
    /// so a source de-asserts the moment its peripheral clears it. Sources
    /// so far: SYSTIMER targets 0..2 (`SRC_SYSTIMER_TARGET0..2`, Task 5), the
    /// SYSTEM `FROM_CPU_0..3` software interrupts, SPI2 (`SRC_SPI2`, `SPI_TRANS_DONE_INT_ST`, Task 9), and
    /// GDMA channels 0..2 (`SRC_DMA_CH0..2`, `INT_RAW & INT_ENA`, Task 10),
    /// and I2C0 (`SRC_I2C_EXT0`, `INT_STATUS != 0`, Milestone 4 Task D-M4-1).
    pub fn pending_sources(&self) -> u64 {
        let mut p = self.systimer.pending_sources();
        p |= u64::from(self.system.pending_mask()) << SRC_FROM_CPU_INTR0;
        p |= self.spi.pending_sources();
        p |= self.gdma.pending_sources();
        p |= self.i2c0.pending_sources();
        p
    }

    /// The mask of CPU interrupt lines asserted to the core right now:
    /// [`InterruptController::poll`] over [`FirmwareBus::pending_sources`].
    /// A pure function of live peripheral state -- this is what
    /// `crate::boot::step_with_interrupts` samples into
    /// `Cpu::set_pending_interrupts` at the start of every step.
    pub fn asserted_lines(&self) -> u32 {
        self.intc.poll(self.pending_sources())
    }

    /// Advances [`FirmwareBus::systimer`]'s counter by one step's worth of
    /// ticks. Call exactly once per `Cpu::step()` (except a step spent
    /// waiting in `WFI`, which advances SYSTIMER by its own fast-forward
    /// instead) — see
    /// `crate::boot::step_with_interrupts`, which samples
    /// [`FirmwareBus::asserted_lines`] itself at the start of the next step,
    /// so this returns nothing.
    pub fn tick_peripherals(&mut self) {
        self.systimer.advance();
    }

    /// Adds a fresh, zero-initialized, real read/write RAM region
    /// `[load_addr, load_addr + len)` to the bus, distinct from (and not
    /// derived from) any image segment. Used by `crate::boot` to reserve a
    /// scratch stack/bss area in DRAM that the flash image itself carries
    /// no bytes for (uninitialized memory doesn't need to be stored in
    /// flash) — see `boot::boot_from_factory_image`'s doc comment.
    pub fn add_scratch_ram(&mut self, load_addr: u32, len: usize) {
        self.ram_regions.push(RamRegion {
            load_addr,
            data: vec![0u8; len],
        });
    }

    /// Maps a read-only, executable [`RomCodeBlob`] onto the bus (see that
    /// type's doc). Only the blob's own `4 * words.len()` bytes become
    /// mapped: every other address in the surrounding ROM aperture stays
    /// exactly as it was -- data reads fall through to the catch-all, and
    /// instruction fetches still trap.
    ///
    /// # Panics
    ///
    /// If `blob.base` is not 4-byte aligned, or the blob overlaps one
    /// already mapped. Both are programming errors in a compile-time
    /// constant table (`crate::rom`), never reachable from firmware or a
    /// browser-supplied image, so failing loudly beats a silently shadowed
    /// blob.
    pub fn map_rom_code(&mut self, blob: RomCodeBlob) {
        self.assert_rom_blob_fits("code", blob.base, blob.words);
        self.rom_code.push(blob);
    }

    /// Maps a read-only, **non-executable** [`RomDataBlob`] onto the bus
    /// (see that type's doc). Only the blob's own `4 * words.len()` bytes
    /// become readable; none of them ever becomes fetchable, and every other
    /// address in the surrounding ROM aperture stays catch-all.
    ///
    /// # Panics
    ///
    /// Same as [`FirmwareBus::map_rom_code`]: a misaligned `blob.base`, or
    /// an overlap with any ROM code *or* data blob already mapped.
    pub fn map_rom_data(&mut self, blob: RomDataBlob) {
        self.assert_rom_blob_fits("data", blob.base, blob.words);
        self.rom_data.push(blob);
    }

    /// The shared precondition of [`FirmwareBus::map_rom_code`] and
    /// [`FirmwareBus::map_rom_data`]: `base` is word-aligned, and the blob
    /// overlaps no ROM blob of either kind already mapped.
    fn assert_rom_blob_fits(&self, kind: &str, base: u32, words: &[u32]) {
        assert!(
            base.is_multiple_of(4),
            "ROM {kind} blob base {base:#x} is not word-aligned"
        );
        let end = base as u64 + 4 * words.len() as u64;
        let mapped = self
            .rom_code
            .iter()
            .map(|b| (b.base, b.words.len()))
            .chain(self.rom_data.iter().map(|b| (b.base, b.words.len())));
        for (other_base, other_len) in mapped {
            let other_end = other_base as u64 + 4 * other_len as u64;
            assert!(
                end <= other_base as u64 || other_end <= base as u64,
                "ROM {kind} blob at {base:#x} overlaps the one at {other_base:#x}"
            );
        }
    }

    /// The most recent catch-all accesses (oldest first), capped at
    /// [`UNMAPPED_LOG_CAPACITY`] entries. Empty in a run that never touched
    /// unmapped/not-yet-modeled address space.
    pub fn unmapped_log(&self) -> &VecDeque<UnmappedAccess> {
        &self.unmapped_log
    }

    fn record_unmapped(&mut self, addr: u32, is_write: bool) {
        if self.unmapped_log.len() >= UNMAPPED_LOG_CAPACITY {
            self.unmapped_log.pop_front();
        }
        self.unmapped_log
            .push_back(UnmappedAccess { addr, is_write });
    }
    fn read_byte(&mut self, addr: u32) -> u8 {
        if Self::in_cache_aperture(addr) {
            if let Some(paddr) = self.mmu.translate(addr) {
                return self.flash_chip.read(paddr % FLASH_SIZE as u32);
            }
            self.record_unmapped(addr, false);
            return 0;
        }
        if let Some(region) = self.ram_regions.iter().find(|r| r.contains(addr)) {
            let offset = (addr - region.load_addr) as usize;
            return region.data[offset];
        }
        if let Some(blob) = self.rom_code.iter().find(|b| b.contains(addr)) {
            return blob.byte_at(addr);
        }
        if let Some(blob) = self.rom_data.iter().find(|b| b.contains(addr)) {
            return blob.byte_at(addr);
        }
        if MMU_TABLE_RANGE.contains(&addr) {
            let offset = addr - MMU_TABLE_RANGE.start;
            if !FlashMmu::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.mmu.read_byte(offset);
        }
        if SYSTIMER_RANGE.contains(&addr) {
            let offset = addr - SYSTIMER_RANGE.start;
            if !SysTimer::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.systimer.read_byte(offset);
        }
        if SYSTEM_RANGE.contains(&addr) && System::handles(addr - SYSTEM_RANGE.start) {
            return self.system.read_byte(addr - SYSTEM_RANGE.start);
        }
        if INTERRUPT_CORE0_RANGE.contains(&addr) {
            let offset = addr - INTERRUPT_CORE0_RANGE.start;
            if !InterruptController::handles(offset) {
                self.record_unmapped(addr, false);
            }
            if offset & !0b11 == intc::CPU_INT_EIP_STATUS_REG {
                // Cross-peripheral read: needs systimer's live pending
                // state. Both fields are concrete on `self`, so this is
                // just direct field access — exactly what the pre-flight
                // "no trait-object peripheral dispatch" ruling anticipated.
                let word = self.intc.eip_status(self.pending_sources());
                return word.to_le_bytes()[(offset & 0b11) as usize];
            }
            return self.intc.read_byte(offset);
        }
        if GPIO_RANGE.contains(&addr) {
            return self.gpio.read_byte(addr - GPIO_RANGE.start);
        }
        if SPI2_RANGE.contains(&addr) {
            return self.spi.read_byte(addr - SPI2_RANGE.start);
        }
        if USB_SERIAL_JTAG_RANGE.contains(&addr) {
            return self
                .usb_serial_jtag
                .read_byte(addr - USB_SERIAL_JTAG_RANGE.start);
        }
        if TIMG0_RANGE.contains(&addr) {
            return self.timg0.read_byte(addr - TIMG0_RANGE.start);
        }
        if TIMG1_RANGE.contains(&addr) {
            return self.timg1.read_byte(addr - TIMG1_RANGE.start);
        }
        if RTC_CNTL_RANGE.contains(&addr) {
            let offset = addr - RTC_CNTL_RANGE.start;
            if !RtcCntl::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.rtc_cntl.read_byte(offset);
        }
        if SPIMEM1_RANGE.contains(&addr) {
            let offset = addr - SPIMEM1_RANGE.start;
            if !Spimem1::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.spimem1.read_byte(offset);
        }
        if GDMA_RANGE.contains(&addr) {
            let offset = addr - GDMA_RANGE.start;
            if !Gdma::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.gdma.read_byte(offset);
        }
        if I2C0_RANGE.contains(&addr) {
            let offset = addr - I2C0_RANGE.start;
            if !I2c::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.i2c0.read_byte(offset);
        }
        self.record_unmapped(addr, false);
        0
    }

    fn write_byte(&mut self, addr: u32, val: u8) {
        if Self::in_cache_aperture(addr) {
            // Flash is read-only at runtime on real hardware; drop silently.
            return;
        }
        if let Some(region) = self.ram_regions.iter_mut().find(|r| r.contains(addr)) {
            let offset = (addr - region.load_addr) as usize;
            region.data[offset] = val;
            return;
        }
        if self.rom_code.iter().any(|b| b.contains(addr))
            || self.rom_data.iter().any(|b| b.contains(addr))
        {
            // Mask ROM is read-only on real hardware; drop silently, same as
            // the XIP tier above.
            return;
        }
        if MMU_TABLE_RANGE.contains(&addr) {
            let offset = addr - MMU_TABLE_RANGE.start;
            if !FlashMmu::handles(offset) {
                self.record_unmapped(addr, true);
            }
            self.mmu.write_byte(offset, val);
            return;
        }
        if SYSTIMER_RANGE.contains(&addr) {
            let offset = addr - SYSTIMER_RANGE.start;
            if !SysTimer::handles(offset) {
                self.record_unmapped(addr, true);
            }
            self.systimer.write_byte(offset, val);
            return;
        }
        if SYSTEM_RANGE.contains(&addr) && System::handles(addr - SYSTEM_RANGE.start) {
            self.system.write_byte(addr - SYSTEM_RANGE.start, val);
            return;
        }
        if INTERRUPT_CORE0_RANGE.contains(&addr) {
            let offset = addr - INTERRUPT_CORE0_RANGE.start;
            if !InterruptController::handles(offset) {
                self.record_unmapped(addr, true);
            }
            if offset & !0b11 != intc::CPU_INT_EIP_STATUS_REG {
                // CPU_INT_EIP_STATUS_REG is read-only; writes to it drop,
                // matching real hardware.
                self.intc.write_byte(offset, val);
            }
            return;
        }
        if GPIO_RANGE.contains(&addr) {
            self.gpio.write_byte(addr - GPIO_RANGE.start, val);
            return;
        }
        if SPI2_RANGE.contains(&addr) {
            let triggered = self.spi.write_byte(addr - SPI2_RANGE.start, val);
            if triggered {
                // Cross-peripheral read: needs gpio's live GPIO0 level (the
                // D/C line) at exactly this moment, not a stale snapshot.
                // Both fields are concrete on `self`, so this is just
                // direct field access — the same pattern the
                // INTERRUPT_CORE0 tier above already uses for its own
                // cross-peripheral read.
                let dc_low = !self.gpio.pin_level(0);
                // DMA transmit (Task 10): with SPI_DMA_TX_ENA set and a
                // GDMA TX channel connected to SPI2, the bytes come from
                // that channel's descriptor link, not W0..W15.
                match self.gdma.channel_for_spi2() {
                    Some(ch) if self.spi.dma_tx_enabled() => {
                        let bytes = self.gdma_pull(ch, self.spi.tx_byte_len());
                        self.spi.handle_dma_bytes(dc_low, &bytes);
                    }
                    _ => self.spi.process_transaction(dc_low),
                }
            }
            return;
        }
        if USB_SERIAL_JTAG_RANGE.contains(&addr) {
            self.usb_serial_jtag.write_byte(
                addr - USB_SERIAL_JTAG_RANGE.start,
                val,
                &mut self.console,
            );
            return;
        }
        if TIMG0_RANGE.contains(&addr) {
            self.timg0.write_byte(addr - TIMG0_RANGE.start, val);
            return;
        }
        if TIMG1_RANGE.contains(&addr) {
            self.timg1.write_byte(addr - TIMG1_RANGE.start, val);
            return;
        }
        if RTC_CNTL_RANGE.contains(&addr) {
            let offset = addr - RTC_CNTL_RANGE.start;
            if !RtcCntl::handles(offset) {
                self.record_unmapped(addr, true);
            }
            // Cross-peripheral read: the RTC timer's "elapsed time" source
            // is systimer's monotonic `elapsed_ticks()` (no free-running
            // clock of its own to model; not a unit counter, which firmware
            // can stop or reload) -- both fields are concrete on `self`, so this
            // is just direct field access, the same pattern the
            // INTERRUPT_CORE0/SPI2 tiers above already use. See
            // `crate::peripherals::rtc_cntl`'s module doc.
            let elapsed_steps = self.systimer.elapsed_ticks();
            self.rtc_cntl.write_byte(offset, val, elapsed_steps);
            return;
        }
        if SPIMEM1_RANGE.contains(&addr) {
            let offset = addr - SPIMEM1_RANGE.start;
            if !Spimem1::handles(offset) {
                self.record_unmapped(addr, true);
            }
            // Cross-peripheral access: a triggering CMD write runs a flash
            // command against the chip, a separate concrete field. Direct
            // disjoint field borrows, no trait object -- same ruling as the
            // SPI2/RTC_CNTL tiers above.
            self.spimem1.write_byte(offset, val, &mut self.flash_chip);
            return;
        }
        if GDMA_RANGE.contains(&addr) {
            let offset = addr - GDMA_RANGE.start;
            if !Gdma::handles(offset) {
                self.record_unmapped(addr, true);
            }
            self.gdma.write_byte(offset, val);
            return;
        }
        if I2C0_RANGE.contains(&addr) {
            let offset = addr - I2C0_RANGE.start;
            if !I2c::handles(offset) {
                self.record_unmapped(addr, true);
            }
            self.i2c0.write_byte(offset, val);
            return;
        }
        self.record_unmapped(addr, true);
    }

    /// Walks GDMA TX channel `ch`'s out-link from its current descriptor and
    /// returns up to `max_len` bytes of transmit data: the bus-level half of
    /// `crate::peripherals::gdma` (the walk reads RAM, which only the bus
    /// can). Descriptor layout is `dma_descriptor_t` (ESP-IDF v5.5.3
    /// `components/hal/include/hal/dma_types.h`): word 0 `length[23:12]`,
    /// `suc_eof` bit 30, `owner` bit 31; word 1 the buffer; word 2 `next`
    /// (`0` ends the link). Per descriptor:
    /// - If `OUT_CHECK_OWNER` is set and `owner` is 0 (CPU), raise
    ///   `OUT_DSCR_ERR` and stop the channel ("owner error",
    ///   `gdma_reg.h`'s `OUT_DSCR_ERR` doc).
    /// - Send `length` bytes of the buffer, resuming at the channel's
    ///   offset if an earlier transaction ended mid-descriptor. If
    ///   `max_len` runs out first, stop there and remember the offset.
    /// - A descriptor fully sent: if `OUT_AUTO_WRBACK` is set, clear its
    ///   `owner` bit in RAM (`gdma_reg.h`: "automatic outlink-writeback
    ///   when all the data in TX buffer has been transmitted"); raise
    ///   `OUT_DONE`; if it has `suc_eof`, raise `OUT_EOF` and
    ///   `OUT_TOTAL_EOF` and latch `OUT_EOF_DES_ADDR` (this descriptor) and
    ///   `OUT_EOF_BFR_DES_ADDR` (the last one the channel sent before it, 0
    ///   if none since `OUT_RST`; the driver resets before every transfer). Follow `next`; at `0` the channel goes
    ///   idle (`PARK`).
    ///
    /// A `RESTART` since the last pull first re-reads the `next` field of
    /// the last descriptor sent, if the link had ended there. The walk is
    /// bounded (at most `max_len + 1` descriptors), so a cyclic link of
    /// empty descriptors cannot hang the host.
    ///
    /// **The walk only touches internal SRAM** (`DRAM_RANGE`, `SOC_DRAM_LOW..
    /// SOC_DRAM_HIGH`, and only where a RAM region actually backs it): a
    /// descriptor (`desc..desc+12`) or a non-empty buffer (`buf..buf+length`)
    /// anywhere else raises `OUT_DSCR_ERR` and stops the channel, sending
    /// nothing from that descriptor. `gdma_reg.h` (v5.5.3) documents
    /// `OUT_DSCR_ERR` (bit 6) as "detecting transmit descriptor error,
    /// including owner error, the second and third word error of transmit
    /// descriptor" -- word 2 is the buffer pointer, word 3 `next`. Exactly
    /// which addresses the silicon rejects is not in the header; restricting
    /// to the SRAM DMA-capable descriptors and buffers must live in
    /// (`esp_ptr_dma_capable`) is this model's reconstruction. It is also
    /// load-bearing for the never-panic invariant: descriptors and buffers
    /// are read and written through a RAM-only accessor, never through full
    /// bus dispatch, so a `next` aimed at an MMIO register (e.g. SPI2's own
    /// `SPI_CMD_REG`, whose write-back would re-trigger `SPI_USR` and recurse
    /// into this function) cannot reach a peripheral.
    ///
    /// `ch` outside `0..gdma::NUM_CHANNELS` returns an empty `Vec` (this is
    /// `pub`; never panic).
    pub fn gdma_pull(&mut self, ch: usize, max_len: usize) -> Vec<u8> {
        if ch >= gdma::NUM_CHANNELS {
            return Vec::new();
        }
        let mut st = self.gdma.out_link_state(ch);
        let check_owner = self.gdma.out_check_owner(ch);
        let wrback = self.gdma.out_auto_wrback(ch);
        let mut out = Vec::with_capacity(max_len);
        let mut raised = 0u32;
        if st.restart_pending {
            st.restart_pending = false;
            if st.cursor == 0 && st.last_desc != 0 {
                // `last_desc` was validated when it was sent.
                st.cursor = self.dma_read32(st.last_desc.wrapping_add(8)).unwrap_or(0);
            }
        }
        let mut walked = 0usize;
        while st.active && st.cursor != 0 && out.len() < max_len && walked <= max_len {
            walked += 1;
            let desc = st.cursor;
            let Some(_) = self.dma_sram_offset(desc, 12) else {
                raised |= gdma::OUT_DSCR_ERR; // descriptor outside SRAM
                st.active = false;
                break;
            };
            let w0 = self.dma_read32(desc).unwrap_or(0);
            if check_owner && w0 & gdma::DESC_OWNER == 0 {
                raised |= gdma::OUT_DSCR_ERR;
                st.active = false;
                break;
            }
            let buf = self.dma_read32(desc.wrapping_add(4)).unwrap_or(0);
            let next = self.dma_read32(desc.wrapping_add(8)).unwrap_or(0);
            let length = (w0 >> 12) & 0xFFF;
            let buf_loc = if length == 0 {
                None
            } else {
                match self.dma_sram_offset(buf, length) {
                    Some(loc) => Some(loc),
                    None => {
                        raised |= gdma::OUT_DSCR_ERR; // buffer outside SRAM
                        st.active = false;
                        break;
                    }
                }
            };
            let remaining = length.saturating_sub(st.offset) as usize;
            let take = remaining.min(max_len - out.len());
            if let Some((r, off)) = buf_loc {
                let start = off + st.offset as usize;
                out.extend_from_slice(&self.ram_regions[r].data[start..start + take]);
            }
            st.offset += take as u32;
            if st.offset < length {
                break; // the SPI transaction ended mid-descriptor
            }
            if wrback {
                self.dma_write32(desc, w0 & !gdma::DESC_OWNER);
            }
            raised |= gdma::OUT_DONE;
            if w0 & gdma::DESC_SUC_EOF != 0 {
                raised |= gdma::OUT_EOF | gdma::OUT_TOTAL_EOF;
                self.gdma.record_out_eof(ch, desc, st.last_desc);
            }
            st.last_desc = desc;
            st.cursor = next;
            st.offset = 0;
            if next == 0 {
                st.active = false;
            }
        }
        self.gdma.set_out_link_state(ch, st);
        self.gdma.raise_out(ch, raised);
        out
    }

    /// GDMA's view of memory (see [`FirmwareBus::gdma_pull`]): if
    /// `addr..addr+len` lies inside internal SRAM (`DRAM_RANGE`) *and* inside
    /// one backing RAM region, that region's index and the byte offset of
    /// `addr` in it; otherwise `None`. Never dispatches to a peripheral. A
    /// span straddling two adjacent RAM regions is rejected too (a
    /// simplification; the real firmware's descriptors and buffers never
    /// do -- `boots_to_first_real_frame` is unchanged).
    fn dma_sram_offset(&self, addr: u32, len: u32) -> Option<(usize, usize)> {
        let start = addr as u64;
        let end = start + len as u64;
        if start < DRAM_RANGE.start as u64 || end > DRAM_RANGE.end as u64 {
            return None;
        }
        self.ram_regions.iter().enumerate().find_map(|(i, r)| {
            let r_start = r.load_addr as u64;
            let r_end = r_start + r.data.len() as u64;
            (start >= r_start && end <= r_end).then(|| (i, (start - r_start) as usize))
        })
    }

    /// RAM-only little-endian word read for the GDMA walk.
    fn dma_read32(&self, addr: u32) -> Option<u32> {
        let (r, off) = self.dma_sram_offset(addr, 4)?;
        let d = &self.ram_regions[r].data[off..off + 4];
        Some(u32::from_le_bytes([d[0], d[1], d[2], d[3]]))
    }

    /// RAM-only little-endian word write for the GDMA walk (dropped outside
    /// SRAM; callers have already validated the address).
    fn dma_write32(&mut self, addr: u32, val: u32) {
        if let Some((r, off)) = self.dma_sram_offset(addr, 4) {
            self.ram_regions[r].data[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }
    }

    /// `true` if `addr` falls inside an XIP, RAM-copied or ROM-code region — i.e. is
    /// "genuinely executable" per [`Bus::fetch16`]'s contract. ROM *data*
    /// blobs ([`RomDataBlob`]) are deliberately not checked here, so they are
    /// never fetchable. Reuses the
    /// same region-membership checks [`FirmwareBus::read_byte`]/
    /// [`FirmwareBus::write_byte`] use, rather than duplicating them.
    fn is_mapped(&self, addr: u32) -> bool {
        (IBUS_CACHE_RANGE.contains(&addr) && self.mmu.translate(addr).is_some())
            || self.ram_regions.iter().any(|r| r.contains(addr))
            || self.rom_code.iter().any(|b| b.contains(addr))
    }
}

impl Bus for FirmwareBus {
    fn read8(&mut self, addr: u32) -> u8 {
        self.read_byte(addr)
    }

    fn read16(&mut self, addr: u32) -> u16 {
        let b0 = self.read_byte(addr);
        let b1 = self.read_byte(addr.wrapping_add(1));
        u16::from_le_bytes([b0, b1])
    }

    fn read32(&mut self, addr: u32) -> u32 {
        let mut buf = [0u8; 4];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.read_byte(addr.wrapping_add(i as u32));
        }
        u32::from_le_bytes(buf)
    }

    fn write8(&mut self, addr: u32, val: u8) {
        self.write_byte(addr, val);
    }

    fn write16(&mut self, addr: u32, val: u16) {
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            self.write_byte(addr.wrapping_add(i as u32), *b);
        }
    }

    fn write32(&mut self, addr: u32, val: u32) {
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            self.write_byte(addr.wrapping_add(i as u32), *b);
        }
    }

    fn fetch16(&mut self, addr: u32) -> Option<u16> {
        let hi_addr = addr.wrapping_add(1);
        if self.is_mapped(addr) && self.is_mapped(hi_addr) {
            Some(self.read16(addr))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::soc::{MMU_DROM_END_ENTRY_ID, MMU_ENTRY_NUM, MMU_INVALID};

    /// Builds a bus from `(load_addr, bytes)` segments laid out the way an
    /// esptool image is: each XIP segment's bytes start at a file offset
    /// with the same low 16 bits as its `load_addr` (padding with `0xFF`),
    /// so `APP_OFFSET + file_offset` and `load_addr` agree mod 64 KiB and
    /// the bootloader-style MMU seeding maps it.
    fn bus_with(segments: Vec<(u32, Vec<u8>)>) -> FirmwareBus {
        let mut flash = Vec::new();
        let mut descriptors = Vec::new();
        for (load_addr, data) in &segments {
            if is_xip_addr(*load_addr) {
                let want = (*load_addr % MMU_PAGE_SIZE) as usize;
                while flash.len() % MMU_PAGE_SIZE as usize != want {
                    flash.push(0xFF);
                }
            }
            let file_offset = flash.len();
            flash.extend_from_slice(data);
            descriptors.push(SegmentDescriptor {
                load_addr: *load_addr,
                file_offset,
                len: data.len(),
            });
        }
        FirmwareBus::from_segments(Arc::from(flash.into_boxed_slice()), &descriptors)
    }

    #[test]
    fn xip_region_reads_from_flash_bytes_and_drops_writes() {
        let mut bus = bus_with(vec![(0x3c000000, vec![0xDE, 0xAD, 0xBE, 0xEF])]);
        assert_eq!(bus.read8(0x3c000000), 0xDE);
        assert_eq!(bus.read32(0x3c000000), 0xEFBEADDE);

        bus.write32(0x3c000000, 0x11223344);
        assert_eq!(
            bus.read32(0x3c000000),
            0xEFBEADDE,
            "write to XIP/flash-mapped range must be a silent no-op"
        );
    }

    #[test]
    fn irom_range_is_also_xip() {
        let mut bus = bus_with(vec![(0x42000020, vec![0x01, 0x02])]);
        assert_eq!(bus.read8(0x42000020), 0x01);
        bus.write8(0x42000020, 0xff);
        assert_eq!(bus.read8(0x42000020), 0x01, "IROM write must be dropped");
    }

    #[test]
    fn seeding_maps_each_xip_segment_like_the_bootloader() {
        // factory.bin's shape: DROM at 0x3c13_0020, IROM at 0x4200_0020.
        let drom = vec![0xA1; 0x30];
        let irom = vec![0xB2; 0x30];
        let bus = bus_with(vec![(0x3c13_0020, drom), (0x4200_0020, irom)]);
        // DROM: file offset 0x20 -> paddr 0x10020 -> page 1 at entry 19.
        assert_eq!(bus.mmu.entry(19), 1);
        // IROM: padded to file offset 0x10020 -> paddr 0x20020 -> page 2 at entry 0.
        assert_eq!(bus.mmu.entry(0), 2);
        // Entry 127 -> the DROM's first physical page.
        assert_eq!(bus.mmu.entry(MMU_DROM_END_ENTRY_ID), 1);
        // Everything else stays invalid.
        assert_eq!(bus.mmu.entry(1), MMU_INVALID);
        assert_eq!(bus.mmu.entry(20), MMU_INVALID);
    }

    #[test]
    fn leading_page_bytes_before_load_addr_read_from_the_chip() {
        // Task D5's cpu_start case: the image header sits 0x20 bytes before
        // the DROM load_addr, in the same page. Through the MMU those bytes
        // are simply the chip's bytes at APP_OFFSET.
        let mut bus = bus_with(vec![(0x3c13_0020, vec![0x5A; 4])]);
        // bus_with pads the first 0x20 bytes of the image with 0xFF.
        assert_eq!(bus.read8(0x3c13_0000), 0xFF);
        assert_eq!(bus.read8(0x3c13_0020), 0x5A);
        // Past the image: blank flash, not catch-all 0.
        assert_eq!(bus.read8(0x3c13_ff00), 0xFF);
        // The entry-127 alias reads the same page.
        assert_eq!(bus.read8(0x3c7f_0020), 0x5A);
    }

    #[test]
    fn invalid_entry_reads_zero_logged_and_fetch_traps() {
        let mut bus = bus_with(vec![(0x4200_0000, vec![0x13, 0x00, 0x00, 0x00])]);
        assert_eq!(bus.read32(0x4210_0000), 0, "entry 16 is invalid");
        assert!(bus
            .unmapped_log()
            .iter()
            .any(|a| a.addr == 0x4210_0000 && !a.is_write));
        assert_eq!(bus.fetch16(0x4210_0000), None);
        assert_eq!(bus.fetch16(0x4200_0000), Some(0x0013));
    }

    #[test]
    fn dbus_is_readable_but_never_fetchable() {
        let mut bus = bus_with(vec![(0x3c00_0000, vec![0x13, 0x00, 0x00, 0x00])]);
        assert_eq!(bus.read16(0x3c00_0000), 0x0013);
        assert_eq!(bus.fetch16(0x3c00_0000), None);
    }

    #[test]
    fn mmu_table_writes_through_the_bus_take_effect() {
        let mut bus = bus_with(vec![]);
        // Map entry 39 to physical page 0 (spi_flash_mmap's window in M3's stall):
        // the synthesized partition table at 0x8000 becomes visible at 0x3c27_8000.
        bus.write32(MMU_TABLE_RANGE.start + 39 * 4, 0);
        assert_eq!(
            bus.read16(0x3c27_8000),
            0x50AA,
            "ESP_PARTITION_MAGIC, little-endian"
        );
        assert_eq!(bus.read32(MMU_TABLE_RANGE.start + 39 * 4), 0, "reads back");
        // Past the table, inside the block: logged catch-all.
        bus.write32(MMU_TABLE_RANGE.start + 0x200, 1);
        assert!(bus
            .unmapped_log()
            .iter()
            .any(|a| a.addr == MMU_TABLE_RANGE.start + 0x200));
    }

    #[test]
    fn remapping_an_entry_redirects_reads_immediately() {
        let mut bus = bus_with(vec![(0x3c00_0000, vec![0x11; 4])]);
        assert_eq!(bus.read8(0x3c00_0000), 0x11);
        bus.write32(MMU_TABLE_RANGE.start, 0); // entry 0 -> page 0 (partition-table page)
        assert_eq!(bus.read16(0x3c00_8000), 0x50AA);
        bus.write32(MMU_TABLE_RANGE.start, MMU_INVALID);
        assert_eq!(bus.read8(0x3c00_0000), 0, "unmapped now");
    }

    #[test]
    fn spimem1_flash_program_is_visible_through_xip() {
        let mut bus = bus_with(vec![]);
        bus.write32(MMU_TABLE_RANGE.start + 43 * 4, 0x2b); // entry 43 -> page 0x2b (storage partition start 0x2b0000)
        assert_eq!(bus.read8(0x3c2b_0000), 0xFF, "blank");
        bus.flash_chip.program(0x2b_0000, &[0x00]);
        assert_eq!(bus.read8(0x3c2b_0000), 0x00);
    }

    #[test]
    fn physical_pages_past_the_chip_wrap() {
        let mut bus = bus_with(vec![]);
        // Page 0x40 is 4 MiB: wraps to page 0 on a 4 MiB chip.
        bus.write32(MMU_TABLE_RANGE.start + 5 * 4, 0x40);
        assert_eq!(bus.read16(0x3c05_8000), 0x50AA);
    }

    #[test]
    fn addresses_past_the_8mib_cache_apertures_do_not_alias_mmu_entries() {
        let mut bus = bus_with(vec![
            (0x3c00_0000, vec![0x77; 4]),
            (0x4200_0000, vec![0x13, 0, 0, 0]),
        ]);
        assert_eq!(
            bus.read8(0x3c80_0000),
            0,
            "0x3c80_0000 & 0x7fffff would alias entry 0"
        );
        assert!(bus.unmapped_log().iter().any(|a| a.addr == 0x3c80_0000));
        assert_eq!(bus.fetch16(0x4280_0000), None);
    }

    #[test]
    fn misaligned_xip_segment_is_left_unmapped() {
        // load_addr low bits 0x20, but file offset 0 -> paddr 0x10000 (low bits 0).
        let flash: Arc<[u8]> = Arc::from(vec![0x13u8, 0, 0, 0].into_boxed_slice());
        let seg = SegmentDescriptor {
            load_addr: 0x4200_0020,
            file_offset: 0,
            len: 4,
        };
        let mut bus = FirmwareBus::from_segments(flash, &[seg]);
        assert_eq!(bus.mmu.entry(0), MMU_INVALID);
        assert_eq!(
            bus.fetch16(0x4200_0020),
            None,
            "traps loudly, never runs wrong bytes"
        );
    }

    #[test]
    fn adversarial_xip_segments_do_not_panic_and_map_nothing_wrong() {
        let flash: Arc<[u8]> = Arc::from(vec![0u8; 16].into_boxed_slice());
        let cases = [
            SegmentDescriptor {
                load_addr: 0x3c00_0000,
                file_offset: usize::MAX,
                len: 4,
            },
            SegmentDescriptor {
                load_addr: 0x3c00_0000,
                file_offset: 0,
                len: usize::MAX,
            },
            // Straddles the end of the DBUS aperture.
            SegmentDescriptor {
                load_addr: 0x3c7f_0000,
                file_offset: 0,
                len: 0x2_0000,
            },
            // In DROM_RANGE but past the cache aperture.
            SegmentDescriptor {
                load_addr: 0x3d00_0000,
                file_offset: 0,
                len: 4,
            },
            SegmentDescriptor {
                load_addr: 0xffff_fff0,
                file_offset: 0,
                len: 0x100,
            },
        ];
        for seg in cases {
            let bus = FirmwareBus::from_segments(flash.clone(), &[seg]);
            for id in 0..MMU_ENTRY_NUM {
                assert_eq!(bus.mmu.entry(id), MMU_INVALID, "{seg:?} entry {id}");
            }
        }
    }

    #[test]
    fn ram_region_is_mutable() {
        let mut bus = bus_with(vec![(0x3fc99c00, vec![0u8; 8])]);
        assert_eq!(bus.read32(0x3fc99c00), 0);
        bus.write32(0x3fc99c00, 0xCAFEBABE);
        assert_eq!(bus.read32(0x3fc99c00), 0xCAFEBABE);

        // IRAM
        let mut bus2 = bus_with(vec![(0x40380000, vec![0u8; 8])]);
        bus2.write16(0x40380004, 0xBEEF);
        assert_eq!(bus2.read16(0x40380004), 0xBEEF);

        // RTC
        let mut bus3 = bus_with(vec![(0x50000000, vec![0u8; 8])]);
        bus3.write8(0x50000003, 0x7f);
        assert_eq!(bus3.read8(0x50000003), 0x7f);
    }

    #[test]
    fn unmapped_addresses_never_panic_and_read_as_zero() {
        let mut bus = bus_with(vec![(0x3c000000, vec![0x11, 0x22])]);
        // Arbitrary address that is neither XIP nor any RAM region.
        assert_eq!(bus.read8(0x1234_5678), 0);
        assert_eq!(bus.read16(0x1234_5678), 0);
        assert_eq!(bus.read32(0x1234_5678), 0);
        bus.write32(0x1234_5678, 0xffff_ffff); // must not panic
        assert_eq!(bus.read32(0x1234_5678), 0, "unmapped write must be dropped");

        // Boundary addresses that could tempt an off-by-one/overflow bug.
        assert_eq!(bus.read8(0), 0);
        assert_eq!(bus.read32(u32::MAX - 3), 0);
        bus.write32(u32::MAX - 3, 0x1); // must not panic (near-top-of-address-space)
    }

    #[test]
    fn from_segments_does_not_panic_on_an_offset_plus_len_overflow() {
        // Regression test for Fix 3: a malformed/adversarial segment
        // descriptor whose file_offset + len overflows must not panic --
        // from_segments must skip it (or otherwise handle it gracefully)
        // instead of computing a bad slice bound.
        let flash: Arc<[u8]> = Arc::from(vec![0xAAu8; 16].into_boxed_slice());
        let segments = [SegmentDescriptor {
            load_addr: 0x3fc80000, // a RAM-copied (non-XIP) address
            file_offset: usize::MAX - 3,
            len: 100, // file_offset + len overflows usize
        }];

        // Must not panic.
        let bus = FirmwareBus::from_segments(flash, &segments);

        // And the malformed segment must not have been silently accepted as
        // a real, readable RAM region either.
        assert!(!bus.is_mapped(0x3fc80000));
    }

    #[test]
    fn from_segments_does_not_panic_when_offset_plus_len_exceeds_the_image_without_overflowing() {
        // A non-overflowing but still out-of-range segment (offset+len is a
        // valid usize, but exceeds the actual flash buffer) must likewise be
        // skipped rather than panicking on an out-of-bounds slice.
        let flash: Arc<[u8]> = Arc::from(vec![0xAAu8; 16].into_boxed_slice());
        let segments = [SegmentDescriptor {
            load_addr: 0x3fc80000,
            file_offset: 10,
            len: 1000, // 10 + 1000 = 1010, way past flash.len() == 16
        }];

        let bus = FirmwareBus::from_segments(flash, &segments);
        assert!(!bus.is_mapped(0x3fc80000));
    }

    #[test]
    fn fetching_zeroed_but_mapped_memory_traps_instead_of_running_forever() {
        // Regression test for Fix 2: a stray jump into a mapped-but-zeroed
        // region (e.g. uninitialized .bss/scratch RAM, which this bus backs
        // with real read/write storage per `boot::add_scratch_ram`) used to
        // decode `0x0000` as a valid C.ADDI4SPN no-op and just keep
        // executing/advancing pc forever. It must now trap as an illegal
        // instruction on the very first fetch.
        let mut bus = bus_with(vec![(0x3fc80000, vec![0u8; 64])]);
        let mut cpu = crate::cpu::Cpu::new();
        cpu.csr.mtvec = 0x9000;
        cpu.regs.pc = 0x3fc80000;

        let info = cpu.step(&mut bus);

        assert!(
            info.trap_taken,
            "fetching zeroed mapped memory must trap, not silently execute"
        );
        assert_eq!(
            cpu.csr.mcause,
            crate::cpu::exception_code::ILLEGAL_INSTRUCTION,
            "0x0000 is a reserved RVC encoding, not a valid instruction"
        );
        assert_eq!(cpu.regs.pc, 0x9000, "pc must redirect to mtvec");
    }

    // ---- Task D6: read-only executable ROM code blobs ----

    static TWO_WORDS: [u32; 2] = [0x0000_0013 /* nop */, 0x0000_8067 /* ret */];

    #[test]
    fn rom_code_blob_is_readable_fetchable_and_read_only() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_code(RomCodeBlob {
            base: 0x4000_1000,
            words: &TWO_WORDS,
        });

        assert_eq!(bus.read32(0x4000_1000), 0x0000_0013);
        assert_eq!(bus.read32(0x4000_1004), 0x0000_8067);
        assert_eq!(bus.read8(0x4000_1005), 0x80);
        assert_eq!(bus.fetch16(0x4000_1000), Some(0x0013));
        assert_eq!(bus.fetch16(0x4000_1006), Some(0x0000));

        bus.write32(0x4000_1000, 0xdead_beef);
        assert_eq!(
            bus.read32(0x4000_1000),
            0x0000_0013,
            "ROM code must be read-only"
        );
        assert!(
            bus.unmapped_log().is_empty(),
            "blob accesses are mapped, not catch-all traffic"
        );
    }

    #[test]
    fn rom_code_blob_maps_only_its_own_bytes() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_code(RomCodeBlob {
            base: 0x4000_1000,
            words: &TWO_WORDS,
        });

        // One halfword either side: still unexecutable, still a catch-all
        // data miss -- the surrounding ROM aperture is unchanged.
        assert_eq!(bus.fetch16(0x4000_0ffe), None);
        assert_eq!(bus.fetch16(0x4000_1008), None);
        assert_eq!(bus.read32(0x4000_1008), 0);
        assert!(bus.unmapped_log().iter().any(|a| a.addr == 0x4000_1008));
    }

    #[test]
    fn a_cpu_executes_a_rom_code_blob() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_code(RomCodeBlob {
            base: 0x4000_1000,
            words: &TWO_WORDS,
        });
        let mut cpu = crate::cpu::Cpu::new();
        cpu.regs.pc = 0x4000_1000;
        cpu.regs.write(1, 0x1234_5678);
        assert!(!cpu.step(&mut bus).trap_taken);
        assert!(!cpu.step(&mut bus).trap_taken);
        assert_eq!(cpu.regs.pc, 0x1234_5678, "the blob's `ret` returned to ra");
    }

    #[test]
    #[should_panic(expected = "overlaps")]
    fn overlapping_rom_code_blobs_are_rejected() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_code(RomCodeBlob {
            base: 0x4000_1000,
            words: &TWO_WORDS,
        });
        bus.map_rom_code(RomCodeBlob {
            base: 0x4000_1004,
            words: &TWO_WORDS,
        });
    }

    // ---- Task D7: read-only, non-executable ROM data blobs ----

    #[test]
    fn rom_data_blob_is_readable_and_read_only() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_data(RomDataBlob {
            base: 0x3ff1_0000,
            words: &TWO_WORDS,
        });

        assert_eq!(bus.read32(0x3ff1_0000), 0x0000_0013);
        assert_eq!(bus.read32(0x3ff1_0004), 0x0000_8067);
        assert_eq!(bus.read8(0x3ff1_0005), 0x80);

        bus.write32(0x3ff1_0000, 0xdead_beef);
        assert_eq!(
            bus.read32(0x3ff1_0000),
            0x0000_0013,
            "ROM data must be read-only"
        );
        assert!(
            bus.unmapped_log().is_empty(),
            "blob accesses are mapped, not catch-all traffic"
        );
    }

    #[test]
    fn rom_data_blob_is_never_fetchable() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_data(RomDataBlob {
            base: 0x3ff1_0000,
            words: &TWO_WORDS,
        });
        // Its words *are* valid instructions (nop; ret), so a fetch that
        // wrongly succeeded would execute them -- it must not.
        for addr in (0x3ff1_0000..0x3ff1_0008).step_by(2) {
            assert_eq!(bus.fetch16(addr), None, "{addr:#x} must not be fetchable");
        }

        let mut cpu = crate::cpu::Cpu::new();
        cpu.csr.mtvec = 0x9000;
        cpu.regs.pc = 0x3ff1_0000;
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken, "jumping into ROM data must trap");
        assert_eq!(
            cpu.csr.mcause,
            crate::cpu::exception_code::INSTRUCTION_ACCESS_FAULT
        );
        assert_eq!(cpu.csr.mtval, 0x3ff1_0000);
    }

    #[test]
    fn rom_data_blob_maps_only_its_own_bytes() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_data(RomDataBlob {
            base: 0x3ff1_0000,
            words: &TWO_WORDS,
        });

        assert_eq!(bus.read32(0x3ff0_fffc), 0);
        assert_eq!(bus.read32(0x3ff1_0008), 0);
        assert!(bus.unmapped_log().iter().any(|a| a.addr == 0x3ff0_fffc));
        assert!(bus.unmapped_log().iter().any(|a| a.addr == 0x3ff1_0008));
    }

    #[test]
    #[should_panic(expected = "overlaps")]
    fn overlapping_rom_data_blobs_are_rejected() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_data(RomDataBlob {
            base: 0x3ff1_0000,
            words: &TWO_WORDS,
        });
        bus.map_rom_data(RomDataBlob {
            base: 0x3ff1_0004,
            words: &TWO_WORDS,
        });
    }

    #[test]
    #[should_panic(expected = "overlaps")]
    fn a_rom_data_blob_overlapping_a_rom_code_blob_is_rejected() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_code(RomCodeBlob {
            base: 0x4000_1000,
            words: &TWO_WORDS,
        });
        bus.map_rom_data(RomDataBlob {
            base: 0x4000_1004,
            words: &TWO_WORDS,
        });
    }

    #[test]
    #[should_panic(expected = "not word-aligned")]
    fn a_misaligned_rom_data_blob_is_rejected() {
        let mut bus = bus_with(vec![]);
        bus.map_rom_data(RomDataBlob {
            base: 0x3ff1_0002,
            words: &TWO_WORDS,
        });
    }

    #[test]
    fn usb_serial_jtag_tx_writes_reach_the_console() {
        let mut bus = bus_with(vec![]);
        bus.write32(0x6004_3000, b'Z' as u32);
        assert_eq!(bus.console.bytes(), b"Z");
    }

    #[test]
    fn timg0_and_timg1_are_independent_and_reachable_through_the_bus() {
        let mut bus = bus_with(vec![]);
        // RTCCALICFG_REG (+0x68) on TIMG0: arm CLK_SEL=0, MAX=10, START.
        bus.write32(0x6001_F000 + 0x68, (10u32 << 16) | (1 << 31));
        assert_ne!(bus.read32(0x6001_F000 + 0x68) & (1 << 15), 0);
        // TIMG1 must be a completely separate, still-unstarted instance.
        assert_eq!(bus.read32(0x6002_0000 + 0x68) & (1 << 15), 0);
    }

    #[test]
    fn rtc_cntl_time_update_reachable_through_the_bus_and_advances_with_systimer() {
        let mut bus = bus_with(vec![]);
        // Advance the live SYSTIMER counter (the RTC timer's "elapsed time"
        // source -- see `crate::peripherals::rtc_cntl`'s module doc) well
        // past the ~294-step quotient threshold before triggering.
        for _ in 0..1_000_000 {
            bus.systimer.advance();
        }
        // TIME_UPDATE_REG (+0x0c): set bit 31 (RTC_CNTL_TIME_UPDATE).
        bus.write32(0x6000_8000 + 0x0c, 1 << 31);
        let low0 = bus.read32(0x6000_8000 + 0x10);
        let high0 = bus.read32(0x6000_8000 + 0x14);
        assert_ne!(
            (low0, high0),
            (0, 0),
            "expected a non-zero RTC timer latch after 1,000,000 systimer ticks"
        );
    }

    #[test]
    fn unhandled_rtc_cntl_offsets_are_logged_but_still_backed_by_storage() {
        let mut bus = bus_with(vec![]);
        // RTC_CNTL_OPTIONS0_REG (+0x00): not one of the three named
        // registers `RtcCntl::handles` reports.
        bus.write32(0x6000_8000, 0xDEAD_BEEF);
        assert_eq!(
            bus.read32(0x6000_8000),
            0xDEAD_BEEF,
            "still real storage, not a hard catch-all"
        );
        let log: Vec<_> = bus
            .unmapped_log()
            .iter()
            .map(|a| (a.addr & !3, a.is_write))
            .collect();
        assert!(log.contains(&(0x6000_8000, true)));
        assert!(log.contains(&(0x6000_8000, false)));
    }

    #[test]
    fn unhandled_systimer_and_intc_offsets_are_logged() {
        let mut bus = bus_with(vec![]);
        bus.write32(0x6002_3000 + 0x0FC, 1); // not a modeled SYSTIMER register
        bus.read32(0x600c_2000 + 0x800); // not a modeled INTC register
        let log: Vec<_> = bus
            .unmapped_log()
            .iter()
            .map(|a| (a.addr & !3, a.is_write))
            .collect();
        assert!(log.contains(&(0x6002_30FC, true)));
        assert!(log.contains(&(0x600c_2800, false)));
    }

    #[test]
    fn handled_systimer_offsets_are_not_logged() {
        let mut bus = bus_with(vec![]);
        bus.read32(0x6002_3000); // CONF_REG
        assert!(bus.unmapped_log().is_empty());
    }

    #[test]
    fn write_only_systimer_registers_read_as_zero_and_are_not_logged() {
        // COMP0_LOAD_REG (0x50) and INT_CLR_REG (0x6c) are write-only
        // trigger/clear registers -- `SysTimer::handles` counts both as
        // modeled (see its doc), so a *read* of either must both read back
        // 0 (no readable storage behind a WT/WTC bit) and not show up in
        // the unmapped-access log.
        let mut bus = bus_with(vec![]);
        assert_eq!(bus.read32(0x6002_3000 + 0x50), 0);
        assert_eq!(bus.read32(0x6002_3000 + 0x6c), 0);
        assert!(bus.unmapped_log().is_empty());
    }

    #[test]
    fn unmapped_log_records_accesses_and_caps_at_capacity() {
        let mut bus = bus_with(vec![]);
        for addr in 0..(UNMAPPED_LOG_CAPACITY as u32 + 10) {
            bus.read8(addr);
        }
        assert_eq!(bus.unmapped_log().len(), UNMAPPED_LOG_CAPACITY);
        // Oldest entries (addr 0..10) should have been evicted; the log
        // should now start at addr 10.
        assert_eq!(bus.unmapped_log().front().unwrap().addr, 10);
    }

    // ---- Milestone 3 Task 8: SPIMEM1 flash controller ----

    /// `memspi_host_read_id_hs`'s RDID through `spi_flash_hal_common_command`,
    /// as whole-word bus writes at `DR_REG_SPI1_BASE` (see
    /// `crate::peripherals::flash`'s tests for the per-LL-call breakdown).
    #[test]
    fn spimem1_rdid_through_the_bus_reads_the_jedec_id_and_completes() {
        let mut bus = bus_with(vec![]);
        let base = 0x6000_2000u32;
        bus.write32(base + 0x20, 0x7000_009F); // USER2: bitlen 7, cmd 0x9F
        bus.write32(base + 0x1C, 0xFC00_0000); // USER1: addr bitlen 0x3F
        bus.write32(base + 0x04, 0); // ADDR
        bus.write32(base + 0x28, 23); // MISO_DLEN: 24 bits
        bus.write32(base + 0x18, 0x9000_0000); // USER: usr_command | usr_miso
        let cmd = bus.read32(base);
        bus.write32(base, cmd | 0x4_0000); // CMD |= SPI_MEM_USR

        assert_eq!(bus.read32(base), 0, "SPI_MEM_USR must self-clear");
        assert_eq!(bus.read32(base + 0x58) & 0x00FF_FFFF, 0x0016_4046);
        assert_eq!(bus.spimem1.transaction_count(), 1);
        assert!(
            bus.unmapped_log().is_empty(),
            "named SPIMEM1 registers are not catch-all traffic"
        );
    }

    #[test]
    fn spimem1_unnamed_offsets_are_logged() {
        let mut bus = bus_with(vec![]);
        bus.read32(0x6000_23FC); // SPI_MEM_DATE_REG: not modeled
        assert!(bus
            .unmapped_log()
            .iter()
            .any(|a| a.addr == 0x6000_23FC && !a.is_write));
    }
    /// I2C0 (`0x6001_3000`) is routed to `FirmwareBus::i2c0`: an
    /// `i2c_master_probe` of an absent address raises NACK and asserts
    /// `SRC_I2C_EXT0`; the gap at `+0x88` stays logged.
    #[test]
    fn i2c0_probe_through_the_bus_nacks_and_asserts_its_source() {
        use crate::mem::soc::SRC_I2C_EXT0;
        let mut bus = bus_with(vec![]);
        let base = 0x6001_3000;
        bus.write32(base + 0x04, 0x20B | (1 << 4)); // CTR: MS_MODE
        bus.write32(base + 0x28, 0x5A8); // INT_ENA: I2C_LL_MASTER_EVENT_INTR
        bus.write32(base + 0x58, 6 << 11); // COMD0: RSTART
        bus.write32(base + 0x1C, 0x19 << 1); // DATA: address byte
        bus.write32(base + 0x5C, (1 << 11) | (1 << 8) | 1); // COMD1: WRITE 1, ack check
        bus.write32(base + 0x60, 2 << 11); // COMD2: STOP
        bus.write32(base + 0x04, 0x20B | (1 << 4) | (1 << 5)); // TRANS_START
        assert_eq!(bus.read32(base + 0x2C), (1 << 10) | (1 << 7)); // NACK | COMPLETE
        assert_ne!(bus.pending_sources() & (1u64 << SRC_I2C_EXT0), 0);
        bus.write32(base + 0x24, 0x5A8); // INT_CLR
        assert_eq!(bus.pending_sources() & (1u64 << SRC_I2C_EXT0), 0);
        assert!(!bus
            .unmapped_log()
            .iter()
            .any(|a| I2C0_RANGE.contains(&a.addr)));
        bus.read32(base + 0x88);
        assert!(bus.unmapped_log().iter().any(|a| a.addr == base + 0x88));
    }
}

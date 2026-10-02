//! [`FirmwareBus`]: the real ESP32-C3 memory map, backing Task 1's [`Bus`]
//! trait with an app image's parsed segments (see `crate::mem::image`).
//!
//! An ordered sequence of named address ranges, checked in this order on
//! every access:
//! 1. **XIP** (flash-mapped, [`crate::mem::soc::is_xip_addr`]): read directly
//!    out of the original flash image bytes at `file_offset + (addr -
//!    load_addr)`. Writes are silently dropped (real hardware: read-only
//!    flash cache). Each XIP segment's *mapped window* is wider than its
//!    own `[load_addr, load_addr + len)` -- see
//!    [`FirmwareBus::from_segments`]'s doc for why (page-granular flash-
//!    cache MMU mapping, matching what the real 2nd-stage bootloader's
//!    `set_cache_and_start_app()` programs; Task D5). A byte inside that
//!    mapped window but past the actual `flash` buffer's own length reads
//!    `0xFF` (Task D5 fix round 1, M2) -- real NOR flash's erased state,
//!    not the catch-all tier's `0`.
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
//!    ([`FirmwareBus::pending_sources`]: SYSTIMER and SYSTEM) to answer a
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
//!    no trait object involved.
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
//!    live unit0 counter as its "elapsed time" source — another
//!    cross-peripheral read the "no trait-object dispatch" ruling
//!    anticipated: [`FirmwareBus::write_byte`] reads `self.systimer.counter()`
//!    directly and hands it to
//!    [`crate::peripherals::rtc_cntl::RtcCntl::write_byte`], no trait object
//!    involved. Like SYSTIMER/INTC, not-yet-modeled RTC_CNTL registers are
//!    still logged into [`FirmwareBus::unmapped_log`] via
//!    [`crate::peripherals::rtc_cntl::RtcCntl::handles`], even though the
//!    peripheral itself gives them real (if inert) storage — see that
//!    module's doc.
//! 12. **SPIMEM1** ([`crate::mem::soc::SPIMEM1_RANGE`], the SPI1 flash
//!    controller): routed to [`FirmwareBus::spimem1`], same ruling — see
//!    `crate::peripherals::flash`. A write that sets `SPI_MEM_CMD_REG`'s
//!    `SPI_MEM_USR` bit runs a flash command against
//!    [`FirmwareBus::flash_chip`] (the emulated 4 MiB chip), another direct
//!    cross-field access. Offsets `Spimem1::handles` does not name are
//!    still logged into [`FirmwareBus::unmapped_log`], like RTC_CNTL's.
//!    `flash_chip` and the XIP tier's `flash` buffer are separate for now:
//!    XIP still reads the app image through Task D5's page-granular
//!    mapping, so a flash write through SPIMEM1 is not visible through XIP
//!    (no observed code path needs that yet; the flash MMU sub-unit would
//!    unify them).
//! 13. **Catch-all**: any address covered by none of the above (every
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
use crate::peripherals::flash::{EmulatedFlash, Spimem1};
use crate::peripherals::gpio::Gpio;
use crate::peripherals::intc::{self, InterruptController};
use crate::peripherals::rtc_cntl::RtcCntl;
use crate::peripherals::spi::Spi;
use crate::peripherals::system::System;
use crate::peripherals::systimer::SysTimer;
use crate::peripherals::timg::Timg;
use crate::peripherals::usb_serial_jtag::UsbSerialJtag;

use super::image::SegmentDescriptor;
use super::soc::{
    is_xip_addr, GPIO_RANGE, INTERRUPT_CORE0_RANGE, MMU_PAGE_SIZE, RTC_CNTL_RANGE, SPI2_RANGE,
    SPIMEM1_RANGE, SRC_FROM_CPU_INTR0, SRC_SYSTIMER_TARGET0, SYSTEM_RANGE, SYSTIMER_RANGE,
    TIMG0_RANGE, TIMG1_RANGE, USB_SERIAL_JTAG_RANGE,
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

/// A flash-mapped (XIP) window: `[load_addr, load_addr + len)` reads
/// straight out of `flash[file_offset + (addr - load_addr)]`.
struct XipRegion {
    load_addr: u32,
    len: u32,
    file_offset: usize,
}

impl XipRegion {
    fn contains(&self, addr: u32) -> bool {
        let start = self.load_addr as u64;
        let end = start + self.len as u64;
        (addr as u64) >= start && (addr as u64) < end
    }
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
    /// The original flash image bytes, kept once and shared (never copied)
    /// — XIP regions index directly into this.
    flash: Arc<[u8]>,
    xip_regions: Vec<XipRegion>,
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
    /// The emulated 4 MiB flash chip behind SPIMEM1
    /// (`crate::peripherals::flash::EmulatedFlash`): blank except a
    /// synthesized partition table and the app image at `0x10000`. Distinct
    /// from `flash` above, which only backs XIP reads (see the module doc's
    /// SPIMEM1 tier for why the two are not unified yet).
    pub flash_chip: EmulatedFlash,
    /// Capped sink for everything the firmware prints
    /// (`crate::peripherals::console`), fed by
    /// [`FirmwareBus::usb_serial_jtag`]'s TX-byte writes.
    pub console: Console,
    /// Ring buffer of the most recent catch-all accesses, capped at
    /// [`UNMAPPED_LOG_CAPACITY`].
    unmapped_log: VecDeque<UnmappedAccess>,
}

/// Computes the page-aligned XIP mapping window a real ESP32-C3 2nd-stage
/// bootloader's flash-cache MMU setup would expose for one segment --
/// see [`FirmwareBus::from_segments`]'s doc comment for the citation chain
/// (`bootloader_support/src/bootloader_utility.c`'s
/// `set_cache_and_start_app()` composed with `hal/mmu_hal.c`'s
/// `mmu_hal_map_region()`). Returns `(aligned_load_addr,
/// aligned_file_offset, aligned_len)`: the same segment, widened to the
/// full 64 KiB page(s) it lives in, both before `load_addr` (where
/// `cpu_start`'s header check actually reads from -- Task D5) and after
/// `load_addr + len` (real hardware's MMU maps whole pages, not partial
/// ones).
///
/// The leading extension is clamped to the bytes actually available before
/// `file_offset` in the source buffer: real hardware's physical flash chip
/// always has *some* bytes there (it's one contiguous chip), but a
/// synthetic or browser-supplied partial image sometimes won't.
///
/// ## The window's end is computed from the true page grid, not from the
/// ## (possibly clamped) leading extension
///
/// **Fix round 1, Important finding**: an earlier version computed the end
/// as `aligned_load_addr + round_up(back_extend + len, page_size)`. That is
/// only correct when the leading extension reached the full `gap` (i.e.
/// `aligned_load_addr` really is page-aligned). When it was clamped short
/// (no earlier bytes available -- see above), `aligned_load_addr` is
/// *not* page-aligned, so rounding up from it lands on the wrong grid and
/// overshoots into the *next* page by up to `gap - back_extend` bytes --
/// concretely, `load_addr=0x4200_0020, file_offset=0, len=2` used to
/// produce `[0x4200_0020, 0x4201_0020)`, 0x20 bytes into the next page,
/// silently mapping address space that must stay a genuine catch-all miss.
/// Fixed by computing the end independently, directly from the *true*
/// (unclamped) page grid -- `round_up(load_addr + len, page_size)` -- which
/// is always 0-grid-aligned regardless of where the clamped start landed,
/// then subtracting `aligned_load_addr` to get the length. See
/// `xip_page_window_clamps_gracefully_when_no_earlier_flash_bytes_exist`'s
/// assertion that the next page is still a catch-all miss.
///
/// ## `aligned_file_offset`'s assumption
///
/// `aligned_file_offset = file_offset - back_extend` implicitly assumes the
/// segment's flash *file offset* and its *virtual load address* have the
/// same low bits mod `page_size` -- i.e. that subtracting the vaddr gap from
/// `file_offset` lands on that same offset's own page-aligned start. Real
/// hardware doesn't need this (`set_cache_and_start_app` page-aligns the
/// physical flash paddr *independently* of the vaddr,
/// `bootloader_utility.c:1068`ish: `drom_addr_aligned = drom_addr &
/// MMU_FLASH_MASK_FROM_VAL(mmu_page_size)`), but this emulator's `flash`
/// buffer is indexed by a single flat `file_offset`, not by a separate
/// paddr, so the two must coincide for this subtraction to land on the
/// right byte. This holds for every segment `esptool`-produced app images
/// (and specifically `factory.bin`) actually contain: an app image's
/// segments are written contiguously into the flash partition in link
/// order with no gaps, so each segment's flash offset within the partition
/// equals `load_addr`'s own low bits by construction (confirmed directly:
/// segment 0's `load_addr=0x3c13_0020` and `file_offset=0x20` already agree
/// mod 64 KiB). Not re-derived from first principles here -- flagged as an
/// assumption specific to esptool-style images, not a general property.
///
/// All arithmetic here is checked -- this must never panic on adversarial
/// input. `FirmwareEmulator::new` (`emulator-wasm`) reaches
/// [`FirmwareBus::from_segments`] directly with a browser-supplied image,
/// and `usize` is 32 bits on that target, so a malformed `len`/
/// `file_offset` must not be able to overflow this arithmetic and panic
/// (the same contract `from_segments`'s existing RAM-region path already
/// holds, see its comment). Any overflow here simply falls back to the
/// segment's own unwidened window -- degrading gracefully, same as the
/// bus's never-panic catch-all tier, rather than fabricating an unsound
/// mapping.
fn xip_page_window(
    load_addr: u32,
    file_offset: usize,
    len: usize,
    page_size: u32,
) -> (u32, usize, u32) {
    // `len as u32` truncates if `len` doesn't fit -- same as this
    // function's caller did unconditionally before Task D5 (`seg.len as
    // u32`), so the fallback preserves pre-existing behavior exactly rather
    // than introducing a new truncation risk.
    let fallback = (load_addr, file_offset, len as u32);
    let Ok(len_u32) = u32::try_from(len) else {
        return fallback;
    };

    let gap = load_addr % page_size;
    let back_extend = (gap as usize).min(file_offset);
    let Ok(back_extend_u32) = u32::try_from(back_extend) else {
        return fallback;
    };
    let aligned_load_addr = load_addr - back_extend_u32;
    let aligned_file_offset = file_offset - back_extend;

    // The end must come from the TRUE (unclamped) page grid -- see this
    // function's doc comment's "Fix round 1" note -- not from rounding up
    // `back_extend + len` starting at `aligned_load_addr`, which is only
    // grid-aligned when `back_extend == gap`.
    let Some(true_end) = load_addr.checked_add(len_u32) else {
        return fallback;
    };
    let page_count_end = true_end.div_ceil(page_size);
    let Some(page_end) = page_count_end.checked_mul(page_size) else {
        return fallback;
    };
    let Some(aligned_len) = page_end.checked_sub(aligned_load_addr) else {
        return fallback;
    };

    (aligned_load_addr, aligned_file_offset, aligned_len)
}

impl FirmwareBus {
    /// Builds a `FirmwareBus` from a flash image and its already-parsed
    /// segment table (see `crate::mem::image::parse_image`). Categorizes
    /// each segment as XIP or RAM-copied purely by `load_addr`
    /// ([`is_xip_addr`]) — XIP segments keep referencing `flash` in place;
    /// RAM segments get their bytes copied into a fresh owned buffer here.
    ///
    /// ## XIP segments are widened to their containing MMU page(s)
    ///
    /// Real hardware's flash cache is paged (`crate::mem::soc::MMU_PAGE_SIZE`,
    /// 64 KiB on ESP32-C3): the 2nd-stage bootloader's
    /// `set_cache_and_start_app()` page-aligns each XIP segment's `load_addr`
    /// *down* and its mapped byte count *up* to whole pages
    /// (`bootloader_support/src/bootloader_utility.c`, composed with
    /// `hal/mmu_hal.c`'s `mmu_hal_map_region()` -- see
    /// [`xip_page_window`]'s doc for the full citation), so whatever else
    /// happens to sit in the same physical flash page as a segment --
    /// before its `load_addr` or after `load_addr + len` -- becomes
    /// readable at the matching virtual address too, not just the
    /// segment's own declared bytes.
    ///
    /// This matters concretely: ESP-IDF v5.5.3's `cpu_start`
    /// (`components/esp_system/port/cpu_start.c`) reads the running app's
    /// own `esp_image_header_t` from `&_rodata_reserved_start -
    /// sizeof(esp_image_header_t) - sizeof(esp_image_segment_header_t)` --
    /// a linker symbol that resolves to the DROM segment's own `load_addr`,
    /// so this read lands 32 bytes *before* `load_addr`, in this leading
    /// page gap. Confirmed against `factory.bin`'s real segment table (Task
    /// D5's report): DROM segment 0's `load_addr` is `0x3c13_0020` (32
    /// bytes into its containing page, `0x3c13_0000`), and a real boot
    /// trace shows `cpu_start` reading exactly 24 consecutive bytes
    /// (`sizeof(esp_image_header_t)`) starting at `0x3c13_0000` -- which,
    /// pre-fix, fell through to the bus's never-panic catch-all (reads 0)
    /// since `[0x3c13_0000, 0x3c13_0020)` sat outside the segment's own
    /// `[load_addr, load_addr+len)`, making the magic-byte check
    /// (`fhdr.magic != ESP_IMAGE_HEADER_MAGIC`) fail and call `abort()`.
    pub fn from_segments(flash: Arc<[u8]>, segments: &[SegmentDescriptor]) -> Self {
        let mut xip_regions = Vec::new();
        let mut ram_regions = Vec::new();

        for seg in segments {
            if is_xip_addr(seg.load_addr) {
                let (load_addr, file_offset, len) =
                    xip_page_window(seg.load_addr, seg.file_offset, seg.len, MMU_PAGE_SIZE);
                xip_regions.push(XipRegion {
                    load_addr,
                    len,
                    file_offset,
                });
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
        Self {
            flash,
            xip_regions,
            ram_regions,
            rom_code: Vec::new(),
            rom_data: Vec::new(),
            systimer: SysTimer::new(),
            intc: InterruptController::new(),
            system: System::new(),
            gpio: Gpio::new(),
            spi: Spi::new(),
            usb_serial_jtag: UsbSerialJtag::new(),
            timg0: Timg::new(),
            timg1: Timg::new(),
            rtc_cntl: RtcCntl::new(),
            spimem1: Spimem1::new(),
            flash_chip,
            console: Console::new(),
            unmapped_log: VecDeque::with_capacity(UNMAPPED_LOG_CAPACITY),
        }
    }

    /// The OR of every peripheral's currently-asserted interrupt *source*
    /// levels (bit `n` = source number `n`, `crate::mem::soc::SRC_*`). A
    /// pure recomputation from live peripheral state -- nothing latched --
    /// so a source de-asserts the moment its peripheral clears it. Sources
    /// so far: SYSTIMER target0 and the SYSTEM `FROM_CPU_0..3` software
    /// interrupts.
    pub fn pending_sources(&self) -> u64 {
        let mut p = 0u64;
        if self.systimer.target0_pending() {
            p |= 1u64 << SRC_SYSTIMER_TARGET0;
        }
        p |= u64::from(self.system.pending_mask()) << SRC_FROM_CPU_INTR0;
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
    /// ticks. Call exactly once per `Cpu::step()` — see
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
        if let Some(region) = self.xip_regions.iter().find(|r| r.contains(addr)) {
            let offset = region.file_offset + (addr - region.load_addr) as usize;
            // Fix round 1, M2: a byte inside a *mapped* XIP window but past
            // the end of the actual `flash` buffer (e.g. the trailing part
            // of a page-widened region this emulator's reconstructed image
            // doesn't carry real bytes for -- see
            // `xip_page_window`'s doc) reads as `0xFF`, not `0`: real NOR
            // flash's erased/blank state is all-ones, not all-zeros. This
            // is distinct from the bus's never-panic *catch-all* tier below
            // (genuinely unmapped address space), which still reads `0` --
            // that's a different, deliberate convention (see the module
            // doc), not flash-specific.
            return self.flash.get(offset).copied().unwrap_or(0xFF);
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
        self.record_unmapped(addr, false);
        0
    }

    fn write_byte(&mut self, addr: u32, val: u8) {
        if self.xip_regions.iter().any(|r| r.contains(addr)) {
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
                self.spi.process_transaction(dc_low);
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
            // is systimer's own live counter (no free-running clock of its
            // own to model) -- both fields are concrete on `self`, so this
            // is just direct field access, the same pattern the
            // INTERRUPT_CORE0/SPI2 tiers above already use. See
            // `crate::peripherals::rtc_cntl`'s module doc.
            let elapsed_steps = self.systimer.counter();
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
        self.record_unmapped(addr, true);
    }

    /// `true` if `addr` falls inside an XIP, RAM-copied or ROM-code region — i.e. is
    /// "genuinely executable" per [`Bus::fetch16`]'s contract. ROM *data*
    /// blobs ([`RomDataBlob`]) are deliberately not checked here, so they are
    /// never fetchable. Reuses the
    /// same region-membership checks [`FirmwareBus::read_byte`]/
    /// [`FirmwareBus::write_byte`] use, rather than duplicating them.
    fn is_mapped(&self, addr: u32) -> bool {
        self.xip_regions.iter().any(|r| r.contains(addr))
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

    fn bus_with(segments: Vec<(u32, Vec<u8>)>) -> FirmwareBus {
        // Build a fake "flash image": concatenate each segment's bytes back
        // to back, tracking file offsets, so FirmwareBus's XIP path has real
        // bytes to index into.
        let mut flash = Vec::new();
        let mut descriptors = Vec::new();
        for (load_addr, data) in &segments {
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

    // Task D5: `cpu_start`'s app-image-header check reads the image header
    // via a linker symbol (`_rodata_reserved_start`, ESP-IDF v5.5.3's
    // `components/esp_system/port/cpu_start.c`) that resolves to the DROM
    // segment's own `load_addr` -- i.e. bytes *before* `load_addr`, in the
    // same 64 KiB flash-cache MMU page, per `bootloader_support/src/
    // bootloader_utility.c`'s `set_cache_and_start_app()` (page-align
    // `load_addr`/flash paddr down, extend size by the leading gap) and
    // `hal/mmu_hal.c`'s `mmu_hal_map_region()` (round the total mapped
    // length up to a whole number of pages). A segment whose `load_addr`
    // isn't itself page-aligned must therefore expose the *whole*
    // containing page -- both the leading gap before `load_addr` and any
    // trailing gap after `load_addr + len` -- not just the segment's own
    // declared bytes.
    #[test]
    fn xip_regions_expose_the_full_containing_64kib_page_not_just_the_declared_segment() {
        // A segment 0x20 bytes into its containing 64 KiB page, with real
        // flash bytes present both before (the "header") and only up to its
        // own declared end (nothing stored past it, mirroring a real image
        // file that simply ends there).
        let load_addr = 0x3c13_0020u32;
        let file_offset = 0x20usize;
        let seg_data = [0xCCu8, 0xDD, 0xEE, 0xFF];
        let mut flash = vec![0u8; file_offset + seg_data.len()];
        for (i, b) in flash.iter_mut().enumerate().take(file_offset) {
            *b = 0xA0 + i as u8;
        }
        flash[file_offset..file_offset + seg_data.len()].copy_from_slice(&seg_data);

        let descriptors = [SegmentDescriptor {
            load_addr,
            file_offset,
            len: seg_data.len(),
        }];
        let mut bus =
            FirmwareBus::from_segments(Arc::from(flash.clone().into_boxed_slice()), &descriptors);

        let page_start = load_addr & !0xFFFF;
        assert_eq!(page_start, 0x3c13_0000);

        // The leading gap (the page's first 0x20 bytes, including where
        // cpu_start's header copy actually reads from) is now real, mapped
        // flash data -- not the never-panic catch-all.
        assert!(bus.unmapped_log().is_empty());
        for i in 0u32..file_offset as u32 {
            assert_eq!(
                bus.read8(page_start + i),
                flash[i as usize],
                "byte at page offset 0x{i:x} (before load_addr) should come from the flash page"
            );
        }
        assert!(
            bus.unmapped_log().is_empty(),
            "page-prefix bytes must be mapped, not fall through to the catch-all"
        );

        // The segment's own declared bytes are unchanged.
        for (i, b) in seg_data.iter().enumerate() {
            assert_eq!(bus.read8(load_addr + i as u32), *b);
        }

        // A trailing same-page address past the segment's own declared end
        // must also be mapped now (real hardware's MMU maps the whole
        // page): this synthetic flash buffer has no real bytes there, so it
        // reads 0xFF (Task D5 fix round 1, M2 -- real NOR flash's erased
        // state), via the mapped region's own out-of-bounds fallback, not
        // the catch-all (which would read 0 instead -- a different,
        // deliberate convention for genuinely unmapped space).
        let just_past_segment = load_addr + seg_data.len() as u32;
        assert_eq!(bus.read8(just_past_segment), 0xFF);
        assert!(
            bus.unmapped_log().is_empty(),
            "trailing same-page bytes must be mapped too, not fall through to the catch-all"
        );

        // Writes anywhere in the widened window are still dropped (still
        // XIP/read-only), same as the pre-existing segment bytes.
        bus.write8(page_start, 0xFF);
        assert_eq!(bus.read8(page_start), flash[0]);

        // But the *next* 64 KiB page must still be a genuine catch-all miss
        // -- this rule must not swallow unrelated address space.
        let next_page = page_start + 0x1_0000;
        assert_eq!(bus.read8(next_page), 0);
        assert_eq!(
            bus.unmapped_log().len(),
            1,
            "the next page must not be swept into this XIP region"
        );
    }

    #[test]
    fn xip_page_window_clamps_gracefully_when_no_earlier_flash_bytes_exist() {
        // A synthetic image whose only bytes ARE the segment itself
        // (file_offset 0), but whose load_addr isn't page-aligned. Real
        // hardware's physical flash chip always has *some* bytes earlier in
        // the same page; this emulator's reconstructed image sometimes
        // won't (e.g. a browser-supplied partial image). Must not panic
        // (no unchecked subtraction) and must not fabricate bytes it
        // doesn't have -- it just can't expose the leading gap in this
        // case, same as real hardware couldn't if handed a truncated image.
        let seg_data = [0x01u8, 0x02];
        let descriptors = [SegmentDescriptor {
            load_addr: 0x4200_0020,
            file_offset: 0,
            len: seg_data.len(),
        }];
        let mut bus = FirmwareBus::from_segments(
            Arc::from(seg_data.to_vec().into_boxed_slice()),
            &descriptors,
        );

        assert_eq!(bus.read8(0x4200_0020), 0x01);
        assert_eq!(bus.read8(0x4200_0021), 0x02);

        // Fix round 1, Important finding: when the leading extension is
        // clamped short (no earlier bytes available, as here), the window's
        // START isn't page-aligned, so its END must be computed from the
        // TRUE (unclamped) page grid -- `round_up(load_addr + len,
        // page_size)` -- not by rounding up from the clamped, unaligned
        // start. An earlier version got this wrong and produced a window
        // extending 0x20 bytes into the NEXT page (`[0x4200_0020,
        // 0x4201_0020)` instead of `[0x4200_0020, 0x4201_0000)`). Assert the
        // true page boundary is still a genuine catch-all miss.
        assert_eq!(bus.read8(0x4201_0000), 0);
        assert!(
            bus.unmapped_log()
                .iter()
                .any(|a| a.addr == 0x4201_0000 && !a.is_write),
            "0x4201_0000 (the next page after this clamped window's TRUE \
             page-grid end) must show up in unmapped_log as a genuine \
             catch-all miss, not silently be swept into the XIP window"
        );
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
}

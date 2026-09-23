//! [`FirmwareBus`]: the real ESP32-C3 memory map, backing Task 1's [`Bus`]
//! trait with an app image's parsed segments (see `crate::mem::image`).
//!
//! An ordered sequence of named address ranges, checked in this order on
//! every access:
//! 1. **XIP** (flash-mapped, [`crate::mem::soc::is_xip_addr`]): read directly
//!    out of the original flash image bytes at `file_offset + (addr -
//!    load_addr)`. Writes are silently dropped (real hardware: read-only
//!    flash cache).
//! 2. **RAM-copied**: a real, mutable, per-segment `Vec<u8>` that the
//!    segment's bytes were copied into at boot. Reads/writes go straight to
//!    it.
//! 3. **SYSTIMER** ([`crate::mem::soc::SYSTIMER_RANGE`]): routed to
//!    [`FirmwareBus::systimer`], a concrete named field per this plan's
//!    pre-flight "no trait-object peripheral dispatch" ruling — see
//!    `crate::peripherals` and `crate::peripherals::systimer`.
//! 4. **INTERRUPT_CORE0** ([`crate::mem::soc::INTERRUPT_CORE0_RANGE`]):
//!    routed to [`FirmwareBus::intc`], same ruling — see
//!    `crate::peripherals::intc`. One register
//!    (`CPU_INT_EIP_STATUS_REG`) needs `systimer`'s live pending state to
//!    answer a read, which is exactly the cross-peripheral access the
//!    ruling anticipated: [`FirmwareBus::read_byte`] reads both concrete
//!    fields directly, no trait object involved.
//! 5. **GPIO** ([`crate::mem::soc::GPIO_RANGE`]): routed to
//!    [`FirmwareBus::gpio`], same ruling — see `crate::peripherals::gpio`
//!    for the register model and the emulated 74HC165 button shift
//!    register.
//! 6. **SPI2/GPSPI2** ([`crate::mem::soc::SPI2_RANGE`]): routed to
//!    [`FirmwareBus::spi`], same ruling — see `crate::peripherals::spi` for
//!    the register model and the ST7789 command/pixel-stream interpreter.
//!    A triggering write (one that sets `SPI_CMD_REG`'s `SPI_USR` bit)
//!    needs `gpio`'s live GPIO0 level (the D/C line) to know whether the
//!    transaction is a command or data — another cross-peripheral read
//!    the "no trait-object dispatch" ruling anticipated:
//!    [`FirmwareBus::write_byte`] reads `self.gpio.pin_level(0)` directly
//!    and hands it to [`crate::peripherals::spi::Spi::process_transaction`],
//!    no trait object involved.
//! 7. **USB-Serial-JTAG** ([`crate::mem::soc::USB_SERIAL_JTAG_RANGE`]):
//!    routed to [`FirmwareBus::usb_serial_jtag`], same ruling — see
//!    `crate::peripherals::usb_serial_jtag`. TX-byte writes also need a
//!    live mutable reference to [`FirmwareBus::console`] (the capped sink
//!    that firmware console output accumulates into); `FirmwareBus::write_byte`
//!    passes `&mut self.console` straight through, another instance of the
//!    "no trait object" ruling's direct concrete-field access.
//! 8. **TIMG0** ([`crate::mem::soc::TIMG0_RANGE`]): routed to
//!    [`FirmwareBus::timg0`], same ruling — see `crate::peripherals::timg`
//!    for the RTC slow-clock calibration model `rtc_clk_cal_internal()`
//!    polls at boot, plus inert MWDT watchdog storage.
//! 9. **TIMG1** ([`crate::mem::soc::TIMG1_RANGE`]): routed to
//!    [`FirmwareBus::timg1`], same peripheral model as TIMG0 (a second,
//!    independent instance) — same ruling.
//! 10. **RTC_CNTL** ([`crate::mem::soc::RTC_CNTL_RANGE`]): routed to
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
//! 11. **Catch-all**: any address covered by none of the above (every
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
use crate::peripherals::gpio::Gpio;
use crate::peripherals::intc::{self, InterruptController};
use crate::peripherals::rtc_cntl::RtcCntl;
use crate::peripherals::spi::Spi;
use crate::peripherals::systimer::SysTimer;
use crate::peripherals::timg::Timg;
use crate::peripherals::usb_serial_jtag::UsbSerialJtag;

use super::image::SegmentDescriptor;
use super::soc::{
    is_xip_addr, GPIO_RANGE, INTERRUPT_CORE0_RANGE, RTC_CNTL_RANGE, SPI2_RANGE, SYSTIMER_RANGE,
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

/// The concrete [`Bus`] implementation used to boot a real ESP-IDF app
/// image. See the module-level docs for the ordered-sequence-of-named-
/// regions read/write dispatch (XIP, RAM, SYSTIMER, INTERRUPT_CORE0, GPIO,
/// SPI2/GPSPI2, then a never-panic catch-all).
pub struct FirmwareBus {
    /// The original flash image bytes, kept once and shared (never copied)
    /// — XIP regions index directly into this.
    flash: Arc<[u8]>,
    xip_regions: Vec<XipRegion>,
    ram_regions: Vec<RamRegion>,
    /// The SYSTIMER peripheral (`crate::peripherals::systimer`), a concrete
    /// named field per this plan's pre-flight design ruling — see the
    /// module doc.
    pub systimer: SysTimer,
    /// The `INTERRUPT_CORE0` interrupt matrix (`crate::peripherals::intc`),
    /// same ruling.
    pub intc: InterruptController,
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
    /// ([`is_xip_addr`]) — XIP segments keep referencing `flash` in place;
    /// RAM segments get their bytes copied into a fresh owned buffer here.
    pub fn from_segments(flash: Arc<[u8]>, segments: &[SegmentDescriptor]) -> Self {
        let mut xip_regions = Vec::new();
        let mut ram_regions = Vec::new();

        for seg in segments {
            if is_xip_addr(seg.load_addr) {
                xip_regions.push(XipRegion {
                    load_addr: seg.load_addr,
                    len: seg.len as u32,
                    file_offset: seg.file_offset,
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

        Self {
            flash,
            xip_regions,
            ram_regions,
            systimer: SysTimer::new(),
            intc: InterruptController::new(),
            gpio: Gpio::new(),
            spi: Spi::new(),
            usb_serial_jtag: UsbSerialJtag::new(),
            timg0: Timg::new(),
            timg1: Timg::new(),
            rtc_cntl: RtcCntl::new(),
            console: Console::new(),
            unmapped_log: VecDeque::with_capacity(UNMAPPED_LOG_CAPACITY),
        }
    }

    /// Advances [`FirmwareBus::systimer`]'s counter by one step's worth of
    /// ticks and polls [`FirmwareBus::intc`] for a newly-pending, enabled
    /// interrupt line. Call exactly once per `Cpu::step()` — see
    /// `crate::boot::step_with_interrupts`, the driving loop that does so
    /// and feeds the result into `Cpu::raise_interrupt`.
    pub fn tick_peripherals(&mut self) -> Option<u32> {
        self.systimer.advance();
        let pending = self.systimer.target0_pending();
        self.intc.poll(pending)
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
            return self.flash.get(offset).copied().unwrap_or(0);
        }
        if let Some(region) = self.ram_regions.iter().find(|r| r.contains(addr)) {
            let offset = (addr - region.load_addr) as usize;
            return region.data[offset];
        }
        if SYSTIMER_RANGE.contains(&addr) {
            let offset = addr - SYSTIMER_RANGE.start;
            if !SysTimer::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.systimer.read_byte(offset);
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
                let word = self.intc.eip_status(self.systimer.target0_pending());
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
        if SYSTIMER_RANGE.contains(&addr) {
            let offset = addr - SYSTIMER_RANGE.start;
            if !SysTimer::handles(offset) {
                self.record_unmapped(addr, true);
            }
            self.systimer.write_byte(offset, val);
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
        self.record_unmapped(addr, true);
    }

    /// `true` if `addr` falls inside an XIP or RAM-copied region — i.e. is
    /// "genuinely executable" per [`Bus::fetch16`]'s contract. Reuses the
    /// same region-membership checks [`FirmwareBus::read_byte`]/
    /// [`FirmwareBus::write_byte`] use, rather than duplicating them.
    fn is_mapped(&self, addr: u32) -> bool {
        self.xip_regions.iter().any(|r| r.contains(addr))
            || self.ram_regions.iter().any(|r| r.contains(addr))
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
}

//! [`FirmwareBus`]: the real ESP32-C3 memory map, backing Task 1's [`Bus`]
//! trait with an app image's parsed segments (see `crate::mem::image`).
//!
//! Three kinds of address ranges, checked in this order on every access:
//! 1. **XIP** (flash-mapped, [`crate::mem::soc::is_xip_addr`]): read directly
//!    out of the original flash image bytes at `file_offset + (addr -
//!    load_addr)`. Writes are silently dropped (real hardware: read-only
//!    flash cache).
//! 2. **RAM-copied**: a real, mutable, per-segment `Vec<u8>` that the
//!    segment's bytes were copied into at boot. Reads/writes go straight to
//!    it.
//! 3. **Catch-all**: any address covered by neither of the above (this is
//!    every not-yet-modeled ESP32-C3 peripheral MMIO register, plus truly
//!    unmapped space). Reads return `0`, writes are dropped — this must
//!    never panic, for any address, since real firmware immediately starts
//!    probing peripheral registers that later tasks haven't built yet. Every
//!    such access is recorded into a small capped ring buffer
//!    ([`FirmwareBus::unmapped_log`]) as a debugging aid for later
//!    "why is boot stuck" investigation.

use std::collections::VecDeque;
use std::sync::Arc;

use super::image::SegmentDescriptor;
use super::soc::is_xip_addr;
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
/// image. See the module-level docs for the three-tier read/write behavior.
pub struct FirmwareBus {
    /// The original flash image bytes, kept once and shared (never copied)
    /// — XIP regions index directly into this.
    flash: Arc<[u8]>,
    xip_regions: Vec<XipRegion>,
    ram_regions: Vec<RamRegion>,
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
                let data = flash[seg.file_offset..seg.file_offset + seg.len].to_vec();
                ram_regions.push(RamRegion {
                    load_addr: seg.load_addr,
                    data,
                });
            }
        }

        Self {
            flash,
            xip_regions,
            ram_regions,
            unmapped_log: VecDeque::with_capacity(UNMAPPED_LOG_CAPACITY),
        }
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
        self.record_unmapped(addr, true);
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

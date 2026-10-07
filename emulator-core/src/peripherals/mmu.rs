//! The ESP32-C3 flash MMU table (`DR_REG_MMU_TABLE = 0x600c_5000`).
//!
//! Sources (ESP-IDF v5.5.3): `components/soc/esp32c3/register/soc/reg_base.h`
//! (base), `components/soc/esp32c3/include/soc/ext_mem_defs.h`
//! (`SOC_MMU_ENTRY_NUM`, `SOC_MMU_INVALID`, `SOC_MMU_VALID_VAL_MASK`,
//! `SOC_MMU_VADDR_MASK`), `components/hal/esp32c3/include/hal/mmu_ll.h`
//! (`mmu_ll_get_entry_id`, `mmu_ll_write_entry`, `mmu_ll_set_entry_invalid`,
//! `mmu_ll_entry_id_to_paddr_base`).
//!
//! 128 32-bit entries, one per 64 KiB virtual page, **shared** by the
//! data-bus (`0x3C00_0000..0x3C80_0000`) and instruction-bus
//! (`0x4200_0000..0x4280_0000`) apertures: entry id =
//! `(vaddr & 0x7FFFFF) >> 16`. Bits 0..7 hold the physical flash page,
//! bit 8 set marks the entry invalid. Entries are plain storage (every bit
//! reads back); translation looks only at bits 0..8.
//!
//! Reset value: every entry invalid, which is what the bootloader's
//! `mmu_hal_unmap_all()` leaves before it maps the app
//! (`crate::mem::bus::FirmwareBus::from_segments` then replays those
//! mappings). No cache is modeled: a table write takes effect on the next
//! access, which is what the firmware's cache invalidation after every
//! table change guarantees on real hardware.

use crate::mem::soc::{MMU_ENTRY_NUM, MMU_INVALID, MMU_PAGE_SIZE, MMU_VADDR_MASK, MMU_VALID_VAL_MASK};

use super::set_byte;

/// Bytes of register space the table occupies.
const TABLE_BYTES: u32 = 4 * MMU_ENTRY_NUM as u32;

#[derive(Clone)]
pub struct FlashMmu {
    entries: [u32; MMU_ENTRY_NUM],
}

impl Default for FlashMmu {
    fn default() -> Self {
        Self::new()
    }
}

impl FlashMmu {
    pub fn new() -> Self {
        Self {
            entries: [MMU_INVALID; MMU_ENTRY_NUM],
        }
    }

    /// `true` iff `offset` (from `DR_REG_MMU_TABLE`) is inside the table.
    pub fn handles(offset: u32) -> bool {
        offset < TABLE_BYTES
    }

    pub fn read_byte(&self, offset: u32) -> u8 {
        if !Self::handles(offset) {
            return 0;
        }
        self.entries[(offset / 4) as usize].to_le_bytes()[(offset % 4) as usize]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        if !Self::handles(offset) {
            return;
        }
        set_byte(&mut self.entries[(offset / 4) as usize], offset % 4, val);
    }

    /// The raw value of entry `id` (panics if `id >= 128`; callers index
    /// with [`FlashMmu::entry_id`] or a constant).
    pub fn entry(&self, id: usize) -> u32 {
        self.entries[id]
    }

    /// `mmu_ll_get_entry_id`: the entry a cache-aperture address uses.
    pub fn entry_id(vaddr: u32) -> usize {
        ((vaddr & MMU_VADDR_MASK) / MMU_PAGE_SIZE) as usize
    }

    /// The physical flash address `vaddr` maps to, or `None` if its entry
    /// is invalid. The caller must already know `vaddr` is inside one of
    /// the two cache apertures (this does not check).
    pub fn translate(&self, vaddr: u32) -> Option<u32> {
        let e = self.entries[Self::entry_id(vaddr)];
        if e & MMU_INVALID != 0 {
            return None;
        }
        Some((e & MMU_VALID_VAL_MASK) * MMU_PAGE_SIZE + (vaddr % MMU_PAGE_SIZE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(m: &mut FlashMmu, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            m.write_byte(off + i as u32, *b);
        }
    }

    fn read_word(m: &FlashMmu, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| m.read_byte(off + i)))
    }

    #[test]
    fn reset_state_is_every_entry_invalid() {
        let m = FlashMmu::new();
        for id in 0..MMU_ENTRY_NUM {
            assert_eq!(m.entry(id), MMU_INVALID, "entry {id}");
        }
        assert_eq!(m.translate(0x3c00_0000), None);
        assert_eq!(m.translate(0x4200_0000), None);
    }

    #[test]
    fn word_write_via_bytes_round_trips_and_translates() {
        let mut m = FlashMmu::new();
        // Entry 39 -> physical page 0 (spi_flash_mmap's window in the M3 stall).
        write_word(&mut m, 39 * 4, 0);
        assert_eq!(read_word(&m, 39 * 4), 0);
        assert_eq!(m.translate(0x3c27_8000), Some(0x0000_8000));
        // Entry 19 -> page 1 (factory.bin's DROM: vaddr 0x3c130020, paddr 0x10020).
        write_word(&mut m, 19 * 4, 1);
        assert_eq!(m.translate(0x3c13_0020), Some(0x0001_0020));
    }

    #[test]
    fn invalid_bit_unmaps_and_other_high_bits_are_stored_but_ignored() {
        let mut m = FlashMmu::new();
        write_word(&mut m, 0, 0x0000_0105); // INVALID | page 5
        assert_eq!(m.translate(0x4200_0000), None);
        write_word(&mut m, 0, 0xFFFF_FE05); // bit 8 clear, junk above it
        assert_eq!(read_word(&m, 0), 0xFFFF_FE05, "plain storage");
        assert_eq!(m.translate(0x4200_1234), Some(0x0005_1234));
    }

    #[test]
    fn dbus_and_ibus_share_entries() {
        let mut m = FlashMmu::new();
        write_word(&mut m, 2 * 4, 0x15);
        assert_eq!(FlashMmu::entry_id(0x3c02_0000), 2);
        assert_eq!(FlashMmu::entry_id(0x4202_0000), 2);
        assert_eq!(m.translate(0x3c02_abcd), Some(0x0015_abcd));
        assert_eq!(m.translate(0x4202_abcd), Some(0x0015_abcd));
    }

    #[test]
    fn page_numbers_up_to_255_translate_unwrapped() {
        // Wrapping to the chip size is the bus's job (it knows the chip);
        // the MMU reports the raw 8-bit page.
        let mut m = FlashMmu::new();
        write_word(&mut m, 127 * 4, 0xff);
        assert_eq!(m.translate(0x3c7f_0010), Some(0x00ff_0010));
    }

    #[test]
    fn offsets_past_the_table_are_not_handled() {
        assert!(FlashMmu::handles(0));
        assert!(FlashMmu::handles(0x1ff));
        assert!(!FlashMmu::handles(0x200));
        let mut m = FlashMmu::new();
        m.write_byte(0x200, 0xaa); // dropped, must not panic
        assert_eq!(m.read_byte(0x200), 0);
    }
}

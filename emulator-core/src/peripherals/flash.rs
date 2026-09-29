//! The badge's SPI NOR flash chip, emulated as an in-memory 4 MiB array
//! ([`EmulatedFlash`]) (Milestone 3 Task 8).
//!
//! ## Contents
//!
//! Real hardware: a 4 MiB chip (JEDEC ID `0x46 0x40 0x16`, per this
//! milestone's hardware facts) holding the 2nd-stage bootloader at `0x0`,
//! the partition table at `0x8000`, and the partitions it lists. This
//! emulator never runs the bootloader (see `crate::boot`'s "shortcut boot"),
//! and the full dump of the physical chip is personal data that is never
//! committed (see `docs/firmware-emulator-notes.md`'s data-handling note),
//! so [`EmulatedFlash::from_app_image`] builds a **synthetic** image
//! instead:
//!
//! - everything blank (`0xFF`, NOR flash's erased state), except
//! - a synthesized partition table at [`PARTITION_TABLE_OFFSET`] listing
//!   exactly the badge's four partitions ([`PARTITIONS`]), and
//! - the app image (`factory.bin`) at [`APP_OFFSET`], the `factory`
//!   partition's own offset.
//!
//! So `nvs`, `phy_init` and `storage` start out blank, as on a freshly
//! erased chip; the bootloader region is blank too (nothing in the app
//! reads it).
//!
//! ## Partition table format
//!
//! ESP-IDF v5.5.3 `components/bootloader_support/include/esp_flash_partitions.h`:
//! each entry is a 32-byte `esp_partition_info_t` (`u16 magic =
//! ESP_PARTITION_MAGIC 0x50AA` stored little-endian as `AA 50`, `u8 type`,
//! `u8 subtype`, `esp_partition_pos_t { u32 offset; u32 size; }`,
//! `u8 label[16]`, `u32 flags`). The table is at most
//! `ESP_PARTITION_TABLE_MAX_LEN` (`0xC00`) bytes. When
//! `CONFIG_PARTITION_TABLE_MD5` is on (its Kconfig default is `y`,
//! `components/partition_table/Kconfig.projbuild`), the entries are followed
//! by one `ESP_PARTITION_MAGIC_MD5` (`0xEBEB`) entry whose MD5 digest of all
//! preceding entry bytes sits `ESP_PARTITION_MD5_OFFSET` (16) bytes in; the
//! bytes between the magic and the digest are `0xFF`. The badge's own table
//! does carry this entry: `tests/flash_partition_table.rs` compares the
//! synthesized table byte-for-byte against the real chip's at `0x8000`
//! (gated on `BADGE_FULL_DUMP`, skipped without it), which also checks
//! [`PARTITION_TABLE_MD5`].
//!
//! The partition layout (names, types, offsets, sizes) is not personal
//! data, so it is committed here as constants.
//!
//! ## Write semantics
//!
//! Real NOR flash: an erase sets a whole sector to `0xFF`; a program can
//! only clear bits (`1 -> 0`), so programming ANDs the new bytes into the
//! old ones. [`EmulatedFlash::erase_sector`] and [`EmulatedFlash::program`]
//! do exactly that, in memory only: nothing is ever written back to any
//! file. Out-of-range offsets never panic: reads return `0xFF` and writes
//! past the end are dropped.

/// Total emulated flash size: the badge's 4 MiB chip.
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;
/// `CONFIG_PARTITION_TABLE_OFFSET`: where the partition table lives (the
/// ESP-IDF default, and where the badge's real table is).
pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;
/// `ESP_PARTITION_TABLE_MAX_LEN` (`esp_flash_partitions.h`).
pub const PARTITION_TABLE_MAX_LEN: u32 = 0xC00;
/// The `factory` app partition's offset: where `factory.bin` is placed.
pub const APP_OFFSET: u32 = 0x1_0000;
/// The erase granule of the flash's sector-erase command (`0x20`, 4 KiB).
pub const SECTOR_SIZE: u32 = 0x1000;

/// One partition-table entry's fields (see the module doc's format).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionEntry {
    pub label: &'static str,
    pub ty: u8,
    pub subtype: u8,
    pub offset: u32,
    pub size: u32,
}

/// The badge's partition table (milestone hardware facts). Types/subtypes
/// per ESP-IDF v5.5.3 `components/esp_partition/include/esp_partition.h`:
/// `ESP_PARTITION_TYPE_APP` 0 / `ESP_PARTITION_TYPE_DATA` 1;
/// `ESP_PARTITION_SUBTYPE_DATA_NVS` 0x02, `..._DATA_PHY` 0x01,
/// `ESP_PARTITION_SUBTYPE_APP_FACTORY` 0x00,
/// `ESP_PARTITION_SUBTYPE_DATA_LITTLEFS` 0x83.
pub const PARTITIONS: [PartitionEntry; 4] = [
    PartitionEntry {
        label: "nvs",
        ty: 0x01,
        subtype: 0x02,
        offset: 0x9000,
        size: 0x4000,
    },
    PartitionEntry {
        label: "phy_init",
        ty: 0x01,
        subtype: 0x01,
        offset: 0xd000,
        size: 0x1000,
    },
    PartitionEntry {
        label: "factory",
        ty: 0x00,
        subtype: 0x00,
        offset: 0x1_0000,
        size: 0x2a_0000,
    },
    PartitionEntry {
        label: "storage",
        ty: 0x01,
        subtype: 0x83,
        offset: 0x2b_0000,
        size: 0x14_0000,
    },
];

/// MD5 digest of the 128 bytes the four [`PARTITIONS`] entries serialize
/// to. A constant rather than computed at run time, since this crate has no
/// MD5 implementation: it was computed with Python's `hashlib.md5` over
/// [`EmulatedFlash::from_app_image`]'s own serialized entries, and is
/// checked against the real chip's table by `tests/flash_partition_table.rs`
/// (gated on `BADGE_FULL_DUMP`). The firmware also re-verifies it itself
/// when it loads the table.
pub const PARTITION_TABLE_MD5: [u8; 16] = [
    0xe3, 0x01, 0x93, 0xf6, 0xc3, 0xbd, 0xa7, 0xf4, 0xfe, 0x3c, 0x05, 0xcd, 0xca, 0xe7, 0x71, 0x36,
];

/// `ESP_PARTITION_MAGIC` (`0x50AA`), as stored (little-endian).
const PARTITION_MAGIC_LE: [u8; 2] = [0xAA, 0x50];
/// `ESP_PARTITION_MAGIC_MD5` (`0xEBEB`).
const PARTITION_MAGIC_MD5_LE: [u8; 2] = [0xEB, 0xEB];
/// `sizeof(esp_partition_info_t)`.
const PARTITION_ENTRY_LEN: usize = 32;
/// `ESP_PARTITION_MD5_OFFSET`.
const PARTITION_MD5_OFFSET: usize = 16;

/// The emulated flash chip's contents. See the module doc.
#[derive(Clone)]
pub struct EmulatedFlash {
    data: Box<[u8]>,
}

impl EmulatedFlash {
    /// A 4 MiB blank (`0xFF`) chip with the synthesized partition table at
    /// [`PARTITION_TABLE_OFFSET`] and `app` at [`APP_OFFSET`]. An `app`
    /// longer than the space left after [`APP_OFFSET`] is truncated at the
    /// end of the chip rather than panicking (a browser-supplied image can
    /// be any size).
    pub fn from_app_image(app: &[u8]) -> Self {
        let mut data = vec![0xFFu8; FLASH_SIZE].into_boxed_slice();

        let table = PARTITION_TABLE_OFFSET as usize;
        for (i, p) in PARTITIONS.iter().enumerate() {
            let e = &mut data[table + i * PARTITION_ENTRY_LEN..][..PARTITION_ENTRY_LEN];
            e[0..2].copy_from_slice(&PARTITION_MAGIC_LE);
            e[2] = p.ty;
            e[3] = p.subtype;
            e[4..8].copy_from_slice(&p.offset.to_le_bytes());
            e[8..12].copy_from_slice(&p.size.to_le_bytes());
            e[12..28].fill(0);
            e[12..12 + p.label.len()].copy_from_slice(p.label.as_bytes());
            e[28..32].copy_from_slice(&0u32.to_le_bytes());
        }
        let md5 =
            &mut data[table + PARTITIONS.len() * PARTITION_ENTRY_LEN..][..PARTITION_ENTRY_LEN];
        md5[0..2].copy_from_slice(&PARTITION_MAGIC_MD5_LE);
        md5[PARTITION_MD5_OFFSET..].copy_from_slice(&PARTITION_TABLE_MD5);

        let app_start = APP_OFFSET as usize;
        let n = app.len().min(FLASH_SIZE - app_start);
        data[app_start..app_start + n].copy_from_slice(&app[..n]);

        Self { data }
    }

    /// The byte at flash offset `off`; `0xFF` past the end of the chip.
    pub fn read(&self, off: u32) -> u8 {
        self.data.get(off as usize).copied().unwrap_or(0xFF)
    }

    /// Erases (sets to `0xFF`) the whole [`SECTOR_SIZE`] sector containing
    /// `off`. Ignored past the end of the chip.
    pub fn erase_sector(&mut self, off: u32) {
        let start = (off & !(SECTOR_SIZE - 1)) as usize;
        if let Some(sector) = self.data.get_mut(start..start + SECTOR_SIZE as usize) {
            sector.fill(0xFF);
        }
    }

    /// Programs `data` starting at `off` with NOR semantics: each stored
    /// byte becomes `old & new` (bits only go `1 -> 0`). Bytes that would
    /// land past the end of the chip are dropped.
    pub fn program(&mut self, off: u32, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            let Some(addr) = (off as usize).checked_add(i) else {
                return;
            };
            match self.data.get_mut(addr) {
                Some(cell) => *cell &= *b,
                None => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(f: &EmulatedFlash, off: u32) -> u32 {
        u32::from_le_bytes([
            f.read(off),
            f.read(off + 1),
            f.read(off + 2),
            f.read(off + 3),
        ])
    }

    #[test]
    fn blank_flash_is_all_ones_outside_the_table_and_app() {
        let f = EmulatedFlash::from_app_image(&[0xE9, 0x01, 0x02]);
        assert_eq!(f.read(0), 0xFF, "bootloader area is blank");
        assert_eq!(f.read(0x7FFF), 0xFF);
        assert_eq!(f.read(0x9000), 0xFF, "nvs is blank");
        assert_eq!(f.read(APP_OFFSET + 3), 0xFF, "past the app image is blank");
        assert_eq!(f.read(FLASH_SIZE as u32 - 1), 0xFF);
    }

    #[test]
    fn app_image_is_placed_at_0x10000() {
        let f = EmulatedFlash::from_app_image(&[0xE9, 0x01, 0x02]);
        assert_eq!(f.read(APP_OFFSET), 0xE9);
        assert_eq!(f.read(APP_OFFSET + 1), 0x01);
        assert_eq!(f.read(APP_OFFSET + 2), 0x02);
    }

    #[test]
    fn an_oversized_app_image_is_truncated_at_the_end_of_flash_without_panicking() {
        let app = vec![0x5Au8; FLASH_SIZE];
        let f = EmulatedFlash::from_app_image(&app);
        assert_eq!(f.read(FLASH_SIZE as u32 - 1), 0x5A);
        assert_eq!(f.read(APP_OFFSET - 1), 0xFF);
    }

    #[test]
    fn out_of_range_reads_are_blank_and_never_panic() {
        let f = EmulatedFlash::from_app_image(&[]);
        assert_eq!(f.read(FLASH_SIZE as u32), 0xFF);
        assert_eq!(f.read(u32::MAX), 0xFF);
    }

    /// `esp_partition_info_t` (`bootloader_support/include/esp_flash_partitions.h`):
    /// `u16 magic (0x50AA, stored AA 50)`, `u8 type`, `u8 subtype`,
    /// `esp_partition_pos_t {u32 offset, u32 size}`, `u8 label[16]`,
    /// `u32 flags`.
    #[test]
    fn partition_table_holds_the_badges_four_entries() {
        let f = EmulatedFlash::from_app_image(&[]);
        let expect: [(u8, u8, u32, u32, &str); 4] = [
            (0x01, 0x02, 0x9000, 0x4000, "nvs"),
            (0x01, 0x01, 0xd000, 0x1000, "phy_init"),
            (0x00, 0x00, 0x1_0000, 0x2a_0000, "factory"),
            (0x01, 0x83, 0x2b_0000, 0x14_0000, "storage"),
        ];
        for (i, (ty, sub, off, size, label)) in expect.iter().enumerate() {
            let e = PARTITION_TABLE_OFFSET + 32 * i as u32;
            assert_eq!([f.read(e), f.read(e + 1)], [0xAA, 0x50], "entry {i} magic");
            assert_eq!(f.read(e + 2), *ty, "entry {i} type");
            assert_eq!(f.read(e + 3), *sub, "entry {i} subtype");
            assert_eq!(le32(&f, e + 4), *off, "entry {i} offset");
            assert_eq!(le32(&f, e + 8), *size, "entry {i} size");
            let mut want = [0u8; 16];
            want[..label.len()].copy_from_slice(label.as_bytes());
            let got: Vec<u8> = (0..16).map(|k| f.read(e + 12 + k)).collect();
            assert_eq!(got, want, "entry {i} label");
            assert_eq!(le32(&f, e + 28), 0, "entry {i} flags");
        }
    }

    /// `ESP_PARTITION_MAGIC_MD5` (`0xEBEB`) entry right after the last
    /// partition: magic, 14 bytes of `0xFF`, then the 16-byte MD5 at
    /// `ESP_PARTITION_MD5_OFFSET` (16), then blank (`0xFF`) to the end of
    /// `ESP_PARTITION_TABLE_MAX_LEN` (0xC00).
    #[test]
    fn partition_table_ends_with_the_md5_entry_then_blank() {
        let f = EmulatedFlash::from_app_image(&[]);
        let md5 = PARTITION_TABLE_OFFSET + 4 * 32;
        assert_eq!([f.read(md5), f.read(md5 + 1)], [0xEB, 0xEB]);
        for k in 2..16 {
            assert_eq!(f.read(md5 + k), 0xFF, "md5 entry padding byte {k}");
        }
        let digest: Vec<u8> = (0..16).map(|k| f.read(md5 + 16 + k)).collect();
        assert_eq!(digest, PARTITION_TABLE_MD5.to_vec());
        for off in md5 + 32..PARTITION_TABLE_OFFSET + 0xC00 {
            assert_eq!(f.read(off), 0xFF, "table tail at {off:#x} must be blank");
        }
    }

    #[test]
    fn erase_sector_blanks_exactly_the_containing_4k_sector() {
        let app = vec![0x00u8; 0x3000];
        let mut f = EmulatedFlash::from_app_image(&app);
        f.erase_sector(APP_OFFSET + 0x1234);
        assert_eq!(
            f.read(APP_OFFSET + 0x0FFF),
            0x00,
            "previous sector untouched"
        );
        assert_eq!(f.read(APP_OFFSET + 0x1000), 0xFF);
        assert_eq!(f.read(APP_OFFSET + 0x1FFF), 0xFF);
        assert_eq!(f.read(APP_OFFSET + 0x2000), 0x00, "next sector untouched");
    }

    #[test]
    fn erase_sector_out_of_range_is_ignored() {
        let mut f = EmulatedFlash::from_app_image(&[]);
        f.erase_sector(FLASH_SIZE as u32);
        f.erase_sector(u32::MAX);
    }

    #[test]
    fn program_can_only_clear_bits() {
        let mut f = EmulatedFlash::from_app_image(&[]);
        f.program(0x9000, &[0x0F, 0xF0]);
        assert_eq!([f.read(0x9000), f.read(0x9001)], [0x0F, 0xF0]);
        // A second program ANDs into what is there: bits never go 0 -> 1.
        f.program(0x9000, &[0xF3, 0xFF]);
        assert_eq!([f.read(0x9000), f.read(0x9001)], [0x03, 0xF0]);
        // Only an erase brings them back.
        f.erase_sector(0x9000);
        assert_eq!([f.read(0x9000), f.read(0x9001)], [0xFF, 0xFF]);
    }

    #[test]
    fn program_past_the_end_drops_the_overflow_without_panicking() {
        let mut f = EmulatedFlash::from_app_image(&[]);
        let last = FLASH_SIZE as u32 - 1;
        f.program(last, &[0x12, 0x34]);
        assert_eq!(f.read(last), 0x12);
        f.program(u32::MAX, &[0x00]);
    }
}

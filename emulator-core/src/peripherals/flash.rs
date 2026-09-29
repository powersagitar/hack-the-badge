//! The badge's SPI NOR flash chip, emulated as an in-memory 4 MiB array
//! ([`EmulatedFlash`]), and the SPI1 flash controller (`SPIMEM1`,
//! [`Spimem1`]) the firmware's flash driver talks to it through (Milestone 3
//! Task 8).
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
//!
//! ## SPIMEM1: the SPI1 flash controller
//!
//! `DR_REG_SPI1_BASE = 0x6000_2000` (`soc/reg_base.h`;
//! `crate::mem::soc::SPIMEM1_RANGE`). Register offsets and fields are from
//! ESP-IDF v5.5.3 `components/soc/esp32c3/register/soc/spi_mem_reg.h`
//! (`SPI_MEM_*_REG(i)`, `REG_SPI_MEM_BASE(1)`); the access sequences are from
//! `components/hal/esp32c3/include/hal/spimem_flash_ll.h`,
//! `components/hal/spi_flash_hal_common.inc` (`spi_flash_hal_common_command`,
//! `spi_flash_hal_configure_host_io_mode`, `spi_flash_hal_device_config`),
//! `components/spi_flash/memspi_host_driver.c` (`memspi_host_read_id_hs`)
//! and `components/bootloader_support/bootloader_flash/src/bootloader_flash.c`
//! (`bootloader_flash_execute_command_common`, used by the app's own
//! `bootloader_flash_update_id()`).
//!
//! **Observed traffic (boot-probe, Task 8 Step 1)**: before this model
//! existed every access below fell through to the bus catch-all. The
//! firmware touches `CMD` (`+0x00`), `ADDR` (`+0x04`), `CTRL` (`+0x08`),
//! `CTRL2` (`+0x10`), `CLOCK` (`+0x14`), `USER` (`+0x18`), `USER1`
//! (`+0x1C`), `USER2` (`+0x20`), `MOSI_DLEN` (`+0x24`), `MISO_DLEN`
//! (`+0x28`), `MISC` (`+0x34`), `W0` (`+0x58`), `FLASH_WAITI_CTRL`
//! (`+0x98`), `FLASH_SUS_CTRL` (`+0x9C`) and `CLOCK_GATE` (`+0xDC`), and
//! issues exactly one flash command, RDID (`0x9F`), three times:
//! once via `bootloader_flash_execute_command_common` and twice via
//! `spi_flash_hal_common_command`. [`Spimem1::handles`] names exactly those
//! registers; any other offset is still backed by storage but logged as
//! "unmapped" by the bus, so a new one shows up in `boot-probe`.
//!
//! What is modeled:
//!
//! - **Storage.** Every register below `0x100` is plain read/write word
//!   storage (`W0..W15` included), matching how the LL functions
//!   read-modify-write them. Offsets at or past `0x100` read 0 and drop
//!   writes (the only register there, `SPI_MEM_DATE_REG` `+0x3FC`, is
//!   unused).
//! - **Trigger.** `spimem_flash_ll_user_start` does `dev->cmd.val |=
//!   usr_pe` with `usr_pe = SPI_MEM_USR` (bit 18), or `SPI_MEM_USR |
//!   SPI_MEM_FLASH_PE` (bits 18 and 17) for program/erase. A 32-bit store
//!   reaches this model as four ascending byte writes (Review Focus 1), so
//!   the transaction fires on **byte 2 only** (bits 16..23, which hold
//!   `SPI_MEM_USR`) when that byte sets bit 2; bytes 0, 1 and 3 never fire
//!   it, so one `sw` runs exactly one transaction. The transaction reads
//!   the other registers it needs (`USER`, `USER2`, `MISO_DLEN`, …), which
//!   were written by earlier stores.
//! - **Completion.** The transaction completes within that same byte write;
//!   then `SPI_MEM_USR` and `SPI_MEM_FLASH_PE` are cleared, so the register
//!   reads 0 and `spimem_flash_ll_cmd_is_done` (`dev->cmd.val == 0`) is true
//!   on the first poll. (The status field `SPI_MEM_MST_ST` reads 0, idle.)
//! - **Command phase.** With `SPI_MEM_USR_COMMAND` (`USER` bit 31) set, the
//!   command is `SPI_MEM_USR_COMMAND_VALUE` (`USER2` bits 15..0), as written
//!   by `spimem_flash_ll_set_command`.
//! - **MISO phase.** With `SPI_MEM_USR_MISO` (`USER` bit 28) set, the
//!   controller receives `SPI_MEM_USR_MISO_DBITLEN + 1` bits
//!   (`MISO_DLEN` bits 9..0, set by `spimem_flash_ll_set_miso_bitlen` as
//!   `bitlen - 1`) into `W0..` starting at byte 0, little-endian within each
//!   word, which is how `spimem_flash_ll_get_buffer_data` reads them back
//!   (`data_buf[i]`). Bytes past the received count keep their old value.
//! - **RDID (`0x9F`, `CMD_RDID` in `spi_flash_defs.h`)** answers the badge's
//!   JEDEC ID [`JEDEC_ID`] (`0x46 0x40 0x16`), so `W0` reads `0x00164046`
//!   and `memspi_host_read_id_hs`'s byte swap gives chip ID `0x464016`. A
//!   read longer than 3 bytes gets only the 3 ID bytes (not observed; what a
//!   chip sends after its ID is chip-specific).
//! - **Any other command**, or a transaction without a command phase, still
//!   completes (so no poll hangs) but has no effect on the buffer or the
//!   chip; its value is recorded in [`Spimem1::unmodeled_commands`]
//!   ([`NO_COMMAND_PHASE`] for "no command phase"), which `boot-probe`
//!   prints. RDSR, READ, WREN, erase and program are deliberately not
//!   modeled until the probe shows the firmware issuing them.
//!
//! Not modeled: the address, dummy and MOSI phases (no observed command
//! uses them), `SPI_MEM_USR_MISO_HIGHPART`, the dedicated `SPI_MEM_FLASH_*`
//! command bits in `CMD` (bits 19..31; stored, but they trigger nothing,
//! so a firmware that used one would visibly spin on `cmd_is_done`), clock
//! and timing registers (stored only), and auto-suspend.

use std::collections::VecDeque;

use super::set_byte;

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

// ---------------------------------------------------------------------------
// SPIMEM1: the SPI1 flash controller
// ---------------------------------------------------------------------------

/// End of [`Spimem1`]'s register storage window. Every register
/// `spi_mem_reg.h` names below `SPI_MEM_DATE_REG` (`0x3FC`) sits below this
/// (the highest is `SPI_MEM_CORE_CLK_SEL_REG`, `0xE0`). Offsets at or past it
/// read 0 and drop writes.
const SPIMEM_REGS_END: u32 = 0x100;
const SPIMEM_REGS_WORDS: usize = (SPIMEM_REGS_END / 4) as usize;

/// `SPI_MEM_CMD_REG` (`+0x000`).
pub const CMD_REG: u32 = 0x000;
/// `SPI_MEM_ADDR_REG` (`+0x004`).
pub const ADDR_REG: u32 = 0x004;
/// `SPI_MEM_CTRL_REG` (`+0x008`).
pub const CTRL_REG: u32 = 0x008;
/// `SPI_MEM_CTRL2_REG` (`+0x010`).
pub const CTRL2_REG: u32 = 0x010;
/// `SPI_MEM_CLOCK_REG` (`+0x014`).
pub const CLOCK_REG: u32 = 0x014;
/// `SPI_MEM_USER_REG` (`+0x018`).
pub const USER_REG: u32 = 0x018;
/// `SPI_MEM_USER1_REG` (`+0x01C`).
pub const USER1_REG: u32 = 0x01C;
/// `SPI_MEM_USER2_REG` (`+0x020`).
pub const USER2_REG: u32 = 0x020;
/// `SPI_MEM_MOSI_DLEN_REG` (`+0x024`).
pub const MOSI_DLEN_REG: u32 = 0x024;
/// `SPI_MEM_MISO_DLEN_REG` (`+0x028`).
pub const MISO_DLEN_REG: u32 = 0x028;
/// `SPI_MEM_MISC_REG` (`+0x034`).
pub const MISC_REG: u32 = 0x034;
/// `SPI_MEM_W0_REG` (`+0x058`); `W1..W15` follow at a `+0x4` stride, up to
/// `SPI_MEM_W15_REG` (`+0x094`).
pub const W0_REG: u32 = 0x058;
/// `SPI_MEM_W15_REG` (`+0x094`).
pub const W15_REG: u32 = 0x094;
/// `SPI_MEM_FLASH_WAITI_CTRL_REG` (`+0x098`).
pub const FLASH_WAITI_CTRL_REG: u32 = 0x098;
/// `SPI_MEM_FLASH_SUS_CTRL_REG` (`+0x09C`).
pub const FLASH_SUS_CTRL_REG: u32 = 0x09C;
/// `SPI_MEM_CLOCK_GATE_REG` (`+0x0DC`).
pub const CLOCK_GATE_REG: u32 = 0x0DC;

/// `SPI_MEM_USR` (`SPI_MEM_CMD_REG` bit 18): lives in byte 2, bit 2.
const CMD_USR_BYTE_IDX: u32 = 2;
const CMD_USR_BIT_IN_BYTE: u8 = 1 << (18 - 16);
/// `SPI_MEM_USR | SPI_MEM_FLASH_PE` (bits 18 and 17), the bits
/// `spimem_flash_ll_user_start` sets and the controller clears on
/// completion.
const CMD_USR_AND_PE: u32 = (1 << 18) | (1 << 17);

/// `SPI_MEM_USR_COMMAND` (`SPI_MEM_USER_REG` bit 31).
pub const USER_USR_COMMAND: u32 = 1 << 31;
/// `SPI_MEM_USR_ADDR` (`SPI_MEM_USER_REG` bit 30).
pub const USER_USR_ADDR: u32 = 1 << 30;
/// `SPI_MEM_USR_MISO` (`SPI_MEM_USER_REG` bit 28).
pub const USER_USR_MISO: u32 = 1 << 28;
/// `SPI_MEM_USR_MOSI` (`SPI_MEM_USER_REG` bit 27).
pub const USER_USR_MOSI: u32 = 1 << 27;

/// `SPI_MEM_USR_COMMAND_VALUE` (`SPI_MEM_USER2_REG` bits `[15:0]`).
const USER2_COMMAND_VALUE_MASK: u32 = 0xFFFF;
/// `SPI_MEM_USR_MISO_DBITLEN` / `SPI_MEM_USR_MOSI_DBITLEN` (bits `[9:0]`).
const DLEN_BITLEN_MASK: u32 = 0x3FF;

/// `CMD_RDID` (`spi_flash/include/spi_flash/spi_flash_defs.h`).
pub const FLASH_CMD_RDID: u8 = 0x9F;
/// The badge's flash JEDEC ID, in the order the chip sends it:
/// manufacturer `0x46`, then device ID `0x40 0x16` (milestone hardware
/// facts; `memspi: chip_id` would log it as `0x464016`).
pub const JEDEC_ID: [u8; 3] = [0x46, 0x40, 0x16];

/// What [`Spimem1::unmodeled_commands`] records for a transaction with
/// `SPI_MEM_USR_COMMAND` clear (no command phase at all). Real command
/// values are at most 16 bits wide (`SPI_MEM_USR_COMMAND_VALUE`), and no
/// flash command is `0xFFFF`, so this cannot collide with an observed one.
pub const NO_COMMAND_PHASE: u16 = 0xFFFF;

/// How many unmodeled commands [`Spimem1`] remembers (a debugging aid for
/// `boot-probe`, not device state).
const UNMODELED_LOG_CAPACITY: usize = 16;

/// The SPI1 flash controller (`SPIMEM1`). See the module doc's "SPIMEM1"
/// section for exactly what is modeled and where each behavior comes from.
pub struct Spimem1 {
    /// Word storage for every register in `0..SPIMEM_REGS_END`, W0..W15
    /// included.
    regs: [u32; SPIMEM_REGS_WORDS],
    /// Number of `SPI_MEM_USR` transactions run so far (diagnostic).
    transactions: u64,
    /// The most recent command values the model has no behavior for,
    /// oldest first, capped at [`UNMODELED_LOG_CAPACITY`].
    unmodeled: VecDeque<u16>,
}

impl Default for Spimem1 {
    fn default() -> Self {
        Self {
            regs: [0; SPIMEM_REGS_WORDS],
            transactions: 0,
            unmodeled: VecDeque::with_capacity(UNMODELED_LOG_CAPACITY),
        }
    }
}

impl Spimem1 {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset`'s word is one of the registers the module doc
    /// lists (the ones the firmware was observed touching, each cited).
    /// `crate::mem::bus::FirmwareBus` logs an access to any other SPIMEM1
    /// offset as "unmapped" even though it is still backed by storage, the
    /// same pattern as `crate::peripherals::rtc_cntl::RtcCntl::handles`.
    pub fn handles(offset: u32) -> bool {
        let word = offset & !0b11;
        matches!(
            word,
            CMD_REG
                | ADDR_REG
                | CTRL_REG
                | CTRL2_REG
                | CLOCK_REG
                | USER_REG
                | USER1_REG
                | USER2_REG
                | MOSI_DLEN_REG
                | MISO_DLEN_REG
                | MISC_REG
                | FLASH_WAITI_CTRL_REG
                | FLASH_SUS_CTRL_REG
                | CLOCK_GATE_REG
        ) || (W0_REG..=W15_REG).contains(&word)
    }

    pub fn read_byte(&self, offset: u32) -> u8 {
        if offset >= SPIMEM_REGS_END {
            return 0;
        }
        self.regs[(offset >> 2) as usize].to_le_bytes()[(offset & 0b11) as usize]
    }

    /// Stores one byte. If it is byte 2 of `SPI_MEM_CMD_REG` and sets
    /// `SPI_MEM_USR`, runs the user transaction against `flash` right away
    /// and clears `SPI_MEM_USR`/`SPI_MEM_FLASH_PE` again (see the module
    /// doc's "Trigger" section).
    pub fn write_byte(&mut self, offset: u32, val: u8, flash: &mut EmulatedFlash) {
        if offset >= SPIMEM_REGS_END {
            return;
        }
        let widx = (offset >> 2) as usize;
        let idx = offset & 0b11;
        set_byte(&mut self.regs[widx], idx, val);
        if offset & !0b11 == CMD_REG && idx == CMD_USR_BYTE_IDX && val & CMD_USR_BIT_IN_BYTE != 0 {
            self.run_user_transaction(flash);
            self.regs[widx] &= !CMD_USR_AND_PE;
        }
    }

    /// Number of `SPI_MEM_USR` transactions run so far.
    pub fn transaction_count(&self) -> u64 {
        self.transactions
    }

    /// The most recent command values that had no modeled behavior (oldest
    /// first, capped). `boot-probe` prints these.
    pub fn unmodeled_commands(&self) -> &VecDeque<u16> {
        &self.unmodeled
    }

    fn reg(&self, offset: u32) -> u32 {
        self.regs[(offset >> 2) as usize]
    }

    /// Number of bytes a `*_DLEN_REG` bit length covers when its phase is
    /// enabled: `bitlen + 1` bits, rounded up to whole bytes, capped at the
    /// 64-byte `W0..W15` buffer.
    fn phase_bytes(&self, enable: u32, dlen_reg: u32) -> usize {
        if self.reg(USER_REG) & enable == 0 {
            return 0;
        }
        let bits = (self.reg(dlen_reg) & DLEN_BITLEN_MASK) as usize + 1;
        bits.div_ceil(8).min(64)
    }

    /// Stores received (MISO) bytes into `W0..` from byte 0, little-endian
    /// within each word (`spimem_flash_ll_get_buffer_data` reads them back
    /// as `data_buf[i]` words).
    fn store_miso(&mut self, bytes: &[u8]) {
        let w0 = (W0_REG >> 2) as usize;
        for (i, b) in bytes.iter().enumerate().take(64) {
            set_byte(&mut self.regs[w0 + i / 4], (i % 4) as u32, *b);
        }
    }

    fn run_user_transaction(&mut self, _flash: &mut EmulatedFlash) {
        self.transactions += 1;
        let user = self.reg(USER_REG);
        let command = if user & USER_USR_COMMAND != 0 {
            (self.reg(USER2_REG) & USER2_COMMAND_VALUE_MASK) as u16
        } else {
            // No command phase: nothing this model knows how to answer.
            self.log_unmodeled(NO_COMMAND_PHASE);
            return;
        };
        let miso_len = self.phase_bytes(USER_USR_MISO, MISO_DLEN_REG);
        match command {
            c if c == FLASH_CMD_RDID as u16 => {
                // Bytes past the 3-byte ID are left as they were: no longer
                // RDID read has been observed, and what a chip clocks out
                // after its ID is chip-specific.
                let n = miso_len.min(JEDEC_ID.len());
                self.store_miso(&JEDEC_ID[..n]);
            }
            other => self.log_unmodeled(other),
        }
    }

    fn log_unmodeled(&mut self, command: u16) {
        if self.unmodeled.len() >= UNMODELED_LOG_CAPACITY {
            self.unmodeled.pop_front();
        }
        self.unmodeled.push_back(command);
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

    // ---- SPIMEM1 (sub-unit 2) ----

    /// Writes `v` to `off` as 4 ascending byte writes, exactly like
    /// `FirmwareBus::write32` delivers a real `sw` (Review Focus 1).
    fn w(dev: &mut Spimem1, flash: &mut EmulatedFlash, off: u32, v: u32) {
        for (k, b) in v.to_le_bytes().iter().enumerate() {
            dev.write_byte(off + k as u32, *b, flash);
        }
    }

    fn r(dev: &Spimem1, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|k| dev.read_byte(off + k)))
    }

    /// `spimem_flash_ll_user_start` (`dev->cmd.val |= usr_pe`): a
    /// read-modify-write of `SPI_MEM_CMD_REG`, delivered as one `sw`.
    fn user_start(dev: &mut Spimem1, flash: &mut EmulatedFlash, pe_ops: bool) {
        let usr_pe = if pe_ops { 0x6_0000 } else { 0x4_0000 };
        let cmd = r(dev, CMD_REG);
        w(dev, flash, CMD_REG, cmd | usr_pe);
    }

    /// The register writes `spi_flash_hal_common_command` makes for
    /// `memspi_host_read_id_hs`'s RDID transaction (command 0x9F, no
    /// address, 3 MISO bytes), as literal values: `spimem_flash_ll_set_command`
    /// (USER.usr_command, USER2 = bitlen 7 | 0x9F), `set_addr_bitlen(0)`
    /// (USER1.usr_addr_bitlen = 0x3F, USER.usr_addr = 0),
    /// `set_miso_bitlen(24)` (USER.usr_miso, MISO_DLEN = 23), then
    /// `user_start(false)`.
    fn rdid_via_hal(dev: &mut Spimem1, flash: &mut EmulatedFlash) {
        w(dev, flash, USER2_REG, 0x7000_009F);
        w(dev, flash, USER1_REG, 0xFC00_0000);
        w(dev, flash, ADDR_REG, 0);
        w(dev, flash, MISO_DLEN_REG, 23);
        w(dev, flash, USER_REG, USER_USR_COMMAND | USER_USR_MISO);
        user_start(dev, flash, false);
    }

    #[test]
    fn rdid_returns_the_badges_jedec_id_in_w0() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        rdid_via_hal(&mut dev, &mut flash);
        // Received bytes 0x46, 0x40, 0x16 land in W0's bytes 0..3.
        assert_eq!(r(&dev, W0_REG) & 0x00FF_FFFF, 0x0016_4046);
        // memspi_host_read_id_hs's byte swap then yields 0x464016.
        let raw = r(&dev, W0_REG) & 0x00FF_FFFF;
        let id = ((raw & 0xFF) << 16) | (raw >> 16) | (raw & 0xFF00);
        assert_eq!(id, 0x46_4016);
    }

    #[test]
    fn usr_trigger_self_clears_so_cmd_is_done_polls_succeed() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        rdid_via_hal(&mut dev, &mut flash);
        // spimem_flash_ll_cmd_is_done: dev->cmd.val == 0.
        assert_eq!(r(&dev, CMD_REG), 0);
    }

    #[test]
    fn a_whole_word_cmd_write_fires_exactly_one_transaction() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        rdid_via_hal(&mut dev, &mut flash);
        assert_eq!(dev.transaction_count(), 1);
        // Writes to the CMD bytes that do not hold SPI_MEM_USR fire nothing.
        for k in [0u32, 1, 3] {
            dev.write_byte(CMD_REG + k, 0x04, &mut flash);
        }
        assert_eq!(dev.transaction_count(), 1);
    }

    #[test]
    fn the_bootloader_style_rdid_sequence_also_reads_the_id() {
        // bootloader_flash_execute_command_common (bootloader_support
        // bootloader_flash.c) as observed on the probe: CTRL = 0, CTRL.wp,
        // USER = usr_command, USER2 = 0x9F / bitlen 7, USER1 addr bitlen
        // 0x3F, USER cleared for dummy/mosi, ADDR = 0, MOSI_DLEN = 0,
        // USER.usr_miso, MISO_DLEN = 23, CMD |= USR.
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        w(&mut dev, &mut flash, CTRL_REG, 0);
        w(&mut dev, &mut flash, CTRL_REG, 0x0020_0000);
        w(&mut dev, &mut flash, USER_REG, USER_USR_COMMAND);
        w(&mut dev, &mut flash, USER2_REG, 0x0000_009F);
        w(&mut dev, &mut flash, USER2_REG, 0x7000_009F);
        w(&mut dev, &mut flash, USER1_REG, 0xFC00_0000);
        w(&mut dev, &mut flash, ADDR_REG, 0);
        w(&mut dev, &mut flash, MOSI_DLEN_REG, 0);
        w(
            &mut dev,
            &mut flash,
            USER_REG,
            USER_USR_COMMAND | USER_USR_MISO,
        );
        w(&mut dev, &mut flash, MISO_DLEN_REG, 23);
        user_start(&mut dev, &mut flash, false);
        assert_eq!(r(&dev, CMD_REG), 0);
        assert_eq!(r(&dev, W0_REG) & 0x00FF_FFFF, 0x0016_4046);
    }

    #[test]
    fn an_unmodeled_command_is_recorded_and_leaves_the_buffer_alone() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        w(&mut dev, &mut flash, W0_REG, 0x1234_5678);
        w(&mut dev, &mut flash, USER2_REG, 0x7000_00AB);
        w(&mut dev, &mut flash, MISO_DLEN_REG, 7);
        w(
            &mut dev,
            &mut flash,
            USER_REG,
            USER_USR_COMMAND | USER_USR_MISO,
        );
        user_start(&mut dev, &mut flash, false);
        assert_eq!(r(&dev, CMD_REG), 0, "still completes (never hangs)");
        assert_eq!(r(&dev, W0_REG), 0x1234_5678);
        assert_eq!(
            dev.unmodeled_commands().iter().copied().collect::<Vec<_>>(),
            vec![0xAB]
        );
    }

    #[test]
    fn registers_are_plain_storage_and_the_window_end_reads_zero() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        w(&mut dev, &mut flash, CLOCK_REG, 0x8000_0000);
        assert_eq!(r(&dev, CLOCK_REG), 0x8000_0000);
        w(&mut dev, &mut flash, W0_REG + 15 * 4, 0xCAFE_F00D);
        assert_eq!(r(&dev, W0_REG + 15 * 4), 0xCAFE_F00D);
        w(&mut dev, &mut flash, 0x3FC, 0xFFFF_FFFF);
        assert_eq!(r(&dev, 0x3FC), 0);
    }

    #[test]
    fn handles_covers_the_registers_this_model_names() {
        for off in [
            CMD_REG,
            ADDR_REG,
            USER_REG,
            USER1_REG,
            USER2_REG,
            MOSI_DLEN_REG,
            MISO_DLEN_REG,
            W0_REG,
            W0_REG + 60,
        ] {
            assert!(Spimem1::handles(off), "{off:#x}");
        }
        assert!(!Spimem1::handles(0x3FC));
    }
}

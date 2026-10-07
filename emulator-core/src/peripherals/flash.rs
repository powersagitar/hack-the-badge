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
//! - **Trigger.** Every bit of `CMD` from 17 to 31 is `R/W/SC` in
//!   `spi_mem_reg.h`: setting it starts an operation, and the controller
//!   clears it when the operation is done. A 32-bit store reaches this model
//!   as four ascending byte writes (Review Focus 1), so an operation fires
//!   on the write of **the byte holding its bit**, and only when that byte
//!   sets it: `SPI_MEM_USR` (bit 18) and `SPI_MEM_FLASH_BE`/`_CE`/`_DP`/
//!   `_RES`/`_HPM` (23..19) in byte 2, `SPI_MEM_FLASH_READ`/`_WREN`/`_WRDI`/
//!   `_RDID`/`_RDSR`/`_WRSR`/`_PP`/`_SE` (31..24) in byte 3. Every field an
//!   operation reads (`ADDR`, `USER*`, `MISO_DLEN`, `W0..`) was stored by
//!   earlier writes, so one `sw` runs exactly one operation.
//!   `spimem_flash_ll_user_start` sets `SPI_MEM_USR` (or `SPI_MEM_USR |
//!   SPI_MEM_FLASH_PE`, bits 18 and 17, for program/erase; `FLASH_PE`
//!   triggers nothing on its own); the dedicated commands are set one at a
//!   time by `spimem_flash_ll_set_write_protect` (`flash_wren`/
//!   `flash_wrdi`), `spimem_flash_ll_erase_sector` (`flash_se`) and
//!   `spimem_flash_ll_program_page` (`flash_pp`), called from
//!   `spi_flash_hal_set_write_protect`/`_erase_sector`/`_program_page`
//!   (`components/hal/spi_flash_hal_iram.c`), the host driver
//!   `ESP_FLASH_DEFAULT_HOST_DRIVER()` (`spi_flash/include/memspi_host_driver.h`)
//!   wires up for SPI1.
//! - **Completion.** Every operation completes within that same byte write;
//!   then its bit (and `SPI_MEM_FLASH_PE`) is cleared, so the register reads
//!   0 and `spimem_flash_ll_cmd_is_done` (`dev->cmd.val == 0`) is true on
//!   the first poll. (The status field `SPI_MEM_MST_ST` reads 0, idle, so
//!   `spimem_flash_ll_host_idle` is true too.)
//! - **The chip is never busy.** Erase and program finish instantly, so the
//!   status register's `SR_WIP` (bit 0, `spi_flash_defs.h`) always reads 0
//!   and `spi_flash_chip_generic_wait_idle` returns on its first `RDSR`.
//! - **Write enable latch.** `SR_WREN` ([`SR_WEL`], bit 1) is modeled because
//!   the generic chip driver checks it: `spi_flash_chip_generic_set_write_protect`
//!   reads it back after `WREN`/`WRDI` (and fails with `ESP_ERR_NOT_FOUND` if
//!   it did not change), and `spi_flash_chip_generic_wait_idle` treats a WEL
//!   still set after an erase or program as "command not accepted"
//!   (`ESP_ERR_NOT_SUPPORTED`). As on a SPI NOR chip, `FLASH_WREN` sets it,
//!   `FLASH_WRDI` clears it, and `FLASH_SE`/`FLASH_PP` run only while it is
//!   set and clear it.
//! - **`SPI_MEM_FLASH_SE` (bit 24)** erases the 4 KiB sector holding
//!   `ADDR[23:0]` (`spi_flash_hal_erase_sector`: 24-bit address phase,
//!   `ADDR = start_address & 0xFFFFFF`).
//! - **`SPI_MEM_FLASH_PP` (bit 25)** programs `ADDR[31:24]` bytes (the
//!   "byte length of a transfer" field of `SPI_MEM_USR_ADDR_VALUE`, set by
//!   `spi_flash_hal_program_page` as `address | length << 24`; at most the
//!   64-byte buffer) from `W0..` (packed little-endian by
//!   `spimem_flash_ll_set_buffer_data`) at `ADDR[23:0]`, with NOR AND
//!   semantics ([`EmulatedFlash::program`]), wrapping within the 256-byte
//!   page as a NOR page program does (the write slicer never crosses one).
//! - **Command phase.** With `SPI_MEM_USR_COMMAND` (`USER` bit 31) set, the
//!   command is `SPI_MEM_USR_COMMAND_VALUE` (`USER2` bits 15..0), as written
//!   by `spimem_flash_ll_set_command`.
//! - **Address phase.** With `SPI_MEM_USR_ADDR` (`USER` bit 30) set, the
//!   address is the low `SPI_MEM_USR_ADDR_BITLEN + 1` bits (`USER1` bits
//!   31..26, `spimem_flash_ll_set_addr_bitlen` stores `bitlen - 1`) of
//!   `ADDR` (`spimem_flash_ll_set_usr_address` stores the address
//!   unshifted). On the ESP32-C3 the read path's address phase is 24 bits:
//!   `SOC_SPI_PERIPH_SUPPORT_CONTROL_DUMMY_OUT` is 1 (`soc/soc_caps.h`), so
//!   `spi_flash_hal_configure_host_io_mode` sends DIO/QIO mode bits through
//!   the dummy phase instead of widening the address.
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
//! - **RDSR (`0x05`, `CMD_RDSR`)**, as `memspi_host_read_status_hs` issues it
//!   (one MISO byte), answers the status register: `SR_WIP` 0, [`SR_WEL`].
//! - **Reads** ([`FLASH_READ_COMMANDS`]: `CMD_READ` 0x03 and the fast reads
//!   0x0B/0x3B/0x6B/0xBB/0xEB that `spi_flash_chip_generic_config_host_io_mode`
//!   chooses by IO mode; the badge runs DIO, so it issues `CMD_FASTRD_DIO`
//!   0xBB through `spi_flash_hal_read`) return the flash bytes at the
//!   address phase's address, `0xFF` past the end of the chip. A read
//!   without an address phase is logged as unmodeled.
//! - **Any other command**, or a transaction without a command phase, or a
//!   dedicated bit other than `WREN`/`WRDI`/`SE`/`PP`, still completes (so no
//!   poll hangs) but has no effect on the buffer or the chip; it is recorded
//!   in [`Spimem1::unmodeled_commands`] ([`NO_COMMAND_PHASE`] for "no command
//!   phase", [`DEDICATED_COMMAND_BASE`]` | bit` for a dedicated bit), which
//!   `boot-probe` prints.
//!
//! **Observed write path (boot-probe and disassembly of `factory.bin`,
//! Milestone 4 Task 5)**: `esp_littlefs` formatting the blank `storage`
//! partition polls `CMD` in `spi_flash_hal_poll_cmd_done` (`0x4039_3916`)
//! after `spi_flash_hal_set_write_protect` (`0x4039_3e2a`) sets
//! `SPI_MEM_FLASH_WREN`; it then uses `spi_flash_hal_erase_sector`
//! (`0x4039_3d4a`) and `spi_flash_hal_program_page` (`0x4039_3ddc`), user
//! `RDSR` (0x05) for every status poll, and user `CMD_FASTRD_DIO` (0xBB)
//! reads. No other command is issued through the end of the format.
//!
//! Not modeled: the dummy and MOSI phases (no observed user command sends
//! data), `SPI_MEM_USR_MISO_HIGHPART`, the dedicated `FLASH_READ`, `RDID`,
//! `RDSR` (which would answer into `SPI_MEM_RD_STATUS_REG`), `WRSR`, `BE`
//! (its `spi_mem_reg.h` description says 32 KiB while ESP-IDF's generic
//! driver treats a block as 64 KiB, so it waits until observed), `CE`, `DP`,
//! `RES`, `HPM`, user-command `WREN`/`WRDI`/erase/program, clock and timing
//! registers (stored only), and auto-suspend (`SPI_MEM_FLASH_SUS_CTRL_REG`
//! stored only; `SPI_MEM_SUS_STATUS_REG`'s `FLASH_SUS` reads as stored, 0).

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
/// to. A constant rather than computed at run time (it was computed with
/// Python's `hashlib.md5` over [`EmulatedFlash::from_app_image`]'s own
/// serialized entries). A unit test recomputes it from [`PARTITIONS`] with
/// the crate's shared MD5 ([`crate::md5`], which the ROM MD5 stubs also
/// use), so editing a partition without updating it fails; it is also checked against the real
/// chip's table by `tests/flash_partition_table.rs` (gated on
/// `BADGE_FULL_DUMP`). The firmware also re-verifies it itself
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

/// `SPI_MEM_USR` (`SPI_MEM_CMD_REG` bit 18).
const CMD_USR: u32 = 1 << 18;
/// `SPI_MEM_FLASH_PE` (`SPI_MEM_CMD_REG` bit 17): set together with
/// `SPI_MEM_USR` by `spimem_flash_ll_user_start(dev, true)` and cleared with
/// it on completion; it triggers nothing on its own.
const CMD_FLASH_PE: u32 = 1 << 17;
/// `SPI_MEM_FLASH_READ` .. `SPI_MEM_FLASH_HPM` (`SPI_MEM_CMD_REG` bits
/// 31..19): the dedicated flash commands. Each is `R/W/SC`: setting it
/// starts the operation and the controller clears it when done.
const CMD_FLASH_WREN_BIT: u32 = 30;
const CMD_FLASH_WRDI_BIT: u32 = 29;
const CMD_FLASH_PP_BIT: u32 = 25;
const CMD_FLASH_SE_BIT: u32 = 24;
const CMD_FLASH_HPM_BIT: u32 = 19;
const CMD_FLASH_READ_BIT: u32 = 31;

/// `SPI_MEM_USR_COMMAND` (`SPI_MEM_USER_REG` bit 31).
pub const USER_USR_COMMAND: u32 = 1 << 31;
/// `SPI_MEM_USR_ADDR` (`SPI_MEM_USER_REG` bit 30).
pub const USER_USR_ADDR: u32 = 1 << 30;
/// `SPI_MEM_USR_MISO` (`SPI_MEM_USER_REG` bit 28).
pub const USER_USR_MISO: u32 = 1 << 28;

/// `SPI_MEM_USR_ADDR_BITLEN` (`SPI_MEM_USER1_REG` bits `[31:26]`).
const USER1_ADDR_BITLEN_SHIFT: u32 = 26;
const USER1_ADDR_BITLEN_MASK: u32 = 0x3F;
/// `SPI_MEM_USR_COMMAND_VALUE` (`SPI_MEM_USER2_REG` bits `[15:0]`).
const USER2_COMMAND_VALUE_MASK: u32 = 0xFFFF;
/// `SPI_MEM_USR_MISO_DBITLEN` (`SPI_MEM_MISO_DLEN_REG` bits `[9:0]`).
const DLEN_BITLEN_MASK: u32 = 0x3FF;
/// The 24-bit flash address in `SPI_MEM_ADDR_REG` for the dedicated
/// commands (bits `[23:0]`; bits `[31:24]` are the byte length, per the
/// register's description in `spi_mem_reg.h`).
const ADDR_24BIT_MASK: u32 = 0xFF_FFFF;
/// Size of `W0..W15`: the most bytes one transaction moves
/// (`SPI_FLASH_HAL_MAX_WRITE_BYTES`/`_READ_BYTES` = 64 in
/// `memspi_host_driver.c`).
const BUFFER_BYTES: usize = 64;
/// A NOR page: page program wraps within it.
const PAGE_SIZE: u32 = 256;

/// `CMD_RDID` (`spi_flash/include/spi_flash/spi_flash_defs.h`).
pub const FLASH_CMD_RDID: u8 = 0x9F;
/// `CMD_RDSR` (`spi_flash_defs.h`): read status register 1.
pub const FLASH_CMD_RDSR: u8 = 0x05;
/// The 24-bit-address read commands `spi_flash_chip_generic_config_host_io_mode`
/// picks from by IO mode (`spi_flash_defs.h`): `CMD_READ` 0x03,
/// `CMD_FASTRD` 0x0B, `CMD_FASTRD_DUAL` 0x3B, `CMD_FASTRD_QUAD` 0x6B,
/// `CMD_FASTRD_DIO` 0xBB, `CMD_FASTRD_QIO` 0xEB. They differ only in line
/// width and dummy cycles, which do not change the bytes returned.
pub const FLASH_READ_COMMANDS: [u8; 6] = [0x03, 0x0B, 0x3B, 0x6B, 0xBB, 0xEB];
/// `SR_WREN` (`spi_flash_defs.h`): status register bit 1, the write enable
/// latch (WEL). `SR_WIP` (bit 0) is always 0 here (see the module doc).
pub const SR_WEL: u8 = 1 << 1;
/// The badge's flash JEDEC ID, in the order the chip sends it:
/// manufacturer `0x46`, then device ID `0x40 0x16` (milestone hardware
/// facts; `memspi: chip_id` would log it as `0x464016`).
pub const JEDEC_ID: [u8; 3] = [0x46, 0x40, 0x16];

/// What [`Spimem1::unmodeled_commands`] records for a transaction with
/// `SPI_MEM_USR_COMMAND` clear (no command phase at all). Real command
/// values are at most 16 bits wide (`SPI_MEM_USR_COMMAND_VALUE`), and no
/// flash command is `0xFFFF`, so this cannot collide with an observed one.
pub const NO_COMMAND_PHASE: u16 = 0xFFFF;

/// What [`Spimem1::unmodeled_commands`] records for a dedicated
/// `SPI_MEM_FLASH_*` command bit with no modeled effect: this base OR'd with
/// the bit's position in `SPI_MEM_CMD_REG` (19..=31, so `0xFF13..=0xFF1F`).
/// The on-wire opcode the controller sends for those bits is not documented
/// in `spi_mem_reg.h`, so the bit position is what is recorded.
pub const DEDICATED_COMMAND_BASE: u16 = 0xFF00;

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
    /// The flash chip's write enable latch (status bit [`SR_WEL`]). Chip
    /// state, kept here because the chip is otherwise plain storage
    /// ([`EmulatedFlash`]) and only this controller reaches it.
    wel: bool,
}

impl Default for Spimem1 {
    fn default() -> Self {
        Self {
            regs: [0; SPIMEM_REGS_WORDS],
            wel: false,
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
        if offset & !0b11 != CMD_REG {
            return;
        }
        // Only the bits this byte sets can fire (Review Focus 1): the
        // fields a command reads were stored by earlier writes, and the
        // other bytes of the same word never re-trigger it.
        let set_now = (val as u32) << (idx * 8);
        if set_now & CMD_USR != 0 {
            self.run_user_transaction(flash);
            self.regs[widx] &= !(CMD_USR | CMD_FLASH_PE);
        }
        for bit in (CMD_FLASH_HPM_BIT..=CMD_FLASH_READ_BIT).rev() {
            if set_now & (1 << bit) != 0 {
                self.run_dedicated_command(bit, flash);
                self.regs[widx] &= !(1 << bit);
            }
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

    /// `true` iff the chip's write enable latch is set (status bit
    /// [`SR_WEL`]).
    pub fn write_enabled(&self) -> bool {
        self.wel
    }

    /// The chip's status register 1 as `RDSR` returns it: `SR_WIP` (bit 0)
    /// is always 0 (never busy), [`SR_WEL`] (bit 1) is the latch.
    fn status(&self) -> u8 {
        if self.wel {
            SR_WEL
        } else {
            0
        }
    }

    /// Runs the dedicated command for `SPI_MEM_CMD_REG` bit `bit` (see the
    /// module doc's `SPI_MEM_FLASH_SE`/`_PP` and "Write enable latch"
    /// items). It completes at once; the caller clears the bit.
    fn run_dedicated_command(&mut self, bit: u32, flash: &mut EmulatedFlash) {
        let addr = self.reg(ADDR_REG) & ADDR_24BIT_MASK;
        match bit {
            CMD_FLASH_WREN_BIT => self.wel = true,
            CMD_FLASH_WRDI_BIT => self.wel = false,
            CMD_FLASH_SE_BIT => {
                if std::mem::take(&mut self.wel) {
                    flash.erase_sector(addr);
                }
            }
            CMD_FLASH_PP_BIT => {
                if std::mem::take(&mut self.wel) {
                    let len = ((self.reg(ADDR_REG) >> 24) as usize).min(BUFFER_BYTES);
                    let page = addr & !(PAGE_SIZE - 1);
                    for i in 0..len {
                        // A page program wraps within its 256-byte page.
                        let a = page | (addr.wrapping_add(i as u32) & (PAGE_SIZE - 1));
                        flash.program(a, &[self.buffer_byte(i)]);
                    }
                }
            }
            other => self.log_unmodeled(DEDICATED_COMMAND_BASE | other as u16),
        }
    }

    /// Byte `i` of `W0..W15` (little-endian within each word, the order
    /// `spimem_flash_ll_set_buffer_data` packs them in).
    fn buffer_byte(&self, i: usize) -> u8 {
        self.regs[(W0_REG >> 2) as usize + i / 4].to_le_bytes()[i % 4]
    }

    /// The address-phase value of a user transaction: the low
    /// `SPI_MEM_USR_ADDR_BITLEN + 1` bits of `ADDR`, or `None` with
    /// `SPI_MEM_USR_ADDR` clear.
    fn user_address(&self) -> Option<u32> {
        if self.reg(USER_REG) & USER_USR_ADDR == 0 {
            return None;
        }
        let bits = ((self.reg(USER1_REG) >> USER1_ADDR_BITLEN_SHIFT) & USER1_ADDR_BITLEN_MASK) + 1;
        let mask = if bits >= 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        Some(self.reg(ADDR_REG) & mask)
    }

    /// Runs one `SPI_MEM_USR` transaction against `flash` (see the module
    /// doc).
    fn run_user_transaction(&mut self, flash: &mut EmulatedFlash) {
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
            c if c == FLASH_CMD_RDSR as u16 => {
                // One status byte; only 1-byte reads are observed.
                let status = [self.status()];
                self.store_miso(&status[..miso_len.min(1)]);
            }
            c if c <= 0xFF && FLASH_READ_COMMANDS.contains(&(c as u8)) => {
                let Some(addr) = self.user_address() else {
                    self.log_unmodeled(c);
                    return;
                };
                let data: Vec<u8> = (0..miso_len as u32)
                    .map(|i| addr.checked_add(i).map_or(0xFF, |a| flash.read(a)))
                    .collect();
                self.store_miso(&data);
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

    /// Guards [`PARTITION_TABLE_MD5`] against going stale: it must equal the
    /// MD5 of the bytes the [`PARTITIONS`] entries serialize to (what
    /// ESP-IDF's `esp_partition` checks when `CONFIG_PARTITION_TABLE_MD5` is
    /// on), so an edit to a partition without updating the constant fails
    /// here instead of in the firmware.
    #[test]
    fn partition_table_md5_constant_is_the_md5_of_the_serialized_entries() {
        let f = EmulatedFlash::from_app_image(&[]);
        let entries: Vec<u8> = (0..PARTITIONS.len() as u32 * 32)
            .map(|k| f.read(PARTITION_TABLE_OFFSET + k))
            .collect();
        assert_eq!(crate::md5::md5(&entries), PARTITION_TABLE_MD5);
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

    // ---- SPIMEM1 erase/program/read (Milestone 4 Task 5) ----
    //
    // Each helper replays the register writes of one HAL function in its
    // source order (`spi_flash_hal_iram.c` / `spi_flash_hal_common.inc` /
    // `spimem_flash_ll.h`, v5.5.3), every store as a whole word delivered
    // as 4 ascending bytes. Bit positions are `spi_mem_reg.h`'s.

    /// `reg = (reg & and) | or`, as one load and one `sw` (a bitfield
    /// assignment in an LL function).
    fn rmw(dev: &mut Spimem1, flash: &mut EmulatedFlash, off: u32, and: u32, or: u32) {
        let v = r(dev, off);
        w(dev, flash, off, (v & and) | or);
    }

    /// `spimem_flash_ll_set_addr_bitlen(dev, 24)`: `USER1.usr_addr_bitlen`
    /// (bits 31:26) = 23, then `USER.usr_addr` (bit 30) = 1.
    fn set_addr_bitlen_24(dev: &mut Spimem1, flash: &mut EmulatedFlash) {
        rmw(dev, flash, USER1_REG, 0x03FF_FFFF, 23 << 26);
        rmw(dev, flash, USER_REG, !0, 1 << 30);
    }

    /// `spi_flash_hal_set_write_protect(host, wp)`: `cmd.flash_wrdi = 1`
    /// (bit 29) or `cmd.flash_wren = 1` (bit 30), then `poll_cmd_done`.
    fn set_write_protect(dev: &mut Spimem1, flash: &mut EmulatedFlash, wp: bool) {
        rmw(dev, flash, CMD_REG, !0, if wp { 1 << 29 } else { 1 << 30 });
    }

    /// `spi_flash_hal_erase_sector`: addr bitlen 24, `ADDR = start &
    /// 0xFFFFFF`, then `spimem_flash_ll_erase_sector` (`ctrl.val = 0`,
    /// `cmd.flash_se = 1`, bit 24).
    fn erase_sector_via_hal(dev: &mut Spimem1, flash: &mut EmulatedFlash, start: u32) {
        set_addr_bitlen_24(dev, flash);
        w(dev, flash, ADDR_REG, start & 0xFF_FFFF);
        w(dev, flash, CTRL_REG, 0);
        rmw(dev, flash, CMD_REG, !0, 1 << 24);
    }

    /// `spi_flash_hal_program_page`: addr bitlen 24, `ADDR = (address &
    /// 0xFFFFFF) | (length << 24)`, then `spimem_flash_ll_program_page`
    /// (`user.usr_dummy = 0` (bit 29), `set_buffer_data` into `W0..`,
    /// `cmd.flash_pp = 1`, bit 25).
    fn program_page_via_hal(dev: &mut Spimem1, flash: &mut EmulatedFlash, addr: u32, data: &[u8]) {
        set_addr_bitlen_24(dev, flash);
        w(
            dev,
            flash,
            ADDR_REG,
            (addr & 0xFF_FFFF) | ((data.len() as u32) << 24),
        );
        rmw(dev, flash, USER_REG, !(1 << 29), 0);
        for (i, chunk) in data.chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            w(dev, flash, W0_REG + 4 * i as u32, u32::from_le_bytes(word));
        }
        rmw(dev, flash, CMD_REG, !0, 1 << 25);
    }

    /// `spi_flash_hal_configure_host_io_mode(command, addr_bitlen, 0, ..)`
    /// (`set_command`, `set_addr_bitlen`, `set_dummy(0)`, MISO/MOSI off).
    fn configure_host_io_mode(
        dev: &mut Spimem1,
        flash: &mut EmulatedFlash,
        command: u32,
        addr_bitlen: u32,
    ) {
        rmw(dev, flash, USER_REG, !0, USER_USR_COMMAND);
        rmw(dev, flash, USER2_REG, 0xFFFF_0000, command);
        rmw(dev, flash, USER2_REG, 0x0FFF_FFFF, 7 << 28);
        let bitlen_field = addr_bitlen.wrapping_sub(1) & 0x3F;
        rmw(dev, flash, USER1_REG, 0x03FF_FFFF, bitlen_field << 26);
        let usr_addr = if addr_bitlen > 0 { 1 << 30 } else { 0 };
        rmw(dev, flash, USER_REG, !(1 << 30), usr_addr);
        rmw(dev, flash, USER_REG, !(1 << 29), 0); // usr_dummy = 0
        rmw(dev, flash, USER_REG, !USER_USR_MISO, 0);
        w(dev, flash, MISO_DLEN_REG, 0);
        rmw(dev, flash, USER_REG, !(1 << 27), 0); // usr_mosi = 0
        w(dev, flash, MOSI_DLEN_REG, 0);
    }

    /// `memspi_host_read_status_hs`: `spi_flash_hal_common_command` with
    /// `CMD_RDSR` (0x05) and one MISO byte; returns that byte.
    fn rdsr_via_hal(dev: &mut Spimem1, flash: &mut EmulatedFlash) -> u8 {
        configure_host_io_mode(dev, flash, 0x05, 0);
        w(dev, flash, ADDR_REG, 0);
        rmw(dev, flash, USER_REG, !0, USER_USR_MISO);
        w(dev, flash, MISO_DLEN_REG, 7);
        user_start(dev, flash, false);
        assert_eq!(r(dev, CMD_REG), 0, "RDSR completes");
        r(dev, W0_REG) as u8
    }

    /// `spi_flash_hal_read` after `configure_host_io_mode(CMD_FASTRD_DIO
    /// 0xBB, 24, ..)`: `ADDR = address`, `set_miso_bitlen(len * 8)`,
    /// `user_start(false)`.
    fn dio_read_via_hal(dev: &mut Spimem1, flash: &mut EmulatedFlash, addr: u32, len: u32) {
        configure_host_io_mode(dev, flash, 0xBB, 24);
        w(dev, flash, ADDR_REG, addr);
        rmw(dev, flash, USER_REG, !0, USER_USR_MISO);
        w(dev, flash, MISO_DLEN_REG, len * 8 - 1);
        user_start(dev, flash, false);
    }

    #[test]
    fn wren_and_wrdi_self_clear_and_toggle_the_write_enable_latch() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        assert_eq!(rdsr_via_hal(&mut dev, &mut flash), 0);
        set_write_protect(&mut dev, &mut flash, false);
        assert_eq!(r(&dev, CMD_REG), 0, "SPI_MEM_FLASH_WREN self-clears");
        assert_eq!(rdsr_via_hal(&mut dev, &mut flash), SR_WEL);
        set_write_protect(&mut dev, &mut flash, true);
        assert_eq!(r(&dev, CMD_REG), 0, "SPI_MEM_FLASH_WRDI self-clears");
        assert_eq!(rdsr_via_hal(&mut dev, &mut flash), 0);
    }

    #[test]
    fn rdsr_is_never_busy() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        set_write_protect(&mut dev, &mut flash, false);
        erase_sector_via_hal(&mut dev, &mut flash, 0x9000);
        // SR_WIP (bit 0) clear straight after the erase.
        assert_eq!(rdsr_via_hal(&mut dev, &mut flash) & 1, 0);
    }

    #[test]
    fn sector_erase_blanks_the_4k_sector_self_clears_and_resets_wel() {
        let app = vec![0x00u8; 0x3000];
        let mut flash = EmulatedFlash::from_app_image(&app);
        let mut dev = Spimem1::new();
        set_write_protect(&mut dev, &mut flash, false);
        erase_sector_via_hal(&mut dev, &mut flash, APP_OFFSET + 0x1000);
        assert_eq!(r(&dev, CMD_REG), 0, "SPI_MEM_FLASH_SE self-clears");
        assert_eq!(flash.read(APP_OFFSET + 0x0FFF), 0x00);
        assert_eq!(flash.read(APP_OFFSET + 0x1000), 0xFF);
        assert_eq!(flash.read(APP_OFFSET + 0x1FFF), 0xFF);
        assert_eq!(flash.read(APP_OFFSET + 0x2000), 0x00);
        assert_eq!(rdsr_via_hal(&mut dev, &mut flash), 0, "WEL cleared");
    }

    #[test]
    fn sector_erase_without_write_enable_is_ignored() {
        let app = vec![0x00u8; 0x1000];
        let mut flash = EmulatedFlash::from_app_image(&app);
        let mut dev = Spimem1::new();
        erase_sector_via_hal(&mut dev, &mut flash, APP_OFFSET);
        assert_eq!(r(&dev, CMD_REG), 0, "still completes");
        assert_eq!(flash.read(APP_OFFSET), 0x00, "chip ignored it");
    }

    #[test]
    fn page_program_ands_the_buffer_into_flash_for_the_programmed_length() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        // Stale buffer bytes past the length must not be programmed.
        w(&mut dev, &mut flash, W0_REG + 4, 0);
        set_write_protect(&mut dev, &mut flash, false);
        program_page_via_hal(
            &mut dev,
            &mut flash,
            0x9010,
            &[0x12, 0x34, 0x56, 0x78, 0x0F],
        );
        assert_eq!(r(&dev, CMD_REG), 0, "SPI_MEM_FLASH_PP self-clears");
        let got: Vec<u8> = (0x900F..0x9016).map(|a| flash.read(a)).collect();
        assert_eq!(got, vec![0xFF, 0x12, 0x34, 0x56, 0x78, 0x0F, 0xFF]);
        assert_eq!(rdsr_via_hal(&mut dev, &mut flash), 0, "WEL cleared");
        // NOR: a second program only clears bits.
        set_write_protect(&mut dev, &mut flash, false);
        program_page_via_hal(&mut dev, &mut flash, 0x9010, &[0xF0]);
        assert_eq!(flash.read(0x9010), 0x10);
    }

    #[test]
    fn page_program_without_write_enable_is_ignored() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        program_page_via_hal(&mut dev, &mut flash, 0x9000, &[0x00]);
        assert_eq!(r(&dev, CMD_REG), 0);
        assert_eq!(flash.read(0x9000), 0xFF);
    }

    #[test]
    fn dio_fast_read_returns_flash_bytes_in_w_little_endian() {
        let mut flash = EmulatedFlash::from_app_image(&[0xE9, 0x06, 0x02, 0x20, 0xAB, 0xCD]);
        let mut dev = Spimem1::new();
        w(&mut dev, &mut flash, W0_REG + 4, 0x1111_1111);
        dio_read_via_hal(&mut dev, &mut flash, APP_OFFSET, 6);
        assert_eq!(r(&dev, CMD_REG), 0);
        assert_eq!(r(&dev, W0_REG), 0x2002_06E9);
        // Bytes past the 6 received keep their old value.
        assert_eq!(r(&dev, W0_REG + 4), 0x1111_CDAB);
        assert!(dev.unmodeled_commands().is_empty());
    }

    #[test]
    fn a_64_byte_read_fills_w0_to_w15() {
        let app: Vec<u8> = (0..64u8).collect();
        let mut flash = EmulatedFlash::from_app_image(&app);
        let mut dev = Spimem1::new();
        dio_read_via_hal(&mut dev, &mut flash, APP_OFFSET, 64);
        assert_eq!(r(&dev, W0_REG), 0x0302_0100);
        assert_eq!(r(&dev, W15_REG), 0x3F3E_3D3C);
    }

    #[test]
    fn out_of_range_erase_program_and_read_never_panic() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        set_write_protect(&mut dev, &mut flash, false);
        erase_sector_via_hal(&mut dev, &mut flash, 0xFF_F000);
        set_write_protect(&mut dev, &mut flash, false);
        program_page_via_hal(&mut dev, &mut flash, 0xFF_FFFE, &[0; 8]);
        dio_read_via_hal(&mut dev, &mut flash, 0xFF_FFFE, 4);
        assert_eq!(r(&dev, W0_REG), 0xFFFF_FFFF, "past the chip reads blank");
        assert_eq!(r(&dev, CMD_REG), 0);
    }

    #[test]
    fn an_unmodeled_dedicated_command_is_recorded_and_still_self_clears() {
        let mut flash = EmulatedFlash::from_app_image(&[]);
        let mut dev = Spimem1::new();
        // SPI_MEM_FLASH_HPM (bit 19).
        rmw(&mut dev, &mut flash, CMD_REG, !0, 1 << 19);
        assert_eq!(r(&dev, CMD_REG), 0);
        assert_eq!(
            dev.unmodeled_commands().iter().copied().collect::<Vec<_>>(),
            vec![DEDICATED_COMMAND_BASE | 19]
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

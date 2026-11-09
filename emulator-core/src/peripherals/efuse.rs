//! ESP32-C3 eFuse controller (`DR_REG_EFUSE_BASE = 0x6000_8800`): only the
//! factory MAC, and only when the host sets one (Milestone 10 Task 4).
//!
//! ## Why
//!
//! NimBLE's host-sync callback builds the badge's BLE random address from
//! `esp_read_mac(…, ESP_MAC_WIFI_STA)` (`docs/milestone-10-decisions.md`,
//! "Measurements"). On ESP-IDF that is the eFuse factory MAC, read from
//! block 1's read registers. Each emulated badge needs its own radio
//! identity, so the host can give a runtime one
//! (`crate::runtime::FirmwareRuntime::set_factory_mac`), as a real chip
//! carries its own. On v392 that read is made by the address builder at
//! `0x42010344`, which the sync callback calls: it reads the MAC with
//! `esp_read_mac(buf, 0)` and ORs `0xC0` into byte 5 to make the random
//! address (`docs/milestone-10-decisions.md`, "hal_radio's init").
//!
//! ## Unset: exactly as before
//!
//! Before this module the eFuse page fell inside the RTC_CNTL range, past
//! RTC_CNTL's modeled words: every read returned 0, every write was
//! dropped, and every access was logged as unmapped. With no MAC set this
//! module does exactly that ([`Efuse::handles`] is `false` for every
//! offset), so every boot without a MAC is byte-identical to before.
//!
//! ## Set: the MAC in block 1's layout
//!
//! With a MAC set, the two words that hold it read back in the eFuse
//! layout, and every other offset stays as above. Sources, ESP-IDF v5.5.3
//! (`https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`):
//!
//! - `components/soc/esp32c3/register/soc/reg_base.h`, line 34:
//!   `DR_REG_EFUSE_BASE 0x60008800` (line 26 `DR_REG_RTCCNTL_BASE
//!   0x60008000`, line 27 `DR_REG_IO_MUX_BASE 0x60009000`, so the block is
//!   `0x6000_8800..0x6000_9000`; `crate::mem::soc::EFUSE_RANGE`).
//! - `components/soc/esp32c3/register/soc/efuse_reg.h`, lines 580 to 602:
//!   `EFUSE_RD_MAC_SPI_SYS_0_REG` (`+0x44`), field `EFUSE_MAC_0`, bits
//!   `[31:0]`, "the low 32 bits of MAC address"; `EFUSE_RD_MAC_SPI_SYS_1_REG`
//!   (`+0x48`), field `EFUSE_MAC_1`, bits `[15:0]`, "the high 16 bits of MAC
//!   address". The rest of `SYS_1` (`EFUSE_SPI_PAD_CONFIG_*` from bit 16)
//!   reads 0, as it did before. The highest register is `EFUSE_DATE_REG`
//!   (`+0x1fc`, line 2626).
//! - `components/efuse/esp32c3/esp_efuse_utility.c`, line 24: block 1
//!   (`EFUSE_BLK1`) is read from `EFUSE_RD_MAC_SPI_SYS_0_REG` on, so bit
//!   `n` of block 1 is bit `n % 32` of word `SYS_0 + 4 * (n / 32)`.
//! - `components/efuse/esp32c3/esp_efuse_table.csv`, lines 147 to 152 (and
//!   `esp_efuse_table.c`'s `MAC[]`, lines 550 to 557): `MAC_FACTORY` is six
//!   8-bit fields of `EFUSE_BLK1` at bit offsets 40, 32, 24, 16, 8, 0, in
//!   that order. `esp_efuse_read_field_blob` packs the fields into the
//!   destination in table order, so `mac[0]` is block-1 bits `[47:40]`.
//!
//! Together: `mac[0]` is `SYS_1[15:8]`, `mac[1]` is `SYS_1[7:0]`, `mac[2]`
//! is `SYS_0[31:24]`, `mac[3]` `SYS_0[23:16]`, `mac[4]` `SYS_0[15:8]` and
//! `mac[5]` `SYS_0[7:0]`, so `02:00:00:00:10:01` reads as `SYS_0 =
//! 0x0000_1001`, `SYS_1 = 0x0000_0200`. No CRC is checked on this chip
//! (`components/esp_hw_support/mac_addr.c`, `get_efuse_factory_mac`, line
//! 269: the CRC check is `CONFIG_IDF_TARGET_ESP32` only).
//!
//! The registers are read-only (`RO` in `efuse_reg.h`); writes are dropped.
//! Programming eFuses is not modeled.
//!
//! ## The firmware reads the MAC once per boot
//!
//! `esp_read_mac` caches what it reads in `s_mac_table`
//! (`mac_addr.c`, `get_mac_addr_from_mac_table`, line 114, and
//! `esp_read_mac`, line 354), so a MAC set after the firmware's first read
//! takes effect at the next boot (a soft or cold reset), as on a real chip
//! whose MAC never changes. On v392 the first read is in provisioning, long
//! before any radio app opens (`docs/milestone-10-decisions.md`).

/// `EFUSE_RD_MAC_SPI_SYS_0_REG` (`efuse_reg.h` v5.5.3, `+0x44`): the low
/// 32 bits of the factory MAC.
pub const RD_MAC_SPI_SYS_0_REG: u32 = 0x44;
/// `EFUSE_RD_MAC_SPI_SYS_1_REG` (`+0x48`): the MAC's high 16 bits in
/// `[15:0]`.
pub const RD_MAC_SPI_SYS_1_REG: u32 = 0x48;

/// The eFuse controller: the host-set factory MAC, if any (module doc).
#[derive(Clone, Default)]
pub struct Efuse {
    factory_mac: Option<[u8; 6]>,
}

impl Efuse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the factory MAC the MAC registers report.
    pub fn set_factory_mac(&mut self, mac: [u8; 6]) {
        self.factory_mac = Some(mac);
    }

    /// The host-set factory MAC, if any.
    pub fn factory_mac(&self) -> Option<[u8; 6]> {
        self.factory_mac
    }

    /// `true` iff `offset` is one of the two MAC words and a MAC is set.
    /// `crate::mem::bus::FirmwareBus` logs every other access as unmapped,
    /// as it did before this module (module doc, "Unset").
    pub fn handles(&self, offset: u32) -> bool {
        self.factory_mac.is_some()
            && matches!(offset & !0b11, RD_MAC_SPI_SYS_0_REG | RD_MAC_SPI_SYS_1_REG)
    }

    /// The word at `offset` (word-aligned): the MAC words when a MAC is
    /// set, 0 everywhere else.
    fn word(&self, offset: u32) -> u32 {
        let Some(m) = self.factory_mac else {
            return 0;
        };
        match offset {
            RD_MAC_SPI_SYS_0_REG => u32::from_be_bytes([m[2], m[3], m[4], m[5]]),
            RD_MAC_SPI_SYS_1_REG => u32::from(u16::from_be_bytes([m[0], m[1]])),
            _ => 0,
        }
    }

    pub fn read_byte(&self, offset: u32) -> u8 {
        self.word(offset & !0b11).to_le_bytes()[(offset & 0b11) as usize]
    }

    /// Every eFuse register this module serves is read-only, and the rest
    /// drop writes as before: a write changes nothing.
    pub fn write_byte(&mut self, _offset: u32, _val: u8) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::bus::FirmwareBus;
    use crate::mem::soc::EFUSE_RANGE;
    use crate::mem::Bus;
    use std::sync::Arc;

    const HACKER_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x10, 0x01];

    fn bus() -> FirmwareBus {
        FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[])
    }

    /// With no MAC set, the whole eFuse block reads 0, drops writes and
    /// logs every access as unmapped, as before this module.
    #[test]
    fn unset_mac_reads_as_before() {
        let mut bus = bus();
        assert_eq!(bus.efuse.factory_mac(), None);
        for addr in (EFUSE_RANGE.start..EFUSE_RANGE.end).step_by(4) {
            bus.write32(addr, 0xffff_ffff);
            assert_eq!(bus.read32(addr), 0, "0x{addr:08x}");
        }
        for reg in [RD_MAC_SPI_SYS_0_REG, RD_MAC_SPI_SYS_1_REG] {
            let addr = EFUSE_RANGE.start + reg;
            let before = bus.unmapped_log().len();
            assert_eq!(bus.read8(addr), 0);
            let log = bus.unmapped_log();
            assert!(log.len() > before || log.len() == crate::mem::bus::UNMAPPED_LOG_CAPACITY);
            let last = log.back().unwrap();
            assert_eq!((last.addr, last.is_write), (addr, false));
        }
    }

    /// A set MAC reads back in `MAC_FACTORY`'s layout (module doc):
    /// `02:00:00:00:10:01` is `SYS_0 = 0x0000_1001`, `SYS_1 = 0x0000_0200`.
    /// Writes do not change it; every other word still reads 0, logged.
    #[test]
    fn set_mac_is_served_in_efuse_layout() {
        let mut bus = bus();
        bus.efuse.set_factory_mac(HACKER_MAC);
        let sys0 = EFUSE_RANGE.start + RD_MAC_SPI_SYS_0_REG;
        let sys1 = EFUSE_RANGE.start + RD_MAC_SPI_SYS_1_REG;
        assert_eq!(bus.read32(sys0), 0x0000_1001);
        assert_eq!(bus.read32(sys1), 0x0000_0200);
        bus.write32(sys0, 0);
        bus.write32(sys1, 0xffff_ffff);
        assert_eq!(bus.read32(sys0), 0x0000_1001);
        assert_eq!(bus.read32(sys1), 0x0000_0200);
        assert!(bus.unmapped_log().is_empty(), "the MAC words are mapped");

        // Every byte of a MAC with no two equal bytes lands where the
        // table puts it.
        let mut bus = self::bus();
        bus.efuse
            .set_factory_mac([0xa0, 0xb1, 0xc2, 0xd3, 0xe4, 0xf5]);
        assert_eq!(bus.read32(sys0), 0xc2d3_e4f5);
        assert_eq!(bus.read32(sys1), 0x0000_a0b1);
        assert_eq!(bus.read8(sys1 + 1), 0xa0, "mac[0] is SYS_1[15:8]");
        assert_eq!(bus.read8(sys0), 0xf5, "mac[5] is SYS_0[7:0]");
        assert_eq!(bus.read32(EFUSE_RANGE.start + 0x4c), 0, "SYS_2");
        assert!(bus
            .unmapped_log()
            .iter()
            .any(|a| a.addr == EFUSE_RANGE.start + 0x4c));
    }
}

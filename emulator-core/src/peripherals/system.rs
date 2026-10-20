//! SYSTEM peripheral (`DR_REG_SYSTEM_BASE = 0x600c_0000`): the four
//! `SYSTEM_CPU_INTR_FROM_CPU_<n>_REG` software-interrupt registers and the
//! two CPU-clock registers, `SYSTEM_CPU_PER_CONF_REG` and
//! `SYSTEM_SYSCLK_CONF_REG` (Milestone 6 Task 2).
//!
//! Sources (ESP-IDF v5.5.3):
//! - `components/soc/esp32c3/register/soc/system_reg.h`:
//!   `SYSTEM_CPU_INTR_FROM_CPU_0_REG` = base + `0x028`, `_1` = `0x02C`,
//!   `_2` = `0x030`, `_3` = `0x034`; each has one R/W field, bit 0
//!   (`SYSTEM_CPU_INTR_FROM_CPU_n`, default 0).
//! - `components/soc/esp32c3/include/soc/interrupts.h`:
//!   `ETS_FROM_CPU_INTR0..3_SOURCE` ("level"), numbered 50..=53 (see
//!   `crate::mem::soc::SRC_FROM_CPU_INTR0`).
//! - `components/hal/esp32c3/include/hal/crosscore_int_ll.h`:
//!   `crosscore_int_ll_trigger_interrupt` writes `1` to `FROM_CPU_0_REG`,
//!   `_clear_interrupt` writes `0`, `_get_state` reads it back. The FreeRTOS
//!   RISC-V port's `vPortYield`
//!   (`components/freertos/FreeRTOS-Kernel/portable/riscv/port.c`) calls
//!   `esp_crosscore_int_send_yield` (that trigger) and then spins until the
//!   ISR's clear makes `get_state` read 0 -- so the register must read back
//!   what was written and de-assert on a `0` write.
//!
//! Model: bit 0 of each register is a **level** interrupt source, asserted
//! while it reads 1 (no latching beyond the register itself). The other 31
//! bits are not modeled and read 0. Every other SYSTEM register except the
//! two clock registers below keeps the bus's logged catch-all behavior (see
//! `FirmwareBus`).
//!
//! Byte-split writes: `FirmwareBus::write32` arrives as four ascending byte
//! writes. Only byte 0 carries bit 0, so the level changes exactly on that
//! byte's write; the other three bytes are dropped.
//!
//! ## CPU-clock registers
//!
//! Sources (ESP-IDF v5.5.3):
//! - `system_reg.h`: `SYSTEM_CPU_PER_CONF_REG` = base + `0x008`:
//!   `CPU_WAITI_DELAY_NUM` [7:4] R/W default 0, `CPU_WAIT_MODE_FORCE_ON`
//!   [3] R/W default 1, `PLL_FREQ_SEL` [2] R/W default 1, `CPUPERIOD_SEL`
//!   [1:0] R/W default 0 (reset value `0x0c`). `SYSTEM_SYSCLK_CONF_REG` =
//!   base + `0x058`: `CLK_DIV_EN` [19] RO, `CLK_XTAL_FREQ` [18:12] RO,
//!   `SOC_CLK_SEL` [11:10] R/W default 0, `PRE_DIV_CNT` [9:0] R/W default 1
//!   (reset value `0x01`).
//! - `components/hal/esp32c3/include/hal/clk_tree_ll.h`:
//!   `clk_ll_cpu_get_src` maps `SOC_CLK_SEL` 0 = XTAL, 1 = PLL,
//!   2 = RC_FAST, anything else = invalid; `clk_ll_cpu_get_freq_mhz_from_pll`
//!   maps `CPUPERIOD_SEL` 0 = 80 MHz, 1 = 160 MHz, anything else = invalid;
//!   `clk_ll_cpu_get_divider` = `PRE_DIV_CNT + 1`.
//! - `components/esp_hw_support/port/esp32c3/rtc_clk.c`,
//!   `rtc_clk_cpu_freq_get_config`: XTAL gives `xtal_mhz / (PRE_DIV_CNT +
//!   1)`, PLL gives 80 or 160 MHz by `CPUPERIOD_SEL` (and aborts on any
//!   other), RC_FAST gives 20 MHz, and any other source aborts.
//!
//! Model: both registers are stored and read back through their R/W masks
//! (`0xff` and `0x0fff`); the RO fields (`CLK_XTAL_FREQ`, `CLK_DIV_EN`)
//! read 0 and ignore writes, as do the reserved bits. [`System::cpu_hz`]
//! decodes the registers as `rtc_clk_cpu_freq_get_config` does, with the
//! crystal at [`crate::mem::soc::XTAL_HZ`], but in Hz: that function works
//! in whole MHz, so a divider that does not divide 40 MHz evenly
//! (`PRE_DIV_CNT` 2) gives 13,333,333 Hz here where it reports 13 MHz
//! (this firmware only selects 40 and 80 MHz). It is `None` for an encoding
//! that function aborts on, and the bus then keeps the previous frequency
//! (it never panics and never sets a zero frequency). Byte-split writes
//! update one byte lane each; the derived frequency is meaningful only once
//! the whole word has landed.

use crate::mem::soc::XTAL_HZ;

pub const CPU_INTR_FROM_CPU_0_REG: u32 = 0x028;
/// Number of FROM_CPU registers (`_0`..`_3`).
pub const FROM_CPU_COUNT: usize = 4;
const FROM_CPU_END: u32 = CPU_INTR_FROM_CPU_0_REG + 4 * FROM_CPU_COUNT as u32;

/// `SYSTEM_CPU_PER_CONF_REG`'s offset (`system_reg.h`).
pub const CPU_PER_CONF_REG: u32 = 0x008;
/// `SYSTEM_SYSCLK_CONF_REG`'s offset (`system_reg.h`).
pub const SYSCLK_CONF_REG: u32 = 0x058;
const CPU_PER_CONF_RESET: u32 = 0x0c;
const CPU_PER_CONF_RW: u32 = 0xff;
const SYSCLK_CONF_RESET: u32 = 0x01;
const SYSCLK_CONF_RW: u32 = 0x0fff;

#[derive(Clone)]
pub struct System {
    /// Bit 0 of `SYSTEM_CPU_INTR_FROM_CPU_<n>_REG`.
    from_cpu: [bool; FROM_CPU_COUNT],
    /// `SYSTEM_CPU_PER_CONF_REG`, R/W bits only.
    cpu_per_conf: u32,
    /// `SYSTEM_SYSCLK_CONF_REG`, R/W bits only.
    sysclk_conf: u32,
}

impl Default for System {
    fn default() -> Self {
        Self {
            from_cpu: [false; FROM_CPU_COUNT],
            cpu_per_conf: CPU_PER_CONF_RESET,
            sysclk_conf: SYSCLK_CONF_RESET,
        }
    }
}

impl System {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset` is one of the six modeled registers.
    pub fn handles(offset: u32) -> bool {
        let word = offset & !0b11;
        (CPU_INTR_FROM_CPU_0_REG..FROM_CPU_END).contains(&word)
            || word == CPU_PER_CONF_REG
            || word == SYSCLK_CONF_REG
    }

    /// The stored clock register at word offset `word`, with its R/W mask.
    fn clock_reg(&mut self, word: u32) -> Option<(&mut u32, u32)> {
        match word {
            CPU_PER_CONF_REG => Some((&mut self.cpu_per_conf, CPU_PER_CONF_RW)),
            SYSCLK_CONF_REG => Some((&mut self.sysclk_conf, SYSCLK_CONF_RW)),
            _ => None,
        }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        if !Self::handles(offset) {
            return 0;
        }
        let lane = 8 * (offset & 0b11);
        if let Some((reg, _)) = self.clock_reg(offset & !0b11) {
            return (*reg >> lane) as u8;
        }
        if offset & 0b11 != 0 {
            return 0;
        }
        let n = ((offset - CPU_INTR_FROM_CPU_0_REG) / 4) as usize;
        u8::from(self.from_cpu[n])
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        if !Self::handles(offset) {
            return;
        }
        let lane = 8 * (offset & 0b11);
        if let Some((reg, rw)) = self.clock_reg(offset & !0b11) {
            let mask = rw & (0xff << lane);
            *reg = (*reg & !mask) | ((u32::from(val) << lane) & mask);
            return;
        }
        if offset & 0b11 != 0 {
            return;
        }
        let n = ((offset - CPU_INTR_FROM_CPU_0_REG) / 4) as usize;
        self.from_cpu[n] = val & 1 != 0;
    }

    /// The CPU frequency these registers select, in Hz, decoded as ESP-IDF
    /// v5.5.3 `rtc_clk_cpu_freq_get_config` does; `None` for an encoding it
    /// would abort on. See the module doc.
    pub fn cpu_hz(&self) -> Option<u64> {
        match (self.sysclk_conf >> 10) & 0b11 {
            0 => Some(XTAL_HZ / (u64::from(self.sysclk_conf & 0x3ff) + 1)),
            1 => match self.cpu_per_conf & 0b11 {
                0 => Some(80_000_000),
                1 => Some(160_000_000),
                _ => None,
            },
            2 => Some(20_000_000),
            _ => None,
        }
    }

    /// Bitmask (bit `n` = `FROM_CPU_INTR<n>`) of asserted software
    /// interrupts, for `FirmwareBus::pending_sources` to shift into
    /// `SRC_FROM_CPU_INTR0..`.
    pub fn pending_mask(&self) -> u32 {
        self.from_cpu
            .iter()
            .enumerate()
            .fold(0, |m, (n, on)| m | (u32::from(*on) << n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(s: &mut System, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            s.write_byte(off + i as u32, *b);
        }
    }

    #[test]
    fn whole_word_write_asserts_and_zero_deasserts() {
        let mut s = System::new();
        assert_eq!(s.pending_mask(), 0);
        write_word(&mut s, CPU_INTR_FROM_CPU_0_REG, 1);
        assert_eq!(s.pending_mask(), 0b0001);
        assert_eq!(s.read_byte(CPU_INTR_FROM_CPU_0_REG), 1, "reads back");
        write_word(&mut s, CPU_INTR_FROM_CPU_0_REG, 0);
        assert_eq!(s.pending_mask(), 0);
        assert_eq!(s.read_byte(CPU_INTR_FROM_CPU_0_REG), 0);
    }

    #[test]
    fn the_four_registers_are_independent() {
        let mut s = System::new();
        write_word(&mut s, CPU_INTR_FROM_CPU_0_REG + 8, 1); // FROM_CPU_2
        assert_eq!(s.pending_mask(), 0b0100);
        write_word(&mut s, CPU_INTR_FROM_CPU_0_REG + 12, 1); // FROM_CPU_3
        assert_eq!(s.pending_mask(), 0b1100);
    }

    #[test]
    fn upper_bytes_do_not_affect_the_level() {
        let mut s = System::new();
        write_word(&mut s, CPU_INTR_FROM_CPU_0_REG, 0xFFFF_FF00);
        assert_eq!(s.pending_mask(), 0, "bit 0 clear => de-asserted");
        assert!(!System::handles(0x024));
        assert!(!System::handles(0x038));
        assert!(System::handles(0x034));
    }

    fn write32(s: &mut System, offset: u32, v: u32) {
        for (i, b) in v.to_le_bytes().into_iter().enumerate() {
            s.write_byte(offset + i as u32, b);
        }
    }

    fn read32(s: &mut System, offset: u32) -> u32 {
        u32::from_le_bytes(core::array::from_fn(|i| s.read_byte(offset + i as u32)))
    }

    #[test]
    fn clock_registers_reset_to_the_header_defaults() {
        let mut s = System::new();
        assert_eq!(read32(&mut s, CPU_PER_CONF_REG), 0x0000_000c);
        assert_eq!(read32(&mut s, SYSCLK_CONF_REG), 0x0000_0001);
        assert_eq!(s.cpu_hz(), Some(20_000_000), "XTAL / 2 at reset");
    }

    #[test]
    fn xtal_source_divides_the_crystal() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 0); // XTAL, PRE_DIV_CNT 0
        assert_eq!(s.cpu_hz(), Some(40_000_000));
        write32(&mut s, SYSCLK_CONF_REG, 3); // XTAL / 4
        assert_eq!(s.cpu_hz(), Some(10_000_000));
    }

    #[test]
    fn pll_source_uses_cpuperiod_sel() {
        let mut s = System::new();
        write32(&mut s, CPU_PER_CONF_REG, 0x0c); // CPUPERIOD_SEL 0
        write32(&mut s, SYSCLK_CONF_REG, 1 << 10); // PLL
        assert_eq!(s.cpu_hz(), Some(80_000_000));
        write32(&mut s, CPU_PER_CONF_REG, 0x0d); // CPUPERIOD_SEL 1
        assert_eq!(s.cpu_hz(), Some(160_000_000));
    }

    #[test]
    fn rc_fast_source_is_20_mhz() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 2 << 10);
        assert_eq!(s.cpu_hz(), Some(20_000_000));
    }

    #[test]
    fn invalid_clock_encodings_keep_the_previous_frequency() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 3 << 10); // SOC_CLK_SEL 3: invalid
        assert_eq!(s.cpu_hz(), None);
        write32(&mut s, SYSCLK_CONF_REG, 1 << 10);
        write32(&mut s, CPU_PER_CONF_REG, 0x0f); // CPUPERIOD_SEL 3: invalid
        assert_eq!(s.cpu_hz(), None);
    }

    #[test]
    fn read_only_sysclk_fields_ignore_writes() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 0xffff_ffff);
        assert_eq!(read32(&mut s, SYSCLK_CONF_REG), 0x0000_0fff);
    }
}

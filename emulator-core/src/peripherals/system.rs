//! SYSTEM peripheral (`DR_REG_SYSTEM_BASE = 0x600c_0000`): only the four
//! `SYSTEM_CPU_INTR_FROM_CPU_<n>_REG` software-interrupt registers.
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
//! bits are not modeled and read 0. Every other SYSTEM register keeps the
//! bus's logged catch-all behavior (see `FirmwareBus`).
//!
//! Byte-split writes: `FirmwareBus::write32` arrives as four ascending byte
//! writes. Only byte 0 carries bit 0, so the level changes exactly on that
//! byte's write; the other three bytes are dropped.

pub const CPU_INTR_FROM_CPU_0_REG: u32 = 0x028;
/// Number of FROM_CPU registers (`_0`..`_3`).
pub const FROM_CPU_COUNT: usize = 4;
const FROM_CPU_END: u32 = CPU_INTR_FROM_CPU_0_REG + 4 * FROM_CPU_COUNT as u32;

#[derive(Default)]
pub struct System {
    /// Bit 0 of `SYSTEM_CPU_INTR_FROM_CPU_<n>_REG`.
    from_cpu: [bool; FROM_CPU_COUNT],
}

impl System {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset` is one of the four modeled registers.
    pub fn handles(offset: u32) -> bool {
        (CPU_INTR_FROM_CPU_0_REG..FROM_CPU_END).contains(&(offset & !0b11))
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        if !Self::handles(offset) || offset & 0b11 != 0 {
            return 0;
        }
        let n = ((offset - CPU_INTR_FROM_CPU_0_REG) / 4) as usize;
        u8::from(self.from_cpu[n])
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        if !Self::handles(offset) || offset & 0b11 != 0 {
            return;
        }
        let n = ((offset - CPU_INTR_FROM_CPU_0_REG) / 4) as usize;
        self.from_cpu[n] = val & 1 != 0;
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
}

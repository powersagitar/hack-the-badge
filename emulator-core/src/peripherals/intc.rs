//! ESP32-C3 interrupt matrix (`INTERRUPT_CORE0`,
//! `DR_REG_INTERRUPT_CORE0_BASE = 0x600c_2000`).
//!
//! **This is not a standard RISC-V PLIC.** Register layout confirmed
//! against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/interrupt_core0_reg.h` (fetched
//! directly via `gh api` this task). Per that header and
//! `components/riscv/vectors_intc.S`'s documented vector-table scheme: every
//! peripheral "interrupt source" (UART, SPI, GPIO, both timer groups, the
//! systimer, ~60 others per the header) has its own read/write **MAP
//! register** that routes it (by writing a value `0..=31`) onto one of 32
//! "CPU interrupt lines." `CPU_INT_ENABLE_REG` then gates, per line, whether
//! that line's (possibly OR-of-multiple-sources) signal is allowed through
//! to the CPU core at all. `mcause`'s low bits for a taken interrupt are
//! literally the CPU line number (0-31) -- see `crate::cpu::mod`'s
//! `enter_trap` (already correct for this, per Task 1) and
//! `crate::boot::step_with_interrupts` (this task's driving loop, which
//! calls `Cpu::raise_interrupt(line)` with exactly the value
//! [`InterruptController::poll`] returns).
//!
//! ## v1 scope (documented simplification)
//!
//! Only `SYSTIMER_TARGET0_INT_MAP_REG` (offset `0x094`) has a real signal
//! wired behind it -- [`InterruptController::poll`] is given
//! `systimer_target0_pending` (from `SysTimer::target0_pending`) by
//! `FirmwareBus`, which owns both peripherals as concrete fields per this
//! plan's pre-flight "no trait-object peripheral dispatch" ruling. Every
//! other source's MAP register (~60 of them, per the header) is modeled as
//! plain read/write storage with no source behind it yet, in
//! [`InterruptController::other_map_regs`] -- so firmware writes to them
//! aren't silently lost, they just don't do anything in v1.
//!
//! `CPU_INT_PRI_<n>_REG` (32 registers, one per line) and
//! `CPU_INT_THRESH_REG` are likewise real read/write storage but not
//! consulted by [`InterruptController::poll`]'s arbitration logic: their
//! hardware reset value is 0, and a threshold of 0 means "any enabled
//! interrupt gets through" on real hardware, so treating every enabled
//! line as always above threshold is priority/threshold-*permissive* by
//! construction, not silently wrong -- it just doesn't yet implement
//! priority arbitration between multiple simultaneously-pending lines
//! (moot for v1 anyway, since only one source -- systimer target0 -- is
//! wired to a real signal). A later task that wires up a second real
//! source should revisit this before it becomes observably wrong.
//! `CPU_INT_TYPE_REG` (edge vs. level per line) and `CPU_INT_CLEAR_REG` are
//! likewise real storage with no behavior wired to them yet (v1's one real
//! source, the systimer, is cleared via its own `INT_CLR_REG`, not this
//! one -- see `systimer::SysTimer`).

use std::collections::HashMap;

use super::set_byte;

pub const SYSTIMER_TARGET0_INT_MAP_REG: u32 = 0x094;
pub const CPU_INT_ENABLE_REG: u32 = 0x104;
pub const CPU_INT_TYPE_REG: u32 = 0x108;
pub const CPU_INT_CLEAR_REG: u32 = 0x10C;
/// Read-only: bit per line, "is this line currently asserted." Computed on
/// read (needs the systimer's live pending state), not stored -- see
/// `crate::mem::bus::FirmwareBus`'s dispatch, which special-cases this one
/// offset to call [`InterruptController::eip_status`] directly rather than
/// going through [`InterruptController::read_byte`].
pub const CPU_INT_EIP_STATUS_REG: u32 = 0x110;
pub const CPU_INT_PRI_BASE_REG: u32 = 0x114; // + 4*n, n in 0..32
pub const CPU_INT_THRESH_REG: u32 = 0x194;

/// End (exclusive) of the MAP-register region (`SYSTIMER_TARGET0`'s and
/// every other source's), per the header's lowest/highest MAP register
/// offsets (`0x000`..`0x0F4`). Anything below this that isn't
/// `SYSTIMER_TARGET0_INT_MAP_REG` is generic storage in
/// [`InterruptController::other_map_regs`].
const MAP_REGION_END: u32 = 0x100;

const LINE_MASK: u32 = 0x1F; // 5 bits: CPU interrupt line 0..=31

/// The ESP32-C3 interrupt matrix. See the module doc for what's real vs.
/// storage-only in v1.
#[derive(Default)]
pub struct InterruptController {
    systimer_target0_map: u32,
    /// Every other source's MAP register (`offset -> value`), real storage,
    /// no signal behind any of them in v1. See the module doc.
    other_map_regs: HashMap<u32, u32>,
    cpu_int_enable: u32,
    cpu_int_type: u32,
    cpu_int_clear: u32,
    cpu_int_pri: [u32; 32],
    cpu_int_thresh: u32,
}

impl InterruptController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up which CPU line `SYSTIMER_TARGET0` is currently routed to
    /// (`SYSTIMER_TARGET0_INT_MAP_REG & 0x1F`), and returns `Some(line)`
    /// only if that source is both pending (`systimer_target0_pending`) and
    /// that line is enabled in `CPU_INT_ENABLE_REG`. This is exactly what
    /// the driving loop (`crate::boot::step_with_interrupts`) uses to decide
    /// whether to call `Cpu::raise_interrupt`.
    ///
    /// Priority/threshold are intentionally not consulted here -- see the
    /// module doc's "v1 scope" section.
    pub fn poll(&self, systimer_target0_pending: bool) -> Option<u32> {
        if !systimer_target0_pending {
            return None;
        }
        let line = self.systimer_target0_map & LINE_MASK;
        if self.cpu_int_enable & (1 << line) != 0 {
            Some(line)
        } else {
            None
        }
    }

    /// `CPU_INT_EIP_STATUS_REG`'s value: bit `line` set iff [`Self::poll`]
    /// would currently select `line`. See that offset's doc comment on
    /// [`CPU_INT_EIP_STATUS_REG`] for why `FirmwareBus` calls this directly
    /// instead of going through [`Self::read_byte`].
    pub fn eip_status(&self, systimer_target0_pending: bool) -> u32 {
        match self.poll(systimer_target0_pending) {
            Some(line) => 1u32 << line,
            None => 0,
        }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            SYSTIMER_TARGET0_INT_MAP_REG => self.systimer_target0_map,
            CPU_INT_ENABLE_REG => self.cpu_int_enable,
            CPU_INT_TYPE_REG => self.cpu_int_type,
            CPU_INT_CLEAR_REG => self.cpu_int_clear,
            CPU_INT_THRESH_REG => self.cpu_int_thresh,
            o if (CPU_INT_PRI_BASE_REG..CPU_INT_THRESH_REG).contains(&o) => {
                self.cpu_int_pri[((o - CPU_INT_PRI_BASE_REG) / 4) as usize]
            }
            o if o < MAP_REGION_END => *self.other_map_regs.get(&o).unwrap_or(&0),
            _ => 0,
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            SYSTIMER_TARGET0_INT_MAP_REG => set_byte(&mut self.systimer_target0_map, idx, val),
            CPU_INT_ENABLE_REG => set_byte(&mut self.cpu_int_enable, idx, val),
            CPU_INT_TYPE_REG => set_byte(&mut self.cpu_int_type, idx, val),
            CPU_INT_CLEAR_REG => set_byte(&mut self.cpu_int_clear, idx, val),
            CPU_INT_THRESH_REG => set_byte(&mut self.cpu_int_thresh, idx, val),
            o if (CPU_INT_PRI_BASE_REG..CPU_INT_THRESH_REG).contains(&o) => {
                let n = ((o - CPU_INT_PRI_BASE_REG) / 4) as usize;
                set_byte(&mut self.cpu_int_pri[n], idx, val);
            }
            o if o < MAP_REGION_END => {
                let mut word = *self.other_map_regs.get(&o).unwrap_or(&0);
                set_byte(&mut word, idx, val);
                self.other_map_regs.insert(o, word);
            }
            // CPU_INT_EIP_STATUS_REG is read-only; FirmwareBus already
            // intercepts reads of it before calling into this peripheral,
            // but a write reaching here (if it ever did) is correctly a
            // no-op. Anything past CPU_INT_THRESH_REG (e.g. the DATE
            // register) is likewise accepted and dropped.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(ic: &mut InterruptController, word_offset: u32, val: u32) {
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            ic.write_byte(word_offset + i as u32, *b);
        }
    }
    fn read_word(ic: &mut InterruptController, word_offset: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = ic.read_byte(word_offset + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    #[test]
    fn map_register_routes_systimer_target0_to_the_written_line() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, SYSTIMER_TARGET0_INT_MAP_REG, 7);
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 7);
        assert_eq!(ic.poll(true), Some(7));
    }

    #[test]
    fn poll_returns_none_when_not_pending() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, SYSTIMER_TARGET0_INT_MAP_REG, 7);
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 7);
        assert_eq!(ic.poll(false), None);
    }

    #[test]
    fn cpu_int_enable_gating_suppresses_a_pending_but_disabled_line() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, SYSTIMER_TARGET0_INT_MAP_REG, 3);
        // CPU_INT_ENABLE_REG left at its reset value (0) -- line 3 disabled.
        assert_eq!(ic.poll(true), None, "pending+disabled must not fire");
    }

    #[test]
    fn eip_status_reflects_the_asserted_line_only() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, SYSTIMER_TARGET0_INT_MAP_REG, 12);
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 12);
        assert_eq!(ic.eip_status(true), 1 << 12);
        assert_eq!(ic.eip_status(false), 0);
    }

    #[test]
    fn map_register_masks_to_5_bits_for_the_line_number() {
        let mut ic = InterruptController::new();
        // Only the low 5 bits are architecturally meaningful.
        write_word(&mut ic, SYSTIMER_TARGET0_INT_MAP_REG, 0xFFFF_FFE1); // low5 = 1
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 1);
        assert_eq!(ic.poll(true), Some(1));
    }

    #[test]
    fn other_map_registers_and_pri_thresh_are_real_readback_storage() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 0x054, 9); // e.g. UART_INTR_MAP_REG offset, arbitrary
        assert_eq!(read_word(&mut ic, 0x054), 9);

        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 4 * 5, 0xA); // line 5's priority
        assert_eq!(read_word(&mut ic, CPU_INT_PRI_BASE_REG + 4 * 5), 0xA);
        // Untouched priority registers stay at reset value 0.
        assert_eq!(read_word(&mut ic, CPU_INT_PRI_BASE_REG), 0);

        write_word(&mut ic, CPU_INT_THRESH_REG, 3);
        assert_eq!(read_word(&mut ic, CPU_INT_THRESH_REG), 3);
    }

    #[test]
    fn cpu_int_type_and_clear_round_trip_as_plain_storage() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, CPU_INT_TYPE_REG, 0xDEAD_BEEF);
        assert_eq!(read_word(&mut ic, CPU_INT_TYPE_REG), 0xDEAD_BEEF);
        write_word(&mut ic, CPU_INT_CLEAR_REG, 0x1234);
        assert_eq!(read_word(&mut ic, CPU_INT_CLEAR_REG), 0x1234);
    }
}

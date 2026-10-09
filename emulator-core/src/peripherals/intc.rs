//! ESP32-C3 interrupt matrix (`INTERRUPT_CORE0`,
//! `DR_REG_INTERRUPT_CORE0_BASE = 0x600c_2000`).
//!
//! **This is not a standard RISC-V PLIC.** Register layout confirmed
//! against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/interrupt_core0_reg.h` (fetched at
//! tag `v5.5.3`, not guessed). Per that header and
//! `components/riscv/vectors_intc.S`'s documented vector-table scheme: every
//! peripheral "interrupt source" (UART, SPI, GPIO, both timer groups, the
//! systimer, ~60 others per the header) has its own read/write **MAP
//! register** that routes it (by writing a value `0..=31`) onto one of 32
//! "CPU interrupt lines." `CPU_INT_ENABLE_REG` then gates, per line, whether
//! that line's (possibly OR-of-multiple-sources) signal is allowed through
//! to the CPU core at all. `mcause`'s low bits for a taken interrupt are
//! literally the CPU line number (0-31) -- see `crate::cpu::mod`'s
//! `enter_trap` and `crate::boot::step_with_interrupts` (the driving loop,
//! which hands the mask [`InterruptController::poll`] returns to
//! `Cpu::set_pending_interrupts`).
//!
//! ## Source-indexed MAP registers
//!
//! The MAP region (`0x000..0x100`) is a uniform `[u32; 64]`
//! ([`InterruptController::map`]) indexed by `offset / 4`. A source's
//! number is its MAP register's `offset / 4` -- the order of
//! `periph_interrupt_t` in `components/soc/esp32c3/include/soc/interrupts.h`
//! (v5.5.3), e.g. `ETS_SPI2_INTR_SOURCE` = 19 (`SPI_INTR_2_MAP_REG`,
//! `0x04C`), `ETS_SYSTIMER_TARGET0_INTR_SOURCE` = 37
//! (`SYSTIMER_TARGET0_INT_MAP_REG`, `0x094`), `ETS_DMA_CH0_INTR_SOURCE` = 44
//! (`0x0B0`), `ETS_FROM_CPU_INTR0_SOURCE` = 50 (`CPU_INTR_FROM_CPU_0_MAP_REG`,
//! `0x0C8`). The constants live in `crate::mem::soc` (`SRC_*`). A source
//! routes to line `MAP[src] & 0x1F`; **line 0 means "not routed"** (the MAP
//! reset value is 0, and ESP-IDF never uses CPU line 0:
//! `components/riscv/include/esp_private/interrupt_intc.h`'s
//! `assert_valid_rv_int_num` asserts `rv_int_num != 0`).
//!
//! ## Priority / threshold rule
//!
//! [`InterruptController::poll`] returns the mask of CPU lines that are
//! (a) routed-to by at least one *asserted* source, (b) enabled in
//! `CPU_INT_ENABLE_REG`, and (c) whose `CPU_INT_PRI_<n>_REG` priority is
//! non-zero (priority 0 is "disabled", below) and **greater than or equal
//! to** `CPU_INT_THRESH_REG` -- i.e. only priorities *strictly less than*
//! the threshold are masked. Sources, all
//! ESP-IDF v5.5.3:
//! - `components/riscv/include/esp_private/interrupt_intc.h`: "On the
//!   legacy INTC, all interrupt priority levels strictly less than the
//!   threshold level are masked" (next to `RVHAL_INTR_ENABLE_THRESH = 1`,
//!   the threshold `xPortStartScheduler` sets via `esprv_int_set_threshold`
//!   in `components/freertos/FreeRTOS-Kernel/portable/riscv/port.c`);
//!   `components/riscv/include/riscv/interrupt.h`'s `esprv_int_set_threshold`
//!   doc says the same ("lower than the threshold are masked").
//! - `components/riscv/vectors.S`'s ISR entry sets
//!   `THRESH = PRI[mcause] + 1` to mask same-level nesting, which only
//!   makes sense under `>=`.
//! - Cross-check: Espressif's own QEMU model (`espressif/qemu`,
//!   `hw/riscv/esp32c3_intmatrix.c`, `esp32c3_intmatrix_line_should_assert`)
//!   uses `irq_prio[line] >= irq_thres`.
//!
//! (Task 4's brief proposed `>` ("strictly greater"); that would mask every
//! priority-1 line under FreeRTOS's threshold of 1 -- including the
//! `FROM_CPU_0` yield interrupt on the real firmware's boot -- and
//! contradicts the sources above, so `>=` is implemented.)
//!
//! Both fields are 4 bits wide (`interrupt_core0_reg.h`: `CPU_INT_THRESH`
//! bitpos `[3:0]`, `CPU_PRI_<n>_MAP` bitpos `[3:0]`), so the comparison
//! masks to 4 bits. Reset values are 0 for every priority *and* for the
//! threshold (`default: 4'b0`).
//!
//! **Priority 0 means "disabled"** (Milestone 3 Task 6): a line whose
//! 4-bit priority is 0 is never asserted to the core, whatever the
//! threshold -- not even under the reset threshold of 0, where `0 >= 0`
//! alone would admit it. Sources: ESP32-C3 TRM v1.4 section 1.5.2 (interrupt
//! controller: priority levels 1..15 are usable, and a line at priority 0
//! is disabled); ESP-IDF v5.5.3 `components/riscv/include/riscv/interrupt.h`
//! documents `esprv_int_set_priority`'s priority as "Interrupt priority
//! level, 1 to 7" (0 is never a valid allocated level), and every ESP-IDF
//! allocation programs the line's priority (`esp_cpu_intr_set_priority` ->
//! `esprv_intc_int_set_priority`, `components/esp_hw_support/cpu.c`) before
//! enabling it. This **diverges from Espressif's QEMU model**, whose
//! `esp32c3_intmatrix_line_should_assert` only rejects line 0 and admits a
//! priority-0 line under threshold 0; we follow the TRM. It matters for
//! `WFI`: a routed, enabled source on a never-prioritized line must not wake
//! the core. The enable register (reset 0) and the 0 reset priorities both
//! keep a fresh controller quiet. ESP-IDF programs each allocated
//! line's priority (`esp_cpu_intr_set_priority` ->
//! `esprv_intc_int_set_priority`, `components/esp_hw_support/cpu.c`)
//! before enabling it. The FreeRTOS port's critical sections raise
//! `CPU_INT_THRESH_REG` (`rv_utils_set_intlevel_regval`, `interrupt_intc.h`)
//! rather than only clearing `mstatus.MIE`; honoring the threshold here is
//! what makes that masking work.
//!
//! [`InterruptController::eip_status`] is the ungated view: lines with any
//! routed pending source, before enable/priority/threshold.
//!
//! ## Level delivery
//!
//! Sources are supplied as a `u64` bitmask of *currently asserted* levels
//! ([`InterruptController::poll`]'s `pending_sources`), recomputed by
//! `FirmwareBus::pending_sources` every step. Nothing is latched here:
//! once a peripheral's raw status is cleared (e.g. SYSTIMER `INT_CLR`, or a
//! `0` written to `SYSTEM_CPU_INTR_FROM_CPU_n_REG`) the line de-asserts on
//! the next step. `CPU_INT_TYPE_REG` (edge vs. level) and
//! `CPU_INT_CLEAR_REG` are still plain read/write storage: every source
//! wired so far is level-type, and ESP-IDF's edge handling is not needed
//! until a stall implicates it.
//!
//! ## Who else writes these registers
//!
//! Real firmware doesn't only reach these registers by executing its own
//! instructions: `crate::rom`'s ESP32-C3 mask-ROM HLE stub table gives six
//! ROM calls (`intr_matrix_set`, `esprv_intc_int_disable`,
//! `esprv_intc_int_enable`, `esprv_intc_int_set_type`,
//! `esprv_intc_int_set_priority`, `esprv_intc_int_set_threshold`) a real
//! `RomStubEffect::BusRegisterWrite` effect
//! (`crate::cpu::rom_stubs::RomStubEffect`) that reads/writes these
//! registers through the same `Bus` path an executed instruction would use
//! -- this module has no way to tell the two apart, nor does it need to.
//! The indexed ones (`intr_matrix_set`, `_set_priority`) are bounded by
//! [`MAP_SOURCE_COUNT`] / [`LINE_COUNT`] so a wild index can never write past
//! the arrays into a neighbouring register.

use super::set_byte;

pub const SYSTIMER_TARGET0_INT_MAP_REG: u32 = 0x094;
pub const CPU_INT_ENABLE_REG: u32 = 0x104;
pub const CPU_INT_TYPE_REG: u32 = 0x108;
pub const CPU_INT_CLEAR_REG: u32 = 0x10C;
/// Read-only: bit per line, "is this line currently asserted." Computed on
/// read (needs the live pending sources), not stored -- see
/// `crate::mem::bus::FirmwareBus`'s dispatch, which special-cases this one
/// offset to call [`InterruptController::eip_status`] directly rather than
/// going through [`InterruptController::read_byte`].
pub const CPU_INT_EIP_STATUS_REG: u32 = 0x110;
pub const CPU_INT_PRI_BASE_REG: u32 = 0x114; // + 4*n, n in 0..32
pub const CPU_INT_THRESH_REG: u32 = 0x194;

/// End (exclusive) of the MAP-register region, per the header's
/// lowest/highest MAP register offsets (`0x000`..`0x0F4`), rounded up to
/// the `[u32; 64]` storage.
const MAP_REGION_END: u32 = 0x100;

/// Number of MAP registers modeled (`MAP_REGION_END / 4`); the exclusive
/// upper bound for a source index.
pub const MAP_SOURCE_COUNT: u32 = MAP_REGION_END / 4;
/// Number of CPU interrupt lines / `CPU_INT_PRI_<n>_REG` registers.
pub const LINE_COUNT: u32 = 32;

const LINE_MASK: u32 = 0x1F; // 5 bits: CPU interrupt line 0..=31
const PRIO_MASK: u32 = 0xF; // 4-bit priority / threshold fields

/// The ESP32-C3 interrupt matrix. See the module doc.
#[derive(Clone)]
pub struct InterruptController {
    /// Every source's MAP register, indexed by `offset / 4`.
    map: [u32; MAP_SOURCE_COUNT as usize],
    cpu_int_enable: u32,
    cpu_int_type: u32,
    cpu_int_clear: u32,
    cpu_int_pri: [u32; LINE_COUNT as usize],
    cpu_int_thresh: u32,
}

impl Default for InterruptController {
    fn default() -> Self {
        Self {
            map: [0; MAP_SOURCE_COUNT as usize],
            cpu_int_enable: 0,
            cpu_int_type: 0,
            cpu_int_clear: 0,
            cpu_int_pri: [0; LINE_COUNT as usize],
            cpu_int_thresh: 0,
        }
    }
}

impl InterruptController {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset`'s word-aligned offset is a register this module
    /// gives real behavior to: any of the named single registers, a
    /// `CPU_INT_PRI_<n>_REG`, `CPU_INT_EIP_STATUS_REG` (computed by
    /// `crate::mem::bus::FirmwareBus` directly), or anywhere in the MAP
    /// region. Pure function of the word offset, used by `FirmwareBus` to
    /// additionally log an access past `CPU_INT_THRESH_REG` (e.g. the DATE
    /// register) as "unmapped."
    pub fn handles(offset: u32) -> bool {
        let o = offset & !0b11;
        o < MAP_REGION_END
            || matches!(
                o,
                CPU_INT_ENABLE_REG
                    | CPU_INT_TYPE_REG
                    | CPU_INT_CLEAR_REG
                    | CPU_INT_EIP_STATUS_REG
                    | CPU_INT_THRESH_REG
            )
            || (CPU_INT_PRI_BASE_REG..CPU_INT_THRESH_REG).contains(&o)
    }

    /// Mask of CPU lines with at least one asserted, routed source
    /// (`pending_sources` bit `s` set and `MAP[s] & 0x1F != 0`), before any
    /// enable/priority gating.
    fn routed_lines(&self, pending_sources: u64) -> u32 {
        let mut lines = 0u32;
        for (src, map) in self.map.iter().enumerate() {
            if pending_sources & (1u64 << src) != 0 {
                let line = map & LINE_MASK;
                if line != 0 {
                    lines |= 1 << line;
                }
            }
        }
        lines
    }

    /// The set of CPU lines currently asserted *to the CPU*: routed pending,
    /// enabled in `CPU_INT_ENABLE_REG`, with a non-zero priority (priority 0
    /// is "disabled"), and with priority greater than or equal to
    /// `CPU_INT_THRESH_REG` (only priorities strictly below the threshold are
    /// masked). See the module doc.
    pub fn poll(&self, pending_sources: u64) -> u32 {
        if pending_sources == 0 {
            return 0; // the common case, every step: skip the scans
        }
        let candidates = self.routed_lines(pending_sources) & self.cpu_int_enable;
        let thresh = self.cpu_int_thresh & PRIO_MASK;
        let mut out = 0u32;
        for line in 0..LINE_COUNT {
            let pri = self.cpu_int_pri[line as usize] & PRIO_MASK;
            // Priority 0 = line disabled (module doc), whatever `thresh` is.
            if candidates & (1 << line) != 0 && pri != 0 && pri >= thresh {
                out |= 1 << line;
            }
        }
        out
    }

    /// Each CPU line's 4-bit `CPU_INT_PRI_n` priority, for the core's
    /// arbitration among simultaneously asserted lines
    /// (`crate::cpu::select_interrupt_line`: highest priority first, ties to
    /// the lowest line -- ESP32-C3 TRM v1.4 section 1.5.2).
    pub fn line_priorities(&self) -> [u8; 32] {
        self.cpu_int_pri.map(|p| (p & PRIO_MASK) as u8)
    }

    /// `CPU_INT_EIP_STATUS_REG`'s value: bit `line` set iff some routed
    /// source is currently asserted, *before* enable/priority gating.
    pub fn eip_status(&self, pending_sources: u64) -> u32 {
        self.routed_lines(pending_sources)
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            CPU_INT_ENABLE_REG => self.cpu_int_enable,
            CPU_INT_TYPE_REG => self.cpu_int_type,
            CPU_INT_CLEAR_REG => self.cpu_int_clear,
            CPU_INT_THRESH_REG => self.cpu_int_thresh,
            o if (CPU_INT_PRI_BASE_REG..CPU_INT_THRESH_REG).contains(&o) => {
                self.cpu_int_pri[((o - CPU_INT_PRI_BASE_REG) / 4) as usize]
            }
            o if o < MAP_REGION_END => self.map[(o / 4) as usize],
            _ => 0,
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            CPU_INT_ENABLE_REG => set_byte(&mut self.cpu_int_enable, idx, val),
            CPU_INT_TYPE_REG => set_byte(&mut self.cpu_int_type, idx, val),
            CPU_INT_CLEAR_REG => set_byte(&mut self.cpu_int_clear, idx, val),
            CPU_INT_THRESH_REG => set_byte(&mut self.cpu_int_thresh, idx, val),
            o if (CPU_INT_PRI_BASE_REG..CPU_INT_THRESH_REG).contains(&o) => {
                let n = ((o - CPU_INT_PRI_BASE_REG) / 4) as usize;
                set_byte(&mut self.cpu_int_pri[n], idx, val);
            }
            o if o < MAP_REGION_END => set_byte(&mut self.map[(o / 4) as usize], idx, val),
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

    /// Routes `src` to `line`, enables it, and gives it priority `pri`
    /// (what ESP-IDF's `esp_intr_alloc` path does), leaving THRESH alone.
    fn arm(ic: &mut InterruptController, src: u32, line: u32, pri: u32) {
        write_word(ic, src * 4, line);
        let en = read_word(ic, CPU_INT_ENABLE_REG);
        write_word(ic, CPU_INT_ENABLE_REG, en | (1 << line));
        write_word(ic, CPU_INT_PRI_BASE_REG + line * 4, pri);
    }

    #[test]
    fn map_register_routes_systimer_target0_to_the_written_line() {
        let mut ic = InterruptController::new();
        arm(&mut ic, 37, 7, 1);
        assert_eq!(ic.poll(1u64 << 37), 1 << 7);
        assert_eq!(SYSTIMER_TARGET0_INT_MAP_REG, 37 * 4);
    }

    #[test]
    fn cpu_int_enable_gating_suppresses_a_pending_but_disabled_line() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 37 * 4, 3);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 3 * 4, 5);
        // CPU_INT_ENABLE_REG left at its reset value (0) -- line 3 disabled.
        assert_eq!(ic.poll(1u64 << 37), 0, "pending+disabled must not fire");
    }

    #[test]
    fn eip_status_is_the_ungated_routed_view() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 37 * 4, 12);
        // Not enabled, priority 0: poll masks it, eip_status still shows it.
        assert_eq!(ic.eip_status(1u64 << 37), 1 << 12);
        assert_eq!(ic.poll(1u64 << 37), 0);
        assert_eq!(ic.eip_status(0), 0);
    }

    #[test]
    fn map_register_masks_to_5_bits_for_the_line_number() {
        let mut ic = InterruptController::new();
        // Only the low 5 bits are architecturally meaningful.
        write_word(&mut ic, 37 * 4, 0xFFFF_FFE1); // low5 = 1
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 1);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 4, 1);
        assert_eq!(ic.poll(1u64 << 37), 1 << 1);
    }

    #[test]
    fn map_registers_pri_and_thresh_are_real_readback_storage() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 0x054, 9); // e.g. UART_INTR_MAP_REG offset, arbitrary
        assert_eq!(read_word(&mut ic, 0x054), 9);
        write_word(&mut ic, 0x0FC, 4); // last slot of the [u32; 64] region
        assert_eq!(read_word(&mut ic, 0x0FC), 4);

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

    #[test]
    fn routed_enabled_source_above_threshold_asserts_its_line() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 19 * 4, 5); // SPI2 -> line 5
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 5);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 5 * 4, 3);
        write_word(&mut ic, CPU_INT_THRESH_REG, 1);
        assert_eq!(ic.poll(1u64 << 19), 1 << 5);
        assert_eq!(ic.poll(0), 0);
    }

    /// Legacy INTC rule (module doc): only priorities *strictly less than*
    /// the threshold are masked.
    #[test]
    fn priority_below_threshold_is_masked_at_or_above_fires() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 37 * 4, 7);
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 7);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 2);
        write_word(&mut ic, CPU_INT_THRESH_REG, 3);
        assert_eq!(ic.poll(1u64 << 37), 0, "pri < thresh must not fire");
        assert_eq!(
            ic.eip_status(1u64 << 37),
            1 << 7,
            "still visible as pending"
        );
        write_word(&mut ic, CPU_INT_THRESH_REG, 2);
        assert_eq!(ic.poll(1u64 << 37), 1 << 7, "pri == thresh fires");
        write_word(&mut ic, CPU_INT_THRESH_REG, 1);
        assert_eq!(ic.poll(1u64 << 37), 1 << 7, "pri > thresh fires");
    }

    /// The real boot's case: FreeRTOS runs at `RVHAL_INTR_ENABLE_THRESH` = 1
    /// with every level-1 line at priority 1; a same-level ISR raises the
    /// threshold to `pri + 1` (`vectors.S`), which masks it.
    #[test]
    fn freertos_threshold_one_admits_priority_one_and_isr_bump_masks_it() {
        let mut ic = InterruptController::new();
        arm(&mut ic, 50, 4, 1); // FROM_CPU_INTR0 -> line 4, priority 1
        write_word(&mut ic, CPU_INT_THRESH_REG, 1);
        assert_eq!(ic.poll(1u64 << 50), 1 << 4);
        write_word(&mut ic, CPU_INT_THRESH_REG, 2);
        assert_eq!(ic.poll(1u64 << 50), 0);
    }

    /// Priority 0 means "disabled" (module doc): a routed, enabled line at
    /// the reset priority never asserts, whatever the threshold, until its
    /// priority is programmed.
    #[test]
    fn priority_zero_line_never_asserts_at_any_threshold() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 37 * 4, 7);
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 7);
        for thresh in [0, 1, 7, 15] {
            write_word(&mut ic, CPU_INT_THRESH_REG, thresh);
            assert_eq!(ic.poll(1u64 << 37), 0, "pri 0 fired at thresh {thresh}");
        }
        assert_eq!(
            ic.eip_status(1u64 << 37),
            1 << 7,
            "still visible as pending"
        );
        write_word(&mut ic, CPU_INT_THRESH_REG, 0);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 1);
        assert_eq!(ic.poll(1u64 << 37), 1 << 7, "pri 1 >= thresh 0 fires");
        // Only the low 4 bits count: 0x10 is priority 0 again.
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 0x10);
        assert_eq!(ic.poll(1u64 << 37), 0);
    }

    #[test]
    fn enable_is_still_a_gate_for_a_prioritized_line() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 37 * 4, 7);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 1);
        assert_eq!(ic.poll(1u64 << 37), 0, "not enabled");
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1 << 7);
        assert_eq!(ic.poll(1u64 << 37), 1 << 7);
    }

    #[test]
    fn two_sources_on_two_lines_both_assert() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, 37 * 4, 7);
        write_word(&mut ic, 44 * 4, 9);
        write_word(&mut ic, CPU_INT_ENABLE_REG, (1 << 7) | (1 << 9));
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 7 * 4, 1);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG + 9 * 4, 1);
        assert_eq!(ic.poll((1u64 << 37) | (1u64 << 44)), (1 << 7) | (1 << 9));
    }

    #[test]
    fn two_sources_sharing_a_line_or_together() {
        let mut ic = InterruptController::new();
        arm(&mut ic, 37, 7, 1);
        write_word(&mut ic, 44 * 4, 7);
        assert_eq!(ic.poll(1u64 << 44), 1 << 7);
        assert_eq!(ic.poll((1u64 << 44) | (1u64 << 37)), 1 << 7);
    }

    #[test]
    fn unrouted_source_line_zero_never_fires() {
        let mut ic = InterruptController::new();
        write_word(&mut ic, CPU_INT_ENABLE_REG, 1);
        write_word(&mut ic, CPU_INT_PRI_BASE_REG, 7);
        assert_eq!(ic.poll(1u64 << 19), 0);
        assert_eq!(ic.eip_status(1u64 << 19), 0);
    }

    #[test]
    fn deasserted_source_stops_asserting_level() {
        let mut ic = InterruptController::new();
        arm(&mut ic, 50, 4, 1);
        assert_eq!(ic.poll(1u64 << 50), 1 << 4);
        assert_eq!(ic.poll(0), 0, "no latching: level follows the source");
    }
}

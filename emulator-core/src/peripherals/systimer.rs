//! ESP32-C3 SYSTIMER peripheral (`DR_REG_SYSTIMER_BASE = 0x6002_3000`).
//!
//! Register layout confirmed against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/systimer_reg.h` (fetched directly
//! via `gh api` this task, not guessed). Only unit 0 + target 0 are modeled
//! with real behavior — that's the pair `components/freertos/port_systick.c`
//! uses for the FreeRTOS OS tick under the default
//! `CONFIG_FREERTOS_SYSTICK_USES_SYSTIMER` config (confirmed this task via
//! that exact file). Unit 1 and targets 1/2 are out of scope for v1 (nothing
//! in this emulator drives them yet); their registers, along with every
//! other systimer register this module doesn't name below, fall through to
//! a real-storage-free "read 0 / write ignored" default within this
//! peripheral's own dispatch (distinct from, but philosophically identical
//! to, `FirmwareBus`'s top-level catch-all for genuinely unmapped space).
//!
//! ## Modeled registers
//! - `SYSTIMER_CONF_REG` (0x00): `TIMER_UNIT0_WORK_EN` (bit 30, reset **1**
//!   per the header — the counter free-runs out of reset without any
//!   firmware write, matching real hardware) gates whether [`SysTimer::advance`]
//!   increments the counter at all. `TARGET0_WORK_EN` (bit 24, reset 0)
//!   gates whether target 0's comparator is checked.
//! - `SYSTIMER_UNIT0_OP_REG` (0x04): `TIMER_UNIT0_UPDATE` (bit 30, WT) is
//!   the "commit" trigger that latches the live counter into
//!   `UNIT0_VALUE_HI/LO_REG`, mirroring the `TIMG_T0UPDATE_REG` idiom the
//!   brief pointed at. This emulator has no propagation delay to model, so
//!   the latch completes synchronously in the same write; `TIMER_UNIT0_VALUE_VALID`
//!   (bit 29, real hardware: R/SS/WTC) is modeled as a simple bool that
//!   becomes (and stays) `true` after the first such trigger, so firmware
//!   that polls it before reading `VALUE_HI/LO` doesn't spin forever — a
//!   documented simplification of the real handshake.
//! - `SYSTIMER_TARGET0_HI/LO_REG` (0x1c/0x20): the live 52-bit comparator
//!   value, split as `HI[19:0] << 32 | LO[31:0]`. Directly read/write
//!   storage (real hardware: R/W too), for the oneshot
//!   "set an absolute future deadline" use case.
//! - `SYSTIMER_TARGET0_CONF_REG` (0x34): `PERIOD[25:0]`, `PERIOD_MODE` (bit
//!   30), `TIMER_UNIT_SEL` (bit 31; only `0` = unit 0 has a real
//!   comparison source in this model — see [`SysTimer::advance`]).
//! - `SYSTIMER_COMP0_LOAD_REG` (0x50): the trigger this module uses to
//!   resolve the "how does period mode ever get an initial target" question
//!   the brief flagged — see the **Period-mode arming** section below.
//! - `SYSTIMER_UNIT0_VALUE_HI/LO_REG` (0x40/0x44): read-only latched
//!   snapshot, written only by the `OP_REG` trigger above.
//! - `SYSTIMER_INT_RAW_REG`/`INT_ENA_REG`/`INT_CLR_REG`/`INT_ST_REG`
//!   (0x68/0x64/0x6c/0x70), bit 0 only (target 0's line):
//!   standard raw/enable/write-to-clear/status-is-raw-and-ena idiom.
//!
//! ## Period-mode arming: the judgment call the brief asked for
//!
//! Reading ESP-IDF v5.5.3's actual call sequence
//! (`port_systick.c::vSystimerSetup` -> `systimer_hal_set_alarm_period` ->
//! `systimer_ll_set_alarm_period`/`systimer_ll_apply_alarm_value`, all
//! fetched this task) shows FreeRTOS's tick setup **never writes
//! `TARGET0_HI/LO_REG` directly** — it only ever sets the `PERIOD` field and
//! then pulses `COMP0_LOAD_REG`. Since `TARGET0_HI/LO` reset to 0 and are
//! never otherwise touched on that path, the register header alone doesn't
//! say how the *first* comparator match ever happens (a target frozen at 0
//! could never produce a rising edge against a counter that starts at 0 and
//! only increases). The most consistent reading of the two call sites that
//! both end in `systimer_ll_apply_alarm_value`/`COMP0_LOAD` --
//! `systimer_hal_set_alarm_target` (oneshot: software computes an absolute
//! deadline, writes `TARGET0_HI/LO` itself, *then* pulses load) vs.
//! `systimer_hal_set_alarm_period` (period: software sets only `PERIOD`,
//! *then* pulses load, `TARGET0_HI/LO` untouched) -- is that the load pulse
//! is the single "arm" operation for both paths, and it must be computing
//! the actual target from whichever inputs software actually provided.
//!
//! This module therefore models `COMP0_LOAD_REG`'s trigger as: **if
//! `TARGET0_CONF_REG.PERIOD_MODE` is already set at the moment of the
//! trigger, set `target0 = (live unit0 counter) + PERIOD`** (a period-mode
//! arm relative to *now*); **otherwise, the trigger is a no-op** in this
//! model (the oneshot path already wrote `TARGET0_HI/LO` directly before
//! triggering, so nothing needs computing). This is a documented
//! simplification, not a literal transcription of undocumented internal
//! hardware state machine behavior -- but it is what makes v1's actual
//! target (a correctly periodic FreeRTOS tick) work end-to-end, which is
//! what the brief asked to prioritize getting right.
//!
//! Auto-rearm on match (also required reading, not assumed): neither
//! `port_systick.c`'s ISR nor any HAL call after setup ever rewrites
//! `PERIOD`/`TARGET0` again for the OS tick (`SysTickIsrHandler`'s own
//! comment: "works in periodic mode no need to calc the next alarm") --
//! confirming period mode's re-arm (`target += PERIOD` on every match) is
//! done by hardware itself, not software. [`SysTimer::advance`] models
//! exactly that.
//!
//! ## Tick-rate assumption (placeholder -- flagged per the brief)
//!
//! Real hardware: SYSTIMER runs off a fixed ~16 MHz reference while the CPU
//! runs up to 160 MHz, so a real tick is on the order of ~10 CPU clock
//! cycles (itself several `Cpu::step()`-equivalent instructions, more for
//! compressed/multi-cycle ones). Task 1's `Cpu::step()` has no notion of
//! clock cycles or wall-clock time at all -- it's exactly one instruction
//! per call, with no cycle-accurate timing model to derive a ratio from.
//! Rather than fabricate a cycle-accurate-looking ratio this project can't
//! actually back up, [`TICKS_PER_STEP`] is the simplest possible mapping:
//! **the counter advances by exactly 1 tick per `Cpu::step()` call.** This
//! is a placeholder, not a derived-from-truth value -- it only guarantees a
//! configured periodic target fires after a documented, deterministic,
//! testable number of `step()` calls, which is what this task's tests (and
//! any later task built on top of them) actually need.

/// See the module doc's "Tick-rate assumption" section: ticks the free-
/// running unit0 counter advances per [`SysTimer::advance`] call, i.e. per
/// `Cpu::step()` in the driving loop (`crate::boot::step_with_interrupts`).
pub const TICKS_PER_STEP: u64 = 1;

/// Unit0's counter is architecturally 52 bits (20 high + 32 low); mask any
/// arithmetic on it to that width so it wraps like real hardware would.
const COUNTER_MASK_52: u64 = (1u64 << 52) - 1;
/// `TARGET0_HI`/`UNIT0_VALUE_HI` are only 20 bits wide (`[19:0]`).
const HI_MASK_20: u32 = 0x000F_FFFF;

pub const CONF_REG: u32 = 0x00;
pub const UNIT0_OP_REG: u32 = 0x04;
pub const TARGET0_HI_REG: u32 = 0x1c;
pub const TARGET0_LO_REG: u32 = 0x20;
pub const TARGET0_CONF_REG: u32 = 0x34;
pub const UNIT0_VALUE_HI_REG: u32 = 0x40;
pub const UNIT0_VALUE_LO_REG: u32 = 0x44;
pub const COMP0_LOAD_REG: u32 = 0x50;
pub const INT_ENA_REG: u32 = 0x64;
pub const INT_RAW_REG: u32 = 0x68;
pub const INT_CLR_REG: u32 = 0x6c;
pub const INT_ST_REG: u32 = 0x70;

const TIMER_UNIT0_WORK_EN: u32 = 1 << 30;
const TARGET0_WORK_EN: u32 = 1 << 24;
const TARGET0_PERIOD_MASK: u32 = 0x03FF_FFFF; // bits [25:0]
const TARGET0_PERIOD_MODE: u32 = 1 << 30;
const TARGET0_TIMER_UNIT_SEL: u32 = 1 << 31; // 0 = unit0 (the only unit modeled)
const UNIT0_UPDATE_BIT: u8 = 0x40; // OP_REG bit 30, within byte index 3
const VALUE_VALID_BIT: u32 = 1 << 29; // OP_REG bit 29 (read side)
const COMP0_LOAD_BIT: u8 = 0x01; // COMP0_LOAD_REG bit 0, within byte index 0
const TARGET0_INT_BIT: u32 = 1; // bit 0 of RAW/ENA/CLR/ST, all within byte index 0

/// The SYSTIMER peripheral: a free-running 52-bit "unit 0" counter and its
/// "target 0" comparator/interrupt. See the module doc for exactly which
/// registers are modeled and why.
pub struct SysTimer {
    conf: u32,
    unit0_counter: u64,
    /// The counter value as of the *previous* [`SysTimer::advance`] call,
    /// used to edge-detect a target0 match (`prev < target && next >=
    /// target`) rather than treating it as a level condition that would
    /// keep re-asserting `INT_RAW` every step after a oneshot match whose
    /// target software never moves.
    unit0_prev_counter: u64,
    value_valid: bool,
    unit0_value_hi: u32,
    unit0_value_lo: u32,
    target0_hi: u32,
    target0_lo: u32,
    target0_conf: u32,
    int_ena: u32,
    int_raw: u32,
}

impl Default for SysTimer {
    fn default() -> Self {
        Self {
            // TIMER_UNIT0_WORK_EN reset default is 1 per the header -- the
            // counter free-runs without any firmware write, matching real
            // hardware.
            conf: TIMER_UNIT0_WORK_EN,
            unit0_counter: 0,
            unit0_prev_counter: 0,
            value_valid: false,
            unit0_value_hi: 0,
            unit0_value_lo: 0,
            target0_hi: 0,
            target0_lo: 0,
            target0_conf: 0,
            int_ena: 0,
            int_raw: 0,
        }
    }
}

impl SysTimer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current live 52-bit unit0 counter value. Exposed for tests; not
    /// itself a memory-mapped register (firmware reads the *latched*
    /// `UNIT0_VALUE_HI/LO_REG` snapshot instead, via `OP_REG`'s trigger).
    pub fn counter(&self) -> u64 {
        self.unit0_counter
    }

    fn target0_value(&self) -> u64 {
        (((self.target0_hi & HI_MASK_20) as u64) << 32) | self.target0_lo as u64
    }

    fn set_target0_value(&mut self, v: u64) {
        let v = v & COUNTER_MASK_52;
        self.target0_hi = ((v >> 32) as u32) & HI_MASK_20;
        self.target0_lo = (v & 0xFFFF_FFFF) as u32;
    }

    /// `raw & ena` for target 0 -- "is target0's interrupt currently
    /// asserted," exactly the method the brief asked this peripheral to
    /// expose for `InterruptController`/the driving loop to poll.
    pub fn target0_pending(&self) -> bool {
        (self.int_raw & self.int_ena & TARGET0_INT_BIT) != 0
    }

    /// Advances the free-running unit0 counter by [`TICKS_PER_STEP`] and
    /// checks target0's comparator for an edge-triggered match, latching
    /// `INT_RAW` and auto-rearming the target (`target += PERIOD`) in
    /// period mode. Call exactly once per `Cpu::step()` -- see
    /// `crate::boot::step_with_interrupts`.
    pub fn advance(&mut self) {
        if self.conf & TIMER_UNIT0_WORK_EN == 0 {
            return;
        }
        let prev = self.unit0_counter;
        let next = prev.wrapping_add(TICKS_PER_STEP) & COUNTER_MASK_52;
        self.unit0_counter = next;

        let comparator_active =
            self.conf & TARGET0_WORK_EN != 0 && self.target0_conf & TARGET0_TIMER_UNIT_SEL == 0; // only unit0 has a real source in v1
        if comparator_active {
            let target = self.target0_value();
            if prev < target && next >= target {
                self.int_raw |= TARGET0_INT_BIT;
                if self.target0_conf & TARGET0_PERIOD_MODE != 0 {
                    let period = (self.target0_conf & TARGET0_PERIOD_MASK) as u64;
                    if period > 0 {
                        let mut new_target = target;
                        while new_target <= next {
                            new_target = new_target.wrapping_add(period);
                        }
                        self.set_target0_value(new_target);
                    }
                    // period == 0 with PERIOD_MODE set: real hardware would
                    // match every tick; not expected from real firmware, so
                    // this model leaves the target unmoved (degenerate case,
                    // documented rather than special-cased further).
                }
                // Oneshot (PERIOD_MODE clear): target is left unchanged.
                // `unit0_prev_counter` still advances below, so the edge
                // condition (`prev < target`) can never be true again for
                // this same target value -- it fires exactly once, matching
                // real R/WTC/SS "set once, stays set until INT_CLR" behavior.
            }
        }
        self.unit0_prev_counter = next;
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            CONF_REG => self.conf,
            UNIT0_OP_REG => {
                if self.value_valid {
                    VALUE_VALID_BIT
                } else {
                    0
                }
            }
            TARGET0_HI_REG => self.target0_hi,
            TARGET0_LO_REG => self.target0_lo,
            TARGET0_CONF_REG => self.target0_conf,
            UNIT0_VALUE_HI_REG => self.unit0_value_hi,
            UNIT0_VALUE_LO_REG => self.unit0_value_lo,
            INT_ENA_REG => self.int_ena,
            INT_RAW_REG => self.int_raw,
            // TARGET0_INT_BIT is bit 0, so this is just the bool as a u32.
            INT_ST_REG => u32::from(self.target0_pending()),
            // COMP0_LOAD_REG and everything else this module doesn't name
            // (unit1/target1/target2/DATE/etc.): no real storage in v1,
            // reads as 0 -- see the module doc.
            _ => 0,
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            CONF_REG => super::set_byte(&mut self.conf, idx, val),
            UNIT0_OP_REG => {
                // TIMER_UNIT0_UPDATE (bit 30) lives entirely within byte
                // index 3; inspecting only that byte (not a word
                // reconstructed from other bytes of this same access) means
                // a `sw`'s 4 constituent byte-writes can't spuriously
                // double-trigger or trigger on a partial value -- see
                // `peripherals::set_byte`'s doc comment.
                if idx == 3 && val & UNIT0_UPDATE_BIT != 0 {
                    self.unit0_value_hi = ((self.unit0_counter >> 32) as u32) & HI_MASK_20;
                    self.unit0_value_lo = (self.unit0_counter & 0xFFFF_FFFF) as u32;
                    self.value_valid = true;
                }
            }
            TARGET0_HI_REG => {
                super::set_byte(&mut self.target0_hi, idx, val);
                self.target0_hi &= HI_MASK_20;
            }
            TARGET0_LO_REG => super::set_byte(&mut self.target0_lo, idx, val),
            TARGET0_CONF_REG => super::set_byte(&mut self.target0_conf, idx, val),
            COMP0_LOAD_REG => {
                // See the module doc's "Period-mode arming" section: this
                // is the trigger that gives period mode its first target,
                // computed relative to the live counter at the instant of
                // the trigger. Bit 0, byte index 0.
                if idx == 0
                    && val & COMP0_LOAD_BIT != 0
                    && self.target0_conf & TARGET0_PERIOD_MODE != 0
                {
                    let period = (self.target0_conf & TARGET0_PERIOD_MASK) as u64;
                    self.set_target0_value(self.unit0_counter.wrapping_add(period));
                }
            }
            INT_ENA_REG => super::set_byte(&mut self.int_ena, idx, val),
            // TARGET0_INT_CLR: WT, bit 0, byte index 0.
            INT_CLR_REG if idx == 0 && val & 0x01 != 0 => {
                self.int_raw &= !TARGET0_INT_BIT;
            }
            INT_CLR_REG => {}
            // UNIT0_VALUE_HI/LO are RO; everything else this module doesn't
            // name is accepted and dropped, matching FirmwareBus's own
            // catch-all philosophy so firmware probing unrelated systimer
            // registers doesn't panic.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_advances_by_ticks_per_step_each_call() {
        let mut t = SysTimer::new();
        assert_eq!(t.counter(), 0);
        t.advance();
        t.advance();
        t.advance();
        assert_eq!(t.counter(), 3 * TICKS_PER_STEP);
    }

    #[test]
    fn op_reg_latches_counter_into_value_hi_lo_and_sets_valid() {
        let mut t = SysTimer::new();
        for _ in 0..5 {
            t.advance();
        }
        assert_eq!(t.read_byte(UNIT0_OP_REG + 3) & 0x20, 0, "not valid yet");
        // Trigger the latch: write bit30 (byte index 3, bit position 6).
        t.write_byte(UNIT0_OP_REG + 3, 0x40);
        assert_eq!(t.read_byte(UNIT0_VALUE_LO_REG), (5 * TICKS_PER_STEP) as u8);
        assert_ne!(
            t.read_byte(UNIT0_OP_REG + 3) & 0x20,
            0,
            "VALUE_VALID must be set"
        );
    }

    fn write_word(t: &mut SysTimer, word_offset: u32, val: u32) {
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            t.write_byte(word_offset + i as u32, *b);
        }
    }
    fn read_word(t: &mut SysTimer, word_offset: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = t.read_byte(word_offset + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    /// Arms target0 in periodic mode with the given period, exactly the way
    /// this module's documented `COMP0_LOAD_REG` contract expects: set
    /// PERIOD_MODE + PERIOD first, *then* trigger the load.
    fn arm_periodic(t: &mut SysTimer, period: u32) {
        write_word(t, TARGET0_CONF_REG, TARGET0_PERIOD_MODE | period);
        write_word(t, COMP0_LOAD_REG, COMP0_LOAD_BIT as u32);
        // TARGET0_WORK_EN, preserving TIMER_UNIT0_WORK_EN already in CONF.
        let conf = read_word(t, CONF_REG);
        write_word(t, CONF_REG, conf | TARGET0_WORK_EN);
        write_word(t, INT_ENA_REG, TARGET0_INT_BIT);
    }

    #[test]
    fn target0_fires_at_expected_count_in_periodic_mode() {
        let mut t = SysTimer::new();
        arm_periodic(&mut t, 10);
        assert!(!t.target0_pending());
        for _ in 0..9 {
            t.advance();
        }
        assert!(!t.target0_pending(), "must not fire early");
        t.advance(); // the 10th advance: counter now == target
        assert!(
            t.target0_pending(),
            "must fire exactly at the configured period"
        );
    }

    #[test]
    fn period_mode_rearms_and_fires_again_after_clear() {
        let mut t = SysTimer::new();
        arm_periodic(&mut t, 10);
        for _ in 0..10 {
            t.advance();
        }
        assert!(t.target0_pending());
        write_word(&mut t, INT_CLR_REG, TARGET0_INT_BIT); // ack
        assert!(
            !t.target0_pending(),
            "INT_CLR must actually clear raw status"
        );
        for _ in 0..9 {
            t.advance();
        }
        assert!(
            !t.target0_pending(),
            "must not fire early for the 2nd period"
        );
        t.advance();
        assert!(
            t.target0_pending(),
            "must fire again after a full 2nd period"
        );
    }

    #[test]
    fn oneshot_mode_fires_exactly_once() {
        let mut t = SysTimer::new();
        // Oneshot: explicit absolute TARGET0_HI/LO write (PERIOD_MODE left
        // clear), matching the real `systimer_hal_set_alarm_target` path.
        write_word(&mut t, TARGET0_HI_REG, 0);
        write_word(&mut t, TARGET0_LO_REG, 5);
        let conf = read_word(&mut t, CONF_REG);
        write_word(&mut t, CONF_REG, conf | TARGET0_WORK_EN);
        write_word(&mut t, INT_ENA_REG, TARGET0_INT_BIT);

        for _ in 0..4 {
            t.advance();
        }
        assert!(!t.target0_pending());
        t.advance(); // counter reaches 5
        assert!(t.target0_pending());

        write_word(&mut t, INT_CLR_REG, TARGET0_INT_BIT);
        assert!(!t.target0_pending());
        for _ in 0..50 {
            t.advance();
        }
        assert!(
            !t.target0_pending(),
            "oneshot target must never fire again once cleared"
        );
    }

    #[test]
    fn cpu_int_gating_disabled_line_means_no_pending() {
        let mut t = SysTimer::new();
        arm_periodic(&mut t, 3);
        write_word(&mut t, INT_ENA_REG, 0); // disable target0's interrupt
        for _ in 0..10 {
            t.advance();
        }
        assert_ne!(
            read_word(&mut t, INT_RAW_REG) & TARGET0_INT_BIT,
            0,
            "raw still latches"
        );
        assert!(
            !t.target0_pending(),
            "pending must be raw&ena -- disabled ena must suppress it"
        );
    }

    #[test]
    fn work_en_gate_stops_the_counter_entirely() {
        let mut t = SysTimer::new();
        write_word(&mut t, CONF_REG, 0); // clear TIMER_UNIT0_WORK_EN
        for _ in 0..100 {
            t.advance();
        }
        assert_eq!(
            t.counter(),
            0,
            "counter must not advance while work_en is clear"
        );
    }

    #[test]
    fn int_st_reg_mirrors_raw_and_ena() {
        let mut t = SysTimer::new();
        arm_periodic(&mut t, 2);
        for _ in 0..2 {
            t.advance();
        }
        assert_eq!(read_word(&mut t, INT_ST_REG), TARGET0_INT_BIT);
    }
}

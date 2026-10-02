//! ESP32-C3 SYSTIMER peripheral (`DR_REG_SYSTIMER_BASE = 0x6002_3000`):
//! two 52-bit counters ("units") and three comparators ("targets"/alarms).
//!
//! ## Sources (ESP-IDF v5.5.3, fetched from
//! `raw.githubusercontent.com/espressif/esp-idf/v5.5.3/...`)
//!
//! - `components/soc/esp32c3/register/soc/systimer_reg.h` -- every offset,
//!   field position, access type and reset value below.
//! - `components/soc/esp32c3/include/soc/soc_caps.h` --
//!   `SOC_SYSTIMER_COUNTER_NUM 2`, `SOC_SYSTIMER_ALARM_NUM 3`,
//!   `SOC_SYSTIMER_FIXED_DIVIDER 1`, `SOC_SYSTIMER_INT_LEVEL 1` (the alarm
//!   interrupts are levels: [`SysTimer::pending_sources`] is a pure function
//!   of `INT_RAW & INT_ENA`), `SOC_SYSTIMER_ALARM_MISS_COMPENSATE 1` (an
//!   alarm whose target is already at/behind the counter fires immediately).
//! - `components/hal/esp32c3/include/hal/systimer_ll.h` -- what each HAL
//!   verb writes: `enable_counter(id)` toggles `CONF` bit `30 - id`;
//!   `enable_alarm(id)` toggles `CONF` bit `24 - id`;
//!   `counter_can_stall_by_cpu` toggles `CONF` bit `(28 - 2*id) - cpu`;
//!   `counter_snapshot` writes `UNITn_OP.update`; `set_counter_value` writes
//!   `UNITn_LOAD_HI/LO`; `apply_counter_value` writes 1 to `UNITn_LOAD`;
//!   `set_alarm_target` writes `TARGETn_HI/LO`; `connect_alarm_counter`
//!   writes `TARGETn_CONF.timer_unit_sel`; `enable_alarm_oneshot/period`
//!   write `TARGETn_CONF.period_mode`; `set_alarm_period` writes
//!   `TARGETn_CONF.period` (26 bits); `apply_alarm_value` writes 1 to
//!   `COMPn_LOAD`; `clear_alarm_int` writes `INT_CLR`.
//! - `components/hal/systimer_hal.c` -- `systimer_hal_init` sets
//!   `CONF.CLK_EN`; `systimer_hal_get_counter_value` = UPDATE, spin on
//!   `VALUE_VALID`, read LO/HI/LO; the `MISS_COMPENSATE` variant of
//!   `systimer_hal_set_alarm_target` = disable alarm, write target, pulse
//!   `COMPn_LOAD`, enable; `systimer_hal_set_alarm_period` = disable alarm,
//!   write period, pulse `COMPn_LOAD`, enable.
//! - `components/freertos/port_systick.c::vSystimerSetup` -- the FreeRTOS
//!   tick: **alarm 0 on counter 1** (unicore). Order: `systimer_hal_init`;
//!   counter 1 := 0 via `UNIT1_LOAD_HI/LO` + `UNIT1_LOAD`; alarm 0 oneshot;
//!   connect alarm 0 to counter 1; `set_alarm_period` (so the `COMP0_LOAD`
//!   pulse happens **while still oneshot**); switch to period mode; stall
//!   bits; `INT_ENA |= 1`; `CONF |= UNIT1_WORK_EN`. `SysTickIsrHandler`
//!   never re-arms ("works in periodic mode no need to calc the next
//!   alarm"), so period re-arm is hardware's job.
//! - `components/esp_timer/src/esp_timer_impl_systimer.c` -- esp_timer:
//!   **alarm 2 on counter 0**, oneshot, armed via `systimer_hal_set_alarm_target`.
//!
//! ## Registers modeled (all others: read 0 / write dropped, and logged as
//! unmapped by `FirmwareBus` because [`SysTimer::handles`] excludes them)
//!
//! - `CONF` (0x00): plain R/W storage. Reset value **`0x4600_0000`**:
//!   `TIMER_UNIT0_WORK_EN` (bit 30, default 1), `TIMER_UNIT1_CORE0_STALL_EN`
//!   (bit 26, default 1) and `TIMER_UNIT1_CORE1_STALL_EN` (bit 25, default
//!   1); every other field defaults to 0 (including `CLK_EN`, bit 31, and
//!   `TIMER_UNIT1_WORK_EN`, bit 29). Bit `30 - n` gates whether unit `n`
//!   counts; bit `24 - n` gates whether comparator `n` is checked. `CLK_EN`
//!   is "register file clk gating": this model's registers are always
//!   accessible, so it is stored but gates nothing. The stall-by-CPU bits
//!   are stored but inert (no debugger stall to model).
//! - `UNIT0_OP`/`UNIT1_OP` (0x04/0x08): `UPDATE` (bit 30, WT) latches the
//!   live counter into `UNITn_VALUE_HI/LO` synchronously and sets
//!   `VALUE_VALID` (bit 29, R/SS/WTC). A written 1 in bit 29 clears it
//!   first (WTC), so the HAL's read-modify-write bitfield store of `update`
//!   (which writes the old `VALUE_VALID = 1` back) ends with it set again
//!   by the same store's `UPDATE` -- and `get_counter_value`'s spin exits.
//! - `UNITn_LOAD_HI/LO` (0x0C/0x10, 0x14/0x18): R/W, HI is 20 bits.
//!   `UNIT0_LOAD`/`UNIT1_LOAD` (0x5C/0x60, bit 0 WT): counter `n` := LOAD.
//! - `TARGETn_HI/LO` (0x1C+8n / 0x20+8n): R/W, HI is 20 bits.
//!   `TARGETn_CONF` (0x34+4n): `PERIOD[25:0]`, `PERIOD_MODE` (30),
//!   `TIMER_UNIT_SEL` (31); bits 29:26 are reserved and read 0.
//! - `UNITn_VALUE_HI/LO` (0x40/0x44, 0x48/0x4C): RO latched snapshot.
//! - `COMPn_LOAD` (0x50+4n, bit 0 WT): arms comparator `n` (below).
//! - `INT_ENA` (0x64, R/W), `INT_RAW` (0x68, R/WTC/SS), `INT_CLR` (0x6C,
//!   WT), `INT_ST` (0x70, RO = RAW & ENA); bits 0..2 = targets 0..2.
//!
//! Pulse/trigger bits (`UNITn_OP.UPDATE`, `UNITn_LOAD`, `COMPn_LOAD`, the
//! WTC bits) act on the byte that holds them. `FirmwareBus::write32` splits
//! a word store into 4 ascending byte writes, so each trigger fires exactly
//! once per word store, and any *other* register it reads (`LOAD_HI/LO`,
//! `TARGETn_*`) was written by an earlier, separate store.
//!
//! ## Comparator model
//!
//! The headers say what software writes, not how the comparator combines a
//! `COMPn_LOAD` pulse with `PERIOD_MODE`. FreeRTOS pulses the load while
//! still in oneshot mode with `TARGET0_HI/LO` never written (0), then flips
//! to period mode, and expects alarms at `P, 2P, 3P...` on counter 1;
//! esp_timer writes an absolute target, pulses the load, and expects one
//! alarm at counter 0 >= target (or at once if it's already past --
//! `MISS_COMPENSATE`). One model satisfies both:
//!
//! - On a `COMPn_LOAD` pulse: latch `load_base := counter(TIMER_UNIT_SEL)`
//!   and `oneshot_target := TARGETn_HI/LO`, reset the period index `k := 1`,
//!   and clear the oneshot "fired since load" flag.
//! - In each [`SysTimer::advance_by`], for a comparator enabled in `CONF`
//!   whose unit is working (`PERIOD_MODE`/`PERIOD` read live):
//!   - **period mode**: the next alarm is `load_base + k*PERIOD`; once the
//!     counter reaches it, set `INT_RAW` bit `n` and re-arm to the next
//!     *future* multiple (`k := (counter - load_base) / PERIOD + 1`). A jump
//!     across several periods therefore fires once, not once per period --
//!     the level line can't express more than one anyway, and
//!     `SysTickIsrHandler` catches up by reading the counter.
//!     `PERIOD == 0` never fires (degenerate; nothing in ESP-IDF sets it).
//!   - **oneshot**: fire once when `counter >= oneshot_target` and it hasn't
//!     fired since the last load. A target already in the past fires on the
//!     next tick (`MISS_COMPENSATE`).
//! - A comparator never loaded since reset never fires.
//! - Loading a unit's counter doesn't rebase its comparators: a period
//!   alarm left behind by a forward jump fires once on the next tick and
//!   re-arms to the next future multiple of `load_base`.
//! - Counters are 52 bits and wrap; alarm arithmetic ignores wrap (2^52
//!   ticks at 16 MHz is ~8.9 years).
//!
//! ## Tick rate (placeholder)
//!
//! Real hardware counts at a fixed 16 MHz; `Cpu::step()` has no cycle
//! model, so [`TICKS_PER_STEP`] is 1 tick per step. Only the relative
//! timing of alarms is meaningful.

use crate::mem::soc::SRC_SYSTIMER_TARGET0;

/// Ticks each working counter advances per [`SysTimer::advance`] call, i.e.
/// per `Cpu::step()` in the driving loop (`crate::boot::step_with_interrupts`).
/// See the module doc's "Tick rate" section.
pub const TICKS_PER_STEP: u64 = 1;

const COUNTER_MASK_52: u64 = (1u64 << 52) - 1;
/// `*_HI` registers are 20 bits (`[19:0]`).
const HI_MASK_20: u32 = 0x000F_FFFF;

pub const CONF_REG: u32 = 0x00;
pub const UNIT0_OP_REG: u32 = 0x04;
pub const UNIT1_OP_REG: u32 = 0x08;
pub const UNIT0_LOAD_HI_REG: u32 = 0x0c;
pub const UNIT0_LOAD_LO_REG: u32 = 0x10;
pub const UNIT1_LOAD_HI_REG: u32 = 0x14;
pub const UNIT1_LOAD_LO_REG: u32 = 0x18;
pub const TARGET0_HI_REG: u32 = 0x1c;
pub const TARGET0_LO_REG: u32 = 0x20;
pub const TARGET1_HI_REG: u32 = 0x24;
pub const TARGET1_LO_REG: u32 = 0x28;
pub const TARGET2_HI_REG: u32 = 0x2c;
pub const TARGET2_LO_REG: u32 = 0x30;
pub const TARGET0_CONF_REG: u32 = 0x34;
pub const TARGET1_CONF_REG: u32 = 0x38;
pub const TARGET2_CONF_REG: u32 = 0x3c;
pub const UNIT0_VALUE_HI_REG: u32 = 0x40;
pub const UNIT0_VALUE_LO_REG: u32 = 0x44;
pub const UNIT1_VALUE_HI_REG: u32 = 0x48;
pub const UNIT1_VALUE_LO_REG: u32 = 0x4c;
pub const COMP0_LOAD_REG: u32 = 0x50;
pub const COMP1_LOAD_REG: u32 = 0x54;
pub const COMP2_LOAD_REG: u32 = 0x58;
pub const UNIT0_LOAD_REG: u32 = 0x5c;
pub const UNIT1_LOAD_REG: u32 = 0x60;
pub const INT_ENA_REG: u32 = 0x64;
pub const INT_RAW_REG: u32 = 0x68;
pub const INT_CLR_REG: u32 = 0x6c;
pub const INT_ST_REG: u32 = 0x70;

/// `SYSTIMER_CLK_EN` (CONF bit 31).
pub const CLK_EN: u32 = 1 << 31;
/// `SYSTIMER_TIMER_UNIT0_WORK_EN` (CONF bit 30).
pub const UNIT0_WORK_EN: u32 = 1 << 30;
/// `SYSTIMER_TIMER_UNIT1_WORK_EN` (CONF bit 29).
pub const UNIT1_WORK_EN: u32 = 1 << 29;
/// `SYSTIMER_TARGET0_WORK_EN` (CONF bit 24).
pub const TARGET0_WORK_EN: u32 = 1 << 24;
/// `SYSTIMER_TARGET1_WORK_EN` (CONF bit 23).
pub const TARGET1_WORK_EN: u32 = 1 << 23;
/// `SYSTIMER_TARGET2_WORK_EN` (CONF bit 22).
pub const TARGET2_WORK_EN: u32 = 1 << 22;
/// `SYSTIMER_TARGETn_PERIOD_MODE` (TARGETn_CONF bit 30).
pub const TARGET_PERIOD_MODE: u32 = 1 << 30;
/// `SYSTIMER_TARGETn_TIMER_UNIT_SEL` (TARGETn_CONF bit 31; 0 = unit 0).
pub const TARGET_TIMER_UNIT_SEL: u32 = 1 << 31;
/// `SYSTIMER_TARGETn_PERIOD` (TARGETn_CONF bits 25:0).
const TARGET_PERIOD_MASK: u32 = 0x03FF_FFFF;
/// Writable TARGETn_CONF bits (29:26 are reserved).
const TARGET_CONF_MASK: u32 = TARGET_TIMER_UNIT_SEL | TARGET_PERIOD_MODE | TARGET_PERIOD_MASK;
/// Reset value of `SYSTIMER_CONF_REG` per `systimer_reg.h`'s field defaults:
/// `TIMER_UNIT0_WORK_EN` | `TIMER_UNIT1_CORE0_STALL_EN` | `TIMER_UNIT1_CORE1_STALL_EN`.
pub const CONF_RESET: u32 = UNIT0_WORK_EN | (1 << 26) | (1 << 25);

/// `TIMER_UNITn_UPDATE` (OP bit 30) within byte 3.
const OP_UPDATE_BYTE3: u8 = 0x40;
/// `TIMER_UNITn_VALUE_VALID` (OP bit 29) within byte 3 (WTC on write).
const OP_VALUE_VALID_BYTE3: u8 = 0x20;
const OP_VALUE_VALID: u32 = 1 << 29;
/// Bits 0..2 of INT_ENA/RAW/CLR/ST.
const INT_MASK: u32 = 0b111;

const UNIT_COUNT: usize = 2;
const COMP_COUNT: usize = 3;

#[derive(Debug, Clone, Default)]
struct Unit {
    counter: u64,
    load_hi: u32,
    load_lo: u32,
    value_hi: u32,
    value_lo: u32,
    value_valid: bool,
}

/// What a `COMPn_LOAD` pulse latched. See the module doc's comparator model.
#[derive(Debug, Clone, Copy)]
struct Armed {
    load_base: u64,
    oneshot_target: u64,
    oneshot_fired: bool,
    /// Index of the next period alarm, `load_base + k * PERIOD`.
    period_k: u64,
}

#[derive(Debug, Clone, Default)]
struct Comparator {
    hi: u32,
    lo: u32,
    conf: u32,
    armed: Option<Armed>,
}

impl Comparator {
    fn unit(&self) -> usize {
        usize::from(self.conf & TARGET_TIMER_UNIT_SEL != 0)
    }

    fn period_mode(&self) -> bool {
        self.conf & TARGET_PERIOD_MODE != 0
    }

    fn period(&self) -> u64 {
        u64::from(self.conf & TARGET_PERIOD_MASK)
    }

    /// The counter value at which this comparator next fires, if armed.
    fn next_alarm(&self) -> Option<u64> {
        let a = self.armed?;
        if self.period_mode() {
            let p = self.period();
            (p != 0).then(|| a.load_base.saturating_add(a.period_k.saturating_mul(p)))
        } else {
            (!a.oneshot_fired).then_some(a.oneshot_target)
        }
    }
}

/// The SYSTIMER peripheral. See the module doc.
#[derive(Debug, Clone)]
pub struct SysTimer {
    conf: u32,
    units: [Unit; UNIT_COUNT],
    comps: [Comparator; COMP_COUNT],
    int_ena: u32,
    int_raw: u32,
    /// Total ticks ever fed to [`SysTimer::advance_by`], independent of
    /// unit work-enables and counter loads -- the emulator's monotonic
    /// notion of elapsed time (see [`SysTimer::elapsed_ticks`]).
    elapsed: u64,
}

impl Default for SysTimer {
    fn default() -> Self {
        Self {
            conf: CONF_RESET,
            units: Default::default(),
            comps: Default::default(),
            int_ena: 0,
            int_raw: 0,
            elapsed: 0,
        }
    }
}

impl SysTimer {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset`'s word is one this module models (see the module
    /// doc) rather than its read-0/write-dropped catch-all. Used by
    /// `crate::mem::bus::FirmwareBus` to log accesses to everything else as
    /// unmapped.
    pub fn handles(offset: u32) -> bool {
        let o = offset & !0b11;
        o <= INT_ST_REG
    }

    /// The live 52-bit counter of unit `unit` (0 or 1). Not itself a
    /// register -- firmware reads the `UNITn_VALUE_HI/LO` snapshot.
    ///
    /// # Panics
    /// If `unit >= 2`.
    pub fn counter(&self, unit: usize) -> u64 {
        self.units[unit].counter
    }

    /// Monotonic ticks since reset: the sum of every [`SysTimer::advance_by`]
    /// amount, unaffected by `UNITn_WORK_EN` or `UNITn_LOAD`. The RTC timer
    /// (`crate::peripherals::rtc_cntl`) derives its elapsed time from this.
    pub fn elapsed_ticks(&self) -> u64 {
        self.elapsed
    }

    fn unit_working(&self, unit: usize) -> bool {
        self.conf & (UNIT0_WORK_EN >> unit) != 0
    }

    fn comp_enabled(&self, n: usize) -> bool {
        self.conf & (TARGET0_WORK_EN >> n) != 0
    }

    /// Interrupt sources currently asserted, as a mask over source numbers
    /// (`SRC_SYSTIMER_TARGET0 + n` for each target `n` with `INT_RAW & INT_ENA`
    /// bit `n` set) -- level, recomputed from live state.
    pub fn pending_sources(&self) -> u64 {
        u64::from(self.int_raw & self.int_ena & INT_MASK) << SRC_SYSTIMER_TARGET0
    }

    /// [`SysTimer::advance_by`]`(`[`TICKS_PER_STEP`]`)`. Call once per
    /// `Cpu::step()` -- see `crate::boot::step_with_interrupts`.
    pub fn advance(&mut self) {
        self.advance_by(TICKS_PER_STEP);
    }

    /// Advances every working counter by `ticks` and fires each enabled
    /// comparator whose alarm was reached within the jump (once, re-arming
    /// a period comparator to its next future multiple). See the module
    /// doc's comparator model.
    pub fn advance_by(&mut self, ticks: u64) {
        self.elapsed = self.elapsed.wrapping_add(ticks);
        for u in 0..UNIT_COUNT {
            if self.unit_working(u) {
                let c = &mut self.units[u].counter;
                *c = c.wrapping_add(ticks) & COUNTER_MASK_52;
            }
        }
        for n in 0..COMP_COUNT {
            let unit = self.comps[n].unit();
            if !self.comp_enabled(n) || !self.unit_working(unit) {
                continue;
            }
            let now = self.units[unit].counter;
            let comp = &mut self.comps[n];
            let Some(target) = comp.next_alarm() else {
                continue;
            };
            if now < target {
                continue;
            }
            self.int_raw |= 1 << n;
            let period_mode = comp.period_mode();
            let period = comp.period();
            let a = comp.armed.as_mut().expect("next_alarm() implies armed");
            if period_mode {
                // now >= target > load_base here, so no underflow.
                a.period_k = (now - a.load_base) / period + 1;
            } else {
                a.oneshot_fired = true;
            }
        }
    }

    /// Smallest positive number of ticks until some comparator that could
    /// raise an interrupt (enabled in `CONF`, its `INT_ENA` bit set, its unit
    /// working, armed) fires; an alarm already due counts as 1 (it fires on
    /// the next tick). `None` if nothing qualifies. For WFI fast-forward.
    pub fn ticks_until_next_alarm(&self) -> Option<u64> {
        (0..COMP_COUNT)
            .filter_map(|n| {
                let comp = &self.comps[n];
                let unit = comp.unit();
                if !self.comp_enabled(n) || self.int_ena & (1 << n) == 0 || !self.unit_working(unit)
                {
                    return None;
                }
                let now = self.units[unit].counter;
                comp.next_alarm().map(|t| t.saturating_sub(now).max(1))
            })
            .min()
    }

    /// `COMPn_LOAD` pulse: see the module doc's comparator model.
    fn load_comparator(&mut self, n: usize) {
        let comp = &self.comps[n];
        let load_base = self.units[comp.unit()].counter;
        let oneshot_target = (u64::from(comp.hi & HI_MASK_20) << 32) | u64::from(comp.lo);
        self.comps[n].armed = Some(Armed {
            load_base,
            oneshot_target,
            oneshot_fired: false,
            period_k: 1,
        });
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            CONF_REG => self.conf,
            UNIT0_OP_REG | UNIT1_OP_REG => {
                let u = ((word_offset - UNIT0_OP_REG) / 4) as usize;
                if self.units[u].value_valid {
                    OP_VALUE_VALID
                } else {
                    0
                }
            }
            UNIT0_LOAD_HI_REG | UNIT1_LOAD_HI_REG => {
                self.units[((word_offset - UNIT0_LOAD_HI_REG) / 8) as usize].load_hi
            }
            UNIT0_LOAD_LO_REG | UNIT1_LOAD_LO_REG => {
                self.units[((word_offset - UNIT0_LOAD_LO_REG) / 8) as usize].load_lo
            }
            TARGET0_HI_REG | TARGET1_HI_REG | TARGET2_HI_REG => {
                self.comps[((word_offset - TARGET0_HI_REG) / 8) as usize].hi
            }
            TARGET0_LO_REG | TARGET1_LO_REG | TARGET2_LO_REG => {
                self.comps[((word_offset - TARGET0_LO_REG) / 8) as usize].lo
            }
            TARGET0_CONF_REG | TARGET1_CONF_REG | TARGET2_CONF_REG => {
                self.comps[((word_offset - TARGET0_CONF_REG) / 4) as usize].conf
            }
            UNIT0_VALUE_HI_REG | UNIT1_VALUE_HI_REG => {
                self.units[((word_offset - UNIT0_VALUE_HI_REG) / 8) as usize].value_hi
            }
            UNIT0_VALUE_LO_REG | UNIT1_VALUE_LO_REG => {
                self.units[((word_offset - UNIT0_VALUE_LO_REG) / 8) as usize].value_lo
            }
            INT_ENA_REG => self.int_ena,
            INT_RAW_REG => self.int_raw,
            INT_ST_REG => self.int_raw & self.int_ena & INT_MASK,
            // COMPn_LOAD / UNITn_LOAD (WT) and INT_CLR (WT): write-only, no
            // readable storage -- read 0. Everything unmodeled also reads 0.
            _ => 0,
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            CONF_REG => super::set_byte(&mut self.conf, idx, val),
            UNIT0_OP_REG | UNIT1_OP_REG => {
                let unit = &mut self.units[((word_offset - UNIT0_OP_REG) / 4) as usize];
                if idx == 3 {
                    if val & OP_VALUE_VALID_BYTE3 != 0 {
                        unit.value_valid = false; // WTC
                    }
                    if val & OP_UPDATE_BYTE3 != 0 {
                        unit.value_hi = ((unit.counter >> 32) as u32) & HI_MASK_20;
                        unit.value_lo = unit.counter as u32;
                        unit.value_valid = true;
                    }
                }
            }
            UNIT0_LOAD_HI_REG | UNIT1_LOAD_HI_REG => {
                let unit = &mut self.units[((word_offset - UNIT0_LOAD_HI_REG) / 8) as usize];
                super::set_byte(&mut unit.load_hi, idx, val);
                unit.load_hi &= HI_MASK_20;
            }
            UNIT0_LOAD_LO_REG | UNIT1_LOAD_LO_REG => {
                let unit = &mut self.units[((word_offset - UNIT0_LOAD_LO_REG) / 8) as usize];
                super::set_byte(&mut unit.load_lo, idx, val);
            }
            TARGET0_HI_REG | TARGET1_HI_REG | TARGET2_HI_REG => {
                let comp = &mut self.comps[((word_offset - TARGET0_HI_REG) / 8) as usize];
                super::set_byte(&mut comp.hi, idx, val);
                comp.hi &= HI_MASK_20;
            }
            TARGET0_LO_REG | TARGET1_LO_REG | TARGET2_LO_REG => {
                let comp = &mut self.comps[((word_offset - TARGET0_LO_REG) / 8) as usize];
                super::set_byte(&mut comp.lo, idx, val);
            }
            TARGET0_CONF_REG | TARGET1_CONF_REG | TARGET2_CONF_REG => {
                let comp = &mut self.comps[((word_offset - TARGET0_CONF_REG) / 4) as usize];
                super::set_byte(&mut comp.conf, idx, val);
                comp.conf &= TARGET_CONF_MASK;
            }
            COMP0_LOAD_REG | COMP1_LOAD_REG | COMP2_LOAD_REG => {
                if idx == 0 && val & 1 != 0 {
                    self.load_comparator(((word_offset - COMP0_LOAD_REG) / 4) as usize);
                }
            }
            UNIT0_LOAD_REG | UNIT1_LOAD_REG => {
                if idx == 0 && val & 1 != 0 {
                    let unit = &mut self.units[((word_offset - UNIT0_LOAD_REG) / 4) as usize];
                    unit.counter = ((u64::from(unit.load_hi) << 32) | u64::from(unit.load_lo))
                        & COUNTER_MASK_52;
                }
            }
            INT_ENA_REG => {
                super::set_byte(&mut self.int_ena, idx, val);
                self.int_ena &= INT_MASK;
            }
            // INT_RAW is R/WTC/SS; INT_CLR is WT. Either way a written 1 in
            // bits 0..2 (byte 0) clears that raw bit.
            INT_RAW_REG | INT_CLR_REG if idx == 0 => {
                self.int_raw &= !(u32::from(val) & INT_MASK);
            }
            // UNITn_VALUE_*, INT_ST are RO; everything unmodeled is dropped.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_word(t: &mut SysTimer, word_offset: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| t.read_byte(word_offset + i)))
    }

    fn target_pending(t: &SysTimer, n: u32) -> bool {
        t.pending_sources() & (1u64 << (SRC_SYSTIMER_TARGET0 + n)) != 0
    }

    /// Arms target0 on unit 0 in period mode the way the Milestone-2 tests
    /// did (period mode set *before* the load pulse; still valid under the
    /// comparator model).
    fn arm_periodic_unit0(t: &mut SysTimer, period: u32) {
        w(t, TARGET0_CONF_REG, TARGET_PERIOD_MODE | period);
        w(t, COMP0_LOAD_REG, 1);
        let conf = read_word(t, CONF_REG);
        w(t, CONF_REG, conf | TARGET0_WORK_EN);
        w(t, INT_ENA_REG, 1);
    }

    #[test]
    fn conf_reset_value_matches_header_defaults() {
        let mut t = SysTimer::new();
        assert_eq!(read_word(&mut t, CONF_REG), 0x4600_0000);
    }

    #[test]
    fn only_unit0_counts_out_of_reset() {
        let mut t = SysTimer::new();
        for _ in 0..3 {
            t.advance();
        }
        assert_eq!(t.counter(0), 3 * TICKS_PER_STEP);
        assert_eq!(t.counter(1), 0, "UNIT1_WORK_EN resets to 0");
        assert_eq!(t.elapsed_ticks(), 3 * TICKS_PER_STEP);
    }

    #[test]
    fn work_en_gate_stops_a_counter_but_not_elapsed_ticks() {
        let mut t = SysTimer::new();
        w(&mut t, CONF_REG, 0);
        t.advance_by(100);
        assert_eq!(t.counter(0), 0);
        assert_eq!(t.elapsed_ticks(), 100);
    }

    #[test]
    fn op_update_word_write_latches_value_and_sets_valid() {
        let mut t = SysTimer::new();
        t.advance_by(5);
        assert_eq!(
            read_word(&mut t, UNIT0_OP_REG) & (1 << 29),
            0,
            "not valid yet"
        );
        w(&mut t, UNIT0_OP_REG, 1 << 30);
        assert_eq!(read_word(&mut t, UNIT0_VALUE_LO_REG), 5);
        assert_eq!(read_word(&mut t, UNIT0_VALUE_HI_REG), 0);
        assert_ne!(read_word(&mut t, UNIT0_OP_REG) & (1 << 29), 0);
        // The HAL's RMW bitfield store writes VALUE_VALID (WTC) back along
        // with UPDATE: still valid afterwards, with the fresh value.
        t.advance_by(7);
        let op = read_word(&mut t, UNIT0_OP_REG);
        w(&mut t, UNIT0_OP_REG, op | (1 << 30));
        assert_ne!(read_word(&mut t, UNIT0_OP_REG) & (1 << 29), 0);
        assert_eq!(read_word(&mut t, UNIT0_VALUE_LO_REG), 12);
        // VALUE_VALID alone (WTC, no UPDATE) clears it.
        w(&mut t, UNIT0_OP_REG, 1 << 29);
        assert_eq!(read_word(&mut t, UNIT0_OP_REG) & (1 << 29), 0);
    }

    #[test]
    fn unit_value_is_a_snapshot_not_live() {
        let mut t = SysTimer::new();
        w(&mut t, CONF_REG, UNIT0_WORK_EN | UNIT1_WORK_EN);
        t.advance_by(10);
        w(&mut t, UNIT1_OP_REG, 1 << 30);
        t.advance_by(10);
        assert_eq!(read_word(&mut t, UNIT1_VALUE_LO_REG), 10);
        assert_eq!(t.counter(1), 20);
    }

    #[test]
    fn unit_load_word_write_loads_once_from_already_written_load_regs() {
        let mut t = SysTimer::new();
        w(&mut t, UNIT0_LOAD_HI_REG, 0xFFFF_FFFF); // HI is 20 bits
        assert_eq!(read_word(&mut t, UNIT0_LOAD_HI_REG), 0x000F_FFFF);
        w(&mut t, UNIT0_LOAD_HI_REG, 0);
        w(&mut t, UNIT0_LOAD_LO_REG, 1000);
        t.advance_by(3);
        w(&mut t, UNIT0_LOAD_REG, 1);
        assert_eq!(t.counter(0), 1000);
        assert_eq!(read_word(&mut t, UNIT0_LOAD_REG), 0, "WT reads 0");
    }

    #[test]
    fn target0_fires_at_expected_count_in_periodic_mode() {
        let mut t = SysTimer::new();
        arm_periodic_unit0(&mut t, 10);
        t.advance_by(9);
        assert!(!target_pending(&t, 0), "must not fire early");
        t.advance();
        assert!(target_pending(&t, 0));
    }

    #[test]
    fn period_mode_rearms_and_fires_again_after_clear() {
        let mut t = SysTimer::new();
        arm_periodic_unit0(&mut t, 10);
        t.advance_by(10);
        assert!(target_pending(&t, 0));
        w(&mut t, INT_CLR_REG, 1);
        assert!(!target_pending(&t, 0));
        t.advance_by(9);
        assert!(!target_pending(&t, 0));
        t.advance();
        assert!(target_pending(&t, 0));
    }

    #[test]
    fn oneshot_fires_once_and_again_only_after_a_rearm() {
        let mut t = SysTimer::new();
        w(&mut t, TARGET2_LO_REG, 5);
        w(&mut t, COMP2_LOAD_REG, 1);
        w(&mut t, CONF_REG, UNIT0_WORK_EN | TARGET2_WORK_EN);
        w(&mut t, INT_ENA_REG, 1 << 2);
        t.advance_by(4);
        assert!(!target_pending(&t, 2));
        t.advance();
        assert!(target_pending(&t, 2));
        w(&mut t, INT_CLR_REG, 1 << 2);
        t.advance_by(50);
        assert!(!target_pending(&t, 2), "never again without a re-arm");
        assert_eq!(t.ticks_until_next_alarm(), None);
        // esp_timer re-arm: disable, new target, load, enable.
        w(&mut t, CONF_REG, UNIT0_WORK_EN);
        w(&mut t, TARGET2_LO_REG, 100);
        w(&mut t, COMP2_LOAD_REG, 1);
        w(&mut t, CONF_REG, UNIT0_WORK_EN | TARGET2_WORK_EN);
        assert_eq!(t.ticks_until_next_alarm(), Some(45));
        t.advance_by(45);
        assert!(target_pending(&t, 2));
    }

    #[test]
    fn never_loaded_comparator_never_fires() {
        let mut t = SysTimer::new();
        w(&mut t, CONF_REG, UNIT0_WORK_EN | TARGET0_WORK_EN);
        w(&mut t, INT_ENA_REG, 1);
        t.advance_by(1000);
        assert_eq!(t.pending_sources(), 0);
    }

    #[test]
    fn comparator_on_a_stopped_unit_does_not_fire() {
        let mut t = SysTimer::new();
        w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL); // unit 1, target 0 (past)
        w(&mut t, COMP0_LOAD_REG, 1);
        w(&mut t, CONF_REG, UNIT0_WORK_EN | TARGET0_WORK_EN); // unit 1 stopped
        w(&mut t, INT_ENA_REG, 1);
        t.advance_by(10);
        assert_eq!(t.pending_sources(), 0);
        assert_eq!(t.ticks_until_next_alarm(), None);
    }

    #[test]
    fn int_ena_gates_pending_but_not_raw() {
        let mut t = SysTimer::new();
        arm_periodic_unit0(&mut t, 3);
        w(&mut t, INT_ENA_REG, 0);
        t.advance_by(10);
        assert_ne!(read_word(&mut t, INT_RAW_REG) & 1, 0, "raw still latches");
        assert_eq!(t.pending_sources(), 0);
        assert_eq!(read_word(&mut t, INT_ST_REG), 0);
        assert_eq!(
            t.ticks_until_next_alarm(),
            None,
            "INT_ENA clear: can't wake"
        );
    }

    #[test]
    fn int_st_mirrors_raw_and_ena_and_raw_is_write_one_to_clear() {
        let mut t = SysTimer::new();
        arm_periodic_unit0(&mut t, 2);
        t.advance_by(2);
        assert_eq!(read_word(&mut t, INT_ST_REG), 1);
        w(&mut t, INT_RAW_REG, 1); // R/WTC
        assert_eq!(read_word(&mut t, INT_RAW_REG), 0);
        assert_eq!(read_word(&mut t, INT_CLR_REG), 0, "WT reads 0");
    }

    #[test]
    fn three_targets_assert_their_own_sources() {
        let mut t = SysTimer::new();
        for (n, tgt) in [(0u32, 10u32), (1, 20), (2, 30)] {
            w(&mut t, TARGET0_LO_REG + 8 * n, tgt);
            w(&mut t, COMP0_LOAD_REG + 4 * n, 1);
        }
        w(
            &mut t,
            CONF_REG,
            UNIT0_WORK_EN | TARGET0_WORK_EN | TARGET1_WORK_EN | TARGET2_WORK_EN,
        );
        w(&mut t, INT_ENA_REG, 0b111);
        assert_eq!(t.ticks_until_next_alarm(), Some(10));
        t.advance_by(20);
        assert_eq!(t.pending_sources(), 0b011 << 37);
        assert_eq!(t.ticks_until_next_alarm(), Some(10));
        t.advance_by(10);
        assert_eq!(t.pending_sources(), 0b111 << 37);
    }

    #[test]
    fn handles_covers_every_modeled_register_and_nothing_past_int_st() {
        for off in (0..=INT_ST_REG).step_by(4) {
            assert!(SysTimer::handles(off), "{off:#x}");
        }
        assert!(!SysTimer::handles(0x74));
        assert!(!SysTimer::handles(0xfc)); // DATE: not modeled
    }

    #[test]
    fn target_conf_reserved_bits_read_zero() {
        let mut t = SysTimer::new();
        w(&mut t, TARGET1_CONF_REG, 0xFFFF_FFFF);
        assert_eq!(read_word(&mut t, TARGET1_CONF_REG), 0xC3FF_FFFF);
    }

    // ---- Milestone 3 Task 5: literal ESP-IDF v5.5.3 HAL register sequences ----

    fn w(t: &mut SysTimer, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            t.write_byte(off + i as u32, *b);
        }
    }

    #[test]
    fn freertos_tick_alarm0_on_counter1_period_mode_fires_every_period() {
        let mut t = SysTimer::new();
        // Literal vSystimerSetup order (port_systick.c, see module doc).
        let conf =
            |t: &mut SysTimer| u32::from_le_bytes([0, 1, 2, 3].map(|i| t.read_byte(CONF_REG + i)));
        let c = conf(&mut t);
        w(&mut t, CONF_REG, c | CLK_EN); // (1)
        w(&mut t, UNIT1_LOAD_HI_REG, 0);
        w(&mut t, UNIT1_LOAD_LO_REG, 0);
        w(&mut t, UNIT1_LOAD_REG, 1); // (2)
        w(&mut t, TARGET0_CONF_REG, 0); // (3) oneshot
        w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL); // (4) unit1
        let c = conf(&mut t);
        w(&mut t, CONF_REG, c & !TARGET0_WORK_EN); // (5) disable
        w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL | 100); //     period
        w(&mut t, COMP0_LOAD_REG, 1); //     load (still oneshot)
        let c = conf(&mut t);
        w(&mut t, CONF_REG, c | TARGET0_WORK_EN); //     enable
        w(
            &mut t,
            TARGET0_CONF_REG,
            TARGET_TIMER_UNIT_SEL | TARGET_PERIOD_MODE | 100,
        ); // (6) period mode
        w(&mut t, INT_ENA_REG, 1); // (8)
        let c = conf(&mut t);
        w(&mut t, CONF_REG, c | UNIT1_WORK_EN); // (9)
        let mut fired_at = vec![];
        for step in 1..=350u64 {
            t.advance_by(1);
            if t.pending_sources() & (1 << 37) != 0 {
                fired_at.push(step);
                w(&mut t, INT_CLR_REG, 1);
            }
        }
        assert_eq!(fired_at, vec![100, 200, 300]);
    }

    #[test]
    fn esp_timer_alarm2_on_counter0_oneshot_fires_once_at_target() {
        let mut t = SysTimer::new();
        w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN);
        w(&mut t, TARGET2_CONF_REG, 0); // unit0, oneshot
        w(&mut t, TARGET2_HI_REG, 0);
        w(&mut t, TARGET2_LO_REG, 50);
        w(&mut t, COMP2_LOAD_REG, 1);
        w(&mut t, INT_ENA_REG, 1 << 2);
        w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN | TARGET2_WORK_EN);
        let mut fired = 0;
        for _ in 0..200 {
            t.advance_by(1);
            if t.pending_sources() & (1 << 39) != 0 {
                fired += 1;
                w(&mut t, INT_CLR_REG, 1 << 2);
            }
        }
        assert_eq!(fired, 1);
    }

    #[test]
    fn unit_load_sets_counter_and_op_update_latches_value() {
        let mut t = SysTimer::new();
        w(&mut t, CONF_REG, CLK_EN | UNIT1_WORK_EN);
        w(&mut t, UNIT1_LOAD_HI_REG, 0x1);
        w(&mut t, UNIT1_LOAD_LO_REG, 0x2);
        w(&mut t, UNIT1_LOAD_REG, 1);
        w(&mut t, UNIT1_OP_REG, 1 << 30);
        assert_eq!(t.counter(1), (1u64 << 32) | 2);
    }

    #[test]
    fn advance_by_large_jump_fires_period_alarm_once_and_rearms_in_future() {
        let mut t = SysTimer::new();
        // Same setup as the FreeRTOS test, condensed (load at counter1 = 0, period 100):
        w(&mut t, TARGET0_CONF_REG, TARGET_TIMER_UNIT_SEL | 100);
        w(&mut t, COMP0_LOAD_REG, 1);
        w(
            &mut t,
            TARGET0_CONF_REG,
            TARGET_TIMER_UNIT_SEL | TARGET_PERIOD_MODE | 100,
        );
        w(&mut t, INT_ENA_REG, 1);
        w(&mut t, CONF_REG, CLK_EN | UNIT1_WORK_EN | TARGET0_WORK_EN);
        t.advance_by(1050); // crosses 100..=1000 (10 alarms) in one jump
        assert_ne!(t.pending_sources() & (1 << 37), 0, "fires (level, once)");
        assert_eq!(
            t.ticks_until_next_alarm(),
            Some(50),
            "re-armed to 1100, not replaying missed periods"
        );
        w(&mut t, INT_CLR_REG, 1);
        assert_eq!(t.pending_sources(), 0);
    }

    #[test]
    fn oneshot_target_already_in_the_past_fires_on_next_tick() {
        let mut t = SysTimer::new();
        w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN);
        t.advance_by(500);
        w(&mut t, TARGET2_LO_REG, 10); // already passed (MISS_COMPENSATE)
        w(&mut t, COMP2_LOAD_REG, 1);
        w(&mut t, INT_ENA_REG, 1 << 2);
        w(&mut t, CONF_REG, CLK_EN | UNIT0_WORK_EN | TARGET2_WORK_EN);
        t.advance_by(1);
        assert_ne!(t.pending_sources() & (1 << 39), 0);
    }

    #[test]
    fn ticks_until_next_alarm_is_none_when_nothing_armed() {
        assert_eq!(SysTimer::new().ticks_until_next_alarm(), None);
    }
}

//! ESP32-C3 TIMG0/TIMG1 (timer group) peripherals — just enough to unblock
//! `rtc_clk_cal_internal()`'s RTC slow-clock calibration spin loop (the
//! pre-Task-3 boot stall: hot PCs confined to `0x4038c80c`-`0x4038c822`,
//! polling `RTCCALICFG_REG`/`RTCCALICFG1_REG` and getting a constant `0`
//! back forever) plus inert storage for the two general-purpose timers and
//! both MWDT (main watchdog) blocks so probing/configuring them never
//! panics or wedges boot.
//!
//! Register layout confirmed against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/timer_group_reg.h` (`DR_REG_TIMG_BASE(i)
//! = REG_TIMG_BASE(i) = DR_REG_TIMERGROUP0_BASE + i*0x1000`, and
//! `DR_REG_TIMERGROUP0_BASE == 0x6001_F000` per
//! `components/soc/esp32c3/include/soc/reg_base.h` — i.e. TIMG0 and TIMG1
//! are two back-to-back 4 KiB register pages). The calibration algorithm
//! itself (what a write to `RTC_CALI_START` actually computes) is modeled
//! from `components/esp_hw_support/port/esp32c3/rtc_time.c`'s
//! `rtc_clk_cal_internal()`: it configures `RTC_CALI_CLK_SEL` +
//! `RTC_CALI_MAX` (the slow-clock cycle count to time), pulses
//! `RTC_CALI_START`, then polls `RTC_CALI_RDY` before reading
//! `RTCCALICFG1_REG`'s `RTC_CALI_VALUE` field — real hardware counts how
//! many 40 MHz XTAL cycles elapse during `RTC_CALI_MAX` cycles of the
//! selected slow clock, i.e. `VALUE = MAX * xtal_hz / slow_hz`. The three
//! slow-clock frequency constants (`SOC_CLK_RC_SLOW_FREQ_APPROX`,
//! `SOC_CLK_RC_FAST_FREQ_APPROX`, `SOC_CLK_XTAL32K_FREQ_APPROX`) are from
//! `components/soc/esp32c3/include/soc/clk_tree_defs.h`; the 40 MHz XTAL
//! figure matches this badge's own confirmed hardware fact (see the plan's
//! handoff notes). All four header/source files fetched this task from
//! `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`; no
//! offset or bitfield below was guessed.
//!
//! ## Modeled registers
//! - `TIMG_WDTCONFIG0..5_REG` (0x48-0x5C), `TIMG_WDTFEED_REG` (0x60),
//!   `TIMG_WDTWPROTECT_REG` (0x64): plain read/write storage, no side
//!   effects — the watchdog itself is never implemented (it never counts
//!   down, never fires), matching the brief's explicit scope for this task.
//!   These registers need no special-casing below; they live inside the
//!   general word-array storage every register in `0x00..0x100` gets by
//!   default (see [`Timg`]'s doc).
//! - `TIMG_RTCCALICFG_REG` (0x68): `RTC_CALI_START_CYCLING` (bit 12),
//!   `RTC_CALI_CLK_SEL` (bits 14:13, "0:rtc slow clock. 1:clk_8m,
//!   2:xtal_32k." per the header comment), `RTC_CALI_MAX` (bits 30:16),
//!   `RTC_CALI_START` (bit 31) are plain read/write storage. `RTC_CALI_RDY`
//!   (bit 15, RO) is **not** stored as a raw bit — it's synthesized on every
//!   read from [`Timg`]'s own `cali_rdy` bool, so a software write can never
//!   forge it (see `write_byte`'s doc comment on the byte-3 trigger).
//! - `TIMG_RTCCALICFG1_REG` (0x6C): `RTC_CALI_VALUE` (bits 31:7, RO) is
//!   fully computed from the last completed calibration's result, not
//!   stored as raw written bytes (there's nothing to write here — the
//!   register is entirely read-only on real hardware). `RTC_CALI_CYCLING_DATA_VLD`
//!   (bit 0) isn't modeled (this task only implements the one-shot
//!   `RTC_CALI_START` path `rtc_clk_cal_internal` actually uses at boot, not
//!   the free-running `RTC_CALI_START_CYCLING` path); reads as 0.
//! - `TIMG_RTCCALICFG2_REG` (0x80): `RTC_CALI_TIMEOUT` (bit 0, RO) always
//!   reads 0 — this model's calibration always completes instantly and
//!   synchronously, so a timeout can never occur. `RTC_CALI_TIMEOUT_RST_CNT`/
//!   `RTC_CALI_TIMEOUT_THRES` (both R/W) fall through to plain storage like
//!   the watchdog registers.
//!
//! ## Calibration model
//!
//! A write that sets `RTC_CALI_START` (bit 31 — byte index 3, bit 7 of that
//! byte) completes the calibration **instantly**, synchronously within the
//! same write, no polling delay to model:
//! ```text
//! RDY := 1
//! VALUE := MAX * 40_000_000 / slow_hz(CLK_SEL)
//! slow_hz = [136_000, 17_500_000 / 256, 32_768, 136_000][CLK_SEL]
//! ```
//! (index 3 isn't a real `RTC_CALI_CLK_SEL` encoding per the header's 2-bit
//! field comment — `[0, 1, 2]` are the only documented values; it's included
//! only so the match is total and an undocumented `CLK_SEL == 3` still
//! produces *some* deterministic, non-panicking result rather than an
//! unreachable!(), falling back to the same 136 kHz RC_SLOW figure as `0`).
//! A write that *clears* `RTC_CALI_START` sets `RDY := 0` (matching
//! `rtc_clk_cal_internal`'s own final `CLEAR_PERI_REG_MASK(...,
//! TIMG_RTC_CALI_START)`, which the *next* calibration's own preceding
//! `CLEAR_PERI_REG_MASK` + `SET_PERI_REG_MASK` pair relies on to force a
//! fresh 0->1 edge). `VALUE` is masked to the field's real 25-bit width
//! (`RTC_CALI_VALUE_V == 0x01FF_FFFF`) so an extreme `MAX`/`CLK_SEL`
//! combination wraps the way a real 25-bit hardware field would, rather
//! than silently growing past what the register could ever actually hold.

use super::set_byte;

/// End of the modeled register window (word-array storage): every named
/// register above (`0x00..=0xFC` per the header) lives inside this range.
/// Offsets at or past this read 0 / drop, per the brief's explicit
/// simplification — there is no real ESP-IDF-touched register beyond
/// `TIMG_REGCLK_REG` (0xFC) in this task's scope.
const REGS_END: u32 = 0x100;
const REGS_WORDS: usize = (REGS_END / 4) as usize;

pub const WDTCONFIG0_REG: u32 = 0x48;
pub const WDTCONFIG1_REG: u32 = 0x4c;
pub const WDTCONFIG2_REG: u32 = 0x50;
pub const WDTCONFIG3_REG: u32 = 0x54;
pub const WDTCONFIG4_REG: u32 = 0x58;
pub const WDTCONFIG5_REG: u32 = 0x5c;
pub const WDTFEED_REG: u32 = 0x60;
pub const WDTWPROTECT_REG: u32 = 0x64;
pub const RTCCALICFG_REG: u32 = 0x68;
pub const RTCCALICFG1_REG: u32 = 0x6c;
pub const RTCCALICFG2_REG: u32 = 0x80;

/// `RTC_CALI_START_CYCLING` is bit 12 of `RTCCALICFG_REG` — plain read/write
/// storage handled generically by the `regs` array (see the module doc),
/// so no dedicated constant/mask is needed here.
/// `RTC_CALI_CLK_SEL`, bits [14:13].
const RTC_CALI_CLK_SEL_SHIFT: u32 = 13;
const RTC_CALI_CLK_SEL_MASK: u32 = 0b11;
/// `RTC_CALI_RDY`, bit 15 (RO — synthesized on read, see the module doc).
pub const RTC_CALI_RDY: u32 = 1 << 15;
/// `RTC_CALI_MAX`, bits [30:16].
const RTC_CALI_MAX_SHIFT: u32 = 16;
const RTC_CALI_MAX_MASK: u32 = 0x7FFF;
/// `RTC_CALI_START`, bit 31 — byte index 3, bit 7 of that byte.
const RTC_CALI_START_BYTE3_BIT: u8 = 0x80;
/// `RTC_CALI_VALUE`, bits [31:7] of `RTCCALICFG1_REG` (25 bits wide).
const RTC_CALI_VALUE_SHIFT: u32 = 7;
const RTC_CALI_VALUE_MASK: u32 = 0x01FF_FFFF;
/// `RTC_CALI_TIMEOUT`, bit 0 of `RTCCALICFG2_REG` (RO, always 0 here).
const RTC_CALI_TIMEOUT: u32 = 1;

/// The XTAL frequency this badge is confirmed to run at (40 MHz — see the
/// plan's handoff notes), used as the calibration's reference clock.
const XTAL_HZ: u64 = 40_000_000;
/// `SOC_CLK_RC_SLOW_FREQ_APPROX` (`clk_tree_defs.h`).
const RC_SLOW_HZ: u64 = 136_000;
/// `SOC_CLK_RC_FAST_FREQ_APPROX / 256` (`clk_tree_defs.h`'s
/// `SOC_CLK_RC_FAST_D256_FREQ_APPROX`), integer-divided exactly as the C
/// macro itself is (an `int`/`int` division), not rounded.
const RC_FAST_D256_HZ: u64 = 17_500_000 / 256;
/// `SOC_CLK_XTAL32K_FREQ_APPROX` (`clk_tree_defs.h`).
const XTAL32K_HZ: u64 = 32_768;

/// A single TIMG (timer group) peripheral instance — TIMG0 or TIMG1 (the two
/// are identical register layouts at different bases; `crate::mem::bus`
/// owns one of each as separate fields). See the module doc for exactly
/// what's modeled.
pub struct Timg {
    /// Word-granular storage for every register in `0x00..REGS_END`,
    /// covering the watchdog config/feed/write-protect registers and the
    /// raw (non-side-effecting) bits of `RTCCALICFG_REG`/`RTCCALICFG2_REG`.
    /// `RTCCALICFG_REG`'s `RTC_CALI_RDY` bit and all of `RTCCALICFG1_REG`
    /// are *not* trusted from this array on read — see `read_byte`.
    regs: [u32; REGS_WORDS],
    /// Whether the last calibration trigger (a write setting
    /// `RTC_CALI_START`) has completed and not yet been superseded by a
    /// write clearing it. Synthesizes `RTCCALICFG_REG`'s `RTC_CALI_RDY` bit
    /// on every read — see the module doc's "not stored as a raw bit" note.
    cali_rdy: bool,
    /// The last completed calibration's result, already masked to
    /// `RTC_CALI_VALUE`'s real 25-bit field width. Synthesizes
    /// `RTCCALICFG1_REG` on every read.
    cali_value: u32,
}

impl Default for Timg {
    fn default() -> Self {
        Self {
            regs: [0; REGS_WORDS],
            cali_rdy: false,
            cali_value: 0,
        }
    }
}

impl Timg {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        if word_offset >= REGS_END {
            // Reserved/not-yet-modeled space past the last named register --
            // see the module doc's REGS_END note.
            return 0;
        }
        let widx = (word_offset >> 2) as usize;
        let word = match word_offset {
            RTCCALICFG_REG => {
                // Every other bit (START_CYCLING/CLK_SEL/MAX/START) is plain
                // storage; only RDY is synthesized, never trusted from the
                // stored word -- see the module doc.
                let stored = self.regs[widx] & !RTC_CALI_RDY;
                if self.cali_rdy {
                    stored | RTC_CALI_RDY
                } else {
                    stored
                }
            }
            RTCCALICFG1_REG => {
                // Fully computed -- there is no raw storage behind this
                // register at all (real hardware: entirely RO).
                self.cali_value << RTC_CALI_VALUE_SHIFT
            }
            RTCCALICFG2_REG => {
                // RTC_CALI_TIMEOUT (bit 0) is always 0 in this model; the
                // rest (RST_CNT/THRES) is plain storage.
                self.regs[widx] & !RTC_CALI_TIMEOUT
            }
            _ => self.regs[widx],
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        if word_offset >= REGS_END {
            return;
        }
        let widx = (word_offset >> 2) as usize;
        // Store the raw byte first (even for RTCCALICFG_REG -- its stored
        // RTC_CALI_RDY bit is simply never read back, see `read_byte`), so
        // the calibration side effect below can read MAX/CLK_SEL fields
        // that include *this exact write's* just-applied byte, matching
        // `peripherals::set_byte`'s "operate on the specific trigger byte,
        // not a word reconstructed from stale/fake state" contract.
        set_byte(&mut self.regs[widx], idx, val);

        if word_offset == RTCCALICFG_REG && idx == 3 {
            if val & RTC_CALI_START_BYTE3_BIT != 0 {
                let word = self.regs[widx];
                let clk_sel = (word >> RTC_CALI_CLK_SEL_SHIFT) & RTC_CALI_CLK_SEL_MASK;
                let max = (word >> RTC_CALI_MAX_SHIFT) & RTC_CALI_MAX_MASK;
                let slow_hz = match clk_sel {
                    0 => RC_SLOW_HZ,
                    1 => RC_FAST_D256_HZ,
                    2 => XTAL32K_HZ,
                    // Undocumented encoding (the field is 2 bits wide but
                    // the header only documents 0/1/2) -- fall back to the
                    // same figure as 0 rather than an unreachable!() panic;
                    // see the module doc.
                    _ => RC_SLOW_HZ,
                };
                let value = (max as u64) * XTAL_HZ / slow_hz;
                self.cali_value = (value as u32) & RTC_CALI_VALUE_MASK;
                self.cali_rdy = true;
            } else {
                self.cali_rdy = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(t: &mut Timg, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            t.write_byte(off + i as u32, *b);
        }
    }
    fn r(t: &mut Timg, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| t.read_byte(off + i)))
    }

    fn cfg(clk_sel: u32, max: u32, start: bool) -> u32 {
        (clk_sel << 13) | (max << 16) | if start { 1 << 31 } else { 0 }
    }

    #[test]
    fn calibration_is_not_ready_before_start() {
        let mut t = Timg::new();
        assert_eq!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
    }

    #[test]
    fn start_completes_rc_slow_calibration_with_xtal_ratio() {
        let mut t = Timg::new();
        w(&mut t, RTCCALICFG_REG, cfg(0, 1024, false));
        w(&mut t, RTCCALICFG_REG, cfg(0, 1024, true));
        assert_ne!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
        let value = r(&mut t, RTCCALICFG1_REG) >> 7;
        // NOTE: the brief's literal `1024 * 40_000_000 / 136_000` overflows
        // u32 at the multiplication step (40_960_000_000 > u32::MAX) and is
        // a hard compile-time error in Rust ("this arithmetic operation
        // will overflow"), not just a runtime debug-assertion panic --
        // confirmed by compiling the expression verbatim. Widened to u64
        // here (same expected numeric value, 301176, just computed in a
        // type that can hold the intermediate product) -- see the task
        // report's discrepancies section.
        assert_eq!(value as u64, 1024u64 * 40_000_000 / 136_000);
        assert_eq!(r(&mut t, RTCCALICFG2_REG) & 1, 0, "never times out");
    }

    #[test]
    fn xtal32k_calibration_uses_32768_hz() {
        let mut t = Timg::new();
        w(&mut t, RTCCALICFG_REG, cfg(2, 100, true));
        assert_eq!(r(&mut t, RTCCALICFG1_REG) >> 7, 100 * 40_000_000 / 32_768);
    }

    #[test]
    fn clearing_start_clears_ready_and_writes_cannot_forge_ready() {
        let mut t = Timg::new();
        w(&mut t, RTCCALICFG_REG, cfg(0, 10, true));
        w(&mut t, RTCCALICFG_REG, cfg(0, 10, false));
        assert_eq!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
        w(&mut t, RTCCALICFG_REG, RTC_CALI_RDY);
        assert_eq!(r(&mut t, RTCCALICFG_REG) & RTC_CALI_RDY, 0);
    }

    #[test]
    fn watchdog_registers_are_plain_storage() {
        let mut t = Timg::new();
        w(&mut t, WDTWPROTECT_REG, 0x50D8_3AA1);
        w(&mut t, WDTCONFIG0_REG, 0);
        assert_eq!(r(&mut t, WDTWPROTECT_REG), 0x50D8_3AA1);
        assert_eq!(r(&mut t, WDTCONFIG0_REG), 0);
    }

    #[test]
    fn offsets_at_or_past_regs_end_read_zero_and_drop_writes() {
        let mut t = Timg::new();
        assert_eq!(t.read_byte(REGS_END), 0);
        t.write_byte(REGS_END, 0xff); // must not panic
        assert_eq!(t.read_byte(REGS_END), 0);
        // Comfortably inside the peripheral's 4 KiB page but past the
        // modeled window.
        assert_eq!(t.read_byte(0x900), 0);
    }
}

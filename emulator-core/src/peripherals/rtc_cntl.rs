//! ESP32-C3 RTC_CNTL RTC timer (`DR_REG_RTCCNTL_BASE = 0x6000_8000`).
//!
//! (Plus, since Task D12, the `STORE4` retention register; see "STORE4"
//! below.)
//!
//! Task D1 scope: just the `RTC_CNTL_TIME_UPDATE_REG` /
//! `RTC_CNTL_TIME_LOW0_REG` / `RTC_CNTL_TIME_HIGH0_REG` trio that
//! `rtc_cntl_ll_get_rtc_time()` (and therefore `rtc_time_get()`) reads at
//! boot -- the post-Task-3 stall (hot PCs `0x4038bbbe`-`0x4038bbe4` /
//! `0x4038f8f6`-`0x4038f8fe`, spinning on these three registers reading a
//! constant `0` forever). Every other RTC_CNTL register is out of scope:
//! plain read/write word storage, logged as an "unmapped" (not-yet-modeled)
//! access via [`RtcCntl::handles`] so a later probe can see what firmware
//! touches there, matching the pattern `crate::peripherals::systimer` and
//! `crate::peripherals::intc` already use (see Milestone 3 Task 2's report).
//!
//! Register layout and the driver's exact access sequence confirmed against
//! ESP-IDF v5.5.3, fetched this task from
//! `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`:
//!
//! - `components/soc/esp32c3/register/soc/reg_base.h`:
//!   `DR_REG_RTCCNTL_BASE == 0x6000_8000`.
//! - `components/soc/esp32c3/register/soc/rtc_cntl_reg.h`:
//!   `RTC_CNTL_TIME_UPDATE_REG` `+0x0c` (`RTC_CNTL_TIME_UPDATE`, bit 31,
//!   **WO** -- "Set 1: to update register with RTC timer"; no ready/valid
//!   bit is defined for this register on the ESP32-C3, unlike TIMG's
//!   `RTC_CALI_RDY` -- see the "No polling / no valid bit" section below),
//!   `RTC_CNTL_TIME_LOW0_REG` `+0x10` (`RTC_CNTL_TIMER_VALUE0_LOW`, bits
//!   `[31:0]`, RO), `RTC_CNTL_TIME_HIGH0_REG` `+0x14`
//!   (`RTC_CNTL_TIMER_VALUE0_HIGH`, bits `[15:0]`, RO -- only the low 16
//!   bits of this word are architected; the header's own `_V` mask is
//!   `0x0000FFFF`). The same header also `#define`s `RTC_CNTL_TIME0_REG`/
//!   `RTC_CNTL_TIME1_REG` as aliases for `TIME_LOW0_REG`/`TIME_HIGH0_REG`
//!   (see `components/hal/esp32c3/include/hal/rtc_cntl_ll.h`, next).
//! - `components/hal/esp32c3/include/hal/rtc_cntl_ll.h`'s
//!   `rtc_cntl_ll_get_rtc_time()`: confirms the exact three-access sequence
//!   this module models --
//!   ```c
//!   SET_PERI_REG_MASK(RTC_CNTL_TIME_UPDATE_REG, RTC_CNTL_TIME_UPDATE);
//!   uint64_t t = READ_PERI_REG(RTC_CNTL_TIME0_REG);
//!   t |= ((uint64_t) READ_PERI_REG(RTC_CNTL_TIME1_REG)) << 32;
//!   ```
//!   `SET_PERI_REG_MASK` is itself a read-modify-write
//!   (`WRITE_PERI_REG(addr, READ_PERI_REG(addr) | mask)`), which is exactly
//!   why the observed pre-fix stall's unmapped-access log shows
//!   `0x6000800c` hit as **both** a read *and* a write in equal counts,
//!   while `0x60008010`/`0x60008014` are read-only -- see the module's
//!   `write_byte` doc for how the trigger is detected regardless of that
//!   read-modify-write shape.
//! - `components/esp_hw_support/port/esp32c3/rtc_time.c`'s
//!   `rtc_time_get()`: confirms this is the one and only caller path
//!   (`return rtc_cntl_ll_get_rtc_time();`), and that nothing in this file
//!   polls a ready/valid bit on the RTC_CNTL side -- the firmware-side
//!   *spin* the pre-fix stall showed came from whatever calls `rtc_time_get()`
//!   in a busy-wait loop (comparing the returned value against a target),
//!   not from a hardware handshake bit this peripheral needs to model. See
//!   the task report for why the ~181k `rom_stub_calls` jump (mostly
//!   `ets_delay_us`, per `crate::rom`) is a symptom of that same spin, not a
//!   separate bug.
//! - `components/soc/esp32c3/include/soc/clk_tree_defs.h`: confirms
//!   `SOC_RTC_SLOW_CLK_SRC_RC_SLOW == 0` is the reset-default slow clock
//!   source (`rtc_clk_slow_src_get()`'s default), matching this module's
//!   choice to always derive the counter from `RC_SLOW_HZ` (see below) --
//!   no register in this task's scope lets firmware switch that source, so
//!   there's nothing to model differently yet.
//!
//! ## No polling / no valid bit
//!
//! Unlike TIMG's `RTCCALICFG_REG` (which has a real `RTC_CALI_RDY` bit
//! software polls -- see `crate::peripherals::timg`), `RTC_CNTL_TIME_UPDATE_REG`
//! has **no** documented ready/valid bit on the ESP32-C3 (confirmed by the
//! header above: only `TIME_UPDATE` (WO) plus three unrelated R/W control
//! bits are defined). `rtc_cntl_ll_get_rtc_time()` writes the trigger and
//! reads the latch back unconditionally, synchronously, with nothing to
//! wait on. This module matches that exactly: the latch completes within the
//! same write, no separate "ready" state to synthesize.
//!
//! ## Deriving the counter value
//!
//! Real hardware: this counter free-runs, counting `RTC_SLOW_CLK` cycles
//! continuously in the background, and the `TIME_UPDATE` trigger just
//! snapshots its live value into `TIME_LOW0`/`TIME_HIGH0`. This emulator has
//! no free-running background clock -- the only notion of "elapsed time" it
//! has is the CPU's own instruction count, which `crate::peripherals::systimer::SysTimer`
//! already tracks as its live `unit0_counter` (advanced by exactly
//! [`crate::peripherals::systimer::TICKS_PER_STEP`] -- documented there as 1
//! tick per `Cpu::step()`, a placeholder since `Cpu::step()` has no
//! cycle-accurate timing model). Rather than add a second, redundant
//! step-counting field here, [`RtcCntl::write_byte`] takes that live
//! SYSTIMER counter value as a parameter at the moment of the trigger --
//! `crate::mem::bus::FirmwareBus` passes `self.systimer.counter()` directly,
//! the same "concrete field, no trait object" cross-peripheral read pattern
//! already used for INTC's `CPU_INT_EIP_STATUS_REG` and SPI2's D/C-line
//! read (see `mem::bus`'s module doc).
//!
//! That step count is then converted to RTC slow-clock cycles by treating
//! one `Cpu::step()` as approximately one `XTAL_HZ` cycle (the same
//! placeholder-but-documented spirit as SYSTIMER's own 1-tick-per-step
//! choice: this emulator has no real cycle-per-instruction model to derive
//! a better ratio from) and scaling by `RC_SLOW_HZ / XTAL_HZ`:
//! ```text
//! rtc_ticks = elapsed_steps * RC_SLOW_HZ / XTAL_HZ
//! ```
//! `RC_SLOW_HZ`/`XTAL_HZ` are reused from `crate::peripherals::timg` (made
//! `pub(crate)` there for exactly this reuse) rather than duplicated, per
//! the task brief. The result is masked to the register pair's real 48-bit
//! width (`TIME_LOW0` 32 bits + `TIME_HIGH0`'s 16 architected bits) so it
//! wraps like real hardware would rather than growing unbounded.
//!
//! This is monotonically non-decreasing in `elapsed_steps` (multiplying and
//! integer-dividing by fixed positive constants preserves ordering), so two
//! latches taken with enough real steps between them are guaranteed to
//! differ once enough steps have passed to move the quotient by at least 1
//! (`XTAL_HZ / RC_SLOW_HZ` ~= 294 steps) -- see the module's tests.
//!
//! ## STORE4 (`RTC_XTAL_FREQ_REG`, Milestone 3 Task D12)
//!
//! `RTC_CNTL_STORE4_REG` (`+0xb8`, `soc/rtc_cntl_reg.h`) is a plain R/W
//! retention register with reset value 0. ESP-IDF names it
//! `RTC_XTAL_FREQ_REG` (`components/esp_rom/esp32c3/include/esp32c3/rom/rtc.h`).
//! On real hardware the 2nd-stage bootloader stores the XTAL frequency in it:
//! `bootloader_init()` -> `bootloader_clock_configure()`
//! (`components/bootloader_support/src/bootloader_clock_init.c`) ->
//! `rtc_clk_init()` (`components/esp_hw_support/port/esp32c3/rtc_clk_init.c`)
//! -> `rtc_clk_xtal_freq_update()` (`rtc_clk.c`) ->
//! `clk_ll_xtal_store_freq_mhz()` (`hal/esp32c3/include/hal/clk_tree_ll.h`).
//! The app reads it back via `clk_ll_xtal_load_freq_mhz()`, which rejects 0.
//! The shortcut boot skips the bootloader, so `crate::boot` seeds the value
//! the bootloader would have stored (`crate::rom::RTC_XTAL_FREQ_REG`) by an
//! ordinary bus write. This module needs no special behavior for it: the
//! generic word storage below holds it. It is listed in [`RtcCntl::handles`]
//! because it is now a deliberately modeled register, so its accesses are
//! not logged as unmapped.

use super::set_byte;
use super::timg::{RC_SLOW_HZ, XTAL_HZ};

/// End of the modeled register window (word-array storage): every RTC_CNTL
/// register named in `rtc_cntl_reg.h` up to its highest cited offset
/// (`0x1FC`) lives inside this range. Offsets at or past this read 0 / drop,
/// same simplification `crate::peripherals::timg` uses.
const REGS_END: u32 = 0x200;
const REGS_WORDS: usize = (REGS_END / 4) as usize;

pub const TIME_UPDATE_REG: u32 = 0x0c;
pub const TIME_LOW0_REG: u32 = 0x10;
pub const TIME_HIGH0_REG: u32 = 0x14;
/// `RTC_CNTL_STORE4_REG` (`+0xb8`, `soc/rtc_cntl_reg.h`): a plain R/W
/// retention register, which ESP-IDF names `RTC_XTAL_FREQ_REG`
/// (`components/esp_rom/esp32c3/include/esp32c3/rom/rtc.h`). Its reset value
/// is 0; the 2nd-stage bootloader stores the XTAL frequency in it, which the
/// shortcut boot seeds instead (`crate::rom::RTC_XTAL_FREQ_REG`). See the
/// module doc's "STORE4" section.
pub const STORE4_REG: u32 = 0xb8;

/// `RTC_CNTL_TIME_UPDATE`, bit 31 of `TIME_UPDATE_REG` -- byte index 3, bit
/// 7 of that byte (`31 - 24 == 7`).
const TIME_UPDATE_BYTE3_BIT: u8 = 0x80;
/// `RTC_CNTL_TIMER_VALUE0_HIGH`'s real field width: only the low 16 bits of
/// `TIME_HIGH0_REG` are architected (`RTC_CNTL_TIMER_VALUE0_HIGH_V ==
/// 0x0000FFFF`).
const HIGH0_MASK: u64 = 0xFFFF;
/// The full latched counter's real width: 32 (`TIME_LOW0`) + 16
/// (`TIME_HIGH0`) = 48 bits.
const COUNTER_MASK_48: u64 = (1u64 << 48) - 1;

/// The RTC_CNTL peripheral: just enough of it for the RTC timer latch (see
/// the module doc). Every other register is generic word storage, logged as
/// "unmapped" via [`RtcCntl::handles`] when accessed -- see
/// `crate::mem::bus::FirmwareBus`.
pub struct RtcCntl {
    /// Word-granular storage for every register in `0x00..REGS_END`,
    /// including `TIME_UPDATE_REG`'s non-trigger bits (`TIMER_SYS_RST`/
    /// `TIMER_XTL_OFF`/`TIMER_SYS_STALL`) and every not-yet-modeled
    /// register. `TIME_LOW0_REG`/`TIME_HIGH0_REG` are *not* read from this
    /// array -- see `read_byte`.
    regs: [u32; REGS_WORDS],
    /// The last-latched 48-bit RTC timer snapshot, already masked to
    /// [`COUNTER_MASK_48`]. Synthesizes `TIME_LOW0_REG`/`TIME_HIGH0_REG` on
    /// every read. Starts at 0 (no trigger has fired yet), matching real
    /// hardware's reset default for both registers.
    latched: u64,
}

impl Default for RtcCntl {
    fn default() -> Self {
        Self {
            regs: [0; REGS_WORDS],
            latched: 0,
        }
    }
}

impl RtcCntl {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset`'s word-aligned offset is one of the three
    /// registers this module gives real latch behavior to, or `STORE4_REG`
    /// (seeded at boot, Task D12; see the module doc). Used by
    /// `crate::mem::bus::FirmwareBus` to log an access to any other
    /// RTC_CNTL register as "unmapped" (even though it's still backed by
    /// real storage below, not a hard failure) -- see the module doc and
    /// `crate::peripherals::systimer::SysTimer::handles`, the pattern this
    /// mirrors.
    pub fn handles(offset: u32) -> bool {
        matches!(
            offset & !0b11,
            TIME_UPDATE_REG | TIME_LOW0_REG | TIME_HIGH0_REG | STORE4_REG
        )
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        if word_offset >= REGS_END {
            return 0;
        }
        let widx = (word_offset >> 2) as usize;
        let word = match word_offset {
            TIME_LOW0_REG => (self.latched & 0xFFFF_FFFF) as u32,
            TIME_HIGH0_REG => ((self.latched >> 32) & HIGH0_MASK) as u32,
            _ => self.regs[widx],
        };
        word.to_le_bytes()[idx]
    }

    /// `elapsed_steps` is the live SYSTIMER unit0 counter value at the
    /// moment of this specific byte write -- see the module doc's "Deriving
    /// the counter value" section. Only consulted when this write is the
    /// one that completes the `TIME_UPDATE` trigger; ignored otherwise (so
    /// passing a stale/arbitrary value on non-triggering writes is
    /// harmless).
    pub fn write_byte(&mut self, offset: u32, val: u8, elapsed_steps: u64) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        if word_offset >= REGS_END {
            return;
        }
        let widx = (word_offset >> 2) as usize;
        match word_offset {
            // RO on real hardware -- see the module doc. Silently dropped,
            // not even stored into `regs` (nothing ever reads `regs[widx]`
            // for these two words, since `read_byte` always overrides them
            // with the computed `latched` value, but not storing keeps the
            // "RO register accepts and drops writes" contract explicit and
            // matches `crate::peripherals::timg::RTCCALICFG1_REG`'s
            // fully-computed treatment).
            TIME_LOW0_REG | TIME_HIGH0_REG => {}
            TIME_UPDATE_REG => {
                // Store the raw byte first (the non-trigger control bits are
                // plain read/write storage), then check whether *this exact
                // byte* is the one containing the trigger bit -- inspecting
                // only byte index 3, not a word reconstructed from other,
                // possibly differently-timed writes. This is safe regardless
                // of `SET_PERI_REG_MASK`'s read-modify-write shape (a read
                // then a single 4-byte write, i.e. bytes 0..=3 delivered
                // ascending in one `sw`) -- matches
                // `peripherals::set_byte`'s documented contract and
                // `crate::peripherals::systimer::SysTimer::write_byte`'s
                // `UNIT0_OP_REG` handling of the same idiom.
                set_byte(&mut self.regs[widx], idx, val);
                if idx == 3 && val & TIME_UPDATE_BYTE3_BIT != 0 {
                    let raw = (elapsed_steps as u128) * (RC_SLOW_HZ as u128) / (XTAL_HZ as u128);
                    self.latched = (raw as u64) & COUNTER_MASK_48;
                }
            }
            _ => set_byte(&mut self.regs[widx], idx, val),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `v` to `off` as 4 ascending byte writes (matching how
    /// `crate::mem::bus::FirmwareBus` dispatches a real `sw`), passing the
    /// same `elapsed_steps` for all four -- realistic, since the live
    /// SYSTIMER counter doesn't move *during* a single instruction's
    /// dispatch.
    fn w(t: &mut RtcCntl, off: u32, v: u32, elapsed_steps: u64) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            t.write_byte(off + i as u32, *b, elapsed_steps);
        }
    }
    fn r(t: &mut RtcCntl, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| t.read_byte(off + i)))
    }
    fn latched_value(t: &mut RtcCntl) -> u64 {
        (r(t, TIME_LOW0_REG) as u64) | ((r(t, TIME_HIGH0_REG) as u64) << 32)
    }

    #[test]
    fn before_any_trigger_low0_high0_read_zero() {
        let mut t = RtcCntl::new();
        assert_eq!(r(&mut t, TIME_LOW0_REG), 0);
        assert_eq!(r(&mut t, TIME_HIGH0_REG), 0);
    }

    #[test]
    fn trigger_latches_elapsed_time_scaled_to_the_rtc_slow_clock() {
        let mut t = RtcCntl::new();
        let elapsed_steps: u64 = 1_000_000;
        w(&mut t, TIME_UPDATE_REG, 1 << 31, elapsed_steps);

        let expected = (elapsed_steps as u128 * RC_SLOW_HZ as u128 / XTAL_HZ as u128) as u64;
        assert_ne!(
            expected, 0,
            "test's own steps budget must produce a real tick count"
        );
        assert_eq!(latched_value(&mut t), expected);
    }

    #[test]
    fn only_the_byte_containing_the_trigger_bit_fires_the_latch() {
        let mut t = RtcCntl::new();
        // Write bytes 0..=2 of TIME_UPDATE_REG (all zero, no trigger bit
        // anywhere in them) with a large elapsed_steps -- must NOT latch.
        t.write_byte(TIME_UPDATE_REG, 0x00, 999_999);
        t.write_byte(TIME_UPDATE_REG + 1, 0x00, 999_999);
        t.write_byte(TIME_UPDATE_REG + 2, 0x00, 999_999);
        assert_eq!(
            latched_value(&mut t),
            0,
            "must not latch before byte 3 arrives"
        );

        // Now byte 3 arrives with the trigger bit set: exactly one latch.
        t.write_byte(TIME_UPDATE_REG + 3, 0x80, 999_999);
        let expected = (999_999u128 * RC_SLOW_HZ as u128 / XTAL_HZ as u128) as u64;
        assert_eq!(latched_value(&mut t), expected);
    }

    #[test]
    fn two_successive_latches_with_time_advancing_yield_increasing_values() {
        let mut t = RtcCntl::new();
        w(&mut t, TIME_UPDATE_REG, 1 << 31, 10_000);
        let first = latched_value(&mut t);

        // Comfortably more than XTAL_HZ/RC_SLOW_HZ (~294) steps later, so
        // the integer-divided quotient is guaranteed to have moved.
        w(&mut t, TIME_UPDATE_REG, 1 << 31, 10_000 + 1_000_000);
        let second = latched_value(&mut t);

        assert!(
            second > first,
            "expected the second latch ({second}) to exceed the first ({first}) \
             after 1,000,000 more elapsed steps"
        );
    }

    #[test]
    fn read_modify_write_shape_of_set_peri_reg_mask_does_not_double_latch_or_corrupt_control_bits()
    {
        // rtc_cntl_ll_get_rtc_time() uses SET_PERI_REG_MASK, a
        // read-then-write-back-the-whole-word -- confirm that shape (a full
        // 4-byte write with TIMER_SYS_RST/XTL_OFF/SYS_STALL bits already
        // set from a prior write, ORed with TIME_UPDATE) still latches
        // exactly once and preserves those other bits.
        let mut t = RtcCntl::new();
        // Pretend a prior write set TIMER_SYS_RST (bit 29) without
        // triggering (bit 31 clear).
        w(&mut t, TIME_UPDATE_REG, 1 << 29, 1);
        assert_eq!(latched_value(&mut t), 0, "no trigger yet");

        // The read-modify-write: same word, OR in TIME_UPDATE (bit 31).
        w(&mut t, TIME_UPDATE_REG, (1 << 29) | (1 << 31), 500_000);
        let expected = (500_000u128 * RC_SLOW_HZ as u128 / XTAL_HZ as u128) as u64;
        assert_eq!(latched_value(&mut t), expected);
        assert_ne!(
            r(&mut t, TIME_UPDATE_REG) & (1 << 29),
            0,
            "TIMER_SYS_RST must survive the trigger write"
        );
    }

    #[test]
    fn unnamed_offsets_are_plain_storage() {
        let mut t = RtcCntl::new();
        // RTC_CNTL_OPTIONS0_REG (+0x00): not one of this module's named
        // registers, so it must fall back to generic read/write storage.
        w(&mut t, 0x00, 0x1234_5678, 0);
        assert_eq!(r(&mut t, 0x00), 0x1234_5678);
    }

    #[test]
    fn offsets_at_or_past_regs_end_read_zero_and_drop_writes() {
        let mut t = RtcCntl::new();
        assert_eq!(t.read_byte(REGS_END), 0);
        t.write_byte(REGS_END, 0xff, 0); // must not panic
        assert_eq!(t.read_byte(REGS_END), 0);
    }

    #[test]
    fn handles_only_reports_the_named_registers() {
        assert!(RtcCntl::handles(TIME_UPDATE_REG));
        assert!(RtcCntl::handles(TIME_UPDATE_REG + 2)); // any byte within the word
        assert!(RtcCntl::handles(TIME_LOW0_REG));
        assert!(RtcCntl::handles(TIME_HIGH0_REG));
        assert!(RtcCntl::handles(STORE4_REG)); // RTC_XTAL_FREQ_REG (Task D12)
        assert!(!RtcCntl::handles(0x00)); // OPTIONS0_REG
        assert!(!RtcCntl::handles(0x18)); // STATE0_REG
        assert!(!RtcCntl::handles(0xbc)); // STORE5_REG (RTC_APB_FREQ_REG): not seeded
    }

    #[test]
    fn store4_is_plain_read_write_storage_with_a_zero_reset_value() {
        let mut t = RtcCntl::new();
        assert_eq!(r(&mut t, STORE4_REG), 0);
        w(&mut t, STORE4_REG, 0x0028_0028, 0);
        assert_eq!(r(&mut t, STORE4_REG), 0x0028_0028);
    }
}

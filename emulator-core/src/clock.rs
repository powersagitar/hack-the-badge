//! Emulated time: converts executed instruction steps into ticks of a
//! fixed-rate timer clock, at a CPU frequency that can change while the
//! guest runs (Milestone 6).
//!
//! Chip-agnostic, like `crate::cpu`: it knows ratios only. Chip setup
//! supplies the numbers (for the ESP32-C3, `crate::mem::soc::SYSTIMER_HZ`
//! and the frequency the SYSTEM clock registers select; see
//! `crate::mem::bus::FirmwareBus::tick_peripherals`).
//!
//! Each step is `cycles_per_step` CPU cycles. A cycle is
//! `timer_hz / cpu_hz` timer ticks, so the clock keeps the sub-tick
//! remainder in units of `1 / cpu_hz` of a tick (`cycles * timer_hz`) and
//! carries it, which makes the conversion exact over any run. When the CPU
//! frequency changes, the pending fraction of a tick is rescaled to the new
//! unit, so no time is lost or invented at a switch.
//!
//! Idle time (a WFI fast-forward) is charged in whole ticks and also
//! counted separately ([`Clock::idle_ticks`]).

/// See the module doc.
#[derive(Clone, Debug)]
pub struct Clock {
    cpu_hz: u64,
    timer_hz: u64,
    cycles_per_step: u64,
    /// Pending fraction of a tick, in units of `1 / cpu_hz` tick.
    remainder: u64,
    total_ticks: u64,
    idle_ticks: u64,
}

impl Clock {
    /// A clock at `cpu_hz` against a `timer_hz` timer. Panics on a zero
    /// argument (chip setup passes constants).
    pub fn new(cpu_hz: u64, timer_hz: u64, cycles_per_step: u64) -> Self {
        assert!(cpu_hz > 0 && timer_hz > 0 && cycles_per_step > 0);
        Self {
            cpu_hz,
            timer_hz,
            cycles_per_step,
            remainder: 0,
            total_ticks: 0,
            idle_ticks: 0,
        }
    }

    pub fn cpu_hz(&self) -> u64 {
        self.cpu_hz
    }

    pub fn timer_hz(&self) -> u64 {
        self.timer_hz
    }

    /// Switches the CPU frequency, keeping the pending fraction of a tick.
    /// `0` is ignored.
    pub fn set_cpu_hz(&mut self, hz: u64) {
        if hz == 0 || hz == self.cpu_hz {
            return;
        }
        self.remainder =
            (u128::from(self.remainder) * u128::from(hz) / u128::from(self.cpu_hz)) as u64;
        self.cpu_hz = hz;
    }

    /// One executed step: returns the whole timer ticks it completed.
    pub fn step(&mut self) -> u64 {
        self.remainder += self.cycles_per_step * self.timer_hz;
        let ticks = self.remainder / self.cpu_hz;
        self.remainder %= self.cpu_hz;
        self.total_ticks += ticks;
        ticks
    }

    /// Idle time skipped by a fast-forward, in whole ticks.
    pub fn advance_idle(&mut self, ticks: u64) {
        self.total_ticks += ticks;
        self.idle_ticks += ticks;
    }

    /// Timer ticks since construction.
    pub fn total_ticks(&self) -> u64 {
        self.total_ticks
    }

    /// The part of [`Clock::total_ticks`] charged by [`Clock::advance_idle`].
    pub fn idle_ticks(&self) -> u64 {
        self.idle_ticks
    }

    /// Emulated time in whole microseconds, saturating at `u64::MAX`.
    pub fn elapsed_us(&self) -> u64 {
        u64::try_from(u128::from(self.total_ticks) * 1_000_000 / u128::from(self.timer_hz))
            .unwrap_or(u64::MAX)
    }

    /// The first tick count whose time is at least `us` microseconds,
    /// saturating at `u64::MAX` (a target that far off is never reached).
    pub fn ticks_at_us(&self, us: u64) -> u64 {
        u64::try_from((u128::from(us) * u128::from(self.timer_hz)).div_ceil(1_000_000))
            .unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMER: u64 = 16_000_000;

    #[test]
    fn legacy_ratio_is_one_tick_per_step() {
        let mut c = Clock::new(TIMER, TIMER, 1);
        for _ in 0..1000 {
            assert_eq!(c.step(), 1);
        }
        assert_eq!(c.total_ticks(), 1000);
    }

    #[test]
    fn eighty_mhz_ticks_once_every_five_steps() {
        let mut c = Clock::new(80_000_000, TIMER, 1);
        let ticks: Vec<u64> = (0..10).map(|_| c.step()).collect();
        assert_eq!(ticks, [0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
        assert_eq!(c.total_ticks(), 2);
    }

    #[test]
    fn forty_mhz_ticks_two_every_five_steps() {
        let mut c = Clock::new(40_000_000, TIMER, 1);
        for _ in 0..5 {
            c.step();
        }
        assert_eq!(c.total_ticks(), 2);
        for _ in 0..995 {
            c.step();
        }
        assert_eq!(c.total_ticks(), 400, "no drift over 1000 steps");
    }

    #[test]
    fn frequency_change_keeps_the_fractional_tick() {
        // Three steps at 80 MHz = 3/5 tick pending. A step at 40 MHz is
        // 2/5 tick, so the first one completes the tick (3/5 + 2/5), and
        // the next tick takes the usual 5/2 steps (2/5, 4/5, 6/5).
        // Dropping the remainder would give [0, 0, 1, 0]; keeping it
        // without rescaling (48M read as 1/40M units, 6/5 tick) would give
        // [1, 1, ...].
        let mut c = Clock::new(80_000_000, TIMER, 1);
        for _ in 0..3 {
            assert_eq!(c.step(), 0);
        }
        c.set_cpu_hz(40_000_000);
        let ticks: Vec<u64> = (0..4).map(|_| c.step()).collect();
        assert_eq!(ticks, [1, 0, 0, 1], "3/5 + 2/5 = 1 tick on the next step");
        assert_eq!(c.total_ticks(), 2);
        assert_eq!(c.cpu_hz(), 40_000_000);
    }

    #[test]
    fn zero_frequency_is_ignored() {
        let mut c = Clock::new(80_000_000, TIMER, 1);
        c.set_cpu_hz(0);
        assert_eq!(c.cpu_hz(), 80_000_000);
    }

    #[test]
    fn idle_ticks_count_toward_time_but_not_steps() {
        let mut c = Clock::new(80_000_000, TIMER, 1);
        c.advance_idle(160_000);
        assert_eq!(c.total_ticks(), 160_000);
        assert_eq!(c.idle_ticks(), 160_000);
        assert_eq!(c.elapsed_us(), 10_000);
    }

    #[test]
    fn ticks_at_us_rounds_up_to_a_whole_tick() {
        let c = Clock::new(80_000_000, TIMER, 1);
        assert_eq!(c.ticks_at_us(0), 0);
        assert_eq!(c.ticks_at_us(1), 16);
        assert_eq!(c.ticks_at_us(100_000), 1_600_000);
        // Saturates instead of wrapping (2^62 us is 2^66 ticks).
        assert_eq!(c.ticks_at_us(1 << 62), u64::MAX);
        assert_eq!(c.ticks_at_us(u64::MAX), u64::MAX);
    }

    #[test]
    fn elapsed_us_saturates_with_a_slow_timer() {
        let mut c = Clock::new(1, 1, 1);
        c.advance_idle(u64::MAX);
        assert_eq!(c.elapsed_us(), u64::MAX);
    }

    #[test]
    fn cycles_per_step_scales_the_rate() {
        let mut c = Clock::new(80_000_000, TIMER, 5);
        assert_eq!(c.step(), 1, "five cycles at 80 MHz = one 16 MHz tick");
    }
}

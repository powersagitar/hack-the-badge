# Milestone 6: design decisions, measurements, and the Milestone 7 backlog

Milestone 6 gives the real-firmware emulator a calibrated clock (emulated
time follows the firmware's own 40/80 MHz CPU frequency switches) and
makes Snake playable. The design spec is
`docs/superpowers/specs/2026-10-09-milestone6-playable-snake-design.md`
and the plan is
`docs/superpowers/plans/2026-10-09-milestone6-playable-snake.md`;
`docs/firmware-emulator-notes.md` holds the forensic findings and the
stall-by-stall history. This file records the rulings, the measurements
and what is deferred. Where the spec or plan disagree with this file or
the code, this file and the code win.

## Rulings

Every ruling of the milestone, with its reason and what it would cost if
wrong. R-PF rulings came from a preflight scan of the plan before Task 1;
R-T*n*-*k* rulings from Task *n* (D-M6-1's are under Task 3, which it
followed).

- **R-PF-1, Task 1's emulated-time test follows the clock switch.**
  Task 1's `emulated_time_starts_at_zero_and_follows_the_clock` pinned
  the legacy ratio (1,600 steps = 100 µs). Task 3 makes the clock follow
  the SYSTEM registers, so the plan's Task 3 had to update that test too,
  although its text did not name it: a legacy-ratio assertion is plainly
  wrong after the switch. The value it was first expected to take (40 µs
  at 40 MHz) was superseded by R-T2-5 and R-T3-1 (20 µs at 80 MHz). If
  wrong: one test line.
- **R-PF-2, the clock-init progress check.** `rom_stub_boot.rs`'s
  `rom_stubbed_boot_gets_past_the_mask_rom_wall` checked that boot hit
  the `ets_get_cpu_frequency` *stub*. Since Task 2 that address is guest
  ROM code, so no stub fires there; the check is kept, with the same
  meaning ("ets_get_cpu_frequency (clock init)"), as "a step executed at
  `ETS_GET_CPU_FREQUENCY`" (`StepInfo::pc_before`). The spec deletes the
  stub, not the progress check. If wrong: boot could stop reaching clock
  init without that test noticing.
- **R-T1-1, the fractional-tick test.** The plan's
  `frequency_change_keeps_the_fractional_tick` expected the two steps
  after an 80 → 40 MHz switch (3/5 tick pending) to tick `[0, 1]`. That
  is arithmetically wrong: a 40 MHz step is 16/40 = 2/5 tick, so
  3/5 + 2/5 completes the tick on the first step, and the plan's own
  `Clock` returns 1 there. The test now asserts the next four steps tick
  `[1, 0, 0, 1]` with `total_ticks() == 2`. That catches both failure
  modes: a dropped remainder gives `[0, 0, 1, 0]`, and a remainder kept
  without rescaling gives `[1, 1, ...]`. If wrong: a frequency switch
  could lose or invent time without a test noticing.
- **R-T1-2, `SYSTIMER_HZ`'s citation.** The plan cited
  `SYSTIMER_LL_TICKS_PER_US` from `systimer_ll.h`, which ESP-IDF v5.5.3
  does not have. `SYSTIMER_HZ` (16 MHz) cites what does exist: the
  ESP32-C3 `systimer.c` (ticks / 16, "40MHz/2.5 = 16MHz") and
  `soc_caps.h`'s `SOC_SYSTIMER_FIXED_DIVIDER`. A citation never names a
  symbol that is not in the source. If wrong: a doc line.
- **R-T1-3, the WFI rung's metric.**
  `boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting`
  now measures `clock.idle_ticks()` (1,827,072 at step 5,550,000) instead
  of SYSTIMER's lead over the step count (1,827,060). The two differ by
  the 12 fast-forward steps: their whole jump is now charged as idle time,
  not one tick of it as a step. The `>= 1,500,000` threshold is
  unchanged. If wrong: the rung would no longer prove that the
  fast-forward is what put time ahead of the steps.
- **R-T1-4, `CLAUDE.md`'s clock entry waits for Task 9.** The plan puts
  `CLAUDE.md` in Task 9, so Task 1 did not add a `clock.rs` entry there.
  If wrong: `CLAUDE.md` was stale for a few commits (Tasks 2, 3 and
  D-M6-1 updated some entries and figures in passing; Task 9 finished
  it).
- **R-T1-5, time conversions saturate.** `Clock::ticks_at_us` and
  `Clock::elapsed_us` compute in `u128` and saturate to `u64::MAX`
  rather than truncating with `as`. Above about 1.15e18 µs the tick count
  exceeds `u64` at 16 MHz, and the browser can reach that through
  `runUntil`'s `f64`. A truncated target wrapped to a small value
  (2^62 µs became 0 ticks, so `run_until` ran nothing), and a
  chip-agnostic `timer_hz` below 1 MHz could do the same to `elapsed_us`.
  A saturated target is never reached, so `run_until` runs to its step
  cap. If wrong: a far-future target from the browser would silently run
  zero steps.
- **R-T1-6, design-changing rulings go in this file with their task.**
  R-T1-1 and R-T1-3 went into this list in Task 1's fix commit, as the
  plan asks of every ruling that changes the design; Task 9 added the
  rulings that had been kept only in the task records. If wrong: a doc
  paragraph.
- **R-T2-1, early log timestamps are timing.** The pre-scheduler log
  timestamp is the cycle counter divided by `ets_get_cpu_frequency() *
  1000`, and the app's `rtc_clk_init()` rescales the cycle counter by
  the new over the old frequency, so the prefixes move. With the R-T2-4
  seeds the early lines read `W (10)` for `rtc_init`, then `I (5)` from
  `cpu_start: Unicore app` (was `I (2)`) through `sleep_gpio: Configure
  ...`, then `I (6)` from `sleep_gpio: Enable ...` through `main_task:
  Calling app_main()` (was `I (5)`); `LVGL: Starting LVGL task` reads
  126 (was 125). (Task 2's first commit, with the XTAL seeds, read
  `I (10)`/`I (11)` and 131.) The rung needles are re-pinned to the new
  prefixes exactly, not loosened. With timestamps stripped, the
  console through step 6,000,000 is identical to `d5e6be3`'s except
  `cpu freq: 80000000 Hz` (was 160000000). If wrong: a needle could
  pass on a changed line.
- **R-T2-2, step docs that moved under passing rungs.** Two rungs whose
  windows still contain the event they describe kept their bounds but
  their docs now quote the new exact step (`spi_hal_init`'s poll,
  `__bswapsi2`'s call); the D10 rung, documented as "the step before
  FreeRTOS's first yield request", is re-pinned to that step (528,150 →
  528,090) so its meaning holds. Historical fault steps quoted as "past
  the old fault on step N" (memcpy 401,761, gpio_matrix_out 571,712,
  newlib 417,992, memchr 490,128) are facts about old builds and are not
  moved. If wrong: a doc would point at a step where nothing happens.
- **R-T2-3, notes lists are history.** Task 2's brief asked to remove
  SYSTEM `0x600c_0058`/`+0x08` from the notes' "unmapped accesses at the
  idle point" lists. The list in "Milestone 4's end state" now says they
  were there until Task 2 modeled them; the two per-task history entries
  (Milestone 4 Task 5 and D-M4-1), which record what that task observed,
  are annotated rather than edited. If wrong: a reader of the history
  could think SYSTEM's clock registers are still unmapped (the
  annotations say otherwise).
- **R-T2-4, the CPU clock at app entry is the bootloader's 80 MHz PLL.**
  (Controller ruling, Task 2 review fix round 1.) The spec and Task 2's
  brief seeded `SYSTEM_SYSCLK_CONF_REG` = 0 (XTAL / 1, 40 MHz) and
  `s_ticks_per_us` = 40 "as the skipped ROM and bootloader leave them".
  The 40 is the mask ROM's `.data` initial value, but the 2nd-stage
  bootloader changes both before the app starts:
  `bootloader_clock_configure()`
  (`components/bootloader_support/src/bootloader_clock_init.c:40,58,83`,
  ESP-IDF v5.5.3) calls `rtc_clk_init()` with `cpu_freq_mhz =
  CPU_CLK_FREQ_MHZ_BTLD` = 80
  (`components/soc/esp32c3/include/soc/soc.h:139`), and
  `rtc_clk_cpu_freq_to_pll_mhz()`
  (`components/esp_hw_support/port/esp32c3/rtc_clk.c:186-192`) sets
  `CPUPERIOD_SEL` 0, `PRE_DIV_CNT` 0, `SOC_CLK_SEL` 1 (PLL) and calls
  `esp_rom_set_cpu_ticks_per_us(80)`. So the seeds are now
  `SYSCLK_CONF` = `1 << 10` and `s_ticks_per_us` = 80
  (`rom::ROM_TICKS_PER_US_AT_APP_ENTRY`, which replaces
  `ROM_TICKS_PER_US_INIT`); `CPU_PER_CONF`'s reset value `0x0c` already
  has `CPUPERIOD_SEL` 0, so it is not seeded. This overrides the spec's
  premise ("XTAL / 1 as the skipped ROM and bootloader leave it") while
  keeping its intent: seed exactly the clock state the skipped boot
  stages leave for the app. It matters beyond the seed itself, because
  the app's `rtc_clk_init()` (`rtc_clk_init.c:56-66`) reads these
  registers as `freq_before` and rescales the cycle counter by it. If
  wrong: the app's first clock switch and the cycle counter would start
  from a frequency the real badge never had.
- **R-T2-5, Task 3's synthetic-image numbers follow the 80 MHz seed.**
  (Amends R-PF-1.) With R-T2-4's seed, the plan's Task 3 numbers that
  assumed a synthetic image at 40 MHz are recomputed at 80 MHz: 1,000
  steps of `run_returns_after_budget_even_if_wfi_never_wakes` are 200
  ticks, and 1,600 steps of `emulated_time_starts_at_zero_and_follows_the_clock`
  are 320 ticks, 20 µs. The bare-bus tests (`xtal_bus()`,
  `a_bare_bus_follows_the_reset_clock_registers`) apply no initializers
  and stand as written. Task 3 measured them (R-T3-1). If wrong: test
  constants.
- **R-T3-1, the synthetic-image runtime tests run at 80 MHz.**
  `FirmwareRuntime::from_image` boots through
  `boot_from_factory_image_with_rom_stubs`, which applies the boot
  initializers, so the synthetic images get the R-T2-4 PLL seed: 1,000
  steps of `run_returns_after_budget_even_if_wfi_never_wakes` are 200
  ticks (1000 × 16 / 80; the brief's 400 assumed XTAL), and Task 1's
  `emulated_time_starts_at_zero_and_follows_the_clock` reads 20 µs after
  1,600 steps (320 ticks; 100 µs under the old clock). Both were measured,
  not only computed. A bare `FirmwareBus::from_segments` bus has no
  initializers: it is constructed at the 40 MHz XTAL clock and its first
  step applies `SYSCLK_CONF`'s reset value, `PRE_DIV_CNT` = 1, i.e. XTAL / 2
  = 20 MHz (`a_bare_bus_follows_the_reset_clock_registers`). If wrong: a
  runtime test would pin a rate the boot never runs at.
- **R-T3-2, the zero-tick test's pattern.** The brief expected the five
  steps after the PLL switch in `zero_tick_steps_leave_the_systimer_untouched`
  to tick `0, 0, 0, 0, 1`. `xtal_bus()`'s own 40 MHz step leaves 2/5 of a
  tick pending, the switch keeps it (rescaled, R-T1-1) and the sync step
  adds 1/5, so the second step completes a tick: the test asserts per-step
  ticks `[0, 1, 0, 0, 0]`, which still checks that four of five steps
  leave SYSTIMER untouched. If wrong: a dropped or doubled remainder at the
  switch would go unnoticed.
- **R-T3-3, the invalid-encoding bus test.** The brief's
  `invalid_clock_encoding_keeps_the_clock_rate` only checked `SOC_CLK_SEL`
  = 3 from 40 MHz, the constructor's own rate, so a bus that fell back to
  its default would pass. The test also switches to PLL 160 MHz (a rate
  only the registers give) and checks that `SOC_CLK_SEL` = 3 and then
  `CPUPERIOD_SEL` = 3 both keep 160 MHz. If wrong: an invalid write could
  reset the clock without a test noticing.
- **R-T3-4, the early-boot shift and the historical fault-step rungs.**
  The clock made early boot about 600,000 steps longer: between the app's
  first `rtc_clk_init()` switch to the 40 MHz XTAL clock (step 836) and
  its switch back to the PLL, `rtc_init()` waits on the RTC timer
  (the loops at `0x4038_bbbe` and `0x4038_f8f6`, the pair Milestone 3
  Task D1 unblocked), and RTC time follows SYSTIMER ticks, which now
  come 2.5 steps apart instead of 1. The first log line moves from about
  step 401,000 to 1,001,000; after it the shift is a near constant
  599,794 steps (to `heap_init`), 599,842 (the `memchr` call), 599,938
  (the first yield) and 600,230 (from `app_main`'s SPI setup to the idle
  `wfi`). Rungs that run to a pre-Milestone-6 fault step (`memcpy`
  401,761, `ets_efuse_get_spiconfig` 402,113, `esprv_intc_int_enable`
  405,806, `itoa` 407,471) now run to that step plus the measured shift
  there, so they cover the same instructions as at `00dd7d5`; windows
  given in round numbers (440,000, 493,000, 583,000..596,000,
  590,000..596,608, 596,000, the ROM-wall test's 420,000 and the
  legacy-data test's 500,000) move by 600,000, and console-needle budgets
  without a doc formula (efuse, cpu freq, heap_init, RTCRAM, spi_flash,
  `main_task`, `sleep_gpio`, LVGL) move by 600,000 too, which keeps each
  one's headroom over its line. Each doc keeps the historical step. If
  wrong: an exact-step rung would stop short of the call it was written
  for and pass vacuously.
- **R-T3-5, tick-period thresholds are re-pinned to the measured floor.**
  Two rungs count FreeRTOS tick periods at a step: the WFI rung (counter 1
  past 40 periods at step 5,550,000; measured 42.8 before, 17.5 now) and
  `rom_stub_boot`'s phase 9 (42 periods at `MD5Init`; 42.86 before, 16.78
  now). A busy step is a fifth of a tick at 80 MHz instead of a whole one,
  so the same steps hold fewer periods; the thresholds become 17 and 16,
  the floor of the measured value, the same rule the old 40 and 42 were
  set by. The rungs still prove the scheduler ticks repeatedly after the
  first wake. If wrong: a stalled tick would still pass if it stopped
  after 16 periods (as it would have after 42 before).
- **R-T3-6, `interrupt_integration.rs` runs at a stated clock.** Its
  synthetic programs assumed one SYSTIMER tick per step and a bare bus;
  with the clock following the registers a bare bus runs at the reset
  value, 20 MHz. `build_bus` now writes `SYSCLK_CONF` = 0 (XTAL / 1, 40
  MHz, 2/5 of a tick per step), and the tests derive their step counts
  from that rate (`ticks_after`, `steps_for`) instead of equating steps
  and ticks: the first alarm's trap step is `steps_for(ticks_after(load
  step) + PERIOD)`, which is `load step + PERIOD` at one tick per step.
  The NOP padding grew to 192 so the threshold test's 75 masked steps stay
  on fetchable code. Not in the brief's file list; the file had to change
  for the suite to pass. If wrong: the exact-step interrupt test would
  pin a rate no bus runs at.
- **R-T3-7, RTC_CNTL's conversion is unchanged (backlog).**
  `rtc_cntl.rs` converts SYSTIMER ticks to RTC slow-clock cycles as
  `ticks * RC_SLOW_HZ / XTAL_HZ`, Milestone 3's placeholder from when a
  tick was a step. Ticks are now real 16 MHz time, so RTC time runs at
  `SYSTIMER_HZ / XTAL_HZ` = 0.4 of emulated time; dividing by
  `SYSTIMER_HZ` instead would be faithful and would shorten R-T3-4's
  early-boot wait. Task 3 rewrote the doc for the clock model but kept
  the formula (the brief asked for docs, and changing it moves every early
  pin again); it is a Milestone 7 backlog item. If wrong: early boot
  spends more steps in `rtc_init()` than the badge would at the same CPU
  frequency. (Superseded by R-T3-9: Task D-M6-1 moved the divisor in
  Milestone 6.)
- **R-T3-8, budgets and doc figures re-measured.** Every rung budget
  with a doc formula was re-measured with the same instrumentation on
  `00dd7d5` and on Task 3 (a temporary `eprintln!` in the stable-frame,
  console and provisioning helpers, removed before commit) and recomputed
  by its own formula; several Milestone 5 doc figures were already stale
  at `00dd7d5` (for example `PROV OK` at 29.99M..30.24M and the
  first-run frame at the 16,000,000 sample, not 15,500,000), so the new
  docs quote only Task 3's measurements. Budgets went down where the
  firmware now needs fewer steps (`LAUNCHER_RESPONSE_STEPS` 33M → 26M,
  `PROVISION_DEADLINE` 38M → 37M, the splash `MAX` 11.1M → 10.1M,
  `SELF_TEST_MAX_STEPS` 32M → 31.75M) and up where the formula says so
  (`FIRST_RUN_MAX_STEPS` 17.5M → 17.75M, `CONSOLE_DEADLINE` 23M → 24M;
  "rounded to 1,000,000" is read as rounded up). Stability windows,
  `FRAME_CHUNK`, `ANIMATED_PAGE_STEPS` and `ONBOARDING_RESPONSE_MARGIN`
  stay in steps, unchanged (spec: they measure firmware work). If wrong:
  a budget would sit closer to its event than its formula intends.
- **R-T3-9, the RTC timer counts SYSTIMER time (Task D-M6-1).**
  (Orchestrator ruling after Task 3's review; it pulls R-T3-7 forward
  from the backlog.) Since Task 3 the value `RtcCntl::write_byte`
  receives is SYSTIMER ticks of emulated time, so dividing it by
  `XTAL_HZ` was a unit error against the register-faithful constraint and
  the spec's one-clock rule. The snapshot is now `elapsed_ticks *
  RC_SLOW_HZ / SYSTIMER_HZ` (`SYSTIMER_HZ` from `systimer.c` /
  `SOC_SYSTIMER_FIXED_DIVIDER`, `RC_SLOW_HZ` =
  `SOC_CLK_RC_SLOW_FREQ_APPROX`), so 16,000,000 ticks (1 s) latch 136,000
  slow cycles (`one_second_of_systimer_ticks_is_one_second_of_rc_slow_cycles`);
  the parameter is renamed `elapsed_ticks` and TIMG's `XTAL_HZ` is private
  again. `rtc_init()`'s RTC-timer waits now take the time they ask for,
  which cancels all but 221 steps of R-T3-4's 600,000-step early shift:
  every early event from the first log line to the idle `wfi` is now 221
  steps after its `00dd7d5` step, and the early timestamps are back to
  `00dd7d5`'s exactly (`W (10)`, `I (5)`, `I (6)`, LVGL `I (126)`). Under
  R-T3-4's rule the historical fault-step rungs run to the old step + 221,
  and the round windows and formula-less budgets return to their
  `00dd7d5` values (every event stays inside its window, with the same
  relations as at `00dd7d5`). If wrong: RTC time would again drift
  against SYSTIMER and every timed wait on it would cost the wrong number
  of steps.
- **R-T4-1, Snake's open frame is time-pinned (variant B), and the
  move count starts there.** Snake does not wait for a key: its start
  screen holds only from 75.5 ms to about 222 ms after A's release, far
  shorter than `AFTER_PRESS_STABLE_FOR`, so `launcher_opens_snake` pins
  the frame `SNAKE_OPEN_FRAME_US` = 150 ms after release. The game-over
  rung runs to that same point before `move_times`, so the start-screen
  redraw (56 ms to 75.5 ms after release) is not counted; the count is
  11 moves plus the game-over redraw, which `move_times` sees as a
  twelfth change (`SNAKE_MOVES_TO_WALL` = 12). If wrong: Task 5 compares
  a count that includes or excludes a redraw the badge's observer counts
  differently; the doc comment says which.
- **R-T4-2, the restart check is "the play area moves again".** A on
  game over restarts at once (the start screen is redrawn 22 ms to 34 ms
  into the 100 ms hold, with a new food position) and the new game runs
  without a key, so the brief's `press_and_settle` found the *next* game
  over (a different frame, because the food moved) and its `assert_ne!`
  passed for the wrong reason. Per the brief's fallback the rung asserts
  that `move_times` over `SNAKE_RESTART_WINDOW_US` = 500 ms after A's
  release is non-empty (three moves are expected). If wrong: a restart
  that left the snake frozen would still be caught; one that ran a
  different game would not be.
  Task 6 tightened the check: the frame after A's hold is not
  `SNAKE_GAME_OVER_HASH`, and the play area makes at least three moves in
  `SNAKE_RESTART_WINDOW_US` = 600 ms with a median interval within ±25 %
  of `BADGE_MOVE_INTERVAL_US`.
- **R-T5-1, the badge's move interval is used at compile time.** The
  constant `BADGE_MOVE_INTERVAL_US` = 170,000 (Task 5's human gate) on its
  own raised `dead_code` until Task 6's rungs used it. Instead of
  `#[allow(dead_code)]`, the test file asserts at compile time that
  `SNAKE_TO_WALL_WINDOW_US` holds `SNAKE_MOVES_TO_WALL` ticks at the
  badge's pace (12 × 170,000 = 2,040,000 < 3,000,000 us). It checks the
  window, not the emulator's speed; Task 6 keeps it. If wrong: a shorter
  to-wall window could cut off a badge-paced run without a test noticing.
- **R-T6-1, the steering rung pins the frame the human approved.**
  `SNAKE_TURNED_HASH` = `0xcccb_b8c5_d80a_93e9` is Task 5's "after one
  turn" frame (UP, `SNAKE_TURN_SLOT` = 7), which the human accepted on
  2026-10-20. Task 5 pressed UP 240 ms after A's release and read the
  frame at 485.5 ms; `snake_steers` keeps the plan's sequence (watch
  1.5 move intervals, then press, then run one interval), which presses at
  255.5 ms, releases at 355.5 ms and reads at 525.5 ms. Both presses fall
  between the first move (222 ms) and the second tick (392 ms), which
  turns the head up; the frame is unchanged from 395.5 ms to the third
  tick at 562 ms, so both readings are the same frame. If wrong: the
  pinned turn frame would not be the one compared with the badge.
- **R-T6-2, no "Variant A" step in the speed rung.** Task 4 chose
  variant B (Snake starts on its own), so the plan's "Variant A only"
  comment line in `snake_moves_on_its_own` is dropped; the rung instead
  runs to the open frame (`SNAKE_OPEN_FRAME_US`) before counting, as the
  game-over rung does (R-T4-1). If wrong: the start-screen redraw would
  count as a move.
- **R-T6-3, the speed rung pins the exact move period.** The badge
  comparison stays as the spec's tolerance (median within ±25 % of
  `BADGE_MOVE_INTERVAL_US`), and the rung also asserts that every gap
  between consecutive moves equals `SNAKE_MOVE_INTERVAL_US` = 170,000 us,
  the emulator's exact, deterministic move period (Tasks 4 and 5; it
  agrees with the badge per Task 5). The reason: Task 6's Step 2 found
  that the ±25 % check alone passes under the pre-Milestone-6 clock
  (160 ms), so it did not guard the timing model as the spec asks, and a
  tighter tolerance such as ±5 % would be arbitrary; exact pins are the
  house style for this deterministic emulator. `SPEED_WINDOW_US` ends
  before the game-over redraw (184 ms after the 11th move, about
  2,106 ms after release), so every counted change is a move and none
  needs excluding. Under the legacy clock the rung now fails ("move gaps
  not all 170000 us"; Task 6 below). If wrong: one more pin to re-pin if
  the firmware's timing legitimately moves. (The pin is as fine as the
  2 ms sampling, `MOVE_SAMPLE_US`.)
- **R-T6-4, the restart window is 600 ms.** `SNAKE_RESTART_WINDOW_US`
  went from 500,000 to 600,000 when Task 6 tightened the restart check
  to three moves: the third move comes 416 ms after A's release, and with
  every delay 25 % longer about 545 ms (Task 6 below). If wrong: a
  slightly looser restart window.
- **R-T7-1, Task 7 commits before the human's browser check.** The plan
  put Task 7's commit after its Step 5 human gate, only so the commit
  could carry any tuning the gate called for. The commit came first, so
  the task's code review ran before the human's browser time, and the
  tuning landed as follow-up commits with their measurements here. If
  wrong: one extra commit.
- **R-T7-2, the speed readout divides by the real frame time.** The
  plan's `speed()` divided the emulated advance by the clamped wall
  delta, so the readout could not fall below 1.0x when frames took
  longer than 50 ms, which hides exactly the slow-machine case the
  readout exists to show. `speed()` uses the unclamped wall delta; a tab
  back from the background costs one near-zero sample at the smoothing's
  0.1 weight. If wrong: a brief dip in the readout after a tab returns.
- **R-T7-3, `stop()` flushes only deferred releases.** A deferred
  release (a button let go before its 100 ms hold ran out) is applied at
  `stop()` rather than left pending; buttons still held keep their press
  times and stay asserted, and `reset()` clears everything. Emulated
  time does not
  advance while stopped, so a held button's hold continues after a
  restart. If wrong: a held button's hold measured across a stop.
- **R-T7-4, the steady 1.2x was a pacing defect.** The human saw the
  readout steady at 1.2x while Snake ran. Task 7's contract is emulated
  time equal to wall time, so this was treated as a defect to root-cause,
  not a CPI question: the WFI fast-forward jumped past `run_until`'s
  target to the next FreeRTOS tick. Fixed by `boot::idle_until` (Task 7
  below). If wrong: one diagnosis round.
- **R-T7-5, the step cap is measured.** `DEFAULT_STEPS_PER_FRAME` went
  from the spec's starting guess, 1,000,000, to 150,000: at 1,000,000 a
  busy stretch ran at 9.6 fps in the human's browser (about 104 ms per
  frame, about 10M steps/s), and those long frames caused the input lag
  and the 328 ms INP. 150,000 steps is about one 60 Hz frame of work.
  The spec allows tuning by measurement. If wrong: busy stretches run a
  little slower than they could on fast machines (throughput per wall
  second is mostly unchanged).

## Measurements

### Task 1 baseline (branch base `ed71dd8`, before any Milestone 6 code)

All green. Wall times on the development machine (18 cores):

| Check | Result | Time |
| --- | --- | --- |
| `cargo test --workspace` (debug) | 505 lib + 86 integration tests pass (55 boot rungs) | about 39 s wall including the build; `boot_progress` 27.3 s |
| `cargo test -p emulator-core --release --test boot_progress` | 55 pass | 41.0 s |
| `cargo clippy --workspace --all-targets` | 3 warnings (2 doc-list indentation in `boot.rs`, 1 no-effect operation in a `console.rs` test) | |
| `bun run build:wasm` | ok | |
| `bun test` | 136 pass, 0 fail | 38.4 s |
| `bun run typecheck` | ok | |

### Task 2: SYSTEM clock registers and the stateful ROM frequency pair

With the R-T2-4 seeds (80 MHz on the PLL, `s_ticks_per_us` = 80),
`ets_update_cpu_frequency` is called with 40 at step 836 (the app's
`rtc_clk_init()` switching to the XTAL clock), 80 at 401,743 (its switch
back to the PLL, after `rtc_init()`'s RTC-timer waits), and 80 again at
404,369 while already at 80 (a second set to the same frequency; its
caller was not traced, and since the setter only stores the value it is
harmless), then 40/80 around every idle. Cycle-scaled waits
count 80 instead of 160 per microsecond, and the app's `rtc_clk_init()`
rescales the cycle counter from 80 MHz, not 40, so step-exact pins move.
No frame hash moved, either in the first commit or after fix round 1:
every hash rung passed without being touched, and the splash still has
2,340 colors. "Old" is `ed71dd8` (Task 1 changed no timing, so these are
also `d5e6be3`'s values). "First commit" is `c1c6ee5`, which used the
superseded XTAL seeds. "New" is the result after fix round 1. All values
were measured with the same throwaway probe.

| Rung or doc (test file) | What | Old (`ed71dd8`) | First commit | New |
| --- | --- | --- | --- | --- |
| `rom_stubbed_boot_gets_past_the_mask_rom_wall` doc (`rom_stub_boot`) | `ets_get_cpu_frequency` / `ets_printf` first call | ~400,695 / ~400,714 | ~400,698 / ~400,720 | ~400,689 / ~400,711 |
| `boot_reports_the_badges_80_mhz_cpu_clock` (new, `boot_progress`) | `cpu_start: cpu freq:` line printed | 160000000 Hz at 407,291 | 80000000 Hz at 407,315 | 80000000 Hz at 407,291 |
| `boot_no_longer_faults_at_the_pre_task_d6_qsort_call_site` | steps until `qsort` has returned | 408,740 | 408,791 | 408,767 |
| `first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line` | vPortYield's write / first trap | 528,151 / 528,152 | 528,259 / 528,260 | 528,091 / 528,092 |
| `boot_no_longer_faults_at_the_pre_task_d10_..._call_site` | the step before the first yield | 528,150 | 528,258 | 528,090 |
| `boot_escapes_the_pre_task_9_spi_update_poll_...` doc | `spi_hal_init` poll first reached | 583,976 | 584,379 | 583,919 |
| `boot_no_longer_faults_at_the_pre_task_d12_bswapsi2_call_site` doc | `__bswapsi2` entered | 593,005 | 594,963 | 594,503 |
| `boot_stubs_reach_load_partitions_md5init_...` (`rom_stub_boot`) phase 1 | through step | 528,151 | 528,259 | 528,091 |
| same, phase 3 | `spi_hal_init` poll | 583,976 | 584,379 | 583,919 |
| same, phase 4 / 5 | `__bswapsi2` call / stub step | 593,004 / 593,005 | 594,962 / 594,963 | 594,502 / 594,503 |
| same, phase 6 / 7 / 8 | idle `wfi` reached / retires / fast-forward | 596,595 / 596,596 / 596,597 | 598,556 / 598,557 / 598,558 | 598,096 / 598,097 / 598,098 |
| same, phase 8 | fast-forward jump to the FreeRTOS tick | 91,455 ticks | 89,602 ticks | 89,894 ticks |
| same, phase 9 / 10 | through step / `MD5Init` step | 5,558,962 / 5,558,963 | 5,560,992 / 5,560,993 | 5,560,532 / 5,560,533 |
| `boot_idles_in_wfi_...` doc | `idle_ticks` at step 5,550,000 | 1,827,072 | 1,825,153 | 1,825,445 |
| 12 needles in 10 `boot_progress` rungs, 3 in 2 `rom_stub_boot` tests | early log prefix (R-T2-1) | `I (5)` | `I (10)` heap_init, spi_flash; `I (11)` sleep_gpio, main_task | `I (5)` heap_init, spi_flash; `I (6)` sleep_gpio Enable, main_task |

With timestamps stripped, the console through step 6,000,000 is the same as
`ed71dd8`'s except for the `cpu freq` line. Other first-appearance steps,
for reference (old → new):

| Console line | Old | New |
| --- | --- | --- |
| `cpu_start: Unicore app` | 402,300 (`I (2)`) | 403,272 (`I (5)`) |
| `heap_init` first line | 409,336 | 409,366 |
| RTCRAM line | 415,325 | 415,367 |
| `detected chip: generic` | 446,695 | 446,740 |
| `sleep_gpio: Enable ...` | 484,091 | 484,145 |
| `main_task: Calling app_main()` | 555,165 | 555,108 |
| `LVGL: Starting LVGL task` | 669,268 (`I (125)`) | 670,838 (`I (126)`) |

### Task 3: the clock follows the firmware's CPU frequency

`FirmwareBus::clock` is constructed at the 40 MHz XTAL clock and, once
per step (after the instruction) and at the start of every idle
fast-forward, takes the frequency `System::cpu_hz` derives from the SYSTEM
clock registers; an invalid encoding keeps the previous rate.
`TICKS_PER_STEP` and `PACKET_STEPS` are gone; one OUT packet is
`PACKET_TICKS` = 821 SYSTIMER ticks (51.3 µs, about 4,105 steps at
80 MHz). Button holds and the shake run in emulated time:
`PRESS_HOLD_US` = 100,000 and `SHAKE_HALF_US` = 62,500 in the rungs, the
WASM twin (`runForUs` over `runUntil`) and the browser runtime.

**No frame hash moved.** Every hash rung passed unchanged on the first
run (splash, first-run screen, self-test, the 11 roles' page 1 and
registered screens, role hacker's 14 onboarding frames and the shaken
page, page 7..11, the launcher, launcher after DOWN, the six frames to
Dice, Dice open and the Dice roll `0x692c6023de06d917`). Nothing went to
the human gate. With log timestamps stripped, the console through step
20,000,000 is identical to `00dd7d5`'s.

Measured on `00dd7d5` ("old") and on Task 3 ("new") with the same
probes; old values from the rung docs are given where the docs were
current.

| Item (file) | Old | New | Reason |
| --- | --- | --- | --- |
| `cpu freq` console line | `I (5) cpu_start: cpu freq: 80000000 Hz` at ~407,291 | `I (12) cpu_start: cpu freq: 80000000 Hz` at ~1,007,000 | R-T3-4 shift; timestamp = cycle counter / 80 |
| early timestamps | `W (10)` rtc_init; `I (5)` cpu_start..spi_flash..sleep_gpio Configure; `I (6)` sleep_gpio Enable, main_task | `W (25)` rtc_init; `I (12)` cpu_start..heap_init; `I (13)` spi_flash, sleep_gpio; `I (14)` main_task | same; needles re-pinned exactly |
| `LVGL: Starting LVGL task` | `I (126)` at ~671,000 | `I (134)` at ~1,272,000 | shift |
| first non-blank frame | 1,133,900 | 1,725,000 (100-step sampling) | shift + LVGL timing |
| splash final (`boots_to_first_real_frame`) | 5,541,000; found at the 6,750,000 sample; `MAX` 11,100,000 | 5,032,000; 6,250,000 sample; `MAX` 10,100,000 | fewer FreeRTOS ticks per step |
| littlefs `mount failed` / superblocks | ~5,620,000 / 5,692,000 | ~5,108,000 / 5,174,000 | |
| `hal_accel` / `hal_sleep` / `launched My Badge` | ~6.09M / ~6.14M / ~12.29M (docs) | ~5.55M / ~5.64M / ~11.59M | |
| first-run screen first sample (`FIRST_RUN_MAX_STEPS`) | 16,000,000 (doc said 15,500,000); 17,500,000 | 15,750,000; 17,750,000 | |
| START → self-test (`SELF_TEST_MAX_STEPS`) | first 23,600,000; 32,000,000 | first 22,658,244 (hold 5,658,244 steps; first frame change 3.47M after the press); 31,750,000 | time-based hold |
| `hal_console: console started` (10k sampling) | 16,860,000 (doc) | 16,650,000; budget 21,000,000 unchanged | |
| console rungs (`CONSOLE_DEADLINE`) | doc 16.78M / 17.24M / 17.33M / 18.4M; 23M | prompt 16.64M (1,000-step sampling), READY 17.05M, `PROV FAIL` 17.50M, stable check 18.70M; 24M | |
| 600-byte `put` (`RING_TEST_DEADLINE`) | 18.63M (doc 17.88M); 23M | 17.68M; 23M unchanged | |
| `PROV OK`, all roles (`PROVISION_DEADLINE`) | 29.99M..30.24M (doc 30.19M); 38M | 28.64M..28.89M; 37M | |
| role hacker: page 1 stable | 31,392,000 | 30,042,000 | |
| shake starts / shaken page stable | 224,042,000 / 230,542,000 | 226,830,776 / 233,962,348 | holds 3.5M..6.9M steps |
| page 7 after A's release | 8.75M | 3.0M | |
| page 8 animation in `ANIMATED_PAGE_STEPS` (9M) | ~14 frames, 0.25M..2M each (doc) | 24 changes, 0.1M..0.85M each | |
| registered screen after START's release (`REGISTERED_RESPONSE_STEPS`) | 9.25M..9.5M (doc 4.3M..4.7M) | 4.5M..5.5M | constant unchanged (margin) |
| role hacker: registered screen stable | 333,742,000 | 338,806,407 | |
| HOME → `launched Launcher` / launcher stable, after the release (`LAUNCHER_RESPONSE_STEPS`) | ~9M before the draw (doc) / 17.75M..18.0M; 33M | 2.6M / 12.25M; 26M | |
| **steps to the launcher** (role hacker, first stable) | 361,092,000 (CLAUDE.md said ~317.5M) | 365,842,438 | |
| launcher after DOWN | 5.5M after release; 376,192,000 | 1.25M; 380,750,668 | |
| **steps to Dice** open / rolled | 467,542,000 / 477,392,000 | 469,745,544 / 480,131,502 | |
| Dice open after A's release | ~3.3M | 3.25M | |
| `first_trap_is_the_from_cpu_0_yield...` | write 528,091 / trap 528,092; no exception to 571,712 | 1,128,029 / 1,128,030; 1,171,942 | R-T3-4 |
| D10 rung (the step before the first yield) | 528,090 | 1,128,028 | R-T3-4 |
| qsort return | 408,767 | 1,008,561 | R-T3-4 |
| D2 / D3 / fix round 1 / D4 exact steps | 401,761 / 402,113 / 405,806 / 407,471 | 1,001,555 / 1,001,907 / 1,005,600 / 1,007,265 | R-T3-4 (+599,794) |
| D9 / memchr / D11 / rtc_xtal runs | 440,000 / 493,000 / 596,000 / 596,000 | 1,040,000 / 1,093,000 / 1,196,000 / 1,196,000 | R-T3-4 (+600,000) |
| SPI poll window, first poll | 583,000..596,000, 583,919 | 1,183,000..1,196,000, 1,184,149 | R-T3-4 |
| `__bswapsi2` window, stub step | 590,000..596,608, 594,503 | 1,190,000..1,196,608, 1,194,733 | R-T3-4 |
| console-needle budgets (efuse, heap_init, cpu freq, RTCRAM, spi_flash, sleep_gpio, `main_task`, LVGL) | 420k, 420k, 500k, 500k, 500k, 500k, 750k, 1M | 1.02M, 1.02M, 1.1M, 1.1M, 1.1M, 1.1M, 1.35M, 1.6M | R-T3-4 (+600,000) |
| GDMA rung: blank at | 1,100,000 | 1,700,000 | first pixels moved |
| WFI rung at 5,550,000: `idle_ticks` / counter 1 | 1,825,445 / 6,847,442 (≥ 40 periods) | 1,920,289 / 2,806,273 (≥ 17 periods) | R-T3-5 |
| `rom_stub_boot` phases 1 / 3 / 4 / 5 | 528,091 / 583,919 / 594,502 / 594,503 | 1,128,029 / 1,184,149 / 1,194,732 / 1,194,733 | R-T3-4 |
| phases 6 / 7 / 8 (wfi, retire, fast-forward) | 598,096 / 598,097 / 598,098, jump 89,894 ticks | 1,198,326 / 1,198,327 / 1,198,328, jump 145,882 ticks | |
| phases 9 / 10 (`MD5Init`) | 5,560,532 / 5,560,533, ≥ 42 periods | 5,050,336 / 5,050,337, ≥ 16 periods (16.78) | R-T3-5 |
| ROM-wall test budget / legacy-data run | 420,000 / 500,000 | 1,020,000 / 1,100,000 | R-T3-4 |
| `a_clone_runs_exactly_like_its_original` checkpoint | 1,100,000 | 1,700,000 | first pixels moved |
| runtime WFI test / emulated-time test | 1,000 ticks / 100 µs | 200 ticks / 20 µs | R-T3-1 |
| WASM twin: splash horizon / first-run cap / hold / shake | 6,750,000 / 17,500,000 / 1.6M steps / 1M steps | 6,250,000 / 17,750,000 / 100 ms / 62.5 ms | mirrors the rungs |

**Chunk gap (`PUT_CHUNK_GAP_STEPS`).** With packets 821 ticks apart
(about 4,105 steps at 80 MHz, was 8,210), the brief's rule was applied
with `provisions_hacker_through_the_console` and
`put_accepts_a_payload_larger_than_the_rx_ring`: 24,000 passes both, so
48,000 stays (2x margin). For the record, 12,000 passes the provisioning
walk but fails the 600-byte `put`. `frontend/src/runtime/provisioner.ts`
and its test keep 48,000.

**Hold cost.** A 100 ms hold is 8M steps of busy CPU at 80 MHz; the
measured holds took 0.18M to 6.9M steps (Dice's A press 176,690, mostly
idle; onboarding presses 3.1M..6.9M), against 1.6M before.
`HOLD_MAX_STEPS` = 40M is never approached.

**Wall times** (18 cores): release `boot_progress` 56 pass in 41.6 s
(before 41.2 s); debug `cargo test --workspace` 29.2 s wall including
`boot_progress`'s 28.0 s; `bun test` 137 pass in 36.4 s. The suite stays
far from the 90 s stop line, so the opt-level-3 backlog item is not
needed for this task.

### Task D-M6-1: the RTC timer counts SYSTIMER time

Ruling R-T3-9. No frame hash moved (every hash rung passed unchanged,
including all hardware-compared frames and the Dice roll), and with
timestamps stripped the console is unchanged; the timestamps themselves
went back to `00dd7d5`'s. Measured with the same probes and
instrumentation as Task 3 (removed before commit).

| Item | `00dd7d5` | Task 3 (`b172f97`) | D-M6-1 | Reason |
| --- | --- | --- | --- | --- |
| first log line (`rtc_init`) | ~401,000, `W (10)` | ~1,001,000, `W (25)` | ~401,000, `W (10)` | RTC waits now take their real time |
| early needles | `I (5)` / `I (6)` | `I (12)` / `I (13)` / `I (14)` | `I (5)` / `I (6)` | same; re-pinned exactly |
| LVGL line | `I (126)` ~671,000 | `I (134)` ~1,272,000 | `I (126)` ~672,000 | |
| `ets_get_cpu_frequency` slot / `ets_printf` first call | 400,689 / 400,711 | 1,000,483 / 1,000,505 | 400,910 / 400,932 | +221 vs `00dd7d5` |
| qsort return | 408,767 | 1,008,561 | 408,988 | |
| D2 / D3 / fix round 1 / D4 runs | 401,761 / 402,113 / 405,806 / 407,471 | 1,001,555 / 1,001,907 / 1,005,600 / 1,007,265 | 401,982 / 402,334 / 406,027 / 407,692 | old + 221 |
| first yield write / trap; D10; no-exception-to | 528,091 / 528,092; 528,090; 571,712 | 1,128,029 / 1,128,030; 1,128,028; 1,171,942 | 528,312 / 528,313; 528,311; 571,933 | |
| SPI poll first reached | 583,919 | 1,184,149 | 584,140 | |
| `__bswapsi2` call / stub | 594,502 / 594,503 | 1,194,732 / 1,194,733 | 594,723 / 594,724 | |
| idle `wfi` / retire / fast-forward; jump | 598,096 / 7 / 8; 89,894 ticks | 1,198,326 / 7 / 8; 145,882 | 598,317 / 8 / 9; 145,940 | |
| round windows (D9, memchr, D11, rtc_xtal, SPI, bswap, ROM wall, legacy data) | 440k, 493k, 596k, 596k, 583k..596k, 590k..596,608, 420k, 500k | +600,000 each | `00dd7d5` values | R-T3-9 |
| needle budgets (efuse, heap_init, cpu freq, RTCRAM, spi_flash, sleep_gpio, `main_task`, LVGL) | 420k, 420k, 500k, 500k, 500k, 500k, 750k, 1M | +600,000 each | `00dd7d5` values | |
| first non-blank frame; GDMA "blank at"; clone checkpoint | 1,133,900; 1.1M; 1.1M | 1,725,000; 1.7M; 1.7M | 1,124,900; 1.1M; 1.1M | |
| splash final; found at; `MAX` | 5,541,000; 6.75M; 11.1M | 5,032,000; 6.25M; 10.1M | 4,432,000; 5.5M; 8.9M | formula |
| `MD5Init` (rom_stub_boot phases 9/10); periods | 5,560,532/3; 42.86 | 5,050,336/7; 16.78 | 4,450,327/8; 16.78 (≥ 16 unchanged) | |
| `load_partitions_accepts_...` run before single-stepping | 5,000,000 | 5,000,000 (passed) | 4,000,000 | `MD5Init` now before 5M |
| WFI rung at 5.55M: `idle_ticks`; counter 1 | 1,825,445; 42.8 periods (≥ 40) | 1,920,289; 17.5 (≥ 17) | 2,262,240; 20.4 (≥ 20) | R-T3-5 floor |
| littlefs mount failed / superblocks | ~5,620,000 / 5,692,000 | ~5,108,000 / 5,174,000 | ~4,508,000 / 4,574,000 | |
| `hal_accel` / `hal_sleep` / `launched My Badge` | ~6.09M / ~6.14M / ~12.29M | ~5.55M / ~5.64M / ~11.59M | ~4.95M / ~5.04M / ~10.99M | |
| first-run screen sample; `FIRST_RUN_MAX_STEPS` | 16.0M; 17.5M | 15.75M; 17.75M | 15.25M; 17.25M | formula |
| self-test first sample; `SELF_TEST_MAX_STEPS` | 23.6M; 32M | 22,658,244; 31.75M | 22,158,230; 31.25M | formula |
| `badge> ` prompt (1,000-step sampling); console started (10,000-step) | ~17.61M; doc 16,860,000 | 16.64M; 16,650,000 | 16.04M; 16,050,000 (budget 21M unchanged) | |
| console rungs: READY / `PROV FAIL` / stable check end; `CONSOLE_DEADLINE` | 18.05M / 21.9M / 23.1M; 23M | 17.05M / 17.50M / 18.70M; 24M | 16.55M / 17.00M / 18.20M; 23M | formula |
| 600-byte `put` done; `RING_TEST_DEADLINE` | 18.63M; 23M | 17.68M; 23M | 17.18M; 22M | formula |
| `PROV OK`, all roles; `PROVISION_DEADLINE` | 29.99M..30.24M; 38M | 28.64M..28.89M; 37M | 28.19M..28.39M; 36M | formula |
| responses after release: registered / page 7 / launcher / nav / Dice open / START's first change | 9.25..9.5M / 8.75M / 17.75..18.0M / 5.5M / ~3.3M / ~6M (doc) | 4.5..5.5M / 3.0M / 12.25M / 1.25M / 3.25M / 3.47M | same as Task 3 (START 3.48M) | `LAUNCHER_RESPONSE_STEPS` 26M and the response docs unchanged |
| walks to the registered screen (11 roles) | 341.5M..342.8M | 346.6M..348.0M | 345.9M..347.9M | |
| **launcher** (hacker, first stable) | 361,092,000 | 365,842,438 | 364,961,924 | |
| **Dice** open / rolled | 467,542,000 / 477,392,000 | 469,745,544 / 480,131,502 | 468,864,915 / 479,250,828 | |
| holds (all presses) | 1.6M | 0.18M..6.9M | 0.18M..6.93M | |
| WASM twin: splash horizon / first-run cap / `PROVISION_DEADLINE` | 6.75M / 17.5M / 38M | 6.25M / 17.75M / 37M | 5.5M / 17.25M / 36M | mirrors |

Wall times (the full check, all green): release `boot_progress` 56 pass
in 41.1 s; debug `cargo test --workspace` 30.8 s (534 lib tests);
`bun test` 137 pass; clippy 3 warnings (unchanged).

### Task 4: Snake opens and reaches game over (provisional until Task 5)

Explored from the hacker launcher checkpoint (DOWN x3, RIGHT x2, A held
100 ms; A released at emulated 118,871,665 us, step 449,179,709), sampling
the framebuffer every 1 ms for the first 2.4 s and every 20 ms for 5 s.
Every value below was provisional until Task 5 compared it with the
physical badge (Task 5 confirmed them).

| Question | Answer |
| --- | --- |
| `app_reg: launched Snake` | printed (`launcher_opens_snake` asserts it) |
| waits for a key before moving? | no: start screen drawn 56..75.5 ms after release, first move ~222 ms |
| start direction | right, three cells on the play area's middle row |
| play area (`PLAY_AREA`) | x 15..305, y 41..235 (half-open, inside the 1-px border at x 14/305, y 40/235; measured along row 140 and column 100 of the start frame); cells 12 px |
| time between moves | 170 ms exactly (each move flushes in two stripes 3.5 ms apart) |
| moves to the wall with no input | 11; the game-over overlay is drawn on the 12th tick, 2,105..2,114 ms after release |
| game over | "Game over / A to restart" box over the play area; stable (unchanged to 5 s) |
| A on game over | restarts at once with a new food position; the snake moves ~72 ms after release |
| cost | 5 s of Snake is 17.6M steps (~1.4 s wall, release); the five navigation presses + A take ~5.7 s wall from the launcher checkpoint |

| Pin | Value |
| --- | --- |
| `SNAKE_OPEN_FRAME_US` | 150,000 |
| `SNAKE_OPEN_HASH` | `0xe7d5_38c7_42f1_9d89` |
| `SNAKE_TO_WALL_WINDOW_US` | 3,000,000 (from the open frame; game over ~1,964 ms after it, + 50 %) |
| `SNAKE_MOVES_TO_WALL` | 12 (11 moves + the game-over redraw, R-T4-1) |
| `SNAKE_GAME_OVER_HASH` | `0x3582_df91_6283_9f3a` |
| `SNAKE_RESTART_WINDOW_US` | 500,000 (R-T4-2) |
| `MOVE_SAMPLE_US` / `MOVE_MERGE_US` | 2,000 / 20,000 |

The food dot's positions come from the emulator's fixed-seed RNG, so
they are not the badge's; Task 5 compares layout, not the food.

### Task 5: Snake compared with the physical badge (human gate, 2026-10-20)

**Emulator side** (from the open point, no input, 3 s window): the 10
intervals between moves 1 and 11 are all exactly 170,000 us, so the
median is **170,000 us** (about 5.88 moves/s); 11 moves to the wall, and
the game-over redraw 184,000 us after the 11th move. The emulator's
three frames (start screen at `SNAKE_OPEN_HASH`, the snake one move
after one turn UP, and game over at `SNAKE_GAME_OVER_HASH`) were
exported as PNGs for the comparison (kept under `local/`, not
committed).

**Badge side.** The human recorded the physical badge with a phone (an
untouched run into the wall, and a run with one turn) and asked that the
numbers be measured from it. They were measured frame by frame (29.97
fps, true frame timestamps):

| Question | Badge | Emulator |
| --- | --- | --- |
| time between moves, untouched run | mean 170.0 ms over 10 intervals (first-to-last-move mean, ±3 ms); per-interval median 166.7 ms, because single intervals quantize to 4 to 6 video frames | 170.0 ms, every interval |
| time between moves, one-turn run | mean 170.8 ms over 17 intervals | n/a |
| moves before game over, untouched | 11 | 11 (`SNAKE_MOVES_TO_WALL` = 12 counts the game-over redraw too, R-T4-1) |
| one-turn run | one turn right → up: 10 moves right, then 8 up to the top wall | n/a |
| geometry | play area about 24 × 16 cells; 3-cell snake starting rightward from the same cell | the same |
| game-over screen | the same as the emulator's | "Game over / A to restart" |
| start screen to first move | about 168 ms | about 147 ms (start screen complete 75.5 ms after A's release, first move at about 222 ms) |
| last move to game-over overlay | about 200 ms | 184 ms gap |

The last two rows agree within the video's frame quantization (33.4 ms
per frame); recorded as an observation, no action.

**Verdicts.** The human pointed at the videos instead of separate
photos. All three emulator PNGs (open, after one turn, game over) are
accepted as matching ("all pngs match"). The food position differs, as
expected: the emulator's RNG has a fixed seed.

**Comparison.** `BADGE_MOVE_INTERVAL_US` = 170,000 (defined in
`boot_progress.rs`, citing this gate; Task 6's speed rung consumes it).
|170,000 − 170,000| = 0 ≤ 170,000 / 4 = 42,500, inside ±25 %: no
calibration. **CPI 1 stands.** No change to `cycles_per_step`
(`mem/bus.rs`) or the `mpccr` cycle counter. Moves to the wall agree
(11 = 11), and no frame was rejected.

CPI could not have moved the interval anyway: the CPU spends 0.9567 of
the 3 s window idle in WFI (`Clock::idle_ticks()` over
`Clock::total_ticks()` deltas: 46,005,504 / 48,087,760 ticks; 0.9604 to
0.9613 per move interval). Each 170 ms tick keeps it busy for only about
6.6 ms, so the move interval is the firmware's own timer period, not CPU
throughput. A mismatch would have pointed at the clock rules, not at
CPI.

With the comparison done, `SNAKE_OPEN_HASH`, `SNAKE_MOVES_TO_WALL` and
`SNAKE_GAME_OVER_HASH` are compared with the badge; they are no longer
provisional. The test file keeps one compile-time check on the new
constant (ruling R-T5-1): `SNAKE_TO_WALL_WINDOW_US` must hold `SNAKE_MOVES_TO_WALL`
ticks at the badge's pace (12 × 170,000 = 2,040,000 < 3,000,000 us).

### Task 6: Snake's speed and steering rungs

Two new rungs, measured from the hacker launcher checkpoint (times after
A's release). No frame hash, step pin or existing constant moved except
`SNAKE_RESTART_WINDOW_US` (below).

| Rung | What it checks | Measured |
| --- | --- | --- |
| `snake_moves_on_its_own` | from the open frame (150 ms), at least six moves in `SPEED_WINDOW_US`; every gap exactly `SNAKE_MOVE_INTERVAL_US` (R-T6-3); median interval within ±25 % of `BADGE_MOVE_INTERVAL_US` | seven moves, 222 ms to 1,242 ms; every interval 170,000 us |
| `snake_steers` | UP (`SNAKE_TURN_SLOT` = 7) after the first move; the frame one interval after release is `SNAKE_TURNED_HASH`; at least three moves follow | first move 222 ms; press 255.5 to 355.5 ms; read at 525.5 ms, hash `0xcccb_b8c5_d80a_93e9` (R-T6-1); eight changes follow (seven moves up, then game over at the top wall) |
| `snake_hits_wall_game_over` (tightened, R-T4-2) | after A on game over: the frame is not game over, and at least three moves with a median within ±25 % of the badge's | moves 76, 246, 416 and 586 ms after release |

**`SPEED_WINDOW_US` = 1,200,000** (from the open frame). The first move
comes 72 ms after the open frame and then one every 170 ms (Task 4), so
six moves need 72 + 5 × 170 = 922 ms. At the slow edge of the ±25 % band
(212.5 ms) they need 72 + 5 × 212.5 = 1,134.5 ms, inside the window. At
the fast edge (127.5 ms) the window holds at most
1 + (1,200 − 72) / 127.5 = 9 moves, short of the 11th move that reaches
the wall, so no game-over redraw is counted anywhere in the band.

**`SNAKE_RESTART_WINDOW_US`: 500,000 → 600,000.** The third move after
A's release comes at 416 ms; with every delay 25 % longer it would come
at about 1.25 × (176 + 340) − 100 = 545 ms (176 ms from the press to the
first move, two intervals of 170 ms, less the 100 ms hold), which the
old 500 ms window would miss.

**Step 2, the regression check.** With the clock forced to the legacy
ratio (one SYSTIMER tick per step, the pre-Milestone-6 clock, through a
temporary probe in `FirmwareBus::sync_cpu_clock`, reverted), measured two
ways:

- From boot, the walk does not reach Snake: the firmware logs
  `launched Launcher`, but no stable launcher frame is found within the
  369.3M-step cap.
- From A's release (the probe switched on after `snake_open()`), the
  median move interval is **160,000 us** (play-area changes after the
  open frame at 226, 448, 608, ..., 1,248 ms after release; the gaps are
  222 ms, then 160 ms each). |160,000 − 170,000| = 10,000 ≤ 42,500,
  so with only the badge comparison `snake_moves_on_its_own` passed
  under the legacy clock: the interval is the firmware's own timer
  period, not CPU throughput (Task 5: the CPU is about 96 % idle). The
  same clock fails the other three Snake rungs: `launcher_opens_snake`
  (the start screen is not drawn by 150 ms), `snake_steers` (a different
  frame at the read point) and `snake_hits_wall_game_over` (13 changes
  instead of 12). Ruling R-T6-3 (see Rulings) added an exact pin, and Step 2
  was re-run.

**Step 2 re-run after ruling R-T6-3** (the exact gap pin; see Rulings),
legacy clock from A's release, probe reverted: `snake_moves_on_its_own`
**FAILS**, "move gaps not all 170000 us", with gaps 221,951 us and then
160,000 us (160,000 ≠ 170,000). The exact pin guards the timing model;
the ±25 % check is the badge comparison.

Checks (after R-T6-3): release `boot_progress` 60 passed in 41.2 s;
`cargo test --workspace` green (`boot_progress` 27.9 s, before R-T6-3);
clippy 3 warnings (unchanged); `cargo fmt --check` clean.

### Task 7: browser pacing to wall-clock time

The browser's frame loop now runs the emulator to each frame's wall-clock
target (`runUntil`), with the wall delta clamped to 50 ms and a step cap
per frame; a capped frame drops its debt, and `speed()` (the status
line's `N.Nx`) shows the smoothed emulated/wall ratio over the real,
unclamped frame time (R-T7-2).

**The human's browser check (Step 5, 100-120 Hz display) found:**

- **Steady 1.2x while Snake runs (R-T7-4).** The cause is in
  `emulator-core`: `run_until` stopped only between steps, and the WFI
  idle fast-forward jumped to the next SYSTIMER alarm even past the
  target. With FreeRTOS's 10 ms tick, a probe of 600 `run_until` calls
  of 8,333 us measured:
  - First-run screen: every frame ended 1,667 us late (on the next tick
    boundary). Ratio 1.200.
  - Snake: frames ended a mean of 1,643 us late. Ratio 1.197.

  Each frame's target is `now + delta`, so the lateness accumulated.
  **Fix**: `boot::idle_until`. When the core waits in WFI with nothing
  asserted and the next event lies past the target, `run_until` advances
  the idle time only to the target, takes no step, and stops. The same
  probe then measured 0 us late on all 600 frames in both places, a
  ratio of 1.000. Splitting one idle wait into two charges changes
  nothing the guest sees (the alarm is reached at the same tick, and the
  waking step is the same step, taken by the next run). All 60
  `boot_progress` rungs pass with no pin moved, including the rungs whose
  holds end inside an idle wait. The new rung
  `run_until_ends_idle_frames_at_the_target` pins it: 120 frames of
  8,333 us on the first-run screen, each ending exactly at its target.
  Before the fix it failed on frame 0, at 1,401,616 us against a target
  of 1,400,005 us.
- **0.2x and 9.6 fps after onboarding; 328 ms INP (R-T7-5).** At the
  first cap of 1,000,000 steps, a busy stretch ran at 9.6 fps, so a
  frame took about 104 ms and the browser ran about 10M steps/s. Bun's
  JavaScriptCore runs the same WASM at 12.3M steps/s (busy boot, 16M
  steps; `framebuffer()` copies in 0.02 ms). **`DEFAULT_STEPS_PER_FRAME`
  = 150,000**, which is 12 to 15 ms of work at those rates, about one
  60 Hz frame. Snake needs about 4,000 to 30,000 steps per 8.3 ms frame
  and about 230,000 for a frame that redraws a move, so only move frames
  are capped (about half a frame of debt each, every 170 ms). Busy
  firmware still runs slowly: at about 10M steps/s against the badge's
  80 MHz, a fully busy stretch cannot exceed about 0.12x in the browser.
- **Input lag.** Button events reach the emulator at once:
  `shell.ts`'s mousedown/keydown handlers call `injectButton`, which
  calls `setRawButton` synchronously, with no queue and no wait for a
  frame. The firmware acts on the press, not the release: on the hacker
  launcher, DOWN held without release redraws the selection 60 ms (of
  emulated time) after the press. So `PRESS_HOLD_US` (which only defers a
  release) adds no lag. The 60 ms is the firmware's own poll and redraw
  and is not changed. The lag was the long frames: a key event waits for
  the running frame (about 104 ms), and its paint waits for the next one,
  which matches the 328 ms INP. The 150,000-step cap bounds both.

Fixes: `36f7551` (`boot::idle_until`) and `292e740` (the 150,000-step
cap); no rung pin moved.

**The human's re-check (same day, after both fixes):**

- Snake runs at 0.9x to 1.0x and 100 to 110 fps and feels like the
  badge; input lag does not affect it. INP is 64 ms (was 328 ms).
- Each launcher move dips the readout from 1.0x to about 0.1x and the
  frame rate from 120 to 60 fps, then both recover. Onboarding and the
  launcher show input lag, and the screen visibly repaints top to bottom.

**Cause:** busy redraws are CPU-bound. The core runs about 10M to 12M
steps/s (about 10M in the browser; natively, the release suite's
longest chain, hacker's walk of about 487M steps to Dice, takes
about 42 s, about 11M/s) against the badge's 80M cycles/s, so a
full-screen redraw plays
back about 8x slower than on the badge, and input waits behind it.
Pacing, Task 7's contract, is correct: an idle-dominated app keeps real
time. Throughput is not in Task 7's scope.

**The human's decision:** accept Task 7 and put emulator core
throughput on the Milestone 7 backlog ("Backlog it").

Steady-state Snake readout: a frame that redraws a move needs about
230,000 steps, more than the 150,000 cap, so about half a frame of debt
is dropped per move (every 170 ms); the readout sits at about 0.95x on
Snake by design, matching the human's 0.9x to 1.0x.

Out of scope: the dev server's "ReferenceError: Can't find variable:
process" comes from `fengari/src/luaconf.js` line 3, which reads
`process.env.FENGARICONF` without a guard. It predates this branch:
`frontend/package.json`, `frontend/bun.lock` and
`frontend/vite.config.ts` are unchanged since `ed71dd8`.

### Task 8 (stretch): Snake eats

`snake_eats` in `boot_progress.rs`. The grid is 24 x 16 cells of 12 px
from (16, 42). At the start frame the snake fills cells (10..=12, 8),
head (12, 8) heading right, and the **food is at (23, 14)** (pixels x
292..=302, y 210..=220), found by a color census of `PLAY_AREA` in a
temporary exploration test. The food comes from the fixed-seed RNG
(`apb_ctrl::RNG_SEED`) and is already part of the pinned
`SNAKE_OPEN_HASH`; the emulator reads no host time or entropy, and
repeated runs gave the same frames step for step.

**Path:** one move right to (13, 8); DOWN (slot 4), held 255 to 355 ms
after A's release, turns move 2 (392 ms), and move 7 reaches (13, 14);
RIGHT (slot 6), held 1,276 to 1,376 ms (the same phase, six intervals
later), turns move 8, and move 17 (2,942 ms) enters the food cell. The
after-eat frame is drawn by 2,948 ms and holds past 3,161 ms; the rung
samples it at 2,991 ms (9.5 intervals after RIGHT's release). The next
move hits the wall.

**`SNAKE_ATE_HASH` = `0x8275_2f57_3ba4_ddab`**: the snake four cells
long on row 14 (snake body color `0x07f5`: 327 px at open, 436 px
after), the new food at (15, 13), score 1. Not compared with the
physical badge (its food position differs, Task 5). No existing pin
moved.

Open question (Milestone 7 backlog): after the eat, the game-over draw
comes 50 to 134 ms after the tick that hits the wall, against about
22 ms with no eat. Not explained; a high-score save is a guess, not
verified.

### Task 9: suite times at the end of the milestone

Measured on the development machine (18 cores) at the Task 9 commit:

| Check | Result | Time |
| --- | --- | --- |
| `cargo test --workspace` (debug) | 534 lib + 93 integration tests pass (62 boot rungs) | 31 s wall including the build check; `boot_progress` 28.2 s |
| `cargo test -p emulator-core --release --test boot_progress` | 62 pass | 41.8 s (42 s wall) |
| `cargo clippy --workspace --all-targets` | 3 warnings, the same three as at `ed71dd8` | |
| `bun run build:wasm`, `bun run typecheck` | ok | |
| `bun test` | 147 pass, 0 fail | 37.1 s |

Against the branch base (`ed71dd8`: release 41.0 s, debug `boot_progress`
27.3 s, `bun test` 38.4 s) the suites are flat: the seven new rungs
(six Snake rungs and one boot rung) branch from shared checkpoints.

## Milestone 7 backlog

The Milestone 6 backlog (`docs/milestone-5-decisions.md`) items this
milestone did not finish are carried here, plus what Milestone 6 found.

### Speed and timing

- **Emulator core throughput** (the human's decision at Task 7's
  browser gate). The core runs about 10M to 12M steps/s against the
  badge's 80M cycles/s, so busy redraws play back about 8x slower than
  on the badge: the launcher and onboarding repaint visibly and input
  waits behind them. Profile the core, try a decoded-instruction cache,
  and run the emulator in a Web Worker so input never waits on a busy
  frame; the goal is busy redraws at real time.
- The release profile is `opt-level = "s"`; at `opt-level = 3` the
  release `boot_progress` suite once ran in 23.0 s instead of 33.8 s
  (Milestone 5, not re-measured), and the WASM build would likely speed
  up the same way. Needs a WASM size and browser-speed check; part of
  the throughput item.
- **Snake's game-over draw after an eat** comes 50 to 134 ms after the
  tick that hits the wall, against about 22 ms with no eat (Task 8).
  Unexplained; possibly a high-score save, unverified.
- CPI 1 is calibrated only against a timer-paced app (Snake's move
  period). Busy work (a redraw, the Dice roll animation, which played "a
  bit faster" than the badge under the pre-Milestone-6 clock and was not
  re-checked) has not been timed against the badge.
- Zero-latency RMT, I2C0 and SPIMEM1 (carried).
- The cycle counter counts one per working step; PCER/PCMR do not gate
  it (carried).
- `PROVISION_TIMEOUT_STEPS` (60M) in `frontend/src/runtime/firmware-runtime.ts`
  is still counted in steps. Under wall-clock pacing an idle firmware
  advances only about 0.5M steps/s in the browser, so a hung provision
  takes minutes to time out. Add an emulated-time or wall-time bound.

### Apps and UI

- Other built-in apps are untested; only Dice and Snake are checked.
- RNG: a browser-supplied seed for `APB_CTRL_RND_DATA_REG` (today fixed).
- A console pane in the frontend (the firmware's REPL is reachable
  through `serialInput`/`consoleOutput`); `serialInput` copies the whole
  JS array before the 1 MiB cap is applied, so cap or chunk it on the JS
  side when the pane lands.
- Motion input beyond the Shake button (`DeviceMotionEvent` on a phone).
- Milestone 5 Task 7 review minors still open: a fetch error message
  lingers and can be masked by a stale provision state (clear it on a
  successful `provision()` and on reset, `main.ts`); two fast clicks
  start concurrent fetches and the last to resolve wins, not the last
  click. (The third, flushing deferred releases in `stop()`, is done.)
- Small Task 7 cleanups: `speed()` returns 0 as its "no reading yet"
  sentinel; `main.ts` writes the status separator as a literal middle dot
  where the file otherwise uses escapes; `pollShake` in
  `firmware-runtime.ts` clears `lastShakeAccel` twice in a row.
- The Lua mode's dev server logs "ReferenceError: Can't find variable:
  process": `fengari/src/luaconf.js` reads `process.env.FENGARICONF`
  without a guard. It predates Milestone 6.
- Only role hacker's frames were compared with hardware; onboarding page 8
  ("Bump to connect") is animated and not pinned; the six navigation
  frames between the launcher and Dice and Snake's after-eat frame are
  pinned but not compared.

### Peripherals

- RMT transmissions are consumed but not decoded, so the LEDs (including
  `prov apply`'s green flashes and role colour) are never shown.
- NFC is not modeled (the self-test waits for a card forever).
- The busy phase after START on the first-run screen (about 6M steps
  under the old clock; hot PCs `0x420b_ead4`, `0x420b_b6a4`) is not
  traced.
- SC7A20H: data ready is always set (no ODR time base); 1 mg/digit is
  the typical sensitivity where the datasheet's table says 1024 counts/g;
  the face-up rest orientation is an assumption.
- I2C0: an `OP_WRITE` command with an empty TX FIFO leaves the address
  phase armed (malformed command lists only; untested).
- USB-Serial-JTAG: where the `put` task spends its CPU time per 64-byte
  packet is not identified (a per-byte VFS read is the suspect); if it
  is an emulator gap, `PUT_CHUNK_GAP_STEPS` could shrink.
- `R_M5_3`'s closure follows direct calls only (indirect VFS, littlefs
  and cJSON pointers were not followed).
- ROM stubs: `strtol`/`atoi` do not set `errno`; the 8 MiB
  `MAX_STUB_MEMORY_BYTES` cap of the string stubs has no test.
- Unmapped accesses at the idle point: ASSIST_DEBUG (`0x600c_e0xx`) and
  RTC_CNTL `0x6000_80bc` (SYSTEM's clock registers are modeled since
  Task 2). Harmless so far.
- Early log timestamps have not been compared with the badge's.

### Build and tests

- The release suite's wall time is one chain (hacker's walk to Dice,
  about 487M steps); only a faster emulator shortens it.
- Test minors left from Milestone 6's reviews: `snake_eats` could assert
  growth of exactly one cell (`after * 3 == at_open * 4`) and that its
  150 ms frame is `SNAKE_OPEN_HASH`, and its `SNAKE_DOWN_SLOT` = 4
  duplicates `NAV_SLOT` without a cross-reference;
  `run_until_a_past_target_runs_nothing` uses target 0 (always past),
  where a mid-run target would test it properly; the
  `DEFAULT_STEPS_PER_FRAME` test restates the constant;
  `snake_moves_on_its_own`'s exact gap pin is as fine as `MOVE_SAMPLE_US`
  (2 ms).

### Carried from Milestones 3 and 4 (untouched)

- ST7789 `MADCTL` (`0x20`, then `0x60`) is unmodeled; orientation is
  human-confirmed for every pinned frame so far, Snake's included.
- GPIO0 (the display D/C pin) is routed as SPI2 `FSPIWP`/`FSPIHD`; routing
  is stored only.
- eFuse block unmodeled: boot logs chip revision v0.0, the badge is v0.4.
- Flagged ROM-stub guesses: `rom_i2c_readReg*` returning 0,
  `Cache_Get_*` returning 0 (notes, history item 5).
- Edge-type interrupts are simplified: `CPU_INT_TYPE_REG` and
  `CPU_INT_CLEAR_REG` are plain storage.
- GDMA: DMA spans that cross into an adjacent RAM region are rejected;
  the RX in-link walk, CPU FIFO push/pop, the `OUT_DSCR*` pre-fetch
  registers and transfer timing are not modeled; a trailing zero-length
  `suc_eof` descriptor is never visited; `RESTART` with nothing to
  restart leaves the channel active.
- `boot_until_console_contains` checks in 250,000-step chunks, so rung
  budgets are looser than they read; stable-frame steps are quantized
  the same way.
- Small duplications: the `memcpy`/`memset` and `memcmp`/`strncmp` stub
  arms, and the `header_with_speed_size` test helper in `image.rs` and
  `rom.rs`.

Resolved from the Milestone 6 backlog in Milestone 6: Snake (playable,
compared with the badge), the `CPU_FREQ_MHZ` = 160 stub, `TICKS_PER_STEP`
= 1 (one clock that follows the firmware's 40/80 MHz switches),
`PACKET_STEPS` (now `PACKET_TICKS`, bus time), SYSTEM's unmapped clock
registers, and the `stop()` flush of deferred releases.

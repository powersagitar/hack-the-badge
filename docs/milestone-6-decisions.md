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
- **R-T1-3, the WFI rung's metric.**
  `boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting`
  now measures `clock.idle_ticks()` (1,827,072 at step 5,550,000) instead
  of SYSTIMER's lead over the step count (1,827,060). The two differ by
  the 12 fast-forward steps: their whole jump is now charged as idle time,
  not one tick of it as a step. The `>= 1,500,000` threshold is
  unchanged. If wrong: the rung would no longer prove that the
  fast-forward is what put time ahead of the steps.
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
- **R-PF-2, the clock-init progress check.** `rom_stub_boot.rs`'s
  `rom_stubbed_boot_gets_past_the_mask_rom_wall` checked that boot hit
  the `ets_get_cpu_frequency` *stub*. Since Task 2 that address is guest
  ROM code, so no stub fires there; the check is kept, with the same
  meaning ("ets_get_cpu_frequency (clock init)"), as "a step executed at
  `ETS_GET_CPU_FREQUENCY`" (`StepInfo::pc_before`). If wrong: boot could
  stop reaching clock init without that test noticing.
- **R-T2-1, early log timestamps are timing.** The pre-scheduler log
  timestamp is the cycle counter divided by `ets_get_cpu_frequency() *
  1000`, so with the getter at 80 instead of 160 the `I (5)` prefixes
  read `I (10)` (through `spi_flash: flash io: dio`) and `I (11)` (from
  `sleep_gpio` through `main_task: Calling app_main()`); the
  post-scheduler ones move by 6 ms (`LVGL: Starting LVGL task` at 131,
  not 125). The rung needles that quoted `I (5)` are re-pinned to the
  new prefixes exactly, not loosened. With timestamps stripped, the
  console through step 6,000,000 is identical to `d5e6be3`'s except
  `cpu freq: 80000000 Hz` (was 160000000). If wrong: a needle could
  pass on a changed line.
- **R-T2-2, step docs that moved under passing rungs.** Two rungs whose
  windows still contain the event they describe kept their bounds but
  their docs now quote the new exact step (`spi_hal_init`'s poll,
  `__bswapsi2`'s call); the D10 rung, documented as "the step before
  FreeRTOS's first yield request", is re-pinned to that step (528,150 →
  528,258) so its meaning holds. Historical fault steps quoted as "past
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

`ets_update_cpu_frequency` is called with 40 at step 845, 40 at 400,752
and 80 at 404,393 (then 40/80 around every idle). Busy-waits that scale
cycles by `ets_get_cpu_frequency` now count 80 instead of 160 per
microsecond, so everything after the first such wait moves later. No
frame hash moved (every hash rung passed unchanged; the splash still has
2,340 colors). Old values are `d5e6be3`'s, measured with the same probe.

| Rung or doc (test file) | What | Old | New |
| --- | --- | --- | --- |
| `rom_stubbed_boot_gets_past_the_mask_rom_wall` doc (`rom_stub_boot`) | `ets_get_cpu_frequency` / `ets_printf` first call | ~400,695 / ~400,714 | ~400,698 / ~400,720 |
| `boot_reports_the_badges_80_mhz_cpu_clock` (new, `boot_progress`) | `cpu_start: cpu freq: 80000000 Hz` printed | 160000000 Hz at 407,291 | 80000000 Hz at 407,315 |
| `boot_no_longer_faults_at_the_pre_task_d6_qsort_call_site` | steps until `qsort` has returned | 408,740 | 408,791 |
| `first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line` | vPortYield's write / first trap | 528,151 / 528,152 | 528,259 / 528,260 |
| `boot_no_longer_faults_at_the_pre_task_d10_..._call_site` | the step before the first yield | 528,150 | 528,258 |
| `boot_escapes_the_pre_task_9_spi_update_poll_...` doc | `spi_hal_init` poll first reached | 583,976 | 584,379 |
| `boot_no_longer_faults_at_the_pre_task_d12_bswapsi2_call_site` doc | `__bswapsi2` entered | 593,005 | 594,963 |
| `boot_stubs_reach_load_partitions_md5init_...` (`rom_stub_boot`) phase 1 | through step | 528,151 | 528,259 |
| same, phase 3 | `spi_hal_init` poll | 583,976 | 584,379 |
| same, phase 4 / 5 | `__bswapsi2` call / stub step | 593,004 / 593,005 | 594,962 / 594,963 |
| same, phase 6 / 7 / 8 | idle `wfi` reached / retires / fast-forward | 596,595 / 596,596 / 596,597 | 598,556 / 598,557 / 598,558 |
| same, phase 8 | fast-forward jump to the FreeRTOS tick | 91,455 ticks | 89,602 ticks |
| same, phase 9 / 10 | through step / `MD5Init` step | 5,558,962 / 5,558,963 | 5,560,992 / 5,560,993 |
| `boot_idles_in_wfi_...` doc | `idle_ticks` at step 5,550,000 | 1,827,072 | 1,825,153 |
| 12 needles in 10 `boot_progress` rungs, 3 in 2 `rom_stub_boot` tests | early log prefix (R-T2-1) | `I (5)` | `I (10)` heap_init, spi_flash; `I (11)` sleep_gpio, main_task |

Console lines (timestamps stripped) are unchanged except the `cpu freq`
line. Other first-appearance steps, for reference (old → new):
`cpu_start: Unicore app` 402,300 → 402,312; `heap_init` first line
409,336 → 409,390; RTCRAM line 415,325 → 415,391; `detected chip:
generic` 446,695 → 446,764; `sleep_gpio: Enable ...` 484,091 →
484,169; `main_task: Calling app_main()` 555,165 → 555,568; `LVGL:
Starting LVGL task` 669,268 → 671,298 (`I (125)` → `I (131)`).

## Milestone 7 backlog

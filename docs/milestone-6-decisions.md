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

## Milestone 7 backlog

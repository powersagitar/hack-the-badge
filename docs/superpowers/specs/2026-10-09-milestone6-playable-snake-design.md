# Milestone 6 — A calibrated clock and playable Snake

Status: design approved in conversation (2026-10-09); written spec awaiting
review. Implementation plan: to be written after spec approval.

## Goal

Make the built-in Snake app playable in the real-firmware emulator, natively
and in the browser, at roughly the physical badge's speed. Snake launches
today (Milestone 5, R-T9-3) but its game timer outruns rendering: the snake
appears at the right wall and "Game over / A to restart" follows about 2M
steps later with no input. The root cause is the emulator's clock, not
Snake: SYSTIMER advances one 16 MHz tick per instruction
(`TICKS_PER_STEP = 1`), so the CPU gets one cycle per timer tick where the
physical badge (80 MHz CPU, per its boot log; the notes' "Ground truth from
the physical badge") gets five. Measured against SYSTIMER, the emulated CPU
is at least 5x too slow, so timer-paced game logic outruns LVGL's rendering.

Milestone 6 replaces that ratio with one calibrated clock used everywhere,
proves Snake against the physical badge (frames and speed), and paces the
browser to wall-clock time.

### Success criteria

1. A chip-agnostic clock model drives all emulated time; `TICKS_PER_STEP`
   is gone. The ESP32-C3 configuration is an 80 MHz CPU against the 16 MHz
   SYSTIMER at one cycle per step (CPI 1) unless calibration says
   otherwise.
2. Every existing `boot_progress` rung passes with re-pinned step budgets.
   Every frame hash that moved is listed for the human; none is silently
   re-pinned.
3. New rungs pass: `launcher_opens_snake`, `snake_moves_on_its_own` (move
   interval within ±25% of the badge's measured speed),
   `snake_steers`, `snake_hits_wall_game_over` (A restarts). Stretch:
   `snake_eats`.
4. The Snake start, after-a-turn and game-over frames are compared with
   the physical badge by the human. The comparison is structural (layout,
   snake, walls, text), because the badge's food position comes from its
   real RNG and the emulator's from the fixed-seed xorshift.
5. In the browser, emulated time follows wall-clock time (capped by a
   per-frame step budget), a speed readout shows the achieved ratio, and
   the human plays Snake to game over.

### Out of scope (stays in the backlog)

Console pane, browser-supplied RNG seed, `DeviceMotionEvent` input, RMT/LED
decode, NFC, other built-in apps, a per-instruction-class cycle model, and
the other Milestone 6 backlog items in `docs/milestone-5-decisions.md` not
named here.

## Design

### Part 1 — The clock model (`emulator-core`)

A new chip-agnostic module, `emulator-core/src/clock.rs`, in the same
spirit as `cpu/`: it knows ratios, and chip setup supplies the
ESP32-C3 numbers.

- `Clock { cpu_hz, timer_hz, cycles_per_step, remainder }`. ESP32-C3:
  `cpu_hz = 80_000_000`, `timer_hz = 16_000_000`, `cycles_per_step = 1`.
- Each step adds `cycles_per_step` CPU cycles. Cycles convert to whole
  SYSTIMER ticks as `cycles * timer_hz / cpu_hz` with the remainder
  carried, so there is no drift. At the ESP32-C3 values that is exactly
  one tick every five steps.
- `cycles_per_step` is the only calibration knob. If calibration (Part 2)
  needs a non-integer value it becomes a rational
  (numerator/denominator); it never becomes a per-instruction table in
  this milestone.
- The clock tracks total emulated time, exposed as
  `FirmwareRuntime::emulated_time_us()`.

Integration, all through the clock:

- `FirmwareBus::tick_peripherals`, `advance_idle` and
  `ticks_until_next_event` (`emulator-core/src/mem/bus.rs`) take time from
  the clock instead of `TICKS_PER_STEP`. The WFI fast-forward in
  `boot::step_with_interrupts` computes the ticks to the next event and
  charges the matching time to the clock, so steps, cycles and ticks stay
  consistent. `TICKS_PER_STEP` is deleted.
- The ROM `CPU_FREQ_MHZ` stub returns `cpu_hz / 1_000_000` (80, was 160).
- The `mpccr` cycle counter (CSR `0x7e2`) counts the clock's cycles: still
  one per working step at CPI 1, and still not counting idle fast-forward
  time, as today.
- USB-Serial-JTAG `PACKET_STEPS` is derived from 51.3 µs at the clock rate
  (about 4,105 steps at 80 MHz, was 8,210). `PUT_CHUNK_GAP_STEPS` (a host
  property) is re-measured under the new clock, not just scaled.
- `SysTimer`'s own unit tests keep testing ticks; the step-to-tick
  conversion is tested in `clock.rs`.

Expected cost: timer-driven waits now take 5x the steps unless they idle in
WFI (fast-forwarded, nearly free). Every rung's budget is re-pinned in one
task. Pinned frame hashes should mostly survive, but frames whose content
depends on timing (animations, the Dice roll's timing) may move; each moved
frame is listed for the human. If the release suite gets much slower than
about 42 s, the figure goes in the notes and the opt-level-3 backlog item
becomes the remedy. The step numbers in the notes' stall history stay as
historical, labelled pre-Milestone 6; the current-state paragraphs (notes,
`CLAUDE.md`) get the new numbers.

### Part 2 — Snake rungs and the hardware check

The Snake rungs branch from hacker's shared launcher checkpoint (the
existing `OnceLock` fixture), so they add little suite time. The walk from
the launcher is DOWN×3, RIGHT×2, A (Milestone 5 Task 9).

1. `launcher_opens_snake`: the first stable Snake frame is pinned and
   compared with the badge.
2. `snake_moves_on_its_own`: with no input, the head advances. The rung
   measures the emulated time between moves, detected as framebuffer
   changes in the play area and timed with `emulated_time_us()`, and
   asserts it is within ±25% of the badge's measured move interval. This
   rung fails on the pre-Milestone 6 clock and is the regression guard.
3. `snake_steers`: a direction press makes the snake turn on the next
   move; the frame after the turn is pinned.
4. `snake_hits_wall_game_over`: with no input, the snake reaches "Game
   over / A to restart" after the expected number of moves; the frame is
   pinned and compared. A then restarts the game.
5. Stretch, `snake_eats`: the fixed-seed RNG makes the food position
   deterministic, so a scripted input sequence eats one food item and the
   snake grows. Dropped if it costs too much.

**Hardware gate** (one human session mid-milestone, like Milestone 5 Task
9): a short phone video from launching Snake through an untouched run into
the wall, from which the human gives moves per second (or the video is put
under `local/` and frames are counted from it); and photos or screenshots
of the start, after-one-turn and game-over screens. Nothing from this
session is committed except the measured speed figure and the human's
verdict on each frame.

**Calibration**: if the measured interval at CPI 1 falls outside ±25%, one
calibration task sets `cycles_per_step` once and records the measurement
and the reasoning in `docs/milestone-6-decisions.md`.

Anything else Snake needs (a peripheral, a ROM stub) follows the usual
path: trace it, ledger ruling, minimal model.

### Part 3 — Real-time pacing in the browser

A new core API, `FirmwareRuntime::run_until(target_time_us, max_steps)`,
runs until emulated time reaches the target or the step cap, whichever
comes first, and reports the steps used and whether the cap was hit. A
step count alone can't pace, because WFI fast-forward makes steps and time
diverge. `emulator-wasm` gets a one-line passthrough.

The frame loop in `frontend/src/runtime/firmware-runtime.ts`:

- Each `requestAnimationFrame` advances emulated time by the wall-clock
  delta since the last frame, clamped to 50 ms so a backgrounded tab
  doesn't return to a catch-up burst.
- A per-frame step budget, 1,000,000 to start (a documented guess, tuned
  by measurement), replaces `DEFAULT_CYCLES_PER_FRAME` as the cap.
- When the cap is hit, the time debt is dropped, not carried: the emulator
  runs in slow motion rather than bursting.
- The firmware status line shows the achieved speed ratio ("1.0x",
  "0.4x").
- `PRESS_HOLD_STEPS` (1.6M steps, a value from the old 1:1 clock) becomes
  a hold in emulated time, 20 ms (1.6M steps at 80 MHz), so a press means
  the same at any pacing. The provisioner's chunk gap stays in steps (a
  host pacing property), taking Part 1's re-measured value.
- Deferred button releases are flushed in `stop()` (a Milestone 5 Task 7
  review minor in this same loop, which pacing makes easier to hit).

### Part 4 — Docs

`docs/firmware-emulator-notes.md` (current state, a timing-model section,
known limitations), `CLAUDE.md` (current state, clock and API changes,
suite timing), and a new `docs/milestone-6-decisions.md` with this
milestone's rulings and the Milestone 7 backlog (the Milestone 5 backlog
items not done here carried forward).

## Task order

1. Clock module and unit tests; no behaviour change yet (the old 1:1
   ratio must be expressible, and the suite passes unchanged).
2. Switch to 80 MHz: `CPU_FREQ_MHZ`, `mpccr`, `PACKET_STEPS`, re-measured
   chunk gap; re-pin every rung; list frame-hash changes for the human.
3. `launcher_opens_snake` and `snake_hits_wall_game_over`, pinned
   provisionally.
4. Human gate: badge video and photos; calibrate CPI if needed; compare
   the pinned frames.
5. `snake_moves_on_its_own` (speed tolerance) and `snake_steers`.
6. `run_until` and its WASM passthrough.
7. Browser pacing, time-based holds, the `stop()` flush.
8. Stretch: `snake_eats`.
9. Docs, then the final whole-branch review.

## Testing strategy

- `clock.rs` unit tests: tick conversion and remainder carry, fast-forward
  charging, CPI as a rational, the frequency the ROM stub reports.
- `cargo test -p emulator-core --release --test boot_progress`: every
  existing rung re-pinned plus the Snake rungs; the debug
  `cargo test --workspace` runs them too.
- Frontend: unit tests of the pacing arithmetic with a fake handle and a
  fake wall clock (the clamp, the dropped debt, the speed ratio, the
  time-based hold); the WASM twin (`test/cpu-wasm.test.ts`) gains a
  `run_until` check; `bun run typecheck`.
- Human: the badge comparison (Task 4) and playing Snake in a real browser
  (closing check).

## Risks

- Some boot phase may busy-wait on SYSTIMER instead of idling in WFI,
  making it 5x more expensive in steps. Task 2 measures this; the response
  is to accept the cost or turn on `opt-level = 3` (a backlog item with its
  own WASM size and browser-speed check).
- Snake may need something beyond timing. It follows the ruling path in
  Part 2.
- The badge's real speed may not match CPI 1 (flash cache misses, CPI > 1).
  Calibration is designed in.

## Execution model

As in Milestone 5: a worktree on branch `milestone-6`, subagent-driven
tasks with an SDD ledger at `local/m6-sdd-ledger.md` (gitignored), human
gates where noted, and merging and pushing left to the human.

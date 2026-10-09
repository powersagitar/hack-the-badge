# Milestone 6 — A Calibrated Clock and Playable Snake — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task (execution method chosen by the human partner on 2026-10-09; see "Orchestrator notes" at the end). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the emulator's one-SYSTIMER-tick-per-instruction timing with a clock that follows the firmware's own CPU frequency (40/80 MHz DFS against the 16 MHz SYSTIMER), then prove Snake playable against the physical badge (frames and speed) natively, and pace the browser to wall-clock time.

**Architecture:** A chip-agnostic `emulator-core/src/clock.rs` converts executed steps into SYSTIMER ticks at a CPU frequency that the bus updates once per step from a register model of SYSTEM's clock registers; the WFI fast-forward charges idle ticks to the same clock. `FirmwareRuntime` gains `emulated_time_us()` and `run_until(target_us, max_steps)`; human-scale durations (button holds, shakes) become emulated time everywhere. Snake rungs branch from the hacker launcher checkpoint; a human gate supplies the badge's move interval and frames. The browser frame loop advances emulated time by wall-clock time under a per-frame step cap.

**Tech Stack:** Rust 2021 (`emulator-core`, `emulator-wasm` via wasm-bindgen/wasm-pack), Bun + TypeScript frontend, ESP-IDF v5.5.3 sources as the register authority, `esp32c3_rev3_rom.elf` (esp-rom-elfs 20241011, local-only) for ROM facts.

**Spec:** `docs/superpowers/specs/2026-10-09-milestone6-playable-snake-design.md` — read it first, including "Found while planning". Also read `docs/milestone-5-decisions.md` (its Milestone 6 backlog is this milestone's input; its decisions stand) and `docs/firmware-emulator-notes.md` "Known limitations", "Ground truth from the physical badge" and "Data-handling note".

## Handoff state (as of 2026-10-09)

- `main` @ the spec commit (on top of `3af5bba`, M5 merge; `origin/main` = `3af5bba`). Create branch `milestone-6` and the worktree `.claude/worktrees/milestone-6` (`git worktree add .claude/worktrees/milestone-6 -b milestone-6`); copy the repo-root `local/` into it (gitignored; never commit it). Build the WASM package in the worktree before `bun test` (`bun install` in `frontend/`, then `bun run build:wasm`).
- Baseline to confirm in Task 1 Step 1: `cargo test --workspace`, `cargo test -p emulator-core --release --test boot_progress` (about 42 s), `bun run build:wasm`, `bun test` (about 40 s), `bun run typecheck`, `cargo clippy --workspace --all-targets` (record the pre-existing warning count in the ledger).
- Measured while planning (throwaway probe, deleted): ROM `ets_update_cpu_frequency` (`0x4000_0588`, argument = MHz) is called with 40 at step 844, 80 at step 404,380, then 40/80 around every idle (about 600 steps at 40, 2,600+ at 80); 1,419 calls each way in 30M steps (blank flash + START self-test); never 160. The console prints `cpu_start: cpu freq: 160000000 Hz` only because `rom.rs`'s `CPU_FREQ_MHZ` stub returns 160; the badge's boot log line is `cpu freq: 80000000 Hz`.
- ROM facts (rev3 ELF): `ets_update_cpu_frequency` body `0x400462e8` is `lui a5,0x3fcdf; sw a0,0x64c(a5); ret`; `ets_get_cpu_frequency` body `0x400462f2` is `lui a5,0x3fcdf; lw a0,0x64c(a5); ret`. The variable is `s_ticks_per_us` at `0x3fcd_f64c` (section `.data_ets_delay`, initial value `0x28` = 40).
- ESP-IDF v5.5.3 facts (fetched to the scratchpad while planning; re-fetch to cite): `system_reg.h` — `SYSTEM_CPU_PER_CONF_REG` = base + `0x008`: `CPU_WAITI_DELAY_NUM` [7:4] R/W default 0, `CPU_WAIT_MODE_FORCE_ON` [3] R/W default 1, `PLL_FREQ_SEL` [2] R/W default 1, `CPUPERIOD_SEL` [1:0] R/W default 0. `SYSTEM_SYSCLK_CONF_REG` = base + `0x058`: `CLK_DIV_EN` [19] RO, `CLK_XTAL_FREQ` [18:12] RO, `SOC_CLK_SEL` [11:10] R/W default 0, `PRE_DIV_CNT` [9:0] R/W default 1. `rtc_clk.c` `rtc_clk_cpu_freq_get_config`: XTAL → `xtal_mhz / (PRE_DIV_CNT + 1)`; PLL → `CPUPERIOD_SEL` 0 = 80, 1 = 160 (else abort); RC_FAST → 20; else abort. `clk_tree_ll.h` `clk_ll_cpu_get_src`: `SOC_CLK_SEL` 0 XTAL, 1 PLL, 2 RC_FAST, else invalid.
- `local/` (gitignored) holds `full_flash_dump.bin` and `boot_log.txt` (personal data: never commit, quote or print values; never paste into a subagent brief; the one fact this plan uses, `cpu freq: 80000000 Hz`, is already in the committed notes as "CPU at 80 MHz"), `local/.venv/` (the only Python), `local/rom-elfs/`, earlier ledgers.
- Orchestrator ledger: `local/m6-sdd-ledger.md` (gitignored), one line per task/review/fix round/ruling, with step counts and hashes.
- ESP-IDF sources: fetch from `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>` into the session scratchpad, never into the repo.
- The shell's `grep` may be a wrapper function that silently returns nothing on some searches; use `/usr/bin/grep` (or `rg`) when a search comes back empty unexpectedly.

## Global Constraints

- Register-faithful: every modeled register behavior cites its ESP-IDF v5.5.3 header / HAL / LL source in the module doc comment. Never guess an offset or bit.
- No PC-based interception of ESP-IDF (non-ROM) code. ROM functions only, via `src/rom.rs` + `cpu/rom_stubs.rs` or guest-executed ROM code blobs, addresses from `components/esp_rom/esp32c3/ld/esp32c3.rom*.ld` and the rev3 ROM ELF.
- `emulator-core/src/clock.rs` and `emulator-core/src/cpu/` stay chip-agnostic: no ESP32-C3 constant in either; chip numbers live in `mem/soc.rs`, `peripherals/`, `rom.rs`.
- One clock: every emulated-time consumer (SYSTIMER, USB-Serial-JTAG packet timing, `emulated_time_us`, `run_until`) reads the same `Clock`. `TICKS_PER_STEP` and `PACKET_STEPS` are gone by the end of Task 3.
- Finish-line tests are never narrowed or loosened. A frame hash changes only with a documented reason, and every moved hash is listed for the human (spec success criterion 2); a hash compared with the physical badge that moves needs the human's OK before it is re-pinned.
- Step-exact rungs are re-pinned exactly (Milestone 5 ruling R-T9-4); budgets are margins, re-measured and noted.
- `FirmwareBus` dispatch stays an ordered sequence of concrete named fields; no `dyn` dispatch. `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays logic-free (one-line passthroughs; numeric casts only).
- Unmapped data access never panics (reads 0, writes drop, logged); unmapped instruction fetch always traps. Browser-supplied input never panics the emulator. An invalid clock-register encoding never panics and never sets a zero frequency.
- Never commit, quote or print values from `local/full_flash_dump.bin`, `local/boot_log.txt` or anything derived from them. Committed tests depend only on `factory.bin` and committed synthetic data. Badge photos/videos from Task 5 stay under `local/`; only the human's measured numbers and verdicts are committed.
- `docs/milestone-3-decisions.md`, `docs/milestone-4-decisions.md` and `docs/milestone-5-decisions.md` stand; overriding one needs the human partner's OK and a recorded reason. Milestone 6 rulings go in a new `docs/milestone-6-decisions.md` (created in Task 1).
- Python via `local/.venv/bin/python` only. JS/TS via Bun only.
- After any change under `emulator-core/` or `emulator-wasm/`, before commit: `cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck`, all green, no new clippy warnings. `cargo fmt --all` before each commit.
- Pushing and merging are the human partner's call. Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A frequency change mid-run must not lose or invent time.** The sub-tick remainder is rescaled on a CPU-frequency change; dropping it would make each of the ~3,000 DFS switches per 30M steps lose up to a tick, and a naive reset would drift SYSTIMER against the CPU. Pinned by Task 1's `frequency_change_keeps_the_fractional_tick`.
2. **A step that produces zero ticks must not fire or re-fire SYSTIMER alarms.** At 80 MHz four of five steps advance zero ticks; calling `advance_by(0)` must be skipped or be a no-op. Pinned by Task 1's `zero_tick_steps_leave_the_systimer_untouched` and Task 3's `bus_steps_at_80_mhz_tick_once_every_five_steps`.
3. **The byte-split halves of one clock-register write must never become the clock's frequency.** A `sw` reaches SYSTEM as four byte writes; the clock samples the derived frequency once per step, after the instruction. Pinned by Task 3's `clock_follows_the_cpu_clock_registers_after_the_step`.
4. **A garbage clock-register value (`SOC_CLK_SEL` = 3, `CPUPERIOD_SEL` = 3) must not panic or divide by zero.** Pinned by Task 2's `invalid_clock_encodings_keep_the_previous_frequency`.
5. **A backgrounded tab must not come back to a catch-up burst, and a slow machine must not freeze the tab.** The wall delta is clamped to 50 ms and the step cap bounds each frame; debt is dropped. Pinned by Task 7's `frame target clamps the wall delta` and `a capped frame drops its debt`.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `emulator-core/src/clock.rs` | Create (Task 1) | Chip-agnostic step→tick clock with a variable CPU frequency |
| `emulator-core/src/lib.rs` | Modify (Task 1) | `pub mod clock;` |
| `emulator-core/src/mem/soc.rs` | Modify (Tasks 1, 2) | `SYSTIMER_HZ`, `XTAL_HZ`, SYSTEM clock register offsets |
| `emulator-core/src/mem/bus.rs` | Modify (Tasks 1, 2, 3) | `clock` field; time advance through the clock; SYSTEM clock-register routing; per-step frequency sync |
| `emulator-core/src/boot.rs` | Modify (Task 1) | Fast-forward through `Option` ticks; `None` = an ordinary step's time |
| `emulator-core/src/runtime.rs` | Modify (Tasks 1, 3) | `emulated_time_us`, `run_until`; WFI test numbers |
| `emulator-core/src/peripherals/systimer.rs` | Modify (Tasks 1, 3) | Drop `advance()`, then `TICKS_PER_STEP`; tick-rate doc |
| `emulator-core/src/peripherals/system.rs` | Modify (Task 2) | `CPU_PER_CONF`/`SYSCLK_CONF` storage and the derived CPU frequency |
| `emulator-core/src/rom.rs` | Modify (Task 2) | Stateful `ets_*_cpu_frequency` as ROM code blobs; `s_ticks_per_us` and `SYSCLK_CONF` boot initializers; delete `CPU_FREQ_MHZ` |
| `emulator-core/src/peripherals/usb_serial_jtag.rs` | Modify (Task 3) | `PACKET_TICKS` = 821 from bus time; delete `PACKET_STEPS` |
| `emulator-core/src/cpu/registers.rs`, `peripherals/rtc_cntl.rs` | Modify (Task 3) | Docs that cite `TICKS_PER_STEP` |
| `emulator-core/tests/boot_progress.rs` | Modify (Tasks 1–6, 8) | Idle rung via `idle_ticks`; cpu-freq rung; time-based holds; re-pins; Snake rungs |
| `emulator-core/tests/common/provision.rs` | Modify (Task 3) | Re-measured `PUT_CHUNK_GAP_STEPS` |
| `emulator-core/tests/rom_stub_boot.rs` | Modify (Tasks 2, 3) | Step-exact re-pins |
| `emulator-wasm/src/lib.rs` | Modify (Task 1) | `runUntil`, `emulatedTimeUs` passthroughs |
| `frontend/src/cpu/bridge.ts` | Modify (Task 1) | Handle gains `runUntil`, `emulatedTimeUs` |
| `frontend/test/cpu-wasm.test.ts` | Modify (Tasks 1, 3) | `runUntil` check; time-based twin; re-pinned hashes |
| `frontend/src/runtime/firmware-runtime.ts` | Modify (Tasks 3, 7) | Time-based hold/shake (3); wall-clock pacing, speed, `stop()` flush (7) |
| `frontend/src/runtime/provisioner.ts`, `frontend/test/provisioner.test.ts` | Modify (Task 3) | Re-measured chunk gap |
| `frontend/test/firmware-runtime.test.ts` | Modify (Tasks 1, 3, 7) | Fake handle; time-based hold/shake; pacing |
| `frontend/src/main.ts` | Modify (Task 7) | Speed readout in the status line |
| `docs/milestone-6-decisions.md` | Create (Task 1), grow per task | Rulings, measurements, M7 backlog |
| `docs/firmware-emulator-notes.md`, `CLAUDE.md` | Modify (Task 9; each task adds its history entry to the notes) | Timing model, current state |

---

### Task 1: The clock, emulated time and `run_until` (no behaviour change)

**Files:**
- Create: `emulator-core/src/clock.rs`, `docs/milestone-6-decisions.md`
- Modify: `emulator-core/src/lib.rs`, `emulator-core/src/mem/soc.rs`, `emulator-core/src/mem/bus.rs` (`tick_peripherals` ~L572, `ticks_until_next_event` ~L580, `advance_idle` ~L593, struct fields ~L320–380, constructor ~L460), `emulator-core/src/boot.rs` (`step_with_interrupts` ~L265), `emulator-core/src/runtime.rs` (`run`/`run_impl` ~L143–193), `emulator-core/src/peripherals/systimer.rs` (`advance()` ~L318), `emulator-core/tests/boot_progress.rs` (`boot_idles_in_wfi_...` ~L521), `emulator-wasm/src/lib.rs`, `frontend/src/cpu/bridge.ts`, `frontend/test/firmware-runtime.test.ts` (fake handle), `frontend/test/cpu-wasm.test.ts`

**Interfaces:**
- Produces (`emulator_core::clock`):
  - `pub struct Clock` (`Clone`, `Debug`)
  - `Clock::new(cpu_hz: u64, timer_hz: u64, cycles_per_step: u64) -> Clock` (panics on a zero argument: chip setup passes constants)
  - `fn cpu_hz(&self) -> u64`, `fn timer_hz(&self) -> u64`
  - `fn set_cpu_hz(&mut self, hz: u64)` (0 ignored; rescales the remainder)
  - `fn step(&mut self) -> u64` (ticks this step produced)
  - `fn advance_idle(&mut self, ticks: u64)`
  - `fn total_ticks(&self) -> u64`, `fn idle_ticks(&self) -> u64`
  - `fn elapsed_us(&self) -> u64`, `fn ticks_at_us(&self, us: u64) -> u64` (first tick count whose time is ≥ `us`)
- Produces (`mem::soc`): `pub const SYSTIMER_HZ: u64 = 16_000_000;`
- Produces (`FirmwareBus`): `pub clock: Clock`; `ticks_until_next_event(&self) -> Option<u64>`
- Produces (`FirmwareRuntime`): `fn emulated_time_us(&self) -> u64`; `fn run_until(&mut self, target_time_us: u64, max_steps: u32) -> RunSummary` (`steps` = steps actually run)
- Produces (WASM/TS): `FirmwareEmulator.runUntil(targetUs: f64, maxSteps: u32) -> RunReport`, getter `emulatedTimeUs: f64`; `FirmwareEmulatorHandle.runUntil(targetUs: number, maxSteps: number): CpuRunSummary`, `.emulatedTimeUs(): number`

- [ ] **Step 1: Baseline.** Run every command in the Global Constraints check list at the branch base; record pass/fail, the release `boot_progress` wall time and the clippy warning count in `local/m6-sdd-ledger.md`.

- [ ] **Step 2: Write the failing clock tests.** Create `emulator-core/src/clock.rs` with only the module doc, `#[cfg(test)] mod tests` below, and `pub struct Clock;` placeholder so it compiles to failures. Add `pub mod clock;` to `src/lib.rs`.

```rust
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
        // Three steps at 80 MHz = 3/5 tick pending. At 40 MHz that is
        // 3/5 tick = 1.5 steps' worth: two more steps reach one tick.
        let mut c = Clock::new(80_000_000, TIMER, 1);
        for _ in 0..3 {
            assert_eq!(c.step(), 0);
        }
        c.set_cpu_hz(40_000_000);
        assert_eq!(c.step(), 0, "3/5 + 2/5 = 1 tick only after the next step");
        assert_eq!(c.step(), 1);
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
    }

    #[test]
    fn cycles_per_step_scales_the_rate() {
        let mut c = Clock::new(80_000_000, TIMER, 5);
        assert_eq!(c.step(), 1, "five cycles at 80 MHz = one 16 MHz tick");
    }
}
```

- [ ] **Step 3: Run them to verify they fail.** Run: `cargo test -p emulator-core --lib clock::` — Expected: compile errors (`no function or associated item named new`).

- [ ] **Step 4: Implement `Clock`.** Replace the placeholder with:

```rust
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

    /// Emulated time in whole microseconds.
    pub fn elapsed_us(&self) -> u64 {
        (u128::from(self.total_ticks) * 1_000_000 / u128::from(self.timer_hz)) as u64
    }

    /// The first tick count whose time is at least `us` microseconds.
    pub fn ticks_at_us(&self, us: u64) -> u64 {
        (u128::from(us) * u128::from(self.timer_hz)).div_ceil(1_000_000) as u64
    }
}
```

- [ ] **Step 5: Run the clock tests.** Run: `cargo test -p emulator-core --lib clock::` — Expected: 8 passed.

- [ ] **Step 6: Wire the clock into the bus at the legacy ratio.** In `mem/soc.rs` add:

```rust
/// SYSTIMER's counting rate: a fixed 16 MHz on the ESP32-C3 (TRM v1.4
/// section 10.2; `systimer_ll.h`'s `SYSTIMER_LL_TICKS_PER_US` is 16).
pub const SYSTIMER_HZ: u64 = 16_000_000;
```

Check the `SYSTIMER_LL_TICKS_PER_US`/TRM citation against ESP-IDF v5.5.3 (`components/hal/esp32c3/include/hal/systimer_ll.h`) before committing; cite what the header actually says.

In `mem/bus.rs`: add a field `pub clock: Clock` (doc: "Emulated time. Until Milestone 6 Task 3 the CPU frequency equals `SYSTIMER_HZ`, i.e. one tick per step, the pre-Milestone-6 timing."), initialize it in the constructor with `Clock::new(SYSTIMER_HZ, SYSTIMER_HZ, 1)`, and replace the three time methods:

```rust
    /// Advances emulated time by one executed step and every timed
    /// peripheral by the ticks that step completed (none on most steps
    /// once the CPU runs faster than SYSTIMER). Call exactly once per
    /// `Cpu::step()` that is not an idle fast-forward -- see
    /// `crate::boot::step_with_interrupts`.
    pub fn tick_peripherals(&mut self) {
        let ticks = self.clock.step();
        if ticks > 0 {
            self.systimer.advance_by(ticks);
            self.usb_serial_jtag.advance(ticks);
        }
    }

    /// How far an idle (WFI) core may fast-forward: the ticks to the next
    /// SYSTIMER alarm or USB host packet, whichever is first, or `None`
    /// when neither is pending.
    pub fn ticks_until_next_event(&self) -> Option<u64> {
        [
            self.systimer.ticks_until_next_alarm(),
            self.usb_serial_jtag.ticks_until_next_packet(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Advances emulated time and every timed peripheral by `ticks` during
    /// an idle fast-forward.
    pub fn advance_idle(&mut self, ticks: u64) {
        self.clock.advance_idle(ticks);
        self.systimer.advance_by(ticks);
        self.usb_serial_jtag.advance(ticks);
    }
```

Update the bus unit test near L1869 that asserted `ticks_until_next_event() == TICKS_PER_STEP` with nothing pending: it now asserts `None`. Remove `TICKS_PER_STEP` from the bus's imports. Add (Review Focus 2):

```rust
    #[test]
    fn zero_tick_steps_leave_the_systimer_untouched() {
        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.clock = Clock::new(80_000_000, SYSTIMER_HZ, 1);
        for _ in 0..4 {
            bus.tick_peripherals();
            assert_eq!(bus.systimer.elapsed_ticks(), 0);
        }
        bus.tick_peripherals();
        assert_eq!(bus.systimer.elapsed_ticks(), 1);
    }
```

(Task 3 adds a per-step frequency sync that would overwrite this hand-set clock from the SYSTEM registers; Task 3 Step 1 moves this test onto its `xtal_bus()` helper with the PLL selected.)

- [ ] **Step 7: Fast-forward through the `Option`.** In `boot.rs` `step_with_interrupts`, replace the fast-forward block:

```rust
    if fast_forward {
        match bus.ticks_until_next_event() {
            // At least one tick, as before: an event already due still
            // costs a tick of time.
            Some(ticks) => bus.advance_idle(ticks.max(1)),
            // Nothing armed: an idle step passes an ordinary step's time.
            None => bus.tick_peripherals(),
        }
        lines = bus.asserted_lines();
    }
```

Update the doc paragraph above it ("or one ordinary step's worth if none is pending" stays true). Delete `SysTimer::advance()` (now unused) and fix its doc references; keep `TICKS_PER_STEP` until Task 3 (USB still uses it).

- [ ] **Step 8: Write the failing runtime tests.** In `runtime.rs`'s tests module:

```rust
    #[test]
    fn emulated_time_starts_at_zero_and_follows_the_clock() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        assert_eq!(rt.emulated_time_us(), 0);
        rt.run(1_600);
        // Legacy ratio (until Task 3): one 16 MHz tick per step.
        assert_eq!(rt.emulated_time_us(), 100);
    }

    #[test]
    fn run_until_stops_at_the_target_time() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let s = rt.run_until(1_000, 1_000_000);
        assert!(rt.emulated_time_us() >= 1_000);
        assert_eq!(u64::from(s.steps), rt.total_steps());
        assert!(s.steps < 1_000_000);
    }

    #[test]
    fn run_until_stops_at_the_step_cap() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let s = rt.run_until(u64::MAX / 2, 500);
        assert_eq!(s.steps, 500);
        assert_eq!(rt.total_steps(), 500);
    }

    #[test]
    fn run_until_a_past_target_runs_nothing() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(10_000);
        let s = rt.run_until(0, 1_000);
        assert_eq!(s.steps, 0);
        assert_eq!(rt.total_steps(), 10_000);
    }
```

Run: `cargo test -p emulator-core --lib runtime::` — Expected: compile errors (no `emulated_time_us`/`run_until`).

- [ ] **Step 9: Implement them.** In `runtime.rs`, give `run_impl` an optional stop time and count the steps actually run:

```rust
    /// Runs until emulated time reaches `target_time_us` or `max_steps`
    /// steps have run, whichever is first; `RunSummary::steps` is the
    /// number run (0 if the target is already past). The browser's frame
    /// pacing and the time-based button holds use this (Milestone 6).
    pub fn run_until(&mut self, target_time_us: u64, max_steps: u32) -> RunSummary {
        let target_ticks = self.bus.clock.ticks_at_us(target_time_us);
        self.run_impl(max_steps, Some(target_ticks), None)
    }

    /// Emulated time since construction or the last reset, in microseconds.
    pub fn emulated_time_us(&self) -> u64 {
        self.bus.clock.elapsed_us()
    }
```

and in `run_impl(&mut self, budget: u32, stop_at_ticks: Option<u64>, mut pc_hist: ...)`:

```rust
        let mut summary = RunSummary::default();
        let mut ran: u32 = 0;
        while ran < budget {
            if stop_at_ticks.is_some_and(|t| self.bus.clock.total_ticks() >= t) {
                break;
            }
            // ... existing per-step body unchanged ...
            ran += 1;
        }
        summary.steps = ran;
        self.total_steps += u64::from(ran);
```

`run` and `run_traced` pass `None`. Confirm `reset()` rebuilds the bus (so the clock restarts at 0); if it does not, reset the clock explicitly and add `reset_restarts_emulated_time` to the tests.

- [ ] **Step 10: Convert the idle rung to `idle_ticks`.** In `tests/boot_progress.rs` `boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting`, replace the `elapsed_ticks() - total_steps()` computation with:

```rust
    let idle = rt.bus().clock.idle_ticks();
    assert!(idle >= 1_500_000, "idle fast-forward added only {idle} ticks");
```

(Identical value at the legacy ratio. Update the rung's doc: "the idle fast-forward charged at least 1,500,000 ticks".)

- [ ] **Step 11: Run the whole Rust suite.** Run: `cargo test --workspace && cargo test -p emulator-core --release --test boot_progress` — Expected: all pass with **no** re-pinned number or hash (legacy ratio, identical timing). If anything moved, stop: the integration is not equivalent; find out why before going on.

- [ ] **Step 12: WASM passthroughs.** In `emulator-wasm/src/lib.rs`, inside `impl FirmwareEmulator`:

```rust
    /// Runs until emulated time reaches `target_us` or `max_steps` steps.
    #[wasm_bindgen(js_name = runUntil)]
    pub fn run_until(&mut self, target_us: f64, max_steps: u32) -> RunReport {
        RunReport {
            inner: self.inner.run_until(target_us as u64, max_steps),
        }
    }

    /// Emulated time since construction or the last reset, in microseconds.
    #[wasm_bindgen(getter, js_name = emulatedTimeUs)]
    pub fn emulated_time_us(&self) -> f64 {
        self.inner.emulated_time_us() as f64
    }
```

- [ ] **Step 13: Bridge methods.** In `frontend/src/cpu/bridge.ts` add to `FirmwareEmulatorHandle`:

```ts
  /** Runs until emulated time reaches `targetUs` or `maxSteps` steps, whichever is first. */
  runUntil(targetUs: number, maxSteps: number): CpuRunSummary;
  /** Emulated time since construction or the last `reset()`, in microseconds. */
  emulatedTimeUs(): number;
```

and implement them in the returned object exactly like `run` (same `report.free()` in `finally`) and `totalSteps` (`return wasm.emulatedTimeUs;`). Factor the `RunReport` → `CpuRunSummary` copy into one local `summarize(report)` function used by both `run` and `runUntil`. Add `runUntil` (records `[targetUs, maxSteps]`, returns `steps: 0`) and `emulatedTimeUs: () => 0` to `makeFakeHandle` in `frontend/test/firmware-runtime.test.ts`.

- [ ] **Step 14: WASM twin check.** In `frontend/test/cpu-wasm.test.ts`, in the "firmware-boot boundary" describe, add:

```ts
  test("runUntil stops at the target emulated time or the step cap", () => {
    const handle = createFirmwareEmulatorFromBytes(image);
    try {
      expect(handle.emulatedTimeUs()).toBe(0);
      const s = handle.runUntil(1_000, 1_000_000);
      expect(handle.emulatedTimeUs()).toBeGreaterThanOrEqual(1_000);
      expect(s.steps).toBe(handle.totalSteps());
      expect(handle.runUntil(1e12, 500).steps).toBe(500);
    } finally {
      handle.dispose();
    }
  });
```

Use whatever constructor/fixture name the surrounding tests in that describe already use for `image` and handle creation (read the file's first 60 lines); do not invent a new loader.

- [ ] **Step 15: Full check, decisions doc, commit.** Create `docs/milestone-6-decisions.md` with a title, a "Rulings" section (empty list), a "Measurements" section holding the Task 1 baseline numbers, and a "Milestone 7 backlog" heading. Add a history entry "Milestone 6 Task 1: the clock" to `docs/firmware-emulator-notes.md` (what changed, legacy ratio, no number moved). Run the full Global Constraints check list. Commit:

```bash
git add emulator-core emulator-wasm frontend/src/cpu/bridge.ts frontend/test docs
git commit -m "feat: emulated clock, emulated time and run_until (legacy ratio)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: SYSTEM clock registers and the stateful ROM frequency pair

**Files:**
- Modify: `emulator-core/src/peripherals/system.rs`, `emulator-core/src/mem/soc.rs`, `emulator-core/src/mem/bus.rs` (SYSTEM routing ~L713, ~L823), `emulator-core/src/rom.rs` (module doc entry 6 ~L121, `CPU_FREQ_MHZ` ~L2055, `NAMED_STUBS` ~L2192–2195, `ESP32C3_ROM_CODE` ~L1810, `esp32c3_rom_ram_initializers` ~L2019), `emulator-core/tests/boot_progress.rs`, `emulator-core/tests/rom_stub_boot.rs`, any test that references `CPU_FREQ_MHZ` (`/usr/bin/grep -rn CPU_FREQ_MHZ emulator-core frontend`)

**Interfaces:**
- Consumes: Task 1's `FirmwareBus`.
- Produces (`peripherals::system`): `pub const CPU_PER_CONF_REG: u32 = 0x008;`, `pub const SYSCLK_CONF_REG: u32 = 0x058;`, `System::cpu_hz(&self) -> Option<u64>` (the frequency the registers select; `None` for an invalid encoding).
- Produces (`mem::soc`): `pub const XTAL_HZ: u64 = 40_000_000;`
- Produces (`rom`): `ETS_UPDATE_CPU_FREQUENCY: u32 = 0x4000_0588`, `ROM_TICKS_PER_US: u32 = 0x3fcd_f64c`, `ROM_TICKS_PER_US_INIT: u32 = 40`; `CPU_FREQ_MHZ` deleted.
- The clock does **not** read `System::cpu_hz` yet (Task 3).

- [ ] **Step 1: Re-fetch and cite the sources.** Fetch `components/soc/esp32c3/register/soc/system_reg.h`, `components/hal/esp32c3/include/hal/clk_tree_ll.h` and `components/esp_hw_support/port/esp32c3/rtc_clk.c` (v5.5.3) into the scratchpad and confirm every field in "Handoff state" against them. Any mismatch → stop and report.

- [ ] **Step 2: Write the failing register tests** in `system.rs`'s tests module (create one if absent):

```rust
    fn write32(s: &mut System, offset: u32, v: u32) {
        for (i, b) in v.to_le_bytes().into_iter().enumerate() {
            s.write_byte(offset + i as u32, b);
        }
    }

    fn read32(s: &mut System, offset: u32) -> u32 {
        u32::from_le_bytes(core::array::from_fn(|i| s.read_byte(offset + i as u32)))
    }

    #[test]
    fn clock_registers_reset_to_the_header_defaults() {
        let mut s = System::new();
        assert_eq!(read32(&mut s, CPU_PER_CONF_REG), 0x0000_000c);
        assert_eq!(read32(&mut s, SYSCLK_CONF_REG), 0x0000_0001);
        assert_eq!(s.cpu_hz(), Some(20_000_000), "XTAL / 2 at reset");
    }

    #[test]
    fn xtal_source_divides_the_crystal() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 0); // XTAL, PRE_DIV_CNT 0
        assert_eq!(s.cpu_hz(), Some(40_000_000));
        write32(&mut s, SYSCLK_CONF_REG, 3); // XTAL / 4
        assert_eq!(s.cpu_hz(), Some(10_000_000));
    }

    #[test]
    fn pll_source_uses_cpuperiod_sel() {
        let mut s = System::new();
        write32(&mut s, CPU_PER_CONF_REG, 0x0c); // CPUPERIOD_SEL 0
        write32(&mut s, SYSCLK_CONF_REG, 1 << 10); // PLL
        assert_eq!(s.cpu_hz(), Some(80_000_000));
        write32(&mut s, CPU_PER_CONF_REG, 0x0d); // CPUPERIOD_SEL 1
        assert_eq!(s.cpu_hz(), Some(160_000_000));
    }

    #[test]
    fn rc_fast_source_is_20_mhz() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 2 << 10);
        assert_eq!(s.cpu_hz(), Some(20_000_000));
    }

    #[test]
    fn invalid_clock_encodings_keep_the_previous_frequency() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 3 << 10); // SOC_CLK_SEL 3: invalid
        assert_eq!(s.cpu_hz(), None);
        write32(&mut s, SYSCLK_CONF_REG, 1 << 10);
        write32(&mut s, CPU_PER_CONF_REG, 0x0f); // CPUPERIOD_SEL 3: invalid
        assert_eq!(s.cpu_hz(), None);
    }

    #[test]
    fn read_only_sysclk_fields_ignore_writes() {
        let mut s = System::new();
        write32(&mut s, SYSCLK_CONF_REG, 0xffff_ffff);
        assert_eq!(read32(&mut s, SYSCLK_CONF_REG), 0x0000_0fff);
    }
```

Run: `cargo test -p emulator-core --lib peripherals::system` — Expected: compile errors.

- [ ] **Step 3: Implement the clock registers.** In `system.rs`: extend the module doc (sources, fields, defaults, the frequency rule with its `rtc_clk.c` citation, "RO fields read 0", "`None` = invalid encoding; the bus keeps the previous frequency"); add the two offsets, `const CPU_PER_CONF_RESET: u32 = 0x0c; const CPU_PER_CONF_RW: u32 = 0xff; const SYSCLK_CONF_RESET: u32 = 0x01; const SYSCLK_CONF_RW: u32 = 0x0fff;`, fields `cpu_per_conf: u32, sysclk_conf: u32` (reset values in `new`), extend `handles(offset)` to cover `CPU_PER_CONF_REG..CPU_PER_CONF_REG + 4` and `SYSCLK_CONF_REG..SYSCLK_CONF_REG + 4`, byte-lane reads/writes (mask writes with the R/W mask), and:

```rust
    /// The CPU frequency these registers select, per ESP-IDF v5.5.3
    /// `rtc_clk_cpu_freq_get_config`; `None` for an encoding it would abort
    /// on. See the module doc.
    pub fn cpu_hz(&self) -> Option<u64> {
        match (self.sysclk_conf >> 10) & 0b11 {
            0 => Some(XTAL_HZ / (u64::from(self.sysclk_conf & 0x3ff) + 1)),
            1 => match self.cpu_per_conf & 0b11 {
                0 => Some(80_000_000),
                1 => Some(160_000_000),
                _ => None,
            },
            2 => Some(20_000_000),
            _ => None,
        }
    }
```

Add `XTAL_HZ` to `mem/soc.rs` with its doc ("the badge's 40 MHz crystal: notes, 'Ground truth from the physical badge'; also the value the bootloader stores in `RTC_XTAL_FREQ_REG`, `rom.rs`"). The bus already routes `System::handles` offsets to `system`; confirm the read (~L713) and write (~L823) paths use `handles` and that the two addresses no longer appear in the unmapped log (add a bus test: write then read `SYSTEM_RANGE.start + 0x58`, assert the value and an empty unmapped log).

- [ ] **Step 4: Run the register tests.** Run: `cargo test -p emulator-core --lib` — Expected: pass.

- [ ] **Step 5: Write the failing ROM frequency tests** in `rom.rs`'s tests module, next to the `strdup` tests (same harness: `FirmwareBus::from_segments` + `install_esp32c3_rom_code`, a `Cpu` with `esp32c3_rom_stubs()`, run from a slot until `pc == RET_SENTINEL`):

```rust
    /// Calls the ROM function at `slot` with `a0 = arg` on `bus` and returns
    /// `a0` after it returns.
    fn call_rom(bus: &mut FirmwareBus, slot: u32, arg: u32) -> u32 {
        use crate::cpu::rom_stubs::{REG_RA, REG_SP};
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_A0, arg);
        cpu.regs.write(REG_RA, RET_SENTINEL);
        cpu.regs.write(REG_SP, STACK_TOP);
        cpu.regs.pc = slot;
        let mut steps = 0u32;
        while cpu.regs.pc != RET_SENTINEL {
            assert!(steps < 100, "{slot:#x} did not return within 100 steps");
            let info = cpu.step(bus);
            assert!(!info.trap_taken, "trapped at {:#x}", info.pc_before);
            steps += 1;
        }
        cpu.regs.read(REG_A0)
    }

    fn cpu_freq_bus() -> FirmwareBus {
        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.add_scratch_ram(ROM_TICKS_PER_US & !0xFFF, 0x1000);
        install_esp32c3_rom_code(&mut bus);
        bus
    }

    #[test]
    fn cpu_frequency_pair_stores_and_loads_s_ticks_per_us() {
        let mut bus = cpu_freq_bus();
        call_rom(&mut bus, ETS_UPDATE_CPU_FREQUENCY, 80);
        assert_eq!(bus.read32(ROM_TICKS_PER_US), 80);
        assert_eq!(call_rom(&mut bus, ETS_GET_CPU_FREQUENCY, 0xdead), 80);
        call_rom(&mut bus, ETS_UPDATE_CPU_FREQUENCY, 40);
        assert_eq!(call_rom(&mut bus, ETS_GET_CPU_FREQUENCY, 0), 40);
    }

    #[test]
    fn s_ticks_per_us_starts_at_the_rom_value() {
        let inits = esp32c3_rom_ram_initializers(&ImageHeader::default());
        let init = inits
            .iter()
            .find(|i| i.addr == ROM_TICKS_PER_US)
            .expect("s_ticks_per_us is initialized");
        assert_eq!(init.bytes, 40u32.to_le_bytes());
    }

    #[test]
    fn the_cpu_frequency_slots_are_code_not_stubs() {
        let stubs = esp32c3_rom_stubs();
        for slot in [ETS_GET_CPU_FREQUENCY, ETS_UPDATE_CPU_FREQUENCY] {
            assert!(stubs.get(slot).is_none(), "{slot:#x} must not be a stub");
            assert!(ESP32C3_ROM_CODE.iter().any(|b| b.base == slot));
        }
    }
```

Adapt only the names the module actually uses: the `ImageHeader` constructor the existing `esp32c3_rom_ram_initializers` tests use (search for them; use their header helper instead of `ImageHeader::default()` if there is no `Default`), and the stub-table lookup method (`esp32c3_rom_stubs()` returns the table type from `cpu/rom_stubs.rs`; use its real lookup method name). Run: `cargo test -p emulator-core --lib rom::` — Expected: FAIL (the constants do not exist yet).

- [ ] **Step 6: Implement the blobs.** In `rom.rs`, after `STRDUP_BODY`:

```rust
/// `ets_update_cpu_frequency`'s jump-table slot (`esp32c3.rom.ld`).
pub const ETS_UPDATE_CPU_FREQUENCY: u32 = 0x4000_0588;
/// The ROM's `s_ticks_per_us` (`.data_ets_delay` in `esp32c3_rev3_rom.elf`):
/// the CPU ticks per microsecond the setter stores and the getter returns.
pub const ROM_TICKS_PER_US: u32 = 0x3fcd_f64c;
/// `s_ticks_per_us`'s initial value in the rev3 ROM ELF (`0x28`): 40, the
/// XTAL clock the ROM runs at.
pub const ROM_TICKS_PER_US_INIT: u32 = 40;
const _: () = assert!(ROM_TICKS_PER_US & 0x800 == 0, "lui needs no +1 carry");

/// `void ets_update_cpu_frequency(uint32_t ticks_per_us)`, as the rev3 ROM
/// body at `0x400462e8`: `lui a5,0x3fcdf; sw a0,0x64c(a5); ret`.
pub const UPDATE_CPU_FREQ_BODY_ADDR: u32 = STRDUP_BODY_ADDR + 4 * STRDUP_BODY.len() as u32;
pub const UPDATE_CPU_FREQ_BODY: [u32; 3] = [
    lui(A5, ROM_TICKS_PER_US >> 12),
    sw(A0, A5, (ROM_TICKS_PER_US & 0xfff) as i32),
    jalr(ZERO, RA, 0),
];
/// `uint32_t ets_get_cpu_frequency(void)`, as the rev3 ROM body at
/// `0x400462f2`: `lui a5,0x3fcdf; lw a0,0x64c(a5); ret`.
pub const GET_CPU_FREQ_BODY_ADDR: u32 = UPDATE_CPU_FREQ_BODY_ADDR + 4 * UPDATE_CPU_FREQ_BODY.len() as u32;
pub const GET_CPU_FREQ_BODY: [u32; 3] = [
    lui(A5, ROM_TICKS_PER_US >> 12),
    lw(A0, A5, (ROM_TICKS_PER_US & 0xfff) as i32),
    jalr(ZERO, RA, 0),
];
pub const UPDATE_CPU_FREQ_SLOT: [u32; 1] =
    [jal(ZERO, (UPDATE_CPU_FREQ_BODY_ADDR - ETS_UPDATE_CPU_FREQUENCY) as i32)];
pub const GET_CPU_FREQ_SLOT: [u32; 1] =
    [jal(ZERO, (GET_CPU_FREQ_BODY_ADDR - ETS_GET_CPU_FREQUENCY) as i32)];
const _: () =
    assert!(GET_CPU_FREQ_BODY_ADDR + 4 * GET_CPU_FREQ_BODY.len() as u32 <= ROM_CODE_FREE_RANGE.end);
```

Add the four blobs to `ESP32C3_ROM_CODE`; delete both entries from `NAMED_STUBS` and delete `CPU_FREQ_MHZ`; rewrite module-doc entry 6 (stateful pair as guest code, why: the firmware's DFS calls update 40/80 thousands of times and its `cpu freq` log line reads the getter; the old 160 constant is history). Add two boot initializers to `esp32c3_rom_ram_initializers` with comments:

```rust
        // 4. The ROM's .data `s_ticks_per_us` (rev3 ELF initial value).
        RamInitializer {
            addr: ROM_TICKS_PER_US,
            bytes: ROM_TICKS_PER_US_INIT.to_le_bytes().to_vec(),
        },
        // 5. SYSTEM_SYSCLK_CONF_REG as the skipped ROM/bootloader leave it:
        //    XTAL / 1 (PRE_DIV_CNT 0), 40 MHz, matching s_ticks_per_us = 40.
        RamInitializer {
            addr: SYSTEM_RANGE.start + SYSCLK_CONF_REG,
            bytes: 0u32.to_le_bytes().to_vec(),
        },
```

(`RamInitializer` writes go through the bus, so item 5 reaches `System`; update the function's doc to say it now also sets a SYSTEM register, as it already does for `RTC_XTAL_FREQ_REG`.) Confirm `0x3fcd_f64c` is backed RAM on the bus (the existing tests write `0x3fcd_f5c0` the same way).

- [ ] **Step 7: Run the unit tests.** Run: `cargo test -p emulator-core --lib` — Expected: pass.

- [ ] **Step 8: The cpu-freq rung.** In `tests/boot_progress.rs`, after `boot_reaches_efuse_inits_chip_rev_line`:

```rust
/// The ROM frequency pair is stateful, so `cpu_start` reports the
/// frequency the firmware selected: 80 MHz, as in the physical badge's
/// boot log (Milestone 6 Task 2; it printed 160 MHz from a constant stub
/// before). Printed at step ~407,600.
#[test]
fn boot_reports_the_badges_80_mhz_cpu_clock() {
    assert_reaches("cpu_start: cpu freq: 80000000 Hz", 500_000);
}
```

Run it: `cargo test -p emulator-core --release --test boot_progress boot_reports_the_badges_80_mhz_cpu_clock` — Expected: PASS.

- [ ] **Step 9: Re-pin what moved.** Run the release `boot_progress` suite and `cargo test -p emulator-core --test rom_stub_boot`. Busy-wait delays now count 80 (not 160) ticks per µs, so step-exact rungs and timestamps move. For each failure: confirm the change is timing only (same console lines, same frames), re-pin exactly, and update its doc's step numbers. A frame-hash change is not expected in this task: if one moves, stop and report it. Record every old → new number in `docs/milestone-6-decisions.md` "Measurements" as a table.

- [ ] **Step 10: Full check, notes, commit.** Notes history entry "Milestone 6 Task 2: SYSTEM clock registers and the stateful ROM frequency pair" (the probe facts from Handoff state, what moved). Remove `0x600c_0058`/`+0x08` from the "unmapped accesses at the idle point" lists in the notes and `docs/milestone-5-decisions.md`'s backlog is left as history (do not edit it; the M7 backlog in the new decisions doc supersedes it). Run the full check list. Commit `feat: SYSTEM clock registers and stateful ROM cpu frequency`.

---

### Task 3: The switch — the clock follows the firmware's CPU frequency

**Files:**
- Modify: `emulator-core/src/mem/bus.rs`, `emulator-core/src/peripherals/usb_serial_jtag.rs` (L93, L120–144, L195, tests L400–510), `emulator-core/src/peripherals/systimer.rs` (tick-rate doc ~L109–122, tests ~L541), `emulator-core/src/cpu/registers.rs` (doc ~L101), `emulator-core/src/peripherals/rtc_cntl.rs` (doc ~L86), `emulator-core/src/runtime.rs` (WFI test ~L346), `emulator-core/tests/boot_progress.rs`, `emulator-core/tests/common/provision.rs`, `emulator-core/tests/rom_stub_boot.rs`, `frontend/test/cpu-wasm.test.ts`, `frontend/src/runtime/firmware-runtime.ts`, `frontend/test/firmware-runtime.test.ts`, `frontend/src/runtime/provisioner.ts`, `frontend/test/provisioner.test.ts`

**Interfaces:**
- Consumes: `Clock::set_cpu_hz`, `System::cpu_hz`, `run_until`, `emulated_time_us`, `XTAL_HZ`, `SYSTIMER_HZ`.
- Produces (`usb_serial_jtag`): `PACKET_TICKS: u64 = 821`; `PACKET_STEPS` and the `TICKS_PER_STEP` import deleted. `systimer::TICKS_PER_STEP` deleted.
- Produces (rungs): `PRESS_HOLD_US: u64 = 100_000`, `SHAKE_HALF_US: u64 = 62_500`, `HOLD_MAX_STEPS: u32 = 40_000_000`.
- Produces (TS, `firmware-runtime.ts`): `PRESS_HOLD_US = 100_000`, `SHAKE_HALF_US = 62_500` (replace `PRESS_HOLD_STEPS`, `SHAKE_HALF_STEPS`); `shakeAcceleration(elapsedUs: number)`.

- [ ] **Step 1: Write the failing bus tests** in `mem/bus.rs`'s tests:

```rust
    const SYSCLK_CONF: u32 = SYSTEM_RANGE.start + 0x058;
    const CPU_PER_CONF: u32 = SYSTEM_RANGE.start + 0x008;

    /// A bare bus (no boot initializers) with SYSCLK_CONF set as the boot
    /// initializer sets it (XTAL / 1) and applied by one step's sync.
    fn xtal_bus() -> FirmwareBus {
        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.write32(SYSCLK_CONF, 0);
        bus.tick_peripherals();
        bus
    }

    #[test]
    fn a_bare_bus_follows_the_reset_clock_registers() {
        // No initializer: SYSCLK_CONF's reset PRE_DIV_CNT = 1 is XTAL / 2.
        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        assert_eq!(bus.clock.cpu_hz(), 40_000_000, "constructed at the XTAL clock");
        bus.tick_peripherals();
        assert_eq!(bus.clock.cpu_hz(), 20_000_000);
    }

    #[test]
    fn clock_follows_the_cpu_clock_registers_after_the_step() {
        let mut bus = xtal_bus();
        assert_eq!(bus.clock.cpu_hz(), 40_000_000);
        bus.write32(CPU_PER_CONF, 0x0c); // CPUPERIOD_SEL 0 (80 MHz)
        bus.write32(SYSCLK_CONF, 1 << 10); // PLL
        assert_eq!(bus.clock.cpu_hz(), 40_000_000, "not until the step ends");
        bus.tick_peripherals();
        assert_eq!(bus.clock.cpu_hz(), 80_000_000);
    }

    #[test]
    fn bus_steps_at_80_mhz_tick_once_every_five_steps() {
        let mut bus = xtal_bus();
        bus.write32(SYSCLK_CONF, 1 << 10);
        bus.tick_peripherals(); // applies 80 MHz
        let before = bus.clock.total_ticks();
        for _ in 0..10 {
            bus.tick_peripherals();
        }
        assert_eq!(bus.clock.total_ticks() - before, 2);
    }

    #[test]
    fn invalid_clock_encoding_keeps_the_clock_rate() {
        let mut bus = xtal_bus();
        bus.write32(SYSCLK_CONF, 3 << 10);
        bus.tick_peripherals();
        assert_eq!(bus.clock.cpu_hz(), 40_000_000);
    }
```

(Use the bus tests' existing imports for `Arc`, `SYSTEM_RANGE` and the `Bus` trait's `write32`; add any that are missing.) Also rewrite Task 1's `zero_tick_steps_leave_the_systimer_untouched` to start from `xtal_bus()`, select the PLL (`CPU_PER_CONF` = `0x0c`, `SYSCLK_CONF` = `1 << 10`) and apply it with one `tick_peripherals()`, then check the SYSTIMER over the next five steps (0, 0, 0, 0, then 1 tick; measure from the post-switch `elapsed_ticks()`, since the sync step itself may complete a tick). Run: `cargo test -p emulator-core --lib mem::bus` — Expected: FAIL.

- [ ] **Step 2: Implement the sync.** In the bus constructor: `clock: Clock::new(XTAL_HZ, SYSTIMER_HZ, 1)` (doc: "starts at the XTAL clock the ROM leaves; follows `System::cpu_hz` from then on"). In `tick_peripherals`, first line:

```rust
        // The CPU clock the SYSTEM registers select now, sampled after the
        // step so a word write's byte-split halves are never seen alone.
        if let Some(hz) = self.system.cpu_hz() {
            self.clock.set_cpu_hz(hz);
        }
```

Also apply it at the start of `advance_idle` (a write just before `wfi` takes effect for the idle time). If `System::cpu_hz` shows up in a profile (Step 9), cache the derived value in `System` on write; do not optimize before measuring.

- [ ] **Step 3: USB packet time in ticks.** In `usb_serial_jtag.rs`: delete the `TICKS_PER_STEP` import and `PACKET_STEPS`; define

```rust
/// SYSTIMER ticks one OUT packet occupies on the full-speed bus: a 64-byte
/// DATA packet plus its OUT token and ACK handshake is ~616 bits, ~51.3 us
/// at 12 Mbit/s; at 16 ticks/us that is 820.8, taken as 821. Bus time,
/// not CPU steps: since Milestone 6 the clock converts steps to time, so
/// the CPU sees ~4,105 steps per packet at 80 MHz.
pub const PACKET_TICKS: u64 = 821;
```

Rewrite the module doc paragraph and the tests that pinned `8_210` (the value test asserts 821; the behaviour tests already use `PACKET_TICKS`). Delete `TICKS_PER_STEP` from `systimer.rs` and rewrite its "Tick rate" doc section (time comes from `crate::clock::Clock` via the bus; real 16 MHz). Fix the doc references in `registers.rs`, `rtc_cntl.rs`, `bus.rs` (module doc L96) and `boot_progress.rs` L938. Update `runtime.rs`'s `run_returns_after_budget_even_if_wfi_never_wakes`: the synthetic image never writes SYSTEM, so the clock stays at 40 MHz and 1000 steps are `1000 * 16 / 40 = 400` ticks; assert `rt.bus().systimer.elapsed_ticks() == 400` with that comment. Run `cargo test -p emulator-core --lib` — Expected: pass.

- [ ] **Step 4: Time-based holds in the rungs.** In `tests/boot_progress.rs` replace `PRESS_HOLD_STEPS` and `SHAKE_HALF_STEPS`:

```rust
/// How long a button press is held in the input tests: a human press,
/// 100 ms of emulated time (Milestone 6: was 1,600,000 steps, 100 ms
/// under the pre-Milestone-6 one-tick-per-step clock).
const PRESS_HOLD_US: u64 = 100_000;
/// One half-period of the shake: 62.5 ms of emulated time (was 1,000,000
/// steps under the old clock). Six half-periods, alternating
/// (+2 g, +2 g, +1 g) and (-2 g, -2 g, +1 g), then rest at (0, 0, +1 g).
const SHAKE_HALF_US: u64 = 62_500;
/// Step cap for any [`run_for_us`] call: 100 ms at 80 MHz is 8M busy
/// steps; five times that is a hang, not a slow hold.
const HOLD_MAX_STEPS: u32 = 40_000_000;

/// Runs `us` microseconds of emulated time, asserting no fault and that
/// the time was reached within [`HOLD_MAX_STEPS`].
fn run_for_us(rt: &mut FirmwareRuntime, us: u64) {
    let target = rt.emulated_time_us() + us;
    let s = rt.run_until(target, HOLD_MAX_STEPS);
    assert_eq!(s.last_instruction_fault, None, "{s:?}");
    assert!(rt.emulated_time_us() >= target, "{us} us not reached in {} steps", s.steps);
}
```

`press` uses `run_for_us(rt, PRESS_HOLD_US)`; `shake` uses `run_for_us(rt, SHAKE_HALF_US)` per half-period. Find every other `PRESS_HOLD_STEPS`/`SHAKE_HALF_STEPS` use in the file (`/usr/bin/grep -n`) and convert it the same way. Leave stability windows, budgets and `FRAME_CHUNK` in steps (spec: they measure firmware work).

- [ ] **Step 5: Run the suite and collect what moved.** Run: `cargo test -p emulator-core --release --test boot_progress 2>&1 | tee local/m6-task3-first-run.txt` and `cargo test -p emulator-core --test rom_stub_boot`. Expect many failures. Sort each into: (a) a step-exact pin or budget that moved with console lines and frames unchanged → re-pin; (b) a frame hash that moved → list it; (c) anything else (a fault, a missing console line, a stall) → treat as a stall: trace it (Task D template below), ruling in the ledger, fix in this task only if it is a clock/timing consequence; otherwise stop and report.

- [ ] **Step 6: Hash changes go to the human.** For every moved hash, dump old and new frames to PNG under `local/m6-task3/` (write a temporary, uncommitted `#[ignore]` test in `boot_progress.rs` that walks to the frame and writes it with the `png` dev-dependency, the same encoder calls as `examples/boot-probe.rs`'s `dump_frame`; delete the test before committing). Report the list (rung, old hash, new hash, PNG paths, whether that frame was hardware-compared) to the orchestrator. **Human gate:** a hardware-compared frame that moved is re-pinned only after the human's OK. Expected movers: animated frames, Dice's roll (the RNG is fixed-seed, but which value is read depends on timing).

- [ ] **Step 7: Re-measure the chunk gap.** In `tests/common/provision.rs`, try `PUT_CHUNK_GAP_STEPS` = 24,000 with `provisions_hacker_through_the_console` and `put_accepts_a_payload_larger_than_the_rx_ring`. Rule: if 24,000 passes both, keep 48,000 (2x margin); if it fails, try 48,000; if 48,000 fails, double until both pass and use twice the smallest passing value. Record the runs in the decisions doc. Mirror the final value in `frontend/src/runtime/provisioner.ts` and `frontend/test/provisioner.test.ts`.

- [ ] **Step 8: Re-pin and convert the WASM twin.** Re-pin every (a) item exactly (step-exact rungs) or as a margin (budgets: the measured value rounded up to a 250,000 multiple plus the rung's existing margin rule; keep each rung's doc formula and update its numbers). Re-pin (b) items the human approved. In `frontend/test/cpu-wasm.test.ts`: replace `PRESS_HOLD_STEPS`/`SHAKE_HALF_STEPS` with `PRESS_HOLD_US = 100_000`/`SHAKE_HALF_US = 62_500` and a `runForUs(handle, us)` helper mirroring `run_for_us` (`handle.runUntil(handle.emulatedTimeUs() + us, 40_000_000)`, assert no fault and time reached); copy every re-pinned hash and budget from `boot_progress.rs`.

- [ ] **Step 9: Frontend holds in emulated time.** In `frontend/src/runtime/firmware-runtime.ts`: replace `PRESS_HOLD_STEPS` with

```ts
/**
 * Minimum emulated time a button stays pressed: 100 ms (same value as
 * `PRESS_HOLD_US` in `emulator-core/tests/boot_progress.rs`). A shorter
 * click would be missed by the firmware's button polling.
 */
export const PRESS_HOLD_US = 100_000;

/** Emulated µs per shake half-period (`SHAKE_HALF_US` in boot_progress.rs). */
export const SHAKE_HALF_US = 62_500;
```

`shakeAcceleration(elapsedUs)` divides by `SHAKE_HALF_US`; `pressedAt`, `pendingRelease`, `shakeStartedAt` and `pollShake`/`pollReleases` use `handle.emulatedTimeUs()` instead of `handle.totalSteps()` (the provisioner timeout stays in steps). In `frontend/test/firmware-runtime.test.ts`, give `rig()` a `setTimeUs(n)` backed by the fake's `emulatedTimeUs`, and convert the "press hold" and "shake" describes to it with the new constants (same assertions, time instead of steps; rename the test "is scheduled from the frame loop off totalSteps" to "... off emulated time").

- [ ] **Step 10: Measure and record.** Record in the decisions doc: the release `boot_progress` wall time (before: ~42 s), the debug workspace time, `bun test` time, the steps to the launcher (was ~317.5M) and to Dice, the console `cpu freq` line, and the moved-hash list with the human's verdicts. If the release suite exceeds 90 s, stop and report (the opt-level-3 backlog item becomes the remedy; do not change profiles without the human).

- [ ] **Step 11: Full check, notes, commit.** Notes history entry "Milestone 6 Task 3: the switch" (old → new figures). Full check list. Commit `feat: clock follows the firmware's CPU frequency; time-based holds`.

---

### Task D (template, repeat per stall)

Used whenever a rung stalls for a reason that is not plain timing. One stall per task; brief to an implementer with: the rung, the last console lines, hot PCs (`boot-probe` or a `run_traced` histogram in a temporary test), unmapped accesses near the stall, and the ROM/ESP-IDF symbol for each hot PC (from the app's own strings/ELF knowledge or the ROM ELF). The fix follows the Global Constraints (register-faithful, cited; ROM via stubs/blobs only). Each D task gets a ruling line in the ledger and an entry in `docs/milestone-6-decisions.md`, and adds or adjusts the rung that proves it.

---

### Task 4: Snake opens and reaches game over (provisional pins)

**Files:**
- Modify: `emulator-core/tests/boot_progress.rs`
- Local only: `local/m6-snake/` PNGs from a temporary, uncommitted exploration test

**Interfaces:**
- Consumes: `launcher_for(OWN_ROLE)`, `press`, `press_and_settle`, `TO_DICE` (re-pinned in Task 3), `run_for_us`, `framebuffer_fnv1a`.
- Produces: `SNAKE: OnceLock<FirmwareRuntime>` checkpoint and `fn snake_open() -> FirmwareRuntime` (clone of the runtime right after A's hold on "Snake" is released); `fn region_fnv1a(fb: &[u16], width: usize, rect: Rect) -> u64`; `struct Rect { x0: usize, y0: usize, x1: usize, y1: usize }` (half-open); `const PLAY_AREA: Rect`; `fn move_times(rt: &mut FirmwareRuntime, window_us: u64) -> Vec<u64>`; `const SNAKE_OPEN_HASH: u64`, `const SNAKE_GAME_OVER_HASH: u64`, `const SNAKE_OPEN_FRAME_US: u64`.

- [ ] **Step 1: Explore (no commit).** Add a temporary `#[ignore]` test `local_snake_explore` that takes `launcher_for(OWN_ROLE)`, walks `&TO_DICE[..5]` (DOWN×3, RIGHT×2: "Snake" selected) with `press_and_settle`, presses A (slot 1), then writes a PNG of the framebuffer every 20 ms of emulated time for 5 s, plus the emulated time and region-free full hash of each, to `local/m6-snake/`. Run: `cargo test -p emulator-core --release --test boot_progress local_snake_explore -- --ignored --nocapture`. Answer, in the ledger: Does Snake wait for a key before moving? Which direction does it start? Where is the play area (pixel rectangle, excluding score/title text)? How many emulated ms between moves? How many moves until game over with no input? What does game over look like, and is it stable? Does `app_reg: launched Snake` print? If Snake stalls, faults, or game over still comes at once (the M5 symptom), stop: that is a Task D, not a pin.

- [ ] **Step 2: Write the helpers and the open rung.** Using Step 1's answers, add to `boot_progress.rs`:

```rust
/// A half-open pixel rectangle `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

/// FNV-1a over `rect`'s pixels only (same byte order as
/// [`framebuffer_fnv1a`]).
fn region_fnv1a(fb: &[u16], width: usize, rect: Rect) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in rect.y0..rect.y1 {
        for px in &fb[y * width + rect.x0..y * width + rect.x1] {
            for b in px.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    h
}

/// Snake's play area (the grid, without the title/score text), measured
/// from the Task 4 exploration frames: <say which frame and how>.
const PLAY_AREA: Rect = Rect { x0: /*measured*/, y0: /*measured*/, x1: /*measured*/, y1: /*measured*/ };

static SNAKE: OnceLock<FirmwareRuntime> = OnceLock::new();

/// From the hacker launcher: DOWN x3, RIGHT x2 ("Snake"), then A, held
/// [`PRESS_HOLD_US`]; the clone is taken the moment A is released.
fn snake_open() -> FirmwareRuntime {
    SNAKE
        .get_or_init(|| {
            let mut rt = launcher_for(OWN_ROLE);
            let mut previous = LAUNCHER_HASH;
            for &(slot, expected) in &TO_DICE[..5] {
                let (hash, step) = press_and_settle(&mut rt, slot, previous);
                assert_eq!(hash, expected, "after slot {slot}, stable at step {step}");
                previous = hash;
            }
            press(&mut rt, 1);
            rt
        })
        .clone()
}
```

The `/*measured*/` values are filled from Step 1 before this compiles; the doc comment says how they were measured. Then the rung (choose the variant Step 1 supports and delete the other):

```rust
/// Milestone 6: Snake opens from the launcher. Its frame <a stable start
/// screen | the frame SNAKE_OPEN_FRAME_US after A's release> is pinned;
/// compared with the physical badge on <date> (Task 5).
#[test]
fn launcher_opens_snake() {
    let mut rt = snake_open();
    // Variant A (Snake waits for input): the first stable frame.
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[TO_DICE[4].1]);
    // Variant B (Snake moves at once): a fixed emulated time after release.
    // run_for_us(&mut rt, SNAKE_OPEN_FRAME_US);
    // let hash = framebuffer_fnv1a(rt.framebuffer());
    assert_eq!(hash, SNAKE_OPEN_HASH, "Snake opened (step {step})");
    assert!(rt.console_output().contains("app_reg: launched Snake"), "console:\n{}", rt.console_output());
    assert_no_panic_text(&rt);
}
```

(Drop the console assertion if Step 1 found Snake does not log a launch line, and say so in the doc.) Run it once with `SNAKE_OPEN_HASH = 0` to read the hash from the failure, pin it, run again — Expected: PASS.

- [ ] **Step 3: `move_times` and the game-over rung.**

```rust
/// Sampling period of [`move_times`].
const MOVE_SAMPLE_US: u64 = 2_000;
/// Changes of [`PLAY_AREA`] closer than this belong to one move (LVGL
/// flushes a redraw in several stripes).
const MOVE_MERGE_US: u64 = 20_000;

/// Emulated times at which [`PLAY_AREA`] started to change, over
/// `window_us`, sampled every [`MOVE_SAMPLE_US`]; changes within
/// [`MOVE_MERGE_US`] of the previous change are one move.
fn move_times(rt: &mut FirmwareRuntime, window_us: u64) -> Vec<u64> {
    let width = rt.screen_width();
    let end = rt.emulated_time_us() + window_us;
    let mut last = region_fnv1a(rt.framebuffer(), width, PLAY_AREA);
    let mut last_change: Option<u64> = None;
    let mut starts = Vec::new();
    while rt.emulated_time_us() < end {
        run_for_us(rt, MOVE_SAMPLE_US);
        let h = region_fnv1a(rt.framebuffer(), width, PLAY_AREA);
        if h != last {
            let now = rt.emulated_time_us();
            if last_change.is_none_or(|t| now - t > MOVE_MERGE_US) {
                starts.push(now);
            }
            last_change = Some(now);
            last = h;
        }
    }
    starts
}
```

(If `Option::is_none_or` is unavailable on the toolchain, use `last_change.map_or(true, |t| ...)`.) Game-over rung:

```rust
/// With no input the snake runs into the wall: "Game over / A to
/// restart" after [`SNAKE_MOVES_TO_WALL`] moves (Task 5 compares that
/// count and the frame with the badge); A restarts.
#[test]
fn snake_hits_wall_game_over() {
    let mut rt = snake_open();
    let moves = move_times(&mut rt, SNAKE_TO_WALL_WINDOW_US);
    assert_eq!(moves.len(), SNAKE_MOVES_TO_WALL, "moves at {moves:?}");
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, BOOT_STABLE_FOR, &[]);
    assert_eq!(hash, SNAKE_GAME_OVER_HASH, "game over, stable at step {step}");
    let (hash, step) = press_and_settle(&mut rt, 1, SNAKE_GAME_OVER_HASH);
    assert_ne!(hash, SNAKE_GAME_OVER_HASH, "A restarts (step {step})");
    assert_no_panic_text(&rt);
}
```

Define `SNAKE_TO_WALL_WINDOW_US` (Step 1's time to game over plus 50 %) and `SNAKE_MOVES_TO_WALL` (Step 1's count; if the move detector counts the game-over redraw as a move, say so and count it consistently). If A after game over does not produce a stable frame (the game runs again at once), replace the last check with "`move_times` over 500 ms is non-empty". Pin, run — Expected: PASS. Record the provisional pins and Step 1 answers in the decisions doc ("provisional until Task 5").

- [ ] **Step 4: Full check, delete the exploration test, commit.** Confirm `local_snake_explore` is gone (`/usr/bin/grep -n local_snake_explore emulator-core/tests/boot_progress.rs` prints nothing). Full check list. Commit `test: Snake opens and reaches game over (provisional pins)`.

---

### Task 5: Human gate — Snake on the physical badge; calibration

**Files:**
- Modify: `docs/milestone-6-decisions.md`, `emulator-core/tests/boot_progress.rs` (constants only), possibly `emulator-core/src/mem/bus.rs` (`cycles_per_step`, only via the calibration ruling)
- Local only: `local/m6-badge/` (the human's video and photos; never committed)

- [ ] **Step 1: Prepare the comparison.** Export the emulator's three frames as PNGs to `local/m6-snake/compare/` (open, after one turn — take it from a temporary test that presses UP one move after opening — and game over), plus the emulator's measured move interval (median of `move_times` diffs) and moves-to-wall.

- [ ] **Step 2: Ask the human** (STOP; one message):
  1. A short phone video from pressing A on "Snake" in the launcher through an untouched run into the wall (put it under `local/m6-badge/` if you want it measured from the video).
  2. From it: the time between moves (or moves per second), and the number of moves until game over.
  3. Photos or screenshots of: the start screen, the snake right after one turn, and game over.
  4. Their verdict on each of the three emulator PNGs against the photos (layout, snake, walls, text; food position will differ: the badge's RNG is real).

- [ ] **Step 3: Record and compare.** Record the human's numbers and verdicts in the decisions doc (no photo content). Define in `boot_progress.rs`: `const BADGE_MOVE_INTERVAL_US: u64 = /* the human's figure */;` with a doc citing the gate date. Compare: `|emulated - badge| <= badge / 4`?
  - Inside ±25 %: no calibration; record "CPI 1 stands".
  - If a CPI change is approved: the `mpccr` cycle counter must then count `cycles_per_step` per working step (spec Part 1: it counts the clock's cycles); change `Csrs::tick_cycle_counter` to take a count and pass the clock's value, with a unit test, in the same commit.
  - Outside: **stop and ask.** Before proposing a CPI change, check whether the emulated CPU was saturated between moves (fraction of steps spent outside WFI over the window, from `rt.bus().clock.idle_ticks()` deltas). If the CPU idles between moves, CPI cannot change the move interval (it is the firmware's own timer period) and the mismatch is a timing-model bug: a factor of 2 or 5 points at the clock rules. Only a saturated CPU makes `cycles_per_step` the right knob; propose the value with the arithmetic, and change it only with the human's OK (ruling in the decisions doc).
  - Moves-to-wall differs from the badge: stop and report (geometry or start-direction issue).
  - A frame the human rejects: stop and report.

- [ ] **Step 4: Commit** the decisions entry and constants: `docs: Snake compared with the physical badge`.

---

### Task 6: Speed and steering rungs

**Files:**
- Modify: `emulator-core/tests/boot_progress.rs`

**Interfaces:**
- Consumes: `snake_open`, `move_times`, `BADGE_MOVE_INTERVAL_US`, `run_for_us`, `press`.
- Produces: `fn median_interval(times: &[u64]) -> u64`; `SNAKE_TURNED_HASH`, `SNAKE_TURN_SLOT`.

- [ ] **Step 1: Write the speed rung.**

```rust
/// Median gap between consecutive move start times. Panics with fewer
/// than three moves (two intervals).
fn median_interval(times: &[u64]) -> u64 {
    assert!(times.len() >= 3, "too few moves: {times:?}");
    let mut gaps: Vec<u64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    gaps[gaps.len() / 2]
}

/// Milestone 6: the snake moves on its own at the badge's speed: the
/// median emulated time between moves is within 25 % of the physical
/// badge's (Task 5). Fails on the pre-Milestone-6 clock (the game timer
/// outran rendering), so it guards the timing model.
#[test]
fn snake_moves_on_its_own() {
    let mut rt = snake_open();
    // Variant A only: start the game the way the badge does (from Task 4).
    let moves = move_times(&mut rt, SPEED_WINDOW_US);
    let interval = median_interval(&moves);
    let badge = BADGE_MOVE_INTERVAL_US;
    assert!(
        interval.abs_diff(badge) <= badge / 4,
        "emulated move interval {interval} us vs badge {badge} us; moves at {moves:?}"
    );
    assert_no_panic_text(&rt);
}
```

`SPEED_WINDOW_US` = enough for at least 6 moves before the wall (from Task 4's numbers; document the arithmetic). Run it — Expected: PASS (Task 5 established the margin).

- [ ] **Step 2: Prove the rung guards the regression.** Temporarily construct the bus clock at the legacy ratio (`Clock::new(SYSTIMER_HZ, SYSTIMER_HZ, 1)` and skip the `set_cpu_hz` sync), run `snake_moves_on_its_own`, confirm it FAILS, revert. Record the failing figure in the ledger.

- [ ] **Step 3: Write the steering rung.**

```rust
/// A direction press turns the snake on the next move: after one move,
/// [`SNAKE_TURN_SLOT`] is pressed (held [`PRESS_HOLD_US`]); the frame
/// one move interval after release is pinned (compared with the badge's
/// "after one turn" photo, Task 5), and the snake does not hit the wall
/// at the no-input game-over time.
#[test]
fn snake_steers() {
    let mut rt = snake_open();
    let first = move_times(&mut rt, BADGE_MOVE_INTERVAL_US * 3 / 2);
    assert!(!first.is_empty(), "no first move");
    press(&mut rt, SNAKE_TURN_SLOT);
    run_for_us(&mut rt, BADGE_MOVE_INTERVAL_US);
    assert_eq!(framebuffer_fnv1a(rt.framebuffer()), SNAKE_TURNED_HASH);
    let more = move_times(&mut rt, SNAKE_TO_WALL_WINDOW_US / 2);
    assert!(more.len() >= 3, "kept moving after the turn: {more:?}");
    assert_no_panic_text(&rt);
}
```

`SNAKE_TURN_SLOT` is UP (7) or DOWN (4), whichever turns away from the start direction (from Task 4; slots per `RAW_SLOT_BY_BUTTON`: UP = 7, DOWN = 4, LEFT = 5, RIGHT = 6). Pin `SNAKE_TURNED_HASH` from a first run; if the hash differs from the frame the human approved in Task 5, stop and report. Run — Expected: PASS.

- [ ] **Step 4: Full check, commit.** Notes history entry. Commit `test: Snake moves at the badge's speed and steers`.

---

### Task 7: Browser pacing to wall-clock time

**Files:**
- Modify: `frontend/src/runtime/firmware-runtime.ts`, `frontend/test/firmware-runtime.test.ts`, `frontend/src/main.ts`

**Interfaces:**
- Consumes: `FirmwareEmulatorHandle.runUntil`, `.emulatedTimeUs`.
- Produces: `MAX_FRAME_DELTA_MS = 50`, `DEFAULT_STEPS_PER_FRAME = 1_000_000` (replaces `DEFAULT_CYCLES_PER_FRAME`), `frameTargetUs(emulatedUs: number, wallDeltaMs: number): number`, `stepFirmwareFrame(handle, ctx, wallDeltaMs: number, stepsPerFrame: number): FrameResult` with `interface FrameResult { emulatedDeltaUs: number; capped: boolean }`, `FirmwareRuntimeOptions.stepsPerFrame` (replaces `cyclesPerFrame`), `FirmwareRuntime.speed(): number` (smoothed emulated/wall ratio, `0` before the second frame).

- [ ] **Step 1: Write the failing tests** (replace the `stepFirmwareFrame` describe and the `DEFAULT_CYCLES_PER_FRAME` test; extend the fake so `runUntil` advances a fake clock: `emulatedTimeUs` returns a variable, `runUntil(target, max)` sets it to `Math.min(target, now + max * usPerStep)` with a configurable `usPerStep`, default 1 (so the default 1,000,000-step cap never binds in a test), and returns `steps` accordingly; tests that need a binding cap pass `usPerStep: 0.0125`, one step at 80 MHz):

```ts
describe("frameTargetUs", () => {
  test("advances emulated time by the wall delta", () => {
    expect(frameTargetUs(1_000, 16)).toBe(17_000);
  });
  test("frame target clamps the wall delta", () => {
    expect(frameTargetUs(0, 5_000)).toBe(MAX_FRAME_DELTA_MS * 1000);
    expect(frameTargetUs(500, -3)).toBe(500);
  });
});

describe("stepFirmwareFrame", () => {
  test("runs to the frame target and blits once", () => {
    const { handle, runUntilCalls } = makeFakeHandle();
    const { ctx, getPutImageDataCalls } = makeStubCtx();
    const r = stepFirmwareFrame(handle, ctx, 16, 1_000_000);
    expect(runUntilCalls).toEqual([[16_000, 1_000_000]]);
    expect(r).toEqual({ emulatedDeltaUs: 16_000, capped: false });
    expect(getPutImageDataCalls()).toBe(1);
  });
  test("a capped frame drops its debt", () => {
    const { handle } = makeFakeHandle({ usPerStep: 0.0125 });
    const { ctx } = makeStubCtx();
    // 100 steps at 80 MHz = 1.25 µs of the 16,000 µs asked for.
    const r = stepFirmwareFrame(handle, ctx, 16, 100);
    expect(r.capped).toBe(true);
    // The next frame targets now + 16 ms, not the missed time.
    stepFirmwareFrame(handle, ctx, 16, 100);
    expect(handle.emulatedTimeUs()).toBeCloseTo(2.5, 5);
  });
});

describe("FirmwareRuntime pacing", () => {
  test("the first frame runs nothing and later frames follow the wall clock", () => {
    const { rt, frameAt, fake } = pacedRig();
    rt.start();
    frameAt(1_000);
    expect(fake.emulatedTimeUs()).toBe(0);
    frameAt(1_016);
    expect(fake.emulatedTimeUs()).toBe(16_000);
  });
  test("speed() reports the emulated/wall ratio", () => {
    const { rt, frameAt } = pacedRig({ usPerStep: 0.0125, stepsPerFrame: 640_000 });
    rt.start();
    frameAt(0);
    for (let t = 16; t <= 16 * 60; t += 16) frameAt(t);
    // 640,000 steps x 0.0125 µs = 8,000 µs per 16 ms frame: 0.5x.
    expect(rt.speed()).toBeCloseTo(0.5, 1);
  });
  test("stop() flushes deferred releases", () => {
    const { rt, buttonCalls, frameAt } = pacedRig();
    rt.start();
    frameAt(0);
    rt.injectButton("A", "pressed");
    rt.injectButton("A", "released");
    rt.stop();
    expect(buttonCalls.at(-1)).toEqual([RAW_SLOT_BY_BUTTON.A, false]);
  });
});
```

`pacedRig(opts)` mirrors `rig()` but the queued callbacks are invoked with an explicit `timeMs` (`frameAt(t)` = `queued.shift()?.(t)`) and passes `stepsPerFrame` through. Write `makeFakeHandle`'s `runUntilCalls` recording and `usPerStep` option concretely. Run: `bun test test/firmware-runtime.test.ts` — Expected: FAIL.

- [ ] **Step 2: Implement.** In `firmware-runtime.ts`: replace the "Cycles-per-frame budget" module-doc section with a "Pacing" section (emulated time follows `requestAnimationFrame` time; delta clamped to `MAX_FRAME_DELTA_MS`; `DEFAULT_STEPS_PER_FRAME` caps the work per frame, a documented guess to tune by measuring the frame time in a real browser; when capped the debt is dropped, so the badge runs in slow motion, shown by `speed()`). Then:

```ts
export const MAX_FRAME_DELTA_MS = 50;
export const DEFAULT_STEPS_PER_FRAME = 1_000_000;

export function frameTargetUs(emulatedUs: number, wallDeltaMs: number): number {
  const delta = Math.min(Math.max(wallDeltaMs, 0), MAX_FRAME_DELTA_MS);
  return emulatedUs + delta * 1000;
}

export interface FrameResult {
  emulatedDeltaUs: number;
  capped: boolean;
}

export function stepFirmwareFrame(
  handle: FirmwareEmulatorHandle,
  ctx: CanvasRenderingContext2D,
  wallDeltaMs: number,
  stepsPerFrame: number,
): FrameResult {
  const before = handle.emulatedTimeUs();
  const target = frameTargetUs(before, wallDeltaMs);
  handle.runUntil(target, stepsPerFrame);
  const after = handle.emulatedTimeUs();
  blitFramebuffer(ctx, handle.framebuffer(), handle.screenWidth, handle.screenHeight);
  return { emulatedDeltaUs: after - before, capped: after < target };
}
```

In `createFirmwareRuntime`: keep `lastWallMs: number | null` (reset to `null` in `start()` so a resume does not count the paused time); `loop(timeMs)` computes `delta = lastWallMs === null ? 0 : timeMs - lastWallMs`, calls `stepFirmwareFrame`, and when `delta > 0` updates `speed = speed === 0 ? r : 0.9 * speed + 0.1 * r` with `r = emulatedDeltaUs / (Math.min(delta, MAX_FRAME_DELTA_MS) * 1000)`. `stop()` releases every pending deferred button (same loop as `reset()`'s) before clearing the maps' entries. Export `speed` on the runtime. `reset()` sets `speed = 0` and `lastWallMs = null`.

- [ ] **Step 3: Speed readout.** In `main.ts`'s 250 ms status interval, when `mode === "firmware"` and `firmwareRuntime` exists, prefix the status text with the speed: `const speed = firmwareRuntime.speed(); const prefix = speed > 0 ? \`${speed.toFixed(1)}x\` : ""; provisionControls.setStatus([prefix, text].filter(Boolean).join(" · "));`.

- [ ] **Step 4: Run.** `bun test && bun run typecheck` — Expected: pass.

- [ ] **Step 5: Browser check (human).** `bun run dev`; ask the human to provision a hacker badge, walk onboarding, open Snake from the launcher and play to game over; report the speed readout and whether it felt like the badge. Measure the frame time in devtools if the readout is below 0.8x and record it; tune `DEFAULT_STEPS_PER_FRAME` only with that measurement (record in the decisions doc).

- [ ] **Step 6: Commit.** `feat: pace the firmware emulator to wall-clock time`.

---

### Task 8 (stretch): Snake eats

**Files:**
- Modify: `emulator-core/tests/boot_progress.rs`

- [ ] **Step 1: Find the path.** In a temporary `#[ignore]` test, dump the open frame and locate the food (fixed-seed RNG, so it is deterministic); compute the button sequence (each press timed one move interval apart, held `PRESS_HOLD_US`) that brings the head onto it. If the food position depends on anything non-deterministic, drop the stretch and record why.

- [ ] **Step 2: The rung.** `snake_eats`: play the sequence from `snake_open()`, then assert the length grew (pinned frame hash after the eat, plus a pixel count of the snake colour in `PLAY_AREA` greater than at open). Document the sequence derivation. Run — Expected: PASS. Delete the temporary test.

- [ ] **Step 3: Full check, commit.** `test: Snake eats (stretch)`. If this costs more than one fix round, drop it and add it to the M7 backlog.

---

### Task 9: Docs, suite time, final review

**Files:**
- Modify: `docs/firmware-emulator-notes.md`, `CLAUDE.md`, `docs/milestone-6-decisions.md`

- [ ] **Step 1: Notes.** Add a "Timing model" section (clock, DFS 40/80, SYSTEM registers, idle fast-forward, USB packet time, emulated time and `run_until`, what CPI means and its value); update "Known limitations" current-state paragraphs with the new step figures (launcher, Dice, Snake), and mark the old step numbers in the history as pre-Milestone-6. Remove the resolved limitations (`CPU_FREQ_MHZ` 160, `TICKS_PER_STEP` 10x, Snake game over).

- [ ] **Step 2: CLAUDE.md.** Current-state paragraph (Snake playable; the clock), the architecture entries for `clock.rs`, `system.rs`, `rom.rs` (stateful frequency pair, no `CPU_FREQ_MHZ`), `runtime.rs` (`emulated_time_us`, `run_until`), `usb_serial_jtag.rs` (`PACKET_TICKS`), `firmware-runtime.ts` (pacing, `PRESS_HOLD_US`), the `boot_progress` rung list and suite timings.

- [ ] **Step 3: Decisions.** Finish `docs/milestone-6-decisions.md`: every ruling, the measurements tables, and the Milestone 7 backlog (carry forward the M5 backlog items not done here, plus anything new).

- [ ] **Step 4: Full check and commit.** `docs: Milestone 6 timing model and current state`.

- [ ] **Step 5: Final review.** Whole-branch review (most capable model) over `main..milestone-6` against the spec and this plan's Review Focus; fix rounds as needed; then superpowers:finishing-a-development-branch (merge and push are the human's call).

## Orchestrator notes

- **Execution method: subagent-driven** (`superpowers:subagent-driven-development`), chosen by the human partner on 2026-10-09. Not `executing-plans`. A fresh implementer subagent per task, a fresh reviewer subagent per task (spec compliance, then code quality), fix rounds until the reviewer approves, then the next task; a whole-branch review (most capable model) after Task 9.
- **Workspace:** before Task 1, create the worktree via `superpowers:using-git-worktrees` at `.claude/worktrees/milestone-6` on a new branch `milestone-6` from `main` (which holds the spec and this plan), copy the repo-root `local/` into it, `bun install` in `frontend/`, `bun run build:wasm`. All dispatched work happens in the worktree.
- **Ledger:** the skill's progress record is `local/m6-sdd-ledger.md` inside the worktree (gitignored), copied to the repo-root `local/` at the finish (as for M5). Start it with a preflight scan of the Interfaces blocks (producer vs consumer per task pair) and the rulings the scan needs. One line per dispatch, review verdict, fix round, ruling (`Ruling R-T<n>-<k>: <decision> — cost if wrong: <...>`), measurement (steps, hashes, suite times) and human answer. Every ruling that changes the design also goes into `docs/milestone-6-decisions.md` in the task's commit.
- **Order:** Task 1 → 2 → 3 (+ Task D instances as stalls appear) → 4 → 5 → 6 → 7 → 8 (stretch; drop after one failed fix round) → 9 → whole-branch review → `superpowers:finishing-a-development-branch` (the human partner decides merge/PR/push). Strictly sequential: each task consumes the previous task's pins. Do not merge Task 1 into Task 3: Task 1's "no number moved" is the refactor's equivalence proof.
- **Human gates (stop and ask; nothing later runs before the answer):** Task 3 Step 6, only if a frame hash that was compared with the physical badge moved (list every moved hash with PNG paths); Task 5 always (the badge video/photos questions, in one message); Task 5 Step 3 whenever the speed is outside ±25 % or a frame is rejected (no CPI change without the OK); Task 7 Step 5 always (the human plays Snake in the browser); a release `boot_progress` suite over 90 s (Task 3 Step 10); any override of a recorded decision (M3–M5 decisions docs). Ask all open questions at a gate in one message.
- **Implementer brief:** this plan's Global Constraints and Review Focus, the full task text, the Interfaces of the tasks it consumes from, the spec path, the relevant ledger lines (latest pins, rulings, measurements), and the worktree path. Paths only for anything under `local/`; never paste data from `local/full_flash_dump.bin`, `local/boot_log.txt` or the human's photos/video into a brief. Tell implementers that the shell's `grep` may silently return nothing (use `/usr/bin/grep`) and that ESP-IDF sources are fetched to the scratchpad, never the repo.
- **Implementer report:** commits made, the full check list's results (test counts, suite times, clippy count), every number or hash that changed (old → new, with reason), any stall or deviation from the task text, and what is left.
- **Reviewer checks (in addition to the skill's):** ESP-IDF citations present and correct for every register field; `clock.rs` and `cpu/` free of ESP32-C3 constants; no `TICKS_PER_STEP`/`PACKET_STEPS`/`CPU_FREQ_MHZ` left after their task; byte-split register writes handled; never-panic on invalid clock encodings and browser input; every re-pinned number/hash matches the implementer's old → new list and has a reason, and none of the hardware-compared hashes moved without a recorded human OK; no temporary `#[ignore]`/`local_*` exploration test, PNG, or `local/` file committed (`git show --stat` on the task's commits); docs (notes history entry, decisions doc) updated in the same commit; the full check list was actually run.
- **Model choice:** implementers and reviewers on the session's model; the final whole-branch review on the most capable model. Task 3 (the switch and re-pins) and Task 4 (Snake exploration) are the riskiest; give their reviewers the ledger's old → new tables.
- **Stalls** go through the Task D template, one stall per D task, each with a ledger ruling.
- **Data handling:** keep briefs free of `local/` personal data; the badge photos and video are the human's and stay local.

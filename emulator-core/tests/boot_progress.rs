//! Boot-progress ratchet: each test asserts the real firmware's console
//! reaches a known line (from the real badge's boot log) within a step
//! budget. Lines are generic ESP-IDF log lines only — never identity data.
//!
//! Task 3 status: TIMG0/TIMG1 (`crate::peripherals::timg`) unblocked
//! `rtc_clk_cal_internal()`'s spin loop (the pre-Task-3 stall — see Task 2's
//! `boot-probe` report), but boot still didn't print anything to the console
//! within 20,000,000 steps, so Task 3 added only the hot-PC-escape fallback
//! test ([`timg_calibration_escapes_the_pre_fix_rtc_clk_cal_spin_loop`]), per
//! its brief's explicit contingency for that case.
//!
//! Task D1 status: RTC_CNTL's RTC timer (`crate::peripherals::rtc_cntl`)
//! unblocked the *next* stall (a busy-wait on `rtc_cntl_ll_get_rtc_time()`),
//! and boot now genuinely reaches the console for the first time — a full
//! ESP-IDF panic dump ("Guru Meditation Error...", from a *later*,
//! not-yet-fixed stall this task explicitly leaves for the next one; see
//! `emulator-core/tests/rom_stub_boot.rs`'s
//! `boot_currently_stalls_retrying_reboot_via_an_unidentified_unstubbed_rom_call`
//! and `docs/firmware-emulator-notes.md`). [`first_console_output_is_the_firmware_s_own_panic_report`]
//! is this file's first real console-line rung, using
//! [`boot_until_console_contains`] below exactly as Task 3 anticipated.
//!
//! Task D2 status: ROM libc `memcpy` (`emulator-core/src/cpu/rom_stubs.rs`'s
//! `RomStubEffect::Memcpy`) unblocked the Task-D1-era fault at `0x4000_0358`,
//! but boot's first console line is still the same generic "Guru Meditation
//! Error" text — no *new* line to ratchet on, since the very next instruction
//! after the unblocked `memcpy` call runs straight into another unstubbed
//! ROM call (`ets_efuse_get_spiconfig`, `0x4000_071c` — see
//! `emulator-core/tests/rom_stub_boot.rs`'s
//! `boot_currently_stalls_on_the_unstubbed_ets_efuse_get_spiconfig_rom_call`).
//! Per this file's own contingency for that case (see Task 3's note above),
//! [`boot_no_longer_faults_at_the_pre_task_d2_memcpy_call_site`] is a
//! no-fault-before-step-N rung instead: it asserts zero traps through the
//! *old* Task-D1-era fault's step count, i.e. concrete, measurable evidence
//! that this task's fix moved the wall forward rather than just moving a
//! test's expected number.
//!
//! Task D3 status: `emulator_core::rom`'s module doc (entry 10) unblocked
//! the Task-D2-era fault at `0x4000_071c` (`ets_efuse_get_spiconfig`) and
//! six more ROM calls it led to in turn, but — same situation as Task D2 —
//! boot's console is still the same generic "Guru Meditation Error" text no
//! *new* line to ratchet on, since the chain runs straight into another
//! unstubbed ROM call (`esprv_intc_int_enable`, `0x4000_05e8` — see
//! `emulator-core/tests/rom_stub_boot.rs`'s
//! `boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call`).
//! [`boot_no_longer_faults_at_the_pre_task_d3_ets_efuse_get_spiconfig_call_site`]
//! is this file's no-fault-before-step-N rung for this task: it asserts zero
//! traps through the *old* Task-D2-era fault's exact step count (402,113),
//! concrete, measurable evidence this task's fixes moved the wall forward.
//! (Note this task's own boot/panic-order text was itself corrected in Fix
//! round 1 — see below and `docs/firmware-emulator-notes.md`: the whole
//! eFuse/UART/interrupt-controller chain, including the `esprv_intc_int_*`
//! calls, is normal *pre*-panic boot code, not something that ran "after" a
//! panic that hadn't actually happened yet.)
//!
//! Fix round 1 status (review of Task D3): the review found
//! `esprv_intc_int_enable` and its four siblings were writing real
//! `crate::peripherals::intc::InterruptController` registers only in
//! *theory* — the siblings were left as `void` no-ops, silently dropping
//! state a later interrupt-arbitration consumer could observe, and
//! `esprv_intc_int_enable` itself was left unstubbed entirely (the
//! Task-D3-era fault above). `emulator_core::rom`'s module doc (entry 11)
//! gives all five real register writes via a new generic
//! `RomStubEffect::BusRegisterWrite` (`emulator-core/src/cpu/rom_stubs.rs`).
//! Boot now runs straight past `0x4000_05e8` and hits a **new**, later,
//! genuinely different unstubbed ROM call: `itoa` (`0x4000_0448`, a ROM
//! libc function, not an interrupt-controller one — see
//! `emulator-core/tests/rom_stub_boot.rs`'s (then-current)
//! `boot_currently_stalls_on_the_unstubbed_itoa_rom_call`). Same situation
//! as Task D2/D3 once more — the console's first line is still the same
//! generic "Guru Meditation Error" text, no *new* line to ratchet on, since
//! this is still the very first fault of the run.
//! [`boot_no_longer_faults_at_the_pre_fix_round_1_esprv_intc_int_enable_call_site`]
//! is this file's no-fault-before-step-N rung for this fix round: it asserts
//! zero traps through the *old* Task-D3-era fault's exact step count
//! (405,806), concrete, measurable evidence this fix round's changes moved
//! the wall forward.
//!
//! Task D4 status: `emulator-core/src/rom.rs`'s module doc (entry 12) gives
//! `itoa` (`0x4000_0448`) and the very next unstubbed call it led to,
//! `strcat` (`0x4000_03d8`, also `esp32c3.rom.libc.ld`), real HLE
//! implementations. `itoa`/`strcat` now really execute (instead of
//! faulting mid-call), so a panic-message-formatting call *completes* for
//! the first time -- and per ESP-IDF's own `panic.c`, an abort-path panic
//! (which this is: a real `ILLEGAL_INSTRUCTION` trap at `panic_abort()`,
//! `0x4038e4fa`, not a ROM-call fault at all) skips the "Guru Meditation
//! Error" header on its first pass (`info->reason` is `NULL` for an abort)
//! and instead prints unconditionally, right before trying to reboot:
//! ESP-IDF's generic "Rebooting..." text (`components/esp_system/panic.c`).
//! The still-unstubbed `software_reset_cpu` ROM call the reboot attempt
//! makes then faults for real, re-entering the panic handler through its
//! *exception* path (where `info->reason` is finally non-`NULL`), which is
//! when "Guru Meditation Error" prints for the first time — so
//! [`first_console_output_is_the_firmware_s_own_panic_report`]'s budget
//! moves out to 750,000 (from 500,000) to stay past that later point.
//! [`boot_no_longer_faults_at_the_pre_task_d4_itoa_call_site`] is this
//! file's no-fault-before-step-N rung for this task: it asserts zero traps
//! through the *old* Fix-round-1-era fault's exact step count (407,471).
//!
//! **Task D4 fix round 1 status (correction, not a code change): the above
//! was misdiagnosed.** A review traced the actual call chain through
//! `factory.bin`'s own bytes and found `itoa`/`strcat` are called from
//! newlib's `abort()` (`components/newlib/abort.c`), which is in turn
//! called by `ets_printf("E (%lu) %s: Invalid app image header\n",
//! "cpu_start", ...)` — i.e. **`cpu_start` (ESP-IDF's early startup)
//! rejects this image's header and aborts.** The console showed nothing at
//! the time of the original `itoa`/`strcat` calls not because nothing had
//! been logged, but because `ets_printf` is stubbed `Return(0)` and never
//! reaches `Console` (`emulator-core/src/rom.rs`'s entry 7 caveat) — the
//! `cpu_start` error line was logged and silently dropped. **Boot has been
//! aborting on this header check since at least Task D3 fix round 1**;
//! neither Task D4 nor this correction moves that wall. So:
//! `boot_reaches_the_panic_handlers_reboot_message_for_the_first_time` is
//! renamed to [`boot_reaches_the_panic_handlers_reboot_message_via_cpu_starts_abort`]
//! and reframed below — "Rebooting..." is panic output from `cpu_start`'s
//! abort, not evidence of boot progress, and this rung is expected to
//! break (deliberately, not a regression) once a later task fixes the
//! header check and `abort()` is no longer called at all. The `itoa`/
//! `strcat` stubs themselves are correct and needed regardless — newlib's
//! `abort()` calls them on real hardware too. See Task D4 fix round 1's
//! report for the full corrected trace (register/caller evidence,
//! confirmed against the actual image bytes at every address cited).
//!
//! **Task 7 status**: `ets_printf` (`emulator_core::rom::ETS_PRINTF`) is no
//! longer `Return(0)` -- it's a real HLE formatter
//! (`emulator_core::cpu::rom_stubs::RomStubEffect::Printf`, see
//! `emulator-core/src/rom.rs`'s module doc, entry 7), so every
//! `ESP_EARLY_LOG*` line the firmware prints before `abort()` now reaches
//! the console, not just the panic path's raw MMIO writes. The header
//! check itself is unchanged (that's the next task's job, D5, not this
//! one's): boot still reaches `cpu_start`'s `E (%lu) %s: Invalid app image
//! header\n` call, but it's a real, console-visible early-log line now
//! instead of a silently-dropped one -- see
//! [`boot_reaches_cpu_starts_own_header_check_error_line`] below, this
//! file's first genuinely new *early-boot* console-line rung (as opposed
//! to a panic-report string). The spec's original ladder rungs
//! (`cpu_start: Pro cpu start user code`, `cpu_start: cpu freq:`) do
//! **not** appear -- boot-probe evidence (this task's report) confirms
//! `cpu_start`'s header check runs and fails before either would print --
//! so per this task's own orchestrator contingency for that case, the new
//! rung asserts the line boot *does* reach instead.
//!
//! **Task D5 status**: `crate::mem::bus::FirmwareBus::from_segments` now
//! widens each XIP (DROM/IROM) segment to its containing 64 KiB flash-cache
//! MMU page (see that function's doc comment), matching what the real
//! 2nd-stage bootloader's `set_cache_and_start_app()` +
//! `mmu_hal_map_region()` actually expose. `cpu_start`'s app-image-header
//! check (`components/esp_system/port/cpu_start.c`) reads the header from
//! exactly this newly-exposed leading page gap, so it now reads the real
//! magic byte (`0xE9`) instead of a catch-all `0`, and **passes** --
//! `cpu_start: Invalid app image header` no longer appears anywhere in the
//! console, and boot reaches several new, genuinely later lines this file
//! had never seen before: `cpu_start: Pro cpu start user code` (step
//! 407,528), `cpu_start: cpu freq: 160000000 Hz` (407,586), a full
//! `app_init`/`efuse_init` block (project name, version, compile time, SHA256,
//! ESP-IDF version, min/max/actual chip revision), ending at `efuse_init:
//! Chip rev: v0.0` (408,344). [`boot_reaches_cpu_starts_own_header_check_error_line`]
//! is retired (renamed to [`boot_reaches_efuse_inits_chip_rev_line`] below,
//! its replacement rung) since the line it pinned no longer prints at all.
//!
//! Boot then hits a **new, different, genuinely unrelated stall**: an
//! unstubbed ROM `qsort` call (`0x4000_0434`, `esp32c3.rom.libc.ld`) --
//! confirmed by symbol address, not guessed; register dump shows a
//! `nmemb`/`size`-shaped call (`A1=5, A2=8`) with a return address inside
//! the app's own `esp_system` startup code, consistent with ESP-IDF's
//! `do_system_init_fn()` sorting its init-function array before running it.
//! This is a plain missing-ROM-stub gap (this task's own scope ruling: "Stop
//! at anything else and report it" -- a new unstubbed ROM call is not
//! another header-field check in `cpu_start`), left for the next task. The
//! resulting `INSTRUCTION_ACCESS_FAULT` (step 408,481) is a genuine
//! hardware-standard exception this time (not an `abort()`), so ESP-IDF's
//! panic handler takes its *exception* path immediately -- `info->reason`
//! is non-`NULL` from the very first pass, so "Guru Meditation Error"
//! prints right away (step 410,197) instead of only after a failed reboot
//! retry. [`first_console_output_is_the_firmware_s_own_panic_report`]'s doc
//! is corrected below to describe this new cause (the assertion/budget
//! still held even before this correction, since both the old and new
//! causes print the same generic string). The still-unstubbed ROM
//! `software_reset_cpu` (`0x4000_0094`) then faults on the reboot attempt
//! exactly as before (step within the existing 650,000 budget -- see
//! `emulator-core/tests/rom_stub_boot.rs`'s renamed pinned-stall test), so
//! [`boot_reaches_the_panic_handlers_reboot_message_via_cpu_starts_abort`]
//! is renamed to
//! [`boot_reaches_the_panic_handlers_reboot_message_via_the_unstubbed_qsort_fault`]
//! below -- same budget, corrected narrative.
use emulator_core::runtime::FirmwareRuntime;
use std::collections::HashMap;
use std::path::PathBuf;

fn factory() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/public/firmware/factory.bin"),
    )
    .expect("factory.bin")
}

/// Runs the real firmware in `CHUNK`-step increments (checking the console
/// after each) until either `needle` appears in [`FirmwareRuntime::console_output`]
/// or `max_steps` is exhausted. Shared plumbing for every console-line
/// ratchet test in this file, kept `pub` per Task 3's brief for later tasks
/// to call directly too.
pub fn boot_until_console_contains(needle: &str, max_steps: u64) -> (FirmwareRuntime, bool) {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    const CHUNK: u32 = 250_000;
    while rt.total_steps() < max_steps {
        rt.run(CHUNK);
        if rt.console_output().contains(needle) {
            return (rt, true);
        }
    }
    (rt, false)
}

/// Asserts `needle` is reached within `max_steps`, printing the final pc and
/// full console buffer on failure for debugging.
fn assert_reaches(needle: &str, max_steps: u64) {
    let (rt, ok) = boot_until_console_contains(needle, max_steps);
    assert!(
        ok,
        "never printed {needle:?} within {max_steps} steps; pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
}

/// The pre-Task-3 stall signature, measured in Task 2's `boot-probe` report:
/// hot PCs confined to this exact 9-address range (a tight spin loop inside
/// `rtc_clk_cal_internal`, polling TIMG0's then-unmodeled `RTCCALICFG_REG`/
/// `RTCCALICFG1_REG` and getting a constant `0` back forever) — each of the
/// 9 addresses was hit ~22,222 times in Task 2's last-200,000-step trace
/// window. This range is still *legitimately* executed once per real boot
/// (the same poll loop, now succeeding on its first check instead of
/// spinning), so the ratchet below asserts it's no longer a *spin*
/// (bounded, small hit count per address), not that it's never visited at
/// all.
const PRE_FIX_RTC_CLK_CAL_LOOP: std::ops::RangeInclusive<u32> = 0x4038c80c..=0x4038c822;

/// A single PC being hit this many times or more within the traced window
/// is unambiguously still a spin (pre-fix: ~22,222; post-fix, measured this
/// task: each address in range hit exactly once over the same window) — see
/// the module doc.
const SPIN_THRESHOLD: u64 = 1_000;

#[test]
fn timg_calibration_escapes_the_pre_fix_rtc_clk_cal_spin_loop() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    let mut hist: HashMap<u32, u64> = HashMap::new();
    // 3,000,000 steps: the same budget Task 2's boot-probe report used to
    // measure the pre-fix stall, so this is an apples-to-apples comparison.
    rt.run_traced(3_000_000, &mut hist);

    let max_hits_in_pre_fix_range = PRE_FIX_RTC_CLK_CAL_LOOP
        .clone()
        .filter_map(|pc| hist.get(&pc).copied())
        .max()
        .unwrap_or(0);

    assert!(
        max_hits_in_pre_fix_range < SPIN_THRESHOLD,
        "still spinning in the pre-Task-3 rtc_clk_cal loop (0x{:08x}..=0x{:08x}): \
         max single-PC hit count in that range = {max_hits_in_pre_fix_range} \
         (pre-fix was ~22222 per address, SPIN_THRESHOLD = {SPIN_THRESHOLD}); pc=0x{:08x}",
        PRE_FIX_RTC_CLK_CAL_LOOP.start(),
        PRE_FIX_RTC_CLK_CAL_LOOP.end(),
        rt.pc(),
    );
}

/// A console-line ratchet — **still panic output, not boot progress, but
/// for a different reason than before.** "Guru Meditation Error" is
/// ESP-IDF's own generic panic-header string (`components/esp_system/
/// panic.c`), present in every ESP-IDF crash report — not badge-specific or
/// identity data.
///
/// **Corrected in Task D5** (this test's assertion/budget didn't need to
/// change, but its cause did, so its doc would otherwise now be wrong):
/// this line used to be reached via `cpu_start`'s header-check `abort()`
/// (see the pre-Task-D5 history below). Task D5's fix
/// (`crate::mem::bus::FirmwareBus::from_segments`'s page-granular XIP
/// mapping) makes that header check pass, so `abort()` is never called any
/// more. Boot instead runs into a **new, unrelated** stall -- an unstubbed
/// ROM `qsort` call (`0x4000_0434`; see this file's module doc's "Task D5
/// status") -- whose `INSTRUCTION_ACCESS_FAULT` is a genuine hardware
/// exception, not an `abort()`, so ESP-IDF's panic handler takes its
/// *exception* path immediately (`info->reason` non-`NULL` from the first
/// pass), printing "Guru Meditation Error" right away (measured at step
/// 410,197) instead of only after a failed reboot retry.
///
/// **Pre-Task-D5 history** (kept for context): 500,000 steps used to be
/// comfortably past this (measured at step 401,761, back when `itoa`
/// faulted mid-format and the truncated abort path printed this header
/// directly); after Task D4 made `itoa`/`strcat` real, the header only
/// appeared once `cpu_start`'s abort path's own reboot attempt faulted for
/// real (measured at step 648,000), hence the 750,000 budget. Both
/// superseded by the cause above; 750,000 remains a comfortable budget for
/// the new cause too (measured at 410,197, well under half the budget).
#[test]
fn first_console_output_is_the_firmware_s_own_panic_report() {
    assert_reaches("Guru Meditation Error", 750_000);
}

/// A console-line ratchet on the panic handler's reboot attempt — **not** a
/// boot-progress rung. **Renamed in Task D5** (from
/// `boot_reaches_the_panic_handlers_reboot_message_via_cpu_starts_abort`):
/// with the header check now passing (see this file's module doc's "Task
/// D5 status"), `cpu_start` never calls `abort()` any more, so this is no
/// longer reached "via cpu_start's abort" -- it's reached via the *new*
/// unstubbed-`qsort`-call panic's own reboot attempt instead (same
/// generic ESP-IDF pre-restart text, `components/esp_system/panic.c`,
/// printed unconditionally right before `panic_restart()`; not
/// badge-specific or identity data). Measured reaching the console at step
/// 648,457 -- close to the pre-Task-D5 measurement (645,410) purely by
/// coincidence of similar code-path length, not because the cause is the
/// same.
///
/// **Fix round 1, M3**: the original 650,000 budget was mislabeled
/// "comfortable" when its actual margin over 648,457 was 1,543 steps
/// (~0.2%) -- genuinely tight, not comfortable. Bumped to 660,000 (margin
/// 11,543 steps, ~1.8%), still comfortably clear of the next event in this
/// trace (the panic handler's own reboot-retry fault, measured at step
/// 649,072 -- see `emulator-core/tests/rom_stub_boot.rs`'s renamed pinned-
/// stall test, which pins that fault step directly) and of the loop's next
/// iteration (measured at step 889,556).
///
/// This rung is **deliberately expected to break** once a later task fixes
/// (or stubs around) the unstubbed `qsort` ROM call: at that point this
/// panic path is never reached either, and whoever makes that fix should
/// delete or replace this test rather than chase a new pinned value here.
#[test]
fn boot_reaches_the_panic_handlers_reboot_message_via_the_unstubbed_qsort_fault() {
    assert_reaches("Rebooting...", 660_000);
}

/// Milestone 3 Task D2's no-new-console-line fallback rung (see the module
/// doc). Before this task, boot faulted on an unstubbed ROM `memcpy` call at
/// a fixed address reached at step 401,761 from a cold boot (Task D1
/// report). `emulator-core/src/cpu/rom_stubs.rs`'s `RomStubEffect::Memcpy`
/// now intercepts that call for real, so a run just past the old fault's
/// step count should show **zero** traps of any kind -- concrete, measured
/// evidence the fix bought real forward progress, not just a relabeled
/// stall. (Boot did still stall shortly after -- at step 402,113, on a
/// *different*, unstubbed ROM call, `ets_efuse_get_spiconfig` -- that was
/// Task D3's stall to fix, not this one's; see this file's next rung.)
#[test]
fn boot_no_longer_faults_at_the_pre_task_d2_memcpy_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    // 401,761: the exact step the pre-Task-D2 `memcpy` fault occurred at.
    // Running exactly that many steps and finding zero traps proves the
    // call that used to fault right at this boundary now completes.
    let summary = rt.run(401_761);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-Task-D2 memcpy fault's exact \
         step count (401,761) now that memcpy is HLE-stubbed; got \
         {} traps, last_instruction_fault = {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// Milestone 3 Task D3's no-new-console-line fallback rung (see the module
/// doc). Before this task, boot faulted on an unstubbed
/// `ets_efuse_get_spiconfig` call at a fixed address reached at step 402,113
/// from a cold boot (Task D2 report). `emulator-core/src/rom.rs`'s module
/// doc (entry 10) now stubs that call, and six more it led to, for real, so
/// a run just past the old fault's step count should show **zero** traps of
/// any kind -- concrete, measured evidence the fixes bought real forward
/// progress, not just a relabeled stall. (Boot does still stall shortly
/// after -- at step 405,806, on a *different*, unstubbed ROM call,
/// `esprv_intc_int_enable` -- pinned exactly by
/// `emulator-core/tests/rom_stub_boot.rs`'s
/// `boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call`,
/// which is this task's job to leave accurately pinned, not this rung's.)
#[test]
fn boot_no_longer_faults_at_the_pre_task_d3_ets_efuse_get_spiconfig_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    // 402,113: the exact step the pre-Task-D3 `ets_efuse_get_spiconfig`
    // fault occurred at. Running exactly that many steps and finding zero
    // traps proves the call that used to fault right at this boundary now
    // completes.
    let summary = rt.run(402_113);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-Task-D3 \
         ets_efuse_get_spiconfig fault's exact step count (402,113) now \
         that it's HLE-stubbed; got {} traps, last_instruction_fault = \
         {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// Fix round 1's no-new-console-line fallback rung (see the module doc).
/// Before this fix round, boot faulted on an unstubbed
/// `esprv_intc_int_enable` call at a fixed address reached at step 405,806
/// from a cold boot (Task D3 report). `emulator-core/src/rom.rs`'s module
/// doc (entry 11) now gives that call, and its four siblings, a real
/// register write via `RomStubEffect::BusRegisterWrite`, so a run just past
/// the old fault's step count should show **zero** traps of any kind --
/// concrete, measured evidence the fixes bought real forward progress, not
/// just a relabeled stall. (Boot does still stall shortly after -- at step
/// 407,471, on a *different*, unstubbed ROM call, `itoa` -- pinned exactly
/// by `emulator-core/tests/rom_stub_boot.rs`'s
/// `boot_currently_stalls_on_the_unstubbed_itoa_rom_call`, which is this fix
/// round's job to leave accurately pinned, not this rung's.)
#[test]
fn boot_no_longer_faults_at_the_pre_fix_round_1_esprv_intc_int_enable_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    // 405,806: the exact step the pre-fix-round-1 `esprv_intc_int_enable`
    // fault occurred at. Running exactly that many steps and finding zero
    // traps proves the call that used to fault right at this boundary now
    // completes.
    let summary = rt.run(405_806);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-fix-round-1 \
         esprv_intc_int_enable fault's exact step count (405,806) now that \
         it's HLE-stubbed; got {} traps, last_instruction_fault = {:?}, \
         pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// Milestone 3 Task D4's no-new-console-line-yet fallback rung (see the
/// module doc) -- though this task's own
/// [`boot_reaches_the_panic_handlers_reboot_message_via_cpu_starts_abort`]
/// rung above *does* have a new console line to ratchet on, so this one is
/// belt-and-suspenders: a fine-grained, step-exact proof the old fault site
/// specifically is gone, independent of anything downstream. Before this
/// task, boot faulted on an unstubbed `itoa` call at a fixed address
/// reached at step 407,471 from a cold boot (Fix round 1's report).
/// `emulator-core/src/cpu/rom_stubs.rs`'s `RomStubEffect::Itoa` now
/// intercepts that call for real, so a run just past the old fault's step
/// count should show **zero** traps of any kind -- concrete, measured
/// evidence this specific ROM-call fault is gone. **This is not evidence
/// of boot progress past `cpu_start`'s header check** (see
/// `emulator-core/src/rom.rs`'s module doc, entry 12, corrected in Task D4
/// fix round 1): the `itoa` call this rung is about was always going to
/// succeed once stubbed, since it's newlib's `abort()` formatting a
/// message for a pre-existing, unrelated header-check failure. (Boot does
/// still fault shortly after -- at step 407,549, on a real
/// `ILLEGAL_INSTRUCTION` trap at ESP-IDF's own `panic_abort()`, reached
/// from that same `abort()` call, not an unstubbed ROM call at all --
/// pinned exactly by `emulator-core/tests/rom_stub_boot.rs`'s
/// `boot_currently_aborts_reaching_the_panic_handlers_reboot_message`,
/// which is this task's job to leave accurately pinned, not this rung's.)
#[test]
fn boot_no_longer_faults_at_the_pre_task_d4_itoa_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    // 407,471: the exact step the pre-Task-D4 `itoa` fault occurred at.
    // Running exactly that many steps and finding zero traps proves the
    // call that used to fault right at this boundary now completes.
    let summary = rt.run(407_471);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-Task-D4 itoa fault's exact \
         step count (407,471) now that it's HLE-stubbed; got {} traps, \
         last_instruction_fault = {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// Milestone 3 Task D5's ratchet rung -- **replaces** Task 7's
/// `boot_reaches_cpu_starts_own_header_check_error_line`, retired here
/// because the line it pinned (`cpu_start: Invalid app image header`) no
/// longer ever prints: Task D5's fix
/// (`crate::mem::bus::FirmwareBus::from_segments`'s page-granular XIP
/// mapping, see this file's module doc's "Task D5 status") makes
/// `cpu_start`'s header check pass for real.
///
/// This is this file's first genuinely new *early-boot* console-line rung
/// since Task 7's (as opposed to a panic-report string): the spec's
/// original ladder rungs Task 7 predicted wouldn't be reached
/// (`cpu_start: Pro cpu start user code`, `cpu_start: cpu freq:`) now are,
/// plus a full `app_init`/`efuse_init` block this emulator had never
/// printed before, ending at `efuse_init: Chip rev: v0.0` (measured
/// reaching the console at step 408,344 -- just before the new unstubbed
/// `qsort` stall's fault at step 408,481). 420,000 keeps this comfortably
/// past that with margin, well short of the panic path's own budgets
/// below. Also asserts the old, now-permanently-wrong line never appears
/// again -- the direct, positive confirmation that the header check is
/// actually passing, not just that boot reached some later point by
/// coincidence.
#[test]
fn boot_reaches_efuse_inits_chip_rev_line() {
    let (rt, ok) = boot_until_console_contains("efuse_init: Chip rev:", 420_000);
    assert!(
        ok,
        "never printed \"efuse_init: Chip rev:\" within 420000 steps; pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    assert!(
        !rt.console_output().contains("Invalid app image header"),
        "cpu_start's header check should now pass for real -- this line \
         must never appear; got console:\n{}",
        rt.console_output()
    );
}

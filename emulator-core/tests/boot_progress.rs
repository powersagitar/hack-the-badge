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
//! five more ROM calls it led to in turn, but — same situation as Task D2 —
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
//! `emulator-core/tests/rom_stub_boot.rs`'s
//! `boot_currently_stalls_on_the_unstubbed_itoa_rom_call`). Same situation
//! as Task D2/D3 once more — the console's first line is still the same
//! generic "Guru Meditation Error" text, no *new* line to ratchet on, since
//! this is still the very first fault of the run.
//! [`boot_no_longer_faults_at_the_pre_fix_round_1_esprv_intc_int_enable_call_site`]
//! is this file's no-fault-before-step-N rung for this fix round: it asserts
//! zero traps through the *old* Task-D3-era fault's exact step count
//! (405,806), concrete, measurable evidence this fix round's changes moved
//! the wall forward.
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

/// The first real console-line ratchet: modeling RTC_CNTL's RTC timer
/// (Task D1) lets boot's `rtc_cntl_ll_get_rtc_time()`-based busy-wait
/// actually terminate, and boot now runs far enough to print real text —
/// specifically, a full ESP-IDF panic dump from the *next* stall (an
/// unstubbed ROM call; see `emulator-core/tests/rom_stub_boot.rs`'s
/// `boot_currently_stalls_retrying_reboot_via_an_unidentified_unstubbed_rom_call`).
/// "Guru Meditation Error" is ESP-IDF's own generic panic-header string
/// (`components/esp_system/panic.c`), present in every ESP-IDF crash
/// report — not badge-specific or identity data. 500,000 steps is
/// comfortably past the fault (measured at step 401,761 in a throwaway
/// experiment) and matches the exact budget
/// `boot_currently_stalls_retrying_reboot_via_an_unidentified_unstubbed_rom_call`
/// uses, for an apples-to-apples comparison between the two tests.
#[test]
fn first_console_output_is_the_firmware_s_own_panic_report() {
    assert_reaches("Guru Meditation Error", 500_000);
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
/// doc (entry 10) now stubs that call, and five more it led to, for real, so
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

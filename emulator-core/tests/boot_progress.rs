//! Boot-progress ratchet for the real firmware (`firmware.bin`, shortcut
//! boot, blank synthetic flash). Each rung asserts one of:
//!
//! - a console line within a step budget (generic ESP-IDF log lines only,
//!   never identity data; post-scheduler timestamps are left out because
//!   they move with boot timing);
//! - a no-fault point: no trap, or no exception, through a step count past
//!   a fault an earlier fix removed;
//! - a hot-PC escape: a former spin loop now runs a bounded number of times;
//! - a framebuffer state, pinned by FNV-1a hash
//!   ([`boots_to_first_real_frame`], [`boots_to_first_run_screen`],
//!   [`boots_to_launcher`]).
//!
//! The finish line (Milestone 5) is provisioning plus the launcher: every
//! `provisions_<role>_through_the_console` rung (the badge is provisioned
//! through its own USB console with a committed fake identity,
//! `common::provision::identity_fixture`, and walked through onboarding to
//! My Badge's registered screen), [`boots_to_launcher`] (HOME opens the
//! launcher) and [`launcher_responds_to_navigation`]. Milestone 4's finish
//! line ([`boots_to_first_run_screen`] plus
//! [`first_run_screen_responds_to_start`]) still holds on blank flash.
//!
//! Rungs that share a long walk continue from a checkpoint (a
//! `FirmwareRuntime` clone cached in a `OnceLock`; see "Checkpoints"
//! below), so each walk runs once per test binary; every rung still
//! asserts its whole walk. Step numbers in the docs below are measured
//! with today's emulator unless marked otherwise; a budget is a margin, not
//! a pin. The stall-by-stall history behind each rung is in
//! `docs/firmware-emulator-notes.md` ("History"); when boot moves, update
//! the rung whose budget it affects.
//!
//! Milestone 6 (emulated time follows the firmware's CPU frequency, and the
//! RTC timer counts that time faithfully) moved every early-boot event 221
//! steps later than at `00dd7d5`, from `rtc_init()`'s first log line to the
//! idle `wfi`. A rung pinned at a pre-Milestone-6 fault step runs to that
//! step plus those 221 steps, so it still covers the same code; its doc
//! keeps the historical step. Windows and budgets given in round numbers
//! are unchanged.
//!
//! Milestone 9 swapped the image from the dumped v335 to the official v392
//! (`v0.1.2-392-gd3089c4`) and re-pinned every rung that moved; each
//! change and its reason is in `docs/milestone-9-decisions.md`, "Re-pins
//! (v335 → v392)". Step numbers in the docs below written with "~" are
//! v392's; a measured number that moved is given for both versions.
//! Every comparison with the physical badge named below was made on v335
//! (the human partner's badge stays on v335); a v392 frame with the same
//! hash is the same frame, and v392's changed frames were checked by eye
//! in Milestone 9, never on hardware.
use emulator_core::runtime::FirmwareRuntime;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

mod common;
use common::provision::{self, Outcome};

fn firmware() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/public/firmware/firmware.bin"),
    )
    .expect("firmware.bin")
}

/// Runs the real firmware in 250,000-step chunks, checking the console after
/// each, until `needle` appears in [`FirmwareRuntime::console_output`] or
/// `max_steps` is exhausted. Returns the runtime and whether it appeared.
pub fn boot_until_console_contains(needle: &str, max_steps: u64) -> (FirmwareRuntime, bool) {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
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

/// `rtc_clk_cal_internal()`'s poll of TIMG0's `RTCCALICFG_REG`/
/// `RTCCALICFG1_REG` (Milestone 2's last stall: ~22,222 hits per address
/// over 200,000 steps). Boot still runs it once, so the rung checks it does
/// not spin rather than that it is never visited.
const PRE_FIX_RTC_CLK_CAL_LOOP: std::ops::RangeInclusive<u32> = 0x4038c80c..=0x4038c822;

/// A single PC hit this many times or more within a trace window is a spin
/// (the pre-fix rtc_clk_cal loop: ~22,222; fixed: once per address).
const SPIN_THRESHOLD: u64 = 1_000;

#[test]
fn timg_calibration_escapes_the_pre_fix_rtc_clk_cal_spin_loop() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
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

/// The SYSTEM `FROM_CPU_0` software interrupt reaches the core: the first
/// trap of the whole boot is vPortYield's request, taken on the next step
/// as an interrupt on CPU line 4 (where the firmware routes
/// `ETS_FROM_CPU_INTR0_SOURCE`), and no exception follows through step
/// 571,933 (v335's pre-Task-D11 `gpio_matrix_out` fault step, 571,712,
/// plus the 221-step shift; kept as a no-fault horizon). The
/// scheduler-start spin at `0x4200_0cd2` is gone.
///
/// v392 (Milestone 9): the request is the `sw` of 1 to
/// `SYSTEM_CPU_INTR_FROM_CPU_0_REG` (`0x600c_0028`) at `0x4038_9662`,
/// executed on step 527,878 (528,312 on v335); the trap is taken on step
/// 527,879 with `mepc` = `0x4038_9664`, the next instruction. The
/// scheduler-start `j .` after the call to `vTaskStartScheduler` is still
/// at `0x4200_0cd2` in v392 (same bytes as v335's).
#[test]
fn first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(527_878);
    assert_eq!(summary.traps, 0, "got {summary:?}, pc=0x{:08x}", rt.pc());
    let summary = rt.run(1);
    assert_eq!(summary.traps, 1, "the yield must be taken on the next step");
    assert_eq!(rt.cpu().csr.mcause, 0x8000_0004);
    let summary = rt.run(571_933 - 527_879);
    assert_eq!(
        summary.last_instruction_fault, None,
        "no exception through step 571,933 (the pre-Task-D11 gpio_matrix_out fault was next); got {summary:?}"
    );
    assert_ne!(
        rt.pc(),
        0x4200_0cd2,
        "the scheduler-start spin must be gone"
    );
}

/// The first context switch works, so FreeRTOS runs `main_task`, which
/// prints `main_task: Started on CPU0` and `main_task: Calling app_main()`
/// (step ~555,000; both are in the physical badge's log too).
#[test]
fn boot_reaches_main_task_calling_app_main() {
    let (rt, ok) = boot_until_console_contains("I (6) main_task: Calling app_main()", 750_000);
    let console = rt.console_output();
    assert!(ok, "pc=0x{:08x}\nconsole:\n{console}", rt.pc());
    assert!(
        console.contains("I (6) main_task: Started on CPU0"),
        "console:\n{console}"
    );
}

/// ROM `gpio_matrix_out`/`gpio_matrix_in` are real stubs, so `app_main`'s
/// SPI bus setup runs: through step 596,000 (past the old fault on step
/// 571,713) no exception is taken (the last trap is an interrupt), no panic
/// text prints, and `main_task: Calling app_main()` is in the console.
#[test]
fn boot_no_longer_faults_or_panics_at_the_pre_task_d11_gpio_matrix_out_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(596_000);
    assert_eq!(
        summary.last_instruction_fault,
        None,
        "got {summary:?}, pc=0x{:08x}",
        rt.pc()
    );
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt, not an exception; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    let console = rt.console_output();
    assert!(
        console.contains("I (6) main_task: Calling app_main()"),
        "console:
{console}"
    );
    for panic_text in ["Guru Meditation Error", "abort()", "Rebooting..."] {
        assert!(
            !console.contains(panic_text),
            "console:
{console}"
        );
    }
}

/// ROM `memcpy` is a real stub: zero traps through step 401,982, the
/// step the unstubbed call used to fault at (401,761) plus the 221-step
/// Milestone 6 shift.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d2_memcpy_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    // 401,761: the exact step the pre-Task-D2 `memcpy` fault occurred at,
    // 401,982 after Milestone 6's 221-step shift. Running exactly that
    // many steps and finding zero traps proves the call that used to fault
    // right at this boundary now completes.
    let summary = rt.run(401_982);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-Task-D2 memcpy fault's exact \
         step count (401,761, now 401,982) now that memcpy is HLE-stubbed; got \
         {} traps, last_instruction_fault = {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// ROM `ets_efuse_get_spiconfig` and the six ROM calls after it are real
/// stubs: zero traps through step 402,334, the step the first used to
/// fault at (402,113) plus the 221-step Milestone 6 shift.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d3_ets_efuse_get_spiconfig_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    // 402,113: the exact step the pre-Task-D3 `ets_efuse_get_spiconfig`
    // fault occurred at, 402,334 after Milestone 6's 221-step shift. Running
    // exactly that many steps and finding zero
    // traps proves the call that used to fault right at this boundary now
    // completes.
    let summary = rt.run(402_334);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-Task-D3 \
         ets_efuse_get_spiconfig fault's exact step count (402,113, now 402,334) now \
         that it's HLE-stubbed; got {} traps, last_instruction_fault = \
         {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// `esprv_intc_int_enable` and its siblings write the interrupt-controller
/// registers for real: zero traps through step 406,027, the step the
/// unstubbed call used to fault at (405,806) plus the 221-step
/// Milestone 6 shift.
#[test]
fn boot_no_longer_faults_at_the_pre_fix_round_1_esprv_intc_int_enable_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    // 405,806: the exact step the pre-fix-round-1 `esprv_intc_int_enable`
    // fault occurred at, 406,027 after Milestone 6's 221-step shift. Running
    // exactly that many steps and finding zero
    // traps proves the call that used to fault right at this boundary now
    // completes.
    let summary = rt.run(406_027);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-fix-round-1 \
         esprv_intc_int_enable fault's exact step count (405,806, now 406,027) now that \
         it's HLE-stubbed; got {} traps, last_instruction_fault = {:?}, \
         pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// ROM `itoa`/`strcat` are real stubs: zero traps through step 407,692,
/// the step the unstubbed `itoa` used to fault at (407,471) plus the
/// 221-step Milestone 6 shift. (It was called by newlib's `abort()`,
/// not by normal boot; the abort's cause was fixed later.)
#[test]
fn boot_no_longer_faults_at_the_pre_task_d4_itoa_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    // 407,471: the exact step the pre-Task-D4 `itoa` fault occurred at,
    // 407,692 after Milestone 6's 221-step shift. Running exactly that many steps
    // and finding zero traps proves the
    // call that used to fault right at this boundary now completes.
    let summary = rt.run(407_692);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through the pre-Task-D4 itoa fault's exact \
         step count (407,471, now 407,692) now that it's HLE-stubbed; got {} traps, \
         last_instruction_fault = {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
}

/// `cpu_start`'s app-image-header check passes (the header bytes before the
/// DROM segment are mapped, today through the flash MMU), so boot prints the
/// `app_init`/`efuse_init` block, ending at `efuse_init: Chip rev:` (step
/// ~409,000), and `Invalid app image header` never appears.
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

/// The ROM frequency pair is stateful, so `cpu_start` reports the
/// frequency the firmware selected: 80 MHz, as in the physical badge's
/// boot log (Milestone 6 Task 2; it printed 160 MHz from a constant stub
/// before). Printed at step ~407,500.
#[test]
fn boot_reports_the_badges_80_mhz_cpu_clock() {
    assert_reaches("cpu_start: cpu freq: 80000000 Hz", 500_000);
}

/// ROM `qsort` is guest-executed code (`emulator_core::rom::QSORT_BODY`):
/// after exactly 408,988 steps its `ret` has run and `pc` is back in its
/// caller, `s_prepare_reserved_regions()`, with zero traps. Step-exact:
/// it moves whenever anything before `qsort` changes.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d6_qsort_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(408_988);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through ROM qsort's return (408,988 steps); got \
         {} traps, last_instruction_fault = {:?}, pc = 0x{:08x}",
        summary.traps, summary.last_instruction_fault, summary.pc
    );
    assert_eq!(
        rt.pc(),
        0x4200_2a18,
        "qsort must have returned to its caller in s_prepare_reserved_regions()"
    );
}

/// The ROM layout table (`ets_rom_layout_p`) is backed, so
/// `s_prepare_reserved_regions()`'s overlap check passes: `heap_init`'s
/// first line prints (step ~410,000) and no `memory_layout` error does.
#[test]
fn boot_reaches_heap_inits_first_line_past_the_reserved_region_check() {
    let (rt, ok) = boot_until_console_contains(
        "I (5) heap_init: Initializing. RAM available for dynamic allocation:",
        420_000,
    );
    assert!(
        ok,
        "never printed heap_init's first line within 420000 steps; pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    assert!(
        !rt.console_output().contains("memory_layout"),
        "the reserved-region overlap error must never print again; got console:\n{}",
        rt.console_output()
    );
}

/// libgcc `__clzsi2`/`__ffssi2` are real stubs, so `heap_init` walks its
/// whole region list and prints its last line, the `RTCRAM` region (step
/// ~416,000; in the physical badge's log too).
#[test]
fn boot_reaches_heap_inits_last_region_line_past_the_libgcc_helpers() {
    assert_reaches(
        "I (5) heap_init: At 50000020 len 00001FC8 (7 KiB): RTCRAM",
        500_000,
    );
}

/// `esp_rom_newlib_init_common_mutexes` and the libc calls after it are
/// real stubs: zero traps through step 440,000, past the old fault on step
/// 417,992.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d9_newlib_init_common_mutexes_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(440_000);
    assert_eq!(
        summary.traps,
        0,
        "boot must run past the old newlib-init fault (step 417,992) with no \
         traps; got {summary:?}, pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    assert!(
        rt.console_output().contains("I (5) heap_init: At 50000020"),
        "heap_init's last region line should be present"
    );
}

/// The SPI1 flash controller answers JEDEC RDID with the badge's ID, so
/// flash-chip detection succeeds (`spi_flash: detected chip: generic`, step
/// ~448,000; in the physical badge's log too) instead of logging `memspi: no
/// response`.
#[test]
fn boot_reaches_spi_flash_detected_chip_generic() {
    let (rt, ok) = boot_until_console_contains("I (5) spi_flash: detected chip: generic", 500_000);
    assert!(
        ok,
        "flash-chip detection should succeed; pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    assert!(
        !rt.console_output().contains("memspi: no response"),
        "the pre-Task-8 RDID failure must be gone"
    );
}

/// ROM `memchr`/`memmove` are real stubs: zero traps through step 493,000,
/// past the old fault on step 490,128, with the `sleep_gpio` lines printed.
#[test]
fn boot_no_longer_faults_at_the_pre_task_8_memchr_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(493_000);
    assert_eq!(
        summary.traps,
        0,
        "boot must run past the old memchr fault (step 490,128) with no \
         traps; got {summary:?}, pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    assert!(
        rt.console_output()
            .contains("I (6) sleep_gpio: Enable automatic switching of GPIO sleep configuration"),
        "the sleep_gpio lines after flash detection should be present"
    );
}

/// `ets_apb_backup_init_lock_func`, `esp_coex_rom_version_get` and
/// `esprv_intc_int_set_threshold` are real stubs: zero traps through step
/// 527,877, the step before FreeRTOS's first yield request (v392; 528,311
/// on v335; see [`first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line`]).
#[test]
fn boot_no_longer_faults_at_the_pre_task_d10_ets_apb_backup_init_lock_func_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(527_877);
    assert_eq!(
        summary.traps,
        0,
        "boot must run past the old ets_apb_backup_init_lock_func fault \
         (step 493,861) with no traps; got {summary:?}, pc=0x{:08x}",
        rt.pc()
    );
}

/// The shortcut boot seeds the ROM's SPI-flash legacy data
/// (`g_rom_flashchip.chip_size` = 4 MiB, from firmware.bin's header), so
/// flash init no longer warns that the image header says 0k (the physical
/// badge's log has no such warning). Checked once the `sleep_gpio` line is
/// out.
#[test]
fn boot_no_longer_warns_that_the_image_header_says_0k_of_flash() {
    let needle = "I (6) sleep_gpio: Enable automatic switching of GPIO sleep configuration";
    let (rt, ok) = boot_until_console_contains(needle, 500_000);
    let console = rt.console_output();
    assert!(
        ok,
        "never printed {needle:?}; pc=0x{:08x}\nconsole:\n{console}",
        rt.pc()
    );
    assert!(console.contains("I (5) spi_flash: flash io: dio"));
    assert!(!console.contains("Detected size"), "console:\n{console}");
}

/// `spi_ll_apply_config()`'s `while (hw->cmd.update);` poll in
/// `spi_hal_init()` (a Milestone 3 stall: tens of thousands of hits per
/// address). v392 (Milestone 9): `0x4210_010a..=0x4210_0110`, found by
/// v335's 52 bytes around the poll (`lw a5, 0(a4)`; `srli a5, a5, 23`;
/// `andi a5, a5, 1`; `bnez`, after the `lui a3, 0x800`/`or`/`sw` that sets
/// `SPI_UPDATE`), which occur once in v392 (v335: `0x420f_d6fc..=0x420f_d702`).
const PRE_TASK_9_SPI_UPDATE_POLL: std::ops::RangeInclusive<u32> = 0x4210_010a..=0x4210_0110;

/// SPI2's `SPI_UPDATE` reads back 0 at once, so the `spi_hal_init()` poll
/// (first reached on step 585,551 in v392; 584,140 in v335) runs but does
/// not spin (each address hit once): over steps
/// 583,000..596,000 each poll address is hit fewer than [`SPIN_THRESHOLD`]
/// times and no exception is taken.
#[test]
fn boot_escapes_the_pre_task_9_spi_update_poll_into_spi_clock_setup() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    rt.run(583_000);
    let mut hist: HashMap<u32, u64> = HashMap::new();
    let summary = rt.run_traced(596_000 - 583_000, &mut hist);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    let max_hits = PRE_TASK_9_SPI_UPDATE_POLL
        .clone()
        .filter_map(|pc| hist.get(&pc).copied())
        .max()
        .unwrap_or(0);
    assert!(
        (1..SPIN_THRESHOLD).contains(&max_hits),
        "the poll must run but not spin: max single-PC hits = {max_hits}; pc=0x{:08x}",
        rt.pc()
    );
    assert!(!PRE_TASK_9_SPI_UPDATE_POLL.contains(&rt.pc()));
    let console = rt.console_output();
    assert!(
        !console.contains("Guru Meditation Error"),
        "console:\n{console}"
    );
}

/// ROM libgcc `__bswapsi2` (`esp32c3.rom.libgcc.ld`).
const ROM_BSWAPSI2: u32 = 0x4000_0788;
/// The return address of `spi_ll_set_command()`'s `__bswapsi2` call.
/// v392 (Milestone 9): `0x4039_463a`, the `ra` of every `__bswapsi2` call
/// in boot; v335's 40 bytes from its return address (`lw a5, 12(sp)`;
/// `slli`/`srli a0, 16`; the 16-bit field merge) occur once in v392, at
/// this address (v335: `0x4039_45fa`).
const SPI_LL_SET_COMMAND_BSWAP_RA: u32 = 0x4039_463a;

/// ROM `__bswapsi2` is a real stub: over steps 590,000..596,608 it is
/// entered exactly once (v392: `pc` is at it after step 594,523 and back at
/// the return address after 594,524; the next call is on step 603,354;
/// v335: step 594,724), from `spi_ll_set_command()`, returns to its
/// caller, and no exception is taken.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d12_bswapsi2_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    rt.run(590_000);
    let mut hist: HashMap<u32, u64> = HashMap::new();
    let summary = rt.run_traced(596_608 - 590_000, &mut hist);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt, not an exception; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    assert_eq!(hist.get(&ROM_BSWAPSI2), Some(&1), "one __bswapsi2 call");
    assert!(
        hist.contains_key(&SPI_LL_SET_COMMAND_BSWAP_RA),
        "the stub returned to spi_ll_set_command()"
    );
}

/// The shortcut boot seeds `RTC_XTAL_FREQ_REG` as the skipped bootloader
/// would, so neither `rtc_clk`'s nor `clk_hal`'s "invalid RTC_XTAL_FREQ_REG"
/// warning prints (the physical badge's log has neither). Checked over a run
/// to step 596,000, past the SPI2 setup.
#[test]
fn boot_no_longer_warns_that_rtc_xtal_freq_reg_is_invalid() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    rt.run(596_000);
    let console = rt.console_output();
    assert!(
        console.contains("I (6) main_task: Calling app_main()"),
        "console:\n{console}"
    );
    assert!(
        !console.contains("invalid RTC_XTAL_FREQ_REG"),
        "console:\n{console}"
    );
}

/// `wfi` is a real instruction, and while the core waits the driving loop
/// fast-forwards SYSTIMER to its next alarm. Over a run of 5,550,000 steps:
/// - no exception is taken (the last trap is an interrupt) and no panic
///   text prints;
/// - the idle fast-forward charged at least 1,500,000 ticks
///   (`Clock::idle_ticks`; measured 1,827,072 in Milestone 6 Task 1, where
///   SYSTIMER ran 1,827,060 ticks ahead of the step count: the 12
///   fast-forward steps are charged as idle, not as steps; 1,825,445 after
///   Task 2's stateful ROM CPU frequency; 2,262,240 since Milestone 6
///   Task D-M6-1; 2,312,365 on v392);
/// - counter 1, the FreeRTOS tick's, is past 20 tick periods of 160,000
///   (measured 3,269,013, 20.4 periods; 3,319,346, 20.7, on v392; 42.8
///   before Milestone 6 Task 3, when a busy step was a whole tick, not a
///   fifth), so the scheduler
///   kept ticking after the first wake.
#[test]
fn boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(5_550_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt, not an exception; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    let idle = rt.bus().clock.idle_ticks();
    assert!(
        idle >= 1_500_000,
        "idle fast-forward added only {idle} ticks"
    );
    let systimer = &rt.bus().systimer;
    assert!(
        systimer.counter(1) >= 20 * 160_000,
        "counter 1 = {}",
        systimer.counter(1)
    );
    let console = rt.console_output();
    assert!(
        console.contains("I (6) main_task: Calling app_main()"),
        "console:\n{console}"
    );
    for panic_text in ["Guru Meditation Error", "abort()", "Rebooting..."] {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// GDMA's TX out-link feeds SPI2, so LVGL's flushes reach the ST7789 model:
/// the framebuffer is still blank at step 1,100,000 (first pixels land at
/// ~1,137,900) and by step 5,550,000 holds a drawn frame of at least 2,000
/// distinct colors (the splash has 2,340), sent through GDMA channel 0,
/// with no exception and no panic text. (The name is Milestone 3's; the
/// `MD5Init` stall it mentions is gone.)
#[test]
fn boot_draws_frames_through_gdma_before_the_md5init_stall() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    rt.run(1_100_000);
    assert!(
        rt.framebuffer().iter().all(|px| *px == 0),
        "nothing drawn yet at step 1,100,000"
    );
    let summary = rt.run(5_550_000 - 1_100_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    let distinct: std::collections::HashSet<u16> = rt.framebuffer().iter().copied().collect();
    assert!(
        distinct.len() >= 2_000,
        "only {} distinct colors in the framebuffer",
        distinct.len()
    );
    assert_eq!(rt.bus().gdma.channel_for_spi2(), Some(0));
    assert_ne!(rt.bus().gdma.out_link_state(0).last_desc, 0);
    let console = rt.console_output();
    for panic_text in ["Guru Meditation Error", "abort()", "Rebooting..."] {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// ROM `MD5Init` is a real stub: through step 5,560,000, past
/// `load_partitions()`'s first call to it, no exception is taken, no panic
/// text prints, and the framebuffer still holds the splash.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d13_md5init_call_site() {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let summary = rt.run(5_560_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt, not an exception; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    let distinct: std::collections::HashSet<u16> = rt.framebuffer().iter().copied().collect();
    assert!(
        distinct.len() >= 2_000,
        "only {} distinct colors in the framebuffer",
        distinct.len()
    );
    let console = rt.console_output();
    for panic_text in ["Guru Meditation Error", "abort()", "Rebooting..."] {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// With the flash MMU modeled, `load_partitions()` reads the synthesized
/// partition table through its own `spi_flash_mmap` window, feeds the
/// entries to ROM `MD5Update`, checks the digest with `MD5Final`, and
/// returns `ESP_OK`, and no DBUS read goes through an invalid MMU entry.
///
/// v392 (Milestone 9): the epilogue is `mv a0, s4` at `0x420f_d282`
/// (v335: `0x420f_a878`). Found by shape: the function whose first
/// `MD5Init` call returns to `0x420f_d0f4` ends, as v335's does, with the
/// `ESP_ERR_NOT_FOUND` (0x105) and `ESP_ERR_INVALID_STATE` (0x103) error
/// paths setting `s4`, a list-free loop, `jal` to the same cleanup
/// (`310010ef`, same offset), then `mv a0, s4` and the 0xb0-byte frame's
/// restore; the epilogue bytes from `mv a0, s4` to `ret` are identical.
#[test]
fn load_partitions_accepts_the_synthesized_table_through_the_flash_mmu() {
    const ROM_MD5_INIT: u32 = 0x4000_0614;
    const ROM_MD5_UPDATE: u32 = 0x4000_0618;
    const ROM_MD5_FINAL: u32 = 0x4000_061c;
    const LOAD_PARTITIONS_EPILOGUE: u32 = 0x420f_d282;
    const ESP_OK: u32 = 0;

    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("firmware.bin boots");
    // MD5Init is first called on step 1,855,883 in v392 (Milestone 9;
    // 4,450,328 in v335 since Milestone 6 Task D-M6-1, 5,560,533 before).
    let summary = rt.run(1_500_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");

    // Single-step to load_partitions()'s first MD5Init, then to its epilogue.
    let mut reached_init = false;
    let (mut updates, mut finals) = (0u32, 0u32);
    let mut err = None;
    for _ in 0..1_000_000 {
        match rt.pc() {
            ROM_MD5_INIT => reached_init = true,
            ROM_MD5_UPDATE if reached_init => updates += 1,
            ROM_MD5_FINAL if reached_init => finals += 1,
            LOAD_PARTITIONS_EPILOGUE if reached_init => {
                err = Some(rt.cpu().regs.read(20));
                break;
            }
            _ => {}
        }
        let s = rt.run(1);
        assert_eq!(s.last_instruction_fault, None, "{s:?}");
    }
    assert!(reached_init, "load_partitions() never called MD5Init");
    assert_eq!(err, Some(ESP_OK), "s4 = err at the epilogue");
    // One MD5Update per partition entry (4) is the expected shape; at least one is required.
    assert!(updates >= 1, "MD5Update calls: {updates}");
    assert_eq!(finals, 1, "MD5Final calls");
    assert!(
        !rt.bus()
            .unmapped_log()
            .iter()
            .any(|a| (0x3c00_0000..0x3c80_0000).contains(&a.addr) && !a.is_write),
        "no DBUS read went through an invalid MMU entry"
    );
}

/// Log output after the scheduler starts reaches the console: the
/// USB-Serial-JTAG model reports SOF frames from an attached host, so
/// ESP-IDF's connection monitor keeps the port "connected" and the VFS
/// write does not drop `stdout` (`LVGL: Starting LVGL task`, step
/// ~669,000).
#[test]
fn boot_prints_console_output_after_the_scheduler_starts() {
    assert_reaches("LVGL: Starting LVGL task", 1_000_000);
}

/// The emulated `storage` partition is blank, so `esp_littlefs` fails to
/// mount it and starts formatting it (step ~1,908,000), without a panic. An
/// emulator-only line: the physical badge's flash holds a filesystem.
#[test]
fn boot_reaches_littlefs_formatting_the_blank_storage_partition() {
    let (rt, ok) = boot_until_console_contains("esp_littlefs: mount failed", 8_000_000);
    let console = rt.console_output();
    assert!(ok, "pc=0x{:08x}\nconsole:\n{console}", rt.pc());
    assert!(console.contains("formatting..."), "console:\n{console}");
    for panic_text in PANIC_TEXTS {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// Console text ESP-IDF's panic handler prints.
const PANIC_TEXTS: [&str; 3] = ["Guru Meditation Error", "abort()", "Rebooting..."];

/// `littlefs`'s superblock magic (`lfs.c`), stored 8 bytes into a
/// metadata block after its revision count and tag.
const LFS_MAGIC: &[u8; 8] = b"littlefs";

/// SPIMEM1 erases, programs and reads the chip, so the format writes the
/// littlefs superblock into both blocks of the root metadata pair (`storage`
/// blocks 0 and 1, at `0x2b_0000`/`0x2b_1000`, by step ~1,978,000) without a
/// fault, with no unmodeled SPIMEM1 command and no write left enabled.
#[test]
fn boot_formats_the_blank_storage_partition_with_littlefs() {
    const MAX: u64 = 8_000_000;
    let has_magic = |rt: &FirmwareRuntime, block: u32| {
        (0..8u32).all(|i| rt.bus().flash_chip.read(block + 8 + i) == LFS_MAGIC[i as usize])
    };
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    while !(has_magic(&rt, 0x2b_0000) && has_magic(&rt, 0x2b_1000)) {
        assert!(
            rt.total_steps() < MAX,
            "no littlefs superblock within {MAX} steps; pc=0x{:08x}\n{}",
            rt.pc(),
            rt.console_output()
        );
        let summary = rt.run(10_000);
        assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    }
    assert!(rt.bus().spimem1.unmodeled_commands().is_empty());
    assert!(!rt.bus().spimem1.write_enabled(), "no write left pending");
}

/// ROM `strlcat`/`strspn`/`strcspn` are real stubs, so the freshly formatted
/// filesystem mounts. The physical badge prints the same line with its own
/// usage figures, so they are left out.
#[test]
fn boot_reaches_hal_fs_littlefs_mounted() {
    assert_reaches("hal_fs: littlefs mounted at /littlefs", 8_000_000);
}

/// The button driver starts (`hal_buttons: buttons ready`; the physical
/// badge prints it too).
#[test]
fn boot_reaches_hal_buttons_ready() {
    assert_reaches("hal_buttons: buttons ready", 10_000_000);
}

/// The SC7A20H accelerometer answers on I2C0 (address 0x19): `hal_accel`
/// reads `WHO_AM_I` = 0x11 and prints the detection line the physical badge
/// prints (step ~4.64M), then configures the sensor and starts its cache
/// task, whose sample reads and ROM `__floatsisf` calls then run for
/// 1,000,000 steps without a fault or panic text. (Until Milestone 5 Task
/// D-M5-2 the bus had no device and this rung pinned `hal_accel`'s
/// "accelerometer setup failed" line instead.)
#[test]
fn boot_detects_the_sc7a20h_accelerometer_on_i2c0() {
    let needle = "hal_accel: SC7A20H detected (0x11)";
    let (mut rt, ok) = boot_until_console_contains(needle, 10_000_000);
    assert!(
        ok,
        "pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    let summary = rt.run(1_000_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    let console = rt.console_output();
    for panic_text in PANIC_TEXTS {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// RMT completes the LED driver's ping-pong transmission on TX channel 0
/// (`TX_END` reaches the driver's ISR through `SRC_RMT`), so `hal_sleep`
/// starts (step ~4.73M; the physical badge prints this line too) and the
/// channel is no longer running.
#[test]
fn boot_reaches_hal_sleep_after_the_rmt_transmission_completes() {
    let needle = "hal_sleep: sleep manager ready";
    let (rt, ok) = boot_until_console_contains(needle, 8_000_000);
    assert!(
        ok,
        "pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    assert!(!rt.bus().rmt.tx_running(0), "the transmission finished");
}

/// ROM `strdup`, `strchr` and `strcpy` are real, so the app registry
/// launches its first app, My Badge (`app_reg: launched My Badge`, step
/// ~10.54M), and runs without a fault or panic text to step 20,000,000, by
/// which point the framebuffer is no longer the splash ([`SPLASH_HASH`]).
#[test]
fn boot_launches_the_first_app_and_leaves_the_splash_without_faulting() {
    let needle = "app_reg: launched My Badge";
    let (mut rt, ok) = boot_until_console_contains(needle, 16_000_000);
    assert!(
        ok,
        "pc=0x{:08x}\nconsole:\n{}",
        rt.pc(),
        rt.console_output()
    );
    while rt.total_steps() < 20_000_000 {
        let summary = rt.run(500_000);
        assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    }
    let console = rt.console_output();
    for panic_text in PANIC_TEXTS {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
    assert_ne!(
        framebuffer_fnv1a(rt.framebuffer()),
        SPLASH_HASH,
        "the app drew over the splash"
    );
}

/// FNV-1a (64-bit) over the framebuffer's RGB565 pixels, each as 2
/// little-endian bytes, in framebuffer order.
fn framebuffer_fnv1a(fb: &[u16]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for px in fb {
        for b in px.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// The boot splash's hash: the first stable frame
/// ([`boots_to_first_real_frame`]). v392's splash ("Hack the North x
/// SOLANA", footer "FW v0.1.2-392-gd3089c4"); v335's was
/// `0x5599_c270_ab04_29fa`.
const SPLASH_HASH: u64 = 0xfb30_5408_a9e5_0e98;

/// The sampling chunk of [`run_until_stable_frame`].
const FRAME_CHUNK: u32 = 250_000;

/// How long a frame must hold to count as stable at boot: a frame that
/// leaves the splash or draws in changes well within this.
const BOOT_STABLE_FOR: u64 = 1_000_000;

/// How long a frame must hold after a button press. After START the
/// firmware computes for millions of steps without drawing before the
/// self-test frame lands (about 6,000,000 under the pre-Milestone-6 clock;
/// since Milestone 6, about 3,480,000 after the press, inside the 100 ms
/// hold), so a 1,000,000-step window would take the unchanged first-run
/// frame for the result.
const AFTER_PRESS_STABLE_FOR: u64 = 8_000_000;

/// Runs `rt` in [`FRAME_CHUNK`]-step chunks, asserting no instruction
/// fault in any chunk, until the framebuffer's hash has held for
/// `stable_for` steps. Uniform frames (blank or a solid fill) and hashes in
/// `skip` never count as stable. Returns `(hash, step)`: the stable frame's
/// hash and the first sampled step that showed it. Panics if no frame is
/// stable by `max_steps` (total steps, not steps from this call).
fn run_until_stable_frame(
    rt: &mut FirmwareRuntime,
    max_steps: u64,
    stable_for: u64,
    skip: &[u64],
) -> (u64, u64) {
    let mut hash = framebuffer_fnv1a(rt.framebuffer());
    let mut hash_since = rt.total_steps();
    while rt.total_steps() < max_steps {
        let summary = rt.run(FRAME_CHUNK);
        assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
        let fb = rt.framebuffer();
        let h = framebuffer_fnv1a(fb);
        if h != hash {
            hash = h;
            hash_since = rt.total_steps();
        } else if rt.total_steps() - hash_since >= stable_for
            && fb.iter().any(|px| *px != fb[0])
            && !skip.contains(&h)
        {
            return (hash, hash_since);
        }
    }
    panic!(
        "no stable frame (held {stable_for} steps, not uniform, not in {skip:x?}) within \
         {max_steps} steps; last change at {hash_since}, hash {hash:#x}; pc=0x{:08x}\n{}",
        rt.pc(),
        rt.console_output()
    );
}

/// Asserts the console holds none of [`PANIC_TEXTS`].
fn assert_no_panic_text(rt: &FirmwareRuntime) {
    let console = rt.console_output();
    for panic_text in PANIC_TEXTS {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// Milestone 3's finish line: the real firmware boots to its first stable
/// ST7789 frame, the boot splash, pinned by hash. The first non-blank
/// frame (about step 1.138M) is not the finished splash; the framebuffer
/// changes many times before it settles (v392: from step 4,250,000), so
/// the test pins the first frame that holds for [`BOOT_STABLE_FOR`] steps
/// (found at the 5,250,000-step sample). `MAX` is the stable step × 2,
/// rounded up.
///
/// The hash is of this framebuffer's row/column order. MADCTL is not
/// modeled (the notes' "Known limitations"); the splash and the first-run
/// screen were compared by eye with the physical badge on v335
/// (2026-10-06 and 2026-10-07) and match in orientation, so a MADCTL model
/// that keeps the image as the badge shows it would leave the hash
/// unchanged. v392's splash was checked by eye in Milestone 9.
///
/// Inspect a failing frame with
/// `cargo run -p emulator-core --release --example boot-probe -- --steps 5000000 --dump-frame first-frame.png`
/// (writes under the gitignored `local/`).
#[test]
fn boots_to_first_real_frame() {
    const MAX: u64 = 8_500_000;
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let (hash, step) = run_until_stable_frame(&mut rt, MAX, BOOT_STABLE_FOR, &[]);
    let distinct: std::collections::HashSet<u16> = rt.framebuffer().iter().copied().collect();
    assert_eq!(
        hash,
        SPLASH_HASH,
        "first stable frame changed ({} distinct colors, stable since step {step}); \
         inspect with boot-probe --dump-frame",
        distinct.len()
    );
    assert_no_panic_text(&rt);
}

/// The My Badge app's first-run screen on blank flash ("Not registered
/// yet", with "Press START for hardware self-test"): the first stable
/// frame after the splash.
const FIRST_RUN_HASH: u64 = 0x8c50_27ce_c0f7_9490;

/// [`boots_to_first_run_screen`]'s cap: the frame is first sampled at step
/// 14,500,000 (v392; 15,250,000 on v335); that + 2,000,000, rounded up to
/// a 250,000 multiple.
const FIRST_RUN_MAX_STEPS: u64 = 16_500_000;

/// Milestone 4's finish line: blank-flash boot reaches the first app's
/// screen and it holds. The emulated flash has no identity
/// (`/littlefs/identity.json`), so the app registry's first app, My Badge,
/// shows its unregistered first-run screen; on an unprovisioned badge it
/// keeps HOME for itself, so the app launcher is reached only after
/// provisioning ([`boots_to_launcher`]). Compared by eye with the physical
/// badge after a factory reset on v335 (2026-10-07): content and
/// orientation match. v392 draws the same frame (same hash).
#[test]
fn boots_to_first_run_screen() {
    let rt = first_run_screen();
    assert!(
        rt.console_output().contains("app_reg: launched My Badge"),
        "console:\n{}",
        rt.console_output()
    );
    assert_no_panic_text(&rt);
}

/// The hardware self-test's first screen ("Press every button", none lit
/// yet).
const SELF_TEST_BUTTONS_HASH: u64 = 0x7f32_5d83_8835_baaa;

/// How long a button press is held in the input tests: a human press,
/// 100 ms of emulated time (Milestone 6: was 1,600,000 steps, 100 ms
/// under the pre-Milestone-6 one-tick-per-step clock). The older
/// 100,000-step (6.25 ms) hold only registered because the undrained
/// console left the firmware idle; once the USB-Serial-JTAG ISR drains the
/// TX ring buffer, button polling falls outside such a short window
/// (Milestone 5 ruling R-T2-1).
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
    assert!(
        rt.emulated_time_us() >= target,
        "{us} us not reached in {} steps",
        s.steps
    );
}

/// [`first_run_screen_responds_to_start`]'s cap: the self-test frame is
/// first sampled at step 20,683,077 (v392: the 100 ms hold took 5,183,077
/// steps from the first-run checkpoint at 15,500,000); that +
/// [`AFTER_PRESS_STABLE_FOR`] + 1,000,000, rounded up to a 250,000
/// multiple.
const SELF_TEST_MAX_STEPS: u64 = 29_750_000;

/// Button input reaches the firmware: from the stable first-run screen, a
/// press and release of slot 0 (START) starts My Badge's hardware
/// self-test, as the screen says and as the physical badge does. The other
/// buttons do nothing on this screen, on the badge too (an unprovisioned My
/// Badge reacts only to START).
#[test]
fn first_run_screen_responds_to_start() {
    const START: usize = 0;
    let mut rt = first_run_screen();

    rt.set_raw_button(START, true);
    run_for_us(&mut rt, PRESS_HOLD_US);
    rt.set_raw_button(START, false);

    let (hash, step) =
        run_until_stable_frame(&mut rt, SELF_TEST_MAX_STEPS, AFTER_PRESS_STABLE_FOR, &[]);
    assert_ne!(hash, FIRST_RUN_HASH, "START changed nothing");
    assert_eq!(hash, SELF_TEST_BUTTONS_HASH, "stable at step {step}");
    assert_no_panic_text(&rt);
}

/// Milestone 5 Task 3: `app_main` reaches `hal_console_start()` on blank
/// flash, as on the physical badge (the line follows `app_reg: launched My
/// Badge`); the console runs the interrupt-driven USB-Serial-JTAG driver,
/// so this needs Task 2's interrupt source. First printed at step
/// 15,200,000 (v392, 10,000-step sampling; 16,050,000 on v335); budget =
/// that + 25%, rounded up to 1,000,000.
#[test]
fn boot_starts_the_console() {
    assert_reaches("hal_console: console started", 19_000_000);
}

/// [`console_answers_prov_show_on_the_first_run_screen`] and later
/// provisioning rungs: total-step deadline. Measured on v392 (Milestone
/// 9): the prompt is out by 15.12M (10,000-step sampling), before the
/// first-run checkpoint at 15.50M; [`console_rejects_an_empty_identity`]'s
/// `prov apply` answers at 19.30M and its stable-frame check ends at
/// 20.30M (50,000-step sampling); that + 25%, rounded up to 1,000,000.
const CONSOLE_DEADLINE: u64 = 26_000_000;

/// [`put_accepts_a_payload_larger_than_the_rx_ring`]: total-step deadline.
/// Measured on v392 (Milestone 9): finished at 16.43M (50,000-step
/// sampling, host-paced chunks); that + 25%, rounded up to 1,000,000.
const RING_TEST_DEADLINE: u64 = 21_000_000;

// ---------------------------------------------------------------------
// Checkpoints. A `FirmwareRuntime` clone is a full snapshot (pinned by
// `runtime::tests::a_clone_runs_exactly_like_its_original`), so a state
// several rungs share is reached once per test binary, with every
// assertion on the way, and each rung continues from its own clone. A
// failed walk leaves its `OnceLock` empty, so every rung that needs it
// re-runs the walk and fails with the walk's own message: nothing is
// skipped or loosened. Rungs start from: blank-flash boot ([`FIRST_RUN`]),
// role hacker's onboarding page 6 ([`OWN_SHAKE_PAGE`]), each role's
// registered screen ([`REGISTERED`]) and launcher ([`LAUNCHER`]).
// ---------------------------------------------------------------------

/// The stable first-run screen on blank flash ([`first_run_screen`]).
static FIRST_RUN: OnceLock<FirmwareRuntime> = OnceLock::new();

/// Boots to the stable first-run screen (blank flash), asserting
/// [`FIRST_RUN_HASH`] within [`FIRST_RUN_MAX_STEPS`]: a clone of the
/// [`FIRST_RUN`] checkpoint.
fn first_run_screen() -> FirmwareRuntime {
    FIRST_RUN
        .get_or_init(|| {
            let mut rt = FirmwareRuntime::from_image(&firmware()).expect("firmware.bin boots");
            let (hash, step) = run_until_stable_frame(
                &mut rt,
                FIRST_RUN_MAX_STEPS,
                BOOT_STABLE_FOR,
                &[SPLASH_HASH],
            );
            assert_eq!(hash, FIRST_RUN_HASH, "stable at step {step}");
            rt
        })
        .clone()
}

/// The console takes input on the first-run screen: `prov show` reports an
/// unprovisioned badge, as the registration desk would see it.
#[test]
fn console_answers_prov_show_on_the_first_run_screen() {
    let mut rt = first_run_screen();
    assert!(
        provision::wait_for_any(&mut rt, 0, &[provision::PROMPT], CONSOLE_DEADLINE).is_some(),
        "no prompt"
    );
    match provision::type_line(&mut rt, "prov show", CONSOLE_DEADLINE) {
        Outcome::Done(text) => assert!(text.contains("provisioned=0"), "{text}"),
        Outcome::Timeout(text) => panic!("no prompt after `prov show`:\n{text}"),
    }
    assert_no_panic_text(&rt);
}

/// `put` of a payload more than twice the driver's 256-byte RX ring buffer
/// (whose ISR drops what does not fit): the model must pace OUT packets
/// against CPU throughput so the `put` task drains the ring in time.
/// Synthetic pattern, no identity (R-M5-2).
#[test]
fn put_accepts_a_payload_larger_than_the_rx_ring() {
    let mut rt = first_run_screen();
    let payload: Vec<u8> = (b'a'..=b'z').cycle().take(600).collect();
    let text = provision::put_file(
        &mut rt,
        "/littlefs/m5-ring-test.bin",
        &payload,
        RING_TEST_DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(text.contains("OK 600"), "{text}");
    assert_no_panic_text(&rt);
}

/// The whole `put` / `prov apply` path, with a record the firmware must
/// reject: an empty JSON object. Nothing identity-shaped is involved. The
/// firmware's message names the file path, not the failing field (Task 1,
/// `APPLY_EFFECTS`: the `%s` is always `/littlefs/identity.json`).
#[test]
fn console_rejects_an_empty_identity() {
    let mut rt = first_run_screen();
    let err = provision::provision(&mut rt, b"{}", CONSOLE_DEADLINE)
        .expect_err("`{}` must not provision");
    assert!(
        err.contains("PROV FAIL invalid or missing /littlefs/identity.json"),
        "{err}"
    );
    let (hash, _) = run_until_stable_frame(&mut rt, CONSOLE_DEADLINE, BOOT_STABLE_FOR, &[]);
    assert_eq!(hash, FIRST_RUN_HASH, "still unregistered");
    assert_no_panic_text(&rt);
}

// ---------------------------------------------------------------------
// Milestone 5: provisioning through the console, the onboarding app
// ("Setup", 11 pages, a shake on page 6), My Badge's registered screen,
// HOME to the launcher. The identities are the committed fake fixtures
// (`provision::identity_fixture`, R-M5-2: permission relayed 2026-10-08).
// Measured with every role (Task 4); the input sequence is the same for
// every role. Frame comparison with the physical badge, on v335
// (2026-10-08, the human partner's badge, role hacker): the registered
// screen, every onboarding page, the launcher and the launcher after DOWN
// match. The other roles' frames were not compared on hardware. On v392
// (Milestone 9) the registered screens, onboarding pages 2 and 4 to 11,
// the shaken page and the launcher after DOWN keep their v335 hashes;
// page 1, page 3's button frames and the launcher changed and were
// checked by eye in Milestone 9, not on hardware.
// ---------------------------------------------------------------------

/// Total-step deadline for provisioning from the first-run screen: the
/// slowest role (workshop_lead) prints `PROV OK` at 28.74M on v392 (all
/// roles 28.54M..28.74M, host-paced `put`, 50,000-step sampling; v335:
/// 28.39M); + 25%, rounded up to 1,000,000.
const PROVISION_DEADLINE: u64 = 36_000_000;
/// How long after a press (or the shake) a response may take to start:
/// page 7 settled about 9M steps after A under the pre-Milestone-6 clock
/// (a transition animation; 3M after A's release since Milestone 6 Task
/// 3), so a response's cap is [`AFTER_PRESS_STABLE_FOR`] plus this.
const ONBOARDING_RESPONSE_MARGIN: u64 = 16_000_000;
/// Raw slots from onboarding page 1 to page 6, each with the stable frame
/// it leads to (role hacker). v392's page 3 asks for 8 buttons ("Press all
/// 8 buttons once"): v335's Aux1 tile (raw slot 8) is gone, and a press of
/// slot 8 leaves the frame unchanged, so the walk no longer presses it.
const TO_SHAKE_PAGE: &[(usize, u64)] = &[
    (1, 0xe788_a781_9328_1b08), // A: page 2 "This is you"
    (1, 0x38a9_d040_707c_c4e8), // A: page 3 "Try every button"
    (1, 0x8a68_96e1_b5c9_5d6b), // A lit
    (2, 0x8b0b_8358_e2b5_d6e8), // B
    (7, 0xf38f_281c_4b05_dabd), // UP
    (5, 0xb185_0f5c_8931_2eb3), // LEFT
    (6, 0x5266_68ba_6df4_b25e), // RIGHT
    (4, 0x3d7b_e2d1_7265_7413), // DOWN
    (3, 0x1fd7_15c4_6f18_7c4c), // HOME (captured on page 3)
    (0, 0xc2eb_c5fd_5353_2d8c), // START: all eight done
    (1, 0xc1ad_004b_a5f7_7b78), // A: page 4 "Lights"
    (1, 0x7598_def3_0e83_8832), // A: page 5 "Tap to unlock"
    (1, 0x4a54_e5a4_0d66_9898), // A: page 6 "Shake it!", G-force 1000 mg
];
/// Page 6 after the shake, back at rest: G-force 1000 mg, Peak 3000 mg,
/// footer "A / START: next   B: back" (role hacker).
const SHAKEN_AT_REST_HASH: u64 = 0xa80a_279b_127e_1bb1;
/// Page 7 ("Badge Connect"), after A.
const ONBOARDING_PAGE7_HASH: u64 = 0x110e_8782_bc4e_5ff0;
/// Onboarding steps after the shake: (slot, expected stable hash for role
/// hacker); `None` is page 8 ("Bump to connect"), which is animated: its
/// frame changes 24 times in the [`ANIMATED_PAGE_STEPS`] after the press
/// (each held 0.1M..0.85M steps, 50,000-step sampling; never held for
/// [`AFTER_PRESS_STABLE_FOR`]), so it has no stable frame to pin. The walk
/// runs [`ANIMATED_PAGE_STEPS`] after that press and presses A.
const AFTER_SHAKE_PAGES: &[(usize, Option<u64>)] = &[
    (1, Some(ONBOARDING_PAGE7_HASH)), // A: page 7 "Badge Connect"
    (1, None),                        // A: page 8 "Bump to connect", animated
    (1, Some(0x0831_b6b6_75e6_fe01)), // A: page 9 "HEAT SAFETY"
    (1, Some(0x21e9_c7e1_b054_39e3)), // A: page 10 "Go explore"
    (1, Some(0x7dd4_5fd3_9362_faac)), // A: page 11 "Badge rules", unticked
    (1, Some(0xe8c6_7af9_7ac7_968e)), // A: ticked, "Finished setup  Start"
];
/// How long the walk stays on the animated page 8 before pressing A:
/// Milestone 5's measured 9M-step page settle, kept in steps (it measures
/// firmware work, not a human's timing).
const ANIMATED_PAGE_STEPS: u32 = 9_000_000;
/// Setup's last press (START) to My Badge's registered screen: first
/// stable 3.75M..4.25M steps after the release, for every role (v392;
/// 4.5M..5.5M on v335).
const REGISTERED_RESPONSE_STEPS: u64 = AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
/// The role on the human partner's physical badge (Task 5).
const OWN_ROLE: &str = "hacker";
/// Raw slot from the registered screen to the launcher: HOME alone. The
/// launcher draws about 11.5M steps after the release (v392; 12.25M on
/// v335; the console says `launched Launcher` about 2.75M after the
/// release, 2.6M on v335, while the registered frame is still on screen).
const TO_LAUNCHER_SLOT: usize = 3;
/// HOME's response: release to first stable launcher 11.5M on v392 (role
/// hacker and workshop_lead, the roles the rungs walk there; 12.25M on
/// v335), then [`AFTER_PRESS_STABLE_FOR`] more to confirm, 19.5M; + 25%,
/// rounded up to 1,000,000.
const LAUNCHER_RESPONSE_STEPS: u64 = 25_000_000;
/// The launcher: a 4x4 icon grid titled "My Badge", first icon selected.
/// The same for all 11 roles (measured on v335 and on v392). v392's
/// differs from v335's (`0xb694_a61f_ebfd_62ff`) in a few pixels, with
/// the same grid and order.
const LAUNCHER_HASH: u64 = 0x94d7_a7a8_5fa9_3ea3;
/// DOWN: moves the launcher's selection one row (My Badge to "Share").
const NAV_SLOT: usize = 4;
/// The launcher after one DOWN (first stable 0.75M steps after release on
/// v392; 1.25M on v335). Unchanged from v335.
const LAUNCHER_AFTER_NAV_HASH: u64 = 0xa2b9_da1f_0e36_757c;

/// (role, onboarding page 1 hash, registered My Badge screen hash), in
/// `ROLE_TABLE` order. Both screens show the fake identity's name (and the
/// role colour), so every role differs. Only hacker's were compared with
/// the physical badge, on v335 (2026-10-08). v392 re-pinned every page 1
/// (checked by eye in Milestone 9); the registered screens are unchanged.
const ROLE_ROWS: &[(&str, u64, u64)] = &[
    ("hacker", 0xbf9c_cb7e_80b4_4b1a, 0xbd6d_7613_08a7_bf24),
    ("organizer", 0x3c31_f95a_a12e_406f, 0x1974_c49f_a562_08a3),
    ("sponsor", 0xc5ba_681b_7473_21bc, 0x0bc2_dee2_1a99_c4c0),
    ("judge", 0x192b_1f45_de51_5d49, 0x54f8_e3d0_65a3_04c1),
    ("mentor", 0x4929_00a1_98c0_3e4d, 0x03a9_7611_bf45_9332),
    ("volunteer", 0xd370_a34f_7bc7_d738, 0xfd95_a0e7_844f_4019),
    ("media", 0xa0b0_7c3c_ff8f_289d, 0x5092_210f_e8d8_f1d2),
    ("staff", 0xe912_c748_aab7_152c, 0xc68e_18b7_16ef_40ea),
    ("general", 0x0c04_ea57_eeb8_f8ec, 0x4574_d9ed_d13c_330a),
    (
        "workshop_lead",
        0x96a7_9955_fffc_187c,
        0xf8e2_3246_19a6_4d76,
    ),
    ("visitor", 0x2770_ab89_5a47_e576, 0x647a_71d4_5ab8_cf23),
];

fn role_row(role: &str) -> (u64, u64) {
    let r = ROLE_ROWS.iter().find(|r| r.0 == role).expect("role row");
    (r.1, r.2)
}

/// Press and release `slot`, held [`PRESS_HOLD_US`], asserting no fault.
fn press(rt: &mut FirmwareRuntime, slot: usize) {
    rt.set_raw_button(slot, true);
    run_for_us(rt, PRESS_HOLD_US);
    rt.set_raw_button(slot, false);
}

/// [`press`], then the next stable frame other than `previous`.
fn press_and_settle(rt: &mut FirmwareRuntime, slot: usize, previous: u64) -> (u64, u64) {
    press(rt, slot);
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    run_until_stable_frame(rt, cap, AFTER_PRESS_STABLE_FOR, &[previous])
}

/// Shakes the badge (see [`SHAKE_HALF_US`]) and puts it back at rest.
fn shake(rt: &mut FirmwareRuntime) {
    for k in 0..6 {
        let s = if k % 2 == 0 { 1 } else { -1 };
        rt.set_acceleration(s * 2000, s * 2000, 1000);
        run_for_us(rt, SHAKE_HALF_US);
    }
    rt.set_acceleration(0, 0, 1000);
}

/// Provisions `role` from the stable first-run screen and returns the
/// runtime on onboarding page 1 ("Welcome to Hack the North"), after
/// asserting `PROV OK` and that page's hash.
fn provisioned(role: &str) -> FirmwareRuntime {
    let (page1, _) = role_row(role);
    let mut rt = first_run_screen();
    let text = provision::provision(
        &mut rt,
        &provision::identity_fixture(role),
        PROVISION_DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{role}: {e}"));
    assert!(text.contains("PROV OK id=test-"), "{role}: {text}");
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let (hash, step) =
        run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[FIRST_RUN_HASH]);
    assert_eq!(
        hash, page1,
        "{role}: onboarding page 1, stable at step {step}"
    );
    rt
}

/// Whether `role`'s intermediate onboarding frames are pinned: only role
/// hacker's, the one compared with the physical badge (other roles reach
/// the same pages, but their name-bearing frames differ).
fn pinned(role: &str) -> bool {
    role == OWN_ROLE
}

/// Provisions `role`, then walks Setup from page 1 to page 6 ("Shake it!",
/// at rest), pinning each frame when [`pinned`].
fn walk_to_shake_page(role: &str) -> (FirmwareRuntime, u64) {
    let (page1, _) = role_row(role);
    let mut rt = provisioned(role);
    let mut previous = page1;
    for &(slot, expected) in TO_SHAKE_PAGE {
        let (hash, step) = press_and_settle(&mut rt, slot, previous);
        if pinned(role) {
            assert_eq!(
                hash, expected,
                "{role}: after slot {slot}, stable at step {step}"
            );
        }
        previous = hash;
    }
    (rt, previous)
}

/// Role hacker on onboarding page 6, at rest ([`walk_to_shake_page`]).
static OWN_SHAKE_PAGE: OnceLock<(FirmwareRuntime, u64)> = OnceLock::new();

/// [`walk_to_shake_page`]: the runtime and page 6's hash. Role hacker's
/// comes from the [`OWN_SHAKE_PAGE`] checkpoint (two rungs continue from
/// it); other roles walk.
fn shake_page(role: &str) -> (FirmwareRuntime, u64) {
    if role == OWN_ROLE {
        OWN_SHAKE_PAGE
            .get_or_init(|| walk_to_shake_page(role))
            .clone()
    } else {
        walk_to_shake_page(role)
    }
}

/// Milestone 5 Task D-M5-2: with the SC7A20H on I2C0, a provisioned
/// badge's onboarding app reaches "Shake it!" (page 6), reads the resting
/// 1 g, detects a shake, offers "A / START: next", and A moves on to page
/// 7.
#[test]
fn onboarding_shake_page_advances_after_a_shake() {
    let (mut rt, previous) = shake_page(OWN_ROLE);
    assert_eq!(rt.acceleration(), [0, 0, 1000], "face up at rest");
    shake(&mut rt);
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[previous]);
    assert_eq!(
        hash, SHAKEN_AT_REST_HASH,
        "after the shake, stable at step {step}"
    );
    let (hash, step) = press_and_settle(&mut rt, 1, hash);
    assert_eq!(hash, ONBOARDING_PAGE7_HASH, "page 7, stable at step {step}");
    assert_no_panic_text(&rt);
}

/// Provisions `role`, then walks Setup (11 pages, a shake on page 6) to
/// My Badge's registered screen. Page 1 and the registered screen are
/// pinned for every role; the pages between only when [`pinned`].
fn walk_to_registered(role: &str) -> FirmwareRuntime {
    let (_, registered_hash) = role_row(role);
    let (mut rt, mut previous) = shake_page(role);
    shake(&mut rt);
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[previous]);
    if pinned(role) {
        assert_eq!(
            hash, SHAKEN_AT_REST_HASH,
            "{role}: after the shake, stable at step {step}"
        );
    }
    previous = hash;
    for &(slot, expected) in AFTER_SHAKE_PAGES {
        match expected {
            Some(expected) => {
                let (hash, step) = press_and_settle(&mut rt, slot, previous);
                if pinned(role) {
                    assert_eq!(
                        hash, expected,
                        "{role}: after slot {slot}, stable at step {step}"
                    );
                }
                previous = hash;
            }
            None => {
                press(&mut rt, slot);
                let s = rt.run(ANIMATED_PAGE_STEPS);
                assert_eq!(s.last_instruction_fault, None, "{s:?}");
            }
        }
    }
    // START finishes Setup: My Badge's registered screen.
    press(&mut rt, 0);
    let cap = rt.total_steps() + REGISTERED_RESPONSE_STEPS;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[previous]);
    assert_eq!(
        hash, registered_hash,
        "{role}: registered screen, stable at step {step}"
    );
    assert_no_panic_text(&rt);
    rt
}

/// Each role's registered screen ([`walk_to_registered`]), by
/// [`ROLE_ROWS`] index.
static REGISTERED: [OnceLock<FirmwareRuntime>; ROLE_ROWS.len()] =
    [const { OnceLock::new() }; ROLE_ROWS.len()];
/// Each role's launcher ([`walk_to_launcher`]), by [`ROLE_ROWS`] index.
static LAUNCHER: [OnceLock<FirmwareRuntime>; ROLE_ROWS.len()] =
    [const { OnceLock::new() }; ROLE_ROWS.len()];

fn role_index(role: &str) -> usize {
    ROLE_ROWS
        .iter()
        .position(|r| r.0 == role)
        .expect("role row")
}

/// [`walk_to_registered`], from the [`REGISTERED`] checkpoint.
fn registered(role: &str) -> FirmwareRuntime {
    REGISTERED[role_index(role)]
        .get_or_init(|| walk_to_registered(role))
        .clone()
}

macro_rules! provisions_role {
    ($name:ident, $role:literal) => {
        /// Provisioning through the console and Setup register the badge
        /// and land on My Badge's registered screen (see
        /// `walk_to_registered`).
        #[test]
        fn $name() {
            registered($role);
        }
    };
}

provisions_role!(provisions_hacker_through_the_console, "hacker");
provisions_role!(provisions_organizer_through_the_console, "organizer");
provisions_role!(provisions_sponsor_through_the_console, "sponsor");
provisions_role!(provisions_judge_through_the_console, "judge");
provisions_role!(provisions_mentor_through_the_console, "mentor");
provisions_role!(provisions_volunteer_through_the_console, "volunteer");
provisions_role!(provisions_media_through_the_console, "media");
provisions_role!(provisions_staff_through_the_console, "staff");
provisions_role!(provisions_general_through_the_console, "general");
provisions_role!(
    provisions_workshop_lead_through_the_console,
    "workshop_lead"
);
provisions_role!(provisions_visitor_through_the_console, "visitor");

/// [`registered`], then HOME to the stable launcher.
fn walk_to_launcher(role: &str) -> FirmwareRuntime {
    let (_, registered_hash) = role_row(role);
    let mut rt = registered(role);
    press(&mut rt, TO_LAUNCHER_SLOT);
    let cap = rt.total_steps() + LAUNCHER_RESPONSE_STEPS;
    let (hash, step) =
        run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[registered_hash]);
    assert_eq!(
        hash, LAUNCHER_HASH,
        "{role}: launcher, stable at step {step}"
    );
    rt
}

/// [`walk_to_launcher`], from the [`LAUNCHER`] checkpoint.
fn launcher_for(role: &str) -> FirmwareRuntime {
    LAUNCHER[role_index(role)]
        .get_or_init(|| walk_to_launcher(role))
        .clone()
}

/// Milestone 5 finish line: a provisioned badge reaches the app launcher.
/// The input sequence and every pinned frame on the way were compared
/// with the physical badge on v335 (2026-10-08, role hacker); v392's
/// changed frames were checked by eye in Milestone 9.
#[test]
fn boots_to_launcher() {
    let rt = launcher_for(OWN_ROLE);
    assert!(rt.console_output().contains("launched Launcher"));
    assert_no_panic_text(&rt);
}

/// The launcher does not depend on the role (11 of 11 measured equal);
/// the longest-name role is checked as a second sample. Not compared on
/// hardware with this role.
#[test]
fn launcher_is_role_independent() {
    let rt = launcher_for("workshop_lead");
    assert_no_panic_text(&rt);
}

/// The launcher takes input: one [`NAV_SLOT`] press moves the selection,
/// as on the badge (compared on v335, 2026-10-08; v392 draws the same
/// frame).
#[test]
fn launcher_responds_to_navigation() {
    let mut rt = launcher_for(OWN_ROLE);
    let (hash, step) = press_and_settle(&mut rt, NAV_SLOT, LAUNCHER_HASH);
    assert_eq!(hash, LAUNCHER_AFTER_NAV_HASH, "stable at step {step}");
    assert_no_panic_text(&rt);
}

/// Milestone 5 stretch (Task 9): from the launcher, the navigation to Dice
/// (bottom row, last column) with each selection's stable frame (role
/// hacker): three DOWNs, three RIGHTs.
const TO_DICE: &[(usize, u64)] = &[
    (4, LAUNCHER_AFTER_NAV_HASH), // DOWN: "Share"
    (4, 0xf742_d1dd_3346_b879),   // DOWN: third row
    (4, 0xd14b_7793_6c29_cffc),   // DOWN: bottom row, first column
    (6, 0xb670_7637_46f5_fcaa),   // RIGHT
    (6, 0x4ba3_86cb_3694_4ff3),   // RIGHT: "Snake"
    (6, 0x3043_c575_7906_740d),   // RIGHT: "Dice"
];
/// Dice's first stable frame: title "Dice", "d20", "--", footer
/// "L/R: die   A or SHAKE: roll" (stable about 2.75M steps after A's
/// release).
const DICE_OPEN_HASH: u64 = 0x9be3_a420_5d54_f3ff;
/// Dice after one A: a d20 roll showing "2". The value comes from
/// `esp_random()`, i.e. the emulator's fixed-seed RNG
/// (`peripherals::apb_ctrl::RNG_SEED`, Task D-M5-3), so it is not the
/// badge's roll.
const DICE_ROLLED_HASH: u64 = 0x692c_6023_de06_d917;

/// Milestone 5 stretch: a built-in app opens from the launcher. A on Dice
/// launches it (`launched Dice`), its first frame is pinned, and A rolls.
/// The open frame and the roll were compared with the physical badge on v335,
/// 2026-10-08 (layout, not the rolled value); the navigation frames on the
/// way were not.
#[test]
fn launcher_opens_dice() {
    let mut rt = launcher_for(OWN_ROLE);
    let mut previous = LAUNCHER_HASH;
    for &(slot, expected) in TO_DICE {
        let (hash, step) = press_and_settle(&mut rt, slot, previous);
        assert_eq!(hash, expected, "after slot {slot}, stable at step {step}");
        previous = hash;
    }
    let (hash, step) = press_and_settle(&mut rt, 1, previous);
    assert_eq!(hash, DICE_OPEN_HASH, "Dice opened, stable at step {step}");
    assert!(
        rt.console_output().contains("app_reg: launched Dice"),
        "console:\n{}",
        rt.console_output()
    );
    let (hash, step) = press_and_settle(&mut rt, 1, DICE_OPEN_HASH);
    assert_eq!(
        hash, DICE_ROLLED_HASH,
        "after A (roll), stable at step {step}"
    );
    assert_no_panic_text(&rt);
}

/// Milestone 9 Task 3: from the launcher, the navigation to Scanner (top
/// row, third column) with each selection's stable frame (role hacker,
/// v392, measured): two RIGHTs.
const TO_SCANNER: &[(usize, u64)] = &[
    (6, 0x7abe_9faf_bf7f_982e), // RIGHT
    (6, 0xb9a6_064d_93a2_1cf4), // RIGHT: "Scanner"
];
/// Scanner's first stable frame after its reboot: title "Scanner", the
/// round "Use your badge's Scanner app!" sticker art, "Align your badge's
/// flower pattern with a matching sticker!", footer "HOME apps". Measured
/// on v392 (first sampled 14,500,000 steps after A's release) and checked
/// by eye; never compared with the physical badge, which runs v335.
const SCANNER_OPEN_HASH: u64 = 0x4958_3da5_06ff_ab49;
/// A's response: release to the first stable Scanner frame 14.5M, then
/// [`AFTER_PRESS_STABLE_FOR`] more to confirm, 22.5M; + 25%, rounded up to
/// 1,000,000.
const SCANNER_RESPONSE_STEPS: u64 = 29_000_000;
/// HOME's response from Scanner: release to the first stable launcher
/// 11.75M, then [`AFTER_PRESS_STABLE_FOR`] more, 19.75M; + 25%, rounded up
/// to 1,000,000.
const SCANNER_HOME_RESPONSE_STEPS: u64 = 25_000_000;

static SCANNER: OnceLock<FirmwareRuntime> = OnceLock::new();

/// From the hacker launcher: [`TO_SCANNER`], then A held
/// [`PRESS_HOLD_US`], then the first stable frame after it, asserted to be
/// [`SCANNER_OPEN_HASH`].
fn scanner_open() -> FirmwareRuntime {
    SCANNER
        .get_or_init(|| {
            let mut rt = launcher_for(OWN_ROLE);
            let mut previous = LAUNCHER_HASH;
            for &(slot, expected) in TO_SCANNER {
                let (hash, step) = press_and_settle(&mut rt, slot, previous);
                assert_eq!(hash, expected, "after slot {slot}, stable at step {step}");
                previous = hash;
            }
            press(&mut rt, 1);
            let cap = rt.total_steps() + SCANNER_RESPONSE_STEPS;
            let (hash, step) =
                run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[previous]);
            assert_eq!(
                hash, SCANNER_OPEN_HASH,
                "Scanner opened, stable at step {step}"
            );
            rt
        })
        .clone()
}

/// Milestone 9: v392 enters Scanner by rebooting into it. A on Scanner
/// logs `app_reboot: reboot into scanner`, `esp_restart()` reaches the
/// mask-ROM `software_reset_cpu` (`emulator_core::rom::SOFTWARE_RESET_CPU`),
/// and the runtime re-boots with the flash (identity, `reboot.cfg`) kept.
/// The new boot reads `reboot.cfg` and launches Scanner directly, exactly
/// once: one soft reset, no reboot loop. The frame is
/// [`SCANNER_OPEN_HASH`] (v392, measured, checked by eye).
#[test]
fn scanner_opens_after_one_reboot() {
    let rt = scanner_open();
    let console = rt.console_output();
    let reboot = console
        .find("app_reboot: reboot into scanner")
        .unwrap_or_else(|| panic!("no reboot line; console:\n{console}"));
    assert!(
        console[reboot..].contains("app_reg: launched Scanner"),
        "Scanner launched after the reboot; console:\n{console}"
    );
    assert_eq!(rt.soft_reset_count(), 1);
    assert_no_panic_text(&rt);
}

/// Leaving Scanner: HOME from the open Scanner. Measured on v392, the
/// exit path is a **direct return**, not a second reboot: the firmware
/// logs `heap after exit Scanner` and `launched Launcher` with no
/// `reboot into`, the soft-reset count stays 1, and the launcher frame
/// (first icon selected, [`LAUNCHER_HASH`]) is stable 11.75M steps after
/// the release.
#[test]
fn scanner_home_returns_to_the_launcher() {
    let mut rt = scanner_open();
    let before = rt.console_output().len();
    press(&mut rt, TO_LAUNCHER_SLOT);
    let cap = rt.total_steps() + SCANNER_HOME_RESPONSE_STEPS;
    let (hash, step) =
        run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[SCANNER_OPEN_HASH]);
    assert_eq!(hash, LAUNCHER_HASH, "launcher, stable at step {step}");
    let console = rt.console_output();
    let after = &console[before..];
    assert!(
        after.contains("app_reg: launched Launcher"),
        "console after HOME:\n{after}"
    );
    assert!(
        !after.contains("reboot into"),
        "console after HOME:\n{after}"
    );
    assert_eq!(
        rt.soft_reset_count(),
        1,
        "a direct return, no second reboot"
    );
    assert_no_panic_text(&rt);
}

/// Milestone 9 Task 4: every launcher app, from role hacker's launcher on
/// v392 (measured; every new frame checked by eye, none compared with the
/// physical badge, which runs v335). Cell = row × 4 + column. "Soft
/// resets" is [`FirmwareRuntime::soft_reset_count`] once the app is open:
/// v392 enters five apps by rebooting into them.
///
/// | Cell | App (`app_reg: launched …`) | Soft resets | Rung |
/// |---|---|---|---|
/// | 0 | My Badge | 0 | [`launcher_opens_my_badge`] |
/// | 1 | Connect | 0, stops at the Bluetooth ROM | [`connect_stops_at_the_bluetooth_controller`] |
/// | 2 | Scanner | 1 | [`scanner_opens_after_one_reboot`] |
/// | 3 | Sync | 1, stops at the Bluetooth ROM | [`sync_reboots_then_stops_at_the_bluetooth_controller`] |
/// | 4 | Share | 1 | [`launcher_opens_share_after_one_reboot`] |
/// | 5 | Solana Scavenger Hunt | 0 | [`launcher_opens_solana_scavenger_hunt`] |
/// | 6 | Sponsors | 1 | [`launcher_opens_sponsors_after_one_reboot`] |
/// | 7 | Workshop Quest | 1 | [`launcher_opens_workshop_quest_after_one_reboot`] |
/// | 8 | Mood LEDs | 0 | [`launcher_opens_mood_leds`] |
/// | 9 | Socials | 0 | [`launcher_opens_socials`] |
/// | 10 | Schedule | 0 | [`launcher_opens_schedule`] |
/// | 11 | Info & Rules | 0 | [`launcher_opens_info_and_rules`] |
/// | 12 | Museum | 0 | [`launcher_opens_museum`] |
/// | 13 | Progress | 0 | [`launcher_opens_progress`] |
/// | 14 | Snake | 0 | [`launcher_opens_snake`] |
/// | 15 | Dice | 0 | [`launcher_opens_dice`] |
///
/// Each entry here is the launcher's stable frame with that cell
/// selected. Cells 0, 1, 2, 4, 8 and 12 to 15 reuse [`LAUNCHER_HASH`],
/// [`TO_SCANNER`]'s and [`TO_DICE`]'s pins; the others are new (v392,
/// measured).
const LAUNCHER_SELECTED: [u64; 16] = [
    LAUNCHER_HASH,           // 0: My Badge
    TO_SCANNER[0].1,         // 1: Connect
    TO_SCANNER[1].1,         // 2: Scanner
    0x57d9_5cfd_9af2_b2cd,   // 3: Sync
    LAUNCHER_AFTER_NAV_HASH, // 4: Share
    0x2554_fbce_7c2c_3424,   // 5: Solana Scavenger Hunt
    0xf265_0006_b483_df82,   // 6: Sponsors
    0xc7c2_2146_83c2_df08,   // 7: Workshop Quest
    TO_DICE[1].1,            // 8: Mood LEDs
    0x93ae_2f49_c557_56ec,   // 9: Socials
    0xc8ea_e827_7574_714f,   // 10: Schedule
    0x13f5_f222_1252_28ec,   // 11: Info & Rules
    TO_DICE[2].1,            // 12: Museum
    TO_DICE[3].1,            // 13: Progress
    TO_DICE[4].1,            // 14: Snake
    TO_DICE[5].1,            // 15: Dice
];
/// RIGHT: moves the launcher's selection one column.
const RIGHT_SLOT: usize = 6;
/// A: opens the selected app.
const A_SLOT: usize = 1;

/// From hacker's launcher (first cell selected): DOWN × row, then RIGHT ×
/// column to `cell`, asserting each selection's frame from
/// [`LAUNCHER_SELECTED`], then A held [`PRESS_HOLD_US`]. Returns the
/// runtime just after A's release and the console's length before A.
fn press_a_on_cell(cell: usize) -> (FirmwareRuntime, usize) {
    let mut rt = launcher_for(OWN_ROLE);
    let mut at = 0;
    while at != cell {
        let (slot, next) = if at / 4 < cell / 4 {
            (NAV_SLOT, at + 4)
        } else {
            (RIGHT_SLOT, at + 1)
        };
        let (hash, step) = press_and_settle(&mut rt, slot, LAUNCHER_SELECTED[at]);
        assert_eq!(
            hash, LAUNCHER_SELECTED[next],
            "cell {next} selected, stable at step {step}"
        );
        at = next;
    }
    let before = rt.console_output().len();
    press(&mut rt, A_SLOT);
    (rt, before)
}

/// A on launcher `cell` opens `app`: within `response_steps` of A's
/// release the first frame other than the selection that holds for
/// `stable_for` steps is `open_hash`, the console since A says
/// `app_reg: launched <app>`, the runtime has soft-reset `soft_resets`
/// times, and nothing panicked.
fn assert_launcher_opens(
    cell: usize,
    app: &str,
    soft_resets: u32,
    response_steps: u64,
    stable_for: u64,
    open_hash: u64,
) {
    let (mut rt, before) = press_a_on_cell(cell);
    let cap = rt.total_steps() + response_steps;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, stable_for, &[LAUNCHER_SELECTED[cell]]);
    assert_eq!(hash, open_hash, "{app} opened, stable at step {step}");
    let console = rt.console_output();
    assert!(
        console[before..].contains(&format!("app_reg: launched {app}")),
        "console since A:\n{}",
        &console[before..]
    );
    assert_eq!(rt.soft_reset_count(), soft_resets, "{app}: soft resets");
    assert_no_panic_text(&rt);
}

/// One parallel rung per app: `name, cell, "app", soft_resets,
/// response_steps, open_hash`. Each response is the measured release to
/// first stable frame, then [`AFTER_PRESS_STABLE_FOR`] more to confirm;
/// + 25 %, rounded up to 1,000,000.
macro_rules! launcher_opens {
    ($(#[$doc:meta])* $name:ident, $cell:expr, $app:literal, $soft_resets:expr, $response:expr, $hash:expr) => {
        $(#[$doc])*
        #[test]
        fn $name() {
            assert_launcher_opens(
                $cell,
                $app,
                $soft_resets,
                $response,
                AFTER_PRESS_STABLE_FOR,
                $hash,
            );
        }
    };
}

launcher_opens!(
    /// My Badge opens on hacker's registered screen (role title, the QR
    /// code, "Test Hacker", the badge ID; `ROLE_ROWS`' hacker entry),
    /// first stable 9.0M steps after A's release: 22M.
    launcher_opens_my_badge,
    0,
    "My Badge",
    0,
    22_000_000,
    role_row(OWN_ROLE).1
);
launcher_opens!(
    /// Share reboots into itself (`app_reboot: reboot into share`):
    /// "Share Custom Apps", "Send an app" selected, "Receive an app",
    /// footer "Up/Down: choose  A: select". First stable 9.0M after the
    /// release: 22M.
    launcher_opens_share_after_one_reboot,
    4,
    "Share",
    1,
    22_000_000,
    0xfc51_aedc_c0ab_617d
);
launcher_opens!(
    /// Solana Scavenger Hunt: its title, the "Find boxes with the Solana
    /// logo …" instructions, footer "A: start hunting  Start: help
    /// anytime". First stable 5.25M after the release: 17M.
    launcher_opens_solana_scavenger_hunt,
    5,
    "Solana Scavenger Hunt",
    0,
    17_000_000,
    0x7ad3_a516_f050_78cf
);
/// How long Sponsors' frame must hold to count as loaded. Sponsors draws
/// its floor map (stable from 16.0M steps after A's release), then the
/// booth icons in four batches, 42 redraws in all; the pauses before the
/// batches are 8.25M, 8.5M, 8.75M and 9.5M steps (measured, v392), so
/// [`AFTER_PRESS_STABLE_FOR`] would stop on the bare map. This window is
/// over twice the longest pause; the last redraw is 60.5M after the
/// release, and the frame was unchanged through 150M.
const SPONSORS_LOADED_STABLE_FOR: u64 = 20_000_000;
/// Sponsors' loaded frame: "Sponsors", "0/64 0%", the floor map ("Floor
/// 1", "Floor 2", "Extended Sponsor Bay") covered in "?" booth icons, the
/// first one outlined, footer "A SPONSOR  B INFO  START LIST  HOME APPS".
/// Measured on v392 and checked by eye; never compared with the physical
/// badge, which runs v335.
const SPONSORS_LOADED_HASH: u64 = 0xcdc7_9972_60aa_f1c8;

/// Sponsors reboots into itself (`app_reboot: reboot into sponsors`) and
/// logs `hal_nfc: rc522_start: ESP_ERR_INVALID_STATE` (NFC is not
/// modeled), as Scanner does. Pinned on its loaded frame (the human
/// partner's ruling at the Task 5 browser check, superseding R-T4-1),
/// [`SPONSORS_LOADED_HASH`]: last redrawn 60.5M steps after A's release,
/// then [`SPONSORS_LOADED_STABLE_FOR`] more to confirm, 80.5M; + 25 %,
/// rounded up to 1,000,000: 101M.
#[test]
fn launcher_opens_sponsors_after_one_reboot() {
    assert_launcher_opens(
        6,
        "Sponsors",
        1,
        101_000_000,
        SPONSORS_LOADED_STABLE_FOR,
        SPONSORS_LOADED_HASH,
    );
}
launcher_opens!(
    /// Workshop Quest reboots into itself (`app_reboot: reboot into
    /// workshops`) and logs the same NFC error as Sponsors: "Workshop
    /// Quest", "0/5", "Internship Maxxing Workshop", a grid of 15 hexagon
    /// tags ("INT" selected), footer "SCAN TAG  D-PAD BROWSE  B PAGE
    /// 1/3". First stable 21.75M after the release: 38M.
    launcher_opens_workshop_quest_after_one_reboot,
    7,
    "Workshop Quest",
    1,
    38_000_000,
    0x894d_5dae_e0d9_8e2a
);
launcher_opens!(
    /// Mood LEDs: "Choose a mood, then leave this app open …", "Green
    /// means: looking for a team!", "Press A to choose your mood", footer
    /// "HOME: apps". First stable 8.0M after the release: 20M.
    launcher_opens_mood_leds,
    8,
    "Mood LEDs",
    0,
    20_000_000,
    0x0c6c_07b5_b963_4775
);
launcher_opens!(
    /// Socials: "My Socials", "No social profiles are loaded yet." (the
    /// fake identity has none), footer "HOME apps". First stable 4.0M
    /// after the release: 15M.
    launcher_opens_socials,
    9,
    "Socials",
    0,
    15_000_000,
    0xeedf_ef97_7fd2_ea52
);
launcher_opens!(
    /// Schedule: a QR code for `https://my.hackthenorth.com/schedule`,
    /// footer "HOME to return". First stable 8.25M after the release: 21M.
    launcher_opens_schedule,
    10,
    "Schedule",
    0,
    21_000_000,
    0x59e7_950e_07fb_f1e9
);
launcher_opens!(
    /// Info & Rules: a QR code, "Scan with your phone for badge rules.",
    /// footer "HOME apps". First stable 8.0M after the release: 20M.
    launcher_opens_info_and_rules,
    11,
    "Info & Rules",
    0,
    20_000_000,
    0x3472_9924_2cdb_54aa
);
launcher_opens!(
    /// Museum: a QR code for `https://museum.hackthenorth.com`, footer
    /// "Scan to explore | HOME to return". First stable 8.25M after the
    /// release: 21M.
    launcher_opens_museum,
    12,
    "Museum",
    0,
    21_000_000,
    0xd7f7_b7e2_9354_c399
);
launcher_opens!(
    /// Progress: a "0/107" ring, "Apps 0/34", "Sponsors 0/64", "Solana
    /// Scavenger Hunt 0/9", footer "A Clues  Home Apps". First stable
    /// 5.75M after the release: 18M.
    launcher_opens_progress,
    13,
    "Progress",
    0,
    18_000_000,
    0x2651_4b50_3ed5_3c44
);

/// The mask-ROM address Connect and Sync fetch after `hal_radio:
/// init_once`: `btdm_controller_rom_data_init`
/// (`esp32c3.rom.bt_funcs.ld`, ESP-IDF v5.5.3). It has no stub, so the
/// fetch traps (an unmapped instruction fetch always does) and the
/// firmware's panic handler prints the register dump. Milestone 10 lifts
/// it.
const BLUETOOTH_ROM_FAULT_PC: u32 = 0x4000_08ec;
/// The panic handler's register-dump line for [`BLUETOOTH_ROM_FAULT_PC`].
const BLUETOOTH_ROM_FAULT_MEPC: &str = "MEPC    : 0x400008ec";

/// Runs `rt` in [`FRAME_CHUNK`]-step chunks until the console since
/// `before` shows [`BLUETOOTH_ROM_FAULT_MEPC`], asserting that the only
/// instruction fault on the way is at [`BLUETOOTH_ROM_FAULT_PC`] and that
/// one happened. Returns the console since `before`. Panics if the line
/// is not there by `max_steps` (total steps).
fn run_to_the_bluetooth_fault(rt: &mut FirmwareRuntime, before: usize, max_steps: u64) -> String {
    let mut faulted = false;
    while rt.total_steps() < max_steps {
        let summary = rt.run(FRAME_CHUNK);
        if let Some(pc) = summary.last_instruction_fault {
            assert_eq!(pc, BLUETOOTH_ROM_FAULT_PC, "{summary:?}");
            faulted = true;
        }
        let since = rt.console_output()[before..].to_string();
        if since.contains(BLUETOOTH_ROM_FAULT_MEPC) {
            assert!(faulted, "MEPC printed without a fault:\n{since}");
            return since;
        }
    }
    panic!(
        "no {BLUETOOTH_ROM_FAULT_MEPC:?} within {max_steps} steps; pc=0x{:08x}\n{}",
        rt.pc(),
        &rt.console_output()[before..]
    );
}

/// Connect (cell 1) does not open on v392 yet: after A, `hal_radio:
/// init_once` and then the fetch of [`BLUETOOTH_ROM_FAULT_PC`]; the
/// register dump is printed 0.75M steps after A's release (cap: + 25 %,
/// rounded up to 1,000,000). No soft reset comes first. Measured after
/// the dump (not pinned): the panic handler prints `Rebooting...`, the
/// runtime soft-resets, and the badge boots with `reset reason=4` into
/// My Badge, with no reboot loop. Milestone 10 (Bluetooth) lifts the
/// stop.
#[test]
fn connect_stops_at_the_bluetooth_controller() {
    let (mut rt, before) = press_a_on_cell(1);
    let cap = rt.total_steps() + 1_000_000;
    let console = run_to_the_bluetooth_fault(&mut rt, before, cap);
    let radio = console
        .find("hal_radio: init_once")
        .unwrap_or_else(|| panic!("console since A:\n{console}"));
    assert!(
        console[radio..].contains(BLUETOOTH_ROM_FAULT_MEPC),
        "console since A:\n{console}"
    );
    assert!(
        !console.contains("app_reg: launched Connect"),
        "console since A:\n{console}"
    );
    assert_eq!(rt.soft_reset_count(), 0);
}

/// Sync (cell 3) reboots into itself (`app_reboot: reboot into sync`,
/// one soft reset), then stops like Connect: after the reboot,
/// `hal_radio: init_once` and the fetch of [`BLUETOOTH_ROM_FAULT_PC`],
/// with no `launched Sync`. The register dump is printed 6.75M steps
/// after A's release (cap: + 25 %, rounded up to 1,000,000). Measured
/// after the dump (not pinned): `Rebooting...`, a second soft reset, and
/// a boot with `reset reason=4` into My Badge, with no reboot loop.
/// Milestone 10 (Bluetooth) lifts the stop.
#[test]
fn sync_reboots_then_stops_at_the_bluetooth_controller() {
    let (mut rt, before) = press_a_on_cell(3);
    let cap = rt.total_steps() + 9_000_000;
    let console = run_to_the_bluetooth_fault(&mut rt, before, cap);
    let reboot = console
        .find("app_reboot: reboot into sync")
        .unwrap_or_else(|| panic!("no reboot line; console since A:\n{console}"));
    let radio = reboot
        + console[reboot..]
            .find("hal_radio: init_once")
            .unwrap_or_else(|| panic!("console since A:\n{console}"));
    assert!(
        console[radio..].contains(BLUETOOTH_ROM_FAULT_MEPC),
        "console since A:\n{console}"
    );
    assert!(
        !console.contains("app_reg: launched Sync"),
        "console since A:\n{console}"
    );
    assert_eq!(rt.soft_reset_count(), 1);
}

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

/// Snake's play area (the grid, without the title/score text and without
/// its 1-px border), measured from the Task 4 exploration's start frame
/// (100 ms after A's release): along row 140 the border color starts at
/// x = 14 and x = 305, along column 100 at y = 40 and y = 235; the area is
/// the 290 x 194 px inside them.
const PLAY_AREA: Rect = Rect {
    x0: 15,
    y0: 41,
    x1: 305,
    y1: 235,
};

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

/// When [`launcher_opens_snake`] samples Snake's frame, in emulated time
/// after A's release. Snake does not wait for a key: it draws its start
/// screen (snake of three cells, heading right, at the play area's middle
/// row) in stripes from 56 ms to 75.5 ms after release and makes its first
/// move at about 222 ms, so 150 ms is mid-way through the start frame's
/// hold.
const SNAKE_OPEN_FRAME_US: u64 = 150_000;
/// Snake's start screen: title "Snake", "Start to pause", "Score 0 Hi 0",
/// the empty play area with the snake and one food dot (its position comes
/// from the emulator's fixed-seed RNG, `peripherals::apb_ctrl::RNG_SEED`,
/// so it is not the badge's). Compared with the physical badge on v335,
/// 2026-10-20 (Task 5): the layout matches; the food position differs.
const SNAKE_OPEN_HASH: u64 = 0xe7d5_38c7_42f1_9d89;

/// Milestone 6: Snake opens from the launcher. Its frame
/// [`SNAKE_OPEN_FRAME_US`] after A's release is pinned (Snake moves without
/// waiting for a key, so no frame is stable for [`AFTER_PRESS_STABLE_FOR`]);
/// the frame was compared with the physical badge on v335, 2026-10-20 (Task 5).
#[test]
fn launcher_opens_snake() {
    let mut rt = snake_open();
    run_for_us(&mut rt, SNAKE_OPEN_FRAME_US);
    let hash = framebuffer_fnv1a(rt.framebuffer());
    assert_eq!(
        hash,
        SNAKE_OPEN_HASH,
        "Snake opened (step {})",
        rt.total_steps()
    );
    assert!(
        rt.console_output().contains("app_reg: launched Snake"),
        "console:\n{}",
        rt.console_output()
    );
    assert_no_panic_text(&rt);
}

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

/// [`snake_hits_wall_game_over`]'s window, from the start frame
/// ([`SNAKE_OPEN_FRAME_US`] after release): the game-over overlay is
/// drawn about 1,964 ms after that point (2,114 ms after release); that
/// + 50 %, rounded up to 3 s.
const SNAKE_TO_WALL_WINDOW_US: u64 = 3_000_000;
/// [`PLAY_AREA`] changes from the start frame to game over with no input:
/// 11 moves right (one cell per 170 ms tick, from the middle of the area to
/// the right-most column), then the game-over redraw on the 12th tick,
/// which [`move_times`] counts as the 12th change. The badge also makes
/// 11 moves to the wall (compared on v335, 2026-10-20, Task 5).
const SNAKE_MOVES_TO_WALL: usize = 12;
/// The game-over frame: a "Game over / A to restart" box over the play
/// area, the snake's head in the right-most column. Compared with the
/// physical badge on v335, 2026-10-20 (Task 5).
const SNAKE_GAME_OVER_HASH: u64 = 0x3582_df91_6283_9f3a;
/// The physical badge's time between Snake moves: the mean over 10
/// intervals of an untouched run into the wall, measured frame by frame
/// from the human's phone video of the badge at the Task 5 human gate
/// (2026-10-20; ±3 ms). The emulator's 170,000 us matches it.
const BADGE_MOVE_INTERVAL_US: u64 = 170_000;
// `SNAKE_TO_WALL_WINDOW_US` must hold the badge's own run to the wall:
// `SNAKE_MOVES_TO_WALL` ticks at the badge's pace.
const _: () =
    assert!(SNAKE_MOVES_TO_WALL as u64 * BADGE_MOVE_INTERVAL_US < SNAKE_TO_WALL_WINDOW_US);

/// How long [`snake_hits_wall_game_over`] watches for moves after A's
/// release on game over. The restarted game redraws its start screen
/// 22 ms to 34 ms into the 100 ms hold and makes its first move about
/// 176 ms after the press (76 ms after release, as sampled), then one
/// every 170 ms: moves at about 76, 246, 416 and 586 ms after release.
/// The rung needs three; even with every delay 25 % longer the third
/// comes at about 1.25 x (176 + 340) - 100 = 545 ms, inside 600 ms.
const SNAKE_RESTART_WINDOW_US: u64 = 600_000;

/// With no input the snake runs into the wall: "Game over / A to
/// restart" after [`SNAKE_MOVES_TO_WALL`] changes of the play area (the
/// count and the frame were compared with the badge on v335, 2026-10-20, Task
/// 5); A restarts, and the new game moves at the badge's pace.
#[test]
fn snake_hits_wall_game_over() {
    let mut rt = snake_open();
    run_for_us(&mut rt, SNAKE_OPEN_FRAME_US);
    let moves = move_times(&mut rt, SNAKE_TO_WALL_WINDOW_US);
    assert_eq!(moves.len(), SNAKE_MOVES_TO_WALL, "moves at {moves:?}");
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, BOOT_STABLE_FOR, &[]);
    assert_eq!(
        hash, SNAKE_GAME_OVER_HASH,
        "game over, stable at step {step}"
    );
    // A restarts at once and the new game runs without a key (the next
    // stable frame is the next game over), so the check is that the game
    // over box is gone and the play area moves again, at the badge's pace.
    press(&mut rt, 1);
    assert_ne!(
        framebuffer_fnv1a(rt.framebuffer()),
        SNAKE_GAME_OVER_HASH,
        "still game over after A"
    );
    let moves = move_times(&mut rt, SNAKE_RESTART_WINDOW_US);
    assert!(moves.len() >= 3, "restarted game, moves at {moves:?}");
    let interval = median_interval(&moves);
    let badge = BADGE_MOVE_INTERVAL_US;
    assert!(
        interval.abs_diff(badge) <= badge / 4,
        "restarted game: move interval {interval} us vs badge {badge} us; moves at {moves:?}"
    );
    assert_no_panic_text(&rt);
}

/// Median gap between consecutive move start times. Panics with fewer
/// than three moves (two intervals).
fn median_interval(times: &[u64]) -> u64 {
    assert!(times.len() >= 3, "too few moves: {times:?}");
    let mut gaps: Vec<u64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    gaps[gaps.len() / 2]
}

/// [`snake_moves_on_its_own`]'s window, from the start frame
/// ([`SNAKE_OPEN_FRAME_US`] after release). The first move comes 72 ms
/// after that point and then one every 170 ms (Task 4), so six moves need
/// 72 + 5 x 170 = 922 ms. At the slow edge of the rung's ±25 % band
/// (212.5 ms) six moves need 72 + 5 x 212.5 = 1,134.5 ms; at the fast
/// edge (127.5 ms) 1.2 s holds at most 1 + (1,200 - 72) / 127.5 = 9
/// moves, short of the 11th move that reaches the wall. So the window
/// sees at least six moves and no game-over redraw across the band (at
/// the emulator's 170 ms: seven moves, the last at 1,242 ms after
/// release, well before the game-over redraw at 2,106 ms).
const SPEED_WINDOW_US: u64 = 1_200_000;

/// The emulator's move period: exact and deterministic, every interval
/// between Snake's moves 1 and 11 measured at 170,000 us (Tasks 4 and 5).
/// It agrees with the physical badge's [`BADGE_MOVE_INTERVAL_US`]
/// (Task 5's human gate, 2026-10-20).
const SNAKE_MOVE_INTERVAL_US: u64 = 170_000;

/// Milestone 6: the snake moves on its own at the badge's speed. Two
/// checks (ruling R-T6-3):
/// - every gap between consecutive moves is exactly
///   [`SNAKE_MOVE_INTERVAL_US`]. This pin guards the timing model: under
///   the pre-Milestone-6 one-tick-per-step clock (from A's release) the
///   moves come 160,000 us apart and the rung fails (Task 6 Step 2);
/// - the median gap is within 25 % of the physical badge's
///   [`BADGE_MOVE_INTERVAL_US`] (Task 5): the badge comparison.
///
/// Counting starts at the start frame, as in
/// [`snake_hits_wall_game_over`], so the start-screen redraw is not a
/// move, and [`SPEED_WINDOW_US`] ends before the game-over redraw (the
/// 11th move is at about 1,922 ms after release, the redraw at 2,106 ms),
/// so every counted change is a move.
#[test]
fn snake_moves_on_its_own() {
    let mut rt = snake_open();
    run_for_us(&mut rt, SNAKE_OPEN_FRAME_US);
    let moves = move_times(&mut rt, SPEED_WINDOW_US);
    assert!(moves.len() >= 6, "moves at {moves:?}");
    assert!(
        moves
            .windows(2)
            .all(|w| w[1] - w[0] == SNAKE_MOVE_INTERVAL_US),
        "move gaps not all {SNAKE_MOVE_INTERVAL_US} us; moves at {moves:?}"
    );
    let interval = median_interval(&moves);
    let badge = BADGE_MOVE_INTERVAL_US;
    assert!(
        interval.abs_diff(badge) <= badge / 4,
        "emulated move interval {interval} us vs badge {badge} us; moves at {moves:?}"
    );
    assert_no_panic_text(&rt);
}

/// The button [`snake_steers`] presses: UP (shift-register bit 6; the
/// frontend's `RAW_SLOT_BY_BUTTON`). The snake starts heading right, so UP
/// turns it.
const SNAKE_TURN_SLOT: usize = 7;
/// The frame after one turn UP: the snake is an L, its head one cell
/// above the second cell (head at (172, 126)). The human compared this
/// frame with the badge on v335, 2026-10-20 (Task 5's `after-turn` frame).
const SNAKE_TURNED_HASH: u64 = 0xcccb_b8c5_d80a_93e9;

/// A direction press turns the snake on the next move. Timing, from A's
/// release: [`move_times`] watches 1.5 move intervals (to about 255 ms),
/// which sees the start-screen redraw and the first move (222 ms);
/// [`SNAKE_TURN_SLOT`] is then pressed for [`PRESS_HOLD_US`] (to about
/// 355 ms); the second tick (392 ms) moves the head up; the frame one
/// move interval after release (about 525 ms; stable from 395 ms to the
/// third tick at 562 ms) is pinned. Task 5 pressed UP at 240 ms and read
/// the frame at 485 ms, the same stable frame. Afterwards the snake keeps
/// moving (now upward).
#[test]
fn snake_steers() {
    let mut rt = snake_open();
    let first = move_times(&mut rt, BADGE_MOVE_INTERVAL_US * 3 / 2);
    // The start-screen redraw (60 ms) and the first move (222 ms).
    assert!(first.len() >= 2, "no first move: {first:?}");
    press(&mut rt, SNAKE_TURN_SLOT);
    run_for_us(&mut rt, BADGE_MOVE_INTERVAL_US);
    assert_eq!(
        framebuffer_fnv1a(rt.framebuffer()),
        SNAKE_TURNED_HASH,
        "after the turn (step {}, {} us)",
        rt.total_steps(),
        rt.emulated_time_us()
    );
    let more = move_times(&mut rt, SNAKE_TO_WALL_WINDOW_US / 2);
    assert!(more.len() >= 3, "kept moving after the turn: {more:?}");
    assert_no_panic_text(&rt);
}

/// DOWN and RIGHT (shift-register bits 3 and 5; the frontend's
/// `RAW_SLOT_BY_BUTTON`), [`snake_eats`]'s two turns.
const SNAKE_DOWN_SLOT: usize = 4;
const SNAKE_RIGHT_SLOT: usize = 6;
/// The snake's body color (RGB565), the bulk of each 11 x 11 px cell; its
/// edges use two darker shades. At the start frame [`PLAY_AREA`] holds
/// 327 pixels of it (three cells).
const SNAKE_BODY_COLOR: u16 = 0x07f5;
/// [`snake_eats`]' wait after RIGHT's release: 9.5 move intervals, which
/// lands mid-way through the after-eat frame's hold (see the rung).
const SNAKE_EAT_FRAME_US: u64 = 9 * SNAKE_MOVE_INTERVAL_US + SNAKE_MOVE_INTERVAL_US / 2;
/// The frame after the eat: the snake four cells long along row 14, its
/// head in the right-most column on the old food cell, the new food at
/// column 15, row 13, and the score 1. Not compared with the physical badge
/// (the food position is the emulator's fixed-seed RNG's, see
/// [`SNAKE_OPEN_HASH`]).
const SNAKE_ATE_HASH: u64 = 0x8275_2f57_3ba4_ddab;

/// Milestone 6 Task 8 (stretch): the snake eats and grows. The grid is
/// 24 x 16 cells of 12 px from (16, 42) (cell (c, r) is the 11 x 11 px
/// square at x = 16 + 12c, y = 42 + 12r). At the start frame the snake
/// fills (10..=12, 8), head at (12, 8) heading right, and the food is at
/// (23, 14) (pixels x 292..=302, y 210..=220). The food comes from
/// `peripherals::apb_ctrl`'s fixed-seed RNG and is part of the pinned
/// [`SNAKE_OPEN_HASH`]; the emulator reads no host time or entropy, so it
/// is the same on every run. The path: one move right to (13, 8), DOWN for
/// six moves to (13, 14), RIGHT for ten moves to (23, 14).
///
/// Timing, from A's release (moves at 222 ms + k x 170 ms): the start
/// frame (150 ms) is sampled for the snake's pixel count; [`move_times`]
/// watches to 255 ms, past the first move, as in [`snake_steers`]; DOWN is
/// held to 355 ms, so move 2 (392 ms) turns down and move 7 (1,242 ms)
/// reaches row 14. Six intervals after DOWN, RIGHT is pressed at the same
/// phase (1,276 ms to 1,376 ms), so move 8 (1,412 ms) turns right and move
/// 17 (2,942 ms) enters the food cell; the after-eat frame is drawn by
/// 2,948 ms and holds past 3,161 ms. [`SNAKE_EAT_FRAME_US`] samples it at
/// 2,991 ms. The next move hits the wall (game over).
#[test]
fn snake_eats() {
    let mut rt = snake_open();
    run_for_us(&mut rt, SNAKE_OPEN_FRAME_US);
    let at_open = color_count(&rt, PLAY_AREA, SNAKE_BODY_COLOR);
    let first = move_times(
        &mut rt,
        BADGE_MOVE_INTERVAL_US * 3 / 2 - SNAKE_OPEN_FRAME_US,
    );
    assert!(!first.is_empty(), "no first move");
    press(&mut rt, SNAKE_DOWN_SLOT);
    run_for_us(&mut rt, 6 * SNAKE_MOVE_INTERVAL_US - PRESS_HOLD_US);
    press(&mut rt, SNAKE_RIGHT_SLOT);
    run_for_us(&mut rt, SNAKE_EAT_FRAME_US);
    assert_eq!(
        framebuffer_fnv1a(rt.framebuffer()),
        SNAKE_ATE_HASH,
        "after the eat (step {}, {} us)",
        rt.total_steps(),
        rt.emulated_time_us()
    );
    let after = color_count(&rt, PLAY_AREA, SNAKE_BODY_COLOR);
    assert!(
        after > at_open,
        "snake did not grow: {at_open} -> {after} px"
    );
    assert_no_panic_text(&rt);
}

/// Pixels of `color` inside `rect`.
fn color_count(rt: &FirmwareRuntime, rect: Rect, color: u16) -> usize {
    let width = rt.screen_width();
    let fb = rt.framebuffer();
    (rect.y0..rect.y1)
        .map(|y| {
            fb[y * width + rect.x0..y * width + rect.x1]
                .iter()
                .filter(|&&p| p == color)
                .count()
        })
        .sum()
}

/// One 120 Hz browser frame of emulated time.
const BROWSER_FRAME_US: u64 = 8_333;

/// Milestone 6 Task 7: the browser's frame pacing tracks the wall clock.
/// On the idle first-run screen, each `run_until` of one 120 Hz frame ends
/// exactly at its target. Before `boot::idle_until` bounded the WFI
/// fast-forward, every frame ran on to FreeRTOS's next 10 ms tick, 1,667 us
/// late, and the badge ran at 1.2x in the browser (also measured in Snake:
/// 1.197x).
#[test]
fn run_until_ends_idle_frames_at_the_target() {
    let mut rt = first_run_screen();
    let start = rt.emulated_time_us();
    for frame in 0..120 {
        let target = rt.emulated_time_us() + BROWSER_FRAME_US;
        let s = rt.run_until(target, 1_000_000);
        assert_eq!(s.last_instruction_fault, None, "{s:?}");
        assert_eq!(rt.emulated_time_us(), target, "frame {frame}: {s:?}");
    }
    assert_eq!(rt.emulated_time_us() - start, 120 * BROWSER_FRAME_US);
}

// ---------------------------------------------------------------------
// Milestone 7: equivalence rungs and the redraw benchmark.
//
// Milestone 7 is pure performance work: nothing the firmware can observe
// may change. The two rungs below pin, at `6cd0a6e` (the Milestone 6
// merge), what the core executes over early boot and where the longest
// checkpoint chain ends. Their constants were measured at `6cd0a6e` and
// must never be re-pinned in Milestone 7: a mismatch is a bug in the
// change. Milestone 9 swapped the firmware to v392 and re-pinned them
// once, from the fully re-pinned walk (Task 2; the values are in
// `docs/milestone-9-decisions.md`, "Re-pins"); from then on the same
// rule holds. `bench_redraws` (ignored) measures the busy redraws the
// milestone is about; its results go in `docs/milestone-7-decisions.md`.
// ---------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x100_0000_01b3;

/// FNV-1a (64-bit) over `bytes`, continuing from `h`.
fn fnv1a_extend(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// FNV-1a (64-bit) over `bytes`.
fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    fnv1a_extend(FNV_OFFSET, bytes)
}

/// How often [`execution_digest`] folds in the register file.
const DIGEST_REGS_EVERY: u64 = 1_000_000;

/// Boots blank flash and runs `steps` single steps, folding into an
/// FNV-1a hash the `pc` after every step (4 little-endian bytes) and, every
/// [`DIGEST_REGS_EVERY`] steps, all 32 registers (x0..x31, same encoding).
fn execution_digest(steps: u64) -> u64 {
    let mut rt = FirmwareRuntime::from_image(&firmware()).expect("boot");
    let mut h = FNV_OFFSET;
    for step in 1..=steps {
        rt.run(1);
        h = fnv1a_extend(h, &rt.pc().to_le_bytes());
        if step % DIGEST_REGS_EVERY == 0 {
            for r in 0..32 {
                h = fnv1a_extend(h, &rt.cpu().regs.read(r).to_le_bytes());
            }
        }
    }
    h
}

/// [`execution_digest`] of the first 20,000,000 steps of blank-flash boot
/// (through the console REPL's start), pinned at `6cd0a6e` and re-pinned for
/// v392 in Milestone 9 (v335: `0x30e1_c4af_04a0_f326`). Never re-pin
/// it for a performance change.
const EXECUTION_DIGEST: u64 = 0x0ae0_2ec4_374c_d285;

/// Milestone 7 equivalence proof: FNV-1a over every step's `pc` for the
/// first 20M steps of blank-flash boot, plus all 32 registers every
/// 1,000,000 steps. Any change to what the core executes moves it.
#[test]
fn execution_digest_is_unchanged() {
    assert_eq!(execution_digest(20_000_000), EXECUTION_DIGEST);
}

/// Total steps at hacker's launcher checkpoint ([`launcher_for`]), pinned
/// at `6cd0a6e`, re-pinned for v392 in Milestone 9. Never re-pin it for a
/// performance change.
const HACKER_LAUNCHER_STEPS: u64 = 342_600_776;
/// Emulated time at hacker's launcher checkpoint, in microseconds, pinned
/// at `6cd0a6e`, re-pinned for v392 in Milestone 9. Never re-pin it for a
/// performance change.
const HACKER_LAUNCHER_US: u64 = 94_521_615;
/// FNV-1a of the whole console output at hacker's launcher checkpoint,
/// pinned at `6cd0a6e`, re-pinned for v392 in Milestone 9. Never re-pin
/// it for a performance change.
const HACKER_LAUNCHER_CONSOLE_HASH: u64 = 0xabcd_cc17_7a35_80fe;

/// Milestone 7 equivalence proof at the end of the longest chain: hacker's
/// launcher checkpoint (`launcher_for`).
#[test]
fn hacker_walk_totals_are_unchanged() {
    let rt = launcher_for(OWN_ROLE);
    assert_eq!(rt.total_steps(), HACKER_LAUNCHER_STEPS);
    assert_eq!(rt.emulated_time_us(), HACKER_LAUNCHER_US);
    assert_eq!(
        fnv1a_bytes(rt.console_output().as_bytes()),
        HACKER_LAUNCHER_CONSOLE_HASH
    );
}

/// A press-driven redraw, measured by [`measure_redraw`] from the press:
/// the last frame change within the window (`emulated_us` after the
/// press, on the 1 ms sampling grid, and `steps` after the press) and the
/// frame it changed to (`end_hash`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Redraw {
    emulated_us: u64,
    steps: u64,
    end_hash: u64,
}

/// HOME from hacker's registered screen to the launcher (3 s window),
/// pinned at `6cd0a6e`, re-pinned for v392 in Milestone 9. Never re-pin
/// it for a performance change.
const HOME_REDRAW: Redraw = Redraw {
    emulated_us: 244_000,
    steps: 18_292_336,
    end_hash: LAUNCHER_HASH,
};
/// DOWN on hacker's launcher (1.5 s window), pinned at `6cd0a6e`,
/// re-pinned for v392 in Milestone 9. Never re-pin it for a performance
/// change.
const DOWN_REDRAW: Redraw = Redraw {
    emulated_us: 109_000,
    steps: 7_942_619,
    end_hash: LAUNCHER_AFTER_NAV_HASH,
};
/// A from onboarding page 1 to page 2 (role hacker, 3 s window), pinned
/// at `6cd0a6e`, re-pinned for v392 in Milestone 9. Never re-pin it for a
/// performance change.
const ONBOARDING_REDRAW: Redraw = Redraw {
    emulated_us: 52_000,
    steps: 2_547_134,
    end_hash: TO_SHAKE_PAGE[0].1,
};

/// [`measure_redraw`]'s framebuffer sampling period, in emulated time.
const REDRAW_SAMPLE_US: u64 = 1_000;
/// How many unsampled timed runs [`measure_redraw`] makes.
const REDRAW_TIMED_RUNS: usize = 5;

/// SYSTIMER ticks spent executing (not fast-forwarding a WFI wait).
fn busy_ticks(rt: &FirmwareRuntime) -> u64 {
    rt.bus().clock.total_ticks() - rt.bus().clock.idle_ticks()
}

/// Steps per [`FirmwareRuntime::run_until`] call in the benchmark: far
/// more than any 1 ms of emulated time takes (80,000 at 80 MHz).
const BENCH_MAX_STEPS: u32 = u32::MAX;

/// Measures the redraw that pressing `slot` on `base` starts.
///
/// Sampled pass, on a clone: press `slot`, then run in
/// [`REDRAW_SAMPLE_US`] slices of emulated time (`run_until`) for
/// `window_us`, hashing the framebuffer after each; the slot is released
/// once [`PRESS_HOLD_US`] has run (before the slice that ends at
/// `PRESS_HOLD_US` + 1,000 µs). The last slice whose frame differs from
/// the one before gives the [`Redraw`], with its busy ticks.
///
/// Timed passes, [`REDRAW_TIMED_RUNS`] of them, each on a fresh clone and
/// without sampling: the same press, run to the redraw's end time, with
/// the release at [`PRESS_HOLD_US`] if the end lies past it (onboarding's
/// does not). Each must end on the redraw's frame after the redraw's
/// steps. Returns the redraw, its busy ticks and the timed passes' wall
/// times (seconds, sorted).
fn measure_redraw(base: &FirmwareRuntime, slot: usize, window_us: u64) -> (Redraw, u64, Vec<f64>) {
    let mut rt = base.clone();
    let t0 = rt.emulated_time_us();
    let s0 = rt.total_steps();
    let b0 = busy_ticks(&rt);
    let mut last = framebuffer_fnv1a(rt.framebuffer());
    let mut redraw = Redraw {
        emulated_us: 0,
        steps: 0,
        end_hash: last,
    };
    let mut busy = 0;
    rt.set_raw_button(slot, true);
    let mut t = 0;
    while t < window_us {
        if t == PRESS_HOLD_US {
            rt.set_raw_button(slot, false);
        }
        t += REDRAW_SAMPLE_US;
        let s = rt.run_until(t0 + t, BENCH_MAX_STEPS);
        assert_eq!(s.last_instruction_fault, None, "{s:?}");
        let h = framebuffer_fnv1a(rt.framebuffer());
        if h != last {
            last = h;
            redraw = Redraw {
                emulated_us: t,
                steps: rt.total_steps() - s0,
                end_hash: h,
            };
            busy = busy_ticks(&rt) - b0;
        }
    }
    assert!(redraw.emulated_us > 0, "no frame change in {window_us} us");

    let mut walls = Vec::with_capacity(REDRAW_TIMED_RUNS);
    for _ in 0..REDRAW_TIMED_RUNS {
        let mut rt = base.clone();
        let start = std::time::Instant::now();
        rt.set_raw_button(slot, true);
        if redraw.emulated_us > PRESS_HOLD_US {
            rt.run_until(t0 + PRESS_HOLD_US, BENCH_MAX_STEPS);
            rt.set_raw_button(slot, false);
        }
        rt.run_until(t0 + redraw.emulated_us, BENCH_MAX_STEPS);
        walls.push(start.elapsed().as_secs_f64());
        assert_eq!(
            framebuffer_fnv1a(rt.framebuffer()),
            redraw.end_hash,
            "the unsampled run ends on another frame"
        );
        // `run_until`'s slicing is invisible to the guest, so the
        // unsampled run takes exactly the sampled run's steps.
        assert_eq!(rt.total_steps() - s0, redraw.steps, "unsampled steps");
    }
    walls.sort_by(f64::total_cmp);
    (redraw, busy, walls)
}

/// Milestone 7's native benchmark (`--ignored`; run it in release with
/// `--nocapture`): times the walk to hacker's launcher, then measures the
/// three busy redraws ([`HOME_REDRAW`], [`DOWN_REDRAW`],
/// [`ONBOARDING_REDRAW`]) with [`measure_redraw`] and checks each against
/// its pin. Prints one `BENCH` line per item; `factor` is emulated time
/// over the median wall time (1.0 is the badge's speed).
#[test]
#[ignore]
fn bench_redraws() {
    let start = std::time::Instant::now();
    let launcher = launcher_for(OWN_ROLE);
    let wall = start.elapsed().as_secs_f64();
    let steps = launcher.total_steps();
    println!(
        "BENCH walk steps={steps} wall_s={wall:.2} msteps_per_s={:.2}",
        steps as f64 / wall / 1e6
    );
    let timer_hz = launcher.bus().clock.timer_hz();
    let items: [(&str, FirmwareRuntime, usize, u64, Redraw); 3] = [
        (
            "home",
            registered(OWN_ROLE),
            TO_LAUNCHER_SLOT,
            3_000_000,
            HOME_REDRAW,
        ),
        ("down", launcher, NAV_SLOT, 1_500_000, DOWN_REDRAW),
        (
            "onboarding",
            provisioned(OWN_ROLE),
            TO_SHAKE_PAGE[0].0,
            3_000_000,
            ONBOARDING_REDRAW,
        ),
    ];
    let mut measured = Vec::new();
    for (name, base, slot, window_us, pinned) in items {
        let (redraw, busy, walls) = measure_redraw(&base, slot, window_us);
        let median = walls[walls.len() / 2];
        let window_ticks = redraw.emulated_us as f64 * timer_hz as f64 / 1e6;
        println!(
            "BENCH {name} emulated_ms={:.1} steps={} busy_pct={:.1} wall_ms_median={:.1} \
             wall_ms_min={:.1} factor={:.3} msteps_per_s={:.2}",
            redraw.emulated_us as f64 / 1e3,
            redraw.steps,
            100.0 * busy as f64 / window_ticks,
            median * 1e3,
            walls[0] * 1e3,
            redraw.emulated_us as f64 / 1e6 / median,
            redraw.steps as f64 / median / 1e6,
        );
        measured.push((name, redraw, pinned));
    }
    for (name, redraw, pinned) in measured {
        assert_eq!(redraw, pinned, "{name}");
    }
}

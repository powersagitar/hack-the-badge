//! Boot-progress ratchet for the real firmware (`factory.bin`, shortcut
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
use emulator_core::runtime::FirmwareRuntime;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

mod common;
use common::provision::{self, Outcome};

fn factory() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/public/firmware/factory.bin"),
    )
    .expect("factory.bin")
}

/// Runs the real firmware in 250,000-step chunks, checking the console after
/// each, until `needle` appears in [`FirmwareRuntime::console_output`] or
/// `max_steps` is exhausted. Returns the runtime and whether it appeared.
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

/// The SYSTEM `FROM_CPU_0` software interrupt reaches the core: the first
/// trap of the whole boot is vPortYield's request (written on step 528,148),
/// taken on the next step as an interrupt on CPU line 4 (where the firmware
/// routes `ETS_FROM_CPU_INTR0_SOURCE`), and no exception follows through
/// step 571,712. The scheduler-start spin at `0x4200_0cd2` is gone.
#[test]
fn first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    let summary = rt.run(528_148);
    assert_eq!(summary.traps, 0, "got {summary:?}, pc=0x{:08x}", rt.pc());
    let summary = rt.run(1);
    assert_eq!(summary.traps, 1, "the yield must be taken on the next step");
    assert_eq!(rt.cpu().csr.mcause, 0x8000_0004);
    let summary = rt.run(571_712 - 528_149);
    assert_eq!(
        summary.last_instruction_fault, None,
        "no exception through step 571,712 (the pre-Task-D11 gpio_matrix_out fault was next); got {summary:?}"
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
    let (rt, ok) = boot_until_console_contains("I (0) main_task: Calling app_main()", 750_000);
    let console = rt.console_output();
    assert!(ok, "pc=0x{:08x}\nconsole:\n{console}", rt.pc());
    assert!(
        console.contains("I (0) main_task: Started on CPU0"),
        "console:\n{console}"
    );
}

/// ROM `gpio_matrix_out`/`gpio_matrix_in` are real stubs, so `app_main`'s
/// SPI bus setup runs: through step 596,000 (past the old fault on step
/// 571,713) no exception is taken (the last trap is an interrupt), no panic
/// text prints, and `main_task: Calling app_main()` is in the console.
#[test]
fn boot_no_longer_faults_or_panics_at_the_pre_task_d11_gpio_matrix_out_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
        console.contains("I (0) main_task: Calling app_main()"),
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

/// ROM `memcpy` is a real stub: zero traps through step 401,761, where the
/// unstubbed call used to fault.
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

/// ROM `ets_efuse_get_spiconfig` and the six ROM calls after it are real
/// stubs: zero traps through step 402,113, where the first used to fault.
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

/// `esprv_intc_int_enable` and its siblings write the interrupt-controller
/// registers for real: zero traps through step 405,806, where the unstubbed
/// call used to fault.
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

/// ROM `itoa`/`strcat` are real stubs: zero traps through step 407,471,
/// where the unstubbed `itoa` used to fault. (It was called by newlib's
/// `abort()`, not by normal boot; the abort's cause was fixed later.)
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

/// `cpu_start`'s app-image-header check passes (the header bytes before the
/// DROM segment are mapped, today through the flash MMU), so boot prints the
/// `app_init`/`efuse_init` block, ending at `efuse_init: Chip rev:` (step
/// ~408,000), and `Invalid app image header` never appears.
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

/// ROM `qsort` is guest-executed code (`emulator_core::rom::QSORT_BODY`):
/// after exactly 408,740 steps its `ret` has run and `pc` is back in its
/// caller, `s_prepare_reserved_regions()`, with zero traps. Step-exact:
/// it moves whenever anything before `qsort` changes.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d6_qsort_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    let summary = rt.run(408_740);
    assert_eq!(
        summary.traps, 0,
        "expected zero traps through ROM qsort's return (408,740 steps); got \
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
/// first line prints (step ~409,400) and no `memory_layout` error does.
#[test]
fn boot_reaches_heap_inits_first_line_past_the_reserved_region_check() {
    let (rt, ok) = boot_until_console_contains(
        "I (0) heap_init: Initializing. RAM available for dynamic allocation:",
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
/// ~415,000; in the physical badge's log too).
#[test]
fn boot_reaches_heap_inits_last_region_line_past_the_libgcc_helpers() {
    assert_reaches(
        "I (0) heap_init: At 50000020 len 00001FC8 (7 KiB): RTCRAM",
        500_000,
    );
}

/// `esp_rom_newlib_init_common_mutexes` and the libc calls after it are
/// real stubs: zero traps through step 440,000, past the old fault on step
/// 417,992.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d9_newlib_init_common_mutexes_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
        rt.console_output().contains("I (0) heap_init: At 50000020"),
        "heap_init's last region line should be present"
    );
}

/// The SPI1 flash controller answers JEDEC RDID with the badge's ID, so
/// flash-chip detection succeeds (`spi_flash: detected chip: generic`, step
/// ~446,000; in the physical badge's log too) instead of logging `memspi: no
/// response`.
#[test]
fn boot_reaches_spi_flash_detected_chip_generic() {
    let (rt, ok) = boot_until_console_contains("I (0) spi_flash: detected chip: generic", 500_000);
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
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
            .contains("I (0) sleep_gpio: Enable automatic switching of GPIO sleep configuration"),
        "the sleep_gpio lines after flash detection should be present"
    );
}

/// `ets_apb_backup_init_lock_func`, `esp_coex_rom_version_get` and
/// `esprv_intc_int_set_threshold` are real stubs: zero traps through step
/// 528,147, the step before FreeRTOS's first yield request.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d10_ets_apb_backup_init_lock_func_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    let summary = rt.run(528_147);
    assert_eq!(
        summary.traps,
        0,
        "boot must run past the old ets_apb_backup_init_lock_func fault \
         (step 493,861) with no traps; got {summary:?}, pc=0x{:08x}",
        rt.pc()
    );
}

/// The shortcut boot seeds the ROM's SPI-flash legacy data
/// (`g_rom_flashchip.chip_size` = 4 MiB, from factory.bin's header), so
/// flash init no longer warns that the image header says 0k (the physical
/// badge's log has no such warning). Checked once the `sleep_gpio` line is
/// out.
#[test]
fn boot_no_longer_warns_that_the_image_header_says_0k_of_flash() {
    let needle = "I (0) sleep_gpio: Enable automatic switching of GPIO sleep configuration";
    let (rt, ok) = boot_until_console_contains(needle, 500_000);
    let console = rt.console_output();
    assert!(
        ok,
        "never printed {needle:?}; pc=0x{:08x}\nconsole:\n{console}",
        rt.pc()
    );
    assert!(console.contains("I (0) spi_flash: flash io: dio"));
    assert!(!console.contains("Detected size"), "console:\n{console}");
}

/// `spi_ll_apply_config()`'s `while (hw->cmd.update);` poll in
/// `spi_hal_init()` (a Milestone 3 stall: tens of thousands of hits per
/// address).
const PRE_TASK_9_SPI_UPDATE_POLL: std::ops::RangeInclusive<u32> = 0x420f_d6fc..=0x420f_d702;

/// SPI2's `SPI_UPDATE` reads back 0 at once, so the `spi_hal_init()` poll
/// (first reached on step 583,989) runs but does not spin: over steps
/// 583,000..596,000 each poll address is hit fewer than [`SPIN_THRESHOLD`]
/// times and no exception is taken.
#[test]
fn boot_escapes_the_pre_task_9_spi_update_poll_into_spi_clock_setup() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
const SPI_LL_SET_COMMAND_BSWAP_RA: u32 = 0x4039_45fa;

/// ROM `__bswapsi2` is a real stub: over steps 590,000..596,608 it is
/// entered exactly once (step 593,018, from `spi_ll_set_command()`),
/// returns to its caller, and no exception is taken.
#[test]
fn boot_no_longer_faults_at_the_pre_task_d12_bswapsi2_call_site() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    rt.run(596_000);
    let console = rt.console_output();
    assert!(
        console.contains("I (0) main_task: Calling app_main()"),
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
/// - SYSTIMER time ran ahead of the step count by at least 1,500,000 ticks
///   (only the idle fast-forward can do that; measured 1,827,031 in
///   Milestone 3);
/// - counter 1, the FreeRTOS tick's, is past 40 tick periods of 160,000, so
///   the scheduler kept ticking after the first wake.
#[test]
fn boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
    let summary = rt.run(5_550_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt, not an exception; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    let systimer = &rt.bus().systimer;
    let ahead = systimer.elapsed_ticks() - rt.total_steps();
    assert!(
        ahead >= 1_500_000,
        "idle fast-forward added only {ahead} ticks"
    );
    assert!(
        systimer.counter(1) >= 40 * 160_000,
        "counter 1 = {}",
        systimer.counter(1)
    );
    let console = rt.console_output();
    assert!(
        console.contains("I (0) main_task: Calling app_main()"),
        "console:\n{console}"
    );
    for panic_text in ["Guru Meditation Error", "abort()", "Rebooting..."] {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}

/// GDMA's TX out-link feeds SPI2, so LVGL's flushes reach the ST7789 model:
/// the framebuffer is still blank at step 1,100,000 (first pixels land at
/// ~1,128,600) and by step 5,550,000 holds a drawn frame of at least 2,000
/// distinct colors (the splash has 2,340), sent through GDMA channel 0,
/// with no exception and no panic text. (The name is Milestone 3's; the
/// `MD5Init` stall it mentions is gone.)
#[test]
fn boot_draws_frames_through_gdma_before_the_md5init_stall() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
#[test]
fn load_partitions_accepts_the_synthesized_table_through_the_flash_mmu() {
    const ROM_MD5_INIT: u32 = 0x4000_0614;
    const ROM_MD5_UPDATE: u32 = 0x4000_0618;
    const ROM_MD5_FINAL: u32 = 0x4000_061c;
    const LOAD_PARTITIONS_EPILOGUE: u32 = 0x420f_a878;
    const ESP_OK: u32 = 0;

    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let summary = rt.run(5_000_000);
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
/// write does not drop `stdout` (`LVGL: Starting LVGL task`, step ~663,700).
#[test]
fn boot_prints_console_output_after_the_scheduler_starts() {
    assert_reaches("LVGL: Starting LVGL task", 1_000_000);
}

/// The emulated `storage` partition is blank, so `esp_littlefs` fails to
/// mount it and starts formatting it (step ~5,591,700), without a panic. An
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
/// blocks 0 and 1, at `0x2b_0000`/`0x2b_1000`, by step ~5,690,100) without a
/// fault, with no unmodeled SPIMEM1 command and no write left enabled.
#[test]
fn boot_formats_the_blank_storage_partition_with_littlefs() {
    const MAX: u64 = 8_000_000;
    let has_magic = |rt: &FirmwareRuntime, block: u32| {
        (0..8u32).all(|i| rt.bus().flash_chip.read(block + 8 + i) == LFS_MAGIC[i as usize])
    };
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
/// prints (step ~6.09M), then configures the sensor and starts its cache
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
/// starts (step ~6.14M; the physical badge prints this line too) and the
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
/// ~12.29M), and runs without a fault or panic text to step 20,000,000, by
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
/// ([`boots_to_first_real_frame`]).
const SPLASH_HASH: u64 = 0x5599_c270_ab04_29fa;

/// The sampling chunk of [`run_until_stable_frame`].
const FRAME_CHUNK: u32 = 250_000;

/// How long a frame must hold to count as stable at boot: a frame that
/// leaves the splash or draws in changes well within this.
const BOOT_STABLE_FOR: u64 = 1_000_000;

/// How long a frame must hold after a button press. After START the
/// firmware computes for about 6,000,000 steps without drawing before the
/// self-test frame lands, so a 1,000,000-step window would take the
/// unchanged first-run frame for the result.
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
/// frame (about step 1.13M) is not the finished splash; the framebuffer
/// changes many times before it settles (from step ~5.54M), so the test
/// pins the first frame that holds for [`BOOT_STABLE_FOR`] steps (found at
/// the 6,750,000-step sample). `MAX` is the stable step × 2, rounded up.
///
/// The hash is of this framebuffer's row/column order. MADCTL is not
/// modeled (the notes' "Known limitations"); the splash and the first-run
/// screen were compared by eye with the physical badge and match in
/// orientation, so a MADCTL model that keeps the image as the badge shows
/// it would leave the hash unchanged.
///
/// Inspect a failing frame with
/// `cargo run -p emulator-core --release --example boot-probe -- --steps 6600000 --dump-frame first-frame.png`
/// (writes under the gitignored `local/`).
#[test]
fn boots_to_first_real_frame() {
    const MAX: u64 = 11_100_000;
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("boot");
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
/// 15,500,000; that + 2,000,000, rounded up to a 250,000 multiple.
const FIRST_RUN_MAX_STEPS: u64 = 17_500_000;

/// Milestone 4's finish line: blank-flash boot reaches the first app's
/// screen and it holds. The emulated flash has no identity
/// (`/littlefs/identity.json`), so the app registry's first app, My Badge,
/// shows its unregistered first-run screen; on an unprovisioned badge it
/// keeps HOME for itself, so the app launcher is reached only after
/// provisioning ([`boots_to_launcher`]). Compared by eye with the physical
/// badge after a factory reset on 2026-10-07: content and orientation
/// match.
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
/// 100 ms of emulated time at SYSTIMER's 16 MHz (`TICKS_PER_STEP` = 1). The
/// old 100,000-step (6.25 ms) hold only registered because the undrained
/// console left the firmware idle; once the USB-Serial-JTAG ISR drains the
/// TX ring buffer, button polling falls outside such a short window
/// (Milestone 5 ruling R-T2-1).
const PRESS_HOLD_STEPS: u32 = 1_600_000;

/// [`first_run_screen_responds_to_start`]'s cap: the self-test frame is
/// first sampled at step 22,850,000 (with the 1.6M-step hold); that +
/// [`AFTER_PRESS_STABLE_FOR`] + 1,000,000, rounded up to a 250,000
/// multiple.
const SELF_TEST_MAX_STEPS: u64 = 32_000_000;

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
    let summary = rt.run(PRESS_HOLD_STEPS);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
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
/// 16,860,000 (10,000-step sampling); budget = that + 25%, rounded to
/// 1,000,000.
#[test]
fn boot_starts_the_console() {
    assert_reaches("hal_console: console started", 21_000_000);
}

/// [`console_answers_prov_show_on_the_first_run_screen`] and later
/// provisioning rungs: total-step deadline. Measured (Milestone 5 Task 3):
/// the prompt appears at 16.78M, `put` is READY at 17.24M, `prov apply`
/// answers by 17.33M, and the stable-frame check after it ends near
/// 18.4M; that + 25%, rounded to 1,000,000.
const CONSOLE_DEADLINE: u64 = 23_000_000;

/// [`put_accepts_a_payload_larger_than_the_rx_ring`]: total-step deadline.
/// Measured (Milestone 5 Task D-M5-1): finished at 17.88M (50,000-step
/// sampling, host-paced chunks); that + 25%, rounded to 1,000,000.
const RING_TEST_DEADLINE: u64 = 23_000_000;

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
            let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
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
// every role. Frame comparison with the physical badge (2026-10-08, the
// human partner's badge, role hacker): the registered screen, every
// onboarding page, the launcher and the launcher after DOWN match. The
// other roles' frames were not compared on hardware.
// ---------------------------------------------------------------------

/// Total-step deadline for provisioning from the first-run screen: the
/// slowest role (workshop_lead) prints `PROV OK` at 30.19M (all roles
/// 29.99M..30.19M, host-paced `put`); + 25%, rounded to 1,000,000.
const PROVISION_DEADLINE: u64 = 38_000_000;
/// How long after a press (or the shake) a response may take to start:
/// page 7 settles about 9M steps after A (a transition animation), so a
/// response's cap is [`AFTER_PRESS_STABLE_FOR`] plus this.
const ONBOARDING_RESPONSE_MARGIN: u64 = 16_000_000;
/// Raw slots from onboarding page 1 to page 6, each with the stable frame
/// it leads to (role hacker).
const TO_SHAKE_PAGE: &[(usize, u64)] = &[
    (1, 0xe788_a781_9328_1b08), // A: page 2 "This is you"
    (1, 0x9621_b8aa_8b04_0d38), // A: page 3 "Try every button"
    (1, 0xa283_f1b3_a862_abff), // A lit
    (2, 0x8522_bf8d_fccb_6084), // B
    (7, 0xb9e7_b94d_dd89_9859), // UP
    (5, 0x362f_6120_6bea_4a4f), // LEFT
    (6, 0x16eb_495b_b805_251a), // RIGHT
    (4, 0x8a7a_4dff_6454_46d3), // DOWN
    (8, 0xc6fb_a44d_8132_2795), // Aux1
    (3, 0x824a_fde3_f915_a562), // HOME (captured on page 3)
    (0, 0x3a72_5bc1_0083_f74a), // START: all nine done
    (1, 0xc1ad_004b_a5f7_7b78), // A: page 4 "Lights"
    (1, 0x7598_def3_0e83_8832), // A: page 5 "Tap to unlock"
    (1, 0x4a54_e5a4_0d66_9898), // A: page 6 "Shake it!", G-force 1000 mg
];
/// One half-period of the shake: 1,000,000 steps (62.5 ms of SYSTIMER
/// time). Six half-periods, alternating (+2 g, +2 g, +1 g) and (-2 g,
/// -2 g, +1 g), then rest at (0, 0, +1 g): the shake starts at 192.84M and
/// the first shaken frame appears at 194.34M (Task D-M5-2).
const SHAKE_HALF_STEPS: u32 = 1_000_000;
/// Page 6 after the shake, back at rest: G-force 1000 mg, Peak 3000 mg,
/// footer "A / START: next   B: back" (role hacker).
const SHAKEN_AT_REST_HASH: u64 = 0xa80a_279b_127e_1bb1;
/// Page 7 ("Badge Connect"), after A.
const ONBOARDING_PAGE7_HASH: u64 = 0x110e_8782_bc4e_5ff0;
/// Onboarding steps after the shake: (slot, expected stable hash for role
/// hacker); `None` is page 8 ("Bump to connect"), which is animated: it
/// cycles through about 14 frames (0.25M..2M steps each, never held for
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
/// How long the walk stays on the animated page 8 before pressing A: the
/// same 9M-step settle as every other measured page.
const ANIMATED_PAGE_STEPS: u32 = 9_000_000;
/// Setup's last press (START) to My Badge's registered screen: first
/// stable 4.3M..4.7M steps after the release, for every role.
const REGISTERED_RESPONSE_STEPS: u64 = AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
/// The role on the human partner's physical badge (Task 5).
const OWN_ROLE: &str = "hacker";
/// Raw slot from the registered screen to the launcher: HOME alone. The
/// launcher draws about 18M steps after the release (the console says
/// `launched Launcher` about 9M earlier, while the registered frame is
/// still on screen).
const TO_LAUNCHER_SLOT: usize = 3;
/// HOME's response: release to first stable launcher 18.0M..18.3M, then
/// [`AFTER_PRESS_STABLE_FOR`] more to confirm, 26.3M; + 25%, rounded up.
const LAUNCHER_RESPONSE_STEPS: u64 = 33_000_000;
/// The launcher: a 4x4 icon grid titled "My Badge", first icon selected.
/// The same for all 11 roles (measured).
const LAUNCHER_HASH: u64 = 0xb694_a61f_ebfd_62ff;
/// DOWN: moves the launcher's selection one row (My Badge to "Share").
const NAV_SLOT: usize = 4;
/// The launcher after one DOWN (first stable 5.5M steps after release).
const LAUNCHER_AFTER_NAV_HASH: u64 = 0xa2b9_da1f_0e36_757c;

/// (role, onboarding page 1 hash, registered My Badge screen hash), in
/// `ROLE_TABLE` order. Both screens show the fake identity's name (and the
/// role colour), so every role differs. Only hacker's were compared with
/// the physical badge.
const ROLE_ROWS: &[(&str, u64, u64)] = &[
    ("hacker", 0xd364_7661_7b1a_89b5, 0xbd6d_7613_08a7_bf24),
    ("organizer", 0x5dba_66e0_b019_1c24, 0x1974_c49f_a562_08a3),
    ("sponsor", 0x997d_dc94_a332_cb23, 0x0bc2_dee2_1a99_c4c0),
    ("judge", 0xa681_3acc_1906_5c12, 0x54f8_e3d0_65a3_04c1),
    ("mentor", 0x2619_ad73_bf0d_7e86, 0x03a9_7611_bf45_9332),
    ("volunteer", 0x05f0_cede_9341_fc3f, 0xfd95_a0e7_844f_4019),
    ("media", 0x49ee_edbe_2a64_a1d6, 0x5092_210f_e8d8_f1d2),
    ("staff", 0xc270_f7ca_eb37_28d3, 0xc68e_18b7_16ef_40ea),
    ("general", 0xa6a3_0959_d158_ab93, 0x4574_d9ed_d13c_330a),
    (
        "workshop_lead",
        0xf723_ec7a_d14b_c6e3,
        0xf8e2_3246_19a6_4d76,
    ),
    ("visitor", 0x6e32_6aa6_bcd6_c3f1, 0x647a_71d4_5ab8_cf23),
];

fn role_row(role: &str) -> (u64, u64) {
    let r = ROLE_ROWS.iter().find(|r| r.0 == role).expect("role row");
    (r.1, r.2)
}

/// Press and release `slot`, held [`PRESS_HOLD_STEPS`], asserting no fault.
fn press(rt: &mut FirmwareRuntime, slot: usize) {
    rt.set_raw_button(slot, true);
    let s = rt.run(PRESS_HOLD_STEPS);
    assert_eq!(s.last_instruction_fault, None, "{s:?}");
    rt.set_raw_button(slot, false);
}

/// [`press`], then the next stable frame other than `previous`.
fn press_and_settle(rt: &mut FirmwareRuntime, slot: usize, previous: u64) -> (u64, u64) {
    press(rt, slot);
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    run_until_stable_frame(rt, cap, AFTER_PRESS_STABLE_FOR, &[previous])
}

/// Shakes the badge (see [`SHAKE_HALF_STEPS`]) and puts it back at rest.
fn shake(rt: &mut FirmwareRuntime) {
    for k in 0..6 {
        let s = if k % 2 == 0 { 1 } else { -1 };
        rt.set_acceleration(s * 2000, s * 2000, 1000);
        let summary = rt.run(SHAKE_HALF_STEPS);
        assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
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
static REGISTERED: [OnceLock<FirmwareRuntime>; ROLE_ROWS.len()] = [const { OnceLock::new() }; ROLE_ROWS.len()];
/// Each role's launcher ([`walk_to_launcher`]), by [`ROLE_ROWS`] index.
static LAUNCHER: [OnceLock<FirmwareRuntime>; ROLE_ROWS.len()] = [const { OnceLock::new() }; ROLE_ROWS.len()];

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
/// with the physical badge on 2026-10-08 (role hacker).
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
/// as on the badge (compared 2026-10-08).
#[test]
fn launcher_responds_to_navigation() {
    let mut rt = launcher_for(OWN_ROLE);
    let (hash, step) = press_and_settle(&mut rt, NAV_SLOT, LAUNCHER_HASH);
    assert_eq!(hash, LAUNCHER_AFTER_NAV_HASH, "stable at step {step}");
    assert_no_panic_text(&rt);
}

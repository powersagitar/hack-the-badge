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
//!   ([`boots_to_first_real_frame`], [`boots_to_first_run_screen`]).
//!
//! The finish line is [`boots_to_first_run_screen`] plus
//! [`first_run_screen_responds_to_start`]. Step numbers in the docs below
//! are measured with today's emulator unless marked otherwise; a budget is a
//! margin, not a pin. The stall-by-stall history behind each rung is in
//! `docs/firmware-emulator-notes.md` ("History"); when boot moves, update
//! the rung whose budget it affects.
use emulator_core::runtime::FirmwareRuntime;
use std::collections::HashMap;
use std::path::PathBuf;

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
/// keeps HOME for itself, so the app launcher is not reachable from here
/// (the notes' "Current state"). Compared by eye with the physical badge
/// after a factory reset on 2026-10-07: content and orientation match.
#[test]
fn boots_to_first_run_screen() {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let (hash, step) = run_until_stable_frame(
        &mut rt,
        FIRST_RUN_MAX_STEPS,
        BOOT_STABLE_FOR,
        &[SPLASH_HASH],
    );
    assert_eq!(hash, FIRST_RUN_HASH, "stable at step {step}");
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
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let (hash, _) = run_until_stable_frame(
        &mut rt,
        FIRST_RUN_MAX_STEPS,
        BOOT_STABLE_FOR,
        &[SPLASH_HASH],
    );
    assert_eq!(hash, FIRST_RUN_HASH);

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
/// Measured (Milestone 5 Task D-M5-1): finished at 17.88M (50,000-step sampling, host-paced chunks); that + 25%, rounded
/// to 1,000,000.
const RING_TEST_DEADLINE: u64 = 23_000_000;

/// Boots to the stable first-run screen (blank flash).
fn first_run_screen() -> FirmwareRuntime {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let (hash, _) = run_until_stable_frame(
        &mut rt,
        FIRST_RUN_MAX_STEPS,
        BOOT_STABLE_FOR,
        &[SPLASH_HASH],
    );
    assert_eq!(hash, FIRST_RUN_HASH);
    rt
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

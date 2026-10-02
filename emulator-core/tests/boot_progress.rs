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
//! and `docs/firmware-emulator-notes.md`). `first_console_output_is_the_firmware_s_own_panic_report`
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
//! `first_console_output_is_the_firmware_s_own_panic_report`'s budget
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
//! retry. `first_console_output_is_the_firmware_s_own_panic_report`'s doc
//! is corrected below to describe this new cause (the assertion/budget
//! still held even before this correction, since both the old and new
//! causes print the same generic string). The still-unstubbed ROM
//! `software_reset_cpu` (`0x4000_0094`) then faults on the reboot attempt
//! exactly as before (step within the existing 650,000 budget -- see
//! `emulator-core/tests/rom_stub_boot.rs`'s renamed pinned-stall test), so
//! [`boot_reaches_the_panic_handlers_reboot_message_via_cpu_starts_abort`]
//! is renamed to
//! `boot_reaches_the_panic_handlers_reboot_message_via_the_reserved_region_overlap_abort`
//! below (renamed again in Task D7) -- same budget, corrected narrative.
//!
//! **Task D6 status**: ROM libc `qsort` (`0x4000_0434`) is now real
//! guest-executed RV32 code mapped into the ROM address space
//! (`emulator_core::rom::QSORT_BODY`; see `emulator-core/src/rom.rs`'s
//! module doc, entry 14), not an HLE stub, because it has to call its
//! firmware comparator. Step 1 of that task **refuted** the Task D5 guess
//! above: the caller is not `do_system_init_fn()` but ESP-IDF v5.5.3's
//! `s_prepare_reserved_regions()` (`components/heap/port/memory_layout_utils.c`),
//! which sorts 5 `soc_reserved_region_t` entries with
//! `s_compare_reserved_regions`. The sort now runs and returns (steps
//! 408,481 to 408,906). The very next thing the firmware does is its own
//! overlap check on the sorted array, which fails because entry 0 comes from
//! the ROM layout table through the unbacked ROM *data* pointer
//! `ets_rom_layout_p` (`0x3ff1fffc`). It reads `0`, so the region becomes
//! `0x00000000 - 0x3fce0000`. The firmware logs
//! `E (0) memory_layout: SOC_RESERVE_MEMORY_REGION region range 0x00000000 -
//! 0x3fce0000 overlaps with 0x3fc80000 - 0x3fc99c00` (step 408,970) and calls
//! `abort()`. `boot_reaches_memory_layouts_reserved_region_check_past_rom_qsort`
//! is the new rung for that line (retired in Task D7), and
//! [`boot_no_longer_faults_at_the_pre_task_d6_qsort_call_site`] pins the
//! fault-free run through `qsort`. The panic path that follows is an
//! `abort()` again, not a hardware exception. So "Guru Meditation Error"
//! now prints only after the failed `software_reset_cpu` reboot retry
//! (step ~648,960), and "Rebooting..." prints before it (step ~646,656).
//! The two panic-text rungs below keep their budgets, with corrected
//! narratives; one is renamed.
//!
//! **Task D9 status**: `esp_rom_newlib_init_common_mutexes` is a real stub
//! (`RomStubEffect::LoadStoreWords`, since Task D10 `StoreWords`) and ROM libc `strlen`/`memcmp`/
//! `strncmp`/`div` are real, so boot runs fault-free to step 442,140. No new
//! good console line appears: the next line is `E (0) memspi: no response`
//! (step 441,439), an *error* from the unmodeled SPI1 flash controller, not
//! progress. It is followed by an `assert failed` `abort()` (ILLEGAL_
//! INSTRUCTION on the 442,141st step). The progress rung is therefore a
//! no-trap-through-step assertion
//! ([`boot_no_longer_faults_at_the_pre_task_d9_newlib_init_common_mutexes_call_site`]).
//! "Rebooting..." now prints at step ~680,821 and "Guru Meditation Error"
//! at ~682,319 (after the `software_reset_cpu` fault on step 681,436); both
//! are panic-path lines, not progress.
//!
//! **Task D8 status**: libgcc `__clzsi2`/`__ffssi2` are real HLE stubs, so
//! `heap_init` prints all four `heap_init: At ...` lines (the last, `RTCRAM`,
//! at step ~415,621; a new rung, [`boot_reaches_heap_inits_last_region_line_past_the_libgcc_helpers`]).
//! On the 417,992nd step boot then faults on the unstubbed ROM
//! `esp_rom_newlib_init_common_mutexes` (`0x4000_0350`). "Guru Meditation
//! Error" prints at step ~419,414 and "Rebooting..." at ~658,042 (panic-path
//! lines, not progress); the reboot-retry fault follows on the 658,657th
//! step.
//!
//! **Task D7 status**: the ROM layout table is now backed
//! (`emulator_core::rom::ESP32C3_ROM_DATA`: the `ets_rom_layout_p` word at
//! `0x3ff1fffc` and the `ets_rom_layout_t` it points at, with
//! `dram0_rtos_reserved_start = 0x3fcdf060` from Espressif's published
//! ESP32-C3 rev3 ROM ELF; see `emulator-core/src/rom.rs`'s module doc,
//! entry 15). The reserved-region check passes, so its `E (0)
//! memory_layout: ...` line and `abort()` are gone, and the Task D6 rung
//! for that line is retired. `qsort` now returns at step 409,036 (its input
//! changed). Boot then prints ESP-IDF's normal `I (0) heap_init:
//! Initializing. RAM available for dynamic allocation:` (step ~409,660).
//! [`boot_reaches_heap_inits_first_line_past_the_reserved_region_check`]
//! is the new rung for it. On the 409,759th step boot faults on the
//! unstubbed libgcc `__clzsi2` (`0x4000_079c`). That fault is a hardware
//! exception, so "Guru Meditation Error" prints right away again (step
//! ~411,500), and "Rebooting..." follows at step ~649,700. Both panic-text
//! rungs keep their budgets, with corrected narratives; one is renamed.
//!
//! Task 8 status: the SPI1 flash controller answers JEDEC RDID, so flash-chip
//! detection succeeds. [`boot_reaches_spi_flash_detected_chip_generic`] is
//! the new rung. With ROM `memchr`/`memmove` also stubbed
//! ([`boot_no_longer_faults_at_the_pre_task_8_memchr_call_site`]), the panic
//! rung was re-pointed at the next fault, the unstubbed ROM
//! `ets_apb_backup_init_lock_func` (step 493,861), and renamed.
//!
//! **Task D10 status**: `ets_apb_backup_init_lock_func`,
//! `esp_coex_rom_version_get` and `esprv_intc_int_set_threshold` are real
//! stubs, and the ROM's SPI-flash legacy data is seeded at boot. Boot now
//! runs with **zero traps**: FreeRTOS starts its scheduler, and the first
//! context switch is requested through the unmodeled SYSTEM cross-core
//! software interrupt (step 528,777), so `vTaskStartScheduler()` returns and
//! the CPU spins on a `j .` (see `tests/rom_stub_boot.rs`'s pinned stall).
//! No new good console line appears (the real badge's next line,
//! `main_task: Started on CPU0`, is printed by the first task), and the
//! `(0k)` flash-size warning is gone. So the rungs are:
//! [`boot_no_longer_faults_at_the_pre_task_d10_ets_apb_backup_init_lock_func_call_site`]
//! (no trap through step 528,776),
//! [`boot_no_longer_warns_that_the_image_header_says_0k_of_flash`], and
//! `boot_no_longer_reaches_the_panic_handler`, which replaced the two
//! panic-text rungs (Guru Meditation / Rebooting...), since there was no
//! panic left to reach.
//!
//! **Task 4 status**: the SYSTEM `FROM_CPU_0..3` software interrupts are
//! modeled and the interrupt matrix is source-indexed with enable and
//! priority/threshold gating (legacy-INTC rule: a line fires iff its
//! priority `>=` the threshold). vPortYield's request (step 528,777) is
//! taken as an interrupt on CPU line 4 on the next step, the first context
//! switch happens, and `main_task` runs: `main_task: Started on CPU0` and
//! `main_task: Calling app_main()` print. Inside `app_main` boot then faults
//! on the unstubbed ROM `gpio_matrix_out` (step 571,713), so the panic
//! handler runs again (see `tests/rom_stub_boot.rs`'s pinned stall). The
//! no-panic rung is therefore retired (per its own doc, it was never to be
//! re-pointed at a panic). The new rungs are
//! [`first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line`] and
//! [`boot_reaches_main_task_calling_app_main`].
//!
//! **Task D11 status**: ROM `gpio_matrix_out`/`gpio_matrix_in` are real
//! stubs writing the GPIO matrix registers, so `app_main`'s SPI bus setup
//! (`spicommon_bus_initialize_io()`) runs through and the panic is gone
//! again. Boot then spins, with no exception, in `spi_hal_init()`'s
//! `spi_ll_apply_config()` poll on SPI2's `SPI_UPDATE` bit (step 584,618
//! on; see `tests/rom_stub_boot.rs`'s pinned stall). No new console line
//! appears (`main_task: Calling app_main()` is still the newest), so, as in
//! Tasks D2 and D10, the rung is a no-fault one:
//! [`boot_no_longer_faults_or_panics_at_the_pre_task_d11_gpio_matrix_out_call_site`].
//!
//! **Task 9 status**: SPI2's `SPI_UPDATE` reads back 0 at once and
//! transactions signal `SPI_TRANS_DONE_INT_RAW`, so the `spi_hal_init()`
//! poll exits on its first check and boot prints a new console line from
//! the SPI clock setup, `W (0) clk_hal: invalid RTC_XTAL_FREQ_REG value,
//! assume 40MHz` (an emulator artifact: the real badge's log has no such
//! line). It then faults on the 602,868th step on the unstubbed ROM
//! `__bswapsi2` (`0x4000_0788`), called from `spi_ll_set_command()`, and
//! the panic handler runs (see `tests/rom_stub_boot.rs`'s pinned stall).
//! The D11 rung's budget drops from 1,500,000 to 602,000 steps (still well
//! past its own old fault, step 571,713), and the new rung is a
//! hot-PC-escape one:
//! [`boot_escapes_the_pre_task_9_spi_update_poll_into_spi_clock_setup`].
//!
//! **Task D12 status**: ROM `__bswapsi2` is a real stub, and the shortcut
//! boot seeds `RTC_XTAL_FREQ_REG` as the skipped bootloader would. Both
//! (emulator-only) XTAL warnings are gone: the `clk_hal` one above and 17
//! `rtc_clk` ones in early boot. Not printing those makes the timeline
//! earlier (296 steps at `qsort`'s return, 629 at the first yield), so the
//! step-exact rungs above carry "Task D12 update" notes. Boot then runs
//! with no exception through the `__bswapsi2` call (step 593,018) and one
//! SPI2 transaction, until the FreeRTOS IDLE task's `wfi` (in
//! `esp_cpu_wait_for_intr()`) traps as an illegal instruction on step
//! 596,609, because the core does not implement `wfi` yet (plan Task 6).
//! The panic handler runs (see `tests/rom_stub_boot.rs`'s pinned stall). No
//! new console line appears (`main_task: Calling app_main()` is still the
//! newest), so the rung is a no-fault one:
//! [`boot_no_longer_faults_at_the_pre_task_d12_bswapsi2_call_site`], plus
//! the XTAL-warning ratchet
//! [`boot_no_longer_warns_that_rtc_xtal_freq_reg_is_invalid`]. The Task 9
//! rung no longer asserts the `clk_hal` line.
//!
//! **Task 6 status**: `wfi` is a real instruction, and while the core waits
//! with nothing asserted the driving loop fast-forwards SYSTIMER to its next
//! alarm. The idle task's `wfi` on step 596,609 now retires, the FreeRTOS
//! tick wakes it on the next step, and FreeRTOS runs on: 42 ticks (12 of
//! them reached by fast-forward) while `app_main` renders with LVGL and
//! flushes frames over SPI2 with DMA (GDMA is not modeled, so nothing is
//! drawn). No new console line prints. The next exception is on step
//! 5,555,258: `load_partitions()` calls the unstubbed ROM `MD5Init`
//! (`0x4000_0614`), and the panic handler runs (see
//! `tests/rom_stub_boot.rs`'s pinned stall). The rungs above keep their
//! budgets (each is still short of the new fault); the new rung is a
//! no-fault one that proves the idle fast-forward happened:
//! [`boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting`].
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

/// Milestone 3 Task 4's proven-state-change rung. Task D10's
/// `boot_no_longer_reaches_the_panic_handler` (zero traps and no panic text
/// over 1,500,000 steps) is retired here: its own doc said to change it
/// only when a later stall is a real fault again, and never to re-point it
/// at a panic, and as of Task 4 the stall is a real fault again (see the
/// module doc's "Task 4 status"). What Task 4 proves instead is that the
/// **first trap of the whole boot is the `FROM_CPU_0` yield interrupt on
/// its routed line**: zero traps through vPortYield's write (step 528,777),
/// then exactly one trap on the next step, with `mcause` = interrupt | line
/// 4 (the line the firmware routes `ETS_FROM_CPU_INTR0_SOURCE` to). And the
/// scheduler-start spin at `0x4200_0cd2` is gone: up to the step before the
/// next fault (571,712) no exception is taken.
///
/// **Task D12 update**: with `RTC_XTAL_FREQ_REG` seeded, early boot no
/// longer prints its (emulator-only) `rtc_clk` XTAL warnings, so the whole
/// timeline is 629 steps earlier here: the yield request is on step
/// 528,148 and taken on step 528,149. The fault-free run still ends at step
/// 571,712 (the old gpio_matrix_out step); the next exception was then the
/// idle task's `wfi` on step 596,609 (Task 6 made `wfi` real; the next
/// exception is now ROM `MD5Init` on step 5,555,258).
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

/// Milestone 3 Task 4's console rung: the newest non-panic line. With the
/// first context switch working, FreeRTOS runs `main_task`, which prints
/// `I (0) main_task: Started on CPU0` (step ~536,400) and then
/// `I (0) main_task: Calling app_main()` (step ~555,650), both lines the
/// real badge's boot log also has. [`boot_until_console_contains`] sees it at
/// its 750,000-step check.
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

/// Milestone 3 Task D11's no-new-console-line fallback rung (see the module
/// doc's "Task D11 status"). Before it, `app_main` faulted on the unstubbed
/// ROM `gpio_matrix_out` on step 571,713 and the panic handler ran. Both
/// GPIO-matrix ROM calls are now real stubs, so a run past that step must
/// show **no exception** (interrupts are fine: the FreeRTOS yield is one)
/// and no panic text, and still have `main_task: Calling app_main()`.
/// Task D11 ran it to 1,500,000 steps (past where the old panic printed
/// "Rebooting...", ~811,450), because boot then spun without faulting;
/// Task 9 ended that spin and boot then faulted on step 602,868 (ROM
/// `__bswapsi2`), so the budget became 602,000 steps. Task D12 stubbed that
/// call and made the timeline earlier; the next exception is the idle
/// task's `wfi` on step 596,609 (see `tests/rom_stub_boot.rs`), so the
/// budget is now 596,000 steps: still ~24,000 steps past the old fault.
/// (Task 6 made `wfi` real; the next exception is now ROM `MD5Init` on step
/// 5,555,258, covered by
/// [`boot_idles_in_wfi_and_fast_forwards_to_the_freertos_tick_without_faulting`],
/// so this rung keeps its budget.)
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

/// Milestone 3 Task D6's no-fault rung. Before that task, boot faulted on
/// the unstubbed ROM `qsort` at step 408,481. `qsort` is now guest-executed
/// ROM code (`emulator_core::rom::QSORT_BODY`), and after exactly 409,036
/// steps its `ret` has executed and `pc` is back at the caller. That run
/// must show zero traps.
///
/// **Task D7 update**: the step count was 408,906 in Task D6. Backing the
/// ROM layout table changed `qsort`'s input: entry 0's `start` is now
/// `0x3fcdf060` rather than `0`, so it sorts last instead of first, and the
/// insertion sort does more work. The first trap after this point is now
/// the `__clzsi2` fault on the 409,759th step (see the module doc's "Task
/// D7 status").
///
/// **Task D12 update**: 408,740 steps. Seeding `RTC_XTAL_FREQ_REG` removed
/// the early (emulator-only) `rtc_clk` XTAL warnings, so `qsort` returns
/// 296 steps sooner.
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

/// Milestone 3 Task D7's ratchet rung: the newest console line boot
/// reaches, and this time it is **normal boot progress**, not an error:
/// ESP-IDF's own `heap_init` banner (generic text, also the first
/// `heap_init` line of the real badge's serial boot log). It prints only
/// after `soc_get_available_memory_regions()` has returned, which means
/// `s_prepare_reserved_regions()`'s overlap check passed with the real ROM
/// layout value. That check aborted in Task D6, and the rung for its error
/// line (`boot_reaches_memory_layouts_reserved_region_check_past_rom_qsort`)
/// is retired here because the line never prints any more. This test also
/// asserts it stays gone. Measured at step ~409,660. The 420,000 budget
/// matches [`boot_reaches_efuse_inits_chip_rev_line`]'s. (In practice
/// [`boot_until_console_contains`] checks every 250,000 steps, so it sees
/// the line at its 500,000-step check.)
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

/// Milestone 3 Task D8's ratchet rung: the newest *boot-progress* console
/// line boot reaches. With libgcc `__clzsi2` (TLSF's `fls()`) and `__ffssi2`
/// (`ffs()`) backed, `heap_init` walks its whole region list and prints the
/// last of its four `heap_init: At ...` lines (the `RTCRAM` region), which
/// the real badge's boot log also has. Measured at step ~415,621. The
/// 500,000 budget is honest given [`boot_until_console_contains`]'s
/// 250,000-step chunking (it sees the line at its 500,000-step check;
/// ~84,000 steps of real margin). The panic that followed at Task D8
/// (the unstubbed ROM `esp_rom_newlib_init_common_mutexes`) is gone in Task
/// D9; see [`boot_no_longer_faults_at_the_pre_task_d9_newlib_init_common_mutexes_call_site`].
#[test]
fn boot_reaches_heap_inits_last_region_line_past_the_libgcc_helpers() {
    assert_reaches(
        "I (0) heap_init: At 50000020 len 00001FC8 (7 KiB): RTCRAM",
        500_000,
    );
}

/// Milestone 3 Task D9's ratchet rung (the no-new-progress-line fallback,
/// see the module doc). Before this task boot faulted on the unstubbed ROM
/// `esp_rom_newlib_init_common_mutexes` at step 417,992. With it and the
/// libc `strlen`/`memcmp`/`strncmp`/`div` calls after it backed, the first
/// trap of any kind is the `abort()` at step 442,141, so a run of 440,000
/// steps -- past the old fault by 22,008 steps -- must show **zero** traps.
/// (The stall after it, `E (0) memspi: no response`, is the unmodeled SPI1
/// flash controller: an error, not progress.)
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

/// Milestone 3 Task 8's ratchet rung (the plan's `spi_flash: detected chip:
/// generic`, a line the real badge's boot log also has). The SPI1 flash
/// controller (`emulator_core::peripherals::flash::Spimem1`) now answers the
/// JEDEC RDID command with the badge's ID, so ESP-IDF's flash-chip
/// detection succeeds instead of logging `E (0) memspi: no response` and
/// aborting. Measured at step ~446,991; [`boot_until_console_contains`]
/// sees it at its 500,000-step check (~53,000 steps of real margin).
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

/// Milestone 3 Task 8's no-fault rung for the two atomic ROM libc calls after
/// flash-chip detection: before they were stubbed, boot faulted on ROM
/// `memchr` on step 490,128 (then `memmove` on step 490,143). With both
/// real, the first trap of any kind was the `ets_apb_backup_init_lock_func`
/// fault on step 493,861 (none at all since Task D10), so a run of 493,000
/// steps must show **zero** traps.
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

/// Milestone 3 Task D10's no-fault rung. Before it, boot faulted on the
/// unstubbed ROM `ets_apb_backup_init_lock_func` on step 493,861; the next
/// two ROM calls, `esp_coex_rom_version_get` and
/// `esprv_intc_int_set_threshold`, were stubbed in the same task. Boot now
/// runs fault-free until FreeRTOS's first yield request (step 528,777;
/// since Task 4 taken as an interrupt on the next step), so a run to step
/// 528,776 must show **zero** traps. (Task D12: the yield request is now
/// on step 528,148, so the run is to step 528,147.)
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

/// Milestone 3 Task D10, item B's ratchet. With the ROM's SPI-flash legacy
/// data seeded at boot (`g_rom_flashchip.chip_size` = 4 MiB, from
/// factory.bin's own header), ESP-IDF's flash init no longer prints `W (0)
/// spi_flash: Detected size(4096k) larger than the size in the binary image
/// header(0k). Using the size in the binary image header.` -- a line the
/// real badge's boot log does not have either. Checked once the last
/// `sleep_gpio:` line is out (measured at step ~484,416;
/// [`boot_until_console_contains`] sees it at its 500,000-step check).
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
/// `spi_hal_init()` (Task D11's stall: every address hit tens of thousands
/// of times from step 584,618 on).
const PRE_TASK_9_SPI_UPDATE_POLL: std::ops::RangeInclusive<u32> = 0x420f_d6fc..=0x420f_d702;

/// Milestone 3 Task 9's rung (see the module doc's "Task 9 status"). With
/// SPI2's `SPI_UPDATE` reading back 0 at once, the `spi_hal_init()` poll is
/// no longer a spin: over a trace window from just before the poll to just
/// before the next exception, each poll address is hit far fewer than
/// [`SPIN_THRESHOLD`] times and no exception is taken.
///
/// **Task D12 update**: Task 9 also asserted the next console line, the
/// SPI clock setup's (emulator-only) `clk_hal` XTAL warning. Task D12 seeds
/// `RTC_XTAL_FREQ_REG`, so that line is gone (see
/// [`boot_no_longer_warns_that_rtc_xtal_freq_reg_is_invalid`]), and this
/// rung now checks only the poll escape. The window moves with the earlier
/// timeline: the poll is first reached on step 583,989 (was 584,618), and
/// the next exception is the idle task's `wfi` on step 596,609, so the
/// window is steps 583,000..596,000 (was 584,000..602,000). (Task 6: that
/// `wfi` no longer faults; the window is unchanged.)
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

/// Milestone 3 Task D12's no-fault rung (see the module doc's "Task D12
/// status"). Before it, `spi_ll_set_command()` faulted on the unstubbed
/// ROM `__bswapsi2` (Task 9's stall). Now that call is a real stub: over a
/// run to the step before the then-next exception (the idle `wfi`, 596,609;
/// no longer an exception since Task 6), the ROM address is
/// entered exactly once (on step 593,018), execution comes back to its
/// caller, and no exception is taken (the last trap is an interrupt).
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

/// Milestone 3 Task D12, part B's ratchet. Seeding `RTC_XTAL_FREQ_REG`
/// (`RTC_CNTL_STORE4_REG`) with the value the skipped 2nd-stage bootloader
/// stores (`emulator-core/src/rom.rs`'s module doc, entry 23) makes ESP-IDF's
/// `clk_ll_xtal_load_freq_mhz()` find a valid 40 MHz. So neither of its
/// "invalid RTC_XTAL_FREQ_REG value" warnings prints any more: not
/// `rtc_clk_xtal_freq_get()`'s (`rtc_clk` tag, 17 times in early boot) and
/// not `clk_hal_xtal_get_freq_mhz()`'s (`clk_hal` tag, from the SPI clock
/// setup). The real badge's log has neither. Checked over a run to step
/// 596,000, past `main_task: Calling app_main()` and the SPI2 setup.
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

/// Milestone 3 Task 6's no-fault rung (see the module doc's "Task 6
/// status"). Before it, the FreeRTOS idle task's `wfi` on step 596,609 was
/// an illegal instruction and the panic handler ran. Now it waits, and the
/// driving loop jumps SYSTIMER to the next alarm while it does. Over a run
/// to short of the next exception (step 5,555,258, ROM `MD5Init`; the
/// budget was 5,555,000 steps, now 5,550,000 for a wider margin):
/// - no exception is taken (the last trap is an interrupt) and no panic
///   text prints;
/// - SYSTIMER time ran **ahead** of the step count, which only the idle
///   fast-forward can do: measured (at the old 5,555,000-step budget)
///   1,827,031 extra ticks over 12 waits
///   (asserted `>= 1_500_000`, about 9.4 tick periods, so at least ~10 of
///   the waits must have jumped most of a period);
/// - counter 1, the FreeRTOS tick's counter, is past 40 tick periods of
///   160,000 (measured: 42 periods, 6,853,983 ticks), so the scheduler kept
///   ticking long after the first wake.
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

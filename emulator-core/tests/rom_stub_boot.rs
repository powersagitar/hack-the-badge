//! Integration test: boot the *real* dumped firmware
//! (`frontend/public/firmware/factory.bin`) with the ESP32-C3 mask-ROM
//! high-level-emulation stub table installed, and record exactly how far it
//! gets.
//!
//! This is the counterpart to `boot_integration.rs`, which deliberately boots
//! *without* stubs and pins down the `INSTRUCTION_ACCESS_FAULT` the mask-ROM
//! call produces. Here the stubs are in play, so the question is no longer
//! "does it fault?" but "how far past the ROM wall does it get, and what
//! stops it next?".
//!
//! See the Task 6 report for the narrative; the `eprintln!` trace below is
//! the evidence it's built from.

use emulator_core::boot::{boot_from_factory_image_with_rom_stubs, step_with_interrupts};
use emulator_core::cpu::exception_code;
use emulator_core::runtime::FirmwareRuntime;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn factory_bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("frontend/public/firmware/factory.bin")
}

fn read_factory_bin() -> Vec<u8> {
    std::fs::read(factory_bin_path()).expect(
        "reading frontend/public/firmware/factory.bin \
         (expected to be committed at the repo's frontend/public/firmware/)",
    )
}

#[test]
fn rom_stubbed_boot_gets_past_the_mask_rom_wall() {
    let image = read_factory_bin();
    let (mut cpu, mut bus) =
        boot_from_factory_image_with_rom_stubs(&image).expect("real factory.bin should boot");

    // Milestone 3 Task D1: reduced from 2,000,000. Modeling RTC_CNTL's RTC
    // timer (`crate::peripherals::rtc_cntl`) let boot's delay loop actually
    // terminate instead of spinning forever, and boot now runs past this
    // budget into a *new*, later stall (see this file's renamed
    // `boot_currently_aborts_on_memspi_no_response_and_reaches_the_panic_handlers_reboot_message`
    // below, which pins the current one). 350,000 keeps this test's
    // original claim -- "gets past the mask ROM wall with zero faults, and
    // reaches every one of the named early-boot ROM calls below" -- true.
    // Task D2 (`RomStubEffect::Memcpy`) moved the fault this budget stays
    // clear of from step 401,761 (an unstubbed `memcpy`) to step 402,113
    // (an unstubbed `ets_efuse_get_spiconfig`); Task D3 (entry 10) moved it
    // further still, to step 405,806 (an unstubbed `esprv_intc_int_enable`);
    // Fix round 1 (entry 11) moved it further again, to step 407,471 (an
    // unstubbed `itoa`); Task D4 (entry 12 -- real `itoa`/`strcat`) moved it
    // further still, to step 407,549 -- a real `ILLEGAL_INSTRUCTION` trap,
    // not an unstubbed-ROM-call fault, at ESP-IDF's own `panic_abort()`
    // (reached from newlib's `abort()`, called because `cpu_start` rejected
    // this image's header -- see entry 12's corrected narrative; this is
    // *not* evidence boot is proceeding normally, only that this budget's
    // "zero faults" claim needs a fresh margin check against the new fault
    // step) -- comfortably clear either way (~57,549-step/~16% margin),
    // without this test needing to also pin the new, later stall (that's
    // the other test's job).
    //
    // Task D5 status: `crate::mem::bus::FirmwareBus::from_segments`'s
    // page-granular XIP mapping fixed `cpu_start`'s header check for real
    // (see `emulator-core/tests/boot_progress.rs`'s module doc's "Task D5
    // status"), so `abort()` is never reached any more -- boot's first
    // fault is now a genuinely different one, an unstubbed ROM `qsort` call
    // at step 408,481 (see this file's renamed test above). 350,000 remains
    // comfortably clear (~58,481-step/~17% margin, essentially unchanged
    // from Task D4's).
    //
    // Task D6 status: ROM `qsort` is now guest-executed code (see
    // `emulator_core::rom`'s module doc, entry 14), so step 408,481 no
    // longer faults; boot's first trap is now `panic_abort()`'s
    // ILLEGAL_INSTRUCTION at step 409,071, from an `abort()` over the
    // unbacked ROM layout table (see the pinned-stall test below). 350,000
    // stays clear by ~59,071 steps (~17%).
    //
    // Task D7 status: the ROM layout table is now backed (`emulator_core::
    // rom`'s module doc, entry 15), so that `abort()` is gone; boot's first
    // trap is now the unstubbed libgcc `__clzsi2` fault at step 409,759
    // (see the pinned-stall test below). 350,000 stays clear by ~59,759
    // steps (~17%).
    //
    // Task D8 status: `__clzsi2`/`__ffssi2` are now real stubs; boot's first
    // trap is now the unstubbed ROM `esp_rom_newlib_init_common_mutexes`
    // fault at step 417,992 (see the pinned-stall test below). 350,000 stays
    // clear by ~67,992 steps (~19%).
    //
    // Task D9 status: `esp_rom_newlib_init_common_mutexes` and the libc
    // `strlen`/`memcmp`/`strncmp`/`div` calls after it are real stubs; boot's
    // first trap is now an `abort()`'s ILLEGAL_INSTRUCTION at step 442,141
    // (after `E (0) memspi: no response`; see the pinned-stall test below).
    // 350,000 stays clear by ~92,141 steps (~26%).
    //
    // Task D12 status: raised from 350,000 to 420,000. With
    // `RTC_XTAL_FREQ_REG` seeded (`emulator_core::rom`'s module doc, entry
    // 23), `rtc_clk_xtal_freq_get()` no longer logs its (emulator-only)
    // "invalid" warning during early clock init. That warning was what
    // reached `ets_get_cpu_frequency` (the log timestamp) and `ets_printf`
    // first, at steps ~557/~576. Now both are first reached by the first
    // real log line, at steps 400,695/400,714, so 350,000 no longer covers
    // them. 420,000 does, by ~19,000 steps, and stays clear of boot's first
    // trap (the FROM_CPU_0 yield interrupt at step 528,149) by ~108,000
    // steps (~26%).
    const STEP_BUDGET: usize = 420_000;

    // The ordered list of distinct ROM stubs hit (first-hit order), plus a
    // per-address hit count -- together these say what the firmware asked the
    // ROM to do, and whether it got stuck asking repeatedly.
    let mut rom_call_order: Vec<u32> = Vec::new();
    let mut rom_call_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut first_fetch_fault: Option<(usize, u32, u32)> = None; // (step, mepc, mtval)
    let mut fetch_fault_addrs: BTreeMap<u32, usize> = BTreeMap::new();
    let mut trap_count = 0usize;
    let mut steps_run = 0usize;

    for i in 0..STEP_BUDGET {
        let info = step_with_interrupts(&mut cpu, &mut bus);
        steps_run = i + 1;
        if let Some(addr) = info.rom_stub {
            let count = rom_call_counts.entry(addr).or_insert(0);
            if *count == 0 {
                rom_call_order.push(addr);
            }
            *count += 1;
        }
        if info.trap_taken {
            trap_count += 1;
            if cpu.csr.mcause == exception_code::INSTRUCTION_ACCESS_FAULT {
                *fetch_fault_addrs.entry(cpu.csr.mtval).or_insert(0) += 1;
                if first_fetch_fault.is_none() {
                    first_fetch_fault = Some((i, cpu.csr.mepc, cpu.csr.mtval));
                }
            }
        }
    }

    let stubs = cpu.rom_stubs();
    let name_of = |addr: u32| -> String {
        stubs
            .lookup(addr)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| "<not stubbed>".to_string())
    };

    eprintln!("=== rom_stub_boot: {steps_run} steps ===");
    eprintln!(
        "pc = 0x{:08x}, sp (x2) = 0x{:08x}, traps = {trap_count}, \
         unmapped MMIO accesses logged = {}",
        cpu.regs.pc,
        cpu.regs.read(2),
        bus.unmapped_log().len(),
    );
    eprintln!("ROM stubs hit, in first-hit order:");
    for addr in &rom_call_order {
        eprintln!(
            "  0x{addr:08x}  {:<34} x{}",
            name_of(*addr),
            rom_call_counts[addr]
        );
    }
    if fetch_fault_addrs.is_empty() {
        eprintln!("instruction-access faults: none");
    } else {
        eprintln!(
            "first instruction-access fault (step, mepc, mtval) = {:?}",
            first_fetch_fault
        );
        eprintln!("all faulting fetch addresses:");
        for (addr, count) in &fetch_fault_addrs {
            eprintln!("  0x{addr:08x}  x{count}");
        }
    }

    // ---- Assertions (the trace above is evidence a human reads) ----

    // 1. The first mask-ROM call is intercepted rather than faulting -- the
    //    specific wall this task exists to get past.
    assert!(
        rom_call_counts.contains_key(&emulator_core::rom::RTC_GET_RESET_REASON),
        "boot should have called rtc_get_reset_reason (the first ROM call real \
         firmware makes) through the HLE stub"
    );

    // 2. NO unmapped instruction fetch happens at all any more. Without stubs
    //    the very same image faults ~20 instructions in (see
    //    `boot_integration.rs`, which pins that down deliberately); with them,
    //    two million instructions execute without the PC ever leaving mapped
    //    space. Any regression here -- a missing stub for a newly-reached ROM
    //    function, or a stub redirecting `pc` somewhere wrong -- shows up as a
    //    named address in the trace above.
    assert_eq!(
        first_fetch_fault, None,
        "no instruction-access fault expected; got {first_fetch_fault:?} \
         (look up the mtval in ESP-IDF's esp32c3.rom*.ld and add a stub -- see \
         emulator_core::rom)"
    );
    assert_eq!(trap_count, 0, "no traps of any kind expected");

    // 3. Boot reaches, specifically, SoC clock init -- far enough to have
    //    cleared .bss, enabled the flash cache/MMU, poked the analog regi2c
    //    bus, read the CPU frequency and emitted its first log line. Each of
    //    these is a distinct, recognizable ESP-IDF startup milestone, so
    //    asserting the set is a real progress check and not just "it ran".
    for (addr, what) in [
        (emulator_core::rom::MEMSET, "clearing .bss via ROM memset"),
        (0x4000_0520, "Cache_Enable_ICache (flash cache bring-up)"),
        (0x4000_0548, "Cache_Set_IDROM_MMU_Size (flash MMU bring-up)"),
        (0x4000_1960, "rom_i2c_writeReg_Mask (analog/PLL config)"),
        (
            emulator_core::rom::ETS_GET_CPU_FREQUENCY,
            "ets_get_cpu_frequency (clock init)",
        ),
        (
            emulator_core::rom::ETS_PRINTF,
            "ets_printf (first log line)",
        ),
        (0x4000_08ac, "__udivdi3 (64-bit time arithmetic)"),
    ] {
        assert!(
            rom_call_counts.contains_key(&addr),
            "expected boot to reach {what} (ROM 0x{addr:08x})"
        );
    }
}

/// This test **pins today's panic path, not boot progress past it**.
/// **Renamed and re-pointed in Task D12** (from
/// `boot_currently_faults_on_the_unstubbed_rom_bswapsi2_in_spi_ll_set_command`,
/// Task 9's name; it was also renamed between Task 8 and Task 9). Earlier,
/// **renamed and re-pointed in Task 8** (from
/// `boot_currently_aborts_on_memspi_no_response_and_reaches_the_panic_handlers_reboot_message`,
/// then briefly `boot_currently_faults_on_the_unstubbed_memchr_call_and_reaches_the_panic_handlers_reboot_message`),
/// previously renamed and re-pointed in Task D7 (from
/// `boot_currently_aborts_on_the_unbacked_rom_layout_reserved_region_overlap_and_reaches_the_panic_handlers_reboot_message`,
/// itself renamed in Task D6 from
/// `boot_currently_faults_on_the_unstubbed_qsort_call_and_reaches_the_panic_handlers_reboot_message`,
/// renamed in Task D5 from
/// `boot_currently_aborts_reaching_the_panic_handlers_reboot_message`).
///
/// **What changed in Task D7**: the ROM *data* pointer `ets_rom_layout_p`
/// (`0x3ff1_fffc`) and the `ets_rom_layout_t` table it points at are now
/// backed (`emulator_core::rom::ESP32C3_ROM_DATA`, see
/// `emulator-core/src/rom.rs`'s module doc, entry 15), so
/// `s_prepare_reserved_regions()`'s entry 0 is the real
/// `{0x3fcdf060, 0x3fce0000}` instead of `{0, 0x3fce0000}`. The overlap
/// check passes: the `E (0) memory_layout: ...` line and its `abort()`
/// (Task D6's pinned stall) are gone. ROM `qsort` now returns at step
/// 409,036 (later than D6's 408,906 because the input differs: entry 0 now
/// sorts last, not first).
///
/// **What changed in Task D8**: libgcc `__clzsi2` (`0x4000_079c`) and
/// `__ffssi2` (`0x4000_07d4`) are now real HLE stubs
/// (`RomStubEffect::Int32Unary`), so heap_init runs to the end of its region
/// list and prints all four `heap_init: At ...` lines (the last, `RTCRAM`,
/// at step ~415,621).
///
/// **What changed in Task D9**: `esp_rom_newlib_init_common_mutexes`
/// (`0x4000_0350`) now really copies `*a0`/`*a1` to the ROM statics
/// `0x3fcd_f660`/`0x3fcd_f65c` (`RomStubEffect::LoadStoreWords`, since Task D10 `StoreWords`), and the
/// atomic ROM libc calls `esp_newlib_init`'s successors make -- `strlen`
/// (`0x4000_0374`), `memcmp` (`0x4000_0360`), `strncmp` (`0x4000_0370`),
/// `div` (`0x4000_0428`) -- are real HLE stubs. Boot runs on with zero traps
/// for another ~24,000 steps.
///
/// **What changed in Task 8 (sub-unit 2)**: the SPI1 flash controller
/// (`emulator_core::peripherals::flash::Spimem1`) now answers the JEDEC RDID
/// command with the badge's ID `0x46 0x40 0x16`, so the `E (0) memspi: no
/// response` error and its `assert`/`abort()` (Task D9's pinned stall) are
/// gone: boot prints `I (0) spi_flash: detected chip: generic` (step
/// ~446,991), `flash io: dio`, and the two `sleep_gpio:` lines.
///
/// **What changed in Task 8 (ROM libc)**: the two atomic ROM libc calls
/// right after, `memchr` (`0x4000_03c8`, first reached on step 490,128) and
/// `memmove` (`0x4000_035c`, step 490,143), are real HLE stubs.
///
/// **Task 8's stall** (history): on the 493,861st step boot fetched from
/// `0x4000_0060`, the then-unstubbed ROM `ets_apb_backup_init_lock_func`
/// (caller RA `0x4200_155c`), and the panic handler printed "Guru
/// Meditation Error" and "Rebooting..." and looped on `software_reset_cpu`.
///
/// **What changed in Task D10**: `ets_apb_backup_init_lock_func` stores its
/// two function-pointer arguments into the ROM statics `0x3fcd_f654`/
/// `0x3fcd_f658` (`RomStubEffect::StoreWords` with `WordSource::Register`);
/// the shortcut boot seeds the ROM's SPI-flash legacy data
/// (`rom_spiflash_legacy_data` -> `0x3fcd_f5c0`, with `chip_size` 4 MiB from
/// factory.bin's header), so the `Detected size(4096k) larger than the size
/// in the binary image header(0k)` warning is gone; the next two ROM calls,
/// `esp_coex_rom_version_get` (`0x4000_18ac`, returns the ROM version string
/// `0x3ff1_b74c`, now backed as ROM data) and `esprv_intc_int_set_threshold`
/// (`0x4000_05e4`, stores `a0` to `CPU_INT_THRESH_REG`), are stubbed too.
///
/// **Task D10's stall** (history): zero traps. FreeRTOS's
/// `xPortStartScheduler()` (IDF `components/freertos/FreeRTOS-Kernel/
/// portable/riscv/port.c`) sets the interrupt threshold to 1, enables
/// interrupts and calls `vPortYield()`, which requests the first context
/// switch by writing `SYSTEM_CPU_INTR_FROM_CPU_0_REG` (`0x600c_0028`,
/// `soc/system_reg.h`; `crosscore_int_ll_trigger_interrupt`) on the
/// 528,777th step. The SYSTEM peripheral was unmodeled, so no software
/// interrupt fired, `vTaskStartScheduler()` returned, and from step 528,805
/// on the CPU spun forever on a `j .` at `0x4200_0cd2`.
///
/// **What changed in Task 4**: the SYSTEM `FROM_CPU_0..3` registers are
/// modeled as level interrupt sources (`emulator_core::peripherals::system`,
/// sources 50..=53), and the interrupt matrix
/// (`emulator_core::peripherals::intc`) is source-indexed and gated by
/// enable and priority/threshold with the legacy-INTC rule (a line fires iff
/// its priority is `>=` `CPU_INT_THRESH`). The firmware routes
/// `ETS_FROM_CPU_INTR0_SOURCE` to CPU line 4 at priority 1, under
/// threshold 1. So the yield request on step 528,777 is taken as an
/// interrupt on step 528,778 (`mcause = 0x8000_0004`), the port's ISR
/// switches to the first task, and FreeRTOS runs: the console prints
/// `I (0) main_task: Started on CPU0` (step ~536,400) and
/// `I (0) main_task: Calling app_main()` (step ~555,650).
///
/// **Task 4's stall** (history): a real fault inside `app_main`. On the
/// 571,713th step it called the then-unstubbed ROM `gpio_matrix_out`
/// (`0x4000_05a4`, `esp32c3.rom.ld`; `a0 = 10`, `a1 = 0x41`), the panic
/// handler printed "Guru Meditation Error" and "Rebooting...", and its reboot
/// faulted on the unstubbed ROM `software_reset_cpu` (`0x4000_0094`).
///
/// **What changed in Task D11**: ROM `gpio_matrix_out` and `gpio_matrix_in`
/// are real stubs that write the GPIO matrix's `GPIO_FUNCn_OUT_SEL_CFG_REG`/
/// `GPIO_ENABLE_W1TS_REG` and `GPIO_FUNCn_IN_SEL_CFG_REG` through the bus
/// (`emulator-core/src/rom.rs`'s module doc, entry 22). The caller is
/// ESP-IDF's `spicommon_bus_initialize_io()` (`components/esp_driver_spi/
/// src/gpspi/spi_common.c:650-705`), routing SPI2 (FSPI) onto the display
/// pads: MOSI (`FSPID`, 65) on GPIO10, SCLK (`FSPICLK`, 63) on GPIO1, and
/// `FSPIWP`/`FSPIHD` (67/66) on GPIO0. Boot runs with **no exception** past
/// both calls, and `I (0) main_task: Calling app_main()` stays the newest
/// console line.
///
/// **Task D11's stall** (history): not a ROM call and not a fault, a spin on
/// SPI2. `spi_bus_initialize()` -> `spi_master_init_driver()`
/// (`spi_master.c:341`) -> `spi_hal_init()` (`components/hal/spi_hal.c:13-28`,
/// at `0x420f_d64c` in factory.bin) ends with `spi_ll_apply_config()`
/// (`components/hal/esp32c3/include/hal/spi_ll.h:264-267`): `hw->cmd.update
/// = 1; while (hw->cmd.update);`. It sets `SPI_UPDATE` (`SPI_CMD_REG` bit
/// 23, `soc/spi_reg.h`) on SPI2 (`0x6002_4000`), and the SPI2 model stored
/// that bit inertly, so from step 584,618 on the CPU spun forever on the
/// four-instruction poll loop `0x420f_d6fc..=0x420f_d702`.
///
/// **What changed in Task 9**: SPI2's `SPI_UPDATE` is a `WT` pulse that
/// reads back 0 at once, `SPI_USR` clears and `SPI_TRANS_DONE_INT_RAW` sets
/// when a transaction completes, and `TRANS_DONE` (RAW & ENA) drives
/// `ETS_SPI2_INTR_SOURCE` (`emulator_core::peripherals::spi`'s module doc).
/// So `spi_ll_apply_config()`'s poll exits on its first check, and boot
/// printed a new (emulator-only) warning from the SPI clock setup, from
/// `clk_hal_xtal_get_freq_mhz()` (`components/hal/esp32c3/clk_tree_hal.c`).
///
/// **Task 9's stall** (history): on the 602,868th step boot fetched from
/// `0x4000_0788`, the then-unstubbed libgcc ROM helper `__bswapsi2`
/// (`esp32c3.rom.libgcc.ld`), called (RA `0x4039_45fa`) from
/// `spi_ll_set_command()` (`spi_ll.h:1018-1030`, at `0x4039_45c0`): its
/// MSB-first branch computes `HAL_SPI_SWAP_DATA_TX(cmd, cmdlen)`, a
/// `HAL_SWAP32` = `__builtin_bswap32` (`hal/misc.h:15`), which RV32IMC
/// without Zbb compiles to that libcall. The panic handler then printed
/// "Guru Meditation Error" and its reboot faulted on `software_reset_cpu`.
///
/// **What changed in Task D12**: `__bswapsi2` is a real HLE stub
/// (`RomStubEffect::Int32Unary(Int32UnaryOp::Bswap)`), and the shortcut boot
/// seeds `RTC_XTAL_FREQ_REG` (`RTC_CNTL_STORE4_REG`, `0x6000_80b8`) with
/// `0x0028_0028`, the value the skipped 2nd-stage bootloader's
/// `clk_ll_xtal_store_freq_mhz(40)` leaves there
/// (`emulator-core/src/rom.rs`'s module doc, entry 23). Both XTAL warnings
/// are gone: the SPI clock setup's `clk_hal` one and 17 earlier `rtc_clk`
/// ones from `rtc_clk_xtal_freq_get()`, which the old pinned stall never
/// asserted. Neither is in the real badge's log. Not printing them makes the
/// whole timeline **earlier** (by 296 steps at ROM `qsort`'s return, by 629
/// steps at the first yield): the step numbers in the history paragraphs
/// above are as measured in their own tasks, not today's. Today's are: the
/// yield request on step 528,148, taken on step 528,149; `main_task: Calling
/// app_main()` at ~554,981; the `spi_hal_init()` poll at step 583,989; and
/// the one `__bswapsi2` call on step 593,018 (`a0 = 0`, `cmd = 0`; RA
/// `0x4039_45fa`). One SPI2 transaction then completes (`TRANS_DONE` raw and
/// enabled; `ETS_SPI2_INTR_SOURCE` is routed to line 6, which is not yet
/// enabled), and nothing is drawn.
///
/// **Task D12's stall** (history): not a ROM call and not a peripheral: on
/// the 596,609th step the CPU took an `ILLEGAL_INSTRUCTION` exception on a
/// `wfi` (`0x1050_0073`, also `mtval`) at `0x4038_b8bc`. That is
/// `esp_cpu_wait_for_intr()` (`components/esp_hw_support/cpu.c:52-64`,
/// `rv_utils_wait_for_intr()`), called from `esp_vApplicationIdleHook()`
/// (`components/esp_system/freertos_hooks.c:41-58`) in FreeRTOS's IDLE task
/// (its TCB name `IDLE` is on the panic dump's stack): `app_main`'s task had
/// blocked, and the idle task waited for the next interrupt. The core did
/// not implement `wfi`; the panic handler printed "Guru Meditation Error"
/// and its reboot faulted on the unstubbed ROM `software_reset_cpu`.
///
/// **What changed in Task 6**: `wfi` is a real instruction. It retires and
/// parks the core until an interrupt line is pending
/// (`emulator_core::cpu::Cpu::is_waiting`), and while the core waits with
/// nothing asserted, `emulator_core::boot::step_with_interrupts` jumps the
/// SYSTIMER straight to its next alarm. So the idle `wfi` on step 596,609
/// retires, and step 596,610 fast-forwards SYSTIMER by 91,439 ticks to the
/// FreeRTOS tick (alarm 0 on counter 1, routed to CPU line 5 at priority 1)
/// and takes it (`mcause = 0x8000_0005`). FreeRTOS then runs normally:
/// over the next ~25,000 steps the idle task waits and is woken 11 more
/// times (each jump 154,855..=158,163 ticks), then `app_main`'s work keeps
/// the CPU busy (LVGL rendering, at the hot fill/`memcpy`/`memset` loops)
/// and the tick keeps firing at the ordinary rate: 42 FreeRTOS ticks in all
/// by the next fault, 12 of them reached by fast-forward. Along the way
/// SPI2 raises 72 `TRANS_DONE` interrupts (then routed to CPU line 8), for
/// transfers alternating 1 byte and 12,800 bytes (20 rows of 320 RGB565
/// pixels), all with `SPI_DMA_TX_ENA` set: LVGL is
/// flushing frames to the ST7789 through GDMA. No new console line prints.
///
/// **What changed in Task 10**: GDMA's TX out-link is modeled and feeds
/// SPI2 whenever `SPI_DMA_TX_ENA` is set, so those transfers now reach the
/// ST7789 interpreter (before, every one sent the stale `W0..W15` buffer).
/// Measured with `boot-probe`: 234 SPI2 transactions by the stall, all on
/// GDMA channel 0, every one delivered in full (the panel init commands
/// with their parameters, then 36 pixel transfers of 12,800 bytes, three
/// full 320x240 frames of 12 bands each). The framebuffer is blank through
/// step 1,128,604 and non-blank from step 1,128,605; by the stall it holds
/// the firmware's boot splash, 2,340 distinct RGB565 values. Nothing else
/// moved: the timeline is step-for-step the same (the DMA path costs no
/// guest instructions), and the stall below is unchanged.
///
/// **The boot reaches `load_partitions()`'s `MD5Init`**, and it returns. On
/// the 5,555,258th step the CPU is on ROM `MD5Init` (`esp32c3.rom.ld`:
/// `MD5Init = 0x40000614`), called by ESP-IDF's `load_partitions()`
/// (`components/esp_partition/partition.c`, at `0x420f_a6ce` in factory.bin)
/// as its first act (a0 = the stack `md5_context_t`, `0x3fcb_fd30`), and
/// the Task D13 stub returns to `0x420f_a6ea`. The test asserts the whole
/// history up to there: the SYSTIMER/WFI fast-forward and FreeRTOS ticks,
/// SPI2 `TRANS_DONE` interrupts for DMA transfers feeding the ST7789 (the
/// framebuffer holds the 2,340-colour boot splash), the console lines
/// through `Calling app_main()`, and no exception before the `MD5Init`
/// call. What happens *after* it (Milestone 3: the unmapped flash MMU made
/// `load_partitions()` fail with `ESP_ERR_NOT_FOUND`) is covered by
/// `boot_progress.rs`'s
/// `load_partitions_accepts_the_synthesized_table_through_the_flash_mmu`.
#[test]
fn boot_stubs_reach_load_partitions_md5init_after_drawing_the_splash() {
    const FROM_CPU_0_REG: u32 = 0x600c_0028;
    const OLD_SPIN_PC: u32 = 0x4200_0cd2;
    const SPI_UPDATE_POLL: std::ops::RangeInclusive<u32> = 0x420f_d6fc..=0x420f_d702;
    const ROM_BSWAPSI2: u32 = 0x4000_0788;
    const SPI_LL_SET_COMMAND_RA: u32 = 0x4039_45fa;
    const ESP_CPU_WAIT_FOR_INTR_WFI: u32 = 0x4038_b8bc;
    // esp32c3.rom.ld: MD5Init = 0x40000614.
    const ROM_MD5_INIT: u32 = 0x4000_0614;
    // load_partitions()'s esp_rom_md5_init() call returns here.
    const LOAD_PARTITIONS_MD5INIT_RA: u32 = 0x420f_a6ea;
    // vSystimerSetup's alarm 0 period (TARGET0_CONF.period = 160,000).
    const FREERTOS_TICK_PERIOD: u64 = 160_000;
    // soc/spi_reg.h: SPI_DMA_INT_ENA_REG (+0x34), SPI_DMA_INT_RAW_REG
    // (+0x3C); SPI_TRANS_DONE_INT_* is bit 12 (byte 1, bit 4).
    const SPI_DMA_INT_ENA_OFFSET: u32 = 0x34;
    const SPI_DMA_INT_RAW_OFFSET: u32 = 0x3C;
    const TRANS_DONE_BYTE1: u8 = 1 << 4;
    // gpio_sig_map.h signal numbers and gpio_reg.h field bits.
    const FSPICLK_OUT_IDX: u32 = 63;
    const FSPID_OUT_IDX: u32 = 65;
    const FSPID_IN_IDX: u32 = 65;
    const SIG_IN_SEL: u32 = 1 << 6;

    let image = read_factory_bin();
    let mut rt = FirmwareRuntime::from_image(&image).expect("real factory.bin should boot");

    // Phase 1: fault-free up to and including vPortYield's write of the
    // cross-core software-interrupt register, which lands in the modeled
    // SYSTEM peripheral (not the unmapped catch-all) and asserts the source.
    let summary = rt.run(528_148);
    assert_eq!(
        summary.traps, 0,
        "expected a fault-free run; got {summary:?}"
    );
    assert_eq!(
        rt.bus().system.pending_mask(),
        0b0001,
        "FROM_CPU_0 asserted"
    );
    assert!(
        !rt.bus()
            .unmapped_log()
            .iter()
            .any(|a| a.addr & !3 == FROM_CPU_0_REG),
        "SYSTEM_CPU_INTR_FROM_CPU_0_REG is modeled now, not unmapped"
    );

    // Phase 2: the very next step takes it as an interrupt on CPU line 4
    // (MAP[ETS_FROM_CPU_INTR0_SOURCE = 50] = 4, PRI[4] = 1 >= THRESH 1).
    let summary = rt.run(1);
    assert_eq!(summary.traps, 1);
    assert_eq!(rt.cpu().csr.mcause, 0x8000_0004, "FROM_CPU_0's routed line");

    // Phase 3: main_task runs app_main, which routes SPI2 through the GPIO
    // matrix via the two ROM stubs and reaches spi_hal_init's UPDATE poll
    // after step 583,989 -- with no exception on the way (only interrupts).
    let summary = rt.run(583_989 - 528_149);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(rt.pc(), *SPI_UPDATE_POLL.start());
    let gpio = &rt.bus().gpio;
    assert_eq!(gpio.func_out_sel_cfg(10), Some(FSPID_OUT_IDX), "MOSI");
    assert_eq!(gpio.func_out_sel_cfg(1), Some(FSPICLK_OUT_IDX), "SCLK");
    assert_eq!(
        gpio.func_in_sel_cfg(FSPID_IN_IDX),
        Some(SIG_IN_SEL | 10),
        "FSPID input from GPIO10, through the matrix"
    );
    assert_eq!(rt.cpu().rom_stub_index_drops(), 0);
    assert_eq!(
        rt.bus().spi.read_byte(2) & 0x80,
        0,
        "SPI_UPDATE (SPI_CMD_REG bit 23) already reads back 0"
    );

    // Phase 4: the poll exits at once and boot runs, fault-free, up to the
    // call into ROM __bswapsi2 from spi_ll_set_command() (cmd = 0).
    let summary = rt.run(593_017 - 583_989);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(rt.pc(), ROM_BSWAPSI2);
    assert_eq!(rt.cpu().regs.read(1), SPI_LL_SET_COMMAND_RA, "ra");
    assert_eq!(rt.cpu().regs.read(10), 0, "a0");
    assert!(!SPI_UPDATE_POLL.contains(&rt.pc()));
    assert_ne!(rt.pc(), OLD_SPIN_PC);

    // Phase 5: the 593,018th step is the __bswapsi2 stub: it returns to
    // spi_ll_set_command() with the byte-swapped a0 (0).
    let summary = rt.run(1);
    assert_eq!(summary.traps, 0, "{summary:?}");
    assert_eq!(summary.rom_stub_calls, 1, "{summary:?}");
    assert_eq!(rt.pc(), SPI_LL_SET_COMMAND_RA);
    assert_eq!(rt.cpu().regs.read(10), 0);

    // Phase 6: no exception up to the idle task's wfi. One SPI2 transaction
    // has completed; nothing has been drawn; neither XTAL warning printed.
    let summary = rt.run(596_608 - 593_018);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    assert_eq!(rt.pc(), ESP_CPU_WAIT_FOR_INTR_WFI);
    let console = rt.console_output();
    assert!(
        console.contains("I (0) main_task: Calling app_main()"),
        "console:\n{console}"
    );
    assert!(
        !console.contains("invalid RTC_XTAL_FREQ_REG"),
        "console:\n{console}"
    );
    assert!(
        !console.contains("Guru Meditation Error"),
        "console:\n{console}"
    );
    assert_ne!(
        rt.bus().spi.read_byte(SPI_DMA_INT_RAW_OFFSET + 1) & TRANS_DONE_BYTE1,
        0,
        "one SPI2 transaction has completed"
    );
    assert_ne!(
        rt.bus().spi.read_byte(SPI_DMA_INT_ENA_OFFSET + 1) & TRANS_DONE_BYTE1,
        0
    );
    assert!(rt.framebuffer().iter().all(|px| *px == 0));

    // Phase 7: the 596,609th step: the idle task's wfi retires and parks
    // the core (no trap; it used to be an ILLEGAL_INSTRUCTION here).
    assert!(!rt.cpu().is_waiting());
    let summary = rt.run(1);
    assert_eq!(summary.traps, 0, "{summary:?}");
    assert!(rt.cpu().is_waiting(), "wfi parks the core");
    assert_eq!(rt.pc(), ESP_CPU_WAIT_FOR_INTR_WFI + 4);

    // Phase 8: the 596,610th step fast-forwards SYSTIMER straight to the
    // FreeRTOS tick (alarm 0, routed to CPU line 5) and takes it, with mepc
    // the instruction after the wfi.
    let jump = rt
        .bus()
        .systimer
        .ticks_until_next_alarm()
        .expect("the FreeRTOS tick is armed");
    assert_eq!(jump, 91_439, "ticks to the next alarm");
    let elapsed_before = rt.bus().systimer.elapsed_ticks();
    let summary = rt.run(1);
    assert_eq!(summary.traps, 1, "{summary:?}");
    assert!(!rt.cpu().is_waiting());
    assert_eq!(rt.cpu().csr.mcause, 0x8000_0005, "the tick's routed line");
    assert_eq!(rt.cpu().csr.mepc, ESP_CPU_WAIT_FOR_INTR_WFI + 4);
    assert_eq!(rt.bus().systimer.elapsed_ticks() - elapsed_before, jump);

    // Phase 9: no exception up to the MD5Init call. LVGL flushes frames
    // over SPI2 through GDMA channel 0 (Task 10), so they are drawn; no new
    // console line and no panic.
    let summary = rt.run(5_555_257 - 596_610);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(
        rt.cpu().csr.mcause & 0x8000_0000,
        0x8000_0000,
        "the last trap taken was an interrupt; mcause=0x{:08x}",
        rt.cpu().csr.mcause
    );
    assert!(
        rt.bus().systimer.counter(1) >= 42 * FREERTOS_TICK_PERIOD,
        "42 FreeRTOS tick periods have elapsed on counter 1: {}",
        rt.bus().systimer.counter(1)
    );
    assert!(rt.bus().spi.dma_tx_enabled(), "SPI2 transfers use DMA");
    assert_eq!(rt.bus().gdma.channel_for_spi2(), Some(0), "via GDMA ch0");
    assert_ne!(
        rt.bus().gdma.out_link_state(0).last_desc,
        0,
        "GDMA ch0 has sent descriptors"
    );
    let distinct: std::collections::HashSet<u16> = rt.framebuffer().iter().copied().collect();
    // Measured 2,340; a margin below that, far above a fill or a band.
    assert!(
        distinct.len() >= 2_000,
        "framebuffer holds a drawn frame: {} distinct colors",
        distinct.len()
    );
    let console = rt.console_output();
    assert!(
        console
            .trim_end()
            .ends_with("I (0) main_task: Calling app_main()"),
        "console:\n{console}"
    );

    // Phase 10: the 5,555,258th step is the ROM MD5Init stub, called by
    // load_partitions() with its stack md5_context_t; it returns.
    assert_eq!(rt.pc(), ROM_MD5_INIT);
    assert_eq!(rt.cpu().regs.read(10), 0x3fcb_fd30, "a0 = &context");
    let summary = rt.run(1);
    assert_eq!(summary.traps, 0, "{summary:?}");
    assert_eq!(summary.rom_stub_calls, 1, "{summary:?}");
    assert_eq!(rt.pc(), LOAD_PARTITIONS_MD5INIT_RA);
}

/// Task D10, item B: the shortcut boot seeds the ROM's SPI-flash legacy data
/// before the first instruction, and the firmware's own writes then land in
/// it, not at address 0 (before Task D10 the NULL `rom_spiflash_legacy_data`
/// pointer sent `bootloader_flash_update_id()`'s `device_id` store to
/// `0x0..0x3` and the flash-size reads to `0x4..0x7`, all unmapped).
#[test]
fn rom_spiflash_legacy_data_is_seeded_and_no_null_legacy_data_access_remains() {
    use emulator_core::mem::Bus;
    let image = read_factory_bin();
    let (mut cpu, mut bus) =
        boot_from_factory_image_with_rom_stubs(&image).expect("real factory.bin should boot");

    // Before the first instruction: the mask ROM's .data init plus the
    // bootloader's esp_rom_spiflash_config_param(), with chip_size from
    // factory.bin's own header (byte 3 = 0x2f: 4 MB).
    assert_eq!(bus.read32(0x3fcd_fff0), 0x3fcd_f5c0);
    let chip = |bus: &mut emulator_core::mem::bus::FirmwareBus| -> Vec<u32> {
        (0..7).map(|i| bus.read32(0x3fcd_f5c0 + 4 * i)).collect()
    };
    let seeded = [0x0046_4016, 0x0040_0000, 0x1_0000, 0x1000, 0x100, 0xffff, 0];
    assert_eq!(chip(&mut bus), seeded);

    // Run through flash-chip detection and the sleep_gpio lines (step
    // ~484,400), checking every step that no access touched the first page
    // of the address space.
    for step in 1..=500_000u32 {
        step_with_interrupts(&mut cpu, &mut bus);
        if let Some(last) = bus.unmapped_log().back() {
            assert!(
                last.addr >= 0x100,
                "step {step}: unmapped access at {:#x} (write={})",
                last.addr,
                last.is_write
            );
        }
    }
    // The app's own bootloader_flash_update_id() (cpu_start.c) re-read the
    // JEDEC ID into the struct: same value, now through a valid pointer.
    assert_eq!(chip(&mut bus), seeded);
    assert!(bus
        .console
        .text()
        .contains("I (0) sleep_gpio: Enable automatic switching of GPIO sleep configuration"));
}

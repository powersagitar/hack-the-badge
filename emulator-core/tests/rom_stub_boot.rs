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
//! The `eprintln!` trace below is evidence a human reads; the stall-by-stall
//! history behind each budget and phase is in
//! `docs/firmware-emulator-notes.md` ("History").

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

    // Past the first real log line's `ets_get_cpu_frequency`/`ets_printf`
    // calls (steps ~400,695/~400,714), so every ROM call asserted below is
    // reached, and well short of boot's first trap (the FROM_CPU_0 yield
    // interrupt on step 528,152).
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

/// Boot with the ROM stubs reaches `load_partitions()`'s ROM `MD5Init`
/// call, and the phases on the way are each checked at their exact step
/// (step-exact by design: any earlier change moves them; the notes'
/// history has how each was reached):
///
/// 1. Fault-free through step 528,151, where vPortYield writes
///    `SYSTEM_CPU_INTR_FROM_CPU_0_REG` (modeled, not unmapped) and asserts
///    `FROM_CPU_0`.
/// 2. The next step takes it as an interrupt on CPU line 4 (the firmware
///    routes `ETS_FROM_CPU_INTR0_SOURCE` there at priority 1, threshold 1).
/// 3. No exception to step 583,976, `spi_hal_init()`'s `SPI_UPDATE` poll;
///    on the way the ROM `gpio_matrix_out`/`gpio_matrix_in` stubs route
///    SPI2 onto the display pads (MOSI on GPIO10, SCLK on GPIO1) and
///    `SPI_UPDATE` already reads back 0.
/// 4. The poll exits at once; no exception to step 593,004, the call into
///    ROM `__bswapsi2` from `spi_ll_set_command()` (`a0 = 0`).
/// 5. Step 593,005 is the `__bswapsi2` stub; it returns to its caller.
/// 6. No exception to the FreeRTOS idle task's `wfi` (`0x4038_b8bc`,
///    `esp_cpu_wait_for_intr()`); one SPI2 transaction has completed,
///    nothing is drawn, and neither (emulator-only) XTAL warning printed.
/// 7. Step 596,596: the `wfi` retires and parks the core.
/// 8. Step 596,597 fast-forwards SYSTIMER by 91,455 ticks to the FreeRTOS
///    tick (alarm 0 on counter 1, CPU line 5) and takes it.
/// 9. No exception to step 5,558,962: 42 tick periods have elapsed, LVGL
///    flushes frames over SPI2 through GDMA channel 0, and the framebuffer
///    holds the boot splash (at least 2,000 distinct colors; 2,340).
/// 10. Step 5,558,963 is ROM `MD5Init`, called by `load_partitions()`
///     with its stack `md5_context_t` (`0x3fcb_fd30`); it returns to
///     `0x420f_a6ea`.
///
/// What follows is `boot_progress.rs`'s
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
    let summary = rt.run(528_151);
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
    // after step 583,976 -- with no exception on the way (only interrupts).
    let summary = rt.run(583_976 - 528_152);
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
    let summary = rt.run(593_004 - 583_976);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");
    assert_eq!(rt.pc(), ROM_BSWAPSI2);
    assert_eq!(rt.cpu().regs.read(1), SPI_LL_SET_COMMAND_RA, "ra");
    assert_eq!(rt.cpu().regs.read(10), 0, "a0");
    assert!(!SPI_UPDATE_POLL.contains(&rt.pc()));
    assert_ne!(rt.pc(), OLD_SPIN_PC);

    // Phase 5: the 593,005th step is the __bswapsi2 stub: it returns to
    // spi_ll_set_command() with the byte-swapped a0 (0).
    let summary = rt.run(1);
    assert_eq!(summary.traps, 0, "{summary:?}");
    assert_eq!(summary.rom_stub_calls, 1, "{summary:?}");
    assert_eq!(rt.pc(), SPI_LL_SET_COMMAND_RA);
    assert_eq!(rt.cpu().regs.read(10), 0);

    // Phase 6: no exception up to the idle task's wfi. One SPI2 transaction
    // has completed; nothing has been drawn; neither XTAL warning printed.
    let summary = rt.run(596_595 - 593_005);
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
        console.contains("I (5) main_task: Calling app_main()"),
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

    // Phase 7: the 596,596th step: the idle task's wfi retires and parks
    // the core (no trap; it used to be an ILLEGAL_INSTRUCTION here).
    assert!(!rt.cpu().is_waiting());
    let summary = rt.run(1);
    assert_eq!(summary.traps, 0, "{summary:?}");
    assert!(rt.cpu().is_waiting(), "wfi parks the core");
    assert_eq!(rt.pc(), ESP_CPU_WAIT_FOR_INTR_WFI + 4);

    // Phase 8: the 596,597th step fast-forwards SYSTIMER straight to the
    // FreeRTOS tick (alarm 0, routed to CPU line 5) and takes it, with mepc
    // the instruction after the wfi.
    let jump = rt
        .bus()
        .systimer
        .ticks_until_next_alarm()
        .expect("the FreeRTOS tick is armed");
    assert_eq!(jump, 91_455, "ticks to the next alarm");
    let elapsed_before = rt.bus().systimer.elapsed_ticks();
    let summary = rt.run(1);
    assert_eq!(summary.traps, 1, "{summary:?}");
    assert!(!rt.cpu().is_waiting());
    assert_eq!(rt.cpu().csr.mcause, 0x8000_0005, "the tick's routed line");
    assert_eq!(rt.cpu().csr.mepc, ESP_CPU_WAIT_FOR_INTR_WFI + 4);
    assert_eq!(rt.bus().systimer.elapsed_ticks() - elapsed_before, jump);

    // Phase 9: no exception up to the MD5Init call. LVGL flushes frames
    // over SPI2 through GDMA channel 0 (Task 10), so they are drawn; no
    // panic. (Milestone 4 Task 4: one new console line, `LVGL: Starting
    // LVGL task`, which moved MD5Init from step 5,555,258 to 5,558,994;
    // Milestone 5 Task D-M5-3's cycle counter moved it to 5,558,963.)
    let summary = rt.run(5_558_962 - 596_597);
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
    // Containment, not "the last line" (Milestone 4 Task 4): post-scheduler
    // log lines (`LVGL: Starting LVGL task`) now reach the console too.
    let console = rt.console_output();
    assert!(
        console.contains("I (5) main_task: Calling app_main()"),
        "console:\n{console}"
    );

    // Phase 10: the 5,558,963rd step is the ROM MD5Init stub, called by
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
        .contains("I (5) sleep_gpio: Enable automatic switching of GPIO sleep configuration"));
}

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
    // budget into a *new*, later, unstubbed-ROM-call fault (see
    // `boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call`
    // below, which pins that exact new stall). 350,000 keeps this test's
    // original claim -- "gets past the mask ROM wall with zero faults, and
    // reaches every one of the named early-boot ROM calls below" -- true.
    // Task D2 (`emulator-core/src/cpu/rom_stubs.rs`'s `RomStubEffect::Memcpy`)
    // moved the fault this budget stays clear of from step 401,761 (an
    // unstubbed `memcpy`) to step 402,113 (an unstubbed
    // `ets_efuse_get_spiconfig`); Task D3 (`emulator_core::rom`'s module doc,
    // entry 10) moved it further still, to step 405,806 (an unstubbed
    // `esprv_intc_int_enable`) -- comfortably clear either way
    // (~55,806-step/~16% margin at the new fault), without this test
    // needing to also pin the new, later stall (that's the other test's
    // job).
    const STEP_BUDGET: usize = 350_000;

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

/// Where boot currently *stops*: **not** a spin loop any more. Milestone 3
/// Task D3 (`emulator-core/src/rom.rs`'s module doc, entry 10) unblocked the
/// fault this test used to pin (an unstubbed `ets_efuse_get_spiconfig` call
/// at `0x4000_071c`) and five more it exposed one at a time
/// (`ets_efuse_get_wp_pad`, `uart_tx_wait_idle`, `intr_matrix_set`,
/// `esprv_intc_int_disable`, `esprv_intc_int_set_type`,
/// `esprv_intc_int_set_priority`), so boot now runs further still and hits a
/// **new** unstubbed ROM call: `esprv_intc_int_enable` (`0x4000_05e8`, named
/// in `esp32c3.rom.ld`, deprecated alias declared in
/// `components/riscv/include/esp_private/interrupt_deprecated.h` — confirmed
/// by fetching both at tag `v5.5.3`, see `emulator_core::rom`'s module doc).
/// Per Task D3's brief/orchestrator ruling, this address is deliberately
/// **not** stubbed by that task: unlike its four `esprv_intc_int_*` siblings
/// (which the boot-probe evidence showed writing already-zero registers, a
/// provably inert no-op), this call sets a bit in `CPU_INT_ENABLE_REG` — a
/// register `crate::peripherals::intc::InterruptController::poll` genuinely
/// consults — so a `void` no-op here isn't a safe default the way it was for
/// its siblings, and a real register write would need new stub-mechanism
/// plumbing the task's brief scoped out ("peripherals are out of scope
/// here"). This is a real `INSTRUCTION_ACCESS_FAULT`, caught by the
/// firmware's own already-working panic handler (it prints a full "Guru
/// Meditation Error" register dump via `ets_printf`, confirming `mtvec`,
/// `ets_printf`, and this emulator's console capture are all working
/// correctly). The panic handler's own reboot attempt then calls a second
/// unstubbed ROM function, `software_reset_cpu` (`0x4000_0094`, named in
/// `esp32c3.rom.ld`), which faults too, re-entering the panic handler's
/// re-entrancy guard ("Panic handler entered multiple times...") and
/// retrying forever — this emulator has no way to actually reboot, so this
/// retry loop is the terminal state within any reasonable step budget. Per
/// the brief, `software_reset_cpu` is not stubbed either (it's only reached
/// via the panic path, and stubbing it wouldn't fix the root cause at
/// `0x4000_05e8`).
///
/// **History**: until Milestone 3 Task D3, this test pinned an *earlier*
/// stall — an unstubbed `ets_efuse_get_spiconfig` call at `0x4000_071c` (see
/// the Task D2 report). That call, and five more it led to, are now stubbed
/// (`emulator-core/src/rom.rs`'s module doc, entry 10), so this test's
/// expected stall point moved forward to this new unstubbed-ROM-call fault.
#[test]
fn boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call() {
    let image = read_factory_bin();
    let mut rt = FirmwareRuntime::from_image(&image).expect("real factory.bin should boot");

    // Comfortably past the fault (step 405,806) but well before the second
    // fault (`software_reset_cpu`, during the panic handler's own reboot
    // attempt) -- a single, stable snapshot: exactly one trap has occurred,
    // at exactly this address. 500,000 is the same budget the pre-Task-D3
    // version of this test used, re-measured to still land in the same
    // single-trap window after this task's fixes.
    let summary = rt.run(500_000);
    assert_eq!(
        summary.last_instruction_fault,
        Some(0x4000_05e8),
        "expected the new unstubbed-ROM-call fault at 0x4000_05e8 \
         (esprv_intc_int_enable)"
    );
    assert_eq!(
        summary.traps, 1,
        "expected exactly one trap so far (the fault above) -- the second \
         fault (software_reset_cpu, during the panic handler's own reboot \
         attempt) is not reached within this budget"
    );

    // The firmware's own panic handler ran and printed a full crash report
    // -- generic ESP-IDF text, never identity data (see this file's module
    // doc and `docs/firmware-emulator-notes.md`'s data-handling note).
    assert!(
        rt.console_output().contains("Guru Meditation Error"),
        "expected the firmware's panic handler to have printed its crash \
         report; got:\n{}",
        rt.console_output()
    );

    // And nothing has been drawn, because the display driver is never reached.
    assert!(rt.framebuffer().iter().all(|px| *px == 0));
}

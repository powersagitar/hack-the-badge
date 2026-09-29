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

/// This test **pins today's panic path, not boot progress past it**.
/// **Renamed and re-pointed in Task D7** (from
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
/// `0x3fcd_f660`/`0x3fcd_f65c` (`RomStubEffect::LoadStoreWords`), and the
/// atomic ROM libc calls `esp_newlib_init`'s successors make -- `strlen`
/// (`0x4000_0374`), `memcmp` (`0x4000_0360`), `strncmp` (`0x4000_0370`),
/// `div` (`0x4000_0428`) -- are real HLE stubs. Boot runs on with zero traps
/// for another ~24,000 steps.
///
/// **The new stall**: ESP-IDF's flash-chip detection (`esp_flash_init` /
/// memspi) reads the flash JEDEC ID through the SPI1 flash controller,
/// which this emulator does not model, and logs `E (0) memspi: no response`
/// (step 441,439). It then fails an `assert` -- `assert failed:
/// 0x4039734c <cached disabled>:118` printed at step ~443,599 -- so
/// `abort()` runs `panic_abort()`'s ILLEGAL_INSTRUCTION on the 442,141st step
/// (mepc `0x4038_e4fa`). Because this is an `abort()`, "Guru Meditation
/// Error" prints only after the panic handler's reboot retry faults, at step
/// ~682,319 ("Rebooting..." at ~680,821, the `software_reset_cpu` fault at
/// step 681,436, next fault at 921,911). `software_reset_cpu` stays
/// unstubbed: it is reached only on the panic path.
///
/// This test is **deliberately expected to break** once a later task models
/// the SPI1 flash controller; whoever makes that fix should delete or
/// replace it rather than chase a new pinned value here.
#[test]
fn boot_currently_aborts_on_memspi_no_response_and_reaches_the_panic_handlers_reboot_message() {
    let image = read_factory_bin();
    let mut rt = FirmwareRuntime::from_image(&image).expect("real factory.bin should boot");

    // Phase 1: through the newlib-init ROM calls, heap_init and the start
    // of flash detection, with zero traps.
    let summary = rt.run(442_140);
    assert_eq!(
        summary.traps, 0,
        "boot must run through esp_newlib_init's ROM calls without any trap \
         (pre-Task-D9 it faulted on esp_rom_newlib_init_common_mutexes at \
         step 417,992); got {summary:?}"
    );
    assert!(
        rt.console_output().contains("E (0) memspi: no response"),
        "expected the flash-detection error line before the abort; got:\n{}",
        rt.console_output()
    );

    // Phase 2: the very next step is the abort()'s ILLEGAL_INSTRUCTION.
    let summary = rt.run(1);
    assert_eq!(summary.traps, 1, "expected the abort trap; got {summary:?}");
    assert_eq!(rt.cpu().csr.mcause, exception_code::ILLEGAL_INSTRUCTION);
    assert_eq!(rt.cpu().csr.mepc, 0x4038_e4fa);

    // Phase 3, up to 690,000 total: the panic handler's reboot attempt
    // faults on unstubbed software_reset_cpu (the 681,436th step). 8,564
    // steps of margin, and the next fault is not until step 921,911.
    let summary = rt.run(690_000 - 442_141);
    assert_eq!(
        summary.traps, 1,
        "expected exactly one more trap: the software_reset_cpu \
         instruction-access fault; got {summary:?}"
    );
    assert_eq!(
        summary.last_instruction_fault,
        Some(0x4000_0094),
        "expected the INSTRUCTION_ACCESS_FAULT to be software_reset_cpu -- \
         the panic handler's own (unstubbed) reboot attempt"
    );

    let console = rt.console_output();
    // Generic ESP-IDF text only, never identity data (see this file's
    // module doc and `docs/firmware-emulator-notes.md`'s data-handling
    // note).
    assert!(
        console.contains("I (0) heap_init: Initializing. RAM available for dynamic allocation:"),
        "expected heap_init's first line; got:\n{console}"
    );
    assert!(
        !console.contains("memory_layout"),
        "the reserved-region overlap error (Task D6's stall) must be gone; got:\n{console}"
    );
    assert!(
        console.contains("assert failed"),
        "expected the failed assert's message; got:\n{console}"
    );
    assert!(
        console.contains("Guru Meditation Error"),
        "expected the panic handler's crash report; got:\n{console}"
    );
    assert!(
        console.contains("Rebooting..."),
        "expected the panic handler's generic pre-restart text; got:\n{console}"
    );

    // Task D5's fix still holds: cpu_start's header check passes.
    assert!(
        !console.contains("Invalid app image header"),
        "cpu_start's header check should pass for real; got:\n{console}"
    );

    // And nothing has been drawn, because the display driver is never reached.
    assert!(rt.framebuffer().iter().all(|px| *px == 0));
}

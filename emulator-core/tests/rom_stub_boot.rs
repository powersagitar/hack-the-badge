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
    // `boot_currently_aborts_on_the_unbacked_rom_layout_reserved_region_overlap_and_reaches_the_panic_handlers_reboot_message`
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
/// **Renamed and re-pointed in Task D6** (from
/// `boot_currently_faults_on_the_unstubbed_qsort_call_and_reaches_the_panic_handlers_reboot_message`,
/// itself renamed in Task D5 from
/// `boot_currently_aborts_reaching_the_panic_handlers_reboot_message`).
///
/// **What changed in Task D6**: ROM libc `qsort` (`0x4000_0434`) is now
/// real guest-executed code (`emulator_core::rom::QSORT_BODY`, see
/// `emulator-core/src/rom.rs`'s module doc, entry 14), so the
/// `INSTRUCTION_ACCESS_FAULT` Task D5 pinned at step 408,481 is gone: the
/// call runs and returns (step 408,906). Task D6 Step 1 also **refuted**
/// D5's unconfirmed guess about the caller: it is not `do_system_init_fn()`
/// but ESP-IDF v5.5.3's `s_prepare_reserved_regions()`
/// (`components/heap/port/memory_layout_utils.c`), sorting its 5
/// `soc_reserved_region_t {start, end}` entries (hence `size` 8) with
/// `s_compare_reserved_regions` (`0x420029bc`).
///
/// **The new stall** is that function's own validity check, right after the
/// sort: entry 0 of the array is the ROM layout table's reserved DRAM range,
/// `{ets_rom_layout_p->dram0_rtos_reserved_start, SOC_DIRAM_DRAM_HIGH}`
/// (`ESP_ROM_HAS_LAYOUT_TABLE` is set for the ESP32-C3). `ets_rom_layout_p`
/// is a **ROM data** pointer (`esp32c3.rom.ld`: `ets_rom_layout_p =
/// 0x3ff1fffc;`) that this emulator does not back, so it reads `0` from the
/// bus catch-all, and so does the `NULL`-relative field load after it. The
/// region becomes `0x00000000 - 0x3fce0000`, which overlaps the next one
/// after sorting, so the firmware logs
/// `E (0) memory_layout: SOC_RESERVE_MEMORY_REGION region range 0x00000000 -
/// 0x3fce0000 overlaps with 0x3fc80000 - 0x3fc99c00` (step 408,970) and calls
/// `abort()`. That line is also end-to-end evidence that `qsort` really
/// sorted: in the unsorted input, `0x3fc80000` was the 4th element, not the
/// 2nd. `abort()` reaches the `ILLEGAL_INSTRUCTION` trap in `panic_abort()`
/// at `0x4038e4fa` (step 409,071), which is the same mechanism Task D4
/// pinned. The panic handler prints `abort() was called at PC 0x42002acf on
/// core 0` plus a register dump, then "Rebooting..." (step ~646,656). It
/// then faults on the still-unstubbed ROM `software_reset_cpu`
/// (`0x4000_0094`, step 647,238), re-enters the panic handler, prints "Guru
/// Meditation Error" (step ~648,960) and loops (next fault at step 887,681).
/// `software_reset_cpu` stays unstubbed: it is reached only on the panic
/// path, and stubbing it would not fix the actual blocker, the unbacked ROM
/// layout table.
///
/// This test is **deliberately expected to break** once a later task gives
/// `ets_rom_layout_p` a real value (or otherwise gets boot past this
/// check). At that point this fault sequence disappears, and whoever makes
/// that fix should delete or replace this test rather than chase a new
/// pinned value here.
#[test]
fn boot_currently_aborts_on_the_unbacked_rom_layout_reserved_region_overlap_and_reaches_the_panic_handlers_reboot_message(
) {
    let image = read_factory_bin();
    let mut rt = FirmwareRuntime::from_image(&image).expect("real factory.bin should boot");

    // Phase 1: through the old qsort fault (step 408,481) and past qsort's
    // return (step 408,906) with zero traps. The first trap comes at step
    // 409,071.
    let summary = rt.run(409_000);
    assert_eq!(
        summary.traps, 0,
        "ROM qsort must run and return without any trap (pre-Task-D6 it \
         faulted at step 408,481); got {summary:?}"
    );

    // Phase 2: the abort()'s panic_abort() trap -- ILLEGAL_INSTRUCTION at
    // 0x4038e4fa, not an instruction-access fault.
    let summary = rt.run(100);
    assert_eq!(
        summary.traps, 1,
        "expected panic_abort()'s trap; got {summary:?}"
    );
    assert_eq!(summary.last_instruction_fault, None);
    assert_eq!(
        rt.cpu().csr.mcause,
        emulator_core::cpu::exception_code::ILLEGAL_INSTRUCTION
    );
    assert_eq!(rt.cpu().csr.mepc, 0x4038_e4fa);

    // Phase 3, up to 660,000 total: the panic handler's reboot attempt
    // faults on unstubbed software_reset_cpu (step 647,238). 12,762 steps of
    // margin, and the next fault is not until step 887,681.
    let summary = rt.run(660_000 - 409_100);
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
    // The root cause, in the firmware's own words. Generic ESP-IDF text
    // plus SoC memory-map addresses, never identity data (see this file's
    // module doc and `docs/firmware-emulator-notes.md`'s data-handling
    // note).
    assert!(
        console.contains(
            "E (0) memory_layout: SOC_RESERVE_MEMORY_REGION region range \
             0x00000000 - 0x3fce0000 overlaps with 0x3fc80000 - 0x3fc99c00"
        ),
        "expected s_prepare_reserved_regions()'s overlap error, which is \
         also proof that qsort sorted the regions; got:\n{console}"
    );
    assert!(
        console.contains("abort() was called at PC 0x42002acf on core 0"),
        "expected the abort() from s_prepare_reserved_regions(); got:\n{console}"
    );
    assert!(
        console.contains("Rebooting..."),
        "expected the panic handler's generic pre-restart text; got:\n{console}"
    );
    assert!(
        console.contains("Guru Meditation Error"),
        "expected the re-entered panic handler's crash report; got:\n{console}"
    );

    // Task D5's fix still holds: cpu_start's header check passes.
    assert!(
        !console.contains("Invalid app image header"),
        "cpu_start's header check should pass for real; got:\n{console}"
    );

    // And nothing has been drawn, because the display driver is never reached.
    assert!(rt.framebuffer().iter().all(|px| *px == 0));
}

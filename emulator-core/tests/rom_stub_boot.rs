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
    // `boot_currently_faults_on_the_unstubbed_qsort_call_and_reaches_the_panic_handlers_reboot_message`
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
/// **Renamed and re-pointed in Task D5** (from
/// `boot_currently_aborts_reaching_the_panic_handlers_reboot_message`):
/// Task D5 (`crate::mem::bus::FirmwareBus::from_segments`'s page-granular
/// XIP mapping, see `emulator-core/tests/boot_progress.rs`'s module doc's
/// "Task D5 status") fixed `cpu_start`'s app-image-header check for real --
/// it now reads the image's actual magic byte (`0xE9`) instead of a
/// catch-all `0`, passes, and never calls `abort()`. Every fault sequence
/// this test used to pin (newlib's `abort()`, `panic_abort()`'s
/// `ILLEGAL_INSTRUCTION` trap at `0x4038e4fa`) is gone; boot instead runs
/// much further -- through `cpu_start`'s remaining startup log lines and a
/// full `app_init`/`efuse_init` block that never used to print at all --
/// before hitting a **new, unrelated** stall this task's own scope ruling
/// says to stop at and report, not fix: an unstubbed ROM `qsort` call
/// (`0x4000_0434`, confirmed against `esp32c3.rom.libc.ld`'s `qsort =
/// 0x40000434;`). The register dump at the fault (`A1=5, A2=8`, return
/// address inside the app's own `esp_system` startup code) is consistent
/// with ESP-IDF's `do_system_init_fn()` sorting its init-function array
/// before running it -- a plain missing-ROM-stub gap, left for the next
/// task.
///
/// That `INSTRUCTION_ACCESS_FAULT` (step 408,481) is a genuine hardware
/// exception this time, not an `abort()`, so ESP-IDF's panic handler takes
/// its *exception* path immediately (`info->reason` non-`NULL` from the
/// first pass) and prints "Guru Meditation Error" right away, then
/// `panic_restart()` calls into the still-unstubbed ROM `software_reset_cpu`
/// (`0x4000_0094`) to actually reboot -- which faults again, re-entering
/// the panic handler and looping, same downstream shape as the old
/// cpu_start-abort scenario (coincidentally: both are a first
/// `INSTRUCTION_ACCESS_FAULT`-class fault immediately followed by the same
/// unstubbed reboot-retry fault), just with a different, earlier root
/// cause. `software_reset_cpu` remains unstubbed -- it's only reached via
/// the panic path, per this module's own scoping, and stubbing it wouldn't
/// address the `qsort` gap, the actual blocker now.
///
/// This test is **deliberately expected to break** once a later task stubs
/// `qsort` (or otherwise gets boot past it): at that point this exact fault
/// sequence disappears, and whoever makes that fix should delete or
/// replace this test rather than chase a new pinned value here.
#[test]
fn boot_currently_faults_on_the_unstubbed_qsort_call_and_reaches_the_panic_handlers_reboot_message()
{
    let image = read_factory_bin();
    let mut rt = FirmwareRuntime::from_image(&image).expect("real factory.bin should boot");

    // 650,000 steps is a single, stable, two-trap snapshot: comfortably
    // past both the qsort INSTRUCTION_ACCESS_FAULT (measured at step
    // 408,481) and the software_reset_cpu INSTRUCTION_ACCESS_FAULT it
    // leads to (measured reaching the console at step 648,457), but well
    // before the panic handler's re-entrancy guard kicks in and the
    // "Rebooting"/panic text starts repeating many more times.
    let summary = rt.run(650_000);
    assert_eq!(
        summary.traps, 2,
        "expected exactly two traps so far: the unstubbed qsort call's \
         instruction-access fault, then the software_reset_cpu \
         instruction-access fault it leads to"
    );
    assert_eq!(
        summary.last_instruction_fault,
        Some(0x4000_0094),
        "expected the most recent INSTRUCTION_ACCESS_FAULT to be \
         software_reset_cpu -- the panic handler's own (unstubbed) reboot \
         attempt"
    );

    // The firmware's own panic handler ran and printed ESP-IDF's generic
    // pre-restart text plus a full crash report -- generic ESP-IDF text,
    // never identity data (see this file's module doc and
    // `docs/firmware-emulator-notes.md`'s data-handling note).
    assert!(
        rt.console_output().contains("Rebooting..."),
        "expected the panic handler's generic pre-restart text; got:\n{}",
        rt.console_output()
    );
    assert!(
        rt.console_output().contains("Guru Meditation Error"),
        "expected the firmware's panic handler to have printed its crash \
         report; got:\n{}",
        rt.console_output()
    );

    // Task D5's actual fix, positively confirmed: cpu_start's header check
    // now passes for real, so its error line must never appear, and boot
    // must reach several genuinely new lines past it (the dedicated
    // ratchet rung for the specific budget is
    // `tests/boot_progress.rs`'s `boot_reaches_efuse_inits_chip_rev_line`;
    // reinforced here as one more assertion on this already-pinned trace).
    assert!(
        !rt.console_output().contains("Invalid app image header"),
        "cpu_start's header check should now pass for real -- this line \
         must never appear; got console:\n{}",
        rt.console_output()
    );
    assert!(
        rt.console_output().contains("efuse_init: Chip rev:"),
        "expected boot to reach the new efuse_init block past the fixed \
         header check; got console:\n{}",
        rt.console_output()
    );

    // And nothing has been drawn, because the display driver is never reached.
    assert!(rt.framebuffer().iter().all(|px| *px == 0));
}

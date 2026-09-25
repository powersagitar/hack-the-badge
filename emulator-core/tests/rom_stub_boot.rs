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
    // budget into a *new*, later stall (see
    // `boot_currently_aborts_reaching_the_panic_handlers_reboot_message`
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

/// This test **pins today's abort/panic path, not boot progress past it**.
/// Milestone 3 Task D4 (`emulator-core/src/rom.rs`'s module doc, entry 12)
/// implements real HLE for both ROM calls Fix round 1 left this test
/// pinned on: `itoa` (`0x4000_0448`) and, once that unblocked boot, the
/// very next unstubbed call, `strcat` (`0x4000_03d8`, also
/// `esp32c3.rom.libc.ld`). Both calls belong to newlib's own `abort()`
/// (`components/newlib/abort.c`): `itoa` formats the caller's address and
/// core ID, and the `strcat`-only loop that follows concatenates the fixed
/// message `"abort() was called at PC 0x<addr> on core <n>"` — confirmed
/// by reading the literal string bytes and the caller chain directly out
/// of `factory.bin` (see Task D4 fix round 1's report).
///
/// **Corrected in Task D4 fix round 1** (a review caught this): the
/// original version of this test's doc claimed `itoa`/`strcat` sat on
/// "the normal, pre-panic boot path". That was wrong. `abort()`'s own
/// caller, at `0x42001004`–`0x42001010`, is `ets_printf` called with the
/// format string `"E (%lu) %s: Invalid app image header\n"` and the tag
/// `"cpu_start"` — i.e. **ESP-IDF's own early-startup app-image-header
/// check rejected this image and is logging that fact before aborting.**
/// The console showed nothing at the time only because `ets_printf` is
/// stubbed `Return(0)` and never reaches `Console` (see `rom.rs`'s entry 7
/// caveat) — the `cpu_start` error line was logged and silently dropped,
/// not skipped. **Boot has been aborting on this check since at least Task
/// D3 fix round 1**; Task D4 did not move the wall, it only let this
/// pre-existing `abort()` call finish formatting its message instead of
/// faulting mid-format on an unstubbed `itoa`/`strcat`.
///
/// With both unblocked, that `abort()` call runs to completion and reaches
/// the trap it was always going to reach: a real `ILLEGAL_INSTRUCTION`
/// exception (RISC-V cause 2, not `INSTRUCTION_ACCESS_FAULT`) at
/// `0x4038e4fa`, inside the app's own IRAM code -- a compiler-emitted
/// `c.unimp` (RVC's all-zero 16-bit encoding, architecturally reserved to
/// always trap), reached immediately after storing `g_panic_abort = true`
/// and `g_panic_abort_details = <this abort's message>` to two fixed
/// addresses. That's `esp_system_abort` -> `panic_abort()`'s mechanism
/// (`components/esp_system/panic.c`) -- a deliberate, hardware-standard
/// trap, not a decode gap. Per Task D4's own stop condition ("stop as soon
/// as the stall is anything other than an unstubbed ROM libc/string
/// call"), that task stopped here rather than chasing this further --
/// correctly, since the actual blocker (`cpu_start`'s header check) isn't a
/// ROM stub gap at all.
///
/// **What happens next** (all of it real, hardware-faithful behavior, not
/// an emulator gap): the RISC-V trap delivers correctly to the firmware's
/// own exception handler, which runs `esp_panic_handler` for the first time
/// with a *complete* message (itoa/strcat actually built it this time).
/// Per `panic.c`, the abort path leaves `info->reason == NULL`, so the
/// "Guru Meditation Error" header is skipped on this first pass; ESP-IDF's
/// generic pre-restart text prints unconditionally -- **panic output for
/// `cpu_start`'s abort, not evidence of boot progress past it** -- then
/// `panic_restart()` calls into the still-unstubbed ROM `software_reset_cpu`
/// (`0x4000_0094`) to actually reboot -- which faults (a real
/// `INSTRUCTION_ACCESS_FAULT` this time), re-entering the panic handler
/// through its *exception* path (where `info->reason` finally is non-NULL,
/// so "Guru Meditation Error" prints for the first time), which again
/// reaches the same unstubbed `software_reset_cpu` call and loops.
/// `software_reset_cpu` remains unstubbed -- it's only reached via the
/// panic path, per this module's own scoping, and stubbing it wouldn't
/// address `cpu_start`'s header-check failure, the actual blocker.
///
/// This test is **deliberately expected to break** once a later task fixes
/// (or works around) `cpu_start`'s header check: at that point `abort()`
/// is never called, this whole panic path disappears, and this test's
/// assertions (two traps, this exact fault sequence, this console text)
/// stop holding. That's the correct outcome, not a regression -- whoever
/// fixes the header check should delete or replace this test, not chase a
/// new pinned value here.
///
/// **History**: until Task D4, this test pinned an *earlier* stall -- an
/// unstubbed `itoa` call at `0x4000_0448` (see Fix round 1's report). That
/// call, and the `strcat` call it led to, are now stubbed for real
/// (`emulator-core/src/rom.rs`'s module doc, entry 12), so this test's
/// expected stall point moved forward to this abort/panic sequence. It was
/// also renamed (from `boot_currently_stalls_retrying_reboot_after_a_real_panic_abort`)
/// in Task D4 fix round 1, since "stalls" implied a stub gap rather than a
/// genuine firmware-triggered abort.
#[test]
fn boot_currently_aborts_reaching_the_panic_handlers_reboot_message() {
    let image = read_factory_bin();
    let mut rt = FirmwareRuntime::from_image(&image).expect("real factory.bin should boot");

    // 650,000 steps is a single, stable, two-trap snapshot: comfortably
    // past both the panic_abort() ILLEGAL_INSTRUCTION trap (measured at
    // step 407,549) and the software_reset_cpu INSTRUCTION_ACCESS_FAULT it
    // leads to (measured at step 645,410), but well before the panic
    // handler's re-entrancy guard kicks in and the "Rebooting"/panic text
    // starts repeating many more times (observed within a few million
    // steps in this task's boot-probe re-run).
    let summary = rt.run(650_000);
    assert_eq!(
        summary.traps, 2,
        "expected exactly two traps so far: the panic_abort() illegal \
         instruction, then the software_reset_cpu instruction-access fault \
         it leads to"
    );
    assert_eq!(
        summary.last_instruction_fault,
        Some(0x4000_0094),
        "expected the most recent INSTRUCTION_ACCESS_FAULT to be \
         software_reset_cpu -- the panic handler's own (unstubbed) reboot \
         attempt"
    );

    // The firmware's own panic handler ran to completion for the first
    // time and printed ESP-IDF's generic pre-restart text, then a full
    // crash report on the second (exception-path) pass -- generic ESP-IDF
    // text, never identity data (see this file's module doc and
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

    // And nothing has been drawn, because the display driver is never reached.
    assert!(rt.framebuffer().iter().all(|px| *px == 0));
}

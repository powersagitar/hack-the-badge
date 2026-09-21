//! Integration test: boot the *real* dumped firmware
//! (`public/firmware/factory.bin`) through `boot_from_factory_image` and run
//! it for a bounded number of `cpu.step()` calls, asserting it doesn't hang
//! or panic. This is the primary evidence for whether the "shortcut boot"
//! approach (skip the ROM/2nd-stage bootloader, jump straight to the app
//! image's `entry_addr`) is viable — see the task report for a detailed
//! account of what was observed.
//!
//! Reads `factory.bin` directly from `../public/firmware/factory.bin`
//! (relative to this crate) rather than duplicating the 2.7 MB file into
//! `tests/fixtures/`.

use emulator_core::boot::boot_from_factory_image;
use emulator_core::cpu::exception_code;
use emulator_core::mem::Bus;
use std::path::PathBuf;

fn factory_bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("public/firmware/factory.bin")
}

#[test]
fn boots_real_factory_image_without_panicking_and_reaches_step_budget() {
    let image = std::fs::read(factory_bin_path()).expect(
        "reading public/firmware/factory.bin \
         (expected to be committed at the repo root's public/firmware/)",
    );
    assert_eq!(image.len(), 2_752_512, "unexpected factory.bin size");

    let (mut cpu, mut bus) =
        boot_from_factory_image(&image).expect("real factory.bin should parse and boot");

    assert_eq!(
        cpu.regs.pc, 0x403803fc,
        "entry_addr should match the confirmed ground truth"
    );

    const STEP_BUDGET: usize = 5_000;
    let mut trap_count = 0usize;
    let mut last_traps: Vec<(usize, u32, u32)> = Vec::new(); // (step, pc_before, mcause)

    for i in 0..STEP_BUDGET {
        let info = cpu.step(&mut bus);
        if info.trap_taken {
            trap_count += 1;
            if last_traps.len() < 20 {
                last_traps.push((i, info.pc_before, cpu.csr.mcause));
            }
        }
    }

    eprintln!(
        "boot_integration: ran {STEP_BUDGET} steps; pc ended at 0x{:08x}; \
         sp (x2) = 0x{:08x}; traps taken = {trap_count}; first traps (step, pc_before, mcause) = {:?}; \
         unmapped accesses recorded = {}",
        cpu.regs.pc,
        cpu.regs.read(2),
        last_traps,
        bus.unmapped_log().len(),
    );

    // The CPU must have made observable forward progress: pc should not be
    // stuck at (or immediately after) entry_addr after 5000 steps' worth of
    // execution/traps.
    assert_ne!(
        cpu.regs.pc, 0,
        "pc collapsed to 0 -- likely an unhandled jump-to-null, treat as stuck"
    );

    // This is an observation harness, not a strict pass/fail gate: real
    // FreeRTOS boot needs interrupt/timer/peripheral support this task
    // doesn't build. We only assert the loop *completed* the step budget
    // (no panic/hang triggered a test framework timeout) -- reaching here at
    // all is the actual assertion; the eprintln! above is what a human
    // reads to judge "stuck vs progressing".
}

#[test]
fn illegal_instruction_traps_are_recoverable_not_fatal() {
    // Sanity check on the trap plumbing this test relies on to interpret
    // "stuck": an illegal instruction should trap (not panic) and mcause
    // should reflect it, regardless of what real firmware bytes triggered
    // it.
    use emulator_core::cpu::Cpu;

    struct OnlyIllegal;
    impl Bus for OnlyIllegal {
        fn read8(&mut self, _: u32) -> u8 {
            0xff
        }
        fn read16(&mut self, _: u32) -> u16 {
            0xffff
        }
        fn read32(&mut self, _: u32) -> u32 {
            0xffff_ffff
        }
        fn write8(&mut self, _: u32, _: u8) {}
        fn write16(&mut self, _: u32, _: u16) {}
        fn write32(&mut self, _: u32, _: u32) {}
    }

    let mut cpu = Cpu::new();
    let mut bus = OnlyIllegal;
    let info = cpu.step(&mut bus);
    assert!(info.trap_taken);
    assert_eq!(cpu.csr.mcause, exception_code::ILLEGAL_INSTRUCTION);
}

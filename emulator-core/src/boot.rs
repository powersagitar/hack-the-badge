//! "Shortcut boot": load a real ESP-IDF app image (as dumped from a
//! physical badge into `public/firmware/factory.bin`) directly into a
//! [`Cpu`]/[`FirmwareBus`] pair, skipping the ROM/2nd-stage-bootloader
//! entirely.
//!
//! Real ESP32-C3 boot is: mask ROM -> 2nd-stage bootloader (itself an
//! app image, reads partition table, decides which app partition to run) ->
//! jumps into *this* app image's `entry_addr` with the flash cache already
//! configured and a valid stack already set up. We don't model the mask
//! ROM or 2nd-stage bootloader at all (out of scope for this emulator) —
//! instead we parse this app image ourselves, set up the flash-mapped/
//! RAM-copied regions the 2nd-stage bootloader would have set up, and jump
//! straight to `entry_addr`, i.e. we start the emulated CPU at the exact
//! point real hardware would be at when the 2nd-stage bootloader hands off.

use std::sync::Arc;

use crate::cpu::Cpu;
use crate::mem::bus::FirmwareBus;
use crate::mem::image::{parse_image, ImageParseError};
use crate::mem::soc::DRAM_RANGE;

/// Size of the scratch stack/bss region reserved in DRAM for the CPU's
/// initial stack pointer (see [`boot_from_factory_image`]'s doc comment for
/// why this exists at all). 64 KiB is an arbitrary but generous choice for
/// early startup code; it is *not* derived from the real firmware's actual
/// `.bss`/stack size (we don't have that — no ELF/map file is committed,
/// only the flattened flash image), so it is exactly the kind of assumption
/// the task brief asked to flag rather than bury.
const SCRATCH_STACK_LEN: u32 = 64 * 1024;

/// Parses `image` (expected to be the raw bytes of an ESP-IDF app image,
/// e.g. `public/firmware/factory.bin`), builds the [`FirmwareBus`] memory
/// map from its segments, and returns a [`Cpu`] with `pc` set to the
/// image's `entry_addr` — ready for the caller to start calling
/// `cpu.step(&mut bus)`.
///
/// ## Stack pointer (x2)
///
/// Investigated empirically (see this crate's `tests/boot_integration.rs`
/// and the task report) rather than assumed: stepping the real firmware
/// from `entry_addr` shows its very first instruction is `c.addi sp, sp,
/// -32` — it decrements whatever `sp` already holds, never first loading an
/// absolute address into it. So (unlike the brief's anticipated "sets its
/// own SP from a linker symbol" case) this firmware's entry code *assumes*
/// `sp` was already set up by its caller — on real hardware, the 2nd-stage
/// bootloader, which this emulator doesn't model at all.
///
/// Per the brief's guidance for that case, we pick a placeholder: a fresh
/// [`SCRATCH_STACK_LEN`]-byte scratch RAM region is reserved immediately
/// after the highest-address DRAM segment the image actually carries bytes
/// for (mirroring where a real ESP-IDF linker script places `.bss`/heap/
/// stack — right after `.data`/`.rodata` in the same DRAM aperture, per
/// [`DRAM_RANGE`]), and `sp` is set to point at the top of it (stack grows
/// down). **This is a placeholder, not a derived-from-truth value**: we
/// don't have the real firmware's linker map, so we don't actually know its
/// true `.bss` size or where its intended stack/heap boundary falls within
/// DRAM — a later task that gets far enough to observe stack-relative
/// memory corruption should revisit this.
///
/// In the current shortcut-boot run, this placeholder has no observable
/// effect either way: execution reaches an unresolved call into on-chip
/// mask ROM (not part of this image, not modeled at all — see the report)
/// only ~20 instructions after entry, before any `sp`-relative load/store
/// occurs. It's set up now anyway so the CPU's register state isn't left in
/// an obviously-nonsensical state (`sp = 0xffff_ffe0`, from decrementing
/// `Cpu::new()`'s zeroed default) for whichever later task picks this back
/// up once ROM-call stubs exist.
pub fn boot_from_factory_image(image: &[u8]) -> Result<(Cpu, FirmwareBus), ImageParseError> {
    let parsed = parse_image(image)?;

    let flash: Arc<[u8]> = Arc::from(image.to_vec().into_boxed_slice());
    let mut bus = FirmwareBus::from_segments(flash, &parsed.segments);

    let mut cpu = Cpu::new();
    cpu.regs.pc = parsed.header.entry_addr;

    let dram_end = parsed
        .segments
        .iter()
        .filter(|s| DRAM_RANGE.contains(&s.load_addr))
        .map(|s| s.load_addr as u64 + s.len as u64)
        .max();
    if let Some(dram_end) = dram_end {
        // dram_end is always < DRAM_RANGE.end (segments were parsed from a
        // real image whose DRAM segments fit inside the SoC's DRAM
        // aperture), so this cast and the following subtraction can't
        // underflow/overflow in practice; still clamp defensively so a
        // pathological image can't push scratch_base past the aperture.
        let scratch_base = (dram_end as u32).min(DRAM_RANGE.end);
        let scratch_len = SCRATCH_STACK_LEN.min(DRAM_RANGE.end - scratch_base);
        if scratch_len > 0 {
            bus.add_scratch_ram(scratch_base, scratch_len as usize);
            cpu.regs.write(2, scratch_base + scratch_len);
        }
    }

    Ok((cpu, bus))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::Bus;

    fn build_synthetic_image(entry_addr: u32, segments: &[(u32, &[u8])]) -> Vec<u8> {
        let mut buf = vec![0u8; 24];
        buf[0] = 0xE9;
        buf[1] = segments.len() as u8;
        buf[4..8].copy_from_slice(&entry_addr.to_le_bytes());
        for (load_addr, data) in segments {
            buf.extend_from_slice(&load_addr.to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
            buf.extend_from_slice(data);
        }
        buf
    }

    #[test]
    fn boot_sets_pc_to_entry_and_wires_up_segments() {
        // A tiny synthetic image: one XIP (IROM) segment holding a NOP-ish
        // instruction at the entry point, one RAM (IRAM) segment.
        let code = [0x13, 0x00, 0x00, 0x00]; // addi x0, x0, 0 (NOP), 4 bytes
        let ram_data = [0xAAu8; 4];
        let image = build_synthetic_image(
            0x4200_0000,
            &[(0x4200_0000, &code), (0x4038_0000, &ram_data)],
        );

        let (mut cpu, mut bus) =
            boot_from_factory_image(&image).expect("synthetic image should parse and boot");

        assert_eq!(cpu.regs.pc, 0x4200_0000);
        // XIP: the code we baked into the image is readable at its address.
        assert_eq!(bus.read32(0x4200_0000), 0x0000_0013);
        // RAM: the copied segment bytes are present and mutable.
        assert_eq!(bus.read8(0x4038_0000), 0xAA);
        bus.write8(0x4038_0000, 0x55);
        assert_eq!(bus.read8(0x4038_0000), 0x55);

        // The CPU can actually execute starting from entry_addr.
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(cpu.regs.pc, 0x4200_0004);
    }

    #[test]
    fn boot_seeds_sp_above_highest_dram_segment_when_one_is_present() {
        let code = [0x13, 0x00, 0x00, 0x00]; // addi x0, x0, 0
        let dram_data = [0u8; 16];
        let image = build_synthetic_image(
            0x4200_0000,
            &[(0x4200_0000, &code), (0x3fc99c00, &dram_data)],
        );

        let (cpu, _bus) = boot_from_factory_image(&image).expect("should boot");

        let expected_scratch_base = 0x3fc99c00 + 16;
        assert_eq!(
            cpu.regs.read(2),
            expected_scratch_base + SCRATCH_STACK_LEN,
            "sp should point at the top of a scratch region placed right \
             after the highest DRAM segment"
        );
    }

    #[test]
    fn boot_leaves_sp_at_default_when_no_dram_segment_present() {
        // No DRAM-range segment at all -> nowhere principled to place a
        // scratch stack, so sp stays at Cpu::new()'s default.
        let code = [0x13, 0x00, 0x00, 0x00];
        let image = build_synthetic_image(0x4200_0000, &[(0x4200_0000, &code)]);
        let (cpu, _bus) = boot_from_factory_image(&image).expect("should boot");
        assert_eq!(cpu.regs.read(2), 0);
    }

    #[test]
    fn boot_propagates_parse_errors_instead_of_panicking() {
        let bad_image = [0x00u8; 24]; // wrong magic
        let result = boot_from_factory_image(&bad_image);
        assert!(matches!(
            result,
            Err(ImageParseError::BadMagic { found: 0x00 })
        ));
    }
}

//! "Shortcut boot": load a real ESP-IDF app image (as dumped from a
//! physical badge into `frontend/public/firmware/factory.bin`) directly into a
//! [`Cpu`]/[`FirmwareBus`] pair, skipping the ROM/2nd-stage-bootloader
//! entirely.
//!
//! Real ESP32-C3 boot is: mask ROM -> 2nd-stage bootloader (itself an
//! app image, reads partition table, decides which app partition to run) ->
//! jumps into *this* app image's `entry_addr` with the flash cache already
//! configured and a valid stack already set up. We don't model the mask
//! ROM or 2nd-stage bootloader at all (out of scope for this emulator) —
//! instead we parse this app image ourselves, set up the flash-mapped/
//! RAM-copied regions the 2nd-stage bootloader would have set up (as of
//! Milestone 3 Task D5, page-granular for the flash-mapped regions, not
//! just each segment's own declared bytes — see
//! [`crate::mem::bus::FirmwareBus::from_segments`]'s doc comment for why
//! that distinction is load-bearing), and jump straight to `entry_addr`,
//! i.e. we start the emulated CPU at the exact point real hardware would
//! be at when the 2nd-stage bootloader hands off.
//!
//! What the skipped steps leave in *RAM* for the app to read later is
//! re-created by [`apply_ram_initializers`] (Milestone 3 Task D10): a
//! generic list of boot-time `(address, bytes)` writes, with the ESP32-C3
//! data in [`crate::rom`] (applied by
//! [`boot_from_factory_image_with_rom_stubs`] only, since it is ROM state).

use std::sync::Arc;

use crate::cpu::Cpu;
use crate::mem::bus::FirmwareBus;
use crate::mem::image::{parse_image, ImageParseError};
use crate::mem::soc::{DRAM_RANGE, IRAM_RANGE, ROM_STACK_SIZE, ROM_STACK_START, RTC_RANGE};

/// Parses `image` (expected to be the raw bytes of an ESP-IDF app image,
/// e.g. `frontend/public/firmware/factory.bin`), builds the [`FirmwareBus`] memory
/// map from its segments, and returns a [`Cpu`] with `pc` set to the
/// image's `entry_addr` — ready for the caller to start calling
/// `cpu.step(&mut bus)`.
///
/// ## Uninitialized RAM (`.bss`, heap, stack)
///
/// An app image only carries bytes for the segments that need init data
/// (`.data`/`.rodata`/code). `.bss`, the heap and the stack occupy the *rest*
/// of the same on-chip SRAM apertures with nothing stored in flash for them,
/// so nothing in the segment table describes them. This function therefore
/// backs the **whole** of each RAM aperture — [`DRAM_RANGE`], [`IRAM_RANGE`]
/// and [`RTC_RANGE`] — with zeroed RAM, appended *after* the image's own
/// segments so that wherever the two overlap the image's real bytes win (see
/// [`FirmwareBus::add_scratch_ram`] and `FirmwareBus`'s first-match region
/// lookup).
///
/// This replaced an earlier, narrower placeholder (a single 64 KiB scratch
/// window starting just past the highest DRAM segment) that turned out to be
/// the direct cause of a boot failure worth recording, since it's exactly the
/// class of bug this layer can hide: the real firmware's `.bss` runs from
/// `0x3fca_87f0` to `0x3fcb_d180` (observed from its own
/// `memset(0x3fca87f0, 0, 0x14990)` during startup), which overran that
/// 64 KiB window by ~19 KiB. Writes to the overrun tail were silently dropped
/// by the never-panic MMIO catch-all and reads came back as `0`, so a FreeRTOS
/// critical-section counter never incremented and `vPortExitCritical`'s
/// `configASSERT(port_uxCriticalNesting[0] > 0)` fired ~250 instructions into
/// boot. Backing whole apertures removes that entire failure mode rather than
/// re-tuning a window size.
///
/// ## Stack pointer (x2)
///
/// Investigated empirically (see this crate's `tests/boot_integration.rs`
/// and the task report) rather than assumed: stepping the real firmware
/// from `entry_addr` shows its very first instruction is `c.addi sp, sp,
/// -32` — it decrements whatever `sp` already holds, never first loading an
/// absolute address into it. So this firmware's entry code *assumes* `sp` was
/// already set up by its caller — on real hardware, the 2nd-stage bootloader,
/// which this emulator doesn't model at all.
///
/// So `sp` is seeded here, to the highest DRAM address not inside the mask
/// ROM's own reserved stack window ([`ROM_STACK_START`]/[`ROM_STACK_SIZE`],
/// read from ESP-IDF's `soc.h`) — which is where a real bootloader's stack
/// sits when it hands over, and comfortably above where any plausible `.bss`
/// ends. The previous scheme put `sp` *inside* `.bss` (it derived the stack
/// from the last DRAM segment's end, which is precisely where `.bss` starts),
/// so early startup was overwriting its own static variables from below while
/// using them.
///
/// **Still an approximation, not ground truth**: we have no linker map for
/// this firmware (only the flattened flash image), so its intended
/// stack/heap boundary is unknown. What's asserted here is narrower and
/// checkable: this `sp` is inside real backed RAM, above `.bss`, and outside
/// the ROM's reserved window.
pub fn boot_from_factory_image(image: &[u8]) -> Result<(Cpu, FirmwareBus), ImageParseError> {
    let parsed = parse_image(image)?;

    let flash: Arc<[u8]> = Arc::from(image.to_vec().into_boxed_slice());
    let mut bus = FirmwareBus::from_segments(flash, &parsed.segments);

    for aperture in [&DRAM_RANGE, &IRAM_RANGE, &RTC_RANGE] {
        bus.add_scratch_ram(aperture.start, (aperture.end - aperture.start) as usize);
    }

    let mut cpu = Cpu::new();
    cpu.regs.pc = parsed.header.entry_addr;
    cpu.regs.write(2, initial_stack_pointer());

    Ok((cpu, bus))
}

/// The seeded initial `sp` — see [`boot_from_factory_image`]'s doc. Kept as a
/// named function so tests can assert the value without restating the
/// arithmetic.
pub fn initial_stack_pointer() -> u32 {
    // The ROM stack grows down from ROM_STACK_START, so its reserved window is
    // [START - SIZE, START); the first address below that window is the app's.
    // Masked to a 16-byte boundary, the RISC-V ABI's stack alignment.
    (ROM_STACK_START - ROM_STACK_SIZE) & !0xf
}

/// Same as [`boot_from_factory_image`], plus the ESP32-C3 mask-ROM
/// high-level-emulation stub table ([`crate::rom::esp32c3_rom_stubs`])
/// installed on the returned [`Cpu`], and the ESP32-C3's guest-executed ROM
/// code blobs ([`crate::rom::install_esp32c3_rom_code`] -- currently ROM
/// libc `qsort`, Milestone 3 Task D6) mapped onto the returned
/// [`FirmwareBus`], along with its read-only ROM data
/// ([`crate::rom::install_esp32c3_rom_data`] -- currently the ROM layout
/// table behind `ets_rom_layout_p`, Milestone 3 Task D7), and finally the
/// ROM's writable RAM state that the skipped mask-ROM reset and 2nd-stage
/// bootloader would have left behind ([`apply_ram_initializers`] with
/// [`crate::rom::esp32c3_rom_ram_initializers`] -- currently the SPI-flash
/// legacy data, Milestone 3 Task D10; its `chip_size` comes from `image`'s
/// own header). Stubs cover routines whose effect can be applied atomically,
/// real guest code covers the ones that must call back into firmware, ROM
/// data covers read-only tables the firmware reads, and RAM initializers
/// cover ROM `.data` the firmware reads and writes (see [`crate::rom`]'s
/// module doc, entries 14, 15 and 20).
///
/// ## Why this is a separate entry point rather than the default
///
/// Real ESP-IDF firmware calls fixed-address on-chip mask-ROM functions
/// within its first ~20 instructions (see [`crate::rom`] and
/// [`crate::cpu::rom_stubs`] for the whole story), so *any* attempt to
/// actually run `factory.bin` needs these stubs. But installing them changes
/// observable behavior — most obviously, boot no longer stops at the
/// `INSTRUCTION_ACCESS_FAULT` that Task 2.1's plain-boot integration test
/// exists specifically to pin down. Keeping [`boot_from_factory_image`]
/// stub-free preserves that test (and every other pre-existing one) exactly
/// as written, and makes "are ROM stubs in play?" an explicit property of the
/// call site rather than a hidden global.
///
/// This is the entry point `crate::runtime::FirmwareRuntime` (and therefore
/// the browser's real-firmware mode) uses.
pub fn boot_from_factory_image_with_rom_stubs(
    image: &[u8],
) -> Result<(Cpu, FirmwareBus), ImageParseError> {
    let (mut cpu, mut bus) = boot_from_factory_image(image)?;
    cpu.set_rom_stubs(crate::rom::esp32c3_rom_stubs());
    crate::rom::install_esp32c3_rom_code(&mut bus);
    crate::rom::install_esp32c3_rom_data(&mut bus);
    // Already parsed successfully above; re-parsing just the header is cheap.
    let header = parse_image(image)?.header;
    apply_ram_initializers(&mut bus, &crate::rom::esp32c3_rom_ram_initializers(&header));
    Ok((cpu, bus))
}

/// One boot-time RAM write: `bytes` stored at guest address `addr` (and up)
/// before the first instruction runs. See [`apply_ram_initializers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RamInitializer {
    /// Guest address of `bytes[0]`.
    pub addr: u32,
    /// The bytes to store, in address order.
    pub bytes: Vec<u8>,
}

/// Applies `inits` to `bus`, in order (a later entry overwrites an earlier
/// one where they overlap), by ordinary byte writes through the bus.
///
/// ## Why this exists
///
/// The shortcut boot skips the mask ROM's reset code and the 2nd-stage
/// bootloader (see this module's doc). Some of what those skipped steps
/// leave in **RAM** is state the app later reads: the mask ROM copies its
/// own writable `.data` initializers into DRAM, and the bootloader calls ROM
/// functions that update that `.data`. This is the generic stand-in: a list
/// of `(address, bytes)` writes applied to already-backed emulated RAM right
/// after the apertures are backed. It holds no chip knowledge; the ESP32-C3
/// data and its citations live in `crate::rom`
/// ([`crate::rom::esp32c3_rom_ram_initializers`]).
///
/// It differs from `FirmwareBus::map_rom_data` (Task D7): that maps
/// *read-only* ROM data in the ROM's own address range, whereas these
/// writes land in ordinary writable RAM that the firmware keeps updating.
///
/// A write that lands outside backed RAM is dropped by the bus's never-panic
/// catch-all (and logged as unmapped) like any other stray write.
pub fn apply_ram_initializers(bus: &mut FirmwareBus, inits: &[RamInitializer]) {
    use crate::mem::Bus;
    for init in inits {
        for (i, byte) in init.bytes.iter().enumerate() {
            bus.write8(init.addr.wrapping_add(i as u32), *byte);
        }
    }
}

/// Samples the interrupt lines currently asserted to the core
/// ([`FirmwareBus::asserted_lines`]) into the CPU, steps `cpu` once, then
/// advances [`FirmwareBus`]'s peripheral time
/// ([`FirmwareBus::tick_peripherals`]: the SYSTIMER's counter) exactly once.
///
/// This is the interrupt-delivery driving loop (interrupt matrix + SYSTIMER
/// + SYSTEM software interrupts, see `crate::peripherals`). Callers that
/// want peripherals (and therefore interrupts) to actually work should call
/// this instead of `cpu.step(&mut bus)` directly.
///
/// ## Level delivery, sampled at the start of each step
///
/// The pending-line set is **replaced** every step
/// ([`Cpu::set_pending_interrupts`]), never OR-accumulated: interrupt
/// sources on the ESP32-C3 are levels, so once an ISR clears its source
/// (SYSTIMER `INT_CLR`, a `0` to `SYSTEM_CPU_INTR_FROM_CPU_n_REG`) or a
/// critical section raises `CPU_INT_THRESH`, the line must stop being
/// pending instead of being re-taken from a stale sticky bit.
///
/// Sampling *before* the step (rather than feeding the step's own
/// post-tick mask forward) means state changed between steps from outside
/// the guest — a test's "ISR", a host button press — is seen by the very
/// next step. For changes the guest makes itself, the timing is identical
/// either way: a store at step `N`, or an alarm reached in the tick after
/// step `N`, is taken at step `N + 1` (pinned by
/// `tests/interrupt_integration.rs`'s precise-step test).
pub fn step_with_interrupts(cpu: &mut Cpu, bus: &mut FirmwareBus) -> crate::cpu::StepInfo {
    cpu.set_pending_interrupts(bus.asserted_lines());
    let info = cpu.step(bus);
    bus.tick_peripherals();
    info
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
    fn boot_seeds_sp_just_below_the_roms_reserved_stack_window() {
        let code = [0x13, 0x00, 0x00, 0x00]; // addi x0, x0, 0
        let dram_data = [0u8; 16];
        let image = build_synthetic_image(
            0x4200_0000,
            &[(0x4200_0000, &code), (0x3fc99c00, &dram_data)],
        );

        let (cpu, mut bus) = boot_from_factory_image(&image).expect("should boot");

        let sp = cpu.regs.read(2);
        assert_eq!(sp, initial_stack_pointer());
        assert_eq!(sp, ROM_STACK_START - ROM_STACK_SIZE);
        assert_eq!(sp % 16, 0, "RISC-V ABI requires 16-byte stack alignment");
        assert!(
            DRAM_RANGE.contains(&sp),
            "sp must land inside the DRAM aperture"
        );
        assert!(
            sp > 0x3fc99c00 + 16,
            "sp must be above the image's own DRAM data, not inside it"
        );

        // And it points at *real* RAM: a push/pop round-trips rather than
        // being swallowed by the never-panic MMIO catch-all.
        bus.write32(sp - 4, 0xc0ffee00);
        assert_eq!(bus.read32(sp - 4), 0xc0ffee00);
    }

    #[test]
    fn boot_backs_every_ram_aperture_even_where_the_image_carries_no_bytes() {
        // The regression this guards: `.bss`/heap/stack have no segment in the
        // image, so an address just past the last DRAM segment used to be
        // unbacked -- writes dropped, reads 0. See this module's doc.
        let code = [0x13, 0x00, 0x00, 0x00];
        let dram_data = [0u8; 16];
        let image = build_synthetic_image(
            0x4200_0000,
            &[(0x4200_0000, &code), (0x3fc99c00, &dram_data)],
        );
        let (_cpu, mut bus) = boot_from_factory_image(&image).expect("should boot");

        for addr in [
            DRAM_RANGE.start,
            0x3fc99c00 + 16, // immediately past the image's DRAM segment
            DRAM_RANGE.end - 4,
            IRAM_RANGE.start,
            IRAM_RANGE.end - 4,
            RTC_RANGE.start,
            RTC_RANGE.end - 4,
        ] {
            bus.write32(addr, 0xa5a5_5a5a);
            assert_eq!(
                bus.read32(addr),
                0xa5a5_5a5a,
                "0x{addr:08x} should be real read/write RAM"
            );
        }

        // ...and the image's own bytes still win where the two overlap.
        assert_eq!(bus.read8(0x3fc99c00 + 15), 0);
        bus.write8(0x3fc99c00 + 15, 0x77);
        assert_eq!(bus.read8(0x3fc99c00 + 15), 0x77);
    }

    #[test]
    fn boot_seeds_sp_even_when_the_image_has_no_dram_segment() {
        // The seeded sp comes from the SoC's documented memory map, not from
        // the image's segment table, so an image with no DRAM segment at all
        // still gets a usable stack.
        let code = [0x13, 0x00, 0x00, 0x00];
        let image = build_synthetic_image(0x4200_0000, &[(0x4200_0000, &code)]);
        let (cpu, _bus) = boot_from_factory_image(&image).expect("should boot");
        assert_eq!(cpu.regs.read(2), initial_stack_pointer());
    }

    #[test]
    fn ram_initializers_are_written_in_order_into_backed_ram() {
        let code = [0x13, 0x00, 0x00, 0x00];
        let image = build_synthetic_image(0x4200_0000, &[(0x4200_0000, &code)]);
        let (_cpu, mut bus) = boot_from_factory_image(&image).expect("should boot");

        apply_ram_initializers(
            &mut bus,
            &[
                RamInitializer {
                    addr: 0x3fcd_f5c0,
                    bytes: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
                },
                // Overlaps the first one's second word: applied later, so it
                // wins there.
                RamInitializer {
                    addr: 0x3fcd_f5c4,
                    bytes: 0xdead_beefu32.to_le_bytes().to_vec(),
                },
                RamInitializer {
                    addr: 0x3fcd_fff0,
                    bytes: 0x3fcd_f5c0u32.to_le_bytes().to_vec(),
                },
            ],
        );

        assert_eq!(bus.read32(0x3fcd_f5c0), 0x4433_2211);
        assert_eq!(bus.read32(0x3fcd_f5c4), 0xdead_beef);
        assert_eq!(bus.read32(0x3fcd_fff0), 0x3fcd_f5c0);
        // Still ordinary writable RAM afterwards.
        bus.write32(0x3fcd_f5c4, 0x0046_4016);
        assert_eq!(bus.read32(0x3fcd_f5c4), 0x0046_4016);
    }

    #[test]
    fn rom_stub_boot_seeds_the_rom_spiflash_legacy_data_before_the_first_instruction() {
        let code = [0x13, 0x00, 0x00, 0x00];
        let mut image = build_synthetic_image(0x4200_0000, &[(0x4200_0000, &code)]);
        image[3] = 0x2f; // factory.bin's spi_speed_size: 4 MB flash
        let (_cpu, mut bus) = boot_from_factory_image_with_rom_stubs(&image).expect("should boot");

        // rom_spiflash_legacy_data -> rom_default_spiflash_legacy_data
        assert_eq!(bus.read32(0x3fcd_fff0), 0x3fcd_f5c0);
        // g_rom_flashchip, as the bootloader's config_param left it.
        let chip: Vec<u32> = (0..7).map(|i| bus.read32(0x3fcd_f5c0 + 4 * i)).collect();
        assert_eq!(
            chip,
            [0x0046_4016, 0x0040_0000, 0x1_0000, 0x1000, 0x100, 0xffff, 0]
        );

        // The plain (ROM-less) boot models no ROM, so seeds nothing.
        let (_cpu, mut plain) = boot_from_factory_image(&image).expect("should boot");
        assert_eq!(plain.read32(0x3fcd_fff0), 0);
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

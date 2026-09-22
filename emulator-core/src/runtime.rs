//! [`FirmwareRuntime`]: the whole real-firmware emulator as one owned,
//! driveable object — a [`Cpu`] with the ESP32-C3 mask-ROM stub table
//! installed, its [`FirmwareBus`], the flash image needed to rebuild both on
//! reset, and the small amount of bookkeeping a UI wants (how far did we get,
//! what's on the screen, which buttons are held).
//!
//! This lives in `emulator-core` rather than in `emulator-wasm` on purpose:
//! everything here is plain Rust and `cargo test`-able natively, so
//! `emulator-wasm` stays the thin wasm-bindgen shim `CLAUDE.md`'s
//! architecture section describes, with no logic of its own to test through a
//! WASM round-trip.
//!
//! ## Raw button slots
//!
//! Buttons are addressed by **raw slot index**, deliberately *not* by a human
//! button name: `emulator-core` has no opinion about which physical badge
//! button sits on which shift-register bit (Task 4 established the 74HC165
//! model with generically-numbered slots for exactly this reason — see
//! `crate::peripherals::gpio`'s module doc). The name→slot mapping is a
//! UI-layer concern and lives in `src/runtime/firmware-runtime.ts`.
//!
//! Slot numbering here matches that module's split:
//! - **slot 0** is `PIN_START` (GPIO9), a direct GPIO line, not part of the
//!   shift register at all.
//! - **slots 1..=8** are the shift register's 8 button bits, i.e. slot `n` is
//!   [`crate::peripherals::gpio::Hc165::set_button`]'s index `n - 1`.
//!
//! See [`FirmwareRuntime::set_raw_button`]. [`NUM_RAW_BUTTON_SLOTS`] is the
//! total (9) — which is exactly the number of names in `src/badge/input.ts`'s
//! `BUTTON_NAMES`, so the UI-side table is a total mapping with nothing left
//! over.

use std::sync::Arc;

use crate::boot::{boot_from_factory_image_with_rom_stubs, step_with_interrupts};
use crate::cpu::{exception_code, Cpu};
use crate::mem::bus::FirmwareBus;
use crate::mem::image::ImageParseError;
use crate::peripherals::spi::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Total number of raw button input slots — 1 direct GPIO (`PIN_START`) plus
/// the 74HC165's [`crate::peripherals::gpio::HC165_SLOTS`] button bits. See
/// the module doc.
pub const NUM_RAW_BUTTON_SLOTS: usize = 1 + crate::peripherals::gpio::HC165_SLOTS;

/// What one [`FirmwareRuntime::run`] call did. Plain `Copy` data so the WASM
/// shim can hand each field across the boundary as a scalar without needing
/// its own serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunSummary {
    /// How many `step()` calls this run made (equal to the requested budget;
    /// this runtime has no halt state — even a firmware stuck in a trap loop
    /// keeps consuming steps).
    pub steps: u32,
    /// How many of those steps took a trap instead of completing an
    /// instruction.
    pub traps: u32,
    /// How many of those steps intercepted a mask-ROM HLE stub (see
    /// `crate::rom`).
    pub rom_stub_calls: u32,
    /// `pc` after the last step of this run.
    pub pc: u32,
    /// `mtval` of the most recent `INSTRUCTION_ACCESS_FAULT` taken during
    /// this run, if any — i.e. the address execution ran off to that isn't
    /// mapped and isn't a known ROM stub. The single most useful "why is boot
    /// stuck" signal, so it's surfaced rather than left to a log scrape.
    pub last_instruction_fault: Option<u32>,
}

/// The real-firmware emulator: CPU + bus + the image they were built from.
pub struct FirmwareRuntime {
    image: Arc<[u8]>,
    cpu: Cpu,
    bus: FirmwareBus,
    /// Held state per raw slot, retained across [`FirmwareRuntime::reset`] so
    /// a reset doesn't strand a physically-held button in the "down" state.
    buttons: [bool; NUM_RAW_BUTTON_SLOTS],
    total_steps: u64,
}

impl FirmwareRuntime {
    /// Boots `image` (the raw bytes of an ESP-IDF app image, i.e.
    /// `public/firmware/factory.bin`) through
    /// [`boot_from_factory_image_with_rom_stubs`].
    pub fn from_image(image: &[u8]) -> Result<Self, ImageParseError> {
        let image: Arc<[u8]> = Arc::from(image.to_vec().into_boxed_slice());
        let (cpu, bus) = boot_from_factory_image_with_rom_stubs(&image)?;
        Ok(Self {
            image,
            cpu,
            bus,
            buttons: [false; NUM_RAW_BUTTON_SLOTS],
            total_steps: 0,
        })
    }

    /// Re-boots from the same image: a fresh `Cpu`/`FirmwareBus` pair (so RAM,
    /// peripheral state and the reconstructed framebuffer all start over),
    /// with the currently-held buttons re-applied.
    ///
    /// Can only fail if the image stopped parsing, which can't happen for an
    /// image that already parsed once — but the error is propagated rather
    /// than unwrapped so this stays panic-free across the WASM boundary.
    pub fn reset(&mut self) -> Result<(), ImageParseError> {
        let (cpu, bus) = boot_from_factory_image_with_rom_stubs(&self.image)?;
        self.cpu = cpu;
        self.bus = bus;
        self.total_steps = 0;
        let held = self.buttons;
        for (slot, pressed) in held.iter().enumerate() {
            self.apply_button(slot, *pressed);
        }
        Ok(())
    }

    /// Runs up to `budget` instruction steps, delivering peripheral
    /// interrupts along the way (via
    /// [`crate::boot::step_with_interrupts`] — the correct one-poll-per-step
    /// cadence, see its doc).
    pub fn run(&mut self, budget: u32) -> RunSummary {
        let mut summary = RunSummary {
            steps: budget,
            ..RunSummary::default()
        };
        for _ in 0..budget {
            let info = step_with_interrupts(&mut self.cpu, &mut self.bus);
            if info.trap_taken {
                summary.traps += 1;
                if self.cpu.csr.mcause == exception_code::INSTRUCTION_ACCESS_FAULT {
                    summary.last_instruction_fault = Some(self.cpu.csr.mtval);
                }
            }
            if info.rom_stub.is_some() {
                summary.rom_stub_calls += 1;
            }
        }
        self.total_steps += u64::from(budget);
        summary.pc = self.cpu.regs.pc;
        summary
    }

    /// Presses/releases raw button `slot` (`0..`[`NUM_RAW_BUTTON_SLOTS`]).
    /// Out-of-range slots are ignored. See the module doc for the slot
    /// numbering.
    pub fn set_raw_button(&mut self, slot: usize, pressed: bool) {
        if slot >= NUM_RAW_BUTTON_SLOTS {
            return;
        }
        self.buttons[slot] = pressed;
        self.apply_button(slot, pressed);
    }

    fn apply_button(&mut self, slot: usize, pressed: bool) {
        match slot {
            0 => self.bus.gpio.set_start_pressed(pressed),
            n if n < NUM_RAW_BUTTON_SLOTS => self.bus.gpio.hc165.set_button(n - 1, pressed),
            _ => {}
        }
    }

    /// `true` if raw button `slot` is currently held.
    pub fn raw_button(&self, slot: usize) -> bool {
        self.buttons.get(slot).copied().unwrap_or(false)
    }

    /// The reconstructed ST7789 framebuffer: RGB565, row-major,
    /// [`SCREEN_WIDTH`]`*`[`SCREEN_HEIGHT`] elements, index `y * width + x`.
    /// Borrowed, not copied — the WASM shim is what decides how to hand it to
    /// JS (see `emulator-wasm`).
    pub fn framebuffer(&self) -> &[u16] {
        self.bus.spi.framebuffer()
    }

    pub fn screen_width(&self) -> usize {
        SCREEN_WIDTH
    }

    pub fn screen_height(&self) -> usize {
        SCREEN_HEIGHT
    }

    /// Current `pc`, for diagnostics/HUDs.
    pub fn pc(&self) -> u32 {
        self.cpu.regs.pc
    }

    /// Total steps run since construction or the last [`FirmwareRuntime::reset`].
    pub fn total_steps(&self) -> u64 {
        self.total_steps
    }

    /// Read-only access to the CPU, for tests and diagnostics.
    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Read-only access to the bus, for tests and diagnostics (the unmapped-
    /// access log in particular).
    pub fn bus(&self) -> &FirmwareBus {
        &self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ESP-IDF-app-image-shaped blob: one IROM (XIP) segment whose
    /// entry instruction is an endless self-branch, so `run()` can be called
    /// without the CPU wandering anywhere. Mirrors
    /// `crate::boot`'s own test helper.
    fn synthetic_image() -> Vec<u8> {
        let mut buf = vec![0u8; 24];
        buf[0] = 0xE9;
        buf[1] = 1; // one segment
        buf[4..8].copy_from_slice(&0x4200_0000u32.to_le_bytes());
        buf.extend_from_slice(&0x4200_0000u32.to_le_bytes());
        // `c.j 0` (0xa001) twice: a 2-byte self-loop, padded to 4 bytes so the
        // segment covers both halfwords the fetcher may look at.
        let code: [u8; 4] = [0x01, 0xa0, 0x01, 0xa0];
        buf.extend_from_slice(&(code.len() as u32).to_le_bytes());
        buf.extend_from_slice(&code);
        buf
    }

    #[test]
    fn runtime_boots_a_synthetic_image_and_runs_a_budget() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        assert_eq!(rt.pc(), 0x4200_0000);
        let summary = rt.run(100);
        assert_eq!(summary.steps, 100);
        assert_eq!(summary.traps, 0, "a self-branch must not trap");
        assert_eq!(summary.last_instruction_fault, None);
        assert_eq!(summary.pc, 0x4200_0000, "still spinning on itself");
        assert_eq!(rt.total_steps(), 100);
    }

    #[test]
    fn runtime_installs_the_rom_stub_table() {
        let rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        assert!(
            !rt.cpu().rom_stubs().is_empty(),
            "real-firmware mode must boot with mask-ROM stubs installed"
        );
        assert!(rt
            .cpu()
            .rom_stubs()
            .lookup(crate::rom::RTC_GET_RESET_REASON)
            .is_some());
    }

    #[test]
    fn framebuffer_has_the_documented_shape_and_starts_blank() {
        let rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        assert_eq!(rt.screen_width(), 320);
        assert_eq!(rt.screen_height(), 240);
        assert_eq!(rt.framebuffer().len(), 320 * 240);
        assert!(rt.framebuffer().iter().all(|px| *px == 0));
    }

    #[test]
    fn raw_button_slot_0_drives_pin_start_and_slots_1_to_8_drive_the_shift_register() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        use crate::peripherals::gpio::PIN_START;

        assert!(
            rt.bus().gpio.pin_level(PIN_START),
            "released -> active-low line idles high"
        );
        rt.set_raw_button(0, true);
        assert!(rt.raw_button(0));
        assert!(
            !rt.bus().gpio.pin_level(PIN_START),
            "slot 0 must pull PIN_START low"
        );
        rt.set_raw_button(0, false);
        assert!(rt.bus().gpio.pin_level(PIN_START));

        // Slot 1 maps to Hc165 index 0, which is the first bit presented
        // after a LOAD pulse. Drive the latch directly through the GPIO
        // registers the way firmware would.
        rt.set_raw_button(1, true);
        pulse_load(&mut rt);
        assert!(
            !rt.bus()
                .gpio
                .pin_level(crate::peripherals::gpio::PIN_HC165_DATA),
            "slot 1 (Hc165 index 0) must be the bit presented right after LOAD"
        );
    }

    #[test]
    fn the_last_raw_button_slot_reaches_the_shift_registers_last_bit() {
        // Guards the off-by-one in the slot->Hc165-index shift: slot
        // NUM_RAW_BUTTON_SLOTS-1 must land on Hc165 index HC165_SLOTS-1, the
        // 8th presented bit (AUX1 on real hardware).
        use crate::peripherals::gpio::PIN_HC165_DATA;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        rt.set_raw_button(NUM_RAW_BUTTON_SLOTS - 1, true);
        pulse_load(&mut rt);

        for i in 0..crate::peripherals::gpio::HC165_SLOTS {
            let pressed = !rt.bus().gpio.pin_level(PIN_HC165_DATA);
            assert_eq!(
                pressed,
                i == crate::peripherals::gpio::HC165_SLOTS - 1,
                "only the last presented bit should read pressed (bit {i})"
            );
            clk_rising_edge(&mut rt);
        }
    }

    #[test]
    fn out_of_range_button_slots_are_ignored() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        rt.set_raw_button(NUM_RAW_BUTTON_SLOTS, true); // must not panic
        rt.set_raw_button(9999, true);
        assert!(!rt.raw_button(NUM_RAW_BUTTON_SLOTS));
    }

    #[test]
    fn reset_rewinds_execution_but_keeps_held_buttons() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        rt.set_raw_button(0, true);
        rt.run(50);
        assert_eq!(rt.total_steps(), 50);

        rt.reset()
            .expect("reset should re-parse the retained image");
        assert_eq!(rt.total_steps(), 0);
        assert_eq!(rt.pc(), 0x4200_0000);
        assert!(rt.raw_button(0), "held buttons survive a reset");
        assert!(
            !rt.bus().gpio.pin_level(crate::peripherals::gpio::PIN_START),
            "and are re-applied to the freshly-built bus"
        );
    }

    #[test]
    fn from_image_propagates_a_parse_error_instead_of_panicking() {
        match FirmwareRuntime::from_image(&[0u8; 24]) {
            Err(ImageParseError::BadMagic { found: 0 }) => {}
            Err(other) => panic!("unexpected parse error: {other:?}"),
            Ok(_) => panic!("an all-zero blob must not parse as an app image"),
        }
    }

    /// Bit-bangs a 74HC165 `LOAD` pulse through the real GPIO registers, the
    /// way firmware would: configure GPIO20/21 as outputs, park `LOAD` at its
    /// idle-high level, then pulse it low and back high (the rising edge is
    /// what latches — see `crate::peripherals::gpio`'s module doc).
    fn pulse_load(rt: &mut FirmwareRuntime) {
        use crate::mem::soc::GPIO_RANGE;
        use crate::mem::Bus;
        use crate::peripherals::gpio::{ENABLE_REG, OUT_REG, PIN_HC165_CLK, PIN_HC165_LOAD};
        let load_bit = 1u32 << PIN_HC165_LOAD;
        let enable = load_bit | (1u32 << PIN_HC165_CLK);
        rt.bus.write32(GPIO_RANGE.start + ENABLE_REG, enable);
        rt.bus.write32(GPIO_RANGE.start + OUT_REG, load_bit); // idle high
        rt.bus.write32(GPIO_RANGE.start + OUT_REG, 0); // LOAD asserted low
        rt.bus.write32(GPIO_RANGE.start + OUT_REG, load_bit); // rising edge: latch
    }

    /// Clocks the shift register on by one bit, same bit-banging route.
    fn clk_rising_edge(rt: &mut FirmwareRuntime) {
        use crate::mem::soc::GPIO_RANGE;
        use crate::mem::Bus;
        use crate::peripherals::gpio::{OUT_REG, PIN_HC165_CLK, PIN_HC165_LOAD};
        let load_bit = 1u32 << PIN_HC165_LOAD;
        let clk_bit = 1u32 << PIN_HC165_CLK;
        rt.bus.write32(GPIO_RANGE.start + OUT_REG, load_bit);
        rt.bus
            .write32(GPIO_RANGE.start + OUT_REG, load_bit | clk_bit);
    }
}

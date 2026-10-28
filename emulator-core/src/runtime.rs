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
//! UI-layer concern and lives in `frontend/src/runtime/firmware-runtime.ts`.
//!
//! Slot numbering here matches that module's split:
//! - **slot 0** is `PIN_START` (GPIO9), a direct GPIO line, not part of the
//!   shift register at all.
//! - **slots 1..=8** are the shift register's 8 button bits, i.e. slot `n` is
//!   [`crate::peripherals::gpio::Hc165::set_button`]'s index `n - 1`.
//!
//! See [`FirmwareRuntime::set_raw_button`]. [`NUM_RAW_BUTTON_SLOTS`] is the
//! total (9) — which is exactly the number of names in `frontend/src/badge/input.ts`'s
//! `BUTTON_NAMES`, so the UI-side table is a total mapping with nothing left
//! over.
//!
//! ## Other host input (Milestone 5)
//!
//! - [`FirmwareRuntime::serial_input`] queues bytes a USB host sends to the
//!   badge's console (USB-Serial-JTAG OUT, paced 64-byte packets; see
//!   `crate::peripherals::usb_serial_jtag`); [`FirmwareRuntime::serial_pending`]
//!   reports what the firmware has not read yet. Provisioning types the
//!   firmware's own `put` / `prov apply` commands through it.
//! - [`FirmwareRuntime::set_acceleration`] sets what the SC7A20H
//!   accelerometer on I2C0 reports (`crate::peripherals::sc7a20h`); the
//!   onboarding app's "Shake it!" page needs a shake.
//!
//! ## Software reset (Milestone 9)
//!
//! v392 opens some apps by rebooting into them: `esp_restart()` ends in the
//! mask-ROM `software_reset_cpu` (`crate::rom`, entry 26), whose stub
//! requests a reset (`crate::cpu::rom_stubs::RomStubEffect::RequestReset`).
//! The run loop collects the request after the step that made it
//! ([`crate::cpu::Cpu::take_reset_request`]) and re-boots the same image
//! there, so a run's step budget stays exact. The new boot is a fresh
//! [`boot_from_firmware_image_with_rom_stubs`] with
//! `crate::rom::esp32c3_rom_stubs_with_reset_reason`, so
//! `rtc_get_reset_reason` reports the reset's reason, plus what a reset
//! keeps on the badge:
//!
//! - **the emulated flash, whole** (the identity, `reboot.cfg`);
//! - **RTC_CNTL `STORE0..7`** (`crate::peripherals::rtc_cntl`; ESP-IDF's
//!   reset-reason hint is `STORE6`, `RTC_RESET_CAUSE_REG`, per
//!   `components/esp_system/port/soc/esp32c3/reset_reason.c`);
//! - **RTC fast memory** (`crate::mem::soc::RTC_RANGE`), with the image's
//!   own RTC segments reloaded over it, as the bootloader would;
//! - **the panel** (`crate::peripherals::spi::St7789`): an external chip
//!   the MCU reset does not touch, so its pixels stay until the firmware
//!   redraws (any command byte clears its transient parameter state);
//! - **the console's output text**; queued host serial input is dropped
//!   with the old USB-Serial-JTAG, as [`FirmwareRuntime::reset`] does;
//! - **held buttons and the host-set acceleration**, re-applied;
//! - **[`FirmwareRuntime::total_steps`] and host time:**
//!   [`FirmwareRuntime::emulated_time_us`] is the time accumulated before
//!   the reset plus the new clock's, so it never goes backwards and a
//!   [`FirmwareRuntime::run_until`] across a reset still stops at its
//!   target.
//!
//! [`FirmwareRuntime::soft_reset_count`] counts them. The host's own
//! [`FirmwareRuntime::reset`] stays a cold power-on.
//!
//! ## Checkpoints
//!
//! A `FirmwareRuntime` is `Clone`, and a clone is a complete, independent
//! snapshot (CPU, bus, every peripheral, the flash chip; the image is
//! shared read-only). The boot rungs reach a shared state once and continue
//! from clones (`tests/boot_progress.rs`); pinned by
//! `tests::a_clone_runs_exactly_like_its_original`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::boot::{boot_from_firmware_image_with_rom_stubs, idle_until, step_with_interrupts};
use crate::cpu::{exception_code, Cpu};
use crate::mem::bus::FirmwareBus;
use crate::mem::image::ImageParseError;
use crate::peripherals::spi::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Total number of raw button input slots — 1 direct GPIO (`PIN_START`) plus
/// the 74HC165's [`crate::peripherals::gpio::HC165_SLOTS`] button bits. See
/// the module doc.
pub const NUM_RAW_BUTTON_SLOTS: usize = 1 + crate::peripherals::gpio::HC165_SLOTS;

/// What one [`FirmwareRuntime::run`], [`FirmwareRuntime::run_until`] or
/// [`FirmwareRuntime::run_traced`] call did. Plain `Copy` data so the WASM
/// shim can hand each field across the boundary as a scalar without needing
/// its own serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunSummary {
    /// How many `step()` calls this run made. For [`FirmwareRuntime::run`]
    /// and [`FirmwareRuntime::run_traced`] this equals the requested budget
    /// (this runtime has no halt state — even a firmware stuck in a trap
    /// loop keeps consuming steps, and so does a core parked in `WFI`: each
    /// waiting step counts once); for
    /// [`FirmwareRuntime::run_until`] it can be fewer, when the target time
    /// is reached first.
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
#[derive(Clone)]
pub struct FirmwareRuntime {
    image: Arc<[u8]>,
    cpu: Cpu,
    bus: FirmwareBus,
    /// Held state per raw slot, retained across [`FirmwareRuntime::reset`] so
    /// a reset doesn't strand a physically-held button in the "down" state.
    buttons: [bool; NUM_RAW_BUTTON_SLOTS],
    total_steps: u64,
    /// Software resets since construction or the last cold reset (module
    /// doc, "Software reset").
    soft_reset_count: u32,
    /// Emulated time, in microseconds, accumulated by the clocks that
    /// software resets replaced; [`FirmwareRuntime::emulated_time_us`] adds
    /// the current clock's.
    time_base_us: u64,
}

impl FirmwareRuntime {
    /// Boots `image` (the raw bytes of an ESP-IDF app image, i.e.
    /// `frontend/public/firmware/firmware.bin`) through
    /// [`boot_from_firmware_image_with_rom_stubs`].
    pub fn from_image(image: &[u8]) -> Result<Self, ImageParseError> {
        let image: Arc<[u8]> = Arc::from(image.to_vec().into_boxed_slice());
        let (cpu, bus) = boot_from_firmware_image_with_rom_stubs(&image)?;
        Ok(Self {
            image,
            cpu,
            bus,
            buttons: [false; NUM_RAW_BUTTON_SLOTS],
            total_steps: 0,
            soft_reset_count: 0,
            time_base_us: 0,
        })
    }

    /// Re-boots from the same image: a fresh `Cpu`/`FirmwareBus` pair (so RAM,
    /// peripheral state, the emulated flash and the reconstructed
    /// framebuffer all start over), with the currently-held buttons and the
    /// host-set acceleration ([`FirmwareRuntime::set_acceleration`])
    /// re-applied. Queued serial input is dropped with the old bus. A cold
    /// power-on: the soft-reset count and emulated time go back to 0, and
    /// `rtc_get_reset_reason` reports `POWERON_RESET` again.
    ///
    /// Can only fail if the image stopped parsing, which can't happen for an
    /// image that already parsed once — but the error is propagated rather
    /// than unwrapped so this stays panic-free across the WASM boundary.
    pub fn reset(&mut self) -> Result<(), ImageParseError> {
        let (cpu, bus) = boot_from_firmware_image_with_rom_stubs(&self.image)?;
        let accel = self.acceleration();
        self.cpu = cpu;
        self.bus = bus;
        self.total_steps = 0;
        self.soft_reset_count = 0;
        self.time_base_us = 0;
        self.reapply_host_input(accel);
        Ok(())
    }

    /// Re-applies the held buttons and `accel` to a freshly-built bus.
    fn reapply_host_input(&mut self, accel: [i32; 3]) {
        let held = self.buttons;
        for (slot, pressed) in held.iter().enumerate() {
            self.apply_button(slot, *pressed);
        }
        let [x, y, z] = accel;
        self.bus.i2c0.accel.set_acceleration(x, y, z);
    }

    /// The software reset a `RequestReset` ROM stub asked for, with
    /// `reason` for the next boot's `rtc_get_reset_reason`. See the module
    /// doc, "Software reset", for what it keeps. If the image somehow
    /// stopped parsing, nothing changes: the core stays on the stub, which
    /// asks again next step.
    fn soft_reset(&mut self, reason: u32) {
        use crate::mem::soc::RTC_RANGE;
        use crate::mem::Bus;
        let Ok((mut cpu, bus)) = boot_from_firmware_image_with_rom_stubs(&self.image) else {
            return;
        };
        let Ok(parsed) = crate::mem::image::parse_image(&self.image) else {
            return;
        };
        cpu.set_rom_stubs(crate::rom::esp32c3_rom_stubs_with_reset_reason(reason));
        let rtc_ram: Vec<u8> = RTC_RANGE.map(|a| self.bus.read8(a)).collect();
        let accel = self.acceleration();
        self.time_base_us += self.bus.clock.elapsed_us();
        let old = std::mem::replace(&mut self.bus, bus);
        self.cpu = cpu;

        self.bus.flash_chip = old.flash_chip;
        self.bus.rtc_cntl.set_store_regs(old.rtc_cntl.store_regs());
        for (a, b) in RTC_RANGE.zip(rtc_ram) {
            self.bus.write8(a, b);
        }
        // The image's RTC segments are reloaded over the kept memory.
        for seg in &parsed.segments {
            let Some(data) = self.image.get(seg.file_offset..seg.file_offset + seg.len) else {
                continue;
            };
            for (a, b) in (seg.load_addr..).zip(data) {
                if RTC_RANGE.contains(&a) {
                    self.bus.write8(a, *b);
                }
            }
        }
        self.bus.spi.st7789 = old.spi.st7789;
        self.bus.console = old.console;
        self.reapply_host_input(accel);
        self.soft_reset_count += 1;
    }

    /// Software resets since construction or the last
    /// [`FirmwareRuntime::reset`] (module doc, "Software reset").
    pub fn soft_reset_count(&self) -> u32 {
        self.soft_reset_count
    }

    /// Runs up to `budget` instruction steps, delivering peripheral
    /// interrupts along the way (via
    /// [`crate::boot::step_with_interrupts`] — the correct one-poll-per-step
    /// cadence, see its doc).
    ///
    /// A core parked by `WFI` (`Cpu::is_waiting`) still costs one step of
    /// `budget` per step, fast-forward or not, so this always returns after
    /// exactly `budget` steps even if the firmware waits forever (Milestone
    /// 3 Task 6; pinned by
    /// `tests::run_returns_after_budget_even_if_wfi_never_wakes`).
    pub fn run(&mut self, budget: u32) -> RunSummary {
        self.run_impl(budget, None, None)
    }

    /// Runs until emulated time reaches `target_time_us` or `max_steps`
    /// steps have run, whichever is first; `RunSummary::steps` is the
    /// number run (0 if the target is already past). The browser's frame
    /// pacing and the time-based button holds use this (Milestone 6).
    ///
    /// An idle `WFI` wait whose next event lies past the target is advanced
    /// only to the target ([`crate::boot::idle_until`]), so a run that ends
    /// idle ends exactly at the target instead of at the next alarm; a run
    /// that ends busy overshoots by at most one step's time. A software
    /// reset on the way does not move the target (module doc).
    pub fn run_until(&mut self, target_time_us: u64, max_steps: u32) -> RunSummary {
        self.run_impl(max_steps, Some(target_time_us), None)
    }

    /// Emulated time since construction or the last cold
    /// [`FirmwareRuntime::reset`], in microseconds. Software resets do not
    /// restart it (module doc, "Software reset").
    pub fn emulated_time_us(&self) -> u64 {
        self.time_base_us + self.bus.clock.elapsed_us()
    }

    /// The current clock's tick count at host time `target_us` (0 if the
    /// target is before this clock started).
    fn ticks_at_host_us(&self, target_us: u64) -> u64 {
        self.bus
            .clock
            .ticks_at_us(target_us.saturating_sub(self.time_base_us))
    }

    /// Same as [`FirmwareRuntime::run`], but also tallies each step's
    /// `pc_before` (the `pc` the CPU was at *before* that step executed)
    /// into `pc_hist`, keyed by address with a running count — a "hot PCs"
    /// histogram for diagnosing where a stalled boot is spinning (see
    /// `examples/boot-probe.rs`). Kept as a separate method (rather than an
    /// `Option` parameter on `run` itself) so `run`'s own hot path stays
    /// allocation-free for the WASM build, which never needs this.
    pub fn run_traced(&mut self, budget: u32, pc_hist: &mut HashMap<u32, u64>) -> RunSummary {
        self.run_impl(budget, None, Some(pc_hist))
    }

    /// Shared body for [`FirmwareRuntime::run`],
    /// [`FirmwareRuntime::run_until`] and [`FirmwareRuntime::run_traced`]:
    /// up to `budget` steps, stopping early (before a step) once emulated
    /// time has reached `stop_at_us`. A software reset requested by a step
    /// is performed right after it, and the stop point is re-derived for
    /// the new clock.
    fn run_impl(
        &mut self,
        budget: u32,
        stop_at_us: Option<u64>,
        mut pc_hist: Option<&mut HashMap<u32, u64>>,
    ) -> RunSummary {
        let mut stop_at_ticks = stop_at_us.map(|us| self.ticks_at_host_us(us));
        let mut summary = RunSummary::default();
        let mut ran: u32 = 0;
        while ran < budget {
            if let Some(t) = stop_at_ticks {
                // An idle wait that runs past the target stops at it
                // (`crate::boot::idle_until`), so time never overshoots.
                if self.bus.clock.total_ticks() >= t || idle_until(&self.cpu, &mut self.bus, t) {
                    break;
                }
            }
            if let Some(hist) = pc_hist.as_deref_mut() {
                *hist.entry(self.cpu.regs.pc).or_insert(0) += 1;
            }
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
            if let Some(reason) = self.cpu.take_reset_request() {
                self.soft_reset(reason);
                stop_at_ticks = stop_at_us.map(|us| self.ticks_at_host_us(us));
            }
            ran += 1;
        }
        summary.steps = ran;
        self.total_steps += u64::from(ran);
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

    /// Total steps run since construction or the last cold
    /// [`FirmwareRuntime::reset`]; software resets do not restart it.
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

    /// Everything the firmware has printed to its console (USB-Serial-JTAG
    /// TX), lossily decoded as UTF-8 — see
    /// `crate::peripherals::console::Console::text`.
    pub fn console_output(&self) -> String {
        self.bus.console.text()
    }

    /// Bytes a USB host sends to the badge's console (USB-Serial-JTAG OUT).
    /// Returns how many were accepted; see
    /// `crate::peripherals::usb_serial_jtag::HOST_QUEUE_CAPACITY`.
    pub fn serial_input(&mut self, bytes: &[u8]) -> usize {
        self.bus.usb_serial_jtag.host_send(bytes)
    }

    /// Serial input the firmware has not read yet.
    pub fn serial_pending(&self) -> usize {
        self.bus.usb_serial_jtag.host_pending()
    }

    /// Sets the acceleration the SC7A20H accelerometer reports, in mg per
    /// axis (`crate::peripherals::sc7a20h`): any value is accepted and
    /// clamped to ±16 g, and the output registers further clamp to the
    /// full scale the firmware configured. The default is the badge lying
    /// face up, (0, 0, +1000). Survives [`FirmwareRuntime::reset`] the way
    /// held buttons do: the badge is still being held the same way.
    pub fn set_acceleration(&mut self, x_mg: i32, y_mg: i32, z_mg: i32) {
        self.bus.i2c0.accel.set_acceleration(x_mg, y_mg, z_mg);
    }

    /// The acceleration the SC7A20H currently reports, in mg (after the
    /// ±16 g clamp).
    pub fn acceleration(&self) -> [i32; 3] {
        self.bus.i2c0.accel.acceleration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ESP-IDF-app-image-shaped blob: one IROM (XIP) segment whose
    /// entry instruction is an endless self-branch, so `run()` can be called
    /// without the CPU wandering anywhere. Mirrors
    /// `crate::boot`'s own test helper.
    /// The segment sits at `0x4200_0020` because its data is at file offset 32:
    /// the MMU seeding needs flash offset and vaddr to agree mod 64 KiB.
    fn synthetic_image() -> Vec<u8> {
        let mut buf = vec![0u8; 24];
        buf[0] = 0xE9;
        buf[1] = 1; // one segment
        buf[4..8].copy_from_slice(&0x4200_0020u32.to_le_bytes());
        buf.extend_from_slice(&0x4200_0020u32.to_le_bytes());
        // `c.j 0` (0xa001) twice: a 2-byte self-loop, padded to 4 bytes so the
        // segment covers both halfwords the fetcher may look at.
        let code: [u8; 4] = [0x01, 0xa0, 0x01, 0xa0];
        buf.extend_from_slice(&(code.len() as u32).to_le_bytes());
        buf.extend_from_slice(&code);
        buf
    }

    /// Same shape as [`synthetic_image`], but the code is `wfi; j .` and
    /// nothing (no SYSTIMER alarm, no interrupt line) is ever armed, so the
    /// WFI never wakes.
    fn wfi_image() -> Vec<u8> {
        let mut buf = vec![0u8; 24];
        buf[0] = 0xE9;
        buf[1] = 1;
        buf[4..8].copy_from_slice(&0x4200_0020u32.to_le_bytes());
        buf.extend_from_slice(&0x4200_0020u32.to_le_bytes());
        let mut code = Vec::new();
        code.extend_from_slice(&0x1050_0073u32.to_le_bytes()); // wfi
        code.extend_from_slice(&0x0000_006fu32.to_le_bytes()); // j .
        buf.extend_from_slice(&(code.len() as u32).to_le_bytes());
        buf.extend_from_slice(&code);
        buf
    }

    /// Review focus 4: a WFI that never wakes must not hang the host. Each
    /// waiting step is one step of the budget, fast-forward or not.
    #[test]
    fn run_returns_after_budget_even_if_wfi_never_wakes() {
        let mut rt = FirmwareRuntime::from_image(&wfi_image()).unwrap();
        let s = rt.run(1000);
        assert_eq!(s.steps, 1000);
        assert_eq!(s.traps, 0);
        assert!(rt.cpu().is_waiting());
        assert_eq!(rt.pc(), 0x4200_0024, "parked after the wfi");
        assert_eq!(rt.total_steps(), 1000);
        // Nothing armed: time still passes at the ordinary per-step rate.
        // `from_image` applies the boot initializers, which seed the
        // bootloader's 80 MHz PLL clock (`crate::rom`, R-T2-4), and this
        // image never writes SYSTEM, so 1000 steps are 1000 * 16 / 80 =
        // 200 SYSTIMER ticks.
        assert_eq!(rt.bus().systimer.elapsed_ticks(), 200);
    }

    #[test]
    fn console_output_reflects_usb_serial_jtag_tx_writes() {
        use crate::mem::soc::USB_SERIAL_JTAG_RANGE;
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        assert_eq!(rt.console_output(), "");
        rt.bus.write32(USB_SERIAL_JTAG_RANGE.start, b'Q' as u32);
        assert_eq!(rt.console_output(), "Q");
    }

    #[test]
    fn runtime_boots_a_synthetic_image_and_runs_a_budget() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        assert_eq!(rt.pc(), 0x4200_0020);
        let summary = rt.run(100);
        assert_eq!(summary.steps, 100);
        assert_eq!(summary.traps, 0, "a self-branch must not trap");
        assert_eq!(summary.last_instruction_fault, None);
        assert_eq!(summary.pc, 0x4200_0020, "still spinning on itself");
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
        assert_eq!(rt.pc(), 0x4200_0020);
        assert!(rt.raw_button(0), "held buttons survive a reset");
        assert!(
            !rt.bus().gpio.pin_level(crate::peripherals::gpio::PIN_START),
            "and are re-applied to the freshly-built bus"
        );
    }

    #[test]
    fn acceleration_defaults_face_up_is_clamped_and_survives_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        assert_eq!(rt.acceleration(), [0, 0, 1000], "face up");
        rt.set_acceleration(i32::MAX, -2000, i32::MIN); // must not panic
        assert_eq!(rt.acceleration(), [16_000, -2000, -16_000]);
        assert_eq!(rt.bus().i2c0.accel.acceleration(), [16_000, -2000, -16_000]);
        rt.reset().expect("reset");
        assert_eq!(rt.acceleration(), [16_000, -2000, -16_000]);
    }

    #[test]
    fn run_traced_tallies_pc_before_each_step_into_the_histogram() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).expect("should boot");
        let mut hist = HashMap::new();
        let summary = rt.run_traced(50, &mut hist);
        assert_eq!(summary.steps, 50);
        assert_eq!(hist.get(&0x4200_0020), Some(&50));
        assert_eq!(hist.len(), 1, "self-branch never leaves this one pc");
    }

    #[test]
    fn from_image_propagates_a_parse_error_instead_of_panicking() {
        match FirmwareRuntime::from_image(&[0u8; 24]) {
            Err(ImageParseError::BadMagic { found: 0 }) => {}
            Err(other) => panic!("unexpected parse error: {other:?}"),
            Ok(_) => panic!("an all-zero blob must not parse as an app image"),
        }
    }

    /// A clone is a full checkpoint (the boot rungs share them,
    /// `tests/boot_progress.rs`): from a real-firmware state just before the
    /// first pixels land (~1.125M steps), a clone and its original each run
    /// 100,000 more steps and end identical, and the original's run does not
    /// leak into the clone.
    #[test]
    fn a_clone_runs_exactly_like_its_original() {
        let image = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../frontend/public/firmware/firmware.bin"),
        )
        .expect("firmware.bin");
        let mut original = FirmwareRuntime::from_image(&image).expect("boot");
        original.run(1_100_000);
        let mut clone = original.clone();
        original.run(100_000);
        assert_eq!(clone.total_steps(), 1_100_000, "the clone did not move");
        clone.run(100_000);
        assert_eq!(clone.pc(), original.pc());
        assert_eq!(clone.total_steps(), original.total_steps());
        assert_eq!(clone.framebuffer(), original.framebuffer());
        assert_eq!(clone.console_output(), original.console_output());
        assert!(
            original.framebuffer().iter().any(|px| *px != 0),
            "the window drew pixels, so the framebuffers compared are not blank"
        );
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

    #[test]
    fn emulated_time_starts_at_zero_and_follows_the_clock() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        assert_eq!(rt.emulated_time_us(), 0);
        rt.run(1_600);
        // The boot initializers seed the bootloader's 80 MHz PLL clock
        // (R-T2-4) and this image never writes SYSTEM: 1,600 steps at
        // 80 MHz are 320 ticks of the 16 MHz SYSTIMER, 20 us. (100 us under
        // the pre-Milestone-6 one-tick-per-step clock.)
        assert_eq!(rt.emulated_time_us(), 20);
    }

    #[test]
    fn run_until_stops_at_the_target_time() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let s = rt.run_until(1_000, 1_000_000);
        assert!(rt.emulated_time_us() >= 1_000);
        assert_eq!(u64::from(s.steps), rt.total_steps());
        assert!(s.steps < 1_000_000);
    }

    #[test]
    fn run_until_stops_at_the_step_cap() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let s = rt.run_until(u64::MAX / 2, 500);
        assert_eq!(s.steps, 500);
        assert_eq!(rt.total_steps(), 500);
    }

    #[test]
    fn run_until_a_past_target_runs_nothing() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(10_000);
        let s = rt.run_until(0, 1_000);
        assert_eq!(s.steps, 0);
        assert_eq!(rt.total_steps(), 10_000);
    }

    /// Runs one step at `stub` (a mask-ROM stub address) with `ra` pointing
    /// back at the synthetic image's self-loop, as an ordinary call would.
    fn call_stub(rt: &mut FirmwareRuntime, stub: u32) -> RunSummary {
        rt.cpu
            .regs
            .write(crate::cpu::rom_stubs::REG_RA, 0x4200_0020);
        rt.cpu.regs.pc = stub;
        rt.run(1)
    }

    /// What `rtc_get_reset_reason` returns right now.
    fn reset_reason(rt: &mut FirmwareRuntime) -> u32 {
        call_stub(rt, crate::rom::RTC_GET_RESET_REASON);
        rt.cpu.regs.read(crate::cpu::rom_stubs::REG_A0)
    }

    /// A blank (`0xFF`) flash offset in the storage partition, for the
    /// retention tests.
    const FLASH_PROBE: u32 = 0x3F_0000;
    /// An RTC fast memory word no image segment covers.
    const RTC_PROBE: u32 = crate::mem::soc::RTC_RANGE.start + 0x100;
    /// `RTC_CNTL_STORE6_REG`, ESP-IDF's `RTC_RESET_CAUSE_REG`.
    const STORE6_ADDR: u32 =
        crate::mem::soc::RTC_CNTL_RANGE.start + crate::peripherals::rtc_cntl::STORE6_REG;

    #[test]
    fn soft_reset_keeps_flash_store_regs_and_counts_steps() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        assert_eq!(rt.soft_reset_count(), 0);
        rt.run(100);
        rt.bus.flash_chip.program(FLASH_PROBE, &[0x5A]);
        rt.bus.write32(STORE6_ADDR, 0x8003_0003);
        rt.bus.write32(RTC_PROBE, 0xC0FF_EE00);
        let steps = rt.total_steps();

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(rt.pc(), 0x4200_0020, "restarted at the image's entry");
        assert_eq!(rt.total_steps(), steps + 1, "the reset step counts once");
        assert_eq!(rt.bus.flash_chip.read(FLASH_PROBE), 0x5A);
        assert_eq!(rt.bus.read32(STORE6_ADDR), 0x8003_0003);
        assert_eq!(rt.bus.read32(RTC_PROBE), 0xC0FF_EE00);
        assert_eq!(reset_reason(&mut rt), crate::rom::RESET_REASON_CPU0_SW);
    }

    #[test]
    fn software_reset_reports_a_core_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET);
        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(reset_reason(&mut rt), crate::rom::RESET_REASON_CORE_SW);
    }

    #[test]
    fn emulated_time_is_monotonic_across_a_soft_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(80_000);
        let t_before = rt.emulated_time_us();
        assert!(t_before > 0);
        rt.cpu
            .regs
            .write(crate::cpu::rom_stubs::REG_RA, 0x4200_0020);
        rt.cpu.regs.pc = crate::rom::SOFTWARE_RESET_CPU;
        let s = rt.run_until(t_before + 50_000, 100_000_000);
        assert_eq!(rt.soft_reset_count(), 1, "the run crossed the reset");
        assert!(s.steps > 1);
        assert!(
            rt.emulated_time_us() >= t_before + 50_000,
            "{} < {}",
            rt.emulated_time_us(),
            t_before + 50_000
        );
        assert!(
            rt.emulated_time_us() < t_before + 50_100,
            "stopped at the target, not past it"
        );

        // Sampled step by step around a second reset: never decreasing.
        let mut last = rt.emulated_time_us();
        rt.cpu.regs.pc = crate::rom::SOFTWARE_RESET_CPU;
        for _ in 0..1_000 {
            rt.run(1);
            let now = rt.emulated_time_us();
            assert!(now >= last, "{now} < {last}");
            last = now;
        }
        assert_eq!(rt.soft_reset_count(), 2);
    }

    #[test]
    fn held_button_survives_a_soft_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.set_raw_button(1, true);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.soft_reset_count(), 1);
        assert!(rt.raw_button(1));
        pulse_load(&mut rt);
        assert!(
            !rt.bus()
                .gpio
                .pin_level(crate::peripherals::gpio::PIN_HC165_DATA),
            "the fresh bus shows slot 1 pressed"
        );
    }

    #[test]
    fn console_output_survives_a_soft_reset() {
        use crate::mem::soc::USB_SERIAL_JTAG_RANGE;
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        for b in b"reboot into scanner\n" {
            rt.bus.write32(USB_SERIAL_JTAG_RANGE.start, u32::from(*b));
        }
        let before = rt.console_output();
        rt.serial_input(b"queued");
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        rt.bus.write32(USB_SERIAL_JTAG_RANGE.start, u32::from(b'!'));
        assert!(rt.console_output().starts_with(&before));
        assert_eq!(rt.console_output(), format!("{before}!"));
        assert_eq!(rt.serial_pending(), 0, "queued host input is dropped");
    }

    #[test]
    fn cold_reset_after_a_soft_reset_is_a_power_on() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.flash_chip.program(FLASH_PROBE, &[0x5A]);
        rt.run(10_000);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        rt.run(10_000);
        assert_eq!(rt.soft_reset_count(), 1);
        rt.reset().expect("reset");
        assert_eq!(rt.bus.flash_chip.read(FLASH_PROBE), 0xFF);
        assert_eq!(rt.soft_reset_count(), 0);
        assert_eq!(rt.emulated_time_us(), 0);
        assert_eq!(reset_reason(&mut rt), crate::rom::POWERON_RESET);
    }
}

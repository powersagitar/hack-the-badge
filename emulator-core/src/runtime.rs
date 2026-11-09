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
//! That is a **CPU reset**, [`ResetKind::Cpu`]; ROM `software_reset` is a
//! **core reset**, [`ResetKind::Core`].
//! The run loop collects the request after the step that made it
//! ([`crate::cpu::Cpu::needs_attention`], then
//! [`crate::cpu::Cpu::take_reset_request`] on a cold path) and re-boots the same image
//! there, so a run's step budget stays exact. The new boot is a fresh
//! [`boot_from_firmware_image_with_rom_stubs`] with
//! `crate::rom::esp32c3_rom_stubs_with_reset_reason`, so
//! `rtc_get_reset_reason` reports the reset's reason, plus what a reset
//! keeps on the badge. Both kinds keep:
//!
//! - **the emulated flash, whole** (the identity, `reboot.cfg`);
//! - **RTC_CNTL `STORE0..7`** (`crate::peripherals::rtc_cntl`; ESP-IDF's
//!   reset-reason hint is `STORE6`, `RTC_RESET_CAUSE_REG`, per
//!   `components/esp_system/port/soc/esp32c3/reset_reason.c`), except
//!   `STORE4` and `STORE5`, which get the values the 2nd-stage bootloader
//!   leaves there (`docs/milestone-11-decisions.md`, ruling R-T1-3);
//! - **the RTC timer**, which keeps counting through the reboot: the new
//!   `RtcCntl`'s offset is the old timer's value at the reset plus
//!   [`crate::rom::REBOOT_US`] in slow-clock ticks;
//! - **RTC fast memory** (`crate::mem::soc::RTC_RANGE`), with the image's
//!   own RTC segments reloaded over it, as the bootloader would;
//! - **the panel** (`crate::peripherals::spi::St7789`): an external chip
//!   the MCU reset does not touch, so its pixels stay until the firmware
//!   redraws (any command byte clears its transient parameter state);
//! - **the console's output text**;
//! - **held buttons and the host-set acceleration**, re-applied;
//! - **[`FirmwareRuntime::total_steps`] and host time:**
//!   [`FirmwareRuntime::emulated_time_us`] is the time accumulated before
//!   the reset, plus [`crate::rom::REBOOT_US`] for the reboot the shortcut
//!   boot skips, plus the new clock's, so it never goes backwards. A
//!   [`FirmwareRuntime::run_until`] across a reset still stops at its
//!   target; one whose target lies inside the reboot's jump returns at the
//!   reset with time past the target, as an idle overshoot would. The
//!   jump moves host time and the RTC timer only: the new boot's cycle
//!   counter and SYSTIMER start at 0, and it runs no steps;
//! - **two diagnostics:** the bus's unmapped-access log
//!   ([`crate::mem::bus::FirmwareBus::unmapped_log`], which counts the
//!   resets it has crossed in
//!   [`crate::mem::bus::FirmwareBus::unmapped_log_resets`]) and the core's
//!   [`crate::cpu::Cpu::rom_stub_index_drops`] count (with its last drop).
//!   They describe the whole run since the last cold power-on.
//!
//! A CPU reset also keeps the peripherals the table in
//! `docs/milestone-11-decisions.md` ("What a CPU reset keeps") marks kept,
//! each moved by its named field: SYSTEM, APB_CTRL's RNG, GPIO and the
//! 74HC165, TIMG0 and TIMG1, the whole of RTC_CNTL, USB-Serial-JTAG (queued
//! host input included, so a provision typed across a reboot reaches the
//! new boot's console) and the SC7A20H's registers. Then come the
//! bootloader's writes to them: the CPU clock back at 80 MHz on the PLL
//! ([`crate::peripherals::system::System::set_bootloader_cpu_clock`]) and
//! GPIO21's output disabled
//! ([`crate::peripherals::gpio::Gpio::release_default_console_io`]). A core
//! reset rebuilds them all, and drops the queued host input with the old
//! USB-Serial-JTAG, as [`FirmwareRuntime::reset`] does.
//!
//! Everything else starts fresh with the new boot.
//!
//! [`FirmwareRuntime::soft_reset_count`] counts the resets. The host's own
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
use crate::cpu::rom_stubs::ResetKind;
use crate::cpu::{exception_code, Cpu, HookHit};
use crate::mem::bus::FirmwareBus;
use crate::mem::image::ImageParseError;
use crate::peripherals::spi::{SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::radio::hci::Advertisement;
use crate::radio::{self, Radio};

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

/// [`crate::rom::REBOOT_US`] in RTC slow-clock ticks: what the RTC timer
/// counts during a reboot.
const fn reboot_rtc_ticks() -> u64 {
    crate::rom::REBOOT_US * crate::peripherals::timg::RC_SLOW_HZ / 1_000_000
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
    /// software resets replaced, plus [`crate::rom::REBOOT_US`] per reset;
    /// [`FirmwareRuntime::emulated_time_us`] adds the current clock's.
    time_base_us: u64,
    /// The fake BLE controller and its hooks (`crate::radio`), installed
    /// on every boot.
    radio: Radio,
}

impl FirmwareRuntime {
    /// Boots `image` (the raw bytes of an ESP-IDF app image, i.e.
    /// `frontend/public/firmware/firmware.bin`) through
    /// [`boot_from_firmware_image_with_rom_stubs`].
    pub fn from_image(image: &[u8]) -> Result<Self, ImageParseError> {
        let image: Arc<[u8]> = Arc::from(image.to_vec().into_boxed_slice());
        let (mut cpu, bus) = boot_from_firmware_image_with_rom_stubs(&image)?;
        let mut radio = Radio::new();
        radio.install(&mut cpu);
        Ok(Self {
            image,
            cpu,
            bus,
            buttons: [false; NUM_RAW_BUTTON_SLOTS],
            total_steps: 0,
            soft_reset_count: 0,
            time_base_us: 0,
            radio,
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
        let mac = self.factory_mac();
        self.cpu = cpu;
        self.bus = bus;
        self.total_steps = 0;
        self.soft_reset_count = 0;
        self.time_base_us = 0;
        self.reapply_host_input(accel, mac);
        self.radio = Radio::new();
        self.radio.set_mac(mac);
        self.radio.install(&mut self.cpu);
        Ok(())
    }

    /// Re-applies the held buttons, `accel` and the factory `mac` to a
    /// freshly-built bus.
    fn reapply_host_input(&mut self, accel: [i32; 3], mac: Option<[u8; 6]>) {
        let held = self.buttons;
        for (slot, pressed) in held.iter().enumerate() {
            self.apply_button(slot, *pressed);
        }
        let [x, y, z] = accel;
        self.bus.i2c0.accel.set_acceleration(x, y, z);
        if let Some(mac) = mac {
            self.bus.efuse.set_factory_mac(mac);
        }
    }

    /// The software reset a `RequestReset` ROM stub asked for, with
    /// `reason` for the next boot's `rtc_get_reset_reason`. See the module
    /// doc, "Software reset", for what each [`ResetKind`] keeps. If the
    /// image somehow stopped parsing, nothing changes: the core stays on
    /// the stub, which asks again next step.
    fn soft_reset(&mut self, reason: u32, kind: ResetKind) {
        use crate::mem::soc::RTC_RANGE;
        use crate::mem::Bus;
        let Ok((mut cpu, bus)) = boot_from_firmware_image_with_rom_stubs(&self.image) else {
            return;
        };
        let Ok(parsed) = crate::mem::image::parse_image(&self.image) else {
            return;
        };
        cpu.set_rom_stubs(crate::rom::esp32c3_rom_stubs_with_reset_reason(reason));
        cpu.carry_rom_stub_index_drops_from(&self.cpu);
        let rtc_ram: Vec<u8> = RTC_RANGE.map(|a| self.bus.read8(a)).collect();
        let accel = self.acceleration();
        self.time_base_us += self.bus.clock.elapsed_us() + crate::rom::REBOOT_US;
        let rtc_at_reset = self
            .bus
            .rtc_cntl
            .rtc_ticks_at(self.bus.systimer.elapsed_ticks());
        let mut old = std::mem::replace(&mut self.bus, bus);
        self.cpu = cpu;

        self.bus.carry_unmapped_log_from(&mut old);
        self.bus.flash_chip = old.flash_chip;
        match kind {
            ResetKind::Cpu => {
                // The table's "kept" rows, by field, then the bootloader's
                // writes to them (`docs/milestone-11-decisions.md`, "What a
                // CPU reset keeps"; rows in parentheses).
                self.bus.system = old.system; // (3)
                self.bus.system.set_bootloader_cpu_clock();
                self.bus.apb_ctrl = old.apb_ctrl; // (4)
                self.bus.gpio = old.gpio; // (5), with the 74HC165 (6)
                self.bus.gpio.release_default_console_io();
                self.bus.timg0 = old.timg0; // (10)
                self.bus.timg1 = old.timg1; // (10)
                self.bus.rtc_cntl = old.rtc_cntl; // (11), with `STORE4`/`STORE5` below
                self.bus.usb_serial_jtag = old.usb_serial_jtag; // (12)
                self.bus.i2c0.accel = old.i2c0.accel; // (14)
            }
            ResetKind::Core => self.bus.rtc_cntl.set_store_regs(old.rtc_cntl.store_regs()),
        }
        // The bootloader runs after both kinds of reset: `STORE4` and
        // `STORE5` as it rewrites them (module doc), and the RTC timer's
        // offset, which restarts with SYSTIMER.
        let mut store = self.bus.rtc_cntl.store_regs();
        store[4] = crate::rom::BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE;
        store[5] = crate::rom::BOOTLOADER_RTC_APB_FREQ_REG_VALUE;
        self.bus.rtc_cntl.set_store_regs(store);
        self.bus
            .rtc_cntl
            .set_rtc_offset_ticks(rtc_at_reset + reboot_rtc_ticks());
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
        self.reapply_host_input(accel, old.efuse.factory_mac());
        self.radio.soft_reset();
        self.radio.install(&mut self.cpu);
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
    /// reset on the way does not move the target, and one whose reboot
    /// jumps past the target ends the run there (module doc).
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

    /// Collects what the last step left for the host
    /// ([`crate::cpu::Cpu::needs_attention`]): an app-image hook hit,
    /// answered by [`FirmwareRuntime::dispatch_hook`], or a software reset
    /// request, which is performed here. Returns `true` if it reset, so
    /// the run loop re-derives its stop point only for a new clock. Out of
    /// line and cold, so the run loop's per-step cost is the flag test
    /// alone (Milestone 10 Task 1; inlining this check cost about 10 %
    /// natively, see `docs/milestone-9-decisions.md`).
    ///
    /// The flag is cleared first and every kind of request is then taken
    /// (ruling R-T1-1), so no request is left behind by another's take. A
    /// raised flag with nothing pending (a driver that took a request
    /// without clearing it) is an empty attend: it clears the flag and
    /// changes nothing else.
    #[cold]
    #[inline(never)]
    fn attend(&mut self) -> bool {
        self.cpu.clear_attention();
        if let Some(hit) = self.cpu.take_hook_hit() {
            self.dispatch_hook(hit);
        }
        if let Some((reason, kind)) = self.cpu.take_reset_request() {
            self.soft_reset(reason, kind);
            return true;
        }
        false
    }

    /// Answers an app-image hook hit (`crate::cpu::app_hooks`): every hook
    /// is the radio's ([`Radio::install`]), which returns or redirects,
    /// never passes through (`crate::radio`'s module doc).
    fn dispatch_hook(&mut self, hit: HookHit) {
        self.radio.on_hook(hit, &mut self.cpu, &mut self.bus);
    }

    /// Shared body for [`FirmwareRuntime::run`],
    /// [`FirmwareRuntime::run_until`] and [`FirmwareRuntime::run_traced`]:
    /// up to `budget` steps, stopping early (before a step) once emulated
    /// time has reached `stop_at_us`. What a step leaves for the host (a
    /// hook hit, a software reset) is collected right after it
    /// ([`FirmwareRuntime::attend`]); after a reset the stop point is
    /// re-derived for the new clock.
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
            if self.cpu.needs_attention() && self.attend() {
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

    /// Gives this badge a factory MAC (host input; Milestone 10 Task 4):
    /// the eFuse block reports `mac` in its MAC registers
    /// (`crate::peripherals::efuse`), which `esp_read_mac` reads and the
    /// BLE random address is built from. Returns `false`, changing nothing,
    /// unless `mac` is exactly 6 bytes. With none set (the default) the
    /// eFuse block reads as it always has.
    ///
    /// The MAC is the chip's: [`FirmwareRuntime::reset`], a software reset
    /// and a clone all keep it. ESP-IDF reads it once per boot and caches
    /// it, and v392 first reads it while provisioning, so set it before the
    /// first step (or reset after setting it) for the firmware to use it
    /// (`docs/milestone-10-decisions.md`, ruling R-T4-1).
    pub fn set_factory_mac(&mut self, mac: &[u8]) -> bool {
        let Ok(mac) = <[u8; 6]>::try_from(mac) else {
            return false;
        };
        self.bus.efuse.set_factory_mac(mac);
        self.radio.set_mac(Some(mac));
        true
    }

    /// The host-set factory MAC, if any ([`FirmwareRuntime::set_factory_mac`]).
    pub fn factory_mac(&self) -> Option<[u8; 6]> {
        self.bus.efuse.factory_mac()
    }

    /// Successful `esp_bt_controller_init` calls since power-on
    /// ([`Radio::init_count`]): 1 once a radio app has opened, and still 1
    /// after it reopens, since the firmware initializes the radio once per
    /// boot. A soft reset keeps the count; [`FirmwareRuntime::reset`]
    /// clears it.
    pub fn radio_init_count(&self) -> u32 {
        self.radio.init_count()
    }

    /// The radio, for tests and diagnostics.
    pub fn radio(&self) -> &Radio {
        &self.radio
    }

    /// What this badge is advertising now
    /// ([`crate::radio::hci::HciController::advertisement`]), for the
    /// host to pass to its linked peers; `None` while no radio app
    /// advertises.
    pub fn radio_advertisement(&self) -> Option<Advertisement> {
        self.radio.controller().advertisement()
    }

    /// [`FirmwareRuntime::radio_advertisement`] in the peers' wire form
    /// ([`crate::radio::advertisement_to_bytes`]: `addr(6) ‖ addr_type(1)
    /// ‖ data`); empty while not advertising.
    pub fn radio_advertisement_bytes(&self) -> Vec<u8> {
        self.radio_advertisement()
            .map(|adv| radio::advertisement_to_bytes(&adv))
            .unwrap_or_default()
    }

    /// This badge hears a peer's advertisement at `rssi` dBm (host input;
    /// a linked peer's is heard at [`radio::LINKED_PEER_RSSI`]). `true` if
    /// it was queued for the firmware, which then receives it from its
    /// idle task as the run continues; `false` if no radio app is scanning
    /// (it is dropped, never held for later), the report is masked or a
    /// duplicate, or `adv` is malformed ([`Radio::deliver`]).
    pub fn deliver_advertisement(&mut self, adv: &Advertisement, rssi: i8) -> bool {
        self.radio.deliver(&mut self.cpu, adv, rssi)
    }

    /// [`FirmwareRuntime::deliver_advertisement`] from a peer's wire form
    /// ([`crate::radio::advertisement_from_bytes`]): `false`, changing
    /// nothing, for malformed bytes (shorter than 7, data over 229 bytes,
    /// an Address_Type over 3). Never panics, whatever the bytes.
    pub fn deliver_advertisement_bytes(&mut self, bytes: &[u8], rssi: i8) -> bool {
        match radio::advertisement_from_bytes(bytes) {
            Some(adv) => self.deliver_advertisement(&adv, rssi),
            None => false,
        }
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
        let target = t_before + crate::rom::REBOOT_US + 50_000;
        let s = rt.run_until(target, 100_000_000);
        assert_eq!(rt.soft_reset_count(), 1, "the run crossed the reset");
        assert!(s.steps > 1);
        assert!(
            rt.emulated_time_us() >= target,
            "{} < {}",
            rt.emulated_time_us(),
            target
        );
        assert!(
            rt.emulated_time_us() < target + 100,
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

    /// R-T6-1's gap, the "nothing replays" half of Milestone 9's Review
    /// Focus 1: a button pressed and released before a soft reset is not
    /// re-applied to the fresh bus. The shift register reads it released
    /// from the reset step on, before the new boot runs anything.
    #[test]
    fn released_button_does_not_replay_after_a_soft_reset() {
        use crate::peripherals::gpio::PIN_HC165_DATA;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.set_raw_button(1, true);
        rt.run(1_000);
        rt.set_raw_button(1, false);
        rt.run(1_000);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.soft_reset_count(), 1);
        assert!(!rt.raw_button(1));
        pulse_load(&mut rt);
        for i in 0..crate::peripherals::gpio::HC165_SLOTS {
            assert!(
                rt.bus().gpio.pin_level(PIN_HC165_DATA),
                "the fresh bus shows every slot released (bit {i})"
            );
            clk_rising_edge(&mut rt);
        }
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
        assert_eq!(rt.serial_pending(), 6, "a CPU reset keeps it");
    }

    /// The RTC timer as `rtc_cntl_ll_get_rtc_time` reads it: `TIME_UPDATE`
    /// set by a read-modify-write, then `TIME_LOW0` and `TIME_HIGH0`.
    fn latch_rtc_timer(rt: &mut FirmwareRuntime) -> u64 {
        use crate::mem::soc::RTC_CNTL_RANGE;
        use crate::mem::Bus;
        use crate::peripherals::rtc_cntl::{TIME_HIGH0_REG, TIME_LOW0_REG, TIME_UPDATE_REG};
        let base = RTC_CNTL_RANGE.start;
        let update = rt.bus.read32(base + TIME_UPDATE_REG);
        rt.bus.write32(base + TIME_UPDATE_REG, update | (1 << 31));
        u64::from(rt.bus.read32(base + TIME_LOW0_REG))
            | (u64::from(rt.bus.read32(base + TIME_HIGH0_REG)) << 32)
    }

    /// 466 ms at the RC slow clock's 136 kHz, written out so the test does
    /// not lean on the conversion it checks.
    const REBOOT_RTC_TICKS: u64 = 63_376;

    /// 79,999 steps at 80 MHz end one fifth of a tick before 1,000 us, so
    /// the next step's own time shows in `emulated_time_us`.
    #[test]
    fn soft_reset_adds_reboot_time() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(79_999);
        let mut plain_step = rt.clone();
        plain_step.run(1);
        let stepped = plain_step.emulated_time_us();
        assert!(stepped > rt.emulated_time_us(), "the step's time is seen");
        let steps = rt.total_steps();

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(rt.emulated_time_us(), stepped + crate::rom::REBOOT_US);
        assert_eq!(rt.total_steps(), steps + 1, "the jump runs no steps");
        assert_eq!(
            rt.bus.clock.total_ticks(),
            0,
            "the new boot's clock starts at 0"
        );
    }

    #[test]
    fn rtc_timer_continues_across_a_soft_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(8_000_000);
        let before = latch_rtc_timer(&mut rt);
        assert!(before > 0, "the timer has counted");

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        let after = latch_rtc_timer(&mut rt);

        assert_eq!(rt.soft_reset_count(), 1);
        let floor = before + REBOOT_RTC_TICKS;
        assert!(after >= floor, "{after} < {floor}");
        assert!(after <= floor + 1, "{after} > {floor} + 1 tick");
    }

    #[test]
    fn cold_reset_clears_the_rtc_offset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(8_000_000);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert!(latch_rtc_timer(&mut rt) >= REBOOT_RTC_TICKS);

        rt.reset().expect("reset");

        assert_eq!(rt.bus.rtc_cntl.rtc_offset_ticks(), 0);
        let first = latch_rtc_timer(&mut rt);
        assert!(first < 1_000, "{first}");
    }

    /// The bootloader runs after every software reset and rewrites `STORE4`
    /// and `STORE5` (decisions doc, ruling R-T1-3); the other six keep the
    /// guest's values.
    #[test]
    fn store4_is_the_bootloaders_after_a_soft_reset() {
        use crate::mem::soc::RTC_CNTL_RANGE;
        use crate::mem::Bus;
        use crate::peripherals::rtc_cntl::{STORE4_REG, STORE5_REG, STORE_REGS};
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let addr = |off: u32| RTC_CNTL_RANGE.start + off;
        for (i, off) in STORE_REGS.iter().enumerate() {
            rt.bus.write32(addr(*off), 0xA5A5_0000 + i as u32);
        }
        rt.bus.write32(addr(STORE4_REG), 0xdead_beef);
        rt.bus.write32(addr(STORE5_REG), 0x2626_2626);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(
            rt.bus.read32(addr(STORE4_REG)),
            crate::rom::BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE
        );
        assert_eq!(rt.bus.read32(addr(STORE4_REG)), 0x0028_0028);
        assert_eq!(rt.bus.read32(addr(STORE5_REG)), 0x4C4B_4C4B);
        for (i, off) in STORE_REGS.iter().enumerate() {
            if *off != STORE4_REG && *off != STORE5_REG {
                assert_eq!(
                    rt.bus.read32(addr(*off)),
                    0xA5A5_0000 + i as u32,
                    "STORE{i} keeps the guest's value"
                );
            }
        }

        // A cold boot is unchanged: STORE5 goes back to its reset value.
        rt.reset().expect("reset");
        assert_eq!(rt.bus.read32(addr(STORE5_REG)), 0);
    }

    /// An address only the bus's catch-all answers.
    const UNMAPPED_PROBE: u32 = 0x3ff1_0008;

    #[test]
    fn diagnostics_carry_across_a_soft_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.read32(UNMAPPED_PROBE);
        assert_eq!(rt.bus.unmapped_log_resets(), 0);
        // intr_matrix_set(_, model_num = 64, _): one past the last MAP slot.
        rt.cpu.regs.write(crate::cpu::rom_stubs::REG_A1, 64);
        call_stub(&mut rt, crate::rom::INTR_MATRIX_SET);
        assert_eq!(rt.cpu.rom_stub_index_drops(), 1);
        let last_drop = rt.cpu.last_rom_stub_index_drop();
        assert!(last_drop.is_some());

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.soft_reset_count(), 1);
        assert!(rt
            .bus
            .unmapped_log()
            .iter()
            .any(|a| a.addr == UNMAPPED_PROBE));
        assert_eq!(rt.bus.unmapped_log_resets(), 1);
        assert_eq!(rt.cpu.rom_stub_index_drops(), 1);
        assert_eq!(rt.cpu.last_rom_stub_index_drop(), last_drop);

        rt.reset().expect("reset");
        assert_eq!(rt.bus.unmapped_log_resets(), 0, "a cold power-on");
        assert!(rt.bus.unmapped_log().is_empty());
        assert_eq!(rt.cpu.rom_stub_index_drops(), 0);
    }

    /// Review Focus 2: a `run_until` target inside the reboot's jump. The
    /// run returns at the reset with time past the target, time never goes
    /// backwards, a repeat of that target runs nothing, and a later target
    /// runs normally.
    #[test]
    fn run_until_target_inside_the_reboot_jump() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(80_000);
        let start = rt.emulated_time_us();
        let target = start + crate::rom::REBOOT_US / 2;
        rt.cpu
            .regs
            .write(crate::cpu::rom_stubs::REG_RA, 0x4200_0020);
        rt.cpu.regs.pc = crate::rom::SOFTWARE_RESET_CPU;

        let s = rt.run_until(target, 1_000_000);

        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(s.steps, 1, "returned at the reset");
        let after = rt.emulated_time_us();
        assert!(after >= start + crate::rom::REBOOT_US, "{after}");
        assert!(after > target, "{after} <= {target}");

        let again = rt.run_until(target, 1_000_000);
        assert_eq!(again.steps, 0, "the target is already past");
        assert_eq!(rt.emulated_time_us(), after, "time did not go backwards");

        let next = after + 1_000;
        let s = rt.run_until(next, 10_000_000);
        assert!(s.steps > 0 && s.steps < 10_000_000, "stopped by the target");
        assert!(rt.emulated_time_us() >= next);
        assert!(rt.emulated_time_us() < next + 100);
        assert_eq!(rt.soft_reset_count(), 1);
    }

    /// Review Focus 5: a reboot into an app that reboots again.
    #[test]
    fn two_soft_resets_accumulate() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.run(8_000_000);
        let start_us = rt.emulated_time_us();
        let start_rtc = latch_rtc_timer(&mut rt);
        rt.bus.read32(UNMAPPED_PROBE);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.soft_reset_count(), 2);
        let two_reboots = 2 * crate::rom::REBOOT_US;
        let us = rt.emulated_time_us();
        assert!(us >= start_us + two_reboots, "{us}");
        assert!(us <= start_us + two_reboots + 1, "{us}: only two steps ran");
        assert_eq!(rt.bus.unmapped_log_resets(), 2);
        assert_eq!(rt.total_steps(), 8_000_002);
        let rtc = latch_rtc_timer(&mut rt);
        let floor = start_rtc + 2 * REBOOT_RTC_TICKS;
        assert!(rtc >= floor, "{rtc} < {floor}");
        assert!(rtc <= floor + 1, "{rtc} > {floor} + 1 tick");
    }

    // What a CPU reset keeps, row by row (`docs/milestone-11-decisions.md`,
    // "What a CPU reset keeps", the table). A core reset is the rebuild.
    const SYSTEM_BASE: u32 = crate::mem::soc::SYSTEM_RANGE.start;
    const FROM_CPU_0: u32 = SYSTEM_BASE + crate::peripherals::system::CPU_INTR_FROM_CPU_0_REG;
    const CPU_PER_CONF: u32 = SYSTEM_BASE + crate::peripherals::system::CPU_PER_CONF_REG;
    const SYSCLK_CONF: u32 = SYSTEM_BASE + crate::peripherals::system::SYSCLK_CONF_REG;
    const RNG_DATA: u32 =
        crate::mem::soc::APB_CTRL_RANGE.start + crate::peripherals::apb_ctrl::RND_DATA_REG;
    const GPIO_OUT: u32 = crate::mem::soc::GPIO_RANGE.start + crate::peripherals::gpio::OUT_REG;
    const GPIO_ENABLE: u32 =
        crate::mem::soc::GPIO_RANGE.start + crate::peripherals::gpio::ENABLE_REG;
    const TIMG0_WDT: u32 =
        crate::mem::soc::TIMG0_RANGE.start + crate::peripherals::timg::WDTCONFIG1_REG;
    const TIMG1_WDT: u32 =
        crate::mem::soc::TIMG1_RANGE.start + crate::peripherals::timg::WDTCONFIG1_REG;
    /// `RTC_CNTL_OPTIONS0_REG`: a word the model only stores.
    const RTC_OPTIONS0: u32 = crate::mem::soc::RTC_CNTL_RANGE.start;
    /// An I2C0 register the model only stores, reset value 0x10
    /// (`TO_REG`, in `crate::peripherals::i2c`'s reset table).
    const I2C0_TO: u32 = crate::mem::soc::I2C0_RANGE.start + 0x0C;
    const USB_EP1: u32 =
        crate::mem::soc::USB_SERIAL_JTAG_RANGE.start + crate::peripherals::usb_serial_jtag::EP1_REG;
    const USB_EP1_CONF: u32 = crate::mem::soc::USB_SERIAL_JTAG_RANGE.start
        + crate::peripherals::usb_serial_jtag::EP1_CONF_REG;
    /// A flash MMU entry nothing in the synthetic image maps.
    const MMU_PROBE: u32 = crate::mem::soc::MMU_TABLE_RANGE.start + 4 * 100;

    /// Writes `SC7A20H` register `reg`, as `i2c_master_transmit` does.
    fn write_accel_reg(rt: &mut FirmwareRuntime, reg: u8, val: u8) {
        let accel = &mut rt.bus.i2c0.accel;
        accel.start(false);
        accel.write(reg);
        accel.write(val);
        accel.stop();
    }

    /// Reads `SC7A20H` register `reg`, as `i2c_master_transmit_receive` does.
    fn read_accel_reg(rt: &mut FirmwareRuntime, reg: u8) -> u8 {
        let accel = &mut rt.bus.i2c0.accel;
        accel.start(false);
        accel.write(reg);
        accel.start(true);
        let val = accel.read();
        accel.stop();
        val
    }

    /// Runs and reads the console's OUT endpoint until `count` bytes came
    /// through it, or nothing more will.
    fn read_serial(rt: &mut FirmwareRuntime, count: usize) -> Vec<u8> {
        use crate::mem::Bus;
        use crate::peripherals::usb_serial_jtag::SERIAL_OUT_EP_DATA_AVAIL as DATA_AVAIL;
        let mut got = Vec::new();
        for _ in 0..20 {
            rt.run(10_000);
            while got.len() < count && rt.bus.read32(USB_EP1_CONF) & DATA_AVAIL != 0 {
                got.push(rt.bus.read32(USB_EP1) as u8);
            }
        }
        got
    }

    /// Review Focus 1, the core half: a provision typed across a reboot.
    /// The firmware reboots before it has read any of the input, and the
    /// new boot reads all of it, in order, exactly once.
    #[test]
    fn queued_serial_input_survives_a_cpu_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let sent: Vec<u8> = (0..100).collect();
        assert_eq!(rt.serial_input(&sent), 100);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(rt.serial_pending(), 100);
        assert_eq!(read_serial(&mut rt, 200), sent);
        assert_eq!(rt.serial_pending(), 0);
    }

    /// The firmware read part of a packet, then rebooted: the rest of the
    /// FIFO and the host queue arrive in order, with no byte twice.
    #[test]
    fn serial_input_read_partly_before_a_cpu_reset_arrives_once() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let sent: Vec<u8> = (0..100).collect();
        rt.serial_input(&sent);
        let read = read_serial(&mut rt, 10);
        assert_eq!(read, sent[..10]);
        assert_ne!(
            rt.bus.read32(USB_EP1_CONF)
                & crate::peripherals::usb_serial_jtag::SERIAL_OUT_EP_DATA_AVAIL,
            0,
            "the packet is half read"
        );

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.serial_pending(), 90);
        assert_eq!(read_serial(&mut rt, 200), sent[10..]);
    }

    #[test]
    fn system_survives_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.write32(FROM_CPU_0, 1);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.bus.read32(FROM_CPU_0), 1);
    }

    /// The bootloader's `rtc_clk_init` leaves the PLL at 80 MHz whatever
    /// `esp_restart_noos` left (table row 3), and writes only these fields:
    /// the rest of `CPU_PER_CONF` is the old boot's.
    #[test]
    fn system_clock_fields_are_the_bootloaders_after_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        // 160 MHz, with WAITI_DELAY_NUM 0xF and WAIT_MODE_FORCE_ON and
        // PLL_FREQ_SEL clear.
        rt.bus.write32(CPU_PER_CONF, 0xF1);
        // XTAL, divider 3.
        rt.bus.write32(SYSCLK_CONF, 0x002);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.bus.read32(SYSCLK_CONF), 1 << 10, "PLL, divider 1");
        assert_eq!(
            rt.bus.read32(CPU_PER_CONF),
            0xF0,
            "CPUPERIOD_SEL 0, the rest kept"
        );
        assert_eq!(rt.bus.system.cpu_hz(), Some(80_000_000));
    }

    /// Ruling R-T1-5: the xorshift sequence continues.
    #[test]
    fn apb_ctrl_rng_survives_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.read32(RNG_DATA);
        let mut no_reset = rt.clone();
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.bus.read32(RNG_DATA), no_reset.bus.read32(RNG_DATA));
    }

    #[test]
    fn gpio_survives_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.write32(GPIO_OUT, 0x0000_0005);
        rt.bus.write32(GPIO_ENABLE, 0x0000_0007);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.bus.read32(GPIO_OUT), 0x0000_0005);
        assert_eq!(rt.bus.read32(GPIO_ENABLE), 0x0000_0007);
    }

    /// Ruling R-T1-4: the bootloader's `release_default_console_io` disables
    /// GPIO21's output (table row 5). Only that bit moves.
    #[test]
    fn gpio21_output_enable_is_the_bootloaders_after_a_cpu_reset() {
        use crate::mem::Bus;
        use crate::peripherals::gpio::PIN_HC165_CLK;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let clk = 1u32 << PIN_HC165_CLK;
        rt.bus.write32(GPIO_ENABLE, clk | 0x3);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(rt.bus.read32(GPIO_ENABLE), 0x3);
    }

    /// The external shift register keeps its latched bits and position, so
    /// a read in progress when the firmware rebooted still shows its bit.
    #[test]
    fn hc165_survives_a_cpu_reset() {
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.set_raw_button(2, true);
        pulse_load(&mut rt);
        clk_rising_edge(&mut rt);
        assert!(!rt.bus.gpio.hc165.data_level(), "slot 2 is shifted out");
        rt.set_raw_button(2, false);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert!(
            !rt.bus.gpio.hc165.data_level(),
            "the snapshot latched before the reset"
        );
    }

    #[test]
    fn timg_survives_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.write32(TIMG0_WDT, 0x0123_4567);
        rt.bus.write32(TIMG1_WDT, 0x89AB_CDEF);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.bus.read32(TIMG0_WDT), 0x0123_4567);
        assert_eq!(rt.bus.read32(TIMG1_WDT), 0x89AB_CDEF);
    }

    /// Every stored word, not only `STORE0..7` (table row 11).
    #[test]
    fn rtc_cntl_survives_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        rt.bus.write32(RTC_OPTIONS0, 0x1234_5678);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.bus.read32(RTC_OPTIONS0), 0x1234_5678);
    }

    /// The accelerometer is outside the SoC and keeps its registers; the
    /// I2C0 controller around it is reset (table rows 13 and 14).
    #[test]
    fn accelerometer_survives_a_cpu_reset_and_i2c0_does_not() {
        use crate::mem::Bus;
        use crate::peripherals::sc7a20h::CTRL_REG1;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        write_accel_reg(&mut rt, CTRL_REG1, 0x57);
        rt.bus.write32(I2C0_TO, 0x1F);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);

        assert_eq!(read_accel_reg(&mut rt, CTRL_REG1), 0x57);
        assert_eq!(rt.bus.read32(I2C0_TO), 0x10, "the reset value");
    }

    /// The bootloader unmaps every entry and maps the app (table row 17),
    /// so a mapping the old app added at run time goes.
    #[test]
    fn mmu_entries_are_the_bootloaders_after_a_cpu_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let seeded = rt.bus.read32(MMU_PROBE);
        rt.bus.write32(MMU_PROBE, 0x05);
        assert_eq!(rt.bus.read32(MMU_PROBE), 0x05);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.bus.read32(MMU_PROBE), seeded);
    }

    /// `software_reset` is the core reset: every peripheral a CPU reset
    /// keeps goes back to its power-on state (Task 2's path).
    #[test]
    fn core_reset_rebuilds_every_peripheral() {
        use crate::mem::Bus;
        use crate::peripherals::sc7a20h::CTRL_REG1;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let first_rng = rt.clone().bus.read32(RNG_DATA);
        rt.serial_input(&[0x41; 100]);
        rt.bus.write32(FROM_CPU_0, 1);
        rt.bus.write32(CPU_PER_CONF, 0xF1);
        rt.bus.read32(RNG_DATA);
        rt.bus.write32(GPIO_OUT, 0x0000_0005);
        rt.bus.write32(GPIO_ENABLE, 0x0000_0007);
        rt.set_raw_button(2, true);
        pulse_load(&mut rt);
        clk_rising_edge(&mut rt);
        rt.set_raw_button(2, false);
        rt.bus.write32(TIMG0_WDT, 0x0123_4567);
        rt.bus.write32(TIMG1_WDT, 0x89AB_CDEF);
        rt.bus.write32(RTC_OPTIONS0, 0x1234_5678);
        write_accel_reg(&mut rt, CTRL_REG1, 0x57);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET);

        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(rt.serial_pending(), 0);
        assert_eq!(rt.bus.read32(FROM_CPU_0), 0);
        assert_eq!(rt.bus.read32(CPU_PER_CONF), 0x0c, "its reset value");
        assert_eq!(rt.bus.read32(RNG_DATA), first_rng, "the sequence restarts");
        assert_eq!(rt.bus.read32(GPIO_OUT), 0);
        assert_eq!(rt.bus.read32(GPIO_ENABLE), 0);
        assert!(rt.bus.gpio.hc165.data_level(), "no snapshot");
        assert_eq!(rt.bus.read32(TIMG0_WDT), 0);
        assert_eq!(rt.bus.read32(TIMG1_WDT), 0);
        assert_eq!(rt.bus.read32(RTC_OPTIONS0), 0);
        assert_eq!(read_accel_reg(&mut rt, CTRL_REG1), 0x07, "its reset value");
    }

    /// Both kinds of reset run the bootloader, so the core reset also takes
    /// `REBOOT_US` and the `STORE4`/`STORE5` rewrites.
    #[test]
    fn core_reset_takes_reboot_time_and_rewrites_the_stores() {
        use crate::mem::soc::RTC_CNTL_RANGE;
        use crate::mem::Bus;
        use crate::peripherals::rtc_cntl::{STORE4_REG, STORE5_REG};
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let store4 = RTC_CNTL_RANGE.start + STORE4_REG;
        let store5 = RTC_CNTL_RANGE.start + STORE5_REG;
        rt.run(8_000_000);
        let before_us = rt.emulated_time_us();
        let before_rtc = latch_rtc_timer(&mut rt);
        rt.bus.write32(store4, 0xdead_beef);
        rt.bus.write32(store5, 0x2626_2626);
        rt.bus.write32(STORE6_ADDR, 0x8003_0003);

        call_stub(&mut rt, crate::rom::SOFTWARE_RESET);

        assert_eq!(rt.soft_reset_count(), 1);
        let us = rt.emulated_time_us();
        assert!(us >= before_us + crate::rom::REBOOT_US, "{us}");
        assert!(us <= before_us + crate::rom::REBOOT_US + 1, "{us}");
        let rtc = latch_rtc_timer(&mut rt);
        let floor = before_rtc + REBOOT_RTC_TICKS;
        assert!(rtc >= floor, "{rtc} < {floor}");
        assert!(rtc <= floor + 1, "{rtc} > {floor} + 1 tick");
        assert_eq!(
            rt.bus.read32(store4),
            crate::rom::BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE
        );
        assert_eq!(
            rt.bus.read32(store5),
            crate::rom::BOOTLOADER_RTC_APB_FREQ_REG_VALUE
        );
        assert_eq!(rt.bus.read32(STORE6_ADDR), 0x8003_0003, "the rest kept");
    }

    /// The fake hacker MAC the radio rungs use (common context).
    const HACKER_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x10, 0x01];
    /// `EFUSE_RD_MAC_SPI_SYS_0_REG` and `_SYS_1_REG` on the bus.
    const MAC_SYS_0: u32 =
        crate::mem::soc::EFUSE_RANGE.start + crate::peripherals::efuse::RD_MAC_SPI_SYS_0_REG;
    const MAC_SYS_1: u32 =
        crate::mem::soc::EFUSE_RANGE.start + crate::peripherals::efuse::RD_MAC_SPI_SYS_1_REG;

    #[test]
    fn set_factory_mac_rejects_a_wrong_length() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        assert_eq!(rt.factory_mac(), None, "no MAC by default");
        let seven = [0x02, 0x00, 0x00, 0x00, 0x10, 0x01, 0x00];
        for bad in [&[][..], &HACKER_MAC[..5], &seven[..]] {
            assert!(!rt.set_factory_mac(bad), "{} bytes", bad.len());
            assert_eq!(rt.factory_mac(), None, "{} bytes", bad.len());
        }
        assert_eq!(rt.bus.read32(MAC_SYS_0), 0);
        assert!(rt.set_factory_mac(&HACKER_MAC));
        assert_eq!(rt.factory_mac(), Some(HACKER_MAC));
        assert_eq!(rt.bus.read32(MAC_SYS_0), 0x0000_1001);
        assert_eq!(rt.bus.read32(MAC_SYS_1), 0x0000_0200);
        assert!(!rt.set_factory_mac(&seven));
        assert_eq!(
            rt.factory_mac(),
            Some(HACKER_MAC),
            "a rejected MAC changes nothing"
        );
    }

    /// The MAC is the chip's: a soft reset keeps it, and so does a cold
    /// power-on, as a real badge keeps its MAC.
    #[test]
    fn mac_survives_cold_and_soft_reset() {
        use crate::mem::Bus;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        assert!(rt.set_factory_mac(&HACKER_MAC));
        rt.run(1_000);
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.soft_reset_count(), 1);
        assert_eq!(rt.factory_mac(), Some(HACKER_MAC));
        assert_eq!(rt.bus.read32(MAC_SYS_0), 0x0000_1001);
        assert_eq!(rt.bus.read32(MAC_SYS_1), 0x0000_0200);

        rt.reset().expect("reset");
        assert_eq!(rt.soft_reset_count(), 0, "a cold power-on");
        assert_eq!(rt.factory_mac(), Some(HACKER_MAC));
        assert_eq!(rt.bus.read32(MAC_SYS_0), 0x0000_1001);
        assert_eq!(rt.bus.read32(MAC_SYS_1), 0x0000_0200);

        // A clone carries it too (the rungs' checkpoints).
        let clone = rt.clone();
        assert_eq!(clone.factory_mac(), Some(HACKER_MAC));
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

    /// The image's entry, `0x4200_0020`, is `addi x7, x7, 1`, then `j -4`
    /// back to it: a loop that counts its passes in `x7`.
    const COUNT_LOOP: u32 = 0x4200_0020;

    fn count_loop_image() -> Vec<u8> {
        let mut buf = vec![0u8; 24];
        buf[0] = 0xE9;
        buf[1] = 1;
        buf[4..8].copy_from_slice(&COUNT_LOOP.to_le_bytes());
        buf.extend_from_slice(&COUNT_LOOP.to_le_bytes());
        let mut code = Vec::new();
        code.extend_from_slice(&0x0013_8393u32.to_le_bytes()); // addi x7, x7, 1
        code.extend_from_slice(&0xffdf_f06fu32.to_le_bytes()); // j -4
        buf.extend_from_slice(&(code.len() as u32).to_le_bytes());
        buf.extend_from_slice(&code);
        buf
    }

    fn hooked_count_loop() -> FirmwareRuntime {
        let mut rt = FirmwareRuntime::from_image(&count_loop_image()).unwrap();
        let mut table = crate::cpu::AppHookTable::new();
        table.insert(COUNT_LOOP, 1);
        rt.cpu.set_app_hooks(table);
        rt
    }

    /// Every boot installs the radio's hook table (`crate::radio`), and the
    /// run loop hands each hit to the radio, which answers it in the same
    /// step: `esp_vhci_host_check_send_available` returns `false` (the
    /// controller status in guest RAM reads IDLE here) to the caller's `ra`
    /// without running the guest function, and the budget stays exact.
    #[test]
    fn the_run_loop_hands_hook_hits_to_the_radio() {
        use crate::radio::v392;
        let mut rt = FirmwareRuntime::from_image(&count_loop_image()).unwrap();
        assert_eq!(
            rt.cpu.app_hooks().lookup(v392::ESP_VHCI_HOST_SEND_PACKET),
            Some(5),
            "installed at boot"
        );
        rt.cpu.regs.write(crate::cpu::rom_stubs::REG_RA, COUNT_LOOP);
        rt.cpu.regs.write(crate::cpu::rom_stubs::REG_A0, 0xDEAD);
        rt.cpu.regs.pc = v392::ESP_VHCI_HOST_CHECK_SEND_AVAILABLE;
        let s = rt.run(1);
        assert_eq!(s.pc, COUNT_LOOP, "returned to ra");
        assert_eq!(rt.cpu.regs.read(crate::cpu::rom_stubs::REG_A0), 0);
        assert_eq!(s.rom_stub_calls, 0, "a hit is not a ROM stub call");
        assert!(!rt.cpu.needs_attention(), "attend cleared the flag");
        assert_eq!(rt.radio().hook_hits(), 1);
        assert_eq!(rt.run(100).steps, 100);
        assert_eq!(rt.total_steps(), 101);
        assert_eq!(rt.cpu.regs.read(7), 50, "the guest runs on");

        // A soft reset rebuilds the core, and the boot installs the table
        // again (ruling R-T3-2).
        call_stub(&mut rt, crate::rom::SOFTWARE_RESET_CPU);
        assert_eq!(rt.soft_reset_count(), 1);
        assert!(rt
            .cpu
            .app_hooks()
            .lookup(v392::ESP_BT_CONTROLLER_INIT)
            .is_some());
        rt.reset().expect("reset");
        assert!(rt
            .cpu
            .app_hooks()
            .lookup(v392::ESP_BT_CONTROLLER_INIT)
            .is_some());
        assert_eq!(rt.radio().hook_hits(), 0, "a cold reset is a new radio");
    }

    /// The attention flag raised with nothing to collect (a driver took a
    /// hit without clearing the flag, as ruling R-T1-1 allows) costs one
    /// empty attend and nothing else: the run matches one without the flag,
    /// step for step and in time, including a `run_until` target.
    #[test]
    fn an_empty_attend_changes_nothing() {
        let mut rt = hooked_count_loop();
        step_with_interrupts(&mut rt.cpu, &mut rt.bus);
        assert!(rt.cpu.take_hook_hit().is_some());
        rt.cpu.hook_pass_through();
        assert!(rt.cpu.needs_attention(), "left raised, nothing pending");

        let mut clean = rt.clone();
        clean.cpu.clear_attention();

        let target = rt.emulated_time_us() + 50;
        let a = rt.run_until(target, 10_000);
        let b = clean.run_until(target, 10_000);
        assert!(!rt.cpu.needs_attention());
        assert_eq!(a, b);
        assert!(a.steps > 0 && a.steps < 10_000, "stopped by the target");
        assert_eq!(rt.total_steps(), clean.total_steps());
        assert_eq!(rt.emulated_time_us(), clean.emulated_time_us());
        assert_eq!(rt.cpu.regs.read(7), clean.cpu.regs.read(7));
        assert_eq!(rt.soft_reset_count(), 0);
    }

    /// Review Focus 4: peer bytes from the network. Too short (under the
    /// 7-byte address header), oversized (data over 229 bytes), a bad
    /// Address_Type, and a stream of random lengths and bytes are all
    /// refused with `false`, without a panic, and nothing is queued for
    /// the firmware. A well-formed advertisement is refused too here: no
    /// app is scanning, and nothing is held for later.
    #[test]
    fn deliver_rejects_malformed_bytes() {
        use crate::radio::LINKED_PEER_RSSI;
        let mut rt = FirmwareRuntime::from_image(&synthetic_image()).unwrap();
        let mut ok = vec![0x02, 0x10, 0, 0, 0, 0xC2, 1];
        ok.extend_from_slice(&[0x03, 0xFF, 0xFF, 0xFF]);
        let mut oversized = vec![0u8; 7 + 230];
        oversized[6] = 1;
        let mut bad_type = ok.clone();
        bad_type[6] = 4;
        for bytes in [
            &[][..],
            &[0x02],
            &ok[..6],
            &oversized[..],
            &bad_type[..],
            &[0xFF; 7][..],
        ] {
            assert!(
                !rt.deliver_advertisement_bytes(bytes, LINKED_PEER_RSSI),
                "{} bytes",
                bytes.len()
            );
        }
        // xorshift32: deterministic random lengths (0 to 299) and bytes.
        let mut x: u32 = 0x1234_5678;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for _ in 0..2_000 {
            let len = (next() % 300) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let rssi = next() as i8;
            assert!(!rt.deliver_advertisement_bytes(&bytes, rssi));
        }
        assert!(
            !rt.deliver_advertisement_bytes(&ok, LINKED_PEER_RSSI),
            "not scanning: dropped, not queued"
        );
        assert!(!rt.radio().controller().has_events());
        assert_eq!(rt.radio_advertisement(), None);
        assert!(rt.radio_advertisement_bytes().is_empty());
        assert_eq!(rt.run(100).steps, 100, "the guest runs on");
    }
}

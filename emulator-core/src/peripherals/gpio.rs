//! ESP32-C3 GPIO peripheral (`DR_REG_GPIO_BASE = 0x6000_4000`) plus an
//! emulated 74HC165 parallel-in/serial-out shift register, since the real
//! badge reads 7 of its 8 buttons through that chip rather than via 8 direct
//! GPIO pins.
//!
//! Register layout confirmed against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/gpio_reg.h` (fetched directly via
//! `gh api` this task, not guessed) and the base address against that same
//! version's `soc/reg_base.h`.
//!
//! ## Pin map (task brief; third-party-researched, not official docs)
//!
//! Confirmed this task via a community member's own working custom ESP-IDF
//! firmware for this exact badge model
//! (`github.com/abigail-liang/hack-the-north-2026-badge`, `firmware/main.c`)
//! — high-confidence, verified against real hardware by its author, but not
//! Hack the North's own documentation:
//!
//! | Signal        | GPIO | Role                                            |
//! |---------------|------|--------------------------------------------------|
//! | `HC165_DATA`  | 7    | shift register serial out -> CPU (GPIO input)   |
//! | `PIN_START`   | 9    | START button, direct GPIO (also a BOOT strap)   |
//! | `HC165_LOAD`  | 20   | shift register parallel-load/latch (GPIO output) |
//! | `HC165_CLK`   | 21   | shift register shift clock (GPIO output)        |
//!
//! ## Modeled registers
//!
//! - `GPIO_OUT_REG` (0x04) / `GPIO_OUT_W1TS_REG` (0x08, write-1-to-set) /
//!   `GPIO_OUT_W1TC_REG` (0x0C, write-1-to-clear): all three read/write the
//!   *same* underlying `out` word (bits `[25:0]`, one per pin) — `W1TS`/
//!   `W1TC` are just OR-in/AND-out-mask conveniences real firmware commonly
//!   uses instead of a read-modify-write on `OUT_REG` directly. All three
//!   styles are exercised in this module's tests and must produce equivalent
//!   results.
//! - `GPIO_ENABLE_REG` (0x20) / `_W1TS` (0x24) / `_W1TC` (0x28): same
//!   pattern, for the per-pin output-enable/direction bit (`1` = pin is
//!   configured as an output).
//! - `GPIO_IN_REG` (0x3C, read-only): per pin, if the pin is enabled as an
//!   output, reflects the driven `out` bit (reading back your own output is
//!   normal on real hardware); otherwise reflects that pin's *external*
//!   input state — see [`Gpio::pin_in_level`] for the per-pin rules (pin 7
//!   is driven by [`Hc165`], pin 9 by [`Gpio::set_start_pressed`], every
//!   other pin by a generic external-input bit that defaults to `0`).
//!
//! Every other `GPIO_*_REG` this module doesn't name (pull-up/down config,
//! strapping, interrupt-status, IO-MUX-adjacent registers, etc. — none of
//! which this task's button-reading path touches) falls through to a real-
//! storage-free "read 0 / write ignored" default, the same catch-all
//! philosophy `systimer`/`intc` already use for their own not-yet-modeled
//! registers.
//!
//! ## The 74HC165 model ([`Hc165`])
//!
//! Standard parallel-in/serial-out shift register behavior. This model
//! exposes **8 generically-numbered raw input slots**: slot 0 is *not*
//! wired to the shift register at all (it's `PIN_START`'s separate direct
//! GPIO9 line, handled by [`Gpio::set_start_pressed`]); the shift register
//! itself carries exactly 7 real button slots (indices `0..7` of
//! [`Hc165::set_button`]) plus 1 constant/unused 8th bit. Which of the 7
//! shift-register bit positions corresponds to which named button
//! (A/B/HOME/UP/DOWN/LEFT/RIGHT/AUX1) is explicitly out of scope for this
//! task (see the task brief) — slots are addressed purely by index.
//!
//! **The unused 8th bit**: modeled as a constant, always-"released" (logical
//! high) input. We don't know what the real chip's 8th parallel input pin is
//! tied to; "always released" is an arbitrary but documented choice, chosen
//! because it can never be mistaken for a real button held down.
//!
//! **Bit order chosen for shift-out** (documented per the brief's
//! instruction to pick *a* consistent order rather than guess the real
//! physical wiring, since we don't have the real per-bit button identity
//! mapping anyway): the bit presented on `HC165_DATA` immediately after a
//! `LOAD` pulse (before any `CLK` edges) is button slot 0; each subsequent
//! `CLK` rising edge advances to the next slot (1, 2, ..., 6); the 8th and
//! final presented bit (after the 7th `CLK` edge) is the constant/unused
//! bit. So the full presented sequence for one load+7-clocks cycle is:
//! `[slot0, slot1, slot2, slot3, slot4, slot5, slot6, constant]`. This is
//! *not* a literal transcription of the real 74HC165's physical
//! A..H/QH-first pin order (which we don't have a source for on this
//! board) — it's a simple, testable, ascending-index convention. A later
//! task with real per-bit button identity should revisit which slot index
//! maps to which physical pin.
//!
//! **`LOAD` edge-detection choice**: a real 74HC165's `SH/LD̄` pin is active
//! low and level-triggered (while held low, the shift register
//! continuously/asynchronously mirrors the parallel inputs; raising it back
//! high freezes whatever was latched at that instant and re-enables
//! clocking). This model does not simulate the asynchronous low-level
//! passthrough (no continuous re-sampling while `LOAD` is held low) — it
//! latches a single snapshot of the 7 button slots (+ the constant bit) at
//! the **rising edge** of `LOAD` (`0 -> 1` transition on GPIO20's driven
//! output level) and resets the shift position back to the start of the
//! sequence. A real firmware "pulse" (drive low, then high) therefore
//! produces exactly the latch this model implements, at the moment `LOAD`
//! returns high — the falling edge itself is a no-op in this model (there is
//! nothing further to do until the level rises again). This satisfies the
//! brief's requirement that a slot changed *after* `LOAD` but *before* the
//! shift-out finishes must not affect the in-progress read: the snapshot is
//! a frozen copy taken once, at that rising edge, independent of whatever
//! [`Hc165::set_button`] is called with afterward.
//!
//! **`CLK` edge-detection choice**: a rising edge (`0 -> 1` transition on
//! GPIO21's driven output level) advances the shift position by one, *but
//! only while `LOAD` is currently high* (matching real 74HC165 behavior:
//! `SH/LD̄` low overrides/inhibits the clock input entirely). A `CLK` rising
//! edge that occurs while `LOAD` is still low is ignored. The shift position
//! saturates at the last (8th, constant) bit — this model does not simulate
//! a serial-in pin or a daisy-chained second chip, so clocking past the 8th
//! bit simply keeps re-presenting that same constant bit rather than
//! shifting in undefined data.
//!
//! **Reset/pre-`LOAD` state**: before any `LOAD` pulse has ever happened,
//! the latched snapshot defaults to "all released" (every slot high,
//! including the constant bit) and the shift position starts at index 0 —
//! i.e. clocking (or just reading `HC165_DATA`) before the first `LOAD`
//! behaves exactly as if an all-released `LOAD` had already happened. This
//! is a documented default, not a reverse-engineered reset value (real
//! 74HC165 shift-register contents are undefined at power-on).

use super::set_byte;

pub const OUT_REG: u32 = 0x04;
pub const OUT_W1TS_REG: u32 = 0x08;
pub const OUT_W1TC_REG: u32 = 0x0C;
pub const ENABLE_REG: u32 = 0x20;
pub const ENABLE_W1TS_REG: u32 = 0x24;
pub const ENABLE_W1TC_REG: u32 = 0x28;
pub const IN_REG: u32 = 0x3C;

/// Pin bit positions this peripheral gives special external-device
/// behavior to; see the module doc's pin map.
pub const PIN_HC165_DATA: u32 = 7;
pub const PIN_START: u32 = 9;
pub const PIN_HC165_LOAD: u32 = 20;
pub const PIN_HC165_CLK: u32 = 21;

/// `GPIO_OUT_DATA`/`GPIO_ENABLE_DATA` are documented (per the fetched
/// header) as 26 bits wide (`[25:0]`); mask any word write to that width so
/// out-of-range bits can't be set via a stray 32-bit write.
const PIN_DATA_MASK: u32 = 0x03FF_FFFF;

/// The 7-button-plus-1-constant 74HC165 shift register model feeding
/// `HC165_DATA` (GPIO7). See the module doc for the exact edge-detection and
/// bit-order choices this implements.
pub struct Hc165 {
    /// The 7 real button slots, settable at any time (by a test harness or,
    /// later, real UI wiring) — these are *not* what gets shifted out;
    /// [`Hc165::latched`] is the frozen snapshot taken at the last `LOAD`
    /// rising edge.
    raw_slots: [bool; 7],
    /// Frozen snapshot as of the last `LOAD` rising edge: `latched[0..7]` =
    /// `raw_slots` at that instant, `latched[7]` = the constant unused bit
    /// (always `true` = released). Defaults to all-released (see module
    /// doc's "Reset/pre-LOAD state").
    latched: [bool; 8],
    /// How many `CLK` rising edges (that were honored, i.e. occurred while
    /// `LOAD` was high) have occurred since the last latch; also the index
    /// into `latched` currently presented on `HC165_DATA`. Saturates at 7.
    shift_pos: usize,
    /// Last observed driven level of `HC165_LOAD` (GPIO20's `out` bit), for
    /// edge detection. Real hardware idle level for `SH/LD̄` is high
    /// (not-asserted); default `true` here matches that.
    prev_load_level: bool,
    /// Last observed driven level of `HC165_CLK` (GPIO21's `out` bit), for
    /// edge detection.
    prev_clk_level: bool,
}

impl Default for Hc165 {
    fn default() -> Self {
        Self {
            raw_slots: [false; 7],
            latched: [true; 8], // all-released default; see module doc
            shift_pos: 0,
            prev_load_level: true,
            prev_clk_level: false,
        }
    }
}

impl Hc165 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets raw button slot `idx` (`0..7`) pressed/released. Takes effect on
    /// the *next* `LOAD` rising edge — does not retroactively affect a
    /// snapshot already latched and being shifted out (see module doc).
    /// Out-of-range `idx` (`>= 7`) is silently ignored — this model only has
    /// 7 real button slots.
    pub fn set_button(&mut self, idx: usize, pressed: bool) {
        if let Some(slot) = self.raw_slots.get_mut(idx) {
            *slot = pressed;
        }
    }

    /// The bit currently presented on `HC165_DATA`'s input level: `true` =
    /// released/high, `false` = pressed/low — i.e. this method returns the
    /// GPIO *level*, matching the "pressed pulls low" active-low convention
    /// [`Gpio`] uses uniformly for every button-shaped input in this module
    /// (see [`Gpio::set_start_pressed`]'s doc for the same convention).
    pub fn data_level(&self) -> bool {
        self.latched[self.shift_pos]
    }

    /// Called whenever `HC165_LOAD` (GPIO20)'s driven output level changes.
    /// See the module doc's "LOAD edge-detection choice."
    fn on_load_level_changed(&mut self, new_level: bool) {
        if !self.prev_load_level && new_level {
            // Rising edge: freeze a fresh snapshot, restart the shift
            // sequence. `raw_slots` stores "pressed" booleans (true =
            // pressed); `latched` stores *levels* (true = released/high,
            // active-low convention — see `data_level`'s doc), so each slot
            // is inverted going in.
            for i in 0..7 {
                self.latched[i] = !self.raw_slots[i];
            }
            self.latched[7] = true; // constant/unused 8th bit, always released
            self.shift_pos = 0;
        }
        self.prev_load_level = new_level;
    }

    /// Called whenever `HC165_CLK` (GPIO21)'s driven output level changes.
    /// See the module doc's "CLK edge-detection choice."
    fn on_clk_level_changed(&mut self, new_level: bool) {
        if !self.prev_clk_level && new_level && self.prev_load_level {
            // Rising edge, and LOAD is currently high (clocking honored):
            // advance to the next bit, saturating at the last one.
            self.shift_pos = (self.shift_pos + 1).min(7);
        }
        self.prev_clk_level = new_level;
    }
}

/// The GPIO peripheral: per-pin output level / output-enable storage for
/// all 26 documented pin-data bits, plus the external-input rules and the
/// [`Hc165`] model described in the module doc.
pub struct Gpio {
    /// `GPIO_OUT_REG`: bit per pin, driven output level.
    out: u32,
    /// `GPIO_ENABLE_REG`: bit per pin, `1` = pin configured as output.
    enable: u32,
    /// `PIN_START` (GPIO9)'s external input state: `true` = pressed.
    /// Defaults to not-pressed. See [`Gpio::set_start_pressed`].
    start_pressed: bool,
    /// Every other pin's external input level (no special device behavior
    /// wired to it in this task) — default `false`/low, real read/write-
    /// backed storage rather than silently dropped, per the brief. Not
    /// consulted for pins 7/9/20/21, which have their own special-cased
    /// rules.
    external_in: u32,
    pub hc165: Hc165,
}

impl Default for Gpio {
    fn default() -> Self {
        Self {
            out: 0,
            enable: 0,
            start_pressed: false,
            external_in: 0,
            hc165: Hc165::new(),
        }
    }
}

impl Gpio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `PIN_START` (GPIO9)'s external input: `pressed = true` drives
    /// the level low (active-low, pull-up convention — matches the same
    /// convention [`Hc165`]'s button slots use), `false` drives it high
    /// (released). Has no effect on `GPIO_IN_REG`'s bit 9 if firmware has
    /// configured pin 9 as an output (its own driven level wins instead,
    /// same rule every pin follows).
    pub fn set_start_pressed(&mut self, pressed: bool) {
        self.start_pressed = pressed;
    }

    fn pin_bit(pin: u32) -> u32 {
        1 << pin
    }

    /// `GPIO_IN_REG` bit for `pin`: driven output level if `pin` is
    /// configured as an output, else that pin's external input rule (see
    /// module doc).
    fn pin_in_level(&self, pin: u32) -> bool {
        let bit = Self::pin_bit(pin);
        if self.enable & bit != 0 {
            return self.out & bit != 0;
        }
        match pin {
            PIN_HC165_DATA => self.hc165.data_level(),
            PIN_START => !self.start_pressed, // active-low
            _ => self.external_in & bit != 0,
        }
    }

    /// The live `GPIO_IN_REG`-equivalent level for a single `pin` (`0..26`)
    /// — same semantics as one bit of [`Gpio::in_word`], but as a targeted
    /// single-pin read that doesn't require reconstructing the whole 26-bit
    /// word. Added for Task 5's SPI/ST7789 peripheral, which needs to read
    /// GPIO0's live driven level (the D/C line) at the exact moment a SPI
    /// transaction is triggered — a direct, synchronous field read on the
    /// concrete `Gpio` field `FirmwareBus` already holds, per this plan's
    /// "no trait-object peripheral dispatch" ruling.
    pub fn pin_level(&self, pin: u32) -> bool {
        self.pin_in_level(pin)
    }

    fn in_word(&self) -> u32 {
        let mut word = 0u32;
        for pin in 0..26 {
            if self.pin_in_level(pin) {
                word |= Self::pin_bit(pin);
            }
        }
        word
    }

    fn set_out(&mut self, new_out: u32) {
        let new_out = new_out & PIN_DATA_MASK;
        if self.out == new_out {
            return;
        }
        let load_bit = Self::pin_bit(PIN_HC165_LOAD);
        let clk_bit = Self::pin_bit(PIN_HC165_CLK);
        let changed = self.out ^ new_out;
        self.out = new_out;
        if changed & load_bit != 0 {
            self.hc165.on_load_level_changed(new_out & load_bit != 0);
        }
        if changed & clk_bit != 0 {
            self.hc165.on_clk_level_changed(new_out & clk_bit != 0);
        }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            OUT_REG => self.out,
            ENABLE_REG => self.enable,
            IN_REG => self.in_word(),
            // W1TS/W1TC are write-only on real hardware; reads of them fall
            // through to the generic 0 default below, same as everything
            // else this module doesn't name.
            _ => 0,
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            OUT_REG => {
                let mut word = self.out;
                set_byte(&mut word, idx, val);
                self.set_out(word);
            }
            OUT_W1TS_REG => {
                let mut mask = 0u32;
                set_byte(&mut mask, idx, val);
                self.set_out(self.out | mask);
            }
            OUT_W1TC_REG => {
                let mut mask = 0u32;
                set_byte(&mut mask, idx, val);
                self.set_out(self.out & !mask);
            }
            ENABLE_REG => {
                set_byte(&mut self.enable, idx, val);
                self.enable &= PIN_DATA_MASK;
            }
            ENABLE_W1TS_REG => {
                let mut mask = 0u32;
                set_byte(&mut mask, idx, val);
                self.enable |= mask;
                self.enable &= PIN_DATA_MASK;
            }
            ENABLE_W1TC_REG => {
                let mut mask = 0u32;
                set_byte(&mut mask, idx, val);
                self.enable &= !mask;
            }
            // GPIO_IN_REG is read-only; everything else this module doesn't
            // name is accepted and dropped, matching FirmwareBus's/
            // systimer's own catch-all philosophy so firmware probing
            // unrelated GPIO registers (pull-up/down, strapping, IO-MUX,
            // interrupt status, etc.) doesn't panic.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(g: &mut Gpio, word_offset: u32, val: u32) {
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            g.write_byte(word_offset + i as u32, *b);
        }
    }
    fn read_word(g: &mut Gpio, word_offset: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = g.read_byte(word_offset + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    // ---- GPIO register unit tests ----

    #[test]
    fn out_reg_direct_write_sets_driven_level_for_an_output_pin() {
        let mut g = Gpio::new();
        write_word(&mut g, ENABLE_REG, 1 << 21); // pin 21 as output
        write_word(&mut g, OUT_REG, 1 << 21);
        assert_eq!(read_word(&mut g, IN_REG) & (1 << 21), 1 << 21);
        write_word(&mut g, OUT_REG, 0);
        assert_eq!(read_word(&mut g, IN_REG) & (1 << 21), 0);
    }

    #[test]
    fn out_w1ts_and_w1tc_affect_only_masked_bits() {
        let mut g = Gpio::new();
        write_word(&mut g, ENABLE_REG, (1 << 20) | (1 << 21));
        write_word(&mut g, OUT_REG, 1 << 20); // pin20 set, pin21 clear
        write_word(&mut g, OUT_W1TS_REG, 1 << 21); // set pin21 only
        assert_eq!(read_word(&mut g, OUT_REG), (1 << 20) | (1 << 21));
        write_word(&mut g, OUT_W1TC_REG, 1 << 20); // clear pin20 only
        assert_eq!(read_word(&mut g, OUT_REG), 1 << 21);
    }

    #[test]
    fn out_reg_w1ts_w1tc_all_produce_equivalent_results() {
        // Three different ways to reach the same final `out` value must
        // agree.
        let mut g1 = Gpio::new();
        write_word(&mut g1, OUT_REG, 0b101);

        let mut g2 = Gpio::new();
        write_word(&mut g2, OUT_W1TS_REG, 0b101);

        let mut g3 = Gpio::new();
        write_word(&mut g3, OUT_W1TS_REG, 0b111);
        write_word(&mut g3, OUT_W1TC_REG, 0b010);

        assert_eq!(read_word(&mut g1, OUT_REG), 0b101);
        assert_eq!(read_word(&mut g2, OUT_REG), 0b101);
        assert_eq!(read_word(&mut g3, OUT_REG), 0b101);
    }

    #[test]
    fn enable_reg_and_w1ts_w1tc_control_direction_bits() {
        let mut g = Gpio::new();
        write_word(&mut g, ENABLE_REG, 1 << 5);
        assert_eq!(read_word(&mut g, ENABLE_REG), 1 << 5);
        write_word(&mut g, ENABLE_W1TS_REG, 1 << 6);
        assert_eq!(read_word(&mut g, ENABLE_REG), (1 << 5) | (1 << 6));
        write_word(&mut g, ENABLE_W1TC_REG, 1 << 5);
        assert_eq!(read_word(&mut g, ENABLE_REG), 1 << 6);
    }

    #[test]
    fn in_reg_reflects_external_input_when_not_output_enabled() {
        let mut g = Gpio::new();
        // Pin 3: not enabled as output, no special device -> external_in
        // default (0/low).
        assert_eq!(read_word(&mut g, IN_REG) & (1 << 3), 0);
    }

    #[test]
    fn in_reg_reflects_driven_output_when_output_enabled_even_with_special_pins() {
        // Pin 7 (HC165_DATA) is normally driven by the shift register, but
        // if firmware (incorrectly, or for board-bringup testing) configures
        // it as an output, GPIO_IN_REG must reflect the driven level
        // instead -- "reading back your own output" per the brief.
        let mut g = Gpio::new();
        write_word(&mut g, ENABLE_REG, 1 << PIN_HC165_DATA);
        write_word(&mut g, OUT_REG, 1 << PIN_HC165_DATA);
        assert_eq!(
            read_word(&mut g, IN_REG) & (1 << PIN_HC165_DATA),
            1 << PIN_HC165_DATA
        );
    }

    // ---- PIN_START unit test ----

    #[test]
    fn pin_start_reflects_the_boolean_setter_directly() {
        let mut g = Gpio::new();
        assert_eq!(
            read_word(&mut g, IN_REG) & (1 << PIN_START),
            1 << PIN_START,
            "not pressed by default -> released -> high"
        );
        g.set_start_pressed(true);
        assert_eq!(
            read_word(&mut g, IN_REG) & (1 << PIN_START),
            0,
            "pressed -> active-low -> level low"
        );
        g.set_start_pressed(false);
        assert_eq!(read_word(&mut g, IN_REG) & (1 << PIN_START), 1 << PIN_START);
    }

    // ---- 74HC165 model unit tests ----

    /// Bit-bangs a LOAD pulse (drive low, then high) directly on the `Gpio`
    /// struct's OUT_REG, assuming GPIO20/21 are already configured as
    /// outputs.
    fn pulse_load(g: &mut Gpio) {
        let load_bit = 1 << PIN_HC165_LOAD;
        let cur = read_word(g, OUT_REG);
        write_word(g, OUT_REG, cur & !load_bit); // LOAD low (asserted)
        write_word(g, OUT_REG, cur | load_bit); // LOAD high (rising edge: latch)
    }

    fn clk_rising_edge(g: &mut Gpio) {
        let clk_bit = 1 << PIN_HC165_CLK;
        let cur = read_word(g, OUT_REG);
        write_word(g, OUT_REG, cur & !clk_bit);
        write_word(g, OUT_REG, cur | clk_bit);
    }

    fn setup_shift_reg_pins(g: &mut Gpio) {
        write_word(g, ENABLE_REG, (1 << PIN_HC165_LOAD) | (1 << PIN_HC165_CLK));
        // Start both idle-high (LOAD not-asserted, CLK low), matching real
        // firmware's typical idle state.
        write_word(g, OUT_REG, 1 << PIN_HC165_LOAD);
    }

    fn read_data_bit(g: &mut Gpio) -> bool {
        read_word(g, IN_REG) & (1 << PIN_HC165_DATA) != 0
    }

    #[test]
    fn shift_register_presents_set_slots_in_documented_ascending_order() {
        let mut g = Gpio::new();
        setup_shift_reg_pins(&mut g);
        // Press slots 1, 3, 5 (0-indexed); active-low -> pressed -> level 0.
        g.hc165.set_button(1, true);
        g.hc165.set_button(3, true);
        g.hc165.set_button(5, true);

        pulse_load(&mut g);

        // Expected sequence: slot0(released=1) slot1(pressed=0)
        // slot2(released=1) slot3(pressed=0) slot4(released=1)
        // slot5(pressed=0) slot6(released=1) constant(released=1).
        let expected = [true, false, true, false, true, false, true, true];
        let mut observed = Vec::new();
        observed.push(read_data_bit(&mut g)); // bit presented immediately after LOAD, before any CLK
        for _ in 0..7 {
            clk_rising_edge(&mut g);
            observed.push(read_data_bit(&mut g));
        }
        assert_eq!(observed, expected);
    }

    #[test]
    fn clocking_without_a_prior_load_uses_the_documented_default_snapshot() {
        let mut g = Gpio::new();
        setup_shift_reg_pins(&mut g);
        // Never call pulse_load: slots set now must NOT be visible (no load
        // pulse latched them) -- the default all-released snapshot is what
        // gets shifted out.
        g.hc165.set_button(0, true);

        let mut observed = Vec::new();
        observed.push(read_data_bit(&mut g));
        for _ in 0..7 {
            clk_rising_edge(&mut g);
            observed.push(read_data_bit(&mut g));
        }
        assert_eq!(
            observed, [true; 8],
            "pre-LOAD default snapshot must be all-released, ignoring \
             raw_slots writes that never got latched"
        );
    }

    #[test]
    fn slot_change_after_load_does_not_affect_in_progress_shift_out() {
        let mut g = Gpio::new();
        setup_shift_reg_pins(&mut g);
        g.hc165.set_button(2, true); // slot2 pressed before LOAD
        pulse_load(&mut g);

        // Read slot0/slot1 (both released, unaffected either way), then --
        // mid shift-out, before slot2/slot4's positions have been read --
        // flip slot2 back to released AND press slot4 (previously
        // released). Neither change has been shifted out yet.
        observed_first_two(&mut g);
        g.hc165.set_button(2, false);
        g.hc165.set_button(4, true);

        // Continue clocking out the rest; the frozen snapshot must still
        // reflect the state *at LOAD time*, not the just-made changes: slot2
        // still reads pressed (it *was* pressed at LOAD time), slot4 still
        // reads released (it was *not* pressed at LOAD time).
        let mut rest = Vec::new();
        for _ in 0..5 {
            clk_rising_edge(&mut g);
            rest.push(read_data_bit(&mut g));
        }
        // index2=slot2(pressed=false) index3=slot3(released) index4=slot4
        // (released, NOT the just-pressed value) index5=slot5(released)
        // index6=slot6(released).
        assert_eq!(rest, [false, true, true, true, true]);
    }

    fn observed_first_two(g: &mut Gpio) -> [bool; 2] {
        let b0 = read_data_bit(g); // slot0
        clk_rising_edge(g);
        let b1 = read_data_bit(g); // slot1
        [b0, b1]
    }

    #[test]
    fn clk_edges_while_load_is_low_are_ignored() {
        let mut g = Gpio::new();
        setup_shift_reg_pins(&mut g);
        g.hc165.set_button(0, true);
        g.hc165.set_button(1, true);
        pulse_load(&mut g); // latches slot0=pressed, slot1=pressed, ...

        // Drive LOAD low (asserted) without a matching rising edge yet, and
        // try to clock while it's low.
        let load_bit = 1 << PIN_HC165_LOAD;
        let cur = read_word(&mut g, OUT_REG);
        write_word(&mut g, OUT_REG, cur & !load_bit); // LOAD low
        clk_rising_edge(&mut g); // must be ignored: LOAD is low
        assert!(
            !read_data_bit(&mut g), // still presenting slot0 (pressed)
            "CLK rising edge while LOAD is low must be ignored"
        );

        // Raise LOAD back high: this itself is a fresh latch (rising edge),
        // re-freezing the *current* raw_slots (both slots still pressed).
        write_word(&mut g, OUT_REG, cur | load_bit);
        assert!(!read_data_bit(&mut g), "slot0 still pressed");
        clk_rising_edge(&mut g);
        assert!(!read_data_bit(&mut g), "slot1 still pressed");
    }

    #[test]
    fn shift_position_saturates_at_the_8th_bit_on_extra_clocks() {
        let mut g = Gpio::new();
        setup_shift_reg_pins(&mut g);
        pulse_load(&mut g);
        for _ in 0..7 {
            clk_rising_edge(&mut g);
        }
        assert!(read_data_bit(&mut g), "constant bit, released");
        for _ in 0..5 {
            clk_rising_edge(&mut g);
            assert!(
                read_data_bit(&mut g),
                "extra clocks past the 8th bit keep presenting the constant bit"
            );
        }
    }
}

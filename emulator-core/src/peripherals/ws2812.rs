//! The badge's WS2812B LED chain, as a passive decoder of the RMT symbols
//! the firmware transmits on `LED_DIN` (GPIO3). Board data, not chip data:
//! [`crate::peripherals::rmt::Rmt`] feeds it every word its transmitter
//! consumes, and it never writes anything back, so it cannot change a
//! step, an interrupt or a byte the firmware sees (Milestone 14 Task 2;
//! `docs/milestone-14-decisions.md`, "LEDs").
//!
//! ## Sources
//!
//! - Symbol layout: ESP-IDF v5.5.3 `components/hal/include/hal/rmt_types.h`
//!   (`rmt_symbol_word_t`: `duration0` 14:0, `level0` 15, `duration1`
//!   30:16, `level1` 31), as [`crate::peripherals::rmt`] cites it.
//! - The firmware's encoder (v392, observed through the RMT model on TX
//!   channel 0, divider 8 on the 80 MHz APB clock, so 0.1 µs a tick): a 1
//!   is `0x0003_8009` (high 9 ticks, low 3: 0.9/0.3 µs), a 0 is
//!   `0x0009_8003` (high 3, low 9: 0.3/0.9 µs), MSB first, G, R, B per
//!   LED; each frame is 144 bit symbols (6 LEDs), then the reset code
//!   `0x0578_0578` (low 1400 + 1400 ticks, 280 µs) and a zero end-marker
//!   (`rmt_tx_mark_eof`). This is the `led_strip` component's RMT encoder
//!   shape (a bytes encoder for the GRB bytes, then a copy of the reset
//!   symbol).
//! - The LED: the WS2812B datasheet (Worldsemi): a 0 is T0H 0.4 µs high,
//!   T0L 0.85 µs low; a 1 is T1H 0.8 µs high, T1L 0.45 µs low (each
//!   ±150 ns); 24 bits per LED, G7..G0, R7..R0, B7..B0, MSB first; a low
//!   longer than the reset time (RES, above 50 µs) latches the frame.
//!
//! ## Decoding
//!
//! - **Bits by relative timing.** A symbol with `level0 = 1` and both
//!   durations non-zero is a bit: 1 when `duration0 > duration1`, else 0.
//!   For any encoder that keeps to the datasheet (T1H > T1L and
//!   T0H < T0L) this is the same answer as comparing the high time with
//!   a threshold in the RMT clock, and it needs no clock divider
//!   (`docs/milestone-14-decisions.md`, ruling R1).
//! - **Bytes.** Every 24 bits is one LED, in G, R, B order, stored as RGB
//!   in chain order.
//! - **Frame end.** Any other symbol (the reset code's long low, an
//!   end-marker) or [`Ws2812::end_of_transmission`] ends the frame: if at
//!   least one LED was assembled it replaces [`Ws2812::colors`] and
//!   [`Ws2812::frame_count`] goes up by one. Partial trailing bits are
//!   dropped. A frame keeps at most [`MAX_LEDS`] LEDs; bits past that are
//!   ignored, so a runaway transmission stays bounded.
//!
//! The chain is outside the SoC: like the panel, it keeps its colours
//! through a software reset (`FirmwareRuntime`'s soft reset carries it),
//! and a cold power-on clears it.

/// The most LEDs one frame keeps (the badge has 6).
pub const MAX_LEDS: usize = 256;

/// The LED chain's latest frame and the one being assembled. See the
/// module doc.
#[derive(Clone, Default)]
pub struct Ws2812 {
    /// The latest complete frame, RGB in chain order.
    colors: Vec<[u8; 3]>,
    frame_count: u32,
    /// The frame being assembled.
    building: Vec<[u8; 3]>,
    /// The current LED's bits so far (MSB first) and their count.
    bits: u32,
    nbits: u32,
}

impl Ws2812 {
    /// One RMT symbol word, as the transmitter sends it.
    pub fn symbol(&mut self, word: u32) {
        let duration0 = word & 0x7FFF;
        let level0 = word >> 15 & 1;
        let duration1 = word >> 16 & 0x7FFF;
        if level0 != 1 || duration0 == 0 || duration1 == 0 {
            self.end_of_transmission();
            return;
        }
        self.bits = self.bits << 1 | u32::from(duration0 > duration1);
        self.nbits += 1;
        if self.nbits == 24 {
            let [_, g, r, b] = self.bits.to_be_bytes();
            if self.building.len() < MAX_LEDS {
                self.building.push([r, g, b]);
            }
            self.bits = 0;
            self.nbits = 0;
        }
    }

    /// The transmitter stopped: publishes the frame, if it has an LED.
    pub fn end_of_transmission(&mut self) {
        self.bits = 0;
        self.nbits = 0;
        if self.building.is_empty() {
            return;
        }
        self.colors.clear();
        self.colors.append(&mut self.building);
        self.frame_count = self.frame_count.wrapping_add(1);
    }

    /// The latest frame, RGB in chain order; empty until the first.
    pub fn colors(&self) -> &[[u8; 3]] {
        &self.colors
    }

    /// Frames published since power-on.
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An RMT symbol word (`rmt_symbol_word_t`): `duration0` 14:0,
    /// `level0` 15, `duration1` 30:16, `level1` 31.
    fn word(level0: u32, d0: u32, level1: u32, d1: u32) -> u32 {
        (d0 & 0x7FFF) | (level0 << 15) | ((d1 & 0x7FFF) << 16) | (level1 << 31)
    }

    /// The firmware's bits (observed): 1 is high 9 ticks, low 3; 0 is
    /// high 3, low 9 (`0x0003_8009`, `0x0009_8003`).
    const BIT1: u32 = 0x0003_8009;
    const BIT0: u32 = 0x0009_8003;
    /// The firmware's reset code: low for 1400 + 1400 ticks.
    const RESET: u32 = 0x0578_0578;

    fn send_byte(d: &mut Ws2812, byte: u8, one: u32, zero: u32) {
        for i in (0..8).rev() {
            d.symbol(if byte >> i & 1 == 1 { one } else { zero });
        }
    }

    fn send_grb(d: &mut Ws2812, [r, g, b]: [u8; 3]) {
        for byte in [g, r, b] {
            send_byte(d, byte, BIT1, BIT0);
        }
    }

    #[test]
    fn the_test_words_are_the_firmwares() {
        assert_eq!(BIT1, word(1, 9, 0, 3));
        assert_eq!(BIT0, word(1, 3, 0, 9));
        assert_eq!(RESET, word(0, 1400, 0, 1400));
    }

    #[test]
    fn decodes_grb_bytes_msb_first() {
        let mut d = Ws2812::default();
        for byte in [0x10, 0x20, 0x30] {
            send_byte(&mut d, byte, BIT1, BIT0);
        }
        d.symbol(0); // rmt_tx_mark_eof's end-marker
        assert_eq!(d.colors(), &[[0x20, 0x10, 0x30]]);
        assert_eq!(d.frame_count(), 1);
    }

    #[test]
    fn two_leds_in_chain_order() {
        let mut d = Ws2812::default();
        send_grb(&mut d, [1, 2, 3]);
        send_grb(&mut d, [0xFF, 0x80, 0x01]);
        d.symbol(RESET);
        assert_eq!(d.colors(), &[[1, 2, 3], [0xFF, 0x80, 0x01]]);
        assert_eq!(d.frame_count(), 1);
    }

    #[test]
    fn partial_bits_are_dropped() {
        let mut d = Ws2812::default();
        send_grb(&mut d, [9, 8, 7]);
        for _ in 0..6 {
            d.symbol(BIT1);
        }
        d.end_of_transmission();
        assert_eq!(d.colors(), &[[9, 8, 7]]);
        assert_eq!(d.frame_count(), 1);
    }

    #[test]
    fn no_bits_no_frame() {
        let mut d = Ws2812::default();
        d.end_of_transmission();
        d.symbol(RESET);
        d.symbol(0);
        assert!(d.colors().is_empty());
        assert_eq!(d.frame_count(), 0);
    }

    #[test]
    fn relative_timing_only() {
        let decode = |one: u32, zero: u32| {
            let mut d = Ws2812::default();
            for byte in [0xA5, 0x3C, 0x81] {
                send_byte(&mut d, byte, one, zero);
            }
            d.end_of_transmission();
            d.colors().to_vec()
        };
        let fast = decode(word(1, 8, 0, 4), word(1, 4, 0, 8));
        let slow = decode(word(1, 64, 0, 32), word(1, 32, 0, 64));
        assert_eq!(fast, vec![[0x3C, 0xA5, 0x81]]);
        assert_eq!(fast, slow);
    }

    #[test]
    fn the_reset_code_and_the_end_both_end_one_frame() {
        let mut d = Ws2812::default();
        send_grb(&mut d, [1, 2, 3]);
        d.symbol(RESET);
        d.symbol(0);
        d.end_of_transmission();
        assert_eq!(d.frame_count(), 1, "one frame, not three");
    }

    #[test]
    fn a_new_frame_replaces_the_last() {
        let mut d = Ws2812::default();
        send_grb(&mut d, [1, 2, 3]);
        send_grb(&mut d, [4, 5, 6]);
        d.end_of_transmission();
        send_grb(&mut d, [7, 8, 9]);
        d.end_of_transmission();
        assert_eq!(d.colors(), &[[7, 8, 9]]);
        assert_eq!(d.frame_count(), 2);
    }

    #[test]
    fn a_low_or_zero_duration_symbol_is_not_a_bit() {
        let mut d = Ws2812::default();
        send_grb(&mut d, [1, 2, 3]);
        // Mid-LED: level0 = 0, or a zero duration, ends the frame there.
        for _ in 0..4 {
            d.symbol(BIT1);
        }
        d.symbol(word(1, 9, 0, 0));
        assert_eq!(d.colors(), &[[1, 2, 3]]);
        send_grb(&mut d, [4, 5, 6]);
        d.symbol(word(0, 9, 1, 3));
        assert_eq!(d.colors(), &[[4, 5, 6]]);
        assert_eq!(d.frame_count(), 2);
    }
}

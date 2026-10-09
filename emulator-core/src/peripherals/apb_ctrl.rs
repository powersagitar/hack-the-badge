//! SYSCON / APB_CTRL (`DR_REG_SYSCON_BASE = 0x6002_6000`): only the
//! hardware RNG's data register (Milestone 5 Task D-M5-3).
//!
//! Sources (ESP-IDF v5.5.3):
//! - `components/soc/esp32c3/register/soc/apb_ctrl_reg.h:431-437`:
//!   `APB_CTRL_RND_DATA_REG` = base + `0x0B0`, one RO field
//!   `APB_CTRL_RND_DATA`, bits `[31:0]` (default 0). `syscon_reg.h:494`
//!   names the same address `SYSCON_RND_DATA_REG`, and `:502` aliases it
//!   as `RNG_DATA_REG`.
//! - `components/soc/esp32c3/include/soc/wdev_reg.h:13`:
//!   `WDEV_RND_REG` = `RNG_DATA_REG`.
//! - `components/esp_hw_support/hw_random.c`, `esp_random()`: XORs
//!   `REG_READ(WDEV_RND_REG)` reads while spinning on the CPU cycle
//!   counter (see `crate::mem::soc::ESP32C3_CYCLE_COUNTER`), then XORs one
//!   final read into the result. On hardware every read returns a fresh
//!   word from a PRNG fed by a noise source.
//!
//! Model (ruling R-T9-2): a deterministic xorshift32 PRNG (Marsaglia,
//! "Xorshift RNGs", 2003: shifts 13, 17, 5) seeded with [`RNG_SEED`]. Each
//! word read returns the next value, so runs are reproducible and tests can
//! pin frames. It is not the hardware's generator, and it is not entropy:
//! everything that reads it (`esp_random()`, Dice, ...) is deterministic
//! in the emulator. A host-supplied seed is Milestone 6 backlog.
//!
//! Byte-split reads: `FirmwareBus::read32` arrives as four ascending byte
//! reads. Only byte lane 0 advances the generator; lanes 1..=3 return the
//! other bytes of the word lane 0 produced. So one `lw` consumes exactly
//! one value, and a byte read of lane 0 alone also consumes one.
//! Writes are dropped (the field is RO). Every other SYSCON register keeps
//! the bus's logged catch-all behavior (see `FirmwareBus`).

/// `APB_CTRL_RND_DATA_REG` offset from `DR_REG_APB_CTRL_BASE`.
pub const RND_DATA_REG: u32 = 0x0B0;

/// The PRNG's fixed seed: any non-zero value works for xorshift32; this is
/// the 32-bit golden-ratio constant.
pub const RNG_SEED: u32 = 0x9E37_79B9;

#[derive(Clone)]
pub struct ApbCtrl {
    /// The word the last lane-0 read produced.
    rnd: u32,
}

impl Default for ApbCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl ApbCtrl {
    pub fn new() -> Self {
        Self { rnd: RNG_SEED }
    }

    /// `true` iff `offset` is inside the modeled register.
    pub fn handles(offset: u32) -> bool {
        offset & !0b11 == RND_DATA_REG
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        if !Self::handles(offset) {
            return 0;
        }
        let lane = offset & 0b11;
        if lane == 0 {
            self.rnd = xorshift32(self.rnd);
        }
        self.rnd.to_le_bytes()[lane as usize]
    }

    /// RO register: writes are dropped.
    pub fn write_byte(&mut self, _offset: u32, _val: u8) {}
}

fn xorshift32(mut x: u32) -> u32 {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_word(a: &mut ApbCtrl, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| a.read_byte(off + i)))
    }

    #[test]
    fn word_reads_follow_the_xorshift32_sequence_from_the_seed() {
        let mut a = ApbCtrl::new();
        let mut x = RNG_SEED;
        let mut seen = Vec::new();
        for _ in 0..4 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            assert_eq!(read_word(&mut a, RND_DATA_REG), x);
            seen.push(x);
        }
        seen.dedup();
        assert_eq!(seen.len(), 4, "consecutive reads differ");
        assert!(seen.iter().all(|&v| v != 0), "never the reset value 0");
    }

    #[test]
    fn only_byte_lane_zero_advances_the_generator() {
        let mut a = ApbCtrl::new();
        let first = read_word(&mut a, RND_DATA_REG);
        // Re-reading the upper lanes returns the same word's bytes.
        assert_eq!(a.read_byte(RND_DATA_REG + 3), first.to_le_bytes()[3]);
        assert_eq!(a.read_byte(RND_DATA_REG + 1), first.to_le_bytes()[1]);
        let mut b = ApbCtrl::new();
        read_word(&mut b, RND_DATA_REG);
        assert_eq!(
            read_word(&mut a, RND_DATA_REG),
            read_word(&mut b, RND_DATA_REG)
        );
    }

    #[test]
    fn writes_are_dropped_and_clones_replay_the_same_sequence() {
        let mut a = ApbCtrl::new();
        for i in 0..4 {
            a.write_byte(RND_DATA_REG + i, 0);
        }
        let mut b = a.clone();
        assert_eq!(
            read_word(&mut a, RND_DATA_REG),
            read_word(&mut b, RND_DATA_REG)
        );
        assert!(ApbCtrl::handles(0x0B3));
        assert!(!ApbCtrl::handles(0x0AC));
        assert!(!ApbCtrl::handles(0x0B4));
    }
}

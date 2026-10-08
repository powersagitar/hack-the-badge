//! The badge's accelerometer, a Silan **SC7A20H**, as an I2C slave on I2C0
//! at 7-bit address [`ADDRESS`] (`0x19`) (Milestone 5 Task D-M5-2; see
//! `docs/milestone-5-decisions.md`). [`crate::peripherals::i2c::I2c`] owns
//! it as a concrete field and drives it from its command list: an address
//! byte that matches is ACKed and opens a transfer, every byte written
//! after it reaches [`Sc7a20h::write`], every byte read comes from
//! [`Sc7a20h::read`], and a STOP or a repeated START ends the transfer.
//!
//! ## Sources
//!
//! - *SC7A20H 说明书* (Hangzhou Silan Microelectronics), version 0.7:
//!   - §10.1 I²C: 7-bit address `001100x`b, `0x19` with SDO floating or
//!     high (`0x32` write / `0x33` read), `0x18` with SDO low. After the
//!     address the master sends an 8-bit sub-address: bits 6:0 are the
//!     register, bit 7 enables auto-increment for multi-byte reads and
//!     writes (e.g. `0xA8` = `0x28 | 0x80` to burst-read the three axes).
//!     A read is `ST, SAD+W, SUB, SR, SAD+R, data.., NMAK, SP`.
//!   - §11 register list (addresses, r/rw) and §12 register descriptions:
//!     `WHO_AM_I` (`0x0F`) = `0x11`; `CTRL_REG1` (`0x20`: `ODR[3:0]`
//!     7:4, `LPen` 3, `Zen`/`Yen`/`Xen` 2:0, the three enables defaulting
//!     to 1); `CTRL_REG4` (`0x23`: `BDU` 7, `BLE` 6, `FS[1:0]` 5:4);
//!     `DRDY_STATUS_REG` (`0x27`: `ZYXOR`..`XOR` 7:4, `ZYXDA` 3,
//!     `ZDA`/`YDA`/`XDA` 2:0); `OUT_X_L`..`OUT_Z_H` (`0x28`..`0x2D`),
//!     two's complement, 12-bit left-justified (the §12.13 table:
//!     `0x4000` = 1024 = 1.0 g at `FS` = 00); `VERSION` (`0x70`) =
//!     `0x28`.
//!   - §6 mechanical parameters: sensitivity 1 / 2 / 4 / 8 mg/digit at
//!     `FS` = ±2 / ±4 / ±8 / ±16 g.
//! - The I2C transfer shapes come from ESP-IDF v5.5.3
//!   `components/esp_driver_i2c/i2c_master.c` (`i2c_master_transmit`,
//!   `i2c_master_transmit_receive`); the controller side is documented in
//!   `crate::peripherals::i2c`.
//!
//! The firmware (`hal_accel`, traced in `docs/milestone-5-decisions.md`
//! `ACCEL`) confirms this map: it reads `WHO_AM_I` and expects `0x11`,
//! writes `CTRL_REG1` = `0x57` and `CTRL_REG4` = `0x80`, polls
//! `DRDY_STATUS_REG` bit 3, burst-reads six bytes from `0x28 | 0x80`, and
//! takes each axis as `(i16)(H << 8 | L) >> 4` counts of 1 mg.
//!
//! ## What is modeled
//!
//! - **Register file**, `0x00..=0x7F`: the `rw` registers of §11 store
//!   what is written; read-only and reserved registers ignore writes.
//!   Reset values: `WHO_AM_I` `0x11`, `CTRL_REG1` `0x07`, `VERSION` `0x28`,
//!   everything else 0 (the `CLICK_COEFF` defaults are not modeled; the
//!   firmware never reads them).
//! - **Sub-address and auto-increment**: the first byte written after the
//!   address sets the register pointer (bits 6:0) and the auto-increment
//!   flag (bit 7); each further byte written, or read, moves the pointer
//!   by one when the flag is set (wrapping within `0x00..=0x7F`). The
//!   pointer and flag survive the repeated START of a register read.
//! - **Output**: `OUT_*` are computed on every read from the host-set
//!   acceleration ([`Sc7a20h::set_acceleration`], in mg): `mg /
//!   sensitivity` for the current `FS`, clamped to the 12-bit range
//!   `-2048..=2047`, shifted left by 4; `BLE` = 1 swaps the two bytes.
//!   Normal, low-power and high-resolution modes all report 12-bit data
//!   (the datasheet's output table has no per-mode resolution).
//! - **Data ready**: with `ODR` != 0 (powered up), `DRDY_STATUS_REG`
//!   reads `ZYXDA | ZDA | YDA | XDA` (`0x0F`) on every read, with no
//!   overrun bits; in power-down (`ODR` = 0) it reads 0. There is no time
//!   base, so a new sample is always ready (see the decisions file).
//! - **Stationary default**: (0, 0, +1000) mg, the badge lying face up.
//!
//! Not modeled: FIFO, interrupts (`INT1`/`INT2` and the AOI/click
//! engines), the high-pass filter, self-test, `BDU` latching (a burst read
//! is atomic here anyway), soft reset, the `0x61..=0x66` output copies,
//! and the SPI interface.

/// The 7-bit I2C address with SDO floating or high (§10.1).
pub const ADDRESS: u8 = 0x19;

/// `WHO_AM_I` and its fixed value (§12.2).
pub const WHO_AM_I: u8 = 0x0F;
pub const WHO_AM_I_VALUE: u8 = 0x11;
/// `CTRL_REG1` (§12.4), reset `0x07` (`Zen`/`Yen`/`Xen`).
pub const CTRL_REG1: u8 = 0x20;
const CTRL_REG1_RESET: u8 = 0x07;
/// `CTRL_REG4` (§12.7).
pub const CTRL_REG4: u8 = 0x23;
const CTRL_REG4_BLE: u8 = 1 << 6;
const CTRL_REG4_FS_S: u8 = 4;
/// `DRDY_STATUS_REG` (§12.10) and its `ZYXDA | ZDA | YDA | XDA` bits.
pub const STATUS_REG: u8 = 0x27;
pub const STATUS_ALL_DATA_READY: u8 = 0x0F;
/// `OUT_X_L`; the six output bytes are `0x28..=0x2D` (§12.11..12.13).
pub const OUT_X_L: u8 = 0x28;
const OUT_Z_H: u8 = 0x2D;
/// `VERSION` (§12.37), reset `0x28`.
pub const VERSION: u8 = 0x70;
const VERSION_RESET: u8 = 0x28;

/// Sub-address bit 7: auto-increment (§10.1).
pub const SUB_AUTO_INCREMENT: u8 = 0x80;
const REG_MASK: u8 = 0x7F;

/// The stationary reading: the badge face up, +1 g on Z.
pub const STATIONARY_MG: [i32; 3] = [0, 0, 1000];

/// The widest full scale, ±16 g, in mg: host input is clamped to it.
const MAX_MG: i32 = 16_000;

/// `true` for the registers §11 lists as `rw`.
fn writable(reg: u8) -> bool {
    matches!(
        reg,
        0x1F..=0x25 | 0x2E | 0x30 | 0x32..=0x34 | 0x36..=0x38 | 0x3A..=0x3D | 0x68 | 0x6F
    )
}

/// The SC7A20H. See the module doc.
#[derive(Clone)]
pub struct Sc7a20h {
    regs: [u8; 0x80],
    pointer: u8,
    auto_increment: bool,
    /// `true` between a write-direction address and its sub-address byte.
    expect_sub: bool,
    accel_mg: [i32; 3],
}

impl Default for Sc7a20h {
    fn default() -> Self {
        Self::new()
    }
}

impl Sc7a20h {
    pub fn new() -> Self {
        let mut regs = [0u8; 0x80];
        regs[WHO_AM_I as usize] = WHO_AM_I_VALUE;
        regs[CTRL_REG1 as usize] = CTRL_REG1_RESET;
        regs[VERSION as usize] = VERSION_RESET;
        Self {
            regs,
            pointer: 0,
            auto_increment: false,
            expect_sub: false,
            accel_mg: STATIONARY_MG,
        }
    }

    /// The address byte matched: a transfer in direction `read` begins.
    /// A write transfer starts with the sub-address.
    pub fn start(&mut self, read: bool) {
        self.expect_sub = !read;
    }

    /// One byte from the master in a write transfer. The device ACKs every
    /// byte (§10.1).
    pub fn write(&mut self, byte: u8) {
        if self.expect_sub {
            self.pointer = byte & REG_MASK;
            self.auto_increment = byte & SUB_AUTO_INCREMENT != 0;
            self.expect_sub = false;
            return;
        }
        if writable(self.pointer) {
            self.regs[self.pointer as usize] = byte;
        }
        self.advance();
    }

    /// One byte to the master in a read transfer.
    pub fn read(&mut self) -> u8 {
        let v = self.register(self.pointer);
        self.advance();
        v
    }

    /// STOP: the transfer ends.
    pub fn stop(&mut self) {
        self.expect_sub = false;
    }

    /// The host's acceleration in mg per axis, clamped to ±16 g; the output
    /// registers further clamp to the configured full scale.
    pub fn set_acceleration(&mut self, x_mg: i32, y_mg: i32, z_mg: i32) {
        self.accel_mg = [x_mg, y_mg, z_mg].map(|v| v.clamp(-MAX_MG, MAX_MG));
    }

    /// The current host acceleration in mg.
    pub fn acceleration(&self) -> [i32; 3] {
        self.accel_mg
    }

    fn advance(&mut self) {
        if self.auto_increment {
            self.pointer = (self.pointer + 1) & REG_MASK;
        }
    }

    /// A register as the master reads it.
    fn register(&self, reg: u8) -> u8 {
        match reg {
            STATUS_REG => {
                if self.regs[CTRL_REG1 as usize] >> 4 != 0 {
                    STATUS_ALL_DATA_READY
                } else {
                    0
                }
            }
            OUT_X_L..=OUT_Z_H => {
                let off = reg - OUT_X_L;
                let raw = self.output_word((off / 2) as usize).to_le_bytes();
                let ble = self.regs[CTRL_REG4 as usize] & CTRL_REG4_BLE != 0;
                // BLE = 0: low byte at the lower address.
                raw[usize::from((off % 2 == 1) != ble)]
            }
            r => self.regs[r as usize],
        }
    }

    /// One axis's 16-bit output word: 12-bit counts, left-justified.
    fn output_word(&self, axis: usize) -> i16 {
        let fs = (self.regs[CTRL_REG4 as usize] >> CTRL_REG4_FS_S) & 0b11;
        let mg_per_digit = 1i32 << fs; // §6: 1, 2, 4, 8 mg/digit
        let counts = (self.accel_mg[axis] / mg_per_digit).clamp(-2048, 2047);
        (counts << 4) as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `i2c_master_transmit_receive(dev, &reg, 1, buf, n)`: address+W, the
    /// sub-address, repeated START, address+R, `n` bytes.
    fn read_regs(d: &mut Sc7a20h, sub: u8, n: usize) -> Vec<u8> {
        d.start(false);
        d.write(sub);
        d.start(true);
        let out = (0..n).map(|_| d.read()).collect();
        d.stop();
        out
    }

    /// `i2c_master_transmit(dev, {reg, val}, 2)`.
    fn write_reg(d: &mut Sc7a20h, reg: u8, val: u8) {
        d.start(false);
        d.write(reg);
        d.write(val);
        d.stop();
    }

    /// `hal_accel`'s setup (`0x4200_a8da`): CTRL_REG1 = 0x57 (100 Hz, XYZ),
    /// CTRL_REG4 = 0x80 (BDU, ±2 g).
    fn configured() -> Sc7a20h {
        let mut d = Sc7a20h::new();
        write_reg(&mut d, CTRL_REG1, 0x57);
        write_reg(&mut d, CTRL_REG4, 0x80);
        d
    }

    /// The firmware's axis decode (`0x4200_a800`..`0x4200_a810`).
    fn decode(l: u8, h: u8) -> i32 {
        i32::from(i16::from_le_bytes([l, h])) >> 4
    }

    #[test]
    fn who_am_i_reads_0x11_and_version_0x28() {
        let mut d = Sc7a20h::new();
        assert_eq!(read_regs(&mut d, WHO_AM_I, 1), [0x11]);
        assert_eq!(read_regs(&mut d, VERSION, 1), [0x28]);
        assert_eq!(read_regs(&mut d, CTRL_REG1, 1), [0x07]);
    }

    #[test]
    fn register_writes_read_back_and_read_only_registers_ignore_writes() {
        let mut d = configured();
        assert_eq!(read_regs(&mut d, CTRL_REG1, 1), [0x57]);
        assert_eq!(read_regs(&mut d, CTRL_REG4, 1), [0x80]);
        write_reg(&mut d, WHO_AM_I, 0x00);
        assert_eq!(read_regs(&mut d, WHO_AM_I, 1), [0x11]);
        write_reg(&mut d, 0x05, 0xAA); // reserved 00..0B
        assert_eq!(read_regs(&mut d, 0x05, 1), [0x00]);
    }

    #[test]
    fn status_reports_all_axes_ready_only_when_powered_up() {
        let mut d = Sc7a20h::new(); // ODR = 0: power-down
        assert_eq!(read_regs(&mut d, STATUS_REG, 1), [0x00]);
        let mut d = configured();
        assert_eq!(read_regs(&mut d, STATUS_REG, 1)[0] & 0x08, 0x08, "ZYXDA");
        assert_eq!(read_regs(&mut d, STATUS_REG, 1), [0x0F]);
    }

    /// The firmware's sample read: six bytes from `0x28 | 0x80`.
    #[test]
    fn burst_read_auto_increments_through_the_six_output_bytes() {
        let mut d = configured();
        let b = read_regs(&mut d, OUT_X_L | SUB_AUTO_INCREMENT, 6);
        // Stationary, face up: (0, 0, +1000 mg) = 1000 counts << 4 = 0x3E80.
        assert_eq!(b, [0x00, 0x00, 0x00, 0x00, 0x80, 0x3E]);
        assert_eq!(decode(b[4], b[5]), 1000);
    }

    #[test]
    fn without_the_auto_increment_bit_the_pointer_stays() {
        let mut d = configured();
        d.set_acceleration(0x123, 0, 0);
        let b = read_regs(&mut d, OUT_X_L, 3);
        assert_eq!(b, [0x30, 0x30, 0x30]);
    }

    #[test]
    fn auto_increment_applies_to_multi_byte_writes_too() {
        let mut d = Sc7a20h::new();
        d.start(false);
        d.write(CTRL_REG1 | SUB_AUTO_INCREMENT);
        for v in [0x57, 0x01, 0x02, 0x80] {
            d.write(v);
        }
        d.stop();
        assert_eq!(
            read_regs(&mut d, CTRL_REG1 | SUB_AUTO_INCREMENT, 4),
            [0x57, 0x01, 0x02, 0x80]
        );
    }

    #[test]
    fn output_encodes_the_full_scale_sensitivity_and_clamps() {
        let mut d = configured(); // ±2 g, 1 mg/digit
        d.set_acceleration(-1500, 2500, -3000);
        let b = read_regs(&mut d, OUT_X_L | SUB_AUTO_INCREMENT, 6);
        assert_eq!(decode(b[0], b[1]), -1500);
        assert_eq!(decode(b[2], b[3]), 2047, "clamped to +full scale");
        assert_eq!(decode(b[4], b[5]), -2048, "clamped to -full scale");
        // ±8 g (FS = 10): 4 mg/digit.
        write_reg(&mut d, CTRL_REG4, 0x80 | (0b10 << 4));
        let b = read_regs(&mut d, OUT_X_L | SUB_AUTO_INCREMENT, 6);
        assert_eq!(decode(b[0], b[1]), -375);
        assert_eq!(decode(b[2], b[3]), 625);
        assert_eq!(decode(b[4], b[5]), -750);
    }

    #[test]
    fn ble_swaps_the_output_bytes() {
        let mut d = configured();
        write_reg(&mut d, CTRL_REG4, 0x80 | CTRL_REG4_BLE);
        let b = read_regs(&mut d, OUT_X_L | SUB_AUTO_INCREMENT, 6);
        assert_eq!(&b[4..], [0x3E, 0x80], "high byte at the lower address");
    }

    #[test]
    fn host_input_never_panics_and_is_clamped_to_16_g() {
        let mut d = configured();
        d.set_acceleration(i32::MIN, i32::MAX, 0);
        assert_eq!(d.acceleration(), [-16_000, 16_000, 0]);
        write_reg(&mut d, CTRL_REG4, 0x80 | (0b11 << 4)); // ±16 g, 8 mg/digit
        let b = read_regs(&mut d, OUT_X_L | SUB_AUTO_INCREMENT, 4);
        assert_eq!(decode(b[0], b[1]), -2000);
        assert_eq!(decode(b[2], b[3]), 2000);
    }

    #[test]
    fn the_pointer_wraps_within_the_7_bit_register_space() {
        let mut d = Sc7a20h::new();
        let b = read_regs(&mut d, 0x7F | SUB_AUTO_INCREMENT, 2);
        assert_eq!(b, [0x00, 0x00]);
        let b = read_regs(&mut d, 0x0E | SUB_AUTO_INCREMENT, 2);
        assert_eq!(b, [0x00, 0x11]);
    }
}

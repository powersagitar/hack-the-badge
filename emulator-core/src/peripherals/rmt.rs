//! The ESP32-C3's Remote Control peripheral, RMT (`DR_REG_RMT_BASE =
//! 0x6001_6000`, `soc/reg_base.h`; `RMTMEM = 0x6001_6400`,
//! `ld/esp32c3/rmt.peripherals.ld`; [`crate::mem::soc::RMT_RANGE`]), as a
//! **TX controller that completes every transmission the firmware starts,
//! in zero emulated time** (Milestone 4 Task D-M4-2; see
//! `docs/milestone-4-decisions.md`).
//!
//! The firmware drives it through ESP-IDF v5.5.3's TX driver
//! (`components/esp_driver_rmt/src/rmt_tx.c`) without DMA (the ESP32-C3 has
//! no `SOC_RMT_SUPPORT_DMA`): `rmt_tx_do_transaction()` encodes symbols
//! straight into the channel's RMT RAM, sets `TX_START`, and the task then
//! blocks until `rmt_tx_default_isr()`, on `ETS_RMT_INTR_SOURCE`
//! ([`crate::mem::soc::SRC_RMT`], source 28 in
//! `soc/esp32c3/include/soc/interrupts.h`), sees `TX_END`. A transaction
//! longer than the channel's RAM runs "ping-pong": wrap mode on, `TX_LIM` =
//! half the RAM, and each `TX_THR_EVENT` interrupt refills the half the
//! transmitter has just finished (`rmt_isr_handle_tx_threshold`).
//!
//! ## Sources (all ESP-IDF v5.5.3 unless noted)
//!
//! - Offsets, fields, access types and reset values:
//!   `components/soc/esp32c3/register/soc/rmt_reg.h` and `rmt_struct.h`.
//! - Accessors: `components/hal/esp32c3/include/hal/rmt_ll.h`
//!   (`RMT_LL_EVENT_TX_DONE(ch)` = bit `ch`, `TX_THRES` = bit `ch + 8`,
//!   `TX_LOOP_END` = bit `ch + 12`, `TX_ERROR` = bit `ch + 4`;
//!   `rmt_ll_tx_start` sets `conf_update` then `tx_start`;
//!   `rmt_ll_tx_reset_pointer` pulses `mem_rd_rst` then `mem_rst`;
//!   `rmt_ll_tx_stop` sets `tx_stop` then `conf_update`;
//!   `rmt_ll_enable_interrupt` read-modify-writes `int_ena`;
//!   `rmt_ll_clear_interrupt_status` writes `int_clr`;
//!   `rmt_ll_tx_get_interrupt_status` reads `int_st`;
//!   `rmt_ll_tx_reset_loop_count` pulses `tx_lim.loop_count_reset`).
//! - Symbol layout: `components/hal/include/hal/rmt_types.h`
//!   (`rmt_symbol_word_t`: `duration0` 14:0, `level0` 15, `duration1`
//!   30:16, `level1` 31). TX channel `n`'s RAM is
//!   `RMTMEM.channels[n].symbols` (`esp_driver_rmt/src/rmt_private.h`:
//!   48 words per channel, `SOC_RMT_MEM_WORDS_PER_CHANNEL`).
//! - Hardware behavior: *ESP32-C3 Technical Reference Manual* v1.4,
//!   chapter 33 (RMT): §33.3.2 (RAM blocks, `MEM_SIZE`), §33.3.4.1 (normal
//!   TX: `TX_START` reads from the start of the channel's block; a zero
//!   period is an end-marker that stops the transmitter and raises
//!   `TX_END`; `TX_STOP` returns it to idle), §33.3.4.2 (wrap TX: the
//!   transmitter loops over its RAM until an end-marker; `TX_THR_EVENT` once
//!   the amount sent reaches `TX_LIM`), §33.3.4.4 (continuous TX: restart
//!   from the first entry at an end-marker or after the last entry; with
//!   `TX_LOOP_CNT_EN` each end-marker counts and `TX_LOOP` fires when the
//!   count reaches `TX_LOOP_NUM`), §33.3.7 (`ERR` when the transmitter
//!   reads past an exhausted RAM), and register 33.8 (`MEM_EMPTY`: TX data
//!   larger than the RAM with wrap disabled).
//!
//! ## What is modeled
//!
//! - **The transmitter runs in zero time, but never ahead of the
//!   firmware.** `TX_START` (WT, `CONF0` bit 0) starts channel `n` at the
//!   first word of its block (`48 * n`); the transmitter consumes words
//!   immediately, inside the register write. It stops at an end-marker
//!   (either 15-bit duration zero: `TX_END`), at the end of its RAM with
//!   wrap and continuous mode off (`MEM_EMPTY` + `ERR`), or on `TX_STOP`.
//!   It *pauses* right after raising an **enabled** `TX_THR_EVENT` or
//!   `TX_LOOP` interrupt, until the firmware clears that raw bit
//!   (`INT_CLR`/`INT_RAW`) or disables it (`INT_ENA`). This is the
//!   observable sequence of a real transmitter whose ISR keeps up: after a
//!   threshold event the hardware is still sending the *other* half of the
//!   RAM, so when the ISR's clear lets the model send that half, the data
//!   is the same, and the half the ISR then refills is the next one sent.
//!   Words are counted toward `TX_LIM` as each is fully sent, the counter
//!   restarting at every event (the driver's ping-pong toggles halves on
//!   every event); `TX_LIM` 0 never fires.
//! - **Wrap and continuous modes**: with `MEM_TX_WRAP_EN` or
//!   `TX_CONTI_MODE` the read address wraps from the end of the channel's
//!   `MEM_SIZE` blocks to its start. In continuous mode an end-marker
//!   restarts from the first word instead of stopping (no `TX_END`), and
//!   with `TX_LOOP_CNT_EN` increments the loop counter, raising `TX_LOOP`
//!   when it reaches `TX_LOOP_NUM`. The ESP32-C3 has no loop auto-stop
//!   (`SOC_RMT_SUPPORT_TX_LOOP_AUTO_STOP` is absent from `soc_caps.h`), so
//!   the channel keeps running until `TX_STOP`; the counter then holds
//!   (judgment call) until `LOOP_COUNT_RESET` or the next `TX_START` resets
//!   it. A channel whose output has become periodic (a whole RAM lap with
//!   no change to `INT_RAW` or the loop counter) is left running without
//!   further work; any later RMT write re-evaluates it.
//! - **`MEM_SIZE`**: channel `n` owns blocks `n .. n + MEM_SIZE` (TRM
//!   §33.3.2), clamped to the RAM's four blocks; `MEM_SIZE` 0 reads as an
//!   exhausted RAM.
//! - **Interrupt registers**: `INT_RAW` (R/WTC/SS: a 1 written clears),
//!   `INT_CLR` (WT: a 1 clears; reads 0), `INT_ENA` (R/W), `INT_ST` =
//!   `INT_RAW & INT_ENA` (RO); bits 13:0. [`Rmt::pending_sources`] asserts
//!   `SRC_RMT` while `INT_ST` is non-zero, as a level.
//! - **TX status** (`CHnSTATUS`, RO): `MEM_RADDR_EX` is the transmitter's
//!   absolute RAM word index (`48 * n` + its offset, reset value 0 until
//!   the first `TX_START` or `MEM_RD_RST`, as
//!   `rmt_ll_rx_get_memory_writer_offset` decodes the RX twin), and
//!   `MEM_EMPTY` is set by an overrun until the next `TX_START`. `STATE`
//!   and the APB-FIFO fields read 0.
//! - **WT bits read 0** with only these effects: `TX_START`, `MEM_RD_RST`
//!   (the transmitter's read address goes back to its block start),
//!   `LOOP_COUNT_RESET`. `TX_STOP` (R/W/SC) takes effect at once and
//!   self-clears. `CONF_UPDATE`, `APB_MEM_RST`, `AFIFO_RST`, `MEM_WR_RST`
//!   and `REF_CNT_RST` have no effect: the model always uses live register
//!   values and keeps no clock-divider phase.
//! - **RMT RAM** (`+0x400..+0x700`, 192 words): plain read/write storage
//!   accessed directly (`SYS_CONF.APB_FIFO_MASK` = 1, which
//!   `rmt_ll_enable_mem_access_nonfifo` sets). A word's last byte (`+3`)
//!   re-evaluates running channels, so a `sw` is seen whole.
//! - **Everything else** (clock selection and dividers, carrier, RX
//!   channels, `TX_SIM`, `DATE`): plain storage of each register's R/W
//!   bits, starting at its `rmt_reg.h` `default:` ([`Rmt::new`]). Timing,
//!   carrier modulation, the output level and the receivers are not
//!   modeled (RX channels never see an edge). `TX_SIM` is stored, but a
//!   sync-group channel starts on its own `TX_START`.
//!
//! The APB FIFO data registers (`CHnDATA`, `+0x00..+0x0C`, FIFO mode),
//! the reserved words `+0x74..+0xC8` and the space after the RAM are not
//! handled: they read 0, drop writes and are logged by the bus as
//! unmapped.

use super::set_byte;
use crate::mem::soc::SRC_RMT;

/// `RMT_CH0CONF0_REG`; channel `n`'s (`n < 2`) is at `+ 4 * n`.
pub const TX_CONF0_REG: u32 = 0x10;
/// `RMT_CH2CONF0_REG`; channel 3's is at `+ 8`.
pub const RX_CONF0_REG: u32 = 0x18;
/// `RMT_CH2CONF1_REG`; channel 3's is at `+ 8`.
pub const RX_CONF1_REG: u32 = 0x1C;
/// `RMT_CH0STATUS_REG`; channel 1's is at `+ 4`.
pub const TX_STATUS_REG: u32 = 0x28;
/// `RMT_CH2STATUS_REG`; channel 3's is at `+ 4`.
pub const RX_STATUS_REG: u32 = 0x30;
/// `RMT_INT_RAW_REG`.
pub const INT_RAW_REG: u32 = 0x38;
/// `RMT_INT_ST_REG`.
pub const INT_ST_REG: u32 = 0x3C;
/// `RMT_INT_ENA_REG`.
pub const INT_ENA_REG: u32 = 0x40;
/// `RMT_INT_CLR_REG`.
pub const INT_CLR_REG: u32 = 0x44;
/// `RMT_CH0CARRIER_DUTY_REG`; channel 1's is at `+ 4`.
pub const TX_CARRIER_REG: u32 = 0x48;
/// `RMT_CH0_TX_LIM_REG`; channel 1's is at `+ 4`.
pub const TX_LIM_REG: u32 = 0x58;
/// `RMT_CH2_RX_LIM_REG`; channel 3's is at `+ 4`.
pub const RX_LIM_REG: u32 = 0x60;
/// `RMT_SYS_CONF_REG`.
pub const SYS_CONF_REG: u32 = 0x68;
/// `RMT_TX_SIM_REG`.
pub const TX_SIM_REG: u32 = 0x6C;
/// `RMT_REF_CNT_RST_REG` (WT).
pub const REF_CNT_RST_REG: u32 = 0x70;
/// `RMT_DATE_REG`.
pub const DATE_REG: u32 = 0xCC;
/// `RMTMEM`: the start of the 192-word RMT RAM.
pub const RAM_START: u32 = 0x400;
/// End of the RMT RAM (exclusive): 4 blocks of 48 words.
pub const RAM_END: u32 = RAM_START + 4 * RAM_WORDS as u32;

/// `SOC_RMT_MEM_WORDS_PER_CHANNEL`.
pub const BLOCK_WORDS: u32 = 48;
/// Words in the whole RAM (`SOC_RMT_CHANNELS_PER_GROUP` blocks).
const RAM_WORDS: usize = 4 * BLOCK_WORDS as usize;
/// Number of TX channels (`SOC_RMT_TX_CANDIDATES_PER_GROUP`).
const TX_CHANNELS: usize = 2;

/// TX `CONF0` bits (`rmt_reg.h`).
pub const CONF0_TX_START: u32 = 1 << 0; // WT
pub const CONF0_MEM_RD_RST: u32 = 1 << 1; // WT
pub const CONF0_APB_MEM_RST: u32 = 1 << 2; // WT
pub const CONF0_TX_CONTI_MODE: u32 = 1 << 3;
pub const CONF0_MEM_TX_WRAP_EN: u32 = 1 << 4;
pub const CONF0_IDLE_OUT_LV: u32 = 1 << 5;
pub const CONF0_IDLE_OUT_EN: u32 = 1 << 6;
pub const CONF0_TX_STOP: u32 = 1 << 7; // R/W/SC
pub const CONF0_MEM_SIZE_S: u32 = 16;
pub const CONF0_AFIFO_RST: u32 = 1 << 23; // WT
pub const CONF0_CONF_UPDATE: u32 = 1 << 24; // WT
/// The R/W bits of a TX `CONF0`: 6:3, `DIV_CNT` 15:8, `MEM_SIZE` 18:16,
/// `CARRIER_EFF_EN`/`CARRIER_EN`/`CARRIER_OUT_LV` 22:20.
const TX_CONF0_RW: u32 = 0x0077_FF78;
/// RX `CONF0` R/W bits: `DIV_CNT` 7:0, `IDLE_THRES` 22:8, `MEM_SIZE`
/// 25:23, `CARRIER_EN` 28, `CARRIER_OUT_LV` 29.
const RX_CONF0_RW: u32 = 0x33FF_FFFF;
/// RX `CONF1` R/W bits: `RX_EN` 0, `MEM_OWNER` 3, `RX_FILTER_EN` 4,
/// `RX_FILTER_THRES` 12:5, `MEM_RX_WRAP_EN` 13 (bits 1, 2, 14, 15 are WT).
const RX_CONF1_RW: u32 = 0x3FF9;

/// `TX_LIM` fields: `TX_LIM` 8:0, `TX_LOOP_NUM` 18:9, `TX_LOOP_CNT_EN` 19
/// (R/W), `LOOP_COUNT_RESET` 20 (WT).
const TX_LIM_LIMIT_MASK: u32 = 0x1FF;
const TX_LIM_LOOP_NUM_S: u32 = 9;
const LOOP_NUM_MASK: u32 = 0x3FF;
pub const TX_LIM_LOOP_CNT_EN: u32 = 1 << 19;
pub const TX_LIM_LOOP_COUNT_RESET: u32 = 1 << 20;
const TX_LIM_RW: u32 = 0xF_FFFF;

/// `CHnSTATUS`: `MEM_RADDR_EX` 8:0, `MEM_EMPTY` 22.
const STATUS_RADDR_EX_MASK: u32 = 0x1FF;
pub const STATUS_MEM_EMPTY: u32 = 1 << 22;

/// Interrupt bits (`RMT_LL_EVENT_*`, the same layout in RAW/ST/ENA/CLR).
pub const fn int_tx_end(ch: usize) -> u32 {
    1 << ch
}
pub const fn int_tx_err(ch: usize) -> u32 {
    1 << (ch + 4)
}
pub const fn int_tx_thr(ch: usize) -> u32 {
    1 << (ch + 8)
}
pub const fn int_tx_loop(ch: usize) -> u32 {
    1 << (ch + 12)
}
/// Bits 13:0 exist (`rmt_struct.h`: `reserved14 : 18`).
const INT_MASK: u32 = 0x3FFF;

/// Every register whose `rmt_reg.h` (v5.5.3) `default:` is non-zero, as
/// `(offset, reset value)`. Every other register resets to 0.
const RESET_VALUES: [(u32, u32); 12] = [
    // CARRIER_OUT_LV, CARRIER_EN, CARRIER_EFF_EN = 1; MEM_SIZE 1; DIV_CNT 2.
    (TX_CONF0_REG, 0x0071_0200),
    (TX_CONF0_REG + 4, 0x0071_0200),
    // CARRIER_OUT_LV, CARRIER_EN = 1; MEM_SIZE 1; IDLE_THRES 0x7FFF; DIV_CNT 2.
    (RX_CONF0_REG, 0x30FF_FF02),
    (RX_CONF0_REG + 8, 0x30FF_FF02),
    // RX_FILTER_THRES 0xF; MEM_OWNER 1.
    (RX_CONF1_REG, 0x1E8),
    (RX_CONF1_REG + 8, 0x1E8),
    // CARRIER_HIGH = CARRIER_LOW = 0x40.
    (TX_CARRIER_REG, 0x0040_0040),
    (TX_CARRIER_REG + 4, 0x0040_0040),
    (TX_LIM_REG, 0x80),
    (TX_LIM_REG + 4, 0x80),
    (RX_LIM_REG, 0x80),
    (RX_LIM_REG + 4, 0x80),
];
/// `SYS_CONF`: `SCLK_ACTIVE` 1, `SCLK_SEL` 1 (APB), `SCLK_DIV_NUM` 1.
const SYS_CONF_RESET: u32 = 0x0500_0010;
/// `RMT_DATE`: `28'h2006231`.
const DATE_RESET: u32 = 0x0200_6231;

/// One TX channel's transmitter state.
#[derive(Clone, Copy, Default)]
struct Transmitter {
    running: bool,
    /// Whether `TX_START` or `MEM_RD_RST` has set the read address yet:
    /// `MEM_RADDR_EX` keeps its reset value 0 until then.
    addressed: bool,
    /// Read offset in words from the start of the channel's block.
    rd: u32,
    /// Words fully sent since the last `TX_THR_EVENT` (or `TX_START`).
    sent: u32,
    /// The loop counter (`TX_LOOP_CNT_EN`), and whether it has reached
    /// `TX_LOOP_NUM` and now holds.
    loop_count: u32,
    loop_done: bool,
    mem_empty: bool,
    /// The enabled interrupt bit the transmitter paused on, or 0.
    waiting: u32,
}

/// The RMT peripheral. See the module doc.
pub struct Rmt {
    /// Word storage for `0x00..=DATE_REG`, indexed by `offset / 4`. STATUS,
    /// `INT_ST`, `INT_CLR` and the WT bits are computed instead.
    regs: [u32; (DATE_REG / 4 + 1) as usize],
    ram: [u32; RAM_WORDS],
    tx: [Transmitter; TX_CHANNELS],
}

impl Default for Rmt {
    fn default() -> Self {
        Self::new()
    }
}

impl Rmt {
    pub fn new() -> Self {
        let mut regs = [0u32; (DATE_REG / 4 + 1) as usize];
        for (off, val) in RESET_VALUES {
            regs[(off / 4) as usize] = val;
        }
        regs[(SYS_CONF_REG / 4) as usize] = SYS_CONF_RESET;
        regs[(DATE_REG / 4) as usize] = DATE_RESET;
        Self {
            regs,
            ram: [0; RAM_WORDS],
            tx: [Transmitter::default(); TX_CHANNELS],
        }
    }

    /// `true` for every offset backed by a named register or RAM word.
    pub fn handles(offset: u32) -> bool {
        let word = offset & !0b11;
        (TX_CONF0_REG..=REF_CNT_RST_REG).contains(&word)
            || word == DATE_REG
            || (RAM_START..RAM_END).contains(&word)
    }

    fn reg(&self, off: u32) -> u32 {
        self.regs[(off / 4) as usize]
    }

    fn reg_mut(&mut self, off: u32) -> &mut u32 {
        &mut self.regs[(off / 4) as usize]
    }

    fn raise(&mut self, bits: u32) {
        *self.reg_mut(INT_RAW_REG) |= bits;
    }

    fn int_st(&self) -> u32 {
        self.reg(INT_RAW_REG) & self.reg(INT_ENA_REG)
    }

    /// Whether TX channel `ch` is transmitting (started, not yet stopped
    /// by an end-marker, an overrun or `TX_STOP`).
    pub fn tx_running(&self, ch: usize) -> bool {
        self.tx[ch].running
    }

    fn tx_status(&self, ch: usize) -> u32 {
        let t = &self.tx[ch];
        let raddr_ex = if t.addressed {
            (BLOCK_WORDS * ch as u32 + t.rd) & STATUS_RADDR_EX_MASK
        } else {
            0
        };
        raddr_ex | if t.mem_empty { STATUS_MEM_EMPTY } else { 0 }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        if !Self::handles(offset) {
            return 0;
        }
        let word = offset & !0b11;
        let idx = offset & 0b11;
        let value = match word {
            w if w >= RAM_START => self.ram[((w - RAM_START) / 4) as usize],
            TX_STATUS_REG => self.tx_status(0),
            0x2C => self.tx_status(1),
            INT_ST_REG => self.int_st(),
            // RX status (never receives), INT_CLR and REF_CNT_RST (WT).
            0x30 | 0x34 | INT_CLR_REG | REF_CNT_RST_REG => 0,
            w => self.reg(w),
        };
        (value >> (idx * 8)) as u8
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        if !Self::handles(offset) {
            return;
        }
        let word = offset & !0b11;
        let idx = offset & 0b11;
        let bits = u32::from(val) << (idx * 8);
        match word {
            w if w >= RAM_START => {
                set_byte(&mut self.ram[((w - RAM_START) / 4) as usize], idx, val);
                if idx != 3 {
                    // Re-evaluate only once the word is whole (see the doc).
                    return;
                }
            }
            TX_CONF0_REG | 0x14 => {
                let ch = ((word - TX_CONF0_REG) / 4) as usize;
                let mut conf = self.reg(word);
                set_byte(&mut conf, idx, val);
                *self.reg_mut(word) = conf & TX_CONF0_RW;
                if bits & CONF0_TX_STOP != 0 {
                    // R/W/SC: the transmitter goes idle at once.
                    self.tx[ch].running = false;
                    self.tx[ch].waiting = 0;
                }
                if bits & CONF0_MEM_RD_RST != 0 {
                    self.tx[ch].rd = 0;
                    self.tx[ch].addressed = true;
                }
                if bits & CONF0_TX_START != 0 {
                    self.tx[ch] = Transmitter {
                        running: true,
                        addressed: true,
                        ..Transmitter::default()
                    };
                }
            }
            0x18 | 0x20 => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= RX_CONF0_RW;
            }
            0x1C | 0x24 => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= RX_CONF1_RW;
            }
            TX_STATUS_REG..=0x34 | INT_ST_REG | REF_CNT_RST_REG => {}
            // INT_RAW is R/WTC/SS, INT_CLR is WT: a 1 clears the raw bit.
            INT_RAW_REG | INT_CLR_REG => *self.reg_mut(INT_RAW_REG) &= !(bits & INT_MASK),
            INT_ENA_REG => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= INT_MASK;
            }
            TX_LIM_REG | 0x5C => {
                let ch = ((word - TX_LIM_REG) / 4) as usize;
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= TX_LIM_RW;
                if bits & TX_LIM_LOOP_COUNT_RESET != 0 {
                    self.tx[ch].loop_count = 0;
                    self.tx[ch].loop_done = false;
                }
            }
            RX_LIM_REG | 0x64 => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= 0x1FF;
            }
            SYS_CONF_REG => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= 0x87FF_FFFF;
            }
            TX_SIM_REG => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= 0x7;
            }
            DATE_REG => {
                set_byte(self.reg_mut(word), idx, val);
                *self.reg_mut(word) &= 0x0FFF_FFFF;
            }
            w => set_byte(self.reg_mut(w), idx, val),
        }
        for ch in 0..TX_CHANNELS {
            self.transmit(ch);
        }
    }

    /// Runs TX channel `ch`'s transmitter as far as it can go now (see the
    /// module doc): to an end-marker, an overrun, a pause on an enabled
    /// threshold/loop interrupt, or a periodic state.
    fn transmit(&mut self, ch: usize) {
        if !self.tx[ch].running {
            return;
        }
        let waiting = self.tx[ch].waiting;
        if waiting != 0 {
            if self.int_st() & waiting != 0 {
                return;
            }
            self.tx[ch].waiting = 0;
        }
        let conf = self.reg(TX_CONF0_REG + 4 * ch as u32);
        let conti = conf & CONF0_TX_CONTI_MODE != 0;
        let wrap = conti || conf & CONF0_MEM_TX_WRAP_EN != 0;
        // Blocks ch .. ch + MEM_SIZE, clamped to the RAM's four (§33.3.2).
        let blocks = ((conf >> CONF0_MEM_SIZE_S) & 0x7).min(4 - ch as u32);
        let cap = blocks * BLOCK_WORDS;
        let lim = self.reg(TX_LIM_REG + 4 * ch as u32);
        let limit = lim & TX_LIM_LIMIT_MASK;
        let loop_num = (lim >> TX_LIM_LOOP_NUM_S) & LOOP_NUM_MASK;
        let loop_cnt_en = lim & TX_LIM_LOOP_CNT_EN != 0;
        let base = (BLOCK_WORDS * ch as u32) as usize;

        // Words read since INT_RAW or the loop counter last changed; a whole
        // lap of them means the output is periodic.
        let mut unchanged = 0;
        loop {
            if unchanged > cap {
                return;
            }
            let before = (self.reg(INT_RAW_REG), self.tx[ch].loop_count);
            if self.tx[ch].rd >= cap {
                if cap == 0 || !wrap {
                    // Reading an exhausted RAM (§33.3.7, register 33.8).
                    self.tx[ch].mem_empty = true;
                    self.tx[ch].running = false;
                    self.raise(int_tx_err(ch));
                    return;
                }
                self.tx[ch].rd = 0;
            }
            let word = self.ram[base + self.tx[ch].rd as usize];
            let end_marker = word & 0x7FFF == 0 || (word >> 16) & 0x7FFF == 0;
            if end_marker {
                if !conti {
                    self.tx[ch].running = false;
                    self.raise(int_tx_end(ch));
                    return;
                }
                // Continuous mode: restart from the first word (§33.3.4.4).
                self.tx[ch].rd = 0;
                let t = &mut self.tx[ch];
                if loop_cnt_en && !t.loop_done {
                    t.loop_count = (t.loop_count + 1) & LOOP_NUM_MASK;
                    if t.loop_count == loop_num {
                        t.loop_done = true;
                        if self.pause_on(ch, int_tx_loop(ch)) {
                            return;
                        }
                    }
                }
            } else {
                let t = &mut self.tx[ch];
                t.rd += 1;
                t.sent += 1;
                if limit != 0 && t.sent >= limit {
                    t.sent = 0;
                    if self.pause_on(ch, int_tx_thr(ch)) {
                        return;
                    }
                }
            }
            if (self.reg(INT_RAW_REG), self.tx[ch].loop_count) == before {
                unchanged += 1;
            } else {
                unchanged = 0;
            }
        }
    }

    /// Raises `bit` for channel `ch`; if it is enabled, the transmitter
    /// pauses until it is cleared or disabled. Returns whether it paused.
    fn pause_on(&mut self, ch: usize, bit: u32) -> bool {
        self.raise(bit);
        if self.reg(INT_ENA_REG) & bit != 0 {
            self.tx[ch].waiting = bit;
            true
        } else {
            false
        }
    }

    /// `SRC_RMT` while any enabled interrupt is raised.
    pub fn pending_sources(&self) -> u64 {
        if self.int_st() != 0 {
            1u64 << SRC_RMT
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole-word store arrives as four ascending byte writes, exactly
    /// like `FirmwareBus::write32`.
    fn w(r: &mut Rmt, off: u32, val: u32) {
        for (k, b) in val.to_le_bytes().iter().enumerate() {
            r.write_byte(off + k as u32, *b);
        }
    }

    fn rd(r: &mut Rmt, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for (k, x) in b.iter_mut().enumerate() {
            *x = r.read_byte(off + k as u32);
        }
        u32::from_le_bytes(b)
    }

    /// Read-modify-write of one field, as the LL's bitfield stores compile.
    fn set_bits(r: &mut Rmt, off: u32, bits: u32) {
        let v = rd(r, off);
        w(r, off, v | bits);
    }

    fn clear_bits(r: &mut Rmt, off: u32, bits: u32) {
        let v = rd(r, off);
        w(r, off, v & !bits);
    }

    fn conf0(ch: usize) -> u32 {
        TX_CONF0_REG + 4 * ch as u32
    }

    fn tx_lim(ch: usize) -> u32 {
        TX_LIM_REG + 4 * ch as u32
    }

    fn status(ch: usize) -> u32 {
        TX_STATUS_REG + 4 * ch as u32
    }

    /// Word `i` of TX channel `ch`'s RAM (`RMTMEM.channels[ch].symbols[i]`).
    fn sym(ch: usize, i: u32) -> u32 {
        RAM_START + 4 * (BLOCK_WORDS * ch as u32 + i)
    }

    /// The WS2812 "0" bit the badge's firmware encodes (observed at boot):
    /// level 1 for 3 ticks, level 0 for 9.
    const BIT0: u32 = 0x0009_8003;
    /// `rmt_tx_mark_eof`: both durations 0 (an end-marker), level 0.
    const EOF: u32 = 0;

    /// `rmt_ll_tx_reset_pointer`.
    fn reset_pointer(r: &mut Rmt, ch: usize) {
        set_bits(r, conf0(ch), CONF0_MEM_RD_RST);
        clear_bits(r, conf0(ch), CONF0_MEM_RD_RST);
        set_bits(r, conf0(ch), CONF0_APB_MEM_RST);
        clear_bits(r, conf0(ch), CONF0_APB_MEM_RST);
    }

    /// `rmt_ll_tx_start`: `conf_update = 1`, then `tx_start = 1`.
    fn tx_start(r: &mut Rmt, ch: usize) {
        set_bits(r, conf0(ch), CONF0_CONF_UPDATE);
        set_bits(r, conf0(ch), CONF0_TX_START);
    }

    /// `rmt_ll_enable_interrupt(dev, mask, enable)`.
    fn enable_interrupt(r: &mut Rmt, mask: u32, enable: bool) {
        if enable {
            set_bits(r, INT_ENA_REG, mask);
        } else {
            clear_bits(r, INT_ENA_REG, mask);
        }
    }

    /// `rmt_new_tx_channel`'s hardware setup for `mem_block_num` blocks
    /// (`rmt_ll_tx_set_mem_blocks`, `rmt_ll_tx_set_limit(ping_pong)`,
    /// `rmt_ll_tx_fix_idle_level`, `rmt_ll_tx_enable_wrap(true)`), as the
    /// firmware does it for channel 0 with two blocks (observed: `MEM_SIZE`
    /// 2, `TX_LIM` 48).
    fn new_tx_channel(r: &mut Rmt, ch: usize, mem_block_num: u32) {
        set_bits(r, SYS_CONF_REG, 1); // rmt_ll_enable_mem_access_nonfifo
        let c = rd(r, conf0(ch)) & !(0x7 << CONF0_MEM_SIZE_S);
        w(r, conf0(ch), c | (mem_block_num << CONF0_MEM_SIZE_S));
        let l = rd(r, tx_lim(ch)) & !TX_LIM_LIMIT_MASK;
        w(r, tx_lim(ch), l | (mem_block_num * BLOCK_WORDS / 2));
        clear_bits(r, conf0(ch), 0x7 << 20); // carrier off
        set_bits(r, conf0(ch), CONF0_IDLE_OUT_EN);
        set_bits(r, conf0(ch), CONF0_MEM_TX_WRAP_EN);
    }

    /// `rmt_tx_do_transaction`'s register steps before encoding (loop
    /// count 0, no DMA).
    fn begin_transaction(r: &mut Rmt, ch: usize) {
        reset_pointer(r, ch);
        clear_bits(r, conf0(ch), CONF0_TX_CONTI_MODE); // rmt_ll_tx_enable_loop(false)
        set_bits(r, tx_lim(ch), TX_LIM_LOOP_COUNT_RESET); // rmt_ll_tx_reset_loop_count
        clear_bits(r, tx_lim(ch), TX_LIM_LOOP_COUNT_RESET);
        clear_bits(r, tx_lim(ch), TX_LIM_LOOP_CNT_EN);
        enable_interrupt(r, int_tx_loop(ch), false);
        enable_interrupt(r, int_tx_thr(ch), true);
        w(r, INT_CLR_REG, int_tx_thr(ch));
        enable_interrupt(r, int_tx_end(ch), true);
    }

    /// `rmt_tx_default_isr`'s first two lines: read `INT_ST` for this
    /// channel's TX events, then clear exactly those.
    fn isr_ack(r: &mut Rmt, ch: usize) -> u32 {
        let st = rd(r, INT_ST_REG) & (int_tx_end(ch) | int_tx_thr(ch) | int_tx_loop(ch));
        w(r, INT_CLR_REG, st);
        st
    }

    /// Every non-zero `default:` in `rmt_reg.h` (v5.5.3).
    #[test]
    fn reset_values_follow_the_register_header() {
        let mut r = Rmt::new();
        assert_eq!(rd(&mut r, 0x10), 0x0071_0200);
        assert_eq!(rd(&mut r, 0x14), 0x0071_0200);
        assert_eq!(rd(&mut r, 0x18), 0x30FF_FF02);
        assert_eq!(rd(&mut r, 0x1C), 0x1E8);
        assert_eq!(rd(&mut r, 0x20), 0x30FF_FF02);
        assert_eq!(rd(&mut r, 0x24), 0x1E8);
        assert_eq!(rd(&mut r, 0x48), 0x0040_0040);
        assert_eq!(rd(&mut r, 0x4C), 0x0040_0040);
        for off in [0x58, 0x5C, 0x60, 0x64] {
            assert_eq!(rd(&mut r, off), 0x80, "TX/RX_LIM at {off:#x}");
        }
        assert_eq!(rd(&mut r, SYS_CONF_REG), 0x0500_0010);
        assert_eq!(rd(&mut r, DATE_REG), 0x0200_6231);
        for off in [0x28, 0x2C, 0x30, 0x34, 0x38, 0x3C, 0x40, 0x44, 0x6C, 0x70] {
            assert_eq!(rd(&mut r, off), 0, "{off:#x}");
        }
        assert_eq!(r.pending_sources(), 0);
    }

    #[test]
    fn handles_names_registers_and_ram_but_not_fifo_or_gaps() {
        assert!(Rmt::handles(TX_CONF0_REG));
        assert!(Rmt::handles(REF_CNT_RST_REG + 3));
        assert!(Rmt::handles(DATE_REG));
        assert!(Rmt::handles(RAM_START));
        assert!(Rmt::handles(RAM_END - 1));
        assert!(!Rmt::handles(0x00), "CH0DATA (APB FIFO) is not modeled");
        assert!(!Rmt::handles(0x0C));
        assert!(!Rmt::handles(0x74));
        assert!(!Rmt::handles(0xC8));
        assert!(!Rmt::handles(RAM_END));
        let mut r = Rmt::new();
        w(&mut r, 0xFFC, 0xDEAD_BEEF);
        assert_eq!(rd(&mut r, 0xFFC), 0);
    }

    #[test]
    fn ram_is_plain_word_storage() {
        let mut r = Rmt::new();
        w(&mut r, sym(0, 0), BIT0);
        w(&mut r, RAM_END - 4, 0x1234_5678);
        assert_eq!(rd(&mut r, sym(0, 0)), BIT0);
        assert_eq!(rd(&mut r, RAM_END - 4), 0x1234_5678);
    }

    /// A transaction that fits in one block: `rmt_tx_do_transaction` with
    /// the encoder done in one session (`rmt_tx_mark_eof` writes the EOF
    /// symbol and disables the threshold interrupt), then `TX_START`.
    #[test]
    fn short_transmission_ends_at_the_end_marker_and_raises_tx_end() {
        let mut r = Rmt::new();
        new_tx_channel(&mut r, 0, 1);
        begin_transaction(&mut r, 0);
        for i in 0..24 {
            w(&mut r, sym(0, i), BIT0);
        }
        w(&mut r, sym(0, 24), EOF);
        enable_interrupt(&mut r, int_tx_thr(0), false);
        set_bits(&mut r, conf0(0), CONF0_IDLE_OUT_EN);
        assert_eq!(r.pending_sources(), 0);
        tx_start(&mut r, 0);

        assert!(!r.tx_running(0));
        assert_eq!(rd(&mut r, INT_ST_REG), int_tx_end(0));
        assert_eq!(r.pending_sources(), 1u64 << SRC_RMT);
        assert_eq!(
            rd(&mut r, status(0)) & STATUS_RADDR_EX_MASK,
            24,
            "stopped on the EOF word"
        );
        assert_eq!(
            rd(&mut r, conf0(0)) & (CONF0_TX_START | CONF0_CONF_UPDATE),
            0,
            "WT bits"
        );

        assert_eq!(isr_ack(&mut r, 0), int_tx_end(0));
        assert_eq!(r.pending_sources(), 0);
        // TX_LIM is 24 (half a block): the 24th word raised the (disabled)
        // threshold event, which the ISR's INT_ST-based clear leaves raw.
        assert_eq!(rd(&mut r, INT_RAW_REG), int_tx_thr(0));
    }

    /// The badge's LED transaction (observed at boot): channel 0 with two
    /// blocks, `TX_LIM` 48, wrap on, the first encoding session filling all
    /// 96 words. Each threshold event lets the ISR refill the half the
    /// transmitter just finished; the last refill writes the EOF and
    /// disables the threshold interrupt.
    #[test]
    fn ping_pong_transmission_pauses_at_each_threshold_until_the_isr_acks() {
        let mut r = Rmt::new();
        new_tx_channel(&mut r, 0, 2);
        begin_transaction(&mut r, 0);
        for i in 0..96 {
            w(&mut r, sym(0, i), BIT0);
        }
        tx_start(&mut r, 0);

        // First half sent: threshold event, transmitter waits for the ISR.
        assert!(r.tx_running(0));
        assert_eq!(rd(&mut r, INT_ST_REG), int_tx_thr(0));
        assert_eq!(rd(&mut r, status(0)) & STATUS_RADDR_EX_MASK, 48);
        assert_eq!(r.pending_sources(), 1u64 << SRC_RMT);

        // ISR 1: the clear lets the second half go; then it refills words
        // 0..48 (rmt_isr_handle_tx_threshold, mem_end = 48).
        assert_eq!(isr_ack(&mut r, 0), int_tx_thr(0));
        assert_eq!(rd(&mut r, INT_ST_REG), int_tx_thr(0), "second half sent");
        assert_eq!(rd(&mut r, status(0)) & STATUS_RADDR_EX_MASK, 96);
        for i in 0..48 {
            w(&mut r, sym(0, i), BIT0);
        }

        // ISR 2: the refilled first half goes; the ISR writes the last 10
        // symbols plus EOF into the second half and disables THR.
        assert_eq!(isr_ack(&mut r, 0), int_tx_thr(0));
        assert_eq!(rd(&mut r, status(0)) & STATUS_RADDR_EX_MASK, 48);
        for i in 48..58 {
            w(&mut r, sym(0, i), BIT0);
        }
        assert!(r.tx_running(0), "waiting on the un-acked threshold");
        w(&mut r, sym(0, 58), EOF);
        enable_interrupt(&mut r, int_tx_thr(0), false);

        assert!(!r.tx_running(0));
        assert_eq!(rd(&mut r, status(0)) & STATUS_RADDR_EX_MASK, 58);
        assert_eq!(rd(&mut r, INT_ST_REG), int_tx_end(0));
        assert_eq!(isr_ack(&mut r, 0), int_tx_end(0));
        assert_eq!(r.pending_sources(), 0);
    }

    /// With the threshold interrupt disabled, a wrap-mode transmission
    /// whose EOF lies in its second block runs straight through; the raw
    /// threshold bit is still set.
    #[test]
    fn disabled_threshold_does_not_pause_the_transmitter() {
        let mut r = Rmt::new();
        new_tx_channel(&mut r, 0, 2);
        begin_transaction(&mut r, 0);
        for i in 0..70 {
            w(&mut r, sym(0, i), BIT0);
        }
        w(&mut r, sym(0, 70), EOF);
        enable_interrupt(&mut r, int_tx_thr(0), false);
        tx_start(&mut r, 0);
        assert!(!r.tx_running(0));
        assert_eq!(rd(&mut r, INT_RAW_REG), int_tx_end(0) | int_tx_thr(0));
        assert_eq!(rd(&mut r, INT_ST_REG), int_tx_end(0));
    }

    /// TRM §33.3.7 / register 33.8: with wrap off, reading past the
    /// channel's RAM is an error: `ERR` and `MEM_EMPTY`, no `TX_END`.
    #[test]
    fn overrun_without_wrap_raises_err_and_mem_empty() {
        let mut r = Rmt::new();
        w(&mut r, INT_ENA_REG, int_tx_end(1) | int_tx_err(1));
        for i in 0..48 {
            w(&mut r, sym(1, i), BIT0);
        }
        tx_start(&mut r, 1);
        assert!(!r.tx_running(1));
        assert_eq!(rd(&mut r, INT_RAW_REG), int_tx_err(1));
        assert_ne!(rd(&mut r, status(1)) & STATUS_MEM_EMPTY, 0);
        assert_eq!(r.pending_sources(), 1u64 << SRC_RMT);
        // The next TX_START clears MEM_EMPTY.
        w(&mut r, sym(1, 0), EOF);
        tx_start(&mut r, 1);
        assert_eq!(rd(&mut r, status(1)) & STATUS_MEM_EMPTY, 0);
        assert_ne!(rd(&mut r, INT_RAW_REG) & int_tx_end(1), 0);
    }

    /// Continuous mode with a loop count (`rmt_tx_do_transaction` with
    /// `loop_count` 3): each end-marker restarts the data and counts;
    /// `TX_LOOP` fires at the third, no `TX_END`. The channel keeps
    /// running (no auto-stop) until the ISR's `rmt_ll_tx_stop`.
    #[test]
    fn loop_count_raises_tx_loop_and_keeps_running_until_tx_stop() {
        let mut r = Rmt::new();
        new_tx_channel(&mut r, 0, 1);
        reset_pointer(&mut r, 0);
        set_bits(&mut r, conf0(0), CONF0_TX_CONTI_MODE);
        set_bits(&mut r, tx_lim(0), TX_LIM_LOOP_COUNT_RESET);
        clear_bits(&mut r, tx_lim(0), TX_LIM_LOOP_COUNT_RESET);
        set_bits(&mut r, tx_lim(0), TX_LIM_LOOP_CNT_EN);
        let l = rd(&mut r, tx_lim(0)) & !(LOOP_NUM_MASK << TX_LIM_LOOP_NUM_S);
        w(&mut r, tx_lim(0), l | (3 << TX_LIM_LOOP_NUM_S));
        enable_interrupt(&mut r, int_tx_loop(0), true);
        enable_interrupt(&mut r, int_tx_thr(0), false);
        enable_interrupt(&mut r, int_tx_end(0), false);
        for i in 0..8 {
            w(&mut r, sym(0, i), BIT0);
        }
        w(&mut r, sym(0, 8), EOF);
        tx_start(&mut r, 0);

        assert_eq!(rd(&mut r, INT_RAW_REG) & !int_tx_thr(0), int_tx_loop(0));
        assert_eq!(r.pending_sources(), 1u64 << SRC_RMT);
        assert_eq!(isr_ack(&mut r, 0), int_tx_loop(0));
        assert!(r.tx_running(0), "no loop auto-stop on the ESP32-C3");
        assert_eq!(rd(&mut r, INT_RAW_REG) & int_tx_loop(0), 0, "counter holds");
        // rmt_ll_tx_stop: tx_stop (R/W/SC) then conf_update.
        set_bits(&mut r, conf0(0), CONF0_TX_STOP);
        set_bits(&mut r, conf0(0), CONF0_CONF_UPDATE);
        assert!(!r.tx_running(0));
        assert_eq!(rd(&mut r, conf0(0)) & CONF0_TX_STOP, 0, "self-cleared");
        assert_eq!(
            rd(&mut r, INT_RAW_REG) & (int_tx_end(0) | int_tx_loop(0)),
            0
        );
    }

    /// A wrap-mode transmission with no end-marker and no enabled event
    /// keeps running (its output is periodic) without hanging the model;
    /// an EOF written later is reached.
    #[test]
    fn periodic_wrap_transmission_runs_until_an_end_marker_appears() {
        let mut r = Rmt::new();
        new_tx_channel(&mut r, 0, 1);
        w(&mut r, INT_ENA_REG, int_tx_end(0));
        for i in 0..48 {
            w(&mut r, sym(0, i), BIT0);
        }
        tx_start(&mut r, 0);
        assert!(r.tx_running(0));
        assert_eq!(rd(&mut r, INT_ST_REG), 0);
        w(&mut r, sym(0, 30), EOF);
        assert!(!r.tx_running(0));
        assert_eq!(rd(&mut r, INT_ST_REG), int_tx_end(0));
    }

    /// `FirmwareBus::write32` delivers bytes 0..3 in order; the
    /// transmission starts on the byte carrying `TX_START` (byte 0) only.
    #[test]
    fn tx_start_fires_from_its_own_byte_only() {
        let mut r = Rmt::new();
        w(&mut r, INT_ENA_REG, int_tx_end(0));
        w(&mut r, sym(0, 0), BIT0);
        w(&mut r, sym(0, 1), EOF);
        let c = rd(&mut r, conf0(0));
        r.write_byte(conf0(0) + 3, ((c | CONF0_CONF_UPDATE) >> 24) as u8);
        assert_eq!(rd(&mut r, INT_RAW_REG), 0);
        r.write_byte(conf0(0), (c | CONF0_TX_START) as u8);
        assert_eq!(rd(&mut r, INT_RAW_REG), int_tx_end(0));
    }

    #[test]
    fn wt_bits_read_zero_and_int_raw_is_write_one_to_clear() {
        let mut r = Rmt::new();
        set_bits(
            &mut r,
            conf0(1),
            CONF0_MEM_RD_RST | CONF0_APB_MEM_RST | CONF0_AFIFO_RST | CONF0_CONF_UPDATE,
        );
        assert_eq!(rd(&mut r, conf0(1)), 0x0071_0200);
        set_bits(&mut r, tx_lim(1), TX_LIM_LOOP_COUNT_RESET);
        assert_eq!(rd(&mut r, tx_lim(1)), 0x80);
        w(&mut r, REF_CNT_RST_REG, 0x3); // rmt_ll_tx_reset_channels_clock_div
        assert_eq!(rd(&mut r, REF_CNT_RST_REG), 0);
        w(&mut r, 0x1C, 0x1E8 | 0b110 | (0b11 << 14)); // RX CONF1 WT bits
        assert_eq!(rd(&mut r, 0x1C), 0x1E8);
        // An EOF-only transmission to raise TX_END, then clear it via RAW.
        w(&mut r, sym(1, 0), EOF);
        tx_start(&mut r, 1);
        assert_eq!(rd(&mut r, INT_RAW_REG), int_tx_end(1));
        w(&mut r, INT_RAW_REG, int_tx_end(1));
        assert_eq!(rd(&mut r, INT_RAW_REG), 0);
        assert_eq!(rd(&mut r, INT_CLR_REG), 0);
    }

    #[test]
    fn plain_registers_read_back_their_rw_bits() {
        let mut r = Rmt::new();
        // rmt_ll_set_group_clock_src(APB, 1, 0, 0) + clocks on.
        w(&mut r, SYS_CONF_REG, 0x8500_0003);
        assert_eq!(rd(&mut r, SYS_CONF_REG), 0x8500_0003);
        w(&mut r, TX_SIM_REG, 0xFFFF_FFFF);
        assert_eq!(rd(&mut r, TX_SIM_REG), 0x7);
        w(&mut r, 0x50, 0x1234_5678);
        assert_eq!(rd(&mut r, 0x50), 0x1234_5678);
        w(&mut r, INT_ENA_REG, 0xFFFF_FFFF);
        assert_eq!(rd(&mut r, INT_ENA_REG), INT_MASK);
    }
}

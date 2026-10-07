//! The ESP32-C3's I2C controller, I2C0 (`DR_REG_I2C_EXT_BASE = 0x6001_3000`,
//! `soc/reg_base.h`; `crate::mem::soc::I2C0_RANGE`), in master mode with
//! **no device attached to the bus** (Milestone 4 Task D-M4-1; see
//! `docs/milestone-4-decisions.md`).
//!
//! The firmware drives it through ESP-IDF v5.5.3's interrupt-driven master
//! driver (`components/esp_driver_i2c/i2c_master.c`):
//! `s_i2c_send_commands()` fills the command list and TX FIFO, calls
//! `i2c_hal_master_trans_start()`, then blocks on `event_queue`, which only
//! `i2c_master_isr_handler_default()` feeds, from `ETS_I2C_EXT0_INTR_SOURCE`
//! ([`crate::mem::soc::SRC_I2C_EXT0`], source 29 in
//! `soc/esp32c3/include/soc/interrupts.h`). On `I2C_EVENT_NACK` it then
//! polls `i2c_ll_is_bus_busy()` until the STOP the hardware sends after the
//! NACK ("start->address->nack->stop") has released the bus.
//!
//! ## Sources (all ESP-IDF v5.5.3)
//!
//! - Offsets, fields, access types and reset values:
//!   `components/soc/esp32c3/register/soc/i2c_reg.h` and `i2c_struct.h`.
//! - Accessors: `components/hal/esp32c3/include/hal/i2c_ll.h`
//!   (`i2c_ll_hw_cmd_t`: `byte_num` 7:0, `ack_en` 8, `ack_exp` 9, `ack_val`
//!   10, `op_code` 13:11, `done` 31; `I2C_LL_CMD_*`;
//!   `I2C_LL_MASTER_EVENT_INTR` = NACK | TIME_OUT | TRANS_COMPLETE |
//!   ARBITRATION_LOST | END_DETECT = `0x5A8`; `i2c_ll_master_write_cmd_reg`
//!   writes `command[i].val`; `i2c_ll_update` sets `ctr.conf_upgate`;
//!   `i2c_ll_start_trans` sets `ctr.trans_start`;
//!   `i2c_ll_write_txfifo` writes `fifo_data.val` per byte and
//!   `i2c_ll_read_rxfifo` reads it per byte; `i2c_ll_txfifo_rst`/`rxfifo_rst`
//!   set then clear `fifo_conf.*_rst`; `i2c_ll_get_intr_mask` reads
//!   `int_status`; `i2c_ll_clear_intr_mask` writes `int_clr`;
//!   `i2c_ll_is_bus_busy` reads `sr.bus_busy`; `i2c_ll_master_clr_bus` sets
//!   `scl_sp_conf.scl_rst_slv_en`, which "hardware will clear ... after
//!   sending SCL pulses", and `i2c_ll_master_is_bus_clear_done` polls it).
//! - Driver flow: `components/esp_driver_i2c/i2c_master.c`
//!   (`s_i2c_send_commands`, `i2c_master_isr_handler_default`,
//!   `i2c_master_probe`, `s_i2c_master_clear_bus`). The ESP32-C3 has
//!   `SOC_I2C_SUPPORT_HW_CLR_BUS` and not `SOC_I2C_STOP_INDEPENDENT`
//!   (`soc/esp32c3/include/soc/soc_caps.h`).
//!
//! ## What is modeled
//!
//! - **Command list** ([`COMD0_REG`]..`COMD7_REG`, `COMMANDn` R/W bits 13:0,
//!   `COMMANDn_DONE` R/W/SS bit 31): writing the CTR byte that sets
//!   `TRANS_START` (bit 5) with `MS_MODE` (bit 4) set runs the list from
//!   command 0 to completion inside that byte write, since nothing on the
//!   bus can stretch or delay it. Each executed command gets its done bit.
//!   The list ends at STOP or END; an invalid op code, or running past
//!   command 7, ends it too without further effect.
//! - **No device: every ACK slot reads 1 (NACK).** With nobody pulling SDA
//!   low, the ACK bit after each byte the master sends is high, so
//!   `SR.RESP_REC` (the received ACK level) reads 1 after a WRITE byte. A
//!   WRITE with `ack_en` set and `ack_exp` 0 therefore fails on its first
//!   byte: the model raises `NACK_INT`, then sends the STOP the driver waits
//!   for (bus released, `TRANS_COMPLETE_INT`, a judgment call: the
//!   controller sees its own STOP bit), and stops; the failing WRITE and the
//!   rest of the list stay not done. A READ (reachable only after a WRITE
//!   without ACK check) samples the released SDA line: every byte is
//!   `0xFF`.
//! - **Other interrupt bits** the list raises: `TRANS_START_INT` on each
//!   RSTART, `BYTE_TRANS_DONE_INT` per byte sent (including a NACKed one)
//!   or received, `TRANS_COMPLETE_INT` on STOP, `END_DETECT_INT` on END,
//!   `MST_TXFIFO_UDF_INT` when a WRITE finds the TX FIFO empty,
//!   `RXFIFO_OVF_INT` when a READ finds the RX FIFO full, `TXFIFO_OVF_INT`
//!   on a `DATA` write to a full TX FIFO, `RXFIFO_UDF_INT` on a `DATA` read
//!   of an empty RX FIFO. Not modeled: timeouts, arbitration, the watermark
//!   interrupts (`TXFIFO_WM_INT_RAW` keeps its reset value 1 until cleared
//!   and is never re-raised), slave mode, non-FIFO mode (`DATA` behaves as
//!   in FIFO mode regardless of `NONFIFO_EN`).
//! - **Interrupt registers**: `INT_RAW` (R/SS/WTC: a 1 written clears),
//!   `INT_CLR` (WT: a 1 clears the raw bit; reads 0), `INT_ENA` (R/W),
//!   `INT_STATUS` = `INT_RAW & INT_ENA` (RO). [`I2c::pending_sources`]
//!   asserts `SRC_I2C_EXT0` while `INT_STATUS` is non-zero, as a level.
//! - **FIFOs**: two 32-byte RAMs (`SOC_I2C_FIFO_LEN`). A byte-0 write to
//!   [`DATA_REG`] pushes to TX and a byte-0 read pops RX (`FIFO_RDATA` is
//!   bits 7:0, so the other three bytes of a word access read 0 and have no
//!   effect). The RAMs are readable at `txfifo_mem`/`rxfifo_mem`
//!   (`+0x100`/`+0x180`, one byte per word); a CPU write lands in TX RAM
//!   without moving the pointers and is dropped for RX RAM.
//!   `FIFO_CONF.TX_FIFO_RST`/`RX_FIFO_RST` (bits 13/12) empty their FIFO
//!   while set. `SR.RXFIFO_CNT`/`TXFIFO_CNT` and `FIFO_ST`'s four pointers
//!   are computed from the FIFO state.
//! - **Bus state**: `SR.BUS_BUSY` is 1 from an RSTART until a STOP (an END
//!   leaves the bus held), otherwise 0. `FSM_RST` releases it.
//! - **Self-clearing bits**: CTR's `TRANS_START`, `FSM_RST` and
//!   `CONF_UPGATE` are WT and always read 0; `CONF_UPGATE`'s register
//!   synchronization is a no-op (the model always uses live register
//!   values). `SCL_SP_CONF.SCL_RST_SLV_EN` (R/W/SC) reads 0 right after it
//!   is set: the bus-clear pulses finish at once with nothing holding SDA.
//! - **Everything else** in `0x00..=0x84` and `DATE` (`0xF8`): plain word
//!   storage. `SR.STRETCH_CAUSE` reads its reset value 3. Every register
//!   with a non-zero `default:` in `i2c_reg.h` starts at it
//!   ([`I2c::new`]); all others start at 0.
//!
//! Offsets [`I2c::handles`] does not name read 0, drop writes and are
//! logged by the bus as unmapped.

use super::set_byte;
use crate::mem::soc::SRC_I2C_EXT0;

/// `I2C_CTR_REG`.
pub const CTR_REG: u32 = 0x04;
/// `I2C_SR_REG` (RO, computed).
pub const SR_REG: u32 = 0x08;
/// `I2C_FIFO_ST_REG` (RO, computed).
pub const FIFO_ST_REG: u32 = 0x14;
/// `I2C_FIFO_CONF_REG`.
pub const FIFO_CONF_REG: u32 = 0x18;
/// `I2C_DATA_REG` (`fifo_data`).
pub const DATA_REG: u32 = 0x1C;
/// `I2C_INT_RAW_REG`.
pub const INT_RAW_REG: u32 = 0x20;
/// `I2C_INT_CLR_REG`.
pub const INT_CLR_REG: u32 = 0x24;
/// `I2C_INT_ENA_REG`.
pub const INT_ENA_REG: u32 = 0x28;
/// `I2C_INT_STATUS_REG`.
pub const INT_STATUS_REG: u32 = 0x2C;
/// `I2C_COMD0_REG`; `COMDn` is at `COMD0_REG + 4 * n`, `n < 8`.
pub const COMD0_REG: u32 = 0x58;
/// `I2C_SCL_STRETCH_CONF_REG`, the last named register before the gap.
/// `I2C_SCL_SP_CONF_REG`.
pub const SCL_SP_CONF_REG: u32 = 0x80;
pub const SCL_STRETCH_CONF_REG: u32 = 0x84;
/// `I2C_DATE_REG`.
pub const DATE_REG: u32 = 0xF8;
/// `I2C_TXFIFO_START_ADDR_REG`: TX RAM, one byte per word.
pub const TXFIFO_START_ADDR: u32 = 0x100;
/// `I2C_RXFIFO_START_ADDR_REG`: RX RAM, one byte per word.
pub const RXFIFO_START_ADDR: u32 = 0x180;
/// End of the register block (exclusive).
pub const REGS_END: u32 = 0x200;

/// The reserved word between `SCL_HIGH_PERIOD` and `SCL_START_HOLD`
/// (`i2c_struct.h`'s `reserved_3c`).
const RESERVED_3C: u32 = 0x3C;

/// `I2C_MS_MODE`, CTR bit 4.
pub const CTR_MS_MODE: u32 = 1 << 4;
/// `I2C_TRANS_START`, CTR bit 5 (WT).
pub const CTR_TRANS_START: u32 = 1 << 5;
/// `I2C_FSM_RST`, CTR bit 10 (WT).
pub const CTR_FSM_RST: u32 = 1 << 10;
/// `I2C_CONF_UPGATE`, CTR bit 11 (WT).
pub const CTR_CONF_UPGATE: u32 = 1 << 11;
const CTR_WT_BITS: u32 = CTR_TRANS_START | CTR_FSM_RST | CTR_CONF_UPGATE;

/// `I2C_RESP_REC`, SR bit 0.
pub const SR_RESP_REC: u32 = 1 << 0;
/// `I2C_BUS_BUSY`, SR bit 4.
pub const SR_BUS_BUSY: u32 = 1 << 4;
/// `I2C_RXFIFO_CNT_S`.
pub const SR_RXFIFO_CNT_S: u32 = 8;
/// `I2C_STRETCH_CAUSE`, SR bits 15:14 (RO, `default: 2'h3`).
const SR_STRETCH_CAUSE_RESET: u32 = 0x3 << 14;
/// `I2C_TXFIFO_CNT_S`.
pub const SR_TXFIFO_CNT_S: u32 = 18;

/// `I2C_NONFIFO_EN`, FIFO_CONF bit 10.
pub const FIFO_CONF_NONFIFO_EN: u32 = 1 << 10;
/// `I2C_RX_FIFO_RST`, FIFO_CONF bit 12.
pub const FIFO_CONF_RX_FIFO_RST: u32 = 1 << 12;
/// `I2C_TX_FIFO_RST`, FIFO_CONF bit 13.
pub const FIFO_CONF_TX_FIFO_RST: u32 = 1 << 13;

/// Interrupt bits (`I2C_*_INT_RAW_S`, the same layout in RAW/CLR/ENA/ST).
pub const INT_TXFIFO_WM: u32 = 1 << 1;
pub const INT_END_DETECT: u32 = 1 << 3;
pub const INT_BYTE_TRANS_DONE: u32 = 1 << 4;
pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_TRANS_START: u32 = 1 << 9;
pub const INT_NACK: u32 = 1 << 10;
pub const INT_TXFIFO_OVF: u32 = 1 << 11;
pub const INT_RXFIFO_UDF: u32 = 1 << 12;
/// Bits 17:0 exist (`i2c_struct.h`: `reserved18 : 14`).
const INT_MASK: u32 = 0x3_FFFF;

/// Command register fields (`i2c_ll_hw_cmd_t`, `i2c_ll.h`).
const CMD_BYTE_NUM_MASK: u32 = 0xFF;
const CMD_ACK_EN: u32 = 1 << 8;
const CMD_ACK_EXP: u32 = 1 << 9;
const CMD_OP_CODE_S: u32 = 11;
const CMD_OP_CODE_MASK: u32 = 0x7;
/// `I2C_COMMAND0` (bits 13:0) is R/W; `I2C_COMMAND0_DONE` is bit 31.
const CMD_FIELD_MASK: u32 = 0x3FFF;
pub const CMD_DONE: u32 = 1 << 31;

/// `I2C_LL_CMD_*`.
pub const OP_WRITE: u32 = 1;
pub const OP_STOP: u32 = 2;
pub const OP_READ: u32 = 3;
pub const OP_END: u32 = 4;
pub const OP_RSTART: u32 = 6;

/// Number of command registers.
const NUM_COMMANDS: u32 = 8;
/// `SOC_I2C_FIFO_LEN`: both RAMs are 32 x 8 bits (`soc_caps.h`).
pub const FIFO_LEN: usize = 32;

/// Every register whose `i2c_reg.h` (v5.5.3) `default:` is non-zero, as
/// `(offset, reset value)`. Every other register resets to 0.
const RESET_VALUES: [(u32, u32); 13] = [
    (CTR_REG, 0x20B),        // SDA/SCL_FORCE_OUT, RX_FULL_ACK_LEVEL, ARBITRATION_EN
    (0x0C, 0x10),            // TO_REG: TIME_OUT_REG 5'h10
    (FIFO_CONF_REG, 0x408B), // FIFO_PRT_EN, TXFIFO_WM_THRHD 4, RXFIFO_WM_THRHD 0xB
    (INT_RAW_REG, INT_TXFIFO_WM),
    (0x40, 8),       // SCL_START_HOLD_TIME
    (0x44, 8),       // SCL_RSTART_SETUP_TIME
    (0x48, 8),       // SCL_STOP_HOLD_TIME
    (0x4C, 8),       // SCL_STOP_SETUP_TIME
    (0x50, 0x300),   // FILTER_CFG: SCL_FILTER_EN | SDA_FILTER_EN
    (0x54, 1 << 21), // CLK_CONF: SCLK_ACTIVE
    (0x78, 0x10),    // SCL_ST_TO_REG
    (0x7C, 0x10),    // SCL_MAIN_ST_TO_REG
    (DATE_REG, 0x2007_0201),
];

/// `I2C_SCL_RST_SLV_EN`, SCL_SP_CONF bit 0 (R/W/SC).
const SCL_RST_SLV_EN: u32 = 1 << 0;
/// `I2C_MST_TXFIFO_UDF_INT`, `I2C_RXFIFO_OVF_INT`.
pub const INT_MST_TXFIFO_UDF: u32 = 1 << 6;
pub const INT_RXFIFO_OVF: u32 = 1 << 2;

/// The level an undriven, pulled-up SDA line reads: a NACK in the ACK
/// slot, and `0xFF` for a data byte.
const RELEASED_ACK: u32 = 1;
const RELEASED_BYTE: u8 = 0xFF;

/// One 32-byte FIFO RAM with its read/write pointers.
#[derive(Clone)]
struct Fifo {
    ram: [u8; FIFO_LEN],
    raddr: usize,
    waddr: usize,
    count: usize,
}

impl Fifo {
    fn new() -> Self {
        Self {
            ram: [0; FIFO_LEN],
            raddr: 0,
            waddr: 0,
            count: 0,
        }
    }

    fn reset(&mut self) {
        self.raddr = 0;
        self.waddr = 0;
        self.count = 0;
    }

    /// `false` if full (the byte is dropped).
    fn push(&mut self, b: u8) -> bool {
        if self.count >= FIFO_LEN {
            return false;
        }
        self.ram[self.waddr] = b;
        self.waddr = (self.waddr + 1) % FIFO_LEN;
        self.count += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.count == 0 {
            return None;
        }
        let b = self.ram[self.raddr];
        self.raddr = (self.raddr + 1) % FIFO_LEN;
        self.count -= 1;
        Some(b)
    }
}

/// The I2C0 controller. See the module doc.
pub struct I2c {
    /// Word storage for `0x00..REGS_END`, indexed by `offset / 4`. SR,
    /// FIFO_ST, INT_STATUS, the RAM windows and the WT bits are computed
    /// instead of read from here.
    regs: [u32; (REGS_END / 4) as usize],
    tx: Fifo,
    rx: Fifo,
    bus_busy: bool,
    resp_rec: bool,
}

impl Default for I2c {
    fn default() -> Self {
        Self::new()
    }
}

impl I2c {
    pub fn new() -> Self {
        let mut regs = [0u32; (REGS_END / 4) as usize];
        for (off, val) in RESET_VALUES {
            regs[(off / 4) as usize] = val;
        }
        Self {
            regs,
            tx: Fifo::new(),
            rx: Fifo::new(),
            bus_busy: false,
            resp_rec: false,
        }
    }

    /// `true` for every offset backed by a named register or RAM word.
    pub fn handles(offset: u32) -> bool {
        let word = offset & !0b11;
        (word <= SCL_STRETCH_CONF_REG && word != RESERVED_3C)
            || word == DATE_REG
            || (TXFIFO_START_ADDR..REGS_END).contains(&word)
    }

    fn reg(&self, off: u32) -> u32 {
        self.regs[(off / 4) as usize]
    }

    fn reg_mut(&mut self, off: u32) -> &mut u32 {
        &mut self.regs[(off / 4) as usize]
    }

    fn int_raw(&self) -> u32 {
        self.reg(INT_RAW_REG)
    }

    fn raise(&mut self, bits: u32) {
        *self.reg_mut(INT_RAW_REG) |= bits;
    }

    fn int_status(&self) -> u32 {
        self.int_raw() & self.reg(INT_ENA_REG)
    }

    fn sr(&self) -> u32 {
        u32::from(self.resp_rec)
            | if self.bus_busy { SR_BUS_BUSY } else { 0 }
            | ((self.rx.count as u32) << SR_RXFIFO_CNT_S)
            | SR_STRETCH_CAUSE_RESET
            | ((self.tx.count as u32) << SR_TXFIFO_CNT_S)
    }

    fn fifo_st(&self) -> u32 {
        (self.rx.raddr as u32)
            | ((self.rx.waddr as u32) << 5)
            | ((self.tx.raddr as u32) << 10)
            | ((self.tx.waddr as u32) << 15)
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        if !Self::handles(offset) {
            return 0;
        }
        let word = offset & !0b11;
        let idx = offset & 0b11;
        let value = match word {
            SR_REG => self.sr(),
            FIFO_ST_REG => self.fifo_st(),
            DATA_REG => {
                // i2c_ll_read_rxfifo reads the whole word; only the byte
                // holding FIFO_RDATA (bits 7:0) pops.
                if idx != 0 {
                    return 0;
                }
                match self.rx.pop() {
                    Some(b) => u32::from(b),
                    None => {
                        self.raise(INT_RXFIFO_UDF);
                        0
                    }
                }
            }
            INT_CLR_REG => 0,
            INT_STATUS_REG => self.int_status(),
            w if w >= RXFIFO_START_ADDR => {
                u32::from(self.rx.ram[((w - RXFIFO_START_ADDR) / 4) as usize])
            }
            w if w >= TXFIFO_START_ADDR => {
                u32::from(self.tx.ram[((w - TXFIFO_START_ADDR) / 4) as usize])
            }
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
            SR_REG | FIFO_ST_REG | INT_STATUS_REG => {}
            DATA_REG => {
                if idx == 0 && !self.tx.push(val) {
                    self.raise(INT_TXFIFO_OVF);
                }
            }
            // INT_RAW is R/SS/WTC, INT_CLR is WT: a 1 clears the raw bit.
            INT_RAW_REG | INT_CLR_REG => *self.reg_mut(INT_RAW_REG) &= !(bits & INT_MASK),
            CTR_REG => {
                let mut ctr = self.reg(CTR_REG);
                set_byte(&mut ctr, idx, val);
                *self.reg_mut(CTR_REG) = ctr & !CTR_WT_BITS;
                let wt = bits & CTR_WT_BITS;
                if wt & CTR_FSM_RST != 0 {
                    self.bus_busy = false;
                }
                if wt & CTR_TRANS_START != 0 && ctr & CTR_MS_MODE != 0 {
                    self.run_commands();
                }
            }
            FIFO_CONF_REG => {
                set_byte(self.reg_mut(FIFO_CONF_REG), idx, val);
                let conf = self.reg(FIFO_CONF_REG);
                if conf & FIFO_CONF_TX_FIFO_RST != 0 {
                    self.tx.reset();
                }
                if conf & FIFO_CONF_RX_FIFO_RST != 0 {
                    self.rx.reset();
                }
            }
            SCL_SP_CONF_REG => {
                set_byte(self.reg_mut(SCL_SP_CONF_REG), idx, val);
                // The bus-clear pulses finish at once: nothing holds SDA.
                *self.reg_mut(SCL_SP_CONF_REG) &= !SCL_RST_SLV_EN;
            }
            w if (COMD0_REG..COMD0_REG + 4 * NUM_COMMANDS).contains(&w) => {
                set_byte(self.reg_mut(w), idx, val);
                *self.reg_mut(w) &= CMD_FIELD_MASK | CMD_DONE;
            }
            w if w >= RXFIFO_START_ADDR => {}
            w if w >= TXFIFO_START_ADDR => {
                if idx == 0 {
                    self.tx.ram[((w - TXFIFO_START_ADDR) / 4) as usize] = val;
                }
            }
            w => set_byte(self.reg_mut(w), idx, val),
        }
    }

    /// Runs the command list from `COMD0` until a STOP, an END, a NACK or
    /// an invalid op code, with no device on the bus.
    fn run_commands(&mut self) {
        for n in 0..NUM_COMMANDS {
            let off = COMD0_REG + 4 * n;
            let cmd = self.reg(off);
            let byte_num = cmd & CMD_BYTE_NUM_MASK;
            match (cmd >> CMD_OP_CODE_S) & CMD_OP_CODE_MASK {
                OP_RSTART => {
                    self.bus_busy = true;
                    self.raise(INT_TRANS_START);
                }
                OP_WRITE => {
                    for _ in 0..byte_num {
                        if self.tx.pop().is_none() {
                            self.raise(INT_MST_TXFIFO_UDF);
                        }
                        self.raise(INT_BYTE_TRANS_DONE);
                        // SR.RESP_REC: the received ACK level, 1 = NACK.
                        self.resp_rec = true;
                        let ack_exp = u32::from(cmd & CMD_ACK_EXP != 0);
                        if cmd & CMD_ACK_EN != 0 && ack_exp != RELEASED_ACK {
                            // NACK_INT, then the STOP s_i2c_send_commands waits for.
                            self.raise(INT_NACK);
                            self.stop();
                            return;
                        }
                    }
                }
                OP_READ => {
                    for _ in 0..byte_num {
                        if !self.rx.push(RELEASED_BYTE) {
                            self.raise(INT_RXFIFO_OVF);
                        }
                        self.raise(INT_BYTE_TRANS_DONE);
                    }
                }
                OP_STOP => {
                    *self.reg_mut(off) |= CMD_DONE;
                    self.stop();
                    return;
                }
                OP_END => {
                    *self.reg_mut(off) |= CMD_DONE;
                    self.raise(INT_END_DETECT);
                    return;
                }
                _ => return,
            }
            *self.reg_mut(off) |= CMD_DONE;
        }
    }

    /// A STOP condition: releases the bus, `TRANS_COMPLETE_INT`.
    fn stop(&mut self) {
        self.bus_busy = false;
        self.raise(INT_TRANS_COMPLETE);
    }

    /// `SRC_I2C_EXT0` while any enabled interrupt is raised.
    pub fn pending_sources(&self) -> u64 {
        if self.int_status() != 0 {
            1u64 << SRC_I2C_EXT0
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
    fn w(i: &mut I2c, off: u32, val: u32) {
        for (k, b) in val.to_le_bytes().iter().enumerate() {
            i.write_byte(off + k as u32, *b);
        }
    }

    fn r(i: &mut I2c, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for (k, x) in b.iter_mut().enumerate() {
            *x = i.read_byte(off + k as u32);
        }
        u32::from_le_bytes(b)
    }

    /// Read-modify-write of one field, as the LL's bitfield stores compile.
    fn set_bits(i: &mut I2c, off: u32, bits: u32) {
        let v = r(i, off);
        w(i, off, v | bits);
    }

    fn clear_bits(i: &mut I2c, off: u32, bits: u32) {
        let v = r(i, off);
        w(i, off, v & !bits);
    }

    fn comd(n: u32) -> u32 {
        COMD0_REG + 4 * n
    }

    fn cmd(op: u32, ack_en: bool, ack_val: bool, byte_num: u32) -> u32 {
        (op << CMD_OP_CODE_S) | (u32::from(ack_en) << 8) | (u32::from(ack_val) << 10) | byte_num
    }

    /// `i2c_hal_master_init` + `i2c_ll_enable_intr_mask` far enough to run a
    /// transaction: master mode, FIFO resets, `I2C_LL_MASTER_EVENT_INTR`.
    fn master_ready() -> I2c {
        let mut i = I2c::new();
        set_bits(&mut i, CTR_REG, CTR_MS_MODE);
        set_bits(&mut i, CTR_REG, CTR_CONF_UPGATE); // i2c_ll_update
        set_bits(&mut i, FIFO_CONF_REG, FIFO_CONF_TX_FIFO_RST); // i2c_ll_txfifo_rst
        clear_bits(&mut i, FIFO_CONF_REG, FIFO_CONF_TX_FIFO_RST);
        set_bits(&mut i, FIFO_CONF_REG, FIFO_CONF_RX_FIFO_RST); // i2c_ll_rxfifo_rst
        clear_bits(&mut i, FIFO_CONF_REG, FIFO_CONF_RX_FIFO_RST);
        set_bits(&mut i, INT_ENA_REG, 0x5A8); // I2C_LL_MASTER_EVENT_INTR
        i
    }

    /// `i2c_hal_master_trans_start`: `i2c_ll_update` then `i2c_ll_start_trans`.
    fn trans_start(i: &mut I2c) {
        set_bits(i, CTR_REG, CTR_CONF_UPGATE);
        set_bits(i, CTR_REG, CTR_TRANS_START);
    }

    /// Every non-zero `default:` in `i2c_reg.h` (v5.5.3).
    #[test]
    fn reset_values_follow_the_register_header() {
        let mut i = I2c::new();
        assert_eq!(r(&mut i, CTR_REG), 0x20B);
        assert_eq!(r(&mut i, SR_REG), 0x3 << 14); // STRETCH_CAUSE 2'h3
        assert_eq!(r(&mut i, 0x0C), 0x10); // TO_REG: TIME_OUT_REG 5'h10
        assert_eq!(r(&mut i, FIFO_CONF_REG), 0x408B);
        assert_eq!(r(&mut i, INT_RAW_REG), INT_TXFIFO_WM); // TXFIFO_WM_INT_RAW 1'b1
        for off in [0x40, 0x44, 0x48, 0x4C] {
            assert_eq!(r(&mut i, off), 8, "SCL start/stop hold/setup at {off:#x}");
        }
        assert_eq!(r(&mut i, 0x50), 0x300); // FILTER_CFG: SCL_EN | SDA_EN
        assert_eq!(r(&mut i, 0x54), 1 << 21); // CLK_CONF: SCLK_ACTIVE
        assert_eq!(r(&mut i, 0x78), 0x10); // SCL_ST_TIME_OUT
        assert_eq!(r(&mut i, 0x7C), 0x10); // SCL_MAIN_ST_TIME_OUT
        assert_eq!(r(&mut i, DATE_REG), 0x2007_0201);
        assert_eq!(r(&mut i, INT_STATUS_REG), 0);
        assert_eq!(i.pending_sources(), 0);
    }

    /// `i2c_master_probe` (i2c_master.c): RSTART; WRITE 1 (address, ACK
    /// check on, expected ACK 0); STOP. With no device: NACK, then STOP.
    #[test]
    fn probe_of_an_absent_address_nacks_and_releases_the_bus() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_RSTART, false, false, 0));
        w(&mut i, DATA_REG, 0x19 << 1); // I2C_ADDRESS_TRANS_WRITE(0x19)
        w(&mut i, comd(1), cmd(OP_WRITE, true, false, 1));
        w(&mut i, comd(2), cmd(OP_STOP, false, false, 0));
        trans_start(&mut i);
        let st = r(&mut i, INT_STATUS_REG);
        assert_eq!(st, INT_NACK | INT_TRANS_COMPLETE);
        assert_eq!(r(&mut i, SR_REG) & SR_BUS_BUSY, 0);
        assert_eq!(
            (r(&mut i, SR_REG) >> SR_TXFIFO_CNT_S) & 0x3F,
            0,
            "address sent"
        );
        assert_eq!(i.pending_sources(), 1u64 << SRC_I2C_EXT0);
    }

    /// `FirmwareBus::write32` delivers bytes 0..3 in order; the transaction
    /// fires on the byte that carries `TRANS_START` (byte 0) and only once.
    #[test]
    fn trans_start_fires_from_its_own_byte_only() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_RSTART, false, false, 0));
        w(&mut i, comd(1), cmd(OP_STOP, false, false, 0));
        // A write of byte 1 alone (CONF_UPGATE) does not start anything.
        let ctr = r(&mut i, CTR_REG);
        i.write_byte(CTR_REG + 1, ((ctr | CTR_CONF_UPGATE) >> 8) as u8);
        assert_eq!(r(&mut i, comd(0)) & CMD_DONE, 0);
        i.write_byte(CTR_REG, (ctr | CTR_TRANS_START) as u8);
        assert_ne!(r(&mut i, comd(1)) & CMD_DONE, 0);
        assert_eq!(
            r(&mut i, INT_RAW_REG) & INT_TRANS_COMPLETE,
            INT_TRANS_COMPLETE
        );
    }

    /// `i2c_ll_master_clr_bus` sets `SCL_RST_SLV_EN` (R/W/SC) and
    /// `s_i2c_master_clear_bus` polls it until hardware clears it. With
    /// nothing holding SDA the pulses finish at once.
    #[test]
    fn bus_clear_self_clears_scl_rst_slv_en() {
        let mut i = master_ready();
        w(&mut i, SCL_SP_CONF_REG, (9 << 1) | 1);
        assert_eq!(r(&mut i, SCL_SP_CONF_REG), 9 << 1);
    }

    #[test]
    fn int_ena_gates_the_source_level() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_STOP, false, false, 0));
        w(&mut i, INT_ENA_REG, 0);
        trans_start(&mut i);
        assert_ne!(r(&mut i, INT_RAW_REG) & INT_TRANS_COMPLETE, 0);
        assert_eq!(i.pending_sources(), 0);
        w(&mut i, INT_ENA_REG, INT_TRANS_COMPLETE);
        assert_eq!(i.pending_sources(), 1u64 << SRC_I2C_EXT0);
    }

    #[test]
    fn plain_registers_read_back_and_wt_bits_read_zero() {
        let mut i = I2c::new();
        w(&mut i, 0x00, 0x31); // SCL_LOW_PERIOD
        w(&mut i, 0x38, 0x2E1B); // SCL_HIGH_PERIOD
        w(&mut i, 0x54, 0x20_0000); // CLK_CONF.sclk_active
        assert_eq!(r(&mut i, 0x00), 0x31);
        assert_eq!(r(&mut i, 0x38), 0x2E1B);
        assert_eq!(r(&mut i, 0x54), 0x20_0000);
        set_bits(&mut i, CTR_REG, CTR_MS_MODE | CTR_CONF_UPGATE | CTR_FSM_RST);
        assert_eq!(r(&mut i, CTR_REG), 0x20B | CTR_MS_MODE);
    }

    #[test]
    fn handles_names_registers_and_rams_but_not_the_gaps() {
        assert!(I2c::handles(CTR_REG));
        assert!(I2c::handles(COMD0_REG + 7 * 4 + 3));
        assert!(I2c::handles(SCL_STRETCH_CONF_REG));
        assert!(I2c::handles(DATE_REG));
        assert!(I2c::handles(TXFIFO_START_ADDR));
        assert!(I2c::handles(RXFIFO_START_ADDR + 31 * 4));
        assert!(!I2c::handles(RESERVED_3C));
        assert!(!I2c::handles(0x88));
        assert!(!I2c::handles(0xFC));
        assert!(!I2c::handles(REGS_END));
        // Unhandled offsets never panic.
        let mut i = I2c::new();
        w(&mut i, 0xFFC, 0xDEAD_BEEF);
        assert_eq!(r(&mut i, 0xFFC), 0);
    }

    #[test]
    fn data_writes_fill_the_tx_fifo() {
        let mut i = master_ready();
        // i2c_ll_write_txfifo: `fifo_data.val = ptr[i]`.
        for b in [0x32, 0x0F, 0x33] {
            w(&mut i, DATA_REG, b);
        }
        assert_eq!((r(&mut i, SR_REG) >> SR_TXFIFO_CNT_S) & 0x3F, 3);
        assert_eq!(r(&mut i, TXFIFO_START_ADDR), 0x32);
        assert_eq!(r(&mut i, TXFIFO_START_ADDR + 4), 0x0F);
        assert_eq!(r(&mut i, TXFIFO_START_ADDR + 8), 0x33);
        // FIFO_ST.tx_fifo_waddr (bits 19:15) follows the pushes.
        assert_eq!((r(&mut i, FIFO_ST_REG) >> 15) & 0x1F, 3);
        set_bits(&mut i, FIFO_CONF_REG, FIFO_CONF_TX_FIFO_RST);
        clear_bits(&mut i, FIFO_CONF_REG, FIFO_CONF_TX_FIFO_RST);
        assert_eq!((r(&mut i, SR_REG) >> SR_TXFIFO_CNT_S) & 0x3F, 0);
        assert_eq!(r(&mut i, FIFO_ST_REG), 0);
    }

    /// The firmware's accelerometer WHO_AM_I read as `s_i2c_send_commands`
    /// builds it (observed at boot): RSTART; WRITE 2 (addr 0x19 W, reg
    /// 0x0F) with ACK check; RSTART; WRITE 1 (addr 0x19 R) with ACK check;
    /// READ 1 with NACK; STOP. With no device the address byte is NACKed.
    #[test]
    fn address_nack_with_no_device_raises_nack_and_stops() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_RSTART, false, false, 0));
        w(&mut i, DATA_REG, 0x32);
        w(&mut i, DATA_REG, 0x0F);
        w(&mut i, comd(1), cmd(OP_WRITE, true, false, 2));
        w(&mut i, comd(2), cmd(OP_RSTART, false, false, 0));
        w(&mut i, DATA_REG, 0x33);
        w(&mut i, comd(3), cmd(OP_WRITE, true, false, 1));
        w(&mut i, comd(4), cmd(OP_READ, false, true, 1));
        w(&mut i, comd(5), cmd(OP_STOP, false, false, 0));
        assert_eq!(i.pending_sources(), 0);
        trans_start(&mut i);

        let raw = r(&mut i, INT_RAW_REG);
        assert_ne!(raw & INT_NACK, 0, "NACK_INT_RAW");
        assert_ne!(raw & INT_TRANS_COMPLETE, 0, "STOP after the NACK");
        assert_ne!(raw & INT_TRANS_START, 0);
        let st = r(&mut i, INT_STATUS_REG);
        assert_eq!(st, raw & 0x5A8);
        assert_eq!(i.pending_sources(), 1u64 << SRC_I2C_EXT0);

        assert_ne!(r(&mut i, comd(0)) & CMD_DONE, 0, "RSTART done");
        for n in 1..6 {
            assert_eq!(r(&mut i, comd(n)) & CMD_DONE, 0, "command {n} not done");
        }
        let sr = r(&mut i, SR_REG);
        assert_eq!(sr & SR_RESP_REC, SR_RESP_REC, "received ACK level 1");
        assert_eq!(sr & SR_BUS_BUSY, 0, "STOP released the bus");
        assert_eq!(r(&mut i, CTR_REG) & CTR_TRANS_START, 0, "WT bit");

        // The ISR: i2c_ll_get_intr_mask then i2c_ll_clear_intr_mask.
        w(&mut i, INT_CLR_REG, st);
        assert_eq!(r(&mut i, INT_STATUS_REG), 0);
        assert_eq!(i.pending_sources(), 0);
        assert_eq!(r(&mut i, INT_CLR_REG), 0);
    }

    #[test]
    fn write_without_ack_check_then_read_completes_with_released_bus_bytes() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_RSTART, false, false, 0));
        w(&mut i, DATA_REG, 0x33);
        w(&mut i, comd(1), cmd(OP_WRITE, false, false, 1));
        w(&mut i, comd(2), cmd(OP_READ, false, false, 2));
        w(&mut i, comd(3), cmd(OP_STOP, false, false, 0));
        trans_start(&mut i);

        for n in 0..4 {
            assert_ne!(r(&mut i, comd(n)) & CMD_DONE, 0, "command {n} done");
        }
        let raw = r(&mut i, INT_RAW_REG);
        assert_eq!(raw & INT_NACK, 0);
        assert_ne!(raw & INT_TRANS_COMPLETE, 0);
        assert_ne!(raw & INT_BYTE_TRANS_DONE, 0);
        assert_eq!((r(&mut i, SR_REG) >> SR_RXFIFO_CNT_S) & 0x3F, 2);
        // i2c_ll_read_rxfifo: a read of fifo_data pops one byte.
        assert_eq!(r(&mut i, DATA_REG), 0xFF);
        assert_eq!(r(&mut i, DATA_REG), 0xFF);
        assert_eq!((r(&mut i, SR_REG) >> SR_RXFIFO_CNT_S) & 0x3F, 0);
        assert_eq!(r(&mut i, INT_RAW_REG) & INT_RXFIFO_UDF, 0);
        r(&mut i, DATA_REG);
        assert_ne!(r(&mut i, INT_RAW_REG) & INT_RXFIFO_UDF, 0);
    }

    #[test]
    fn end_command_raises_end_detect_and_holds_the_bus() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_RSTART, false, false, 0));
        w(&mut i, DATA_REG, 0x32);
        w(&mut i, comd(1), cmd(OP_WRITE, false, false, 1));
        w(&mut i, comd(2), cmd(OP_END, false, false, 0));
        trans_start(&mut i);
        assert_ne!(r(&mut i, INT_RAW_REG) & INT_END_DETECT, 0);
        assert_eq!(r(&mut i, INT_RAW_REG) & INT_TRANS_COMPLETE, 0);
        assert_ne!(r(&mut i, SR_REG) & SR_BUS_BUSY, 0);
        // i2c_ll_master_fsm_rst releases it.
        set_bits(&mut i, CTR_REG, CTR_FSM_RST);
        assert_eq!(r(&mut i, SR_REG) & SR_BUS_BUSY, 0);
    }

    #[test]
    fn trans_start_in_slave_mode_runs_nothing() {
        let mut i = I2c::new();
        w(&mut i, INT_ENA_REG, 0x5A8);
        w(&mut i, comd(0), cmd(OP_STOP, false, false, 0));
        set_bits(&mut i, CTR_REG, CTR_TRANS_START);
        assert_eq!(r(&mut i, comd(0)) & CMD_DONE, 0);
        assert_eq!(r(&mut i, INT_RAW_REG), INT_TXFIFO_WM, "reset value only");
    }

    #[test]
    fn rewriting_a_command_clears_its_done_bit_and_raw_is_write_one_to_clear() {
        let mut i = master_ready();
        w(&mut i, comd(0), cmd(OP_STOP, false, false, 0));
        trans_start(&mut i);
        assert_ne!(r(&mut i, comd(0)) & CMD_DONE, 0);
        w(&mut i, comd(0), cmd(OP_STOP, false, false, 0));
        assert_eq!(r(&mut i, comd(0)), cmd(OP_STOP, false, false, 0));
        // INT_RAW is R/SS/WTC (i2c_reg.h).
        w(&mut i, INT_RAW_REG, INT_TRANS_COMPLETE);
        assert_eq!(r(&mut i, INT_RAW_REG) & INT_TRANS_COMPLETE, 0);
    }

    #[test]
    fn tx_fifo_overflow_drops_the_byte_and_raises_txfifo_ovf() {
        let mut i = master_ready();
        for b in 0..FIFO_LEN as u32 {
            w(&mut i, DATA_REG, b);
        }
        assert_eq!(r(&mut i, INT_RAW_REG) & INT_TXFIFO_OVF, 0);
        w(&mut i, DATA_REG, 0xAA);
        assert_ne!(r(&mut i, INT_RAW_REG) & INT_TXFIFO_OVF, 0);
        assert_eq!((r(&mut i, SR_REG) >> SR_TXFIFO_CNT_S) & 0x3F, 32);
        assert_eq!(r(&mut i, TXFIFO_START_ADDR), 0);
    }
}

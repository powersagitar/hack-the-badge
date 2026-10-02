//! ESP32-C3 GDMA (`DR_REG_GDMA_BASE = 0x6003_F000`): three channels, each
//! with an RX ("in-link") and a TX ("out-link") half. Milestone 3 Task 10
//! models the **TX out-link** that feeds SPI2's DMA transmit, which is how
//! ESP-IDF's `spi_master` driver (and so `esp_lcd` + LVGL) pushes every byte
//! to the badge's ST7789 once the bus is initialized with DMA.
//!
//! ## Sources (ESP-IDF v5.5.3, fetched from `raw.githubusercontent.com/espressif/esp-idf/v5.5.3/...`)
//!
//! - `components/soc/esp32c3/register/soc/reg_base.h`: `DR_REG_GDMA_BASE
//!   0x6003f000`.
//! - `components/soc/esp32c3/register/soc/gdma_reg.h` (offsets, bit
//!   positions, access types, reset values) and `.../soc/gdma_struct.h`
//!   (the same layout as `gdma_dev_t`: `intr[3]` then `channel[3]`).
//! - `components/hal/esp32c3/include/hal/gdma_ll.h`: what firmware does with
//!   them — `gdma_ll_tx_set_desc_addr` (`out_link.addr = addr`, a bitfield
//!   read-modify-write), `gdma_ll_tx_start`/`_stop`/`_restart` (`out_link.
//!   start/stop/restart = 1`, separate read-modify-writes),
//!   `gdma_ll_tx_reset_channel` (`out_rst = 1` then `= 0`),
//!   `gdma_ll_tx_connect_to_periph` (`out_peri_sel.sel = periph_id`),
//!   `gdma_ll_tx_disconnect_from_periph` (`= GDMA_LL_INVALID_PERIPH_ID`,
//!   `0x3F`), `gdma_ll_tx_enable_interrupt`/`_clear_interrupt_status`
//!   (`intr[ch].ena`/`.clr`), `gdma_ll_tx_get_eof_desc_addr`
//!   (`out_eof_des_addr`), `gdma_ll_tx_is_desc_fsm_idle` (`out_link.park`).
//! - `components/soc/esp32c3/include/soc/gdma_channel.h`:
//!   `SOC_GDMA_TRIG_PERIPH_SPI2 (0)` — the `OUT_PERI_SEL` value for SPI2.
//! - `components/hal/include/hal/dma_types.h`: `dma_descriptor_t` — word 0
//!   `size[11:0] length[23:12] err_eof[28] suc_eof[30] owner[31]`, word 1
//!   buffer pointer, word 2 next pointer (`NULL` ends the link);
//!   `DMA_DESCRIPTOR_BUFFER_MAX_SIZE 4095`.
//! - `components/esp_driver_spi/src/gpspi/spi_common.c`
//!   (`spicommon_dma_desc_setup_link`: TX chunks of
//!   `DMA_DESCRIPTOR_BUFFER_MAX_SIZE_4B_ALIGNED` = 4092 bytes, `size ==
//!   length`, `owner = DMA`, last chunk `suc_eof = 1, next = NULL`) and
//!   `spi_master.c` (`s_spi_dma_prepare_data`: `gdma_reset`, then
//!   `spi_hal_hw_prepare_tx`, then `gdma_start`; `spi_new_trans` then kicks
//!   `SPI_USR`). The SPI driver never calls `gdma_apply_strategy` or
//!   registers GDMA callbacks: `OUT_AUTO_WRBACK`, `OUT_CHECK_OWNER` and the
//!   GDMA interrupt enables all stay 0 for it.
//!
//! **Corrections to the task brief** (the header wins): the brief listed
//! the out-link block as `OUT_CONF0 +0x60, OUT_CONF1 +0x64, OUT_INT_RAW
//! +0x68, ... OUT_LINK +0x80, OUT_EOF_DES_ADDR +0x88` inside a `0xC0`
//! channel stride, with interrupt bits `OUT_DONE 0, OUT_EOF 1, OUT_DSCR_ERR
//! 2, OUT_TOTAL_EOF 3`. On the ESP32-C3 the interrupt registers are not in
//! the channel block at all, and the bits are different:
//!
//! | register | offset (channel `n`) | access |
//! |---|---|---|
//! | `INT_RAW_CHn` / `INT_ST_CHn` / `INT_ENA_CHn` / `INT_CLR_CHn` | `0x00` / `0x04` / `0x08` / `0x0C` `+ n*0x10` | `R/WTC/SS` / `RO` / `R/W` / `WT` |
//! | `MISC_CONF` / `DATE` | `0x44` / `0x48` | `R/W` / `R/W` (default 33587792) |
//! | `IN_CONF0_CHn` .. `IN_PERI_SEL_CHn` | `0x70`..`0xA0` `+ n*0xC0` | (13 words) |
//! | `OUT_CONF0_CHn` | `0xD0 + n*0xC0` | `R/W`; `OUT_RST` bit 0, `OUT_AUTO_WRBACK` bit 2, `OUT_EOF_MODE` bit 3 |
//! | `OUT_CONF1_CHn` | `0xD4` | `R/W`; `OUT_CHECK_OWNER` bit 12 |
//! | `OUTFIFO_STATUS_CHn` | `0xD8` | `RO`; `OUTFIFO_EMPTY` bit 1 |
//! | `OUT_PUSH_CHn` | `0xDC` | `OUTFIFO_WDATA [8:0]` `R/W`, `OUTFIFO_PUSH` bit 9 `R/W/SC` |
//! | `OUT_LINK_CHn` | `0xE0` | `OUTLINK_ADDR [19:0]` `R/W`; `STOP` 20, `START` 21, `RESTART` 22 `R/W/SC`; `PARK` 23 `RO` |
//! | `OUT_STATE_CHn` | `0xE4` | `RO`; `OUTLINK_DSCR_ADDR [17:0]` |
//! | `OUT_EOF_DES_ADDR_CHn` / `OUT_EOF_BFR_DES_ADDR_CHn` | `0xE8` / `0xEC` | `RO` |
//! | `OUT_DSCR_CHn` / `_BF0` / `_BF1` | `0xF0` / `0xF4` / `0xF8` | `RO` |
//! | `OUT_PRI_CHn` / `OUT_PERI_SEL_CHn` | `0xFC` / `0x100` | `R/W`; `PERI_OUT_SEL` default 63 |
//!
//! Interrupt bits (`INT_*_CHn`, `gdma_reg.h` and `gdma_ll.h`'s
//! `GDMA_LL_EVENT_*`): `IN_DONE 0, IN_SUC_EOF 1, IN_ERR_EOF 2, OUT_DONE 3,
//! OUT_EOF 4, IN_DSCR_ERR 5, OUT_DSCR_ERR 6, IN_DSCR_EMPTY 7,
//! OUT_TOTAL_EOF 8, INFIFO_OVF 9, INFIFO_UDF 10, OUTFIFO_OVF 11,
//! OUTFIFO_UDF 12`. The brief's `OUT_LINK` bit positions (20/21/22) were
//! right. The firmware's own unmapped-access log before this task showed
//! exactly `0x6003_F0D0` (`OUT_CONF0_CH0`) and `0x6003_F0E0`
//! (`OUT_LINK_CH0`), matching the header.
//!
//! ## Model
//!
//! - **Interrupts**: `RAW` is set by the out-link walk (below); a software
//!   write of 1 to `RAW` clears that bit (`WTC`), as does a 1 written to
//!   `CLR` (`WT`, reads 0). `ST` = `RAW & ENA` (`RO`). [`Gdma::pending_sources`]
//!   asserts `ETS_DMA_CHn_INTR_SOURCE` (`crate::mem::soc::SRC_DMA_CH0..2`,
//!   44..46) while channel `n`'s `RAW & ENA` is non-zero: a level, like
//!   every other source here. Only bits 0..12 exist.
//! - **Out-link FSM** ([`OutLinkState`]): `START` latches the descriptor
//!   address and makes the channel active; `STOP` makes it inactive;
//!   `RESTART` makes it active and marks the next pull to re-read the
//!   `next` field of the last descriptor it finished ("restart a new
//!   outlink from the last address", `gdma_reg.h`). `OUT_RST` held at 1
//!   resets the FSM (`gdma_ll_tx_reset_channel` pulses it). `PARK` reads 1
//!   while the channel is inactive (`gdma_ll_tx_is_desc_fsm_idle`).
//! - **Descriptor address**: `OUTLINK_ADDR` holds only the low 20 bits
//!   ("the 20 least significant bits of the first transmit descriptor's
//!   address", `gdma_reg.h`). DMA-capable descriptors live in internal SRAM
//!   (`SOC_DRAM_LOW..SOC_DRAM_HIGH` = `0x3FC8_0000..0x3FCE_0000`, `soc.h`),
//!   whose addresses all share the top 12 bits `0x3FC`, so the full address
//!   is `0x3FC0_0000 | addr` ([`DESC_SRAM_BASE`]). That is this model's
//!   reconstruction, not a header fact: no header states the fixed upper
//!   bits.
//! - **Byte-split writes** (review focus #1): `FirmwareBus::write32` arrives
//!   as four ascending byte writes. `OUT_LINK`'s `STOP`/`START`/`RESTART`
//!   all live in byte 2 (bits 20..22), which also carries `ADDR[19:16]`; by
//!   the time byte 2 arrives, bytes 0 and 1 of the same word are already
//!   stored, so the trigger reads the complete new 20-bit address. The
//!   trigger bits are never stored (`SC`), so a later read-modify-write of
//!   the register cannot re-fire them: each fires exactly once per word
//!   write that carries it. `OUT_RST` lives in byte 0 of `OUT_CONF0`.
//! - **The walk itself** reads descriptors and buffers from RAM, which this
//!   module cannot see; it lives in `crate::mem::bus::FirmwareBus::gdma_pull`
//!   and drives this module through [`Gdma::out_link_state`]/
//!   [`Gdma::set_out_link_state`], [`Gdma::raise_out`] and
//!   [`Gdma::record_out_eof`]. See that function for the descriptor rules.
//! - **Everything is synchronous**: data moves at the instant SPI2's
//!   `SPI_USR` fires, so the L1 FIFO is always drained: `OUTFIFO_STATUS`
//!   reads `OUTFIFO_EMPTY` (bit 1) set and nothing else (the header's
//!   `default: 0` is a reset value before the FIFO is first reset);
//!   `OUT_PUSH`'s `OUTFIFO_PUSH` pulse has no effect (CPU-pushed FIFO data
//!   is not modeled; `OUTFIFO_WDATA` is stored).
//! - **In-link** (RX) registers are plain read/write storage only (the
//!   badge's SPI2 traffic is transmit-only); `IN_PERI_SEL` resets to 63.
//! - `OUT_DSCR`/`OUT_DSCR_BF0`/`OUT_DSCR_BF1` (the descriptor pre-fetch
//!   pipeline) read 0 and are reported as unmodeled by [`Gdma::handles`],
//!   so the bus logs any access to them. `OUT_STATE` reports
//!   `OUTLINK_DSCR_ADDR` as the low 18 bits of the next descriptor to
//!   fetch (or, at the end of a link, of the last one), its state fields 0.

use super::set_byte;
use crate::mem::soc::SRC_DMA_CH0;

pub const NUM_CHANNELS: usize = 3;

pub const INT_RAW_CH0_REG: u32 = 0x00;
pub const INT_ST_CH0_REG: u32 = 0x04;
pub const INT_ENA_CH0_REG: u32 = 0x08;
pub const INT_CLR_CH0_REG: u32 = 0x0C;
/// `INT_*_CH1_REG` = `INT_*_CH0_REG + 0x10`, `CH2` `+ 0x20`.
pub const INT_CH_STRIDE: u32 = 0x10;
const INT_END: u32 = INT_CH_STRIDE * NUM_CHANNELS as u32;
pub const MISC_CONF_REG: u32 = 0x44;
pub const DATE_REG: u32 = 0x48;
/// `GDMA_DATE` reset value (`gdma_reg.h`: `default: 33587792`).
const DATE_DEFAULT: u32 = 33_587_792;

/// Per-channel stride of the in/out register blocks.
pub const CH_STRIDE: u32 = 0xC0;
pub const IN_CONF0_CH0_REG: u32 = 0x70;
pub const IN_LINK_CH0_REG: u32 = 0x80;
pub const IN_PERI_SEL_CH0_REG: u32 = 0xA0;
/// Words in one in-link block (`IN_CONF0`..`IN_PERI_SEL`).
const IN_WORDS: usize = 13;
const IN_PERI_SEL_IDX: usize = ((IN_PERI_SEL_CH0_REG - IN_CONF0_CH0_REG) / 4) as usize;
pub const OUT_CONF0_CH0_REG: u32 = 0xD0;
pub const OUT_CONF1_CH0_REG: u32 = 0xD4;
pub const OUTFIFO_STATUS_CH0_REG: u32 = 0xD8;
pub const OUT_PUSH_CH0_REG: u32 = 0xDC;
pub const OUT_LINK_CH0_REG: u32 = 0xE0;
pub const OUT_STATE_CH0_REG: u32 = 0xE4;
pub const OUT_EOF_DES_ADDR_CH0_REG: u32 = 0xE8;
pub const OUT_EOF_BFR_DES_ADDR_CH0_REG: u32 = 0xEC;
pub const OUT_DSCR_CH0_REG: u32 = 0xF0;
pub const OUT_DSCR_BF0_CH0_REG: u32 = 0xF4;
pub const OUT_DSCR_BF1_CH0_REG: u32 = 0xF8;
pub const OUT_PRI_CH0_REG: u32 = 0xFC;
pub const OUT_PERI_SEL_CH0_REG: u32 = 0x100;

/// `GDMA_OUT_DONE_CHn_INT_*`, bit 3.
pub const OUT_DONE: u32 = 1 << 3;
/// `GDMA_OUT_EOF_CHn_INT_*`, bit 4.
pub const OUT_EOF: u32 = 1 << 4;
/// `GDMA_OUT_DSCR_ERR_CHn_INT_*`, bit 6.
pub const OUT_DSCR_ERR: u32 = 1 << 6;
/// `GDMA_OUT_TOTAL_EOF_CHn_INT_*`, bit 8.
pub const OUT_TOTAL_EOF: u32 = 1 << 8;
/// The 13 interrupt bits that exist (`[12:0]`).
const INT_MASK: u32 = 0x1FFF;

/// `GDMA_OUT_RST_CHn`, `OUT_CONF0` bit 0.
pub const OUT_RST: u32 = 1 << 0;
/// `GDMA_OUT_AUTO_WRBACK_CHn`, `OUT_CONF0` bit 2.
pub const OUT_AUTO_WRBACK: u32 = 1 << 2;
/// `GDMA_OUT_CHECK_OWNER_CHn`, `OUT_CONF1` bit 12.
pub const OUT_CHECK_OWNER: u32 = 1 << 12;
/// `GDMA_OUTFIFO_EMPTY_CHn`, `OUTFIFO_STATUS` bit 1.
const OUTFIFO_EMPTY: u32 = 1 << 1;
/// `GDMA_OUTFIFO_WDATA_CHn`, `OUT_PUSH` bits `[8:0]`.
const OUTFIFO_WDATA_MASK: u32 = 0x1FF;
pub const OUTLINK_ADDR_MASK: u32 = 0x000F_FFFF;
pub const OUTLINK_STOP: u32 = 1 << 20;
pub const OUTLINK_START: u32 = 1 << 21;
pub const OUTLINK_RESTART: u32 = 1 << 22;
pub const OUTLINK_PARK: u32 = 1 << 23;
/// `GDMA_OUTLINK_DSCR_ADDR_CHn`, `OUT_STATE` bits `[17:0]`.
const OUTLINK_DSCR_ADDR_MASK: u32 = 0x3_FFFF;
/// `GDMA_PERI_{IN,OUT}_SEL_CHn`, bits `[5:0]`.
const PERI_SEL_MASK: u32 = 0x3F;
/// `SOC_GDMA_TRIG_PERIPH_SPI2` (`soc/gdma_channel.h`).
pub const PERI_SEL_SPI2: u32 = 0;
/// `GDMA_LL_INVALID_PERIPH_ID` (`gdma_ll.h`), also the reset value.
pub const PERI_SEL_INVALID: u32 = 0x3F;
/// The fixed upper bits of every internal-SRAM data address (see the module
/// doc's "Descriptor address").
pub const DESC_SRAM_BASE: u32 = 0x3FC0_0000;

/// `dma_descriptor_t.dw0.owner` (bit 31): 1 = `DMA_DESCRIPTOR_BUFFER_OWNER_DMA`.
pub const DESC_OWNER: u32 = 1 << 31;
/// `dma_descriptor_t.dw0.suc_eof` (bit 30): the last descriptor of a link.
pub const DESC_SUC_EOF: u32 = 1 << 30;

/// The TX out-link FSM of one channel, as the bus-level walk
/// (`FirmwareBus::gdma_pull`) reads and writes it. All addresses are full
/// 32-bit addresses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutLinkState {
    /// Started and not stopped, reset, or run off the end of its link.
    pub active: bool,
    /// The next descriptor to fetch (`0` = none).
    pub cursor: u32,
    /// Bytes of `cursor`'s buffer already sent (an SPI transaction may end
    /// mid-descriptor).
    pub offset: u32,
    /// The last descriptor fully sent (`0` = none since reset).
    pub last_desc: u32,
    /// Set by `RESTART`: the next pull first re-reads `last_desc`'s `next`.
    pub restart_pending: bool,
}

#[derive(Clone, Copy)]
struct OutChannel {
    conf0: u32,
    conf1: u32,
    push_wdata: u32,
    link_addr: u32,
    eof_des_addr: u32,
    eof_bfr_des_addr: u32,
    pri: u32,
    peri_sel: u32,
    fsm: OutLinkState,
}

impl Default for OutChannel {
    fn default() -> Self {
        Self {
            conf0: 0,
            conf1: 0,
            push_wdata: 0,
            link_addr: 0,
            eof_des_addr: 0,
            eof_bfr_des_addr: 0,
            pri: 0,
            peri_sel: PERI_SEL_INVALID,
            fsm: OutLinkState::default(),
        }
    }
}

/// Which register an offset names, after per-channel decoding.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reg {
    IntRaw(usize),
    IntSt(usize),
    IntEna(usize),
    IntClr(usize),
    MiscConf,
    Date,
    In(usize, usize),
    OutConf0(usize),
    OutConf1(usize),
    OutfifoStatus(usize),
    OutPush(usize),
    OutLink(usize),
    OutState(usize),
    OutEofDesAddr(usize),
    OutEofBfrDesAddr(usize),
    /// `OUT_DSCR`/`_BF0`/`_BF1`: named in the header, not modeled.
    OutDscrUnmodeled,
    OutPri(usize),
    OutPeriSel(usize),
}

fn decode(offset: u32) -> Option<Reg> {
    let word = offset & !0b11;
    if word < INT_END {
        let ch = (word / INT_CH_STRIDE) as usize;
        return Some(match word % INT_CH_STRIDE {
            INT_RAW_CH0_REG => Reg::IntRaw(ch),
            INT_ST_CH0_REG => Reg::IntSt(ch),
            INT_ENA_CH0_REG => Reg::IntEna(ch),
            _ => Reg::IntClr(ch),
        });
    }
    match word {
        MISC_CONF_REG => return Some(Reg::MiscConf),
        DATE_REG => return Some(Reg::Date),
        _ => {}
    }
    if word < IN_CONF0_CH0_REG {
        return None;
    }
    let rel = word - IN_CONF0_CH0_REG;
    let ch = (rel / CH_STRIDE) as usize;
    if ch >= NUM_CHANNELS {
        return None;
    }
    let in_ch = rel % CH_STRIDE;
    if (in_ch as usize) < IN_WORDS * 4 {
        return Some(Reg::In(ch, (in_ch / 4) as usize));
    }
    let out_off = in_ch + IN_CONF0_CH0_REG; // the channel-0 offset
    Some(match out_off {
        OUT_CONF0_CH0_REG => Reg::OutConf0(ch),
        OUT_CONF1_CH0_REG => Reg::OutConf1(ch),
        OUTFIFO_STATUS_CH0_REG => Reg::OutfifoStatus(ch),
        OUT_PUSH_CH0_REG => Reg::OutPush(ch),
        OUT_LINK_CH0_REG => Reg::OutLink(ch),
        OUT_STATE_CH0_REG => Reg::OutState(ch),
        OUT_EOF_DES_ADDR_CH0_REG => Reg::OutEofDesAddr(ch),
        OUT_EOF_BFR_DES_ADDR_CH0_REG => Reg::OutEofBfrDesAddr(ch),
        OUT_DSCR_CH0_REG | OUT_DSCR_BF0_CH0_REG | OUT_DSCR_BF1_CH0_REG => Reg::OutDscrUnmodeled,
        OUT_PRI_CH0_REG => Reg::OutPri(ch),
        OUT_PERI_SEL_CH0_REG => Reg::OutPeriSel(ch),
        _ => return None,
    })
}

/// The GDMA peripheral. See the module doc.
pub struct Gdma {
    int_raw: [u32; NUM_CHANNELS],
    int_ena: [u32; NUM_CHANNELS],
    misc_conf: u32,
    date: u32,
    in_regs: [[u32; IN_WORDS]; NUM_CHANNELS],
    out: [OutChannel; NUM_CHANNELS],
}

impl Default for Gdma {
    fn default() -> Self {
        let mut in_block = [0u32; IN_WORDS];
        in_block[IN_PERI_SEL_IDX] = PERI_SEL_INVALID;
        Self {
            int_raw: [0; NUM_CHANNELS],
            int_ena: [0; NUM_CHANNELS],
            misc_conf: 0,
            date: DATE_DEFAULT,
            in_regs: [in_block; NUM_CHANNELS],
            out: [OutChannel::default(); NUM_CHANNELS],
        }
    }
}

impl Gdma {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` iff `offset` is a register this module models. The header's
    /// reserved gaps and the unmodeled `OUT_DSCR*` pre-fetch registers are
    /// `false`, so `FirmwareBus` logs accesses to them.
    pub fn handles(offset: u32) -> bool {
        !matches!(decode(offset), None | Some(Reg::OutDscrUnmodeled))
    }

    fn read_word(&self, reg: Reg) -> u32 {
        match reg {
            Reg::IntRaw(ch) => self.int_raw[ch],
            Reg::IntSt(ch) => self.int_raw[ch] & self.int_ena[ch],
            Reg::IntEna(ch) => self.int_ena[ch],
            Reg::IntClr(_) => 0,
            Reg::MiscConf => self.misc_conf,
            Reg::Date => self.date,
            Reg::In(ch, idx) => self.in_regs[ch][idx],
            Reg::OutConf0(ch) => self.out[ch].conf0,
            Reg::OutConf1(ch) => self.out[ch].conf1,
            Reg::OutfifoStatus(_) => OUTFIFO_EMPTY,
            Reg::OutPush(ch) => self.out[ch].push_wdata,
            Reg::OutLink(ch) => {
                let o = &self.out[ch];
                let park = if o.fsm.active { 0 } else { OUTLINK_PARK };
                o.link_addr | park
            }
            Reg::OutState(ch) => {
                let f = &self.out[ch].fsm;
                let at = if f.cursor != 0 { f.cursor } else { f.last_desc };
                at & OUTLINK_DSCR_ADDR_MASK
            }
            Reg::OutEofDesAddr(ch) => self.out[ch].eof_des_addr,
            Reg::OutEofBfrDesAddr(ch) => self.out[ch].eof_bfr_des_addr,
            Reg::OutDscrUnmodeled => 0,
            Reg::OutPri(ch) => self.out[ch].pri,
            Reg::OutPeriSel(ch) => self.out[ch].peri_sel,
        }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        match decode(offset) {
            Some(reg) => self.read_word(reg).to_le_bytes()[(offset & 0b11) as usize],
            None => 0,
        }
    }

    /// Stores one byte of a register write and fires any trigger that byte
    /// carries (see the module doc's "Byte-split writes").
    pub fn write_byte(&mut self, offset: u32, val: u8) {
        let Some(reg) = decode(offset) else {
            return;
        };
        let idx = offset & 0b11;
        let bits = (val as u32) << (idx * 8);
        match reg {
            // R/WTC/SS: software can only clear, by writing 1.
            Reg::IntRaw(ch) | Reg::IntClr(ch) => self.int_raw[ch] &= !bits,
            Reg::IntSt(_) => {}
            Reg::IntEna(ch) => {
                set_byte(&mut self.int_ena[ch], idx, val);
                self.int_ena[ch] &= INT_MASK;
            }
            Reg::MiscConf => set_byte(&mut self.misc_conf, idx, val),
            Reg::Date => set_byte(&mut self.date, idx, val),
            Reg::In(ch, i) => set_byte(&mut self.in_regs[ch][i], idx, val),
            Reg::OutConf0(ch) => {
                set_byte(&mut self.out[ch].conf0, idx, val);
                if idx == 0 && val as u32 & OUT_RST != 0 {
                    self.out[ch].fsm = OutLinkState::default();
                }
            }
            Reg::OutConf1(ch) => set_byte(&mut self.out[ch].conf1, idx, val),
            Reg::OutfifoStatus(_) => {}
            Reg::OutPush(ch) => {
                // OUTFIFO_PUSH (bit 9) is an SC pulse with no modeled effect.
                set_byte(&mut self.out[ch].push_wdata, idx, val);
                self.out[ch].push_wdata &= OUTFIFO_WDATA_MASK;
            }
            Reg::OutLink(ch) => self.write_out_link_byte(ch, idx, bits),
            Reg::OutState(_) | Reg::OutEofDesAddr(_) | Reg::OutEofBfrDesAddr(_) => {}
            Reg::OutDscrUnmodeled => {}
            Reg::OutPri(ch) => set_byte(&mut self.out[ch].pri, idx, val),
            Reg::OutPeriSel(ch) => {
                set_byte(&mut self.out[ch].peri_sel, idx, val);
                self.out[ch].peri_sel &= PERI_SEL_MASK;
            }
        }
    }

    /// `OUT_LINK`: only `OUTLINK_ADDR` is stored; `STOP`/`START`/`RESTART`
    /// (byte 2) fire and self-clear; `PARK` is read-only.
    fn write_out_link_byte(&mut self, ch: usize, idx: u32, bits: u32) {
        let byte_mask = 0xFFu32 << (idx * 8);
        let o = &mut self.out[ch];
        o.link_addr = (o.link_addr & !(byte_mask & OUTLINK_ADDR_MASK)) | (bits & OUTLINK_ADDR_MASK);
        if bits & OUTLINK_STOP != 0 {
            o.fsm.active = false;
        }
        if bits & OUTLINK_START != 0 {
            o.fsm = OutLinkState {
                active: true,
                cursor: DESC_SRAM_BASE | o.link_addr,
                offset: 0,
                last_desc: o.fsm.last_desc,
                restart_pending: false,
            };
        }
        if bits & OUTLINK_RESTART != 0 {
            o.fsm.active = true;
            o.fsm.restart_pending = true;
        }
    }

    /// The TX channel connected to SPI2 (`OUT_PERI_SEL == 0`), lowest
    /// number first if (mis)configured on several.
    pub fn channel_for_spi2(&self) -> Option<usize> {
        self.out.iter().position(|o| o.peri_sel == PERI_SEL_SPI2)
    }

    /// The interrupt sources GDMA asserts right now: bit `SRC_DMA_CH0 + n`
    /// iff channel `n`'s `RAW & ENA` is non-zero. Level, not latched.
    pub fn pending_sources(&self) -> u64 {
        (0..NUM_CHANNELS)
            .filter(|&ch| self.int_raw[ch] & self.int_ena[ch] != 0)
            .fold(0, |p, ch| p | 1u64 << (SRC_DMA_CH0 as usize + ch))
    }

    pub fn out_link_state(&self, ch: usize) -> OutLinkState {
        self.out[ch].fsm
    }

    pub fn set_out_link_state(&mut self, ch: usize, s: OutLinkState) {
        self.out[ch].fsm = s;
    }

    /// `OUT_AUTO_WRBACK`: clear each descriptor's owner bit once sent.
    pub fn out_auto_wrback(&self, ch: usize) -> bool {
        self.out[ch].conf0 & OUT_AUTO_WRBACK != 0
    }

    /// `OUT_CHECK_OWNER`: a CPU-owned descriptor is an error.
    pub fn out_check_owner(&self, ch: usize) -> bool {
        self.out[ch].conf1 & OUT_CHECK_OWNER != 0
    }

    /// Sets `INT_RAW_CHn` bits (hardware `SS`).
    pub fn raise_out(&mut self, ch: usize, bits: u32) {
        self.int_raw[ch] |= bits & INT_MASK;
    }

    /// Latches `OUT_EOF_DES_ADDR` (the `suc_eof` descriptor) and
    /// `OUT_EOF_BFR_DES_ADDR` (the last descriptor the channel sent before it,
    /// `0` if none since `OUT_RST`).
    pub fn record_out_eof(&mut self, ch: usize, eof_desc: u32, before: u32) {
        self.out[ch].eof_des_addr = eof_desc;
        self.out[ch].eof_bfr_des_addr = before;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::soc::{SRC_DMA_CH0, SRC_DMA_CH1, SRC_DMA_CH2};

    /// Review focus #1: a whole-word register write arrives as four
    /// ascending byte writes, exactly like `FirmwareBus::write32`.
    fn write_word(g: &mut Gdma, off: u32, val: u32) {
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            g.write_byte(off + i as u32, *b);
        }
    }

    fn read_word(g: &mut Gdma, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for (i, x) in b.iter_mut().enumerate() {
            *x = g.read_byte(off + i as u32);
        }
        u32::from_le_bytes(b)
    }

    fn out_reg(base: u32, ch: usize) -> u32 {
        base + ch as u32 * CH_STRIDE
    }

    fn int_reg(base: u32, ch: usize) -> u32 {
        base + ch as u32 * INT_CH_STRIDE
    }

    #[test]
    fn plain_storage_registers_read_back_and_reset_values_match_the_header() {
        let mut g = Gdma::new();
        // gdma_reg.h defaults: PERI_{IN,OUT}_SEL = 63, DATE = 33587792.
        for ch in 0..NUM_CHANNELS {
            assert_eq!(read_word(&mut g, out_reg(OUT_PERI_SEL_CH0_REG, ch)), 0x3F);
            assert_eq!(read_word(&mut g, out_reg(IN_PERI_SEL_CH0_REG, ch)), 0x3F);
        }
        assert_eq!(read_word(&mut g, DATE_REG), 33_587_792);

        write_word(
            &mut g,
            out_reg(OUT_CONF0_CH0_REG, 1),
            OUT_AUTO_WRBACK | 0x30,
        );
        write_word(&mut g, out_reg(OUT_CONF1_CH0_REG, 1), OUT_CHECK_OWNER);
        write_word(&mut g, out_reg(OUT_PRI_CH0_REG, 2), 5);
        write_word(&mut g, out_reg(IN_CONF0_CH0_REG, 0), 0x1C);
        write_word(&mut g, MISC_CONF_REG, 0b1000);
        assert_eq!(
            read_word(&mut g, out_reg(OUT_CONF0_CH0_REG, 1)),
            OUT_AUTO_WRBACK | 0x30
        );
        assert_eq!(
            read_word(&mut g, out_reg(OUT_CONF1_CH0_REG, 1)),
            OUT_CHECK_OWNER
        );
        assert_eq!(read_word(&mut g, out_reg(OUT_PRI_CH0_REG, 2)), 5);
        assert_eq!(read_word(&mut g, out_reg(IN_CONF0_CH0_REG, 0)), 0x1C);
        assert_eq!(read_word(&mut g, MISC_CONF_REG), 0b1000);
        assert!(g.out_auto_wrback(1));
        assert!(g.out_check_owner(1));
        assert!(!g.out_auto_wrback(0));
    }

    /// `gdma_ll_tx_set_desc_addr` then `gdma_ll_tx_start`, each a whole-word
    /// read-modify-write of `OUT_LINK`: START latches the 20-bit address
    /// (already applied by the same word's lower bytes) as a full SRAM
    /// address, activates the channel exactly once, and self-clears.
    #[test]
    fn out_link_start_latches_the_descriptor_address_once_and_self_clears() {
        let mut g = Gdma::new();
        let reg = out_reg(OUT_LINK_CH0_REG, 1);
        write_word(&mut g, reg, 0xC_D530);
        assert!(!g.out_link_state(1).active);
        assert_eq!(read_word(&mut g, reg) & OUTLINK_ADDR_MASK, 0xC_D530);
        assert_ne!(
            read_word(&mut g, reg) & OUTLINK_PARK,
            0,
            "idle: PARK reads 1"
        );

        let word = read_word(&mut g, reg) | OUTLINK_START;
        write_word(&mut g, reg, word);
        let s = g.out_link_state(1);
        assert!(s.active);
        assert_eq!(s.cursor, 0x3FCC_D530);
        assert_eq!(s.offset, 0);
        let back = read_word(&mut g, reg);
        assert_eq!(back & OUTLINK_START, 0, "START is R/W/SC");
        assert_eq!(back & OUTLINK_PARK, 0, "working: PARK reads 0");
        assert_eq!(back & OUTLINK_ADDR_MASK, 0xC_D530);
        // Other channels are untouched.
        assert!(!g.out_link_state(0).active);
        assert!(!g.out_link_state(2).active);
    }

    #[test]
    fn start_in_the_same_word_as_a_new_address_uses_that_address() {
        let mut g = Gdma::new();
        let reg = out_reg(OUT_LINK_CH0_REG, 0);
        write_word(&mut g, reg, 0x1_0000);
        write_word(&mut g, reg, OUTLINK_START | 0xA_BCDC);
        assert_eq!(g.out_link_state(0).cursor, 0x3FCA_BCDC);
    }

    #[test]
    fn out_link_start_fires_once_per_word_write() {
        let mut g = Gdma::new();
        let reg = out_reg(OUT_LINK_CH0_REG, 0);
        write_word(&mut g, reg, OUTLINK_START | 0xC_0000);
        // Simulate partial consumption, then a word write without START
        // (e.g. a later addr-only RMW): must not re-latch.
        let mut s = g.out_link_state(0);
        s.offset = 7;
        g.set_out_link_state(0, s);
        write_word(&mut g, reg, 0xC_0000);
        assert_eq!(g.out_link_state(0).offset, 7);
    }

    #[test]
    fn out_link_stop_deactivates_and_restart_reactivates_from_the_last_address() {
        let mut g = Gdma::new();
        let reg = out_reg(OUT_LINK_CH0_REG, 2);
        write_word(&mut g, reg, OUTLINK_START | 0xC_0100);
        write_word(&mut g, reg, OUTLINK_STOP | 0xC_0100);
        let s = g.out_link_state(2);
        assert!(!s.active);
        assert_eq!(read_word(&mut g, reg) & OUTLINK_STOP, 0, "STOP is R/W/SC");

        write_word(&mut g, reg, OUTLINK_RESTART | 0xC_0100);
        let s = g.out_link_state(2);
        assert!(s.active);
        assert!(s.restart_pending);
        assert_eq!(
            read_word(&mut g, reg) & OUTLINK_RESTART,
            0,
            "RESTART is R/W/SC"
        );
    }

    /// `gdma_ll_tx_reset_channel`: `out_rst = 1` then `out_rst = 0`.
    #[test]
    fn out_rst_resets_the_out_link_fsm() {
        let mut g = Gdma::new();
        write_word(
            &mut g,
            out_reg(OUT_LINK_CH0_REG, 0),
            OUTLINK_START | 0xC_0000,
        );
        let mut s = g.out_link_state(0);
        s.offset = 4;
        s.last_desc = 0x3FCC_0000;
        g.set_out_link_state(0, s);
        let conf0 = out_reg(OUT_CONF0_CH0_REG, 0);
        write_word(&mut g, conf0, OUT_RST);
        write_word(&mut g, conf0, 0);
        assert_eq!(g.out_link_state(0), OutLinkState::default());
    }

    #[test]
    fn channel_for_spi2_honors_out_peri_sel() {
        let mut g = Gdma::new();
        assert_eq!(g.channel_for_spi2(), None, "reset value 63 is invalid");
        // gdma_ll_tx_connect_to_periph(dev, 1, ..., SOC_GDMA_TRIG_PERIPH_SPI2)
        write_word(&mut g, out_reg(OUT_PERI_SEL_CH0_REG, 1), PERI_SEL_SPI2);
        assert_eq!(g.channel_for_spi2(), Some(1));
        // An RX channel connected to SPI2 is not a TX source.
        write_word(&mut g, out_reg(IN_PERI_SEL_CH0_REG, 0), PERI_SEL_SPI2);
        assert_eq!(g.channel_for_spi2(), Some(1));
        // gdma_ll_tx_disconnect_from_periph writes GDMA_LL_INVALID_PERIPH_ID.
        write_word(&mut g, out_reg(OUT_PERI_SEL_CH0_REG, 1), PERI_SEL_INVALID);
        assert_eq!(g.channel_for_spi2(), None);
        // UHCI0 (2) is not SPI2.
        write_word(&mut g, out_reg(OUT_PERI_SEL_CH0_REG, 2), 2);
        assert_eq!(g.channel_for_spi2(), None);
    }

    #[test]
    fn int_raw_ena_st_clr_semantics() {
        let mut g = Gdma::new();
        g.raise_out(1, OUT_EOF | OUT_DONE);
        assert_eq!(
            read_word(&mut g, int_reg(INT_RAW_CH0_REG, 1)),
            OUT_EOF | OUT_DONE
        );
        assert_eq!(
            read_word(&mut g, int_reg(INT_ST_CH0_REG, 1)),
            0,
            "ST = RAW & ENA"
        );
        assert_eq!(read_word(&mut g, int_reg(INT_RAW_CH0_REG, 0)), 0);

        write_word(&mut g, int_reg(INT_ENA_CH0_REG, 1), OUT_EOF);
        assert_eq!(read_word(&mut g, int_reg(INT_ENA_CH0_REG, 1)), OUT_EOF);
        assert_eq!(read_word(&mut g, int_reg(INT_ST_CH0_REG, 1)), OUT_EOF);

        // CLR is WT: each written 1 clears that RAW bit; reads 0.
        write_word(&mut g, int_reg(INT_CLR_CH0_REG, 1), OUT_EOF);
        assert_eq!(read_word(&mut g, int_reg(INT_RAW_CH0_REG, 1)), OUT_DONE);
        assert_eq!(read_word(&mut g, int_reg(INT_CLR_CH0_REG, 1)), 0);

        // RAW is R/WTC/SS: a written 1 clears too, a written 0 is a no-op.
        g.raise_out(1, OUT_TOTAL_EOF);
        write_word(&mut g, int_reg(INT_RAW_CH0_REG, 1), OUT_TOTAL_EOF);
        assert_eq!(read_word(&mut g, int_reg(INT_RAW_CH0_REG, 1)), OUT_DONE);
        write_word(&mut g, int_reg(INT_RAW_CH0_REG, 1), 0);
        assert_eq!(read_word(&mut g, int_reg(INT_RAW_CH0_REG, 1)), OUT_DONE);

        // ST is RO.
        write_word(&mut g, int_reg(INT_ST_CH0_REG, 1), 0xFFFF_FFFF);
        assert_eq!(read_word(&mut g, int_reg(INT_RAW_CH0_REG, 1)), OUT_DONE);
    }

    #[test]
    fn pending_sources_are_raw_and_ena_levels_on_bits_44_to_46() {
        let mut g = Gdma::new();
        assert_eq!(g.pending_sources(), 0);
        for ch in 0..NUM_CHANNELS {
            write_word(&mut g, int_reg(INT_ENA_CH0_REG, ch), OUT_TOTAL_EOF);
        }
        g.raise_out(0, OUT_TOTAL_EOF);
        g.raise_out(2, OUT_TOTAL_EOF);
        g.raise_out(1, OUT_DONE); // not enabled
        assert_eq!(
            g.pending_sources(),
            (1u64 << SRC_DMA_CH0) | (1u64 << SRC_DMA_CH2)
        );
        write_word(&mut g, int_reg(INT_ENA_CH0_REG, 1), OUT_DONE);
        assert_ne!(g.pending_sources() & (1u64 << SRC_DMA_CH1), 0);
        write_word(&mut g, int_reg(INT_CLR_CH0_REG, 0), OUT_TOTAL_EOF);
        write_word(&mut g, int_reg(INT_ENA_CH0_REG, 2), 0);
        assert_eq!(g.pending_sources(), 1u64 << SRC_DMA_CH1);
    }

    #[test]
    fn eof_descriptor_addresses_and_out_state_read_back() {
        let mut g = Gdma::new();
        g.record_out_eof(0, 0x3FCC_0018, 0x3FCC_000C);
        assert_eq!(read_word(&mut g, OUT_EOF_DES_ADDR_CH0_REG), 0x3FCC_0018);
        assert_eq!(read_word(&mut g, OUT_EOF_BFR_DES_ADDR_CH0_REG), 0x3FCC_000C);
        // Both are RO.
        write_word(&mut g, OUT_EOF_DES_ADDR_CH0_REG, 0);
        assert_eq!(read_word(&mut g, OUT_EOF_DES_ADDR_CH0_REG), 0x3FCC_0018);

        write_word(&mut g, OUT_LINK_CH0_REG, OUTLINK_START | 0xC_D534);
        assert_eq!(read_word(&mut g, OUT_STATE_CH0_REG), 0x3FCC_D534 & 0x3_FFFF);
    }

    #[test]
    fn handles_names_the_header_registers_and_not_the_gaps() {
        assert!(Gdma::handles(INT_RAW_CH0_REG));
        assert!(Gdma::handles(int_reg(INT_CLR_CH0_REG, 2) + 3));
        assert!(!Gdma::handles(0x30), "reserved_30");
        assert!(Gdma::handles(MISC_CONF_REG));
        assert!(Gdma::handles(DATE_REG));
        assert!(Gdma::handles(out_reg(IN_PERI_SEL_CH0_REG, 2)));
        assert!(Gdma::handles(out_reg(OUT_LINK_CH0_REG, 1)));
        assert!(Gdma::handles(out_reg(OUT_PERI_SEL_CH0_REG, 2)));
        assert!(!Gdma::handles(0xA4), "reserved_a4");
        assert!(!Gdma::handles(0x104), "reserved_104");
        // Named, but their pre-read state is not modeled: logged.
        assert!(!Gdma::handles(OUT_DSCR_CH0_REG));
        assert!(!Gdma::handles(OUT_DSCR_BF1_CH0_REG));
        assert!(!Gdma::handles(0x300));
    }

    #[test]
    fn unmodeled_offsets_read_zero_and_drop_writes() {
        let mut g = Gdma::new();
        write_word(&mut g, 0x30, 0xFFFF_FFFF);
        assert_eq!(read_word(&mut g, 0x30), 0);
        write_word(&mut g, 0xFFC, 0xFFFF_FFFF);
        assert_eq!(read_word(&mut g, 0xFFC), 0);
    }
}

//! ESP32-C3 SPI2 (GPSPI2) peripheral (`DR_REG_SPI2_BASE = 0x6002_4000`) plus
//! an ST7789 LCD command/pixel-stream interpreter that reconstructs a
//! framebuffer from what real firmware writes to drive the badge's display.
//!
//! Register layout confirmed against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/spi_reg.h` (fetched directly via
//! `gh api` this task, not guessed) and the base address against that same
//! version's `soc/reg_base.h`. **Correction to the task brief's own
//! citation**: the brief describes `SPI_W0_REG..SPI_W15_REG` as "`+0x98` to
//! `+0xD4`, each `+0x8` apart" — the range endpoints are right but the
//! stride is wrong. The fetched header shows `SPI_W0_REG(i) = base+0x98`,
//! `SPI_W1_REG(i) = base+0x9C`, `SPI_W2_REG(i) = base+0xA0`, ...,
//! `SPI_W15_REG(i) = base+0xD4` — a `+0x4` stride (`0x98 + 15*0x4 == 0xD4`,
//! whereas `+0x8` would overrun to `0x98 + 15*0x8 == 0x110`). This module
//! uses the header-verified `+0x4` stride.
//!
//! Milestone 3 Task 9 added `SPI_UPDATE`, `SPI_DMA_CONF_REG` and the
//! `SPI_DMA_INT_*` registers. Their offsets, bit positions and access types
//! (`WT`, `R/W/SC`, `R/W/WTC/SS`, `RO`) come from the same v5.5.3
//! `spi_reg.h` (fetched from `raw.githubusercontent.com/espressif/esp-idf/
//! v5.5.3/...`; every offset the plan marked "verify" matched), and the
//! behavior firmware relies on from `components/hal/esp32c3/include/hal/
//! spi_ll.h` (`spi_ll_apply_config`, `spi_ll_user_start`,
//! `spi_ll_usr_is_done`, `spi_ll_dma_tx_enable`, `spi_ll_clear_int_stat`,
//! `spi_ll_set_int_stat`); line numbers are cited per register below.
//!
//! ## Scope (per the task brief)
//!
//! Real firmware driving an ST7789 via ESP-IDF's standard LCD panel-IO
//! driver does *not* use SPI2's own command/address/dummy/MISO transaction
//! phases (`SPI_USR_COMMAND`/`_ADDR`/`_DUMMY`/`_MISO` in `SPI_USER_REG`) —
//! the command-vs-data (D/C) distinction is carried on a plain GPIO line
//! (GPIO0) toggled by software between transactions, not by SPI protocol
//! phases. So this module only models plain `SPI_USR_MOSI`-style byte
//! transfers; `SPI_USER_REG`'s other phase-enable bits are accepted and
//! stored (real read/write storage, per the brief) but otherwise inert —
//! they have no effect on transaction processing.
//!
//! **CS (chip-select, GPIO2) is not modeled at all** — every
//! `SPI_USR`-triggered transaction is treated as atomically complete the
//! instant it's triggered, mirroring every other peripheral this project has
//! built so far (synchronous, non-DMA, no propagation delay). This is a
//! documented scope choice per the brief, not an oversight.
//!
//! ## Modeled registers
//!
//! - `SPI_CMD_REG` (`+0x00`):
//!   - Bit 24 (`SPI_USR`, `R/W/SC`) starts a transaction
//!     (`spi_ll_user_start`, `spi_ll.h:287-290`). A write that sets it
//!     (checked byte-granular — only byte index 3, which is where bit 24
//!     lives — same "inspect only the byte containing the trigger bit"
//!     technique `systimer::SysTimer`'s `UNIT0_OP_REG` already uses, so a
//!     `sw`'s four constituent byte-writes can't spuriously double-trigger
//!     on a partially-reconstructed word) fires [`Spi::write_byte`]'s
//!     `bool` return, telling the caller (`FirmwareBus`) a transaction
//!     needs processing. `SPI_USR` stays set until [`Spi::finish_transaction`]
//!     (the `SC` half) clears it, which [`Spi::process_transaction`] calls
//!     before returning; the caller processes synchronously (see "CS
//!     handling" above — "atomically complete"), so polling firmware sees
//!     it done right away.
//!   - Bit 23 (`SPI_UPDATE`, `WT`) asks the hardware to sync the APB-side
//!     configuration into the SPI clock domain;
//!     `spi_ll_apply_config` (`spi_ll.h:264-267`) sets it and spins
//!     `while (hw->cmd.update);`. There is no second clock domain here, so
//!     the sync is instant: the bit is never stored and always reads 0.
//!     It needs no other side effect, so it fires none.
//!   - Every other bit is real read/write storage but otherwise inert.
//! - `SPI_USER_REG` (`+0x10`): real read/write storage, otherwise inert
//!   (see "Scope" above).
//! - `SPI_MS_DLEN_REG` (`+0x1C`): `SPI_MS_DATA_BITLEN` (bits `[17:0]`) is
//!   real read/write storage, consulted at trigger time to compute the
//!   transaction's byte count: `bit_count = (field + 1)`, standard ESP32
//!   off-by-one register convention, then `byte_count = ceil(bit_count /
//!   8)`. The brief says real transfers here are always byte-aligned; this
//!   module rounds up (rather than truncating) anyway, defensively, so a
//!   mis-programmed odd bit count doesn't silently drop a trailing byte.
//!   The result is also capped at 64 (`SPI_W0..W15_REG`'s total capacity)
//!   so a wildly out-of-range `SPI_MS_DATA_BITLEN` can't index past the `W`
//!   array.
//! - `SPI_DMA_CONF_REG` (`+0x30`): real read/write storage, except the
//!   three `WT` FIFO-reset pulses (`SPI_DMA_AFIFO_RST`/`SPI_BUF_AFIFO_RST`/
//!   `SPI_RX_AFIFO_RST`, bits 31/30/29), which there is no FIFO state for
//!   and which read 0. `SPI_DMA_TX_ENA` (bit 28, `spi_ll_dma_tx_enable`,
//!   `spi_ll.h:403-406`) is exposed as [`Spi::dma_tx_enabled`] but nothing
//!   consumes it yet: every transaction still sends the `W` buffer. The
//!   GDMA transmit path is plan Task 10.
//! - `SPI_DMA_INT_ENA_REG`/`_CLR_REG`/`_RAW_REG`/`_ST_REG`
//!   (`+0x34`/`+0x38`/`+0x3C`/`+0x40`), the transaction-done interrupt.
//!   Only `SPI_TRANS_DONE_INT_*` (bit 12 of each) is ever set by the model:
//!   [`Spi::finish_transaction`] sets its RAW bit, which is what
//!   `spi_ll_usr_is_done` (`spi_ll.h:277-280`) polls. Per the header's
//!   access types: ENA is `R/W` storage; CLR is `WT` (each written 1
//!   clears that RAW bit; reads 0); ST is `RO` = RAW & ENA. RAW is
//!   `R/W/WTC/SS` and is kept as plain read/write storage, because
//!   `spi_ll_clear_int_stat`/`spi_ll_set_int_stat` (`spi_ll.h:1155-1168`)
//!   clear and set `dma_int_raw.trans_done` by writing RAW directly.
//!   [`Spi::pending_sources`] asserts `ETS_SPI2_INTR_SOURCE`
//!   ([`crate::mem::soc::SRC_SPI2`], 19) while `TRANS_DONE`'s RAW & ENA is
//!   set: a level, recomputed live, so clearing RAW or ENA drops it.
//!   `FirmwareBus::pending_sources` ORs it in.
//! - `SPI_W0_REG..SPI_W15_REG` (`+0x98..+0xD4`, `+0x4` stride — see the
//!   correction above): 16 x 32-bit real read/write storage words, the MOSI
//!   byte payload. **Byte packing convention** (judgment call, per the
//!   brief): standard little-endian-within-word, ascending-word FIFO
//!   packing — transmitted byte 0 is `W0`'s least-significant byte, byte 1
//!   is `W0`'s next byte, ..., byte 4 is `W1`'s least-significant byte, etc.
//!   This matches how ESP-IDF's `spi_master` driver lays out `tx_buffer`
//!   into these registers; we're reconstructing logical byte *values*, not
//!   literal wire-level bit order (`SPI_WR_BIT_ORDER`/`SPI_D_POL`/etc. are
//!   out of scope, per the brief).
//! - Any other register in the SPI2 address range: real-storage-free "read
//!   0 / write ignored" default, same catch-all philosophy every other
//!   peripheral in this project uses.
//!
//! ## Transaction trigger flow
//!
//! [`Spi::write_byte`] only handles register storage + detecting the
//! `SPI_USR` trigger; it does *not* itself read GPIO0 or invoke the ST7789
//! interpreter, because it has no access to [`crate::peripherals::gpio::Gpio`].
//! Per this plan's "no trait-object peripheral dispatch" ruling, that
//! cross-peripheral read happens one level up, in
//! `crate::mem::bus::FirmwareBus`'s SPI2 dispatch tier: on a triggering
//! write, it reads `self.gpio.pin_level(0)` (GPIO0's *live* driven level,
//! not a stale snapshot) and calls [`Spi::process_transaction`] with the
//! D/C phase that read implies; [`Spi::process_transaction`] ends by
//! calling [`Spi::finish_transaction`] (clears `SPI_USR`, sets
//! `SPI_TRANS_DONE_INT_RAW`). See `mem::bus`'s module doc for exactly
//! where in the read/write dispatch order this sits.
//!
//! ## ST7789 command/data interpreter ([`St7789`])
//!
//! - **D/C low (command phase)**: each byte in the transaction becomes "the
//!   current command," processed in order (real firmware sends exactly one
//!   command byte per low-D/C transaction, but this doesn't assume that).
//!   Entering a new command byte resets any in-progress `CASET`/`RASET`
//!   parameter accumulation and any pending odd `RAMWR` byte (judgment
//!   call: a half-fed `CASET`/`RASET`, or a half-pixel `RAMWR` byte, that
//!   never got completed before a new command arrived is simply discarded,
//!   rather than carried forward into a different command's meaning). If
//!   the new command is `RAMWR` (`0x2C`), the pixel write cursor resets to
//!   `(XS, YS)` — matching real ST7789 behavior (each `RAMWR` restarts
//!   addressing at the write window's start).
//! - **D/C high (data phase)**: bytes are parameters/pixel data for
//!   whatever the current command is.
//!   - `CASET` (`0x2A`)/`RASET` (`0x2B`): accumulated 4 bytes at a time as
//!     `{XS,XE}` or `{YS,YE}` (big-endian 16-bit pairs). **Out-of-range
//!     clamping** (judgment call, per the brief): any parsed value `>=
//!     SCREEN_WIDTH`/`SCREEN_HEIGHT` is clamped to the last valid index
//!     (`SCREEN_WIDTH - 1`/`SCREEN_HEIGHT - 1`), not wrapped modulo —
//!     clamping is simpler to reason about, and there's no ST7789-defined
//!     wraparound semantics for an out-of-range *window* itself (as opposed
//!     to streaming pixel data past a valid window, which is a separate,
//!     intentional auto-increment behavior handled below). This keeps
//!     `XS/XE/YS/YE` always valid framebuffer indices, so `RAMWR`'s pixel
//!     writes can never go out of bounds/panic regardless of what firmware
//!     sends.
//!   - `RAMWR` (`0x2C`): all subsequent data bytes — across this
//!     transaction *and* any further D/C-high transactions, until a new
//!     D/C-low command byte arrives — are RGB565 pixel data, 2 bytes per
//!     pixel, big-endian (`R:5,G:6,B:5`, high byte first). **Odd-byte
//!     handling** (judgment call, per the brief): an incoming data byte
//!     while a `RAMWR` "pending high byte" is already buffered completes a
//!     pixel (`px = (pending_hi << 8) | this_byte`); otherwise the byte is
//!     buffered as the pending high byte. Since `RAMWR` streaming is
//!     explicitly defined to span multiple transactions, this means an odd
//!     trailing byte in one transaction correctly pairs with the next
//!     transaction's first byte to form a pixel — it is *not* silently
//!     dropped, unless a new command byte arrives first (see above).
//!     Pixels are written starting at `(XS, YS)`, advancing column-first
//!     up to `XE`, then wrapping to `(XS, row+1)`, continuing down to `YE`,
//!     then wrapping back to `(XS, YS)` if data keeps arriving past `YE` —
//!     standard ST7789 auto-increment addressing.
//!   - Any other command byte (`SWRESET` `0x01`, `SLPOUT` `0x11`, `COLMOD`
//!     `0x3A`, `MADCTL` `0x36`, `DISPON` `0x29`, etc.): accepted as "the
//!     current command" (so a subsequent stray data byte doesn't misfire
//!     into `CASET`/`RASET`/`RAMWR` logic) but has no framebuffer side
//!     effect. `MADCTL` orientation/mirroring is explicitly out of scope —
//!     this module hardcodes the natural unrotated `320x240` orientation
//!     (matching `SCREEN_WIDTH`/`SCREEN_HEIGHT` in `frontend/src/badge/ui.ts`, the
//!     same physical panel), per the brief.
//!
//! **Default write window**: before any `CASET`/`RASET`, the window
//! defaults to the full panel (`XS=0, XE=SCREEN_WIDTH-1, YS=0,
//! YE=SCREEN_HEIGHT-1`) — a documented default (real ST7789 hardware reset
//! state isn't specified by the brief), not a reverse-engineered value.
//!
//! ## Framebuffer storage + accessor
//!
//! [`St7789::framebuffer`] returns `&[u16]`: RGB565 values, row-major,
//! `SCREEN_WIDTH * SCREEN_HEIGHT` (`320 * 240`) elements, index
//! `y * SCREEN_WIDTH + x`. [`Spi::framebuffer`] forwards to it, so Task 6's
//! `emulator-wasm` bridge can call `bus.spi.framebuffer()` directly. This is
//! a plain Rust method — no `wasm-bindgen` surface is added in this task
//! (that's Task 6's scope).

use super::set_byte;

pub const CMD_REG: u32 = 0x00;
pub const USER_REG: u32 = 0x10;
pub const MS_DLEN_REG: u32 = 0x1C;
pub const DMA_CONF_REG: u32 = 0x30;
pub const DMA_INT_ENA_REG: u32 = 0x34;
pub const DMA_INT_CLR_REG: u32 = 0x38;
pub const DMA_INT_RAW_REG: u32 = 0x3C;
pub const DMA_INT_ST_REG: u32 = 0x40;
pub const W0_REG: u32 = 0x98;
pub const W15_REG: u32 = 0xD4;

/// `SPI_UPDATE`, `SPI_CMD_REG` bit 23 (`WT`: write-1 trigger, reads 0).
pub const CMD_UPDATE: u32 = 1 << 23;
/// `SPI_USR`, `SPI_CMD_REG` bit 24 (`R/W/SC`) — lives entirely within byte
/// index 3.
pub const CMD_USR: u32 = 1 << 24;
const CMD_USR_BYTE_IDX: u32 = 3;
/// `SPI_USR`'s bit position within its own byte (byte 3 = bits `[31:24]`,
/// so bit 24 is that byte's bit 0).
const CMD_USR_BIT_IN_BYTE: u8 = 0x01;

/// `SPI_DMA_TX_ENA`, `SPI_DMA_CONF_REG` bit 28 (`R/W`).
pub const DMA_TX_ENA: u32 = 1 << 28;
/// `SPI_DMA_AFIFO_RST`/`SPI_BUF_AFIFO_RST`/`SPI_RX_AFIFO_RST`,
/// `SPI_DMA_CONF_REG` bits 31/30/29 — all `WT` (write-1 pulse, reads 0).
const DMA_CONF_WT_MASK: u32 = 0b111 << 29;

/// `SPI_TRANS_DONE_INT_{ENA,CLR,RAW,ST}`, bit 12 of each `SPI_DMA_INT_*`
/// register.
pub const TRANS_DONE: u32 = 1 << 12;

/// `SPI_MS_DATA_BITLEN`, `SPI_MS_DLEN_REG` bits `[17:0]`.
const MS_DATA_BITLEN_MASK: u32 = 0x0003_FFFF;

/// Number of 32-bit `W` words / max transaction byte count (`16 * 4`).
const NUM_W_WORDS: usize = 16;
const MAX_TRANSACTION_BYTES: usize = NUM_W_WORDS * 4;

pub const SCREEN_WIDTH: usize = 320;
pub const SCREEN_HEIGHT: usize = 240;

pub const CMD_CASET: u8 = 0x2A;
pub const CMD_RASET: u8 = 0x2B;
pub const CMD_RAMWR: u8 = 0x2C;

/// The ST7789 command/pixel-stream interpreter. See the module doc for the
/// full behavior and every judgment call it required.
pub struct St7789 {
    framebuffer: Vec<u16>,
    current_command: Option<u8>,
    xs: u16,
    xe: u16,
    ys: u16,
    ye: u16,
    cursor_x: u16,
    cursor_y: u16,
    caset_buf: Vec<u8>,
    raset_buf: Vec<u8>,
    ramwr_pending_high: Option<u8>,
}

impl Default for St7789 {
    fn default() -> Self {
        Self {
            framebuffer: vec![0u16; SCREEN_WIDTH * SCREEN_HEIGHT],
            current_command: None,
            xs: 0,
            xe: (SCREEN_WIDTH - 1) as u16,
            ys: 0,
            ye: (SCREEN_HEIGHT - 1) as u16,
            cursor_x: 0,
            cursor_y: 0,
            caset_buf: Vec::with_capacity(4),
            raset_buf: Vec::with_capacity(4),
            ramwr_pending_high: None,
        }
    }
}

impl St7789 {
    pub fn new() -> Self {
        Self::default()
    }

    /// The reconstructed framebuffer: RGB565, row-major, `SCREEN_WIDTH *
    /// SCREEN_HEIGHT` elements, index `y * SCREEN_WIDTH + x`.
    pub fn framebuffer(&self) -> &[u16] {
        &self.framebuffer
    }

    /// The current write window `(xs, xe, ys, ye)`, as last set by
    /// `CASET`/`RASET` (or the documented full-panel default). Exposed for
    /// tests.
    pub fn window(&self) -> (u16, u16, u16, u16) {
        (self.xs, self.xe, self.ys, self.ye)
    }

    /// Feeds one SPI transaction's byte sequence to the interpreter.
    /// `dc_low`: `true` if the D/C line was low (command phase) for this
    /// transaction, `false` if high (data phase). See the module doc for
    /// the full command/data behavior.
    pub fn handle_transaction(&mut self, dc_low: bool, bytes: &[u8]) {
        if dc_low {
            for &b in bytes {
                self.current_command = Some(b);
                // A new command byte discards any in-progress CASET/RASET
                // parameter accumulation and any pending odd RAMWR byte —
                // see the module doc's documented judgment call.
                self.caset_buf.clear();
                self.raset_buf.clear();
                self.ramwr_pending_high = None;
                if b == CMD_RAMWR {
                    self.cursor_x = self.xs;
                    self.cursor_y = self.ys;
                }
            }
            return;
        }

        for &b in bytes {
            match self.current_command {
                Some(CMD_CASET) => self.feed_caset_byte(b),
                Some(CMD_RASET) => self.feed_raset_byte(b),
                Some(CMD_RAMWR) => self.feed_ramwr_byte(b),
                _ => {}
            }
        }
    }

    fn feed_caset_byte(&mut self, b: u8) {
        self.caset_buf.push(b);
        if self.caset_buf.len() == 4 {
            let xs = u16::from_be_bytes([self.caset_buf[0], self.caset_buf[1]]);
            let xe = u16::from_be_bytes([self.caset_buf[2], self.caset_buf[3]]);
            self.xs = xs.min((SCREEN_WIDTH - 1) as u16);
            self.xe = xe.min((SCREEN_WIDTH - 1) as u16);
            self.caset_buf.clear();
        }
    }

    fn feed_raset_byte(&mut self, b: u8) {
        self.raset_buf.push(b);
        if self.raset_buf.len() == 4 {
            let ys = u16::from_be_bytes([self.raset_buf[0], self.raset_buf[1]]);
            let ye = u16::from_be_bytes([self.raset_buf[2], self.raset_buf[3]]);
            self.ys = ys.min((SCREEN_HEIGHT - 1) as u16);
            self.ye = ye.min((SCREEN_HEIGHT - 1) as u16);
            self.raset_buf.clear();
        }
    }

    fn feed_ramwr_byte(&mut self, b: u8) {
        match self.ramwr_pending_high.take() {
            Some(hi) => {
                let px = ((hi as u16) << 8) | b as u16;
                self.write_pixel(px);
            }
            None => self.ramwr_pending_high = Some(b),
        }
    }

    fn write_pixel(&mut self, px: u16) {
        let idx = self.cursor_y as usize * SCREEN_WIDTH + self.cursor_x as usize;
        if let Some(slot) = self.framebuffer.get_mut(idx) {
            *slot = px;
        }
        if self.cursor_x >= self.xe {
            self.cursor_x = self.xs;
            self.cursor_y = if self.cursor_y >= self.ye {
                self.ys
            } else {
                self.cursor_y + 1
            };
        } else {
            self.cursor_x += 1;
        }
    }
}

/// The SPI2 (GPSPI2) peripheral: register storage for `SPI_CMD_REG`/
/// `SPI_USER_REG`/`SPI_MS_DLEN_REG`/`SPI_DMA_CONF_REG`/`SPI_DMA_INT_*`/
/// `SPI_W0..W15_REG`, plus the [`St7789`]
/// interpreter it feeds on each triggered transaction. See the module doc
/// for the full register model and the trigger flow (which needs a
/// cross-peripheral GPIO read `Spi` itself doesn't have access to — see
/// [`Spi::write_byte`]'s doc).
pub struct Spi {
    cmd: u32,
    user: u32,
    ms_dlen: u32,
    dma_conf: u32,
    dma_int_ena: u32,
    dma_int_raw: u32,
    w: [u32; NUM_W_WORDS],
    pub st7789: St7789,
}

impl Default for Spi {
    fn default() -> Self {
        Self {
            cmd: 0,
            user: 0,
            ms_dlen: 0,
            dma_conf: 0,
            dma_int_ena: 0,
            dma_int_raw: 0,
            w: [0u32; NUM_W_WORDS],
            st7789: St7789::new(),
        }
    }
}

impl Spi {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`St7789::framebuffer`], forwarded — Task 6's `emulator-wasm` bridge
    /// consumes this directly via `bus.spi.framebuffer()`.
    pub fn framebuffer(&self) -> &[u16] {
        self.st7789.framebuffer()
    }

    pub fn read_byte(&self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            CMD_REG => self.cmd,
            USER_REG => self.user,
            MS_DLEN_REG => self.ms_dlen,
            DMA_CONF_REG => self.dma_conf,
            DMA_INT_ENA_REG => self.dma_int_ena,
            // SPI_DMA_INT_CLR_REG is all `WT`: reads 0 (falls to `_`).
            DMA_INT_RAW_REG => self.dma_int_raw,
            DMA_INT_ST_REG => self.dma_int_raw & self.dma_int_ena,
            W0_REG..=W15_REG => {
                let word_idx = ((word_offset - W0_REG) / 4) as usize;
                self.w.get(word_idx).copied().unwrap_or(0)
            }
            _ => 0,
        };
        word.to_le_bytes()[idx]
    }

    /// Stores one byte of a register write. Returns `true` if this write
    /// just set `SPI_CMD_REG`'s `SPI_USR` bit (i.e. a transaction needs
    /// processing). `SPI_USR` stays set (it is `R/W/SC`) until
    /// [`Spi::finish_transaction`] clears it — which
    /// [`Spi::process_transaction`] does before returning, so with the
    /// caller's synchronous processing, firmware never observes it set.
    /// `SPI_UPDATE` (`WT`) is never stored: it reads back 0 at once (see the
    /// module doc).
    ///
    /// This method deliberately does *not* itself call into
    /// [`St7789::handle_transaction`] — it has no access to
    /// [`crate::peripherals::gpio::Gpio`], and reading the D/C line (GPIO0)
    /// is `FirmwareBus`'s job (see the module doc's "Transaction trigger
    /// flow" section). On a `true` return, the caller must read GPIO0's
    /// live level and call [`Spi::process_transaction`].
    pub fn write_byte(&mut self, offset: u32, val: u8) -> bool {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            CMD_REG => {
                set_byte(&mut self.cmd, idx, val);
                // SPI_UPDATE is a write-1 pulse: the APB->SPI register sync
                // it requests completes instantly here, so it never sticks.
                self.cmd &= !CMD_UPDATE;
                idx == CMD_USR_BYTE_IDX && val & CMD_USR_BIT_IN_BYTE != 0
            }
            USER_REG => {
                set_byte(&mut self.user, idx, val);
                false
            }
            MS_DLEN_REG => {
                set_byte(&mut self.ms_dlen, idx, val);
                false
            }
            DMA_CONF_REG => {
                set_byte(&mut self.dma_conf, idx, val);
                // The three AFIFO_RST bits are `WT` pulses: there is no
                // FIFO state to reset, and they read back 0.
                self.dma_conf &= !DMA_CONF_WT_MASK;
                false
            }
            DMA_INT_ENA_REG => {
                set_byte(&mut self.dma_int_ena, idx, val);
                false
            }
            DMA_INT_CLR_REG => {
                // `WT`: each written 1 clears the matching RAW bit. Byte-
                // granular, so a whole-word write clears exactly the bits it
                // carries, once.
                self.dma_int_raw &= !((val as u32) << (idx * 8));
                false
            }
            DMA_INT_RAW_REG => {
                // R/W from software: spi_ll_set_int_stat/
                // spi_ll_clear_int_stat write dma_int_raw.trans_done directly.
                set_byte(&mut self.dma_int_raw, idx, val);
                false
            }
            W0_REG..=W15_REG => {
                let word_idx = ((word_offset - W0_REG) / 4) as usize;
                if let Some(w) = self.w.get_mut(word_idx) {
                    set_byte(w, idx, val);
                }
                false
            }
            // Every other SPI2 register this module doesn't name: accepted
            // and dropped, matching every other peripheral's catch-all
            // philosophy.
            _ => false,
        }
    }

    /// The transaction's byte count from `SPI_MS_DATA_BITLEN`:
    /// `ceil((field + 1) / 8)`. Not capped — a DMA transfer (Task 10) can
    /// exceed the 64-byte `W0..W15` buffer; [`Spi::process_transaction`]
    /// applies that cap itself for the CPU-buffer path.
    pub fn tx_byte_len(&self) -> usize {
        let bit_count = (self.ms_dlen & MS_DATA_BITLEN_MASK) + 1;
        (bit_count as usize).div_ceil(8)
    }

    /// `SPI_DMA_TX_ENA` (`SPI_DMA_CONF_REG` bit 28), as set by
    /// `spi_ll_dma_tx_enable`.
    pub fn dma_tx_enabled(&self) -> bool {
        self.dma_conf & DMA_TX_ENA != 0
    }

    /// Completes the in-flight transaction: clears `SPI_USR` (its `SC`
    /// half) and sets `SPI_TRANS_DONE_INT_RAW`, which is what
    /// `spi_ll_usr_is_done` polls.
    pub fn finish_transaction(&mut self) {
        self.cmd &= !CMD_USR;
        self.dma_int_raw |= TRANS_DONE;
    }

    /// The interrupt sources SPI2 asserts right now: bit
    /// [`crate::mem::soc::SRC_SPI2`] iff `SPI_TRANS_DONE_INT_ST` (RAW & ENA)
    /// is set. Level, not latched — clearing RAW or ENA de-asserts it.
    pub fn pending_sources(&self) -> u64 {
        if self.dma_int_raw & self.dma_int_ena & TRANS_DONE != 0 {
            1u64 << crate::mem::soc::SRC_SPI2
        } else {
            0
        }
    }

    /// Extracts the triggered transaction's byte payload from `SPI_W0..
    /// W15_REG` per `SPI_MS_DLEN_REG` and the module doc's byte-packing
    /// convention, hands it to [`St7789::handle_transaction`], then
    /// [`Spi::finish_transaction`]. `dc_low` is the D/C line's live level at
    /// the moment of the trigger, as read by the caller (`FirmwareBus`) —
    /// see the module doc.
    pub fn process_transaction(&mut self, dc_low: bool) {
        let byte_count = self.tx_byte_len().min(MAX_TRANSACTION_BYTES);

        let mut bytes = Vec::with_capacity(byte_count);
        for i in 0..byte_count {
            let word = self.w[i / 4];
            let shift = (i % 4) * 8;
            bytes.push(((word >> shift) & 0xFF) as u8);
        }

        self.st7789.handle_transaction(dc_low, &bytes);
        self.finish_transaction();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(s: &mut Spi, word_offset: u32, val: u32) -> bool {
        let mut triggered = false;
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            triggered |= s.write_byte(word_offset + i as u32, *b);
        }
        triggered
    }

    fn read_word(s: &Spi, word_offset: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = s.read_byte(word_offset + i as u32);
        }
        u32::from_le_bytes(bytes)
    }

    // ---- SPI register unit tests ----

    #[test]
    fn user_reg_and_ms_dlen_and_w_regs_are_stored_and_read_back() {
        let mut s = Spi::new();
        write_word(&mut s, USER_REG, 0xF000_0000);
        assert_eq!(read_word(&s, USER_REG), 0xF000_0000);

        write_word(&mut s, MS_DLEN_REG, 31);
        assert_eq!(read_word(&s, MS_DLEN_REG), 31);

        write_word(&mut s, W0_REG, 0xDEAD_BEEF);
        write_word(&mut s, W15_REG, 0xCAFE_BABE);
        assert_eq!(read_word(&s, W0_REG), 0xDEAD_BEEF);
        assert_eq!(read_word(&s, W15_REG), 0xCAFE_BABE);
        // An untouched middle word must still read back as its default 0.
        assert_eq!(read_word(&s, W0_REG + 4 * 5), 0);
    }

    /// Counts how many of a whole-word write's four constituent byte writes
    /// reported a trigger (the review-focus #1 "fires exactly once" check).
    fn write_word_count_triggers(s: &mut Spi, word_offset: u32, val: u32) -> usize {
        val.to_le_bytes()
            .iter()
            .enumerate()
            .filter(|(i, b)| s.write_byte(word_offset + *i as u32, **b))
            .count()
    }

    #[test]
    fn setting_spi_usr_triggers_once_and_clears_when_the_transaction_finishes() {
        let mut s = Spi::new();
        write_word(&mut s, MS_DLEN_REG, 7); // 1 byte
        write_word(&mut s, W0_REG, 0x2A);
        assert_eq!(write_word_count_triggers(&mut s, CMD_REG, CMD_USR), 1);
        // SPI_USR is R/W/SC: still set until the transaction completes.
        assert_ne!(read_word(&s, CMD_REG) & CMD_USR, 0);
        s.process_transaction(false);
        assert_eq!(
            read_word(&s, CMD_REG) & CMD_USR,
            0,
            "SPI_USR must read back as cleared once the transaction has finished"
        );
    }

    #[test]
    fn update_bit_self_clears() {
        let mut s = Spi::new();
        write_word(&mut s, CMD_REG, CMD_UPDATE);
        assert_eq!(read_word(&s, CMD_REG) & CMD_UPDATE, 0);
    }

    #[test]
    fn update_write_keeps_the_rest_of_the_word_and_does_not_trigger_a_transaction() {
        let mut s = Spi::new();
        // SPI_CONF_BITLEN [17:0] alongside UPDATE in the same whole-word write.
        let triggers = write_word_count_triggers(&mut s, CMD_REG, CMD_UPDATE | 0x1234);
        assert_eq!(triggers, 0, "UPDATE alone must not start a transaction");
        assert_eq!(read_word(&s, CMD_REG), 0x1234);
    }

    #[test]
    fn update_and_usr_in_one_word_trigger_exactly_once_and_update_reads_zero() {
        let mut s = Spi::new();
        write_word(&mut s, MS_DLEN_REG, 7);
        let triggers = write_word_count_triggers(&mut s, CMD_REG, CMD_UPDATE | CMD_USR);
        assert_eq!(triggers, 1);
        assert_eq!(read_word(&s, CMD_REG) & CMD_UPDATE, 0);
        s.process_transaction(true);
        assert_eq!(read_word(&s, DMA_INT_RAW_REG) & TRANS_DONE, TRANS_DONE);
        assert_eq!(read_word(&s, CMD_REG), 0);
    }

    #[test]
    fn transaction_sets_trans_done_raw_and_int_when_enabled() {
        let mut s = Spi::new();
        write_word(&mut s, DMA_INT_ENA_REG, TRANS_DONE);
        write_word(&mut s, MS_DLEN_REG, 7);
        write_word(&mut s, W0_REG, 0x2C);
        if write_word(&mut s, CMD_REG, CMD_USR) {
            s.process_transaction(true);
        }
        assert_ne!(read_word(&s, DMA_INT_RAW_REG) & TRANS_DONE, 0);
        assert_ne!(s.pending_sources() & (1u64 << crate::mem::soc::SRC_SPI2), 0);
        write_word(&mut s, DMA_INT_CLR_REG, TRANS_DONE);
        assert_eq!(s.pending_sources(), 0);
    }

    #[test]
    fn trans_done_raw_is_set_even_when_masked_but_does_not_assert_the_source() {
        let mut s = Spi::new();
        s.finish_transaction();
        assert_eq!(read_word(&s, DMA_INT_RAW_REG), TRANS_DONE);
        assert_eq!(read_word(&s, DMA_INT_ST_REG), 0, "ST = RAW & ENA");
        assert_eq!(s.pending_sources(), 0);
        write_word(&mut s, DMA_INT_ENA_REG, TRANS_DONE);
        assert_eq!(read_word(&s, DMA_INT_ST_REG), TRANS_DONE);
        assert_eq!(s.pending_sources(), 1u64 << crate::mem::soc::SRC_SPI2);
    }

    #[test]
    fn dma_int_raw_is_software_writable_like_spi_ll_clear_and_set_int_stat() {
        // spi_ll_set_int_stat / spi_ll_clear_int_stat write
        // dma_int_raw.trans_done directly.
        let mut s = Spi::new();
        write_word(&mut s, DMA_INT_RAW_REG, TRANS_DONE);
        assert_eq!(read_word(&s, DMA_INT_RAW_REG), TRANS_DONE);
        write_word(&mut s, DMA_INT_RAW_REG, 0);
        assert_eq!(read_word(&s, DMA_INT_RAW_REG), 0);
    }

    #[test]
    fn dma_int_clr_is_write_one_to_clear_and_reads_zero() {
        let mut s = Spi::new();
        write_word(&mut s, DMA_INT_RAW_REG, TRANS_DONE | 1);
        write_word(&mut s, DMA_INT_CLR_REG, TRANS_DONE);
        assert_eq!(
            read_word(&s, DMA_INT_RAW_REG),
            1,
            "only the written-1 bit clears"
        );
        assert_eq!(read_word(&s, DMA_INT_CLR_REG), 0);
        write_word(&mut s, DMA_INT_ENA_REG, 0xFFFF);
        assert_eq!(read_word(&s, DMA_INT_ENA_REG), 0xFFFF);
    }

    #[test]
    fn dma_conf_stores_tx_ena_and_fifo_reset_pulses_read_zero() {
        let mut s = Spi::new();
        assert!(!s.dma_tx_enabled());
        write_word(&mut s, DMA_CONF_REG, DMA_TX_ENA | DMA_CONF_WT_MASK | 0x3);
        assert!(s.dma_tx_enabled());
        assert_eq!(read_word(&s, DMA_CONF_REG), DMA_TX_ENA | 0x3);
        write_word(&mut s, DMA_CONF_REG, 0);
        assert!(!s.dma_tx_enabled());
    }

    #[test]
    fn tx_byte_len_follows_ms_dlen_rounding_up() {
        let mut s = Spi::new();
        write_word(&mut s, MS_DLEN_REG, 7);
        assert_eq!(s.tx_byte_len(), 1);
        write_word(&mut s, MS_DLEN_REG, 8);
        assert_eq!(s.tx_byte_len(), 2);
        write_word(&mut s, MS_DLEN_REG, 200 * 8 - 1);
        assert_eq!(
            s.tx_byte_len(),
            200,
            "not capped: DMA lengths exceed the W buffer"
        );
    }

    #[test]
    fn writing_cmd_reg_without_usr_bit_does_not_trigger() {
        let mut s = Spi::new();
        let triggered = write_word(&mut s, CMD_REG, 0x0000_00FF); // bit24 not set
        assert!(!triggered);
    }

    #[test]
    fn unmodeled_spi2_register_reads_zero_and_drops_writes() {
        let mut s = Spi::new();
        assert_eq!(s.read_byte(0x40), 0); // arbitrary unmodeled offset
        assert!(!s.write_byte(0x40, 0xFF)); // must not panic, must not trigger
        assert_eq!(s.read_byte(0x40), 0);
    }

    // ---- ST7789 interpreter unit tests ----

    #[test]
    fn caset_and_raset_set_the_write_window() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x05, 0x00, 0x0A]); // xs=5, xe=10
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x02, 0x00, 0x03]); // ys=2, ye=3
        assert_eq!(lcd.window(), (5, 10, 2, 3));
    }

    #[test]
    fn ramwr_writes_pixels_big_endian_advancing_column_first_then_row() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]); // xs=0, xe=1
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]); // ys=0, ye=1
        lcd.handle_transaction(true, &[CMD_RAMWR]);
        // 4 pixels, big-endian RGB565: red, green, blue, white.
        lcd.handle_transaction(false, &[0xF8, 0x00, 0x07, 0xE0, 0x00, 0x1F, 0xFF, 0xFF]);
        let fb = lcd.framebuffer();
        assert_eq!(fb[0], 0xF800, "(0,0) red");
        assert_eq!(fb[1], 0x07E0, "(1,0) green");
        assert_eq!(fb[SCREEN_WIDTH], 0x001F, "(0,1) blue");
        assert_eq!(fb[SCREEN_WIDTH + 1], 0xFFFF, "(1,1) white");
    }

    #[test]
    fn ramwr_data_split_across_multiple_transactions_still_streams_correctly() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]);
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]);
        lcd.handle_transaction(true, &[CMD_RAMWR]);
        // Split the 4 pixels' 8 bytes across three transactions, including
        // one that splits a pixel's high/low byte across the boundary.
        lcd.handle_transaction(false, &[0xF8, 0x00, 0x07]);
        lcd.handle_transaction(false, &[0xE0, 0x00]);
        lcd.handle_transaction(false, &[0x1F, 0xFF, 0xFF]);
        let fb = lcd.framebuffer();
        assert_eq!(fb[0], 0xF800);
        assert_eq!(fb[1], 0x07E0);
        assert_eq!(fb[SCREEN_WIDTH], 0x001F);
        assert_eq!(fb[SCREEN_WIDTH + 1], 0xFFFF);
    }

    #[test]
    fn ramwr_streaming_past_the_window_wraps_back_to_its_start() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]); // xs=0,xe=1
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]); // ys=0,ye=1
        lcd.handle_transaction(true, &[CMD_RAMWR]);
        // Window holds 4 pixels; stream 6 -- pixels 5 and 6 must wrap back
        // to (0,0) and (1,0) without panicking or corrupting anything.
        let mut bytes = Vec::new();
        for px in [0x0001u16, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006] {
            bytes.extend_from_slice(&px.to_be_bytes());
        }
        lcd.handle_transaction(false, &bytes);
        let fb = lcd.framebuffer();
        assert_eq!(fb[0], 0x0005, "wrapped back to (0,0)");
        assert_eq!(fb[1], 0x0006, "wrapped back to (1,0)");
    }

    #[test]
    fn a_command_with_no_pixel_behavior_does_not_panic_or_corrupt_state() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]);
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]);
        let window_before = lcd.window();
        let fb_before = lcd.framebuffer().to_vec();

        lcd.handle_transaction(true, &[0x11]); // SLPOUT
        lcd.handle_transaction(false, &[0xAA, 0xBB, 0xCC]); // stray data bytes

        assert_eq!(lcd.window(), window_before, "window must be unchanged");
        assert_eq!(
            lcd.framebuffer(),
            fb_before.as_slice(),
            "framebuffer must be unchanged"
        );
    }

    #[test]
    fn dc_level_routes_new_command_vs_data_for_current_command() {
        let mut lcd = St7789::new();
        // D/C low: 0x2A becomes the current command (CASET), not data.
        lcd.handle_transaction(true, &[CMD_CASET]);
        // D/C high: these 4 bytes are CASET's parameters, not a new command.
        lcd.handle_transaction(false, &[0x00, 0x01, 0x00, 0x02]);
        assert_eq!(lcd.window().0, 1, "xs parsed from data phase");
        assert_eq!(lcd.window().1, 2, "xe parsed from data phase");
    }

    #[test]
    fn out_of_range_caset_raset_values_clamp_instead_of_panicking() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        // xs/xe far beyond SCREEN_WIDTH.
        lcd.handle_transaction(false, &[0xFF, 0xFF, 0xFF, 0xFF]);
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(
            lcd.window(),
            (
                (SCREEN_WIDTH - 1) as u16,
                (SCREEN_WIDTH - 1) as u16,
                (SCREEN_HEIGHT - 1) as u16,
                (SCREEN_HEIGHT - 1) as u16
            )
        );
        // And a RAMWR against this clamped window must not panic.
        lcd.handle_transaction(true, &[CMD_RAMWR]);
        lcd.handle_transaction(false, &[0x12, 0x34]);
        assert_eq!(
            lcd.framebuffer()[(SCREEN_HEIGHT - 1) * SCREEN_WIDTH + (SCREEN_WIDTH - 1)],
            0x1234
        );
    }

    #[test]
    fn odd_trailing_ramwr_byte_carries_into_the_next_transaction() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x00]); // xs=xe=0
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x00]); // ys=ye=0
        lcd.handle_transaction(true, &[CMD_RAMWR]);
        lcd.handle_transaction(false, &[0xAB]); // odd trailing byte
        lcd.handle_transaction(false, &[0xCD]); // completes the pixel next transaction
        assert_eq!(lcd.framebuffer()[0], 0xABCD);
    }

    #[test]
    fn a_new_command_discards_a_pending_odd_ramwr_byte() {
        let mut lcd = St7789::new();
        lcd.handle_transaction(true, &[CMD_CASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x01]);
        lcd.handle_transaction(true, &[CMD_RASET]);
        lcd.handle_transaction(false, &[0x00, 0x00, 0x00, 0x00]);
        lcd.handle_transaction(true, &[CMD_RAMWR]);
        lcd.handle_transaction(false, &[0xAB]); // odd trailing byte, never completed
        lcd.handle_transaction(true, &[CMD_RAMWR]); // new command byte: discards it, resets cursor
        lcd.handle_transaction(false, &[0x12, 0x34]); // a fresh, complete pixel
        assert_eq!(
            lcd.framebuffer()[0],
            0x1234,
            "the discarded 0xAB must not have leaked into this pixel"
        );
    }

    // ---- Spi::process_transaction (register model -> ST7789) glue test ----

    #[test]
    fn process_transaction_extracts_bytes_per_the_packing_convention_and_ms_dlen() {
        let mut s = Spi::new();
        // Construct W0 = [0x00,0x00,0x00,0x01] (xs_hi,xs_lo,xe_hi,xe_lo) per
        // the little-endian-within-word packing convention: byte0 in
        // bits[7:0].
        let w0 = 0x01u32 << 24;
        write_word(&mut s, W0_REG, w0);
        write_word(&mut s, MS_DLEN_REG, 31); // 4 bytes

        s.st7789.handle_transaction(true, &[CMD_CASET]);
        s.process_transaction(false);

        assert_eq!(s.st7789.window().0, 0);
        assert_eq!(s.st7789.window().1, 1);
    }
}

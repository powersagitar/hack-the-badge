//! ESP32-C3 USB-Serial-JTAG peripheral (`DR_REG_USB_SERIAL_JTAG_BASE ==
//! 0x6004_3000`), modeled just far enough to capture everything ESP-IDF's
//! console driver (`usb_serial_jtag_vfs`/`esp_rom_uart` TX path) writes to
//! it, into a [`crate::peripherals::console::Console`].
//!
//! Register layout confirmed against ESP-IDF v5.5.3's
//! `components/soc/esp32c3/register/soc/usb_serial_jtag_reg.h` (fetched
//! directly this task, not guessed):
//!
//! - [`EP1_REG`] (`USB_SERIAL_JTAG_EP1_REG`, +0x00): `RDWR_BYTE` is
//!   documented in the header's prose comment as bitpos `[8:0]`, but the
//!   header's own mask macro (`USB_SERIAL_JTAG_RDWR_BYTE == 0x000000FF`) is
//!   an 8-bit field, bits `[7:0]` — the prose comment is a header typo, the
//!   mask wins. Writing byte index 0 of this register pushes one TX byte
//!   into the console (real hardware: the same write enqueues into the
//!   USB-CDC TX FIFO); a read of byte lane 0 pops one RX byte from the OUT
//!   FIFO (0 when empty; see "RX" below).
//! - [`EP1_CONF_REG`] (`USB_SERIAL_JTAG_EP1_CONF_REG`, +0x04): bit 0
//!   `WR_DONE` (WT, "commit the FIFO write") is a write-trigger flush this
//!   emulator has nothing to flush (each `EP1_REG` write already lands
//!   immediately) — accepted and ignored. Bit 1
//!   `SERIAL_IN_EP_DATA_FREE` (RO, header default 1) always reads 1: TX is
//!   modeled as having infinite room, so a driver polling "can I write
//!   more" never blocks. Bit 2 `SERIAL_OUT_EP_DATA_AVAIL` (RO, header
//!   default 0) reads 1 while the OUT FIFO holds bytes (see "RX" below).
//! - `USB_SERIAL_JTAG_INT_RAW_REG` (+0x08): plain word storage (see below),
//!   except bit 3 `SERIAL_IN_EMPTY_INT_RAW` (header: R/WTC/SS, default 1) —
//!   confirmed at bit position 3 in the header — which this model forces to
//!   read as 1 unconditionally, regardless of what was last written there.
//!   TX is modeled as always-empty (see `EP1_CONF_REG` above), so an
//!   interrupt-driven TX driver polling this bit for "FIFO empty, safe to
//!   queue more" must always see it set, or it stalls waiting for an
//!   interrupt this emulator's peripheral model has no timing/edge
//!   machinery to raise. Bit 1 `SOF_INT_RAW` (header: R/WTC/SS, default
//!   0, "turns to high level when a SOF frame is received") is forced to
//!   read 1 the same way (Milestone 4 Task 4): the emulated badge has a USB
//!   host attached that sends a start-of-frame every 1 ms, so the bit is
//!   always set again by the next read, whatever `INT_CLR_REG` (+0x14,
//!   `USB_SERIAL_JTAG_SOF_INT_CLR` = bit 1) or a store here did. ESP-IDF
//!   depends on it: `usb_serial_jtag_connection_monitor.c` (v5.5.3,
//!   `esp_driver_usb_serial_jtag`) reads it from a FreeRTOS tick hook
//!   through `usb_serial_jtag_ll_get_intraw_mask()` (`hal/esp32c3/include/
//!   hal/usb_serial_jtag_ll.h`, `int_raw.val`), clears it with
//!   `usb_serial_jtag_ll_clr_intsts_mask()` (`int_clr.val = mask`), and
//!   marks the port disconnected on the first tick without it; while
//!   disconnected, `usb_serial_jtag_vfs.c`'s `usb_serial_jtag_write()`
//!   returns -1 without touching the FIFO, so every `stdout` byte (every
//!   `ESP_LOG*` line after the scheduler starts) is dropped.
//! - `INT_ST_REG` (+0x0C, RO), `INT_ENA_REG` (+0x10, R/W) and `INT_CLR_REG`
//!   (+0x14, WT) are real (Milestone 5 Task 2). `INT_ST = INT_RAW &
//!   INT_ENA`, where the `INT_RAW` view includes the forced bits above; a 1
//!   written to `INT_CLR` or to `INT_RAW` (R/WTC) clears a latched raw bit
//!   (the forced ones re-assert on the next read); `INT_CLR` reads 0.
//!   `ETS_USB_SERIAL_JTAG_INTR_SOURCE` (26, `soc/esp32c3/include/soc/
//!   interrupts.h`) is a level while `INT_ST != 0`. Because
//!   `SERIAL_IN_EMPTY` is forced, enabling it asserts the interrupt at once
//!   -- exactly what `esp_driver_usb_serial_jtag/src/usb_serial_jtag.c`'s
//!   TX path relies on (it enables SERIAL_IN_EMPTY while its ring buffer
//!   holds bytes and disables it when drained).
//! - Every other offset (`CONF0_REG`, `MISC_CONF_REG`, the various `*_ST`
//!   status registers, …): plain read/write word
//!   storage, keyed by word-aligned offset in [`UsbSerialJtag::regs`], so a
//!   driver that pokes at USB-Serial-JTAG's interrupt/config plumbing
//!   during setup doesn't get lost (reads back whatever was last written,
//!   defaulting to 0) without this module needing to name every one of
//!   them individually.
//!
//! ## RX (Milestone 5 Task 2)
//!
//! A host-side queue ([`UsbSerialJtag::host_send`]) feeds the 64-byte OUT
//! FIFO one full-speed bulk packet at a time: a packet loads when the FIFO
//! is empty and [`PACKET_TICKS`] have passed since the previous load, and
//! latches `SERIAL_OUT_RECV_PKT_INT_RAW` (bit 2 of RAW/ST/ENA/CLR,
//! `usb_serial_jtag_reg.h`). `hal/esp32c3/include/hal/usb_serial_jtag_ll.h`'s
//! `usb_serial_jtag_ll_read_rxfifo` loops while
//! `ep1_conf.serial_out_ep_data_avail`, reading `ep1.rdwr_byte`; a `lw` of
//! `EP1_REG` reaches this module as four ascending byte reads and only
//! lane 0 pops. Time arrives via [`UsbSerialJtag::advance`] (one step's
//! ticks, or an idle fast-forward bounded by
//! [`UsbSerialJtag::ticks_until_next_packet`]). No USB bus reset, JTAG or
//! EP2 is modeled.
//!
//! This mirrors the pattern `crate::peripherals::intc` used for its MAP
//! registers until Milestone 3 Task 4 (concrete named registers get real
//! behavior; everything else in the peripheral's address window is a
//! generic word-storage `HashMap`) rather than introducing a new idiom.

use std::collections::{HashMap, VecDeque};

use super::console::Console;
use super::set_byte;
use crate::mem::soc::SRC_USB_SERIAL_JTAG;

/// `USB_SERIAL_JTAG_EP1_REG`. See the module doc.
pub const EP1_REG: u32 = 0x00;
/// `USB_SERIAL_JTAG_EP1_CONF_REG`. See the module doc.
pub const EP1_CONF_REG: u32 = 0x04;
/// `USB_SERIAL_JTAG_INT_RAW_REG`. See the module doc.
pub const INT_RAW_REG: u32 = 0x08;
/// `USB_SERIAL_JTAG_INT_ST_REG` (RO): `INT_RAW & INT_ENA`.
pub const INT_ST_REG: u32 = 0x0C;
/// `USB_SERIAL_JTAG_INT_ENA_REG` (R/W).
pub const INT_ENA_REG: u32 = 0x10;

/// `USB_SERIAL_JTAG_WR_DONE`, bit 0 of [`EP1_CONF_REG`].
pub const WR_DONE: u32 = 1 << 0;
/// `USB_SERIAL_JTAG_SERIAL_IN_EP_DATA_FREE`, bit 1 of [`EP1_CONF_REG`].
pub const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
/// `USB_SERIAL_JTAG_SERIAL_OUT_EP_DATA_AVAIL`, bit 2 of [`EP1_CONF_REG`].
pub const SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;
/// `USB_SERIAL_JTAG_SERIAL_IN_EMPTY_INT_RAW`, bit 3 of [`INT_RAW_REG`].
pub const SERIAL_IN_EMPTY_INT_RAW: u32 = 1 << 3;
/// `USB_SERIAL_JTAG_INT_CLR_REG` (WT).
pub const INT_CLR_REG: u32 = 0x14;
/// `USB_SERIAL_JTAG_SOF_INT_RAW`, bit 1 of [`INT_RAW_REG`].
pub const SOF_INT_RAW: u32 = 1 << 1;
/// `USB_SERIAL_JTAG_SOF_INT_CLR`, bit 1 of [`INT_CLR_REG`].
pub const SOF_INT_CLR: u32 = 1 << 1;
/// `USB_SERIAL_JTAG_SERIAL_OUT_RECV_PKT_INT_*`, bit 2 of RAW/ST/ENA/CLR:
/// "a packet was received by the OUT endpoint".
pub const SERIAL_OUT_RECV_PKT_INT: u32 = 1 << 2;
/// Full-speed bulk max packet size of the CDC OUT endpoint (the FIFO depth).
pub const OUT_EP_MAX_PACKET: usize = 64;
/// SYSTIMER ticks (16 MHz) one OUT packet occupies on a full-speed bus: a
/// 64-byte DATA packet plus its OUT token and ACK handshake is ~616 bits,
/// ~51.3 us at 12 Mbit/s, x 16 ticks/us = 821. A host cannot send the next
/// packet sooner; see `docs/milestone-5-decisions.md`.
pub const PACKET_TICKS: u64 = 821;
/// Bytes the emulated USB host will buffer before refusing more (a browser
/// must not grow emulator memory without bound).
pub const HOST_QUEUE_CAPACITY: usize = 1 << 20;

/// Raw bits this model holds set on every read (see the module doc).
const FORCED_RAW: u32 = SERIAL_IN_EMPTY_INT_RAW | SOF_INT_RAW;

/// The USB-Serial-JTAG peripheral. See the module doc for what's modeled.
#[derive(Default)]
pub struct UsbSerialJtag {
    /// Plain word storage for every register offset not given real
    /// behavior above, keyed by word-aligned offset.
    regs: HashMap<u32, u32>,
    /// Latched (non-forced) `INT_RAW` bits.
    int_raw: u32,
    /// Bytes the host has buffered and not yet sent as a packet.
    host_queue: VecDeque<u8>,
    /// The OUT endpoint FIFO: the current packet, popped by `EP1_REG` reads.
    out_fifo: VecDeque<u8>,
    /// Ticks left before the host may send another packet.
    ticks_to_next_packet: u64,
}

impl UsbSerialJtag {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues bytes the USB host sends; returns how many were accepted
    /// (fewer than `bytes.len()` once [`HOST_QUEUE_CAPACITY`] is reached).
    pub fn host_send(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(HOST_QUEUE_CAPACITY - self.host_queue.len());
        self.host_queue.extend(&bytes[..n]);
        n
    }

    /// Bytes the firmware has not read yet (host queue plus FIFO).
    pub fn host_pending(&self) -> usize {
        self.host_queue.len() + self.out_fifo.len()
    }

    /// Advances bus time by `ticks` SYSTIMER ticks: the host sends its next
    /// packet once the FIFO is empty and a packet time has passed.
    pub fn advance(&mut self, ticks: u64) {
        self.ticks_to_next_packet = self.ticks_to_next_packet.saturating_sub(ticks);
        if self.ticks_to_next_packet == 0 && self.out_fifo.is_empty() && !self.host_queue.is_empty()
        {
            let n = self.host_queue.len().min(OUT_EP_MAX_PACKET);
            self.out_fifo.extend(self.host_queue.drain(..n));
            self.int_raw |= SERIAL_OUT_RECV_PKT_INT;
            self.ticks_to_next_packet = PACKET_TICKS;
        }
    }

    /// Ticks until [`UsbSerialJtag::advance`] would deliver a packet, or
    /// `None` if none can arrive without the firmware reading first (idle
    /// host, or a non-empty FIFO). Bounds the WFI fast-forward.
    pub fn ticks_until_next_packet(&self) -> Option<u64> {
        if self.host_queue.is_empty() || !self.out_fifo.is_empty() {
            None
        } else {
            Some(self.ticks_to_next_packet.max(1))
        }
    }

    fn int_ena(&self) -> u32 {
        self.regs.get(&INT_ENA_REG).copied().unwrap_or(0)
    }

    fn int_st(&self) -> u32 {
        (self.int_raw | FORCED_RAW) & self.int_ena()
    }

    /// `ETS_USB_SERIAL_JTAG_INTR_SOURCE` as a level while `INT_ST != 0`.
    pub fn pending_sources(&self) -> u64 {
        if self.int_st() != 0 {
            1u64 << SRC_USB_SERIAL_JTAG
        } else {
            0
        }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            // Only lane 0 is RDWR_BYTE; reading it pops one byte.
            EP1_REG if idx == 0 => return self.out_fifo.pop_front().unwrap_or(0),
            EP1_REG => 0,
            EP1_CONF_REG => {
                SERIAL_IN_EP_DATA_FREE
                    | if self.out_fifo.is_empty() {
                        0
                    } else {
                        SERIAL_OUT_EP_DATA_AVAIL
                    }
            }
            INT_RAW_REG => self.int_raw | FORCED_RAW,
            INT_ST_REG => self.int_st(),
            INT_CLR_REG => 0,
            _ => self.regs.get(&word_offset).copied().unwrap_or(0),
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8, console: &mut Console) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            EP1_REG => {
                // Only byte index 0 (USB_SERIAL_JTAG_RDWR_BYTE, bits [7:0])
                // carries the TX byte; the other lanes are ignored.
                if idx == 0 {
                    console.push(val);
                }
            }
            // WR_DONE has nothing to flush; the rest is RO.
            EP1_CONF_REG | INT_ST_REG => {}
            // R/WTC (INT_RAW) and WT (INT_CLR): a 1 clears the raw bit. The
            // forced bits re-assert on the next read.
            INT_RAW_REG | INT_CLR_REG => self.int_raw &= !(u32::from(val) << (8 * idx)),
            _ => {
                let mut word = self.regs.get(&word_offset).copied().unwrap_or(0);
                set_byte(&mut word, idx, val);
                self.regs.insert(word_offset, word);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::console::Console;

    fn write_word(u: &mut UsbSerialJtag, c: &mut Console, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            u.write_byte(off + i as u32, *b, c);
        }
    }
    fn read_word(u: &mut UsbSerialJtag, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| u.read_byte(off + i)))
    }

    #[test]
    fn word_write_to_ep1_pushes_exactly_one_byte() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        write_word(&mut u, &mut c, EP1_REG, 0x41);
        write_word(&mut u, &mut c, EP1_REG, 0x42);
        assert_eq!(c.bytes(), b"AB");
    }

    #[test]
    fn byte_write_to_ep1_pushes_that_byte() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        u.write_byte(EP1_REG, b'x', &mut c);
        assert_eq!(c.bytes(), b"x");
    }

    #[test]
    fn ep1_conf_reports_tx_free_and_no_rx_while_the_host_is_idle() {
        let mut u = UsbSerialJtag::new();
        let conf = read_word(&mut u, EP1_CONF_REG);
        assert_ne!(conf & SERIAL_IN_EP_DATA_FREE, 0);
        assert_eq!(conf & SERIAL_OUT_EP_DATA_AVAIL, 0);
    }

    #[test]
    fn wr_done_write_is_accepted_and_emits_nothing() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        write_word(&mut u, &mut c, EP1_CONF_REG, WR_DONE);
        assert!(c.bytes().is_empty());
    }

    #[test]
    fn other_registers_are_plain_storage() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        write_word(&mut u, &mut c, 0x10, 0xDEAD_BEEF);
        assert_eq!(read_word(&mut u, 0x10), 0xDEAD_BEEF);
    }

    #[test]
    fn int_raw_serial_in_empty_always_reads_set() {
        let mut u = UsbSerialJtag::new();
        let raw = read_word(&mut u, INT_RAW_REG);
        assert_ne!(
            raw & SERIAL_IN_EMPTY_INT_RAW,
            0,
            "TX is modeled as always-empty; interrupt-driven TX drivers must see this bit set"
        );
    }

    /// Milestone 4 Task 4: ESP-IDF's connection monitor
    /// (`usb_serial_jtag_connection_monitor.c`) reads `SOF_INT_RAW` from a
    /// FreeRTOS tick hook, then clears it through `INT_CLR` (a word store,
    /// which reaches the peripheral as four ascending byte writes). A host
    /// sends a SOF every 1 ms, so by the next read the bit is set again; a
    /// clear must never make the badge look unplugged.
    #[test]
    fn sof_int_raw_reads_set_even_after_a_byte_split_int_clr() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        assert_ne!(
            read_word(&mut u, INT_RAW_REG) & SOF_INT_RAW,
            0,
            "before any clear"
        );
        write_word(&mut u, &mut c, INT_CLR_REG, SOF_INT_CLR);
        assert_ne!(
            read_word(&mut u, INT_RAW_REG) & SOF_INT_RAW,
            0,
            "a host keeps sending SOF frames; the next read sees the bit again"
        );
        // A raw-register store (R/WTC on hardware) must not clear it either.
        write_word(&mut u, &mut c, INT_RAW_REG, 0);
        assert_ne!(read_word(&mut u, INT_RAW_REG) & SOF_INT_RAW, 0);
        assert!(c.bytes().is_empty(), "no register store prints a byte");
    }

    fn tick(u: &mut UsbSerialJtag, ticks: u64) {
        u.advance(ticks);
    }

    #[test]
    fn host_bytes_arrive_as_a_64_byte_packet_on_the_next_tick() {
        let mut u = UsbSerialJtag::new();
        let input: Vec<u8> = (0..100u8).collect();
        assert_eq!(u.host_send(&input), 100);
        assert_eq!(
            read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL,
            0,
            "not before a tick"
        );
        tick(&mut u, 1);
        assert_ne!(
            read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL,
            0
        );
        let mut got = Vec::new();
        while read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL != 0 {
            got.push(u.read_byte(EP1_REG));
        }
        assert_eq!(got, (0..64u8).collect::<Vec<_>>(), "one full-speed packet");
        assert_eq!(u.host_pending(), 36);
    }

    #[test]
    fn word_read_of_ep1_pops_exactly_one_byte() {
        let mut u = UsbSerialJtag::new();
        u.host_send(b"ab");
        tick(&mut u, 1);
        assert_eq!(
            read_word(&mut u, EP1_REG),
            u32::from(b'a'),
            "lane 0 is RDWR_BYTE; lanes 1..3 read 0"
        );
        assert_eq!(read_word(&mut u, EP1_REG), u32::from(b'b'));
        assert_eq!(read_word(&mut u, EP1_REG), 0, "empty FIFO reads 0");
        assert_eq!(u.host_pending(), 0);
    }

    #[test]
    fn next_packet_waits_for_an_empty_fifo_and_one_packet_time() {
        let mut u = UsbSerialJtag::new();
        u.host_send(&[7u8; 130]);
        tick(&mut u, 1);
        for _ in 0..64 {
            u.read_byte(EP1_REG);
        }
        // The first packet loaded at the tick above; the next is due
        // PACKET_TICKS after it.
        tick(&mut u, PACKET_TICKS - 1);
        assert_eq!(
            read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL,
            0,
            "packet time not elapsed"
        );
        tick(&mut u, 1);
        assert_ne!(
            read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL,
            0
        );
        // A full FIFO blocks the next packet however long the host waits.
        tick(&mut u, 10 * PACKET_TICKS);
        assert_eq!(u.host_pending(), 130 - 64, "FIFO (64) + queue (2) unread");
    }

    #[test]
    fn recv_pkt_raw_is_set_per_packet_and_cleared_by_a_byte_split_int_clr() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        assert_eq!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        u.host_send(b"x");
        tick(&mut u, 1);
        assert_ne!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        write_word(&mut u, &mut c, INT_CLR_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        // Writing 1 to INT_RAW (R/WTC) also clears; writing 0 does not.
        u.host_send(b"y");
        u.read_byte(EP1_REG);
        tick(&mut u, PACKET_TICKS);
        write_word(&mut u, &mut c, INT_RAW_REG, 0);
        assert_ne!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        write_word(&mut u, &mut c, INT_RAW_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        assert_eq!(read_word(&mut u, INT_CLR_REG), 0, "INT_CLR is write-only");
    }

    #[test]
    fn int_st_is_raw_masked_by_ena_and_drives_the_interrupt_source() {
        use crate::mem::soc::SRC_USB_SERIAL_JTAG;
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        // Nothing enabled: forced SOF/IN_EMPTY raw bits do not reach INT_ST.
        assert_eq!(read_word(&mut u, INT_ST_REG), 0);
        assert_eq!(u.pending_sources(), 0);
        write_word(&mut u, &mut c, INT_ENA_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(
            read_word(&mut u, INT_ENA_REG),
            SERIAL_OUT_RECV_PKT_INT,
            "ENA reads back"
        );
        assert_eq!(u.pending_sources(), 0, "enabled but not raised");
        u.host_send(b"z");
        tick(&mut u, 1);
        assert_eq!(read_word(&mut u, INT_ST_REG), SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(u.pending_sources(), 1u64 << SRC_USB_SERIAL_JTAG);
        write_word(&mut u, &mut c, INT_CLR_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(u.pending_sources(), 0, "level drops when cleared");
        // The TX path: enabling SERIAL_IN_EMPTY asserts at once (TX is always empty).
        write_word(&mut u, &mut c, INT_ENA_REG, SERIAL_IN_EMPTY_INT_RAW);
        assert_eq!(u.pending_sources(), 1u64 << SRC_USB_SERIAL_JTAG);
        write_word(&mut u, &mut c, INT_ENA_REG, 0);
        assert_eq!(u.pending_sources(), 0);
    }

    #[test]
    fn ticks_until_next_packet_is_none_when_nothing_can_arrive() {
        let mut u = UsbSerialJtag::new();
        assert_eq!(u.ticks_until_next_packet(), None, "idle host");
        u.host_send(&[1u8; 65]);
        assert_eq!(u.ticks_until_next_packet(), Some(1));
        tick(&mut u, 1);
        assert_eq!(
            u.ticks_until_next_packet(),
            None,
            "FIFO full: waits on the firmware, not on time"
        );
        for _ in 0..64 {
            u.read_byte(EP1_REG);
        }
        assert_eq!(
            u.ticks_until_next_packet(),
            Some(PACKET_TICKS),
            "a packet time after the last load"
        );
    }

    #[test]
    fn host_queue_is_capped_and_refuses_the_excess() {
        let mut u = UsbSerialJtag::new();
        let big = vec![0u8; HOST_QUEUE_CAPACITY + 12_345];
        assert_eq!(u.host_send(&big), HOST_QUEUE_CAPACITY);
        assert_eq!(u.host_send(b"more"), 0);
        assert_eq!(u.host_pending(), HOST_QUEUE_CAPACITY);
        // Non-UTF-8 and NUL bytes are just bytes.
        let mut v = UsbSerialJtag::new();
        v.host_send(&[0x00, 0xff, 0xfe]);
        tick(&mut v, 1);
        assert_eq!(
            [
                v.read_byte(EP1_REG),
                v.read_byte(EP1_REG),
                v.read_byte(EP1_REG)
            ],
            [0x00, 0xff, 0xfe]
        );
    }
}

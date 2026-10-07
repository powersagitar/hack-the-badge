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
//!   USB-CDC TX FIFO); reads return 0 (real hardware: pops an RX byte —
//!   this emulator models no host-to-badge input, so there is never
//!   anything to pop).
//! - [`EP1_CONF_REG`] (`USB_SERIAL_JTAG_EP1_CONF_REG`, +0x04): bit 0
//!   `WR_DONE` (WT, "commit the FIFO write") is a write-trigger flush this
//!   emulator has nothing to flush (each `EP1_REG` write already lands
//!   immediately) — accepted and ignored. Bit 1
//!   `SERIAL_IN_EP_DATA_FREE` (RO, header default 1) always reads 1: TX is
//!   modeled as having infinite room, so a driver polling "can I write
//!   more" never blocks. Bit 2 `SERIAL_OUT_EP_DATA_AVAIL` (RO, header
//!   default 0) always reads 0: no RX data is ever available, matching
//!   `EP1_REG`'s read-as-0 above.
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
//! - Every other offset (`CONF0_REG`, `MISC_CONF_REG`, the various `*_ST`
//!   status registers, interrupt enable/clear, …): plain read/write word
//!   storage, keyed by word-aligned offset in [`UsbSerialJtag::regs`], so a
//!   driver that pokes at USB-Serial-JTAG's interrupt/config plumbing
//!   during setup doesn't get lost (reads back whatever was last written,
//!   defaulting to 0) without this module needing to name every one of
//!   them individually.
//!
//! This mirrors the pattern `crate::peripherals::intc` used for its MAP
//! registers until Milestone 3 Task 4 (concrete named registers get real
//! behavior; everything else in the peripheral's address window is a
//! generic word-storage `HashMap`) rather than introducing a new idiom.

use std::collections::HashMap;

use super::console::Console;
use super::set_byte;

/// `USB_SERIAL_JTAG_EP1_REG`. See the module doc.
pub const EP1_REG: u32 = 0x00;
/// `USB_SERIAL_JTAG_EP1_CONF_REG`. See the module doc.
pub const EP1_CONF_REG: u32 = 0x04;
/// `USB_SERIAL_JTAG_INT_RAW_REG`. See the module doc.
pub const INT_RAW_REG: u32 = 0x08;

/// `USB_SERIAL_JTAG_WR_DONE`, bit 0 of [`EP1_CONF_REG`].
pub const WR_DONE: u32 = 1 << 0;
/// `USB_SERIAL_JTAG_SERIAL_IN_EP_DATA_FREE`, bit 1 of [`EP1_CONF_REG`].
pub const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
/// `USB_SERIAL_JTAG_SERIAL_OUT_EP_DATA_AVAIL`, bit 2 of [`EP1_CONF_REG`].
pub const SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;
/// `USB_SERIAL_JTAG_SERIAL_IN_EMPTY_INT_RAW`, bit 3 of [`INT_RAW_REG`].
pub const SERIAL_IN_EMPTY_INT_RAW: u32 = 1 << 3;
/// `USB_SERIAL_JTAG_INT_CLR_REG`. Plain storage (see the module doc).
pub const INT_CLR_REG: u32 = 0x14;
/// `USB_SERIAL_JTAG_SOF_INT_RAW`, bit 1 of [`INT_RAW_REG`].
pub const SOF_INT_RAW: u32 = 1 << 1;
/// `USB_SERIAL_JTAG_SOF_INT_CLR`, bit 1 of [`INT_CLR_REG`].
pub const SOF_INT_CLR: u32 = 1 << 1;

/// The USB-Serial-JTAG peripheral. See the module doc for what's modeled.
#[derive(Default)]
pub struct UsbSerialJtag {
    /// Plain word storage for every register offset not given real
    /// behavior above, keyed by word-aligned offset.
    regs: HashMap<u32, u32>,
}

impl UsbSerialJtag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            EP1_REG => 0,
            EP1_CONF_REG => SERIAL_IN_EP_DATA_FREE,
            INT_RAW_REG => {
                self.regs.get(&word_offset).copied().unwrap_or(0)
                    | SERIAL_IN_EMPTY_INT_RAW
                    | SOF_INT_RAW
            }
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
                // carries the TX byte; the other three byte lanes of this
                // word are unused padding on real hardware and ignored
                // here.
                if idx == 0 {
                    console.push(val);
                }
            }
            EP1_CONF_REG => {
                // WR_DONE is a write-trigger flush this emulator has
                // nothing to flush; SERIAL_IN_EP_DATA_FREE/
                // SERIAL_OUT_EP_DATA_AVAIL are RO on real hardware. All
                // writes to this register are accepted and ignored.
            }
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
    fn ep1_conf_always_reports_tx_free_and_no_rx() {
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
}

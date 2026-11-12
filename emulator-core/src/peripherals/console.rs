//! Capped byte sink for what the firmware prints. Its only feed is the
//! USB-Serial-JTAG EP1 TX FIFO (`usb_serial_jtag.rs`), which both firmware
//! code and the ROM `ets_printf` HLE stub (`cpu/rom_stubs.rs`, sink address
//! `0x6004_3000`) write to; UART0 and the ROM putc functions are not
//! modeled, so they feed nothing. Diagnostic output, not device state:
//! capped so a firmware stuck in a print loop can't grow host memory
//! without bound.
use std::collections::VecDeque;

pub const CONSOLE_CAPACITY: usize = 256 * 1024;

#[derive(Clone, Default)]
pub struct Console {
    buf: VecDeque<u8>,
    /// Bytes ever pushed, including those the cap has since dropped.
    written: u64,
}

impl Console {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, b: u8) {
        if self.buf.len() >= CONSOLE_CAPACITY {
            self.buf.pop_front();
        }
        self.buf.push_back(b);
        self.written += 1;
    }

    /// Bytes ever pushed since construction: an absolute offset into the
    /// console's output that the ring's cap never resets. The ring holds
    /// the bytes from `written() - bytes().len()` up to `written()`. It
    /// survives a software reset with the console; a cold reset builds a
    /// new console, so it starts at 0 again.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Owned copy (the ring may be non-contiguous); diagnostic use only.
    pub fn bytes(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }

    /// Lossy UTF-8 decode of [`Console::bytes`] — firmware console output is
    /// not guaranteed to be valid UTF-8 (e.g. a torn multi-byte write), so
    /// invalid sequences become U+FFFD rather than this panicking/erroring.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushes_bytes_and_renders_lossy_text() {
        let mut c = Console::new();
        for b in b"I (5) boot: hi\n" {
            c.push(*b);
        }
        assert_eq!(c.text(), "I (5) boot: hi\n");
    }

    #[test]
    fn caps_at_capacity_dropping_oldest() {
        let mut c = Console::new();
        for i in 0..(CONSOLE_CAPACITY + 10) {
            c.push((i % 251) as u8);
        }
        assert_eq!(c.bytes().len(), CONSOLE_CAPACITY);
        assert_eq!(c.bytes()[0], 10u8, "oldest 10 bytes dropped");
    }

    #[test]
    fn written_counts_past_the_cap() {
        let mut c = Console::new();
        assert_eq!(c.written(), 0);
        for i in 0..(CONSOLE_CAPACITY + 10) {
            c.push((i % 251) as u8);
        }
        assert_eq!(c.written(), CONSOLE_CAPACITY as u64 + 10);
        assert_eq!(c.bytes().len(), CONSOLE_CAPACITY);
    }
}

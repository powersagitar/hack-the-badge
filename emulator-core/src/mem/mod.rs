//! Memory/bus interface shared between the CPU core and whatever backs
//! memory-mapped addresses.
//!
//! This module intentionally contains *only* the [`Bus`] trait. A later task
//! builds the real flash/RAM/MMIO-backed implementation of this trait (plus
//! the ESP32-C3 memory map); the CPU core in `crate::cpu` only needs the
//! trait shape to compile and be testable in isolation, via a trivial test
//! double (see `crate::cpu`'s test modules for an example: a flat
//! `Vec<u8>`-backed `Bus`).

/// A byte-addressable, 32-bit-address memory/peripheral bus.
///
/// All accesses take `&mut self` (including reads) since a real
/// implementation may need to log/trace accesses or trigger side effects on
/// memory-mapped I/O reads (e.g. a UART RX FIFO pop). Multi-byte accesses are
/// little-endian, matching the RISC-V standard and the ESP32-C3's native
/// endianness.
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, val: u8);
    fn write16(&mut self, addr: u32, val: u16);
    fn write32(&mut self, addr: u32, val: u32);
}

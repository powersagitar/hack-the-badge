//! Memory/bus interface shared between the CPU core and whatever backs
//! memory-mapped addresses.
//!
//! [`Bus`] itself is the trait Task 1's CPU core is generic over (the CPU
//! core in `crate::cpu` only needs the trait shape to compile and be
//! testable in isolation, via a trivial test double — see `crate::cpu`'s
//! test modules for an example: a flat `Vec<u8>`-backed `Bus`). The
//! submodules here build the real ESP32-C3 memory map on top of it:
//! - [`image`]: parser for the ESP-IDF app image format (header + segment
//!   table) bundled in `public/firmware/factory.bin`.
//! - [`soc`]: the ESP32-C3 address-space regions used to categorize each
//!   segment as flash-mapped (XIP) vs. RAM-copied.
//! - [`bus`]: [`bus::FirmwareBus`], the concrete `Bus` implementation that
//!   ties the above together (plus a never-panics catch-all for
//!   not-yet-modeled peripheral MMIO).

/// A byte-addressable, 32-bit-address memory/peripheral bus.
///
/// All accesses take `&mut self` (including reads) since a real
/// implementation may need to log/trace accesses or trigger side effects on
/// memory-mapped I/O reads (e.g. a UART RX FIFO pop). Multi-byte accesses are
/// little-endian, matching the RISC-V standard and the ESP32-C3's native
/// endianness.
pub mod bus;
pub mod image;
pub mod soc;

pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, val: u8);
    fn write16(&mut self, addr: u32, val: u16);
    fn write32(&mut self, addr: u32, val: u32);

    /// Fetches 16 bits at `addr` for **instruction fetch**, distinct from
    /// [`Bus::read16`]. Returns `Some(value)` if `addr` is genuinely
    /// executable/mapped; `None` if it would otherwise fall through to a
    /// never-panic catch-all (unmapped space, or not-yet-modeled peripheral
    /// MMIO that a real CPU cannot execute out of).
    ///
    /// This is intentionally a separate method from `read16`, not a
    /// replacement: ordinary *data* loads through `read16`/`read8`/`read32`
    /// must keep returning `0` for unmapped addresses (never trap) since
    /// that's correct behavior for MMIO probes firmware makes before later
    /// tasks implement the relevant peripheral. Only the CPU core's
    /// instruction-fetch path should call `fetch16`, and it should treat
    /// `None` as an instruction-access-fault condition rather than decoding
    /// a fabricated value.
    fn fetch16(&mut self, addr: u32) -> Option<u16>;
}

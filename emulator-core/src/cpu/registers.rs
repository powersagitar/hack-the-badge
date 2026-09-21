//! Register file: 32 general-purpose registers, `pc`, and the minimal M-mode
//! CSR set needed for trap handling.

/// The 32 general-purpose integer registers plus the program counter.
///
/// `x0` is hardwired to zero: [`Registers::write`] silently discards writes
/// to it, and [`Registers::read`] always returns 0 for it, matching the
/// RISC-V spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Registers {
    x: [u32; 32],
    pub pc: u32,
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn read(&self, reg: u8) -> u32 {
        debug_assert!(reg < 32);
        self.x[reg as usize]
    }

    #[inline]
    pub fn write(&mut self, reg: u8, val: u32) {
        debug_assert!(reg < 32);
        if reg != 0 {
            self.x[reg as usize] = val;
        }
    }
}

/// Standard M-mode CSR addresses (RISC-V privileged spec, "Machine-Level
/// CSRs" chapter). Only the CSRs this core implements are listed.
pub mod csr_addr {
    pub const MSTATUS: u16 = 0x300;
    pub const MIE: u16 = 0x304;
    pub const MTVEC: u16 = 0x305;
    pub const MSCRATCH: u16 = 0x340;
    pub const MEPC: u16 = 0x341;
    pub const MCAUSE: u16 = 0x342;
    pub const MTVAL: u16 = 0x343;
    pub const MIP: u16 = 0x344;
}

/// `mstatus` bit positions this core actually models.
pub mod mstatus_bits {
    /// Machine Interrupt Enable.
    pub const MIE: u32 = 1 << 3;
    /// Machine Previous Interrupt Enable.
    pub const MPIE: u32 = 1 << 7;
    /// Machine Previous Privilege (2 bits). This core is M-mode only, so
    /// this field is always `0b11` in practice, but we still model it for
    /// spec fidelity (`mret` restores privilege from here).
    pub const MPP_MASK: u32 = 0b11 << 11;
    pub const MPP_M: u32 = 0b11 << 11;
}

/// Standard `mcause` exception codes (RISC-V privileged spec). Only the
/// codes this core actually raises are listed; the field width in `mcause`
/// is intentionally left able to hold any value an external caller passes
/// to `Cpu::raise_interrupt`, since a later ESP32-C3-specific interrupt
/// controller owns the mapping of its own peripherals to cause numbers.
pub mod exception_code {
    pub const ILLEGAL_INSTRUCTION: u32 = 2;
    pub const BREAKPOINT: u32 = 3;
    pub const ENVIRONMENT_CALL_FROM_M_MODE: u32 = 11;
}

/// The minimal M-mode CSR set needed for trap handling: `mstatus`, `mie`,
/// `mip`, `mtvec`, `mepc`, `mcause`, `mtval`, `mscratch`. Modeled as plain
/// `u32` fields — no per-bit semantics beyond what trap-entry/`mret` need.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Csrs {
    pub mstatus: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtvec: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mscratch: u32,
}

impl Csrs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a CSR by its 12-bit address. CSR addresses this core doesn't
    /// implement read as 0 (rather than panicking or trapping), so that
    /// unrelated CSR probes made by real-world startup code (e.g.
    /// `mhartid`, `misa`) don't crash the emulator before a later task
    /// fills in the real ESP32-C3 CSR set. This is a documented,
    /// intentional simplification — see the module-level report.
    pub fn read(&self, addr: u16) -> u32 {
        match addr {
            csr_addr::MSTATUS => self.mstatus,
            csr_addr::MIE => self.mie,
            csr_addr::MIP => self.mip,
            csr_addr::MTVEC => self.mtvec,
            csr_addr::MEPC => self.mepc,
            csr_addr::MCAUSE => self.mcause,
            csr_addr::MTVAL => self.mtval,
            csr_addr::MSCRATCH => self.mscratch,
            _ => 0,
        }
    }

    /// Writes a CSR by its 12-bit address. Writes to unimplemented CSR
    /// addresses are silently discarded (see [`Csrs::read`]'s doc comment).
    pub fn write(&mut self, addr: u16, val: u32) {
        match addr {
            csr_addr::MSTATUS => self.mstatus = val,
            csr_addr::MIE => self.mie = val,
            csr_addr::MIP => self.mip = val,
            csr_addr::MTVEC => self.mtvec = val,
            csr_addr::MEPC => self.mepc = val,
            csr_addr::MCAUSE => self.mcause = val,
            csr_addr::MTVAL => self.mtval = val,
            csr_addr::MSCRATCH => self.mscratch = val,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x0_reads_zero_and_ignores_writes() {
        let mut r = Registers::new();
        r.write(0, 0xdead_beef);
        assert_eq!(r.read(0), 0);
    }

    #[test]
    fn other_registers_round_trip() {
        let mut r = Registers::new();
        r.write(5, 42);
        assert_eq!(r.read(5), 42);
    }

    #[test]
    fn unimplemented_csr_reads_zero_and_ignores_writes() {
        let mut c = Csrs::new();
        c.write(0xf14, 0x1234); // mhartid, not implemented
        assert_eq!(c.read(0xf14), 0);
    }

    #[test]
    fn implemented_csrs_round_trip() {
        let mut c = Csrs::new();
        c.write(csr_addr::MTVEC, 0x1000);
        c.write(csr_addr::MSCRATCH, 0xabcd);
        assert_eq!(c.read(csr_addr::MTVEC), 0x1000);
        assert_eq!(c.read(csr_addr::MSCRATCH), 0xabcd);
    }
}

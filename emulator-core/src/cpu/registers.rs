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
    pub const INSTRUCTION_ACCESS_FAULT: u32 = 1;
    pub const ILLEGAL_INSTRUCTION: u32 = 2;
    pub const BREAKPOINT: u32 = 3;
    pub const ENVIRONMENT_CALL_FROM_M_MODE: u32 = 11;
}

/// Where a chip puts its cycle counter in CSR space: the counter itself
/// and up to [`CycleCounterCsrs::CONTROL_COUNT`] companion control CSRs
/// (event select, mode/enable) that read back what was written. Supplied
/// by chip setup ([`Csrs::set_cycle_counter`]); the core holds no chip
/// addresses itself. `crate::mem::soc::ESP32C3_CYCLE_COUNTER` is the
/// ESP32-C3's (`mpccr` `0x7E2`, with `mpcer` `0x7E0` and `mpcmr` `0x7E1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleCounterCsrs {
    /// The counter CSR: reads the count, a write sets it.
    pub counter: u16,
    /// Control CSRs, stored as written and read back. They do **not** gate
    /// counting: the counter counts on every retired step (see
    /// [`CycleCounter`]).
    pub controls: [u16; Self::CONTROL_COUNT],
}

impl CycleCounterCsrs {
    /// How many control CSRs a counter carries.
    pub const CONTROL_COUNT: usize = 2;
}

/// A free-running cycle counter in CSR space (Milestone 5 Task D-M5-3).
///
/// It advances by one on every [`crate::cpu::Cpu::step`] that does work:
/// an instruction, a ROM-stub interception or a trap entry. A step parked
/// in `WFI` with nothing pending does not count, and neither do the steps
/// the driving loop fast-forwards over while the core waits
/// (`crate::boot::step_with_interrupts`). One count per step matches the
/// emulator's timing model (`TICKS_PER_STEP = 1`: one instruction per
/// 16 MHz SYSTIMER tick); there is no per-instruction cycle model.
///
/// The count advances *before* the step's instruction executes, so a CSR
/// write to the counter is what the next instruction reads (RISC-V
/// privileged spec v1.12, section 3.1.11: a write to a counter is seen
/// after the writing instruction's own increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleCounter {
    csrs: CycleCounterCsrs,
    count: u32,
    controls: [u32; CycleCounterCsrs::CONTROL_COUNT],
}

impl CycleCounter {
    fn new(csrs: CycleCounterCsrs) -> Self {
        Self {
            csrs,
            count: 0,
            controls: [0; CycleCounterCsrs::CONTROL_COUNT],
        }
    }

    fn read(&self, addr: u16) -> Option<u32> {
        if addr == self.csrs.counter {
            return Some(self.count);
        }
        let i = self.csrs.controls.iter().position(|&a| a == addr)?;
        Some(self.controls[i])
    }

    /// `true` if `addr` belongs to this counter (and the write was taken).
    fn write(&mut self, addr: u16, val: u32) -> bool {
        if addr == self.csrs.counter {
            self.count = val;
            return true;
        }
        match self.csrs.controls.iter().position(|&a| a == addr) {
            Some(i) => {
                self.controls[i] = val;
                true
            }
            None => false,
        }
    }
}

/// The minimal M-mode CSR set needed for trap handling: `mstatus`, `mie`,
/// `mip`, `mtvec`, `mepc`, `mcause`, `mtval`, `mscratch`. Modeled as plain
/// `u32` fields — no per-bit semantics beyond what trap-entry/`mret` need.
/// Plus an optional chip-placed [`CycleCounter`].
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
    /// `None` unless chip setup installs one ([`Csrs::set_cycle_counter`]).
    cycle_counter: Option<CycleCounter>,
}

impl Csrs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a cycle counter at `csrs`' addresses, starting at 0 with
    /// zeroed control CSRs (replacing any previous one).
    pub fn set_cycle_counter(&mut self, csrs: CycleCounterCsrs) {
        self.cycle_counter = Some(CycleCounter::new(csrs));
    }

    /// Advances the cycle counter (if installed) by one, wrapping at 2^32.
    pub fn tick_cycle_counter(&mut self) {
        if let Some(c) = self.cycle_counter.as_mut() {
            c.count = c.count.wrapping_add(1);
        }
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
            _ => self.cycle_counter.and_then(|c| c.read(addr)).unwrap_or(0),
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
            _ => {
                if let Some(c) = self.cycle_counter.as_mut() {
                    c.write(addr, val);
                }
            }
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

    const TEST_COUNTER: CycleCounterCsrs = CycleCounterCsrs {
        counter: 0x7e2,
        controls: [0x7e0, 0x7e1],
    };

    #[test]
    fn without_a_counter_its_csrs_stay_unimplemented() {
        let mut c = Csrs::new();
        c.tick_cycle_counter();
        c.write(0x7e2, 5);
        assert_eq!(c.read(0x7e2), 0);
    }

    #[test]
    fn cycle_counter_counts_ticks_and_wraps() {
        let mut c = Csrs::new();
        c.set_cycle_counter(TEST_COUNTER);
        assert_eq!(c.read(0x7e2), 0);
        for _ in 0..3 {
            c.tick_cycle_counter();
        }
        assert_eq!(c.read(0x7e2), 3);
        c.write(0x7e2, u32::MAX);
        c.tick_cycle_counter();
        assert_eq!(c.read(0x7e2), 0, "wraps at 2^32");
    }

    #[test]
    fn cycle_counter_write_sets_the_count() {
        let mut c = Csrs::new();
        c.set_cycle_counter(TEST_COUNTER);
        c.tick_cycle_counter();
        c.write(0x7e2, 1000);
        assert_eq!(c.read(0x7e2), 1000);
        c.tick_cycle_counter();
        assert_eq!(c.read(0x7e2), 1001);
    }

    #[test]
    fn control_csrs_read_back_and_do_not_gate_counting() {
        let mut c = Csrs::new();
        c.set_cycle_counter(TEST_COUNTER);
        c.write(0x7e0, 0);
        c.write(0x7e1, 0);
        c.tick_cycle_counter();
        assert_eq!(c.read(0x7e2), 1, "counts with the controls cleared");
        c.write(0x7e0, 1);
        c.write(0x7e1, 0x5);
        assert_eq!((c.read(0x7e0), c.read(0x7e1)), (1, 5));
        assert_eq!(c.read(0x7e3), 0, "a neighbour stays unimplemented");
        assert_eq!(c.read(csr_addr::MSTATUS), 0, "standard CSRs unaffected");
    }
}

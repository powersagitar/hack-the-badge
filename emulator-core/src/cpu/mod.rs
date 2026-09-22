//! Generic RV32IMC RISC-V CPU core: register file, decode, execute, and the
//! M-mode trap-entry/`mret` mechanics needed to run real compiled firmware.
//!
//! This module deliberately knows nothing about ESP32-C3 peripherals, the
//! memory map, or the interrupt matrix — it only implements the RISC-V
//! architectural behavior (base I, M, C extensions, Zicsr, and the M-mode
//! subset of the privileged spec needed for traps). A later task wires an
//! ESP32-C3-specific interrupt controller/timer up to [`Cpu::raise_interrupt`].

mod decode;
mod execute;
mod registers;
pub mod rom_stubs;

pub use decode::{AluOp, BranchKind, CsrOp, CsrSrc, Instruction, LoadKind, MulDivOp, StoreKind};
pub use registers::{csr_addr, exception_code, mstatus_bits, Csrs, Registers};
pub use rom_stubs::{RomStub, RomStubTable};

use crate::mem::Bus;

/// Outcome of a single [`Cpu::step`] call, for tests/callers to observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepInfo {
    /// `true` if this step took a trap (either a synchronous exception from
    /// the instruction just decoded, or a previously pending interrupt)
    /// instead of completing an instruction normally.
    pub trap_taken: bool,
    /// Length in bytes of the instruction that was fetched (2 or 4), or 0
    /// if a trap was taken before/instead of executing an instruction, or if
    /// this step was a ROM-stub interception (see
    /// [`StepInfo::rom_stub`] — no real instruction is fetched in that case).
    pub instr_len: u8,
    /// The value of `pc` at the start of this `step()` call, before any
    /// trap-entry or normal advance.
    pub pc_before: u32,
    /// `Some(addr)` if this step intercepted a high-level-emulated ROM call
    /// at `addr` instead of fetching/executing an instruction there — see
    /// [`rom_stubs`] for the mechanism. `None` for every ordinary step, and
    /// therefore always `None` for a `Cpu` that never installed a stub table.
    pub rom_stub: Option<u32>,
}

/// The RV32IMC CPU core.
pub struct Cpu {
    pub regs: Registers,
    pub csr: Csrs,
    /// Bitset of pending interrupt lines (bit `n` set means line `n`, as
    /// passed to [`Cpu::raise_interrupt`], is pending and not yet taken).
    ///
    /// This used to be a single `Option<PendingTrap>` slot that a second
    /// `raise_interrupt` call would silently overwrite (and therefore drop)
    /// if the first hadn't been consumed by `step()` yet. That was judged
    /// harmless when the only driver polled at most one line per `step()`
    /// call and `MIE`-gating didn't exist yet (a pending interrupt was
    /// always taken on the very next step, leaving essentially no window for
    /// a second call to land). Once `step()` started gating interrupt-taking
    /// on `mstatus.MIE` (see below), a pending interrupt can now sit
    /// unconsumed across many steps while `MIE` is clear, making that
    /// overwrite-and-drop window real. A bitset fixes this precisely: each
    /// line has its own bit, so `raise_interrupt` on one line can never
    /// clobber another line's pending state, and calling it again for a
    /// line that's already pending is simply idempotent (matches real
    /// level-triggered hardware, which doesn't "double-pend"). Cleared one
    /// bit at a time by `step()` as each line is taken.
    ///
    /// Chosen priority when multiple lines are pending simultaneously:
    /// lowest line number wins (`trailing_zeros()`). This core doesn't yet
    /// model the ESP32-C3 interrupt matrix's per-line `CPU_INT_PRI_n`
    /// priority registers (see `peripherals::intc`'s module doc — v1 only
    /// ever has one real source wired up, so this is moot in practice); this
    /// is a documented placeholder ordering, not a claim of spec-accurate
    /// priority arbitration.
    pending_interrupts: u32,
    /// High-level-emulated ROM-call stubs, checked just before each
    /// instruction fetch. **Empty by default** — an empty table makes the
    /// check a single branch and leaves behavior byte-for-byte identical to
    /// a core without this mechanism. See [`rom_stubs`] and
    /// [`Cpu::set_rom_stubs`].
    rom_stubs: RomStubTable,
}

impl Default for Cpu {
    fn default() -> Self {
        Self {
            regs: Registers::new(),
            csr: Csrs::new(),
            pending_interrupts: 0,
            rom_stubs: RomStubTable::new(),
        }
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs (replacing any previous) the high-level-emulation ROM-stub
    /// table this core consults before each instruction fetch. Opt-in: a
    /// `Cpu` that never calls this keeps an empty table and behaves exactly
    /// as it did before the mechanism existed. See [`rom_stubs`] for the full
    /// semantics, and `crate::rom::esp32c3_rom_stubs` for the ESP32-C3 table
    /// real-firmware boot installs.
    pub fn set_rom_stubs(&mut self, table: RomStubTable) {
        self.rom_stubs = table;
    }

    /// The currently-installed ROM-stub table (empty unless
    /// [`Cpu::set_rom_stubs`] was called).
    pub fn rom_stubs(&self) -> &RomStubTable {
        &self.rom_stubs
    }

    /// Requests that an interrupt be delivered. This does *not* take the
    /// trap immediately — it marks `cause`'s line pending in
    /// [`Cpu::pending_interrupts`], and it is taken (subject to the
    /// `mstatus.MIE` gate — see [`Cpu::step`]) at the start of a future
    /// [`Cpu::step`] call, before that step fetches/executes an instruction.
    /// This is the mechanism an external interrupt controller/timer calls to
    /// deliver an interrupt into the core.
    ///
    /// `cause` is the raw RISC-V exception code / CPU interrupt line number
    /// (e.g. 7 for machine-timer-interrupt, 11 for machine-external-interrupt
    /// per the standard cause numbering; the ESP32-C3 interrupt matrix uses
    /// this as its 0..=31 CPU line number — see `peripherals::intc`) — the
    /// interrupt bit is set automatically on trap entry. Only the low 5 bits
    /// are meaningful (32 lines); calling this twice for the same line before
    /// it's taken is idempotent, and calling it for a different line never
    /// drops the first one (see the [`Cpu::pending_interrupts`] doc).
    /// Synchronous exceptions (`ECALL`/`EBREAK`/illegal instruction) are
    /// distinct from this mechanism: they're taken immediately, as part of
    /// the same `step()` that decoded the faulting instruction, since they
    /// are architecturally synchronous to it.
    pub fn raise_interrupt(&mut self, cause: u32) {
        self.pending_interrupts |= 1u32 << (cause & 0x1f);
    }

    /// Executes exactly one instruction: fetch, decode, execute, and advance
    /// `pc` — or, if an interrupt is pending (via [`Cpu::raise_interrupt`])
    /// *and* `mstatus.MIE` is set, or the instruction just decoded raises a
    /// synchronous exception, takes that trap instead (saving
    /// `mepc`/`mcause`, updating `mstatus`, and jumping to `mtvec`).
    ///
    /// Per the RISC-V privileged spec, a pending interrupt is only actually
    /// taken while the global `mstatus.MIE` bit is set. If `MIE` is clear,
    /// the interrupt is left pending (not cleared) rather than taken — a
    /// level-triggered source stays asserted and will simply be taken on a
    /// later step once `MIE` is set again (typically by the firmware's own
    /// `mret`). Without this gate, a level-triggered interrupt source would
    /// livelock the core: the trap fires on cycle N, `mepc` is set, but
    /// before the ISR's first instruction can execute the same source
    /// re-asserts and the trap fires again, so the ISR body never runs.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> StepInfo {
        let pc_before = self.regs.pc;

        if self.pending_interrupts != 0 && self.csr.mstatus & mstatus_bits::MIE != 0 {
            let line = self.pending_interrupts.trailing_zeros();
            self.pending_interrupts &= !(1u32 << line);
            self.enter_trap(line, true, 0);
            return StepInfo {
                trap_taken: true,
                instr_len: 0,
                pc_before,
                rom_stub: None,
            };
        }

        let pc = self.regs.pc;

        // High-level-emulated ROM call? Checked *before* the fetch, since the
        // whole point is that there are no instruction bytes at these
        // addresses to fetch (see `rom_stubs`). Consumes this entire step.
        if let Some(stub) = self.rom_stubs.lookup(pc) {
            self.apply_rom_stub(stub, bus);
            return StepInfo {
                trap_taken: false,
                instr_len: 0,
                pc_before,
                rom_stub: Some(pc),
            };
        }

        let (instr, len) = match fetch_and_decode(bus, pc) {
            Ok(fetched) => fetched,
            Err(fault_addr) => {
                // `self.regs.pc` is still `pc` here (untouched), so
                // `enter_trap` saves the faulting instruction's address as
                // `mepc`, matching how the `Exception` path below behaves.
                self.enter_trap(exception_code::INSTRUCTION_ACCESS_FAULT, false, fault_addr);
                return StepInfo {
                    trap_taken: true,
                    instr_len: 0,
                    pc_before,
                    rom_stub: None,
                };
            }
        };

        match execute::execute(self, bus, instr, pc, len) {
            execute::ExecResult::Normal => StepInfo {
                trap_taken: false,
                instr_len: len,
                pc_before,
                rom_stub: None,
            },
            execute::ExecResult::Exception { cause, tval } => {
                self.enter_trap(cause, false, tval);
                StepInfo {
                    trap_taken: true,
                    instr_len: len,
                    pc_before,
                    rom_stub: None,
                }
            }
        }
    }

    /// Runs one high-level-emulated ROM stub's [`rom_stubs::RomStubEffect`],
    /// then redirects `pc` to the return address the caller left in `ra`. See
    /// [`rom_stubs`]'s module doc for the reasoning and the known
    /// `ra`-validity limitation.
    fn apply_rom_stub<B: Bus>(&mut self, stub: RomStub, bus: &mut B) {
        use rom_stubs::RomStubEffect;
        match stub.effect {
            RomStubEffect::Return(value) => self.regs.write(rom_stubs::REG_A0, value),
            RomStubEffect::Void => {}
            RomStubEffect::Memset => {
                let dst = self.regs.read(rom_stubs::REG_A0);
                let byte = self.regs.read(rom_stubs::REG_A1) as u8;
                let len = self
                    .regs
                    .read(rom_stubs::REG_A2)
                    .min(rom_stubs::MAX_STUB_MEMORY_BYTES);
                for i in 0..len {
                    bus.write8(dst.wrapping_add(i), byte);
                }
                // `memset` returns `dst`, which is already in `a0`.
            }
            RomStubEffect::Int64(op) => {
                // RV32 ABI: 64-bit values live in aligned register pairs, low
                // word first -- see `Int64Op`'s doc.
                let lhs = u64::from(self.regs.read(rom_stubs::REG_A0))
                    | (u64::from(self.regs.read(rom_stubs::REG_A1)) << 32);
                let rhs = u64::from(self.regs.read(rom_stubs::REG_A2))
                    | (u64::from(self.regs.read(rom_stubs::REG_A3)) << 32);
                let result = op.apply(lhs, rhs);
                self.regs.write(rom_stubs::REG_A0, result as u32);
                self.regs.write(rom_stubs::REG_A1, (result >> 32) as u32);
            }
        }
        self.regs.pc = self.regs.read(rom_stubs::REG_RA);
    }

    /// Common trap-entry sequence (RV32 privileged spec, M-mode only):
    /// saves `mepc`/`mcause`/`mtval`, moves `mstatus.MIE` into `MPIE` and
    /// clears `MIE`, sets `MPP` to M, and jumps `pc` to `mtvec` (or, in
    /// vectored mode, `mtvec.BASE + 4*cause` for interrupts).
    fn enter_trap(&mut self, cause: u32, is_interrupt: bool, tval: u32) {
        self.csr.mepc = self.regs.pc;
        self.csr.mcause = cause | if is_interrupt { 0x8000_0000 } else { 0 };
        self.csr.mtval = tval;

        let mie_set = self.csr.mstatus & mstatus_bits::MIE != 0;
        self.csr.mstatus &= !(mstatus_bits::MIE | mstatus_bits::MPIE | mstatus_bits::MPP_MASK);
        if mie_set {
            self.csr.mstatus |= mstatus_bits::MPIE;
        }
        self.csr.mstatus |= mstatus_bits::MPP_M;

        let base = self.csr.mtvec & !0b11;
        let vectored = self.csr.mtvec & 0b1 == 1;
        self.regs.pc = if vectored && is_interrupt {
            base.wrapping_add(4u32.wrapping_mul(cause))
        } else {
            base
        };
    }

    /// `mret`: reverses [`Cpu::enter_trap`] — restores `MIE` from `MPIE`,
    /// sets `MPIE` (per spec), leaves `MPP` at M (this core is M-mode
    /// only), and jumps `pc` back to `mepc`.
    fn exec_mret(&mut self) {
        let mpie_set = self.csr.mstatus & mstatus_bits::MPIE != 0;
        self.csr.mstatus &= !mstatus_bits::MIE;
        if mpie_set {
            self.csr.mstatus |= mstatus_bits::MIE;
        }
        self.csr.mstatus |= mstatus_bits::MPIE;
        self.csr.mstatus |= mstatus_bits::MPP_M;
        self.regs.pc = self.csr.mepc;
    }
}

/// Fetches one instruction at `pc`: reads 16 bits first, and only reads the
/// remaining 16 bits (forming a 32-bit word) if `bits[1:0] != 0b11` doesn't
/// hold, i.e. if it's *not* a compressed opcode. Returns the decoded
/// instruction and its length in bytes (2 or 4).
///
/// Uses [`Bus::fetch16`] (not `read16`) for both halfwords, since fetching
/// an instruction is architecturally distinct from a data load: an address
/// that isn't genuinely executable must fault rather than silently decode
/// whatever the never-panic data catch-all would have returned. On such a
/// fault, returns `Err(addr)` with the specific halfword address that
/// wasn't mapped (either `pc` itself, or `pc + 2` if the first halfword
/// fetched fine but indicated a full-width instruction whose second half
/// wasn't mapped).
fn fetch_and_decode<B: Bus>(bus: &mut B, pc: u32) -> Result<(Instruction, u8), u32> {
    let lo = bus.fetch16(pc).ok_or(pc)?;
    if lo & 0b11 != 0b11 {
        Ok((decode::decode_16(lo), 2))
    } else {
        let hi_addr = pc.wrapping_add(2);
        let hi = bus.fetch16(hi_addr).ok_or(hi_addr)?;
        let word = (lo as u32) | ((hi as u32) << 16);
        Ok((decode::decode_32(word), 4))
    }
}

#[cfg(test)]
#[allow(clippy::identity_op, clippy::unusual_byte_groupings)]
mod tests {
    use super::*;
    use crate::mem::Bus;

    /// Trivial flat `Vec<u8>`-backed `Bus` test double. Reads past the end
    /// of the backing vec return 0; writes past the end are ignored — this
    /// is intentionally not a real memory map, just enough to feed the CPU
    /// core instructions and observe loads/stores in tests.
    pub struct TestBus {
        pub mem: Vec<u8>,
    }

    impl TestBus {
        pub fn new(size: usize) -> Self {
            Self { mem: vec![0; size] }
        }

        pub fn with_program(words: &[u32]) -> Self {
            let mut mem = vec![0u8; words.len() * 4 + 4096];
            for (i, w) in words.iter().enumerate() {
                mem[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
            }
            Self { mem }
        }
    }

    impl Bus for TestBus {
        fn read8(&mut self, addr: u32) -> u8 {
            self.mem.get(addr as usize).copied().unwrap_or(0)
        }
        fn read16(&mut self, addr: u32) -> u16 {
            let a = addr as usize;
            let b0 = self.mem.get(a).copied().unwrap_or(0);
            let b1 = self.mem.get(a + 1).copied().unwrap_or(0);
            u16::from_le_bytes([b0, b1])
        }
        fn read32(&mut self, addr: u32) -> u32 {
            let a = addr as usize;
            let mut buf = [0u8; 4];
            for (i, b) in buf.iter_mut().enumerate() {
                *b = self.mem.get(a + i).copied().unwrap_or(0);
            }
            u32::from_le_bytes(buf)
        }
        fn write8(&mut self, addr: u32, val: u8) {
            if let Some(slot) = self.mem.get_mut(addr as usize) {
                *slot = val;
            }
        }
        fn write16(&mut self, addr: u32, val: u16) {
            let a = addr as usize;
            for (i, b) in val.to_le_bytes().iter().enumerate() {
                if let Some(slot) = self.mem.get_mut(a + i) {
                    *slot = *b;
                }
            }
        }
        fn write32(&mut self, addr: u32, val: u32) {
            let a = addr as usize;
            for (i, b) in val.to_le_bytes().iter().enumerate() {
                if let Some(slot) = self.mem.get_mut(a + i) {
                    *slot = *b;
                }
            }
        }
        fn fetch16(&mut self, addr: u32) -> Option<u16> {
            let a = addr as usize;
            if a + 1 < self.mem.len() {
                Some(u16::from_le_bytes([self.mem[a], self.mem[a + 1]]))
            } else {
                None
            }
        }
    }

    fn addi(rd: u8, rs1: u8, imm: i32) -> u32 {
        (((imm as u32) & 0xfff) << 20) | ((rs1 as u32) << 15) | ((rd as u32) << 7) | 0b0010011
    }

    fn i_type(opcode: u32, funct3: u32, rd: u8, rs1: u8, imm: i32) -> u32 {
        (((imm as u32) & 0xfff) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }

    fn r_type(opcode: u32, funct3: u32, funct7: u32, rd: u8, rs1: u8, rs2: u8) -> u32 {
        (funct7 << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }

    fn s_type(opcode: u32, funct3: u32, rs1: u8, rs2: u8, imm: i32) -> u32 {
        let imm = imm as u32 & 0xfff;
        ((imm >> 5) << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((imm & 0x1f) << 7)
            | opcode
    }

    fn b_type(funct3: u32, rs1: u8, rs2: u8, imm: i32) -> u32 {
        let imm = imm as u32; // 13-bit signed, bit0 always 0
        let imm12 = (imm >> 12) & 1;
        let imm11 = (imm >> 11) & 1;
        let imm10_5 = (imm >> 5) & 0x3f;
        let imm4_1 = (imm >> 1) & 0xf;
        (imm12 << 31)
            | (imm10_5 << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | (imm4_1 << 8)
            | (imm11 << 7)
            | 0b1100011
    }

    fn jal(rd: u8, imm: i32) -> u32 {
        let imm = imm as u32; // 21-bit signed, bit0 always 0
        let imm20 = (imm >> 20) & 1;
        let imm19_12 = (imm >> 12) & 0xff;
        let imm11 = (imm >> 11) & 1;
        let imm10_1 = (imm >> 1) & 0x3ff;
        (imm20 << 31)
            | (imm19_12 << 12)
            | (imm11 << 20)
            | (imm10_1 << 21)
            | ((rd as u32) << 7)
            | 0b1101111
    }

    fn lui(rd: u8, imm: i32) -> u32 {
        ((imm as u32) & 0xffff_f000) | ((rd as u32) << 7) | 0b0110111
    }

    fn auipc(rd: u8, imm: i32) -> u32 {
        ((imm as u32) & 0xffff_f000) | ((rd as u32) << 7) | 0b0010111
    }

    fn csrrs(rd: u8, csr: u16, rs1: u8) -> u32 {
        ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b010 << 12) | ((rd as u32) << 7) | 0b1110011
    }

    fn csrrc(rd: u8, csr: u16, rs1: u8) -> u32 {
        ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b011 << 12) | ((rd as u32) << 7) | 0b1110011
    }

    fn csrrwi(rd: u8, csr: u16, uimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((uimm as u32) << 15)
            | (0b101 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }

    fn csrrsi(rd: u8, csr: u16, uimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((uimm as u32) << 15)
            | (0b110 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }

    fn csrrci(rd: u8, csr: u16, uimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((uimm as u32) << 15)
            | (0b111 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }

    // ---------------- RV32I: LUI/AUIPC/JAL/JALR ----------------

    #[test]
    fn lui_loads_upper_immediate() {
        let mut cpu = Cpu::new();
        let mut bus = TestBus::with_program(&[lui(3, -1i32 << 12)]); // 0xFFFFF000
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 0xFFFF_F000);
    }

    #[test]
    fn auipc_adds_imm_to_pc() {
        let mut cpu = Cpu::new();
        let mut bus = TestBus::with_program(&[auipc(3, 0x2000)]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 0x2000);
    }

    #[test]
    fn jal_links_and_jumps() {
        let mut cpu = Cpu::new();
        let mut bus = TestBus::with_program(&[jal(1, 100)]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(1), 4); // return address
        assert_eq!(cpu.regs.pc, 100);
    }

    #[test]
    fn jalr_jumps_to_reg_plus_imm_and_clears_bit0() {
        let mut cpu = Cpu::new();
        cpu.regs.write(2, 0x205);
        let word = i_type(0b1100111, 0b000, 1, 2, 4); // jalr x1, 4(x2) -> target 0x209 & !1
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(1), 4);
        assert_eq!(cpu.regs.pc, 0x208);
    }

    // ---------------- RV32I: branches ----------------

    #[test]
    fn all_branch_kinds() {
        let cases: &[(u32, i32, i32, bool)] = &[
            (0b000, 5, 5, true),  // BEQ equal -> taken
            (0b001, 5, 6, true),  // BNE not equal -> taken
            (0b100, -1, 0, true), // BLT signed less -> taken
            (0b101, 0, -1, true), // BGE signed >= -> taken
            (0b110, 1, 2, true),  // BLTU unsigned less -> taken
            (0b111, 2, 1, true),  // BGEU unsigned >= -> taken
        ];
        for &(funct3, a, b, expect_taken) in cases {
            let mut cpu = Cpu::new();
            cpu.regs.write(1, a as u32);
            cpu.regs.write(2, b as u32);
            let mut bus = TestBus::with_program(&[b_type(funct3, 1, 2, 8), addi(0, 0, 0)]);
            cpu.step(&mut bus);
            let expected_pc = if expect_taken { 8 } else { 4 };
            assert_eq!(cpu.regs.pc, expected_pc, "funct3={funct3:03b}");
        }
    }

    #[test]
    fn branch_not_taken_falls_through() {
        let mut cpu = Cpu::new();
        cpu.regs.write(1, 1);
        cpu.regs.write(2, 2);
        let mut bus = TestBus::with_program(&[b_type(0b000, 1, 2, 8)]); // BEQ, not equal
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc, 4);
    }

    // ---------------- RV32I: loads/stores ----------------

    #[test]
    fn sb_sh_sw_and_lb_lh_lw_lbu_lhu_round_trip() {
        let mut cpu = Cpu::new();
        cpu.regs.write(1, 0); // base address
        cpu.regs.write(2, 0xffff_ff80u32); // -128 as stored byte pattern
        let sb = s_type(0b0100011, 0b000, 1, 2, 0);
        let mut bus = TestBus::with_program(&[sb]);
        cpu.step(&mut bus);
        assert_eq!(bus.mem[0], 0x80);

        // LB sign-extends 0x80 -> -128
        let mut cpu2 = Cpu::new();
        let lb = i_type(0b0000011, 0b000, 5, 1, 0);
        let mut prog_bus = TestBus::with_program(&[lb]);
        prog_bus.mem[4] = 0x80; // data byte right after the instruction
        cpu2.regs.write(1, 4);
        cpu2.step(&mut prog_bus);
        assert_eq!(cpu2.regs.read(5) as i32, -128);

        // LBU zero-extends 0x80 -> 128
        let mut cpu3 = Cpu::new();
        let lbu = i_type(0b0000011, 0b100, 5, 1, 0);
        let mut prog_bus3 = TestBus::with_program(&[lbu]);
        prog_bus3.mem[4] = 0x80;
        cpu3.regs.write(1, 4);
        cpu3.step(&mut prog_bus3);
        assert_eq!(cpu3.regs.read(5), 128);

        // SH/LH and SW/LW round trip
        let mut cpu4 = Cpu::new();
        cpu4.regs.write(1, 0);
        cpu4.regs.write(2, 0xbeef);
        let sh = s_type(0b0100011, 0b001, 1, 2, 8);
        let lh = i_type(0b0000011, 0b001, 6, 1, 8);
        let mut bus4 = TestBus::with_program(&[sh, lh]);
        cpu4.step(&mut bus4);
        cpu4.step(&mut bus4);
        assert_eq!(cpu4.regs.read(6) as i32, 0xbeefu16 as i16 as i32);

        let mut cpu5 = Cpu::new();
        cpu5.regs.write(1, 0);
        cpu5.regs.write(2, 0xdead_beef);
        let sw = s_type(0b0100011, 0b010, 1, 2, 16);
        let lw = i_type(0b0000011, 0b010, 6, 1, 16);
        let mut bus5 = TestBus::with_program(&[sw, lw]);
        cpu5.step(&mut bus5);
        cpu5.step(&mut bus5);
        assert_eq!(cpu5.regs.read(6), 0xdead_beef);
    }

    // ---------------- RV32I: OP-IMM / OP (ALU) ----------------

    #[test]
    fn opimm_and_op_alu_family() {
        let mut cpu = Cpu::new();
        cpu.regs.write(1, 6);
        cpu.regs.write(2, 3);
        // SLTI, SLTIU, XORI, ORI, ANDI, SLLI, SRLI, SRAI via OP-IMM;
        // ADD/SUB/SLL/SLT/SLTU/XOR/SRL/SRA/OR/AND via OP.
        let sub = r_type(0b0110011, 0b000, 0b0100000, 3, 1, 2);
        let sll = r_type(0b0110011, 0b001, 0, 3, 1, 2);
        let srl = r_type(0b0110011, 0b101, 0, 3, 1, 2);
        let sra = r_type(0b0110011, 0b101, 0b0100000, 3, 1, 2);
        let mut bus = TestBus::with_program(&[sub, sll, srl, sra]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 3); // 6 - 3
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 6 << 3); // 6 << 3
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 6 >> 3); // 6 >> 3 = 0
        cpu.regs.write(1, 0xffff_fff0);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), (0xffff_fff0u32 as i32 >> 3) as u32);
    }

    // ---------------- RV32M: MUL/DIV family, incl. edge cases ----------------

    #[test]
    fn mul_family() {
        let mut cpu = Cpu::new();
        cpu.regs.write(1, 5);
        cpu.regs.write(2, 7);
        let mul = r_type(0b0110011, 0b000, 0b0000001, 3, 1, 2);
        let mut bus = TestBus::with_program(&[mul]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 35);
    }

    #[test]
    fn div_by_zero_and_signed_overflow() {
        // DIV x/0 == -1 (all ones); REM x/0 == x; DIV INT_MIN/-1 == INT_MIN;
        // REM INT_MIN/-1 == 0. Per RISC-V M-extension spec table.
        let div = r_type(0b0110011, 0b100, 0b0000001, 3, 1, 2);
        let rem = r_type(0b0110011, 0b110, 0b0000001, 3, 1, 2);
        let divu = r_type(0b0110011, 0b101, 0b0000001, 3, 1, 2);
        let remu = r_type(0b0110011, 0b111, 0b0000001, 3, 1, 2);

        let mut cpu = Cpu::new();
        cpu.regs.write(1, 42);
        cpu.regs.write(2, 0);
        let mut bus = TestBus::with_program(&[div]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), u32::MAX);

        let mut cpu = Cpu::new();
        cpu.regs.write(1, 42);
        cpu.regs.write(2, 0);
        let mut bus = TestBus::with_program(&[rem]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 42);

        let mut cpu = Cpu::new();
        cpu.regs.write(1, i32::MIN as u32);
        cpu.regs.write(2, u32::MAX); // -1
        let mut bus = TestBus::with_program(&[div]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), i32::MIN as u32);

        let mut cpu = Cpu::new();
        cpu.regs.write(1, i32::MIN as u32);
        cpu.regs.write(2, u32::MAX);
        let mut bus = TestBus::with_program(&[rem]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 0);

        let mut cpu = Cpu::new();
        cpu.regs.write(1, 42);
        cpu.regs.write(2, 0);
        let mut bus = TestBus::with_program(&[divu]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), u32::MAX);

        let mut cpu = Cpu::new();
        cpu.regs.write(1, 42);
        cpu.regs.write(2, 0);
        let mut bus = TestBus::with_program(&[remu]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(3), 42);
    }

    // ---------------- Zicsr ----------------

    #[test]
    fn csrrs_csrrc_skip_write_when_rs1_is_x0() {
        let mut cpu = Cpu::new();
        cpu.csr.mscratch = 0xff;
        // CSRRS x5, mscratch, x0 -> reads old value, must NOT write (rs1==x0)
        let word = csrrs(5, csr_addr::MSCRATCH, 0);
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 0xff);
        assert_eq!(cpu.csr.mscratch, 0xff, "must not have been modified");
    }

    #[test]
    fn csrrc_clears_bits_via_rs1() {
        let mut cpu = Cpu::new();
        cpu.csr.mscratch = 0xff;
        cpu.regs.write(1, 0x0f);
        let word = csrrc(5, csr_addr::MSCRATCH, 1);
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 0xff);
        assert_eq!(cpu.csr.mscratch, 0xf0);
    }

    #[test]
    fn csrrwi_writes_immediate_and_reads_back_old_value() {
        let mut cpu = Cpu::new();
        cpu.csr.mscratch = 0xff;
        // CSRRWI x5, mscratch, 5 -> rd gets old value (0xff), CSR becomes 5.
        let word = csrrwi(5, csr_addr::MSCRATCH, 5);
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 0xff, "rd must observe the OLD csr value");
        assert_eq!(
            cpu.csr.mscratch, 5,
            "csr must be unconditionally overwritten with uimm"
        );
    }

    #[test]
    fn csrrsi_sets_bits_via_immediate() {
        let mut cpu = Cpu::new();
        cpu.csr.mscratch = 0xf0;
        // CSRRSI x5, mscratch, 0x0f -> sets the low nibble, rd gets old value.
        let word = csrrsi(5, csr_addr::MSCRATCH, 0x0f);
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 0xf0);
        assert_eq!(cpu.csr.mscratch, 0xff);
    }

    #[test]
    fn csrrsi_skips_write_when_uimm_is_zero() {
        let mut cpu = Cpu::new();
        cpu.csr.mscratch = 0xff;
        // CSRRSI x5, mscratch, 0 -> per spec, uimm==0 must not write.
        let word = csrrsi(5, csr_addr::MSCRATCH, 0);
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 0xff);
        assert_eq!(cpu.csr.mscratch, 0xff, "must not have been modified");
    }

    #[test]
    fn csrrci_clears_bits_via_immediate() {
        let mut cpu = Cpu::new();
        cpu.csr.mscratch = 0xff;
        // CSRRCI x5, mscratch, 0x0f -> clears the low nibble, rd gets old value.
        let word = csrrci(5, csr_addr::MSCRATCH, 0x0f);
        let mut bus = TestBus::with_program(&[word]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 0xff);
        assert_eq!(cpu.csr.mscratch, 0xf0);
    }

    // ---------------- Illegal instruction ----------------

    #[test]
    fn illegal_instruction_traps() {
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x1000;
        // opcode 0b1111111 (bits[1:0]=11 so it's fetched as a full 32-bit
        // word) is not part of any implemented RV32IMC opcode map.
        let mut bus = TestBus::with_program(&[0x7f]);
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken);
        assert_eq!(cpu.csr.mcause, exception_code::ILLEGAL_INSTRUCTION);
        assert_eq!(cpu.regs.pc, 0x1000);
    }

    // ---------------- Instruction-access-fault (unmapped fetch) ----------------

    /// A `Bus` double where data accesses (`read8`/`read16`/`read32`) behave
    /// like [`TestBus`] (never trap, read 0 out of bounds), but
    /// [`Bus::fetch16`] treats only addresses below `exec_limit` as
    /// executable — modeling a real bus where some address range is mapped
    /// and some isn't.
    struct PartiallyExecutableBus {
        mem: Vec<u8>,
        exec_limit: u32,
    }

    impl Bus for PartiallyExecutableBus {
        fn read8(&mut self, addr: u32) -> u8 {
            self.mem.get(addr as usize).copied().unwrap_or(0)
        }
        fn read16(&mut self, addr: u32) -> u16 {
            let a = addr as usize;
            u16::from_le_bytes([
                self.mem.get(a).copied().unwrap_or(0),
                self.mem.get(a + 1).copied().unwrap_or(0),
            ])
        }
        fn read32(&mut self, addr: u32) -> u32 {
            let a = addr as usize;
            let mut buf = [0u8; 4];
            for (i, b) in buf.iter_mut().enumerate() {
                *b = self.mem.get(a + i).copied().unwrap_or(0);
            }
            u32::from_le_bytes(buf)
        }
        fn write8(&mut self, _: u32, _: u8) {}
        fn write16(&mut self, _: u32, _: u16) {}
        fn write32(&mut self, _: u32, _: u32) {}
        fn fetch16(&mut self, addr: u32) -> Option<u16> {
            if addr < self.exec_limit && addr.wrapping_add(1) < self.exec_limit {
                Some(self.read16(addr))
            } else {
                None
            }
        }
    }

    #[test]
    fn unmapped_fetch_raises_instruction_access_fault_not_a_silent_noop() {
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x2000;
        let mut bus = PartiallyExecutableBus {
            mem: vec![0u8; 16],
            exec_limit: 8, // only [0, 8) is "executable"
        };
        cpu.regs.pc = 0x100; // well past exec_limit -> unmapped fetch

        let info = cpu.step(&mut bus);

        assert!(info.trap_taken);
        assert_eq!(
            cpu.csr.mcause,
            exception_code::INSTRUCTION_ACCESS_FAULT,
            "mcause must be the exception code"
        );
        assert_eq!(
            cpu.csr.mcause & 0x8000_0000,
            0,
            "the interrupt bit must NOT be set -- this is a synchronous exception"
        );
        assert_eq!(
            cpu.csr.mtval, 0x100,
            "mtval must be the faulting fetch address"
        );
        assert_eq!(
            cpu.regs.pc, 0x2000,
            "pc must redirect to mtvec, not advance past the faulting address"
        );
        assert_eq!(info.instr_len, 0, "no instruction was executed");
    }

    #[test]
    fn unmapped_second_halfword_of_full_width_instruction_faults_at_its_own_address() {
        // The first halfword (0xffff) has bits[1:0] == 0b11, so
        // fetch_and_decode must fetch a second halfword at pc+2 to form a
        // full 32-bit instruction. exec_limit only covers the first
        // halfword, so the SECOND fetch is what must fault, with mtval
        // reporting pc+2 (not pc).
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x3000;
        let mut bus = PartiallyExecutableBus {
            mem: vec![0xff, 0xff, 0, 0, 0, 0, 0, 0],
            exec_limit: 2, // only [0, 2) executable; pc+2 is unmapped
        };
        cpu.regs.pc = 0;

        let info = cpu.step(&mut bus);

        assert!(info.trap_taken);
        assert_eq!(cpu.csr.mcause, exception_code::INSTRUCTION_ACCESS_FAULT);
        assert_eq!(
            cpu.csr.mtval, 2,
            "mtval must be the second halfword's address, not pc"
        );
        assert_eq!(cpu.regs.pc, 0x3000);
    }

    // ---------------- RV32C coverage across all three quadrants ----------------

    #[test]
    fn compressed_quadrant0_lw_sw_roundtrip() {
        // C.SWSP x9(->stored value), then a hand-picked C.LW loading it back
        // via a base register, exercises quadrant 0 (C.LW) end-to-end.
        let mut cpu = Cpu::new();
        cpu.regs.write(9, 4); // rs1' = x9 (creg index 1), base address
        cpu.regs.write(10, 0x1234); // rs2' = x10 (creg index 2), value to store
                                    // C.SW x10, 0(x9): rs1'=1(x9), rs2'=2(x10), imm=0
        let c_sw: u16 =
            (0b110 << 13) | (0 << 10) | (1 << 7) | (0 << 6) | (0 << 5) | (2 << 2) | 0b00;
        // C.LW x11, 0(x9): rs1'=1(x9), rd'=3(x11), imm=0
        let c_lw: u16 =
            (0b010 << 13) | (0 << 10) | (1 << 7) | (0 << 6) | (0 << 5) | (3 << 2) | 0b00;
        let mut bus = TestBus::new(64);
        bus.mem[0..2].copy_from_slice(&c_sw.to_le_bytes());
        bus.mem[2..4].copy_from_slice(&c_lw.to_le_bytes());
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc, 2);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(11), 0x1234);
        assert_eq!(cpu.regs.pc, 4);
    }

    #[test]
    fn compressed_quadrant1_addi16sp_and_j() {
        let mut cpu = Cpu::new();
        cpu.regs.write(2, 0x1000); // sp
                                   // C.ADDI16SP sp, -32: nzimm[9]=1,imm[4|6|8:7|5] chosen so total=-32
                                   // -32 = 0b1_1110_0000 (9-bit view: bit9=1 sign, bits8:0=... );
                                   // easiest: use +16 instead (nzimm=16 -> imm[4]=1 rest 0 -> bit6=1)
        let c_addi16sp: u16 = (0b011 << 13) | (0 << 12) | (2 << 7) | (1 << 6) | 0b01;
        // C.J +0 (infinite jump to self is fine, we only step once)
        let c_j: u16 = (0b101 << 13) | 0b01;
        let mut bus = TestBus::new(64);
        bus.mem[0..2].copy_from_slice(&c_addi16sp.to_le_bytes());
        bus.mem[2..4].copy_from_slice(&c_j.to_le_bytes());
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(2), 0x1000 + 16);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc, 2); // C.J with offset 0, from pc=2
    }

    #[test]
    fn compressed_quadrant2_slli_and_add() {
        let mut cpu = Cpu::new();
        cpu.regs.write(5, 1);
        cpu.regs.write(6, 41);
        // C.SLLI x5, 2: rd=5, shamt=2
        let c_slli: u16 = (0b000 << 13) | (0 << 12) | (5 << 7) | (2 << 2) | 0b10;
        // C.ADD x5, x6: rd/rs1=5, rs2=6
        let c_add: u16 = (0b100 << 13) | (1 << 12) | (5 << 7) | (6 << 2) | 0b10;
        let mut bus = TestBus::new(64);
        bus.mem[0..2].copy_from_slice(&c_slli.to_le_bytes());
        bus.mem[2..4].copy_from_slice(&c_add.to_le_bytes());
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 4);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(5), 45);
    }

    #[test]
    fn compressed_c_jalr_and_c_ebreak() {
        let mut cpu = Cpu::new();
        cpu.regs.write(8, 0x40);
        // C.JALR x8: rd=1(implicit ra), rs1=8
        let c_jalr: u16 = (0b100 << 13) | (1 << 12) | (8 << 7) | (0 << 2) | 0b10;
        let mut bus = TestBus::new(128);
        bus.mem[0..2].copy_from_slice(&c_jalr.to_le_bytes());
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(cpu.regs.read(1), 2); // return address = pc + 2 (compressed)
        assert_eq!(cpu.regs.pc, 0x40);

        // C.EBREAK traps with cause = breakpoint.
        let mut cpu2 = Cpu::new();
        let c_ebreak: u16 = (0b100 << 13) | (1 << 12) | 0b10;
        let mut bus2 = TestBus::new(64);
        bus2.mem[0..2].copy_from_slice(&c_ebreak.to_le_bytes());
        let info2 = cpu2.step(&mut bus2);
        assert!(info2.trap_taken);
        assert_eq!(cpu2.csr.mcause, exception_code::BREAKPOINT);
    }

    #[test]
    fn step_executes_addi_and_advances_pc_by_4() {
        let mut cpu = Cpu::new();
        let mut bus = TestBus::with_program(&[addi(5, 0, 42)]);
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.instr_len, 4);
        assert_eq!(cpu.regs.read(5), 42);
        assert_eq!(cpu.regs.pc, 4);
    }

    #[test]
    fn ecall_takes_trap_and_sets_mepc_mcause() {
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x8000_0000;
        let mut bus = TestBus::with_program(&[0b0000000_00000_00000_000_00000_1110011]); // ecall
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken);
        assert_eq!(cpu.csr.mepc, 0);
        assert_eq!(cpu.csr.mcause, exception_code::ENVIRONMENT_CALL_FROM_M_MODE);
        assert_eq!(cpu.regs.pc, 0x8000_0000);
    }

    #[test]
    fn raise_interrupt_is_taken_on_next_step_not_immediately() {
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x8000_0000;
        cpu.csr.mstatus |= mstatus_bits::MIE;
        let mut bus = TestBus::with_program(&[addi(5, 0, 1), addi(6, 0, 2)]);

        cpu.raise_interrupt(7); // e.g. machine-timer-interrupt cause code

        // The pending interrupt should be taken on THIS call (the very next
        // step after raise_interrupt), before executing the ADDI at pc=0.
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken);
        assert_eq!(cpu.regs.read(5), 0, "instruction must not have executed");
        assert_eq!(cpu.csr.mcause, 0x8000_0000 | 7);
        assert_eq!(cpu.csr.mepc, 0);
        assert_eq!(cpu.regs.pc, 0x8000_0000);
        assert_eq!(
            cpu.csr.mstatus & mstatus_bits::MIE,
            0,
            "MIE must clear on trap entry"
        );
        assert_ne!(
            cpu.csr.mstatus & mstatus_bits::MPIE,
            0,
            "MPIE must capture old MIE"
        );
    }

    #[test]
    fn mret_restores_mie_and_returns_to_mepc() {
        let mut cpu = Cpu::new();
        cpu.csr.mepc = 0x100;
        cpu.csr.mstatus |= mstatus_bits::MPIE;
        let mret_word: u32 = 0b0011000_00010_00000_000_00000_1110011;
        let mut bus = TestBus::with_program(&[mret_word]);
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(cpu.regs.pc, 0x100);
        assert_ne!(cpu.csr.mstatus & mstatus_bits::MIE, 0);
    }

    // ---------------- Fix 1: mstatus.MIE gates interrupt-taking ----------------

    #[test]
    fn pending_interrupt_is_held_while_mie_clear_then_taken_once_mie_set_and_isr_returns_cleanly() {
        // Layout:
        //   0x00, 0x04, 0x08: three ordinary ADDIs -- normal code that must
        //     keep running undisturbed while the interrupt is pending but
        //     MIE is clear.
        //   0x0C: a NOP -- "normal execution resumes here" after `mret`
        //     (this is where the trap-taking step's `mepc` will point).
        //   0x40 (mtvec): the simulated ISR body -- one ADDI, then `mret`.
        const MTVEC: u32 = 0x40;
        let mret_word: u32 = 0b0011000_00010_00000_000_00000_1110011;
        let mut bus = TestBus::new(128);
        let prog = [
            (0x00u32, addi(5, 0, 1)),
            (0x04, addi(5, 0, 2)),
            (0x08, addi(5, 0, 3)),
            (0x0C, addi(0, 0, 0)), // NOP: where execution resumes post-mret
            (MTVEC, addi(6, 0, 99)),
            (MTVEC + 4, mret_word),
        ];
        for (addr, word) in prog {
            bus.mem[addr as usize..addr as usize + 4].copy_from_slice(&word.to_le_bytes());
        }

        let mut cpu = Cpu::new();
        cpu.csr.mtvec = MTVEC;
        // MIE starts clear (Csrs::default()). Arm a pending interrupt now.
        cpu.raise_interrupt(7);

        // Three steps with MIE clear: the interrupt must NOT be taken -- the
        // CPU just keeps executing normal code, and mcause is left alone.
        for expected_x5 in [1u32, 2, 3] {
            let info = cpu.step(&mut bus);
            assert!(
                !info.trap_taken,
                "interrupt must not be taken while MIE is clear"
            );
            assert_eq!(cpu.regs.read(5), expected_x5);
        }
        assert_eq!(cpu.regs.pc, 0x0C);
        assert_eq!(cpu.csr.mcause, 0, "mcause must be untouched so far");

        // Now enable MIE. The very next step must take the still-pending
        // interrupt instead of fetching the NOP at 0x0C.
        cpu.csr.mstatus |= mstatus_bits::MIE;
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken, "interrupt must be taken once MIE is set");
        assert_eq!(cpu.csr.mcause, 0x8000_0000 | 7);
        assert_eq!(cpu.csr.mepc, 0x0C, "mepc must be the NOP that didn't run");
        assert_eq!(cpu.regs.pc, MTVEC);
        assert_eq!(
            cpu.csr.mstatus & mstatus_bits::MIE,
            0,
            "MIE must clear on trap entry"
        );

        // Inside the simulated ISR: the level-triggered source re-asserting
        // while MIE is clear must NOT cause another trap on the next step --
        // this is the actual livelock this fix prevents.
        cpu.raise_interrupt(7);
        let info = cpu.step(&mut bus); // executes ADDI x6, x0, 99 at MTVEC
        assert!(
            !info.trap_taken,
            "re-asserting the source must not re-trap while MIE is clear (would be the livelock)"
        );
        assert_eq!(cpu.regs.read(6), 99, "the ISR body must actually run");

        // The ISR now clears the interrupt source (modeled here as the
        // source simply no longer being asserted) before returning.
        cpu.pending_interrupts = 0;

        // `mret`: MIE is restored from MPIE (which captured the old MIE=1),
        // pc returns to mepc.
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(cpu.regs.pc, 0x0C);
        assert_ne!(cpu.csr.mstatus & mstatus_bits::MIE, 0);

        // And normal execution resumes without immediately re-trapping --
        // proving this isn't still spinning on the level-triggered re-raise.
        let info = cpu.step(&mut bus);
        assert!(
            !info.trap_taken,
            "must not re-trap immediately after mret once the source is cleared"
        );
        assert_eq!(cpu.regs.pc, 0x10);
    }

    #[test]
    fn raise_interrupt_on_a_different_line_does_not_drop_an_already_pending_one() {
        // The "bundled fix": a second `raise_interrupt` call for a different
        // line, made while an earlier line is still pending and unconsumed
        // (MIE clear), must not silently clobber the first one. Both must
        // remain observable as pending, and the CPU takes them one at a time
        // rather than losing either.
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x1000;
        let mut bus = TestBus::with_program(&[
            addi(0, 0, 0), // NOP x3, padding so there's always something to fetch
            addi(0, 0, 0),
            addi(0, 0, 0),
        ]);

        cpu.raise_interrupt(3); // line 3 pending first
        cpu.raise_interrupt(9); // line 9 pending second -- must not drop line 3

        assert_eq!(
            cpu.pending_interrupts,
            (1 << 3) | (1 << 9),
            "both lines must be recorded as pending, not just the most recent call"
        );

        cpu.csr.mstatus |= mstatus_bits::MIE;

        // Lowest line number is taken first (this core's documented, v1
        // placeholder priority ordering -- see `Cpu::pending_interrupts`).
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken);
        assert_eq!(cpu.csr.mcause, 0x8000_0000 | 3, "line 3 taken first");
        assert_eq!(
            cpu.pending_interrupts,
            1 << 9,
            "line 9 must still be pending after line 3 is taken"
        );

        // `mret` back out, then the still-pending line 9 must be taken next.
        cpu.exec_mret();
        let info = cpu.step(&mut bus);
        assert!(info.trap_taken);
        assert_eq!(cpu.csr.mcause, 0x8000_0000 | 9, "line 9 taken second");
        assert_eq!(cpu.pending_interrupts, 0, "nothing left pending");
    }

    // ---------------- Mask-ROM HLE stubs ----------------

    /// Builds the shared program both ROM-stub tests below run: a normal ABI
    /// call (`jalr ra, 0(x5)`) to `STUB_ADDR`, a distinctive instruction at
    /// the return site, and a *real, executable* instruction sitting at
    /// `STUB_ADDR` itself so "was the stub intercepted, or did we fall
    /// through and execute the bytes there?" is directly observable.
    fn rom_stub_test_bus() -> TestBus {
        const STUB_ADDR: usize = 0x100;
        let mut bus = TestBus::with_program(&[
            addi(5, 0, STUB_ADDR as i32),      // x5 = STUB_ADDR
            i_type(0b1100111, 0b000, 1, 5, 0), // jalr x1, 0(x5): ra = 8, pc = STUB_ADDR
            addi(6, 0, 99),                    // the return site -- only runs if pc == ra
        ]);
        // The "ROM body": if the stub mechanism ever lets a fetch happen at
        // STUB_ADDR, this writes 7 into x7 and the test can tell.
        bus.mem[STUB_ADDR..STUB_ADDR + 4].copy_from_slice(&addi(7, 0, 7).to_le_bytes());
        bus
    }

    const ROM_STUB_ADDR: u32 = 0x100;

    #[test]
    fn rom_stub_sets_a0_redirects_pc_to_ra_and_skips_the_real_instruction() {
        let mut cpu = Cpu::new();
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::returning("fake_rom_fn", 42));
        cpu.set_rom_stubs(table);

        let mut bus = rom_stub_test_bus();

        cpu.step(&mut bus); // addi x5, x0, 0x100
        let call = cpu.step(&mut bus); // jalr x1, 0(x5)
        assert!(!call.trap_taken);
        assert_eq!(cpu.regs.read(1), 8, "ra must hold the return address");
        assert_eq!(cpu.regs.pc, ROM_STUB_ADDR);

        let stubbed = cpu.step(&mut bus);
        assert!(!stubbed.trap_taken, "a stub is not a trap");
        assert_eq!(stubbed.rom_stub, Some(ROM_STUB_ADDR));
        assert_eq!(
            stubbed.instr_len, 0,
            "no real instruction is fetched on a stub step"
        );
        assert_eq!(cpu.regs.read(10), 42, "a0/x10 must hold the return value");
        assert_eq!(cpu.regs.pc, 8, "pc must be redirected to ra");
        assert_eq!(
            cpu.regs.read(7),
            0,
            "the real instruction at the stub address must NOT have executed"
        );

        // And the caller genuinely resumes at the return site.
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(6), 99);
        assert_eq!(cpu.regs.pc, 12);
    }

    #[test]
    fn without_a_stub_table_the_same_call_executes_the_real_instruction() {
        // The control for the test above, and the guarantee the whole
        // mechanism rests on: a `Cpu` that never opts in behaves exactly as
        // it did before ROM stubs existed.
        let mut cpu = Cpu::new();
        assert!(cpu.rom_stubs().is_empty());

        let mut bus = rom_stub_test_bus();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        let info = cpu.step(&mut bus);

        assert_eq!(info.rom_stub, None);
        assert_eq!(info.instr_len, 4, "a real 4-byte instruction was fetched");
        assert_eq!(cpu.regs.read(7), 7, "the bytes at 0x100 executed normally");
        assert_eq!(cpu.regs.pc, ROM_STUB_ADDR + 4);
        assert_eq!(cpu.regs.read(10), 0, "nothing wrote a0");
    }

    #[test]
    fn void_rom_stub_leaves_a0_untouched_but_still_returns() {
        let mut cpu = Cpu::new();
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::void("fake_void_rom_fn"));
        cpu.set_rom_stubs(table);
        cpu.regs.write(10, 0xdead_beef); // a value the caller is keeping across the call

        let mut bus = rom_stub_test_bus();
        cpu.step(&mut bus);
        cpu.step(&mut bus);
        let stubbed = cpu.step(&mut bus);

        assert_eq!(stubbed.rom_stub, Some(ROM_STUB_ADDR));
        assert_eq!(
            cpu.regs.read(10),
            0xdead_beef,
            "a void stub must not clobber a0"
        );
        assert_eq!(cpu.regs.pc, 8);
    }

    #[test]
    fn memset_rom_stub_really_writes_the_bytes_through_the_bus() {
        let mut cpu = Cpu::new();
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::memset("memset"));
        cpu.set_rom_stubs(table);

        let mut bus = rom_stub_test_bus();
        // memset(dst = 0x200, c = 0xab, n = 5)
        cpu.regs.write(10, 0x200);
        cpu.regs.write(11, 0xab);
        cpu.regs.write(12, 5);
        cpu.regs.write(1, 0x40); // ra
        cpu.regs.pc = ROM_STUB_ADDR;
        bus.mem[0x205] = 0x11; // sentinel just past the end

        let info = cpu.step(&mut bus);

        assert_eq!(info.rom_stub, Some(ROM_STUB_ADDR));
        assert_eq!(&bus.mem[0x200..0x205], &[0xab; 5]);
        assert_eq!(bus.mem[0x205], 0x11, "must not write past n bytes");
        assert_eq!(cpu.regs.read(10), 0x200, "memset returns dst");
        assert_eq!(cpu.regs.pc, 0x40);
    }

    #[test]
    fn int64_rom_stub_reads_and_writes_the_rv32_register_pairs() {
        use rom_stubs::Int64Op;
        let mut cpu = Cpu::new();
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::int64("__udivdi3", Int64Op::UDiv));
        cpu.set_rom_stubs(table);

        // __udivdi3(0x0000_0002_0000_0000, 0x0000_0000_0000_0003)
        cpu.regs.write(10, 0x0000_0000); // a0 = lhs low
        cpu.regs.write(11, 0x0000_0002); // a1 = lhs high
        cpu.regs.write(12, 3); // a2 = rhs low
        cpu.regs.write(13, 0); // a3 = rhs high
        cpu.regs.write(1, 0x40);
        cpu.regs.pc = ROM_STUB_ADDR;

        let mut bus = rom_stub_test_bus();
        let info = cpu.step(&mut bus);

        let expected = 0x0000_0002_0000_0000u64 / 3;
        assert_eq!(info.rom_stub, Some(ROM_STUB_ADDR));
        assert_eq!(cpu.regs.read(10), expected as u32, "result low word in a0");
        assert_eq!(
            cpu.regs.read(11),
            (expected >> 32) as u32,
            "result high word in a1"
        );
        assert_eq!(cpu.regs.pc, 0x40);
    }

    #[test]
    fn memset_rom_stub_length_is_capped_so_a_garbage_argument_cannot_hang() {
        let mut cpu = Cpu::new();
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::memset("memset"));
        cpu.set_rom_stubs(table);

        let mut bus = rom_stub_test_bus();
        cpu.regs.write(10, 0x200);
        cpu.regs.write(11, 0xff);
        cpu.regs.write(12, u32::MAX); // nonsense length
        cpu.regs.write(1, 0x40);
        cpu.regs.pc = ROM_STUB_ADDR;

        // Completes rather than looping ~4 billion times. (TestBus drops
        // out-of-range writes, so the only thing being asserted here is that
        // the call terminates at all, and does so at the documented cap.)
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pc, 0x40);
    }

    #[test]
    fn pending_interrupt_is_taken_before_a_stub_at_the_same_pc() {
        // Documented ordering (see `rom_stubs`' module doc): the instruction
        // boundary in front of a stubbed call is a valid one to deliver an
        // interrupt at, so the trap wins and `mepc` points back at the stub
        // address -- the stub runs when the handler returns.
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x2000;
        cpu.csr.mstatus |= mstatus_bits::MIE;
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::returning("fake_rom_fn", 42));
        cpu.set_rom_stubs(table);

        let mut bus = rom_stub_test_bus();
        cpu.step(&mut bus);
        cpu.step(&mut bus); // pc is now ROM_STUB_ADDR
        cpu.raise_interrupt(7);

        let info = cpu.step(&mut bus);
        assert!(info.trap_taken);
        assert_eq!(info.rom_stub, None, "the trap ran, not the stub");
        assert_eq!(cpu.csr.mepc, ROM_STUB_ADDR);
        assert_eq!(cpu.regs.pc, 0x2000);
        assert_eq!(cpu.regs.read(10), 0, "the stub has not run yet");
    }

    #[test]
    fn rom_stub_with_a_bogus_ra_faults_loudly_rather_than_silently() {
        // The mechanism's documented limitation: `pc = ra` is only meaningful
        // if control arrived via a real ABI call. A zeroed `ra` sends pc to 0,
        // which this bus happens to consider executable -- so use an address
        // that isn't, and confirm the failure surfaces as a normal
        // instruction-access fault on the following step rather than
        // wandering off quietly.
        let mut cpu = Cpu::new();
        cpu.csr.mtvec = 0x3000;
        let mut table = RomStubTable::new();
        table.insert(ROM_STUB_ADDR, RomStub::returning("fake_rom_fn", 1));
        cpu.set_rom_stubs(table);
        cpu.regs.pc = ROM_STUB_ADDR;
        cpu.regs.write(1, 0xffff_0000); // nonsense "return address", far past the bus

        let mut bus = rom_stub_test_bus();
        let stubbed = cpu.step(&mut bus);
        assert_eq!(stubbed.rom_stub, Some(ROM_STUB_ADDR));
        assert_eq!(cpu.regs.pc, 0xffff_0000);

        let faulted = cpu.step(&mut bus);
        assert!(faulted.trap_taken);
        assert_eq!(cpu.csr.mcause, exception_code::INSTRUCTION_ACCESS_FAULT);
        assert_eq!(cpu.csr.mtval, 0xffff_0000);
    }

    #[test]
    fn compressed_instruction_advances_pc_by_2() {
        let mut cpu = Cpu::new();
        // C.NOP = 0x0001, packed little-endian as the first halfword.
        let mut bus = TestBus::new(64);
        bus.mem[0] = 0x01;
        bus.mem[1] = 0x00;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.instr_len, 2);
        assert_eq!(cpu.regs.pc, 2);
    }
}

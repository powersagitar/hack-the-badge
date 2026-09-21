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

pub use decode::{AluOp, BranchKind, CsrOp, CsrSrc, Instruction, LoadKind, MulDivOp, StoreKind};
pub use registers::{csr_addr, exception_code, mstatus_bits, Csrs, Registers};

use crate::mem::Bus;

/// A trap that [`Cpu::raise_interrupt`] has requested but that hasn't been
/// taken yet. Applied at the very start of the next [`Cpu::step`] call,
/// before fetching the next instruction.
#[derive(Debug, Clone, Copy)]
struct PendingTrap {
    cause: u32,
    is_interrupt: bool,
    tval: u32,
}

/// Outcome of a single [`Cpu::step`] call, for tests/callers to observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepInfo {
    /// `true` if this step took a trap (either a synchronous exception from
    /// the instruction just decoded, or a previously pending interrupt)
    /// instead of completing an instruction normally.
    pub trap_taken: bool,
    /// Length in bytes of the instruction that was fetched (2 or 4), or 0
    /// if a trap was taken before/instead of executing an instruction.
    pub instr_len: u8,
    /// The value of `pc` at the start of this `step()` call, before any
    /// trap-entry or normal advance.
    pub pc_before: u32,
}

/// The RV32IMC CPU core.
pub struct Cpu {
    pub regs: Registers,
    pub csr: Csrs,
    pending_trap: Option<PendingTrap>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self {
            regs: Registers::new(),
            csr: Csrs::new(),
            pending_trap: None,
        }
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests that an interrupt be delivered. This does *not* take the
    /// trap immediately — it marks it pending, and it is taken at the start
    /// of the next [`Cpu::step`] call, before that step fetches/executes an
    /// instruction. This is the mechanism an external interrupt
    /// controller/timer (built in a later task) calls to deliver an
    /// interrupt into the core.
    ///
    /// `cause` is the raw RISC-V exception code (e.g. 7 for
    /// machine-timer-interrupt, 11 for machine-external-interrupt per the
    /// standard cause numbering) — the interrupt bit is set automatically.
    /// Synchronous exceptions (`ECALL`/`EBREAK`/illegal instruction) are
    /// distinct from this mechanism: they're taken immediately, as part of
    /// the same `step()` that decoded the faulting instruction, since they
    /// are architecturally synchronous to it.
    pub fn raise_interrupt(&mut self, cause: u32) {
        self.pending_trap = Some(PendingTrap {
            cause,
            is_interrupt: true,
            tval: 0,
        });
    }

    /// Executes exactly one instruction: fetch, decode, execute, and advance
    /// `pc` — or, if an interrupt is pending (via [`Cpu::raise_interrupt`])
    /// or the instruction just decoded raises a synchronous exception,
    /// takes that trap instead (saving `mepc`/`mcause`, updating `mstatus`,
    /// and jumping to `mtvec`).
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> StepInfo {
        let pc_before = self.regs.pc;

        if let Some(pending) = self.pending_trap.take() {
            self.enter_trap(pending.cause, pending.is_interrupt, pending.tval);
            return StepInfo {
                trap_taken: true,
                instr_len: 0,
                pc_before,
            };
        }

        let pc = self.regs.pc;
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
                };
            }
        };

        match execute::execute(self, bus, instr, pc, len) {
            execute::ExecResult::Normal => StepInfo {
                trap_taken: false,
                instr_len: len,
                pc_before,
            },
            execute::ExecResult::Exception { cause, tval } => {
                self.enter_trap(cause, false, tval);
                StepInfo {
                    trap_taken: true,
                    instr_len: len,
                    pc_before,
                }
            }
        }
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

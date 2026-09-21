//! Execute: runs one already-decoded [`Instruction`] against a [`Cpu`] and
//! [`Bus`]. Shared by both the compressed and full-width fetch paths, since
//! `decode::decode_16` expands compressed encodings into the same
//! [`Instruction`] values `decode::decode_32` produces.

use super::registers::exception_code;
use super::{AluOp, BranchKind, Cpu, CsrOp, CsrSrc, Instruction, LoadKind, MulDivOp, StoreKind};
use crate::mem::Bus;

/// What running one instruction did.
pub(super) enum ExecResult {
    /// Ran normally; `cpu.regs.pc` has already been updated to the next
    /// instruction's address (either `pc + len` or a taken jump/branch
    /// target).
    Normal,
    /// The instruction raised a synchronous exception. `cpu.regs.pc` is
    /// left untouched (still the address of the faulting instruction) so
    /// the caller can use it as `mepc`.
    Exception { cause: u32, tval: u32 },
}

pub(super) fn execute<B: Bus>(
    cpu: &mut Cpu,
    bus: &mut B,
    instr: Instruction,
    pc: u32,
    len: u8,
) -> ExecResult {
    let fallthrough = pc.wrapping_add(len as u32);

    match instr {
        Instruction::Lui { rd, imm } => {
            cpu.regs.write(rd, imm as u32);
            cpu.regs.pc = fallthrough;
        }
        Instruction::Auipc { rd, imm } => {
            cpu.regs.write(rd, pc.wrapping_add(imm as u32));
            cpu.regs.pc = fallthrough;
        }
        Instruction::Jal { rd, imm } => {
            cpu.regs.write(rd, fallthrough);
            cpu.regs.pc = pc.wrapping_add(imm as u32);
        }
        Instruction::Jalr { rd, rs1, imm } => {
            let target = cpu.regs.read(rs1).wrapping_add(imm as u32) & !1;
            cpu.regs.write(rd, fallthrough);
            cpu.regs.pc = target;
        }
        Instruction::Branch {
            rs1,
            rs2,
            imm,
            kind,
        } => {
            let a = cpu.regs.read(rs1);
            let b = cpu.regs.read(rs2);
            let taken = match kind {
                BranchKind::Eq => a == b,
                BranchKind::Ne => a != b,
                BranchKind::Lt => (a as i32) < (b as i32),
                BranchKind::Ge => (a as i32) >= (b as i32),
                BranchKind::Ltu => a < b,
                BranchKind::Geu => a >= b,
            };
            cpu.regs.pc = if taken {
                pc.wrapping_add(imm as u32)
            } else {
                fallthrough
            };
        }
        Instruction::Load { rd, rs1, imm, kind } => {
            let addr = cpu.regs.read(rs1).wrapping_add(imm as u32);
            let val = match kind {
                LoadKind::B => bus.read8(addr) as i8 as i32 as u32,
                LoadKind::H => bus.read16(addr) as i16 as i32 as u32,
                LoadKind::W => bus.read32(addr),
                LoadKind::Bu => bus.read8(addr) as u32,
                LoadKind::Hu => bus.read16(addr) as u32,
            };
            cpu.regs.write(rd, val);
            cpu.regs.pc = fallthrough;
        }
        Instruction::Store {
            rs1,
            rs2,
            imm,
            kind,
        } => {
            let addr = cpu.regs.read(rs1).wrapping_add(imm as u32);
            let val = cpu.regs.read(rs2);
            match kind {
                StoreKind::B => bus.write8(addr, val as u8),
                StoreKind::H => bus.write16(addr, val as u16),
                StoreKind::W => bus.write32(addr, val),
            }
            cpu.regs.pc = fallthrough;
        }
        Instruction::OpImm { rd, rs1, imm, kind } => {
            let a = cpu.regs.read(rs1);
            let result = alu(kind, a, imm as u32);
            cpu.regs.write(rd, result);
            cpu.regs.pc = fallthrough;
        }
        Instruction::Op { rd, rs1, rs2, kind } => {
            let a = cpu.regs.read(rs1);
            let b = cpu.regs.read(rs2);
            let result = alu(kind, a, b);
            cpu.regs.write(rd, result);
            cpu.regs.pc = fallthrough;
        }
        Instruction::MulDiv { rd, rs1, rs2, kind } => {
            let a = cpu.regs.read(rs1);
            let b = cpu.regs.read(rs2);
            cpu.regs.write(rd, muldiv(kind, a, b));
            cpu.regs.pc = fallthrough;
        }
        Instruction::Fence => {
            cpu.regs.pc = fallthrough;
        }
        Instruction::Ecall => {
            return ExecResult::Exception {
                cause: exception_code::ENVIRONMENT_CALL_FROM_M_MODE,
                tval: 0,
            };
        }
        Instruction::Ebreak => {
            return ExecResult::Exception {
                cause: exception_code::BREAKPOINT,
                tval: 0,
            };
        }
        Instruction::Mret => {
            cpu.exec_mret();
        }
        Instruction::Csr { rd, csr, src, kind } => {
            let old = cpu.csr.read(csr);
            let operand = match src {
                CsrSrc::Reg(r) => cpu.regs.read(r),
                CsrSrc::Imm(i) => i as u32,
            };
            // Per spec: CSRRS/CSRRC with rs1==x0, and CSRRSI/CSRRCI with
            // uimm==0, must not perform the write (no write side effects).
            let skip_write = matches!(kind, CsrOp::Rs | CsrOp::Rc)
                && matches!((src, operand), (CsrSrc::Reg(0), _) | (CsrSrc::Imm(_), 0));
            if !skip_write {
                let new = match kind {
                    CsrOp::Rw => operand,
                    CsrOp::Rs => old | operand,
                    CsrOp::Rc => old & !operand,
                };
                cpu.csr.write(csr, new);
            }
            cpu.regs.write(rd, old);
            cpu.regs.pc = fallthrough;
        }
        Instruction::Illegal(bits) => {
            return ExecResult::Exception {
                cause: exception_code::ILLEGAL_INSTRUCTION,
                tval: bits,
            };
        }
    }
    ExecResult::Normal
}

fn alu(kind: AluOp, a: u32, b: u32) -> u32 {
    match kind {
        AluOp::Add => a.wrapping_add(b),
        AluOp::Sub => a.wrapping_sub(b),
        AluOp::Sll => a.wrapping_shl(b & 0x1f),
        AluOp::Slt => ((a as i32) < (b as i32)) as u32,
        AluOp::Sltu => (a < b) as u32,
        AluOp::Xor => a ^ b,
        AluOp::Srl => a.wrapping_shr(b & 0x1f),
        AluOp::Sra => ((a as i32).wrapping_shr(b & 0x1f)) as u32,
        AluOp::Or => a | b,
        AluOp::And => a & b,
    }
}

/// RV32M `MUL`/`DIV`/`REM` family, with the RISC-V-spec-mandated results
/// for division-by-zero and signed overflow (`INT_MIN / -1`) instead of a
/// Rust panic/UB.
fn muldiv(kind: MulDivOp, a: u32, b: u32) -> u32 {
    match kind {
        MulDivOp::Mul => a.wrapping_mul(b),
        MulDivOp::Mulh => {
            let r = (a as i32 as i64).wrapping_mul(b as i32 as i64);
            (r >> 32) as u32
        }
        MulDivOp::Mulhsu => {
            let r = (a as i32 as i64).wrapping_mul(b as i64);
            (r >> 32) as u32
        }
        MulDivOp::Mulhu => {
            let r = (a as u64).wrapping_mul(b as u64);
            (r >> 32) as u32
        }
        MulDivOp::Div => {
            let (a, b) = (a as i32, b as i32);
            if b == 0 {
                u32::MAX // -1
            } else if a == i32::MIN && b == -1 {
                a as u32 // overflow: result is the dividend, per spec
            } else {
                (a.wrapping_div(b)) as u32
            }
        }
        MulDivOp::Divu => a.checked_div(b).unwrap_or(u32::MAX),
        MulDivOp::Rem => {
            let (a, b) = (a as i32, b as i32);
            if b == 0 {
                a as u32 // dividend, per spec
            } else if a == i32::MIN && b == -1 {
                0 // overflow: remainder is 0
            } else {
                (a.wrapping_rem(b)) as u32
            }
        }
        MulDivOp::Remu => a.checked_rem(b).unwrap_or(a),
    }
}

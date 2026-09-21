//! Decode: turns a raw 32-bit or 16-bit instruction word into an
//! [`Instruction`] value that `crate::cpu::execute` knows how to run.
//!
//! Compressed (RVC) instructions are decoded by expanding them to the
//! equivalent full-width [`Instruction`] variant/fields (per the RISC-V
//! Unprivileged ISA spec's "C" chapter), so `execute` never needs to know
//! whether an instruction originally came from a 2-byte or 4-byte encoding.

/// A fully decoded instruction, independent of whether it was originally a
/// 16-bit (compressed) or 32-bit encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    Lui {
        rd: u8,
        imm: i32,
    },
    Auipc {
        rd: u8,
        imm: i32,
    },
    Jal {
        rd: u8,
        imm: i32,
    },
    Jalr {
        rd: u8,
        rs1: u8,
        imm: i32,
    },
    Branch {
        rs1: u8,
        rs2: u8,
        imm: i32,
        kind: BranchKind,
    },
    Load {
        rd: u8,
        rs1: u8,
        imm: i32,
        kind: LoadKind,
    },
    Store {
        rs1: u8,
        rs2: u8,
        imm: i32,
        kind: StoreKind,
    },
    OpImm {
        rd: u8,
        rs1: u8,
        imm: i32,
        kind: AluOp,
    },
    Op {
        rd: u8,
        rs1: u8,
        rs2: u8,
        kind: AluOp,
    },
    Fence,
    Ecall,
    Ebreak,
    Mret,
    Csr {
        rd: u8,
        csr: u16,
        src: CsrSrc,
        kind: CsrOp,
    },
    MulDiv {
        rd: u8,
        rs1: u8,
        rs2: u8,
        kind: MulDivOp,
    },
    /// Decode failure. Carries the raw instruction bits (zero-extended for
    /// compressed instructions) so `execute` can stash them in `mtval`.
    Illegal(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Eq,
    Ne,
    Lt,
    Ge,
    Ltu,
    Geu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    B,
    H,
    W,
    Bu,
    Hu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    B,
    H,
    W,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Sub,
    Sll,
    Slt,
    Sltu,
    Xor,
    Srl,
    Sra,
    Or,
    And,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulDivOp {
    Mul,
    Mulh,
    Mulhsu,
    Mulhu,
    Div,
    Divu,
    Rem,
    Remu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrOp {
    Rw,
    Rs,
    Rc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrSrc {
    Reg(u8),
    /// 5-bit zero-extended immediate (the `*I` CSR variants).
    Imm(u8),
}

// ---------------------------------------------------------------------
// 32-bit (full-width) decode
// ---------------------------------------------------------------------

#[inline]
fn bits(word: u32, hi: u32, lo: u32) -> u32 {
    (word >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

/// Sign-extends `val`'s bit `bit` (0-indexed) upward through a 32-bit value.
#[inline]
fn sext(val: u32, bit: u32) -> i32 {
    let shift = 31 - bit;
    ((val << shift) as i32) >> shift
}

pub fn decode_32(word: u32) -> Instruction {
    let opcode = bits(word, 6, 0);
    let rd = bits(word, 11, 7) as u8;
    let rs1 = bits(word, 19, 15) as u8;
    let rs2 = bits(word, 24, 20) as u8;
    let funct3 = bits(word, 14, 12) as u8;
    let funct7 = bits(word, 31, 25) as u8;

    match opcode {
        0b0110111 => Instruction::Lui {
            rd,
            imm: (word & 0xffff_f000) as i32,
        },
        0b0010111 => Instruction::Auipc {
            rd,
            imm: (word & 0xffff_f000) as i32,
        },
        0b1101111 => {
            // J-type: imm[20|10:1|11|19:12]
            let imm20 = bits(word, 31, 31);
            let imm10_1 = bits(word, 30, 21);
            let imm11 = bits(word, 20, 20);
            let imm19_12 = bits(word, 19, 12);
            let raw = (imm20 << 20) | (imm19_12 << 12) | (imm11 << 11) | (imm10_1 << 1);
            Instruction::Jal {
                rd,
                imm: sext(raw, 20),
            }
        }
        0b1100111 if funct3 == 0b000 => Instruction::Jalr {
            rd,
            rs1,
            imm: i_imm(word),
        },
        0b1100011 => {
            let kind = match funct3 {
                0b000 => BranchKind::Eq,
                0b001 => BranchKind::Ne,
                0b100 => BranchKind::Lt,
                0b101 => BranchKind::Ge,
                0b110 => BranchKind::Ltu,
                0b111 => BranchKind::Geu,
                _ => return Instruction::Illegal(word),
            };
            Instruction::Branch {
                rs1,
                rs2,
                imm: b_imm(word),
                kind,
            }
        }
        0b0000011 => {
            let kind = match funct3 {
                0b000 => LoadKind::B,
                0b001 => LoadKind::H,
                0b010 => LoadKind::W,
                0b100 => LoadKind::Bu,
                0b101 => LoadKind::Hu,
                _ => return Instruction::Illegal(word),
            };
            Instruction::Load {
                rd,
                rs1,
                imm: i_imm(word),
                kind,
            }
        }
        0b0100011 => {
            let kind = match funct3 {
                0b000 => StoreKind::B,
                0b001 => StoreKind::H,
                0b010 => StoreKind::W,
                _ => return Instruction::Illegal(word),
            };
            Instruction::Store {
                rs1,
                rs2,
                imm: s_imm(word),
                kind,
            }
        }
        0b0010011 => {
            // OP-IMM
            match funct3 {
                0b001 => {
                    // SLLI: shamt in bits[24:20], bits[31:25] must be 0.
                    if funct7 != 0b0000000 {
                        return Instruction::Illegal(word);
                    }
                    Instruction::OpImm {
                        rd,
                        rs1,
                        imm: rs2 as i32,
                        kind: AluOp::Sll,
                    }
                }
                0b101 => {
                    let kind = match funct7 {
                        0b0000000 => AluOp::Srl,
                        0b0100000 => AluOp::Sra,
                        _ => return Instruction::Illegal(word),
                    };
                    Instruction::OpImm {
                        rd,
                        rs1,
                        imm: rs2 as i32,
                        kind,
                    }
                }
                _ => {
                    let kind = match funct3 {
                        0b000 => AluOp::Add,
                        0b010 => AluOp::Slt,
                        0b011 => AluOp::Sltu,
                        0b100 => AluOp::Xor,
                        0b110 => AluOp::Or,
                        0b111 => AluOp::And,
                        _ => return Instruction::Illegal(word),
                    };
                    Instruction::OpImm {
                        rd,
                        rs1,
                        imm: i_imm(word),
                        kind,
                    }
                }
            }
        }
        0b0110011 => {
            if funct7 == 0b0000001 {
                // M extension
                let kind = match funct3 {
                    0b000 => MulDivOp::Mul,
                    0b001 => MulDivOp::Mulh,
                    0b010 => MulDivOp::Mulhsu,
                    0b011 => MulDivOp::Mulhu,
                    0b100 => MulDivOp::Div,
                    0b101 => MulDivOp::Divu,
                    0b110 => MulDivOp::Rem,
                    0b111 => MulDivOp::Remu,
                    _ => unreachable!("funct3 is 3 bits"),
                };
                return Instruction::MulDiv { rd, rs1, rs2, kind };
            }
            let kind = match (funct3, funct7) {
                (0b000, 0b0000000) => AluOp::Add,
                (0b000, 0b0100000) => AluOp::Sub,
                (0b001, 0b0000000) => AluOp::Sll,
                (0b010, 0b0000000) => AluOp::Slt,
                (0b011, 0b0000000) => AluOp::Sltu,
                (0b100, 0b0000000) => AluOp::Xor,
                (0b101, 0b0000000) => AluOp::Srl,
                (0b101, 0b0100000) => AluOp::Sra,
                (0b110, 0b0000000) => AluOp::Or,
                (0b111, 0b0000000) => AluOp::And,
                _ => return Instruction::Illegal(word),
            };
            Instruction::Op { rd, rs1, rs2, kind }
        }
        0b0001111 => Instruction::Fence, // FENCE / FENCE.I: no-op for this core
        0b1110011 => decode_system(word, rd, rs1, funct3, funct7, rs2),
        _ => Instruction::Illegal(word),
    }
}

fn decode_system(word: u32, rd: u8, rs1: u8, funct3: u8, funct7: u8, rs2: u8) -> Instruction {
    if funct3 == 0 {
        // funct7/rs2 select ECALL/EBREAK/MRET among the funct3==0 SYSTEM ops.
        return match (funct7, rs2, rd, rs1) {
            (0b0000000, 0b00000, 0, 0) => Instruction::Ecall,
            (0b0000000, 0b00001, 0, 0) => Instruction::Ebreak,
            (0b0011000, 0b00010, 0, 0) => Instruction::Mret,
            _ => Instruction::Illegal(word),
        };
    }
    let csr = bits(word, 31, 20) as u16;
    let kind = match funct3 & 0b011 {
        0b01 => CsrOp::Rw,
        0b10 => CsrOp::Rs,
        0b11 => CsrOp::Rc,
        _ => return Instruction::Illegal(word),
    };
    let src = if funct3 & 0b100 != 0 {
        CsrSrc::Imm(rs1)
    } else {
        CsrSrc::Reg(rs1)
    };
    Instruction::Csr { rd, csr, src, kind }
}

fn i_imm(word: u32) -> i32 {
    sext(bits(word, 31, 20), 11)
}

fn s_imm(word: u32) -> i32 {
    let hi = bits(word, 31, 25);
    let lo = bits(word, 11, 7);
    sext((hi << 5) | lo, 11)
}

fn b_imm(word: u32) -> i32 {
    let imm12 = bits(word, 31, 31);
    let imm10_5 = bits(word, 30, 25);
    let imm4_1 = bits(word, 11, 8);
    let imm11 = bits(word, 7, 7);
    let raw = (imm12 << 12) | (imm11 << 11) | (imm10_5 << 5) | (imm4_1 << 1);
    sext(raw, 12)
}

// ---------------------------------------------------------------------
// 16-bit (compressed, RVC) decode
// ---------------------------------------------------------------------

#[inline]
fn cbits(half: u16, hi: u32, lo: u32) -> u32 {
    ((half as u32) >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

/// Maps a compressed 3-bit register field (`rd'`/`rs1'`/`rs2'`) to `x8..x15`.
#[inline]
fn creg(field: u32) -> u8 {
    (field as u8) + 8
}

/// Decodes a 16-bit compressed instruction by expanding it to the
/// equivalent full-width [`Instruction`]. `half` is the raw compressed word
/// (bits 15:0); the caller has already confirmed `bits[1:0] != 0b11`.
pub fn decode_16(half: u16) -> Instruction {
    let op = cbits(half, 1, 0);
    let funct3 = cbits(half, 15, 13);

    match (op, funct3) {
        // ---------------- Quadrant 0 ----------------
        (0b00, 0b000) => {
            // C.ADDI4SPN: rd' = x2 + nzuimm (nzuimm != 0, else HINT/no-op)
            let rd = creg(cbits(half, 4, 2));
            let nzuimm = (cbits(half, 10, 7) << 6)
                | (cbits(half, 12, 11) << 4)
                | (cbits(half, 5, 5) << 3)
                | (cbits(half, 6, 6) << 2);
            Instruction::OpImm {
                rd,
                rs1: 2,
                imm: nzuimm as i32,
                kind: AluOp::Add,
            }
        }
        (0b00, 0b010) => {
            // C.LW
            let rd = creg(cbits(half, 4, 2));
            let rs1 = creg(cbits(half, 9, 7));
            let imm =
                (cbits(half, 5, 5) << 6) | (cbits(half, 12, 10) << 3) | (cbits(half, 6, 6) << 2);
            Instruction::Load {
                rd,
                rs1,
                imm: imm as i32,
                kind: LoadKind::W,
            }
        }
        (0b00, 0b110) => {
            // C.SW
            let rs2 = creg(cbits(half, 4, 2));
            let rs1 = creg(cbits(half, 9, 7));
            let imm =
                (cbits(half, 5, 5) << 6) | (cbits(half, 12, 10) << 3) | (cbits(half, 6, 6) << 2);
            Instruction::Store {
                rs1,
                rs2,
                imm: imm as i32,
                kind: StoreKind::W,
            }
        }

        // ---------------- Quadrant 1 ----------------
        (0b01, 0b000) => {
            // C.ADDI (rd==0,imm==0 is C.NOP; handled naturally as ADDI x0,x0,0)
            let rd = cbits(half, 11, 7) as u8;
            let imm = ci_imm(half);
            Instruction::OpImm {
                rd,
                rs1: rd,
                imm,
                kind: AluOp::Add,
            }
        }
        (0b01, 0b001) => {
            // C.JAL (RV32): rd = x1
            Instruction::Jal {
                rd: 1,
                imm: cj_imm(half),
            }
        }
        (0b01, 0b010) => {
            // C.LI
            let rd = cbits(half, 11, 7) as u8;
            Instruction::OpImm {
                rd,
                rs1: 0,
                imm: ci_imm(half),
                kind: AluOp::Add,
            }
        }
        (0b01, 0b011) => {
            let rd = cbits(half, 11, 7) as u8;
            if rd == 2 {
                // C.ADDI16SP
                let raw = (cbits(half, 12, 12) << 9)
                    | (cbits(half, 4, 3) << 7)
                    | (cbits(half, 5, 5) << 6)
                    | (cbits(half, 2, 2) << 5)
                    | (cbits(half, 6, 6) << 4);
                let imm = sext(raw, 9);
                Instruction::OpImm {
                    rd: 2,
                    rs1: 2,
                    imm,
                    kind: AluOp::Add,
                }
            } else {
                // C.LUI
                let raw = (cbits(half, 12, 12) << 5) | cbits(half, 6, 2);
                let imm = sext(raw, 5) << 12;
                Instruction::Lui { rd, imm }
            }
        }
        (0b01, 0b100) => {
            let rd_rs1 = creg(cbits(half, 9, 7));
            match cbits(half, 11, 10) {
                0b00 => Instruction::OpImm {
                    rd: rd_rs1,
                    rs1: rd_rs1,
                    imm: c_shamt(half),
                    kind: AluOp::Srl,
                },
                0b01 => Instruction::OpImm {
                    rd: rd_rs1,
                    rs1: rd_rs1,
                    imm: c_shamt(half),
                    kind: AluOp::Sra,
                },
                0b10 => Instruction::OpImm {
                    rd: rd_rs1,
                    rs1: rd_rs1,
                    imm: ci_imm(half),
                    kind: AluOp::And,
                },
                0b11 => {
                    let rs2 = creg(cbits(half, 4, 2));
                    if cbits(half, 12, 12) != 0 {
                        // C.SUBW/C.ADDW (RV64/128 only) - not valid on RV32.
                        return Instruction::Illegal(half as u32);
                    }
                    let kind = match cbits(half, 6, 5) {
                        0b00 => AluOp::Sub,
                        0b01 => AluOp::Xor,
                        0b10 => AluOp::Or,
                        0b11 => AluOp::And,
                        _ => unreachable!("2 bits"),
                    };
                    Instruction::Op {
                        rd: rd_rs1,
                        rs1: rd_rs1,
                        rs2,
                        kind,
                    }
                }
                _ => unreachable!("2 bits"),
            }
        }
        (0b01, 0b101) => Instruction::Jal {
            rd: 0,
            imm: cj_imm(half),
        }, // C.J
        (0b01, 0b110) => {
            // C.BEQZ
            let rs1 = creg(cbits(half, 9, 7));
            Instruction::Branch {
                rs1,
                rs2: 0,
                imm: cb_imm(half),
                kind: BranchKind::Eq,
            }
        }
        (0b01, 0b111) => {
            // C.BNEZ
            let rs1 = creg(cbits(half, 9, 7));
            Instruction::Branch {
                rs1,
                rs2: 0,
                imm: cb_imm(half),
                kind: BranchKind::Ne,
            }
        }

        // ---------------- Quadrant 2 ----------------
        (0b10, 0b000) => {
            // C.SLLI
            let rd = cbits(half, 11, 7) as u8;
            Instruction::OpImm {
                rd,
                rs1: rd,
                imm: c_shamt(half),
                kind: AluOp::Sll,
            }
        }
        (0b10, 0b010) => {
            // C.LWSP (rd == 0 is reserved but we decode it uniformly; the
            // resulting write to x0 is silently discarded, matching a HINT).
            let rd = cbits(half, 11, 7) as u8;
            let imm =
                (cbits(half, 3, 2) << 6) | (cbits(half, 12, 12) << 5) | (cbits(half, 6, 4) << 2);
            Instruction::Load {
                rd,
                rs1: 2,
                imm: imm as i32,
                kind: LoadKind::W,
            }
        }
        (0b10, 0b100) => {
            let rs1_or_rd = cbits(half, 11, 7) as u8;
            let rs2 = cbits(half, 6, 2) as u8;
            match (cbits(half, 12, 12), rs2) {
                (0, 0) => Instruction::Jalr {
                    rd: 0,
                    rs1: rs1_or_rd,
                    imm: 0,
                }, // C.JR
                (0, _) => Instruction::Op {
                    rd: rs1_or_rd,
                    rs1: 0,
                    rs2,
                    kind: AluOp::Add,
                }, // C.MV
                (1, 0) if rs1_or_rd == 0 => Instruction::Ebreak, // C.EBREAK
                (1, 0) => Instruction::Jalr {
                    rd: 1,
                    rs1: rs1_or_rd,
                    imm: 0,
                }, // C.JALR
                (1, _) => Instruction::Op {
                    rd: rs1_or_rd,
                    rs1: rs1_or_rd,
                    rs2,
                    kind: AluOp::Add,
                }, // C.ADD
                _ => unreachable!("1 bit"),
            }
        }
        (0b10, 0b110) => {
            // C.SWSP
            let rs2 = cbits(half, 6, 2) as u8;
            let imm = (cbits(half, 8, 7) << 6) | (cbits(half, 12, 9) << 2);
            Instruction::Store {
                rs1: 2,
                rs2,
                imm: imm as i32,
                kind: StoreKind::W,
            }
        }

        _ => Instruction::Illegal(half as u32),
    }
}

/// CI-format signed immediate used by C.ADDI/C.LI/C.ANDI: imm[5]=bit12,
/// imm[4:0]=bits[6:2], sign-extended from bit 5.
fn ci_imm(half: u16) -> i32 {
    let raw = (cbits(half, 12, 12) << 5) | cbits(half, 6, 2);
    sext(raw, 5)
}

/// Shift amount used by C.SLLI/C.SRLI/C.SRAI. Bit 12 would be shamt[5] on
/// RV64/128; on RV32 it must be 0 for a legal encoding, but we mask to 5
/// bits rather than rejecting a set bit 12, since well-formed RV32C code
/// never sets it.
fn c_shamt(half: u16) -> i32 {
    (cbits(half, 6, 2) & 0x1f) as i32
}

/// CJ-format jump-target immediate used by C.J/C.JAL:
/// imm[11|4|9:8|10|6|7|3:1|5], sign-extended from bit 11.
fn cj_imm(half: u16) -> i32 {
    let raw = (cbits(half, 12, 12) << 11)
        | (cbits(half, 8, 8) << 10)
        | (cbits(half, 10, 9) << 8)
        | (cbits(half, 6, 6) << 7)
        | (cbits(half, 7, 7) << 6)
        | (cbits(half, 2, 2) << 5)
        | (cbits(half, 11, 11) << 4)
        | (cbits(half, 5, 3) << 1);
    sext(raw, 11)
}

/// CB-format branch-offset immediate used by C.BEQZ/C.BNEZ:
/// imm[8|4:3|7:6|2:1|5], sign-extended from bit 8.
fn cb_imm(half: u16) -> i32 {
    let raw = (cbits(half, 12, 12) << 8)
        | (cbits(half, 6, 5) << 6)
        | (cbits(half, 2, 2) << 5)
        | (cbits(half, 11, 10) << 3)
        | (cbits(half, 4, 3) << 1);
    sext(raw, 8)
}

#[cfg(test)]
#[allow(clippy::identity_op, clippy::unusual_byte_groupings)]
mod tests {
    use super::*;

    // -------- 32-bit decode --------

    #[test]
    fn decodes_addi() {
        // addi x5, x6, -1  -> imm=0xfff, rs1=x6, funct3=000, rd=x5, opcode=0010011
        let word = (0xfffu32 << 20) | (6 << 15) | (0b000 << 12) | (5 << 7) | 0b0010011;
        match decode_32(word) {
            Instruction::OpImm {
                rd,
                rs1,
                imm,
                kind: AluOp::Add,
            } => {
                assert_eq!(rd, 5);
                assert_eq!(rs1, 6);
                assert_eq!(imm, -1);
            }
            other => panic!("unexpected decode: {other:?}"),
        }
    }

    #[test]
    fn decodes_lui() {
        let word = (0xABCDEu32 << 12) | (7 << 7) | 0b0110111;
        match decode_32(word) {
            Instruction::Lui { rd, imm } => {
                assert_eq!(rd, 7);
                assert_eq!(imm, 0xABCDE000u32 as i32);
            }
            other => panic!("unexpected decode: {other:?}"),
        }
    }

    #[test]
    fn decodes_beq_negative_offset() {
        // beq x1, x2, -4
        // imm = -4 = 0b1_1111_1111_1100 (13 bits incl implicit 0)
        // fields: imm[12]=1 imm[11]=1 imm[10:5]=111111 imm[4:1]=1110
        let imm12 = 1u32;
        let imm11 = 1u32;
        let imm10_5 = 0b111111u32;
        let imm4_1 = 0b1110u32;
        let word = (imm12 << 31)
            | (imm10_5 << 25)
            | (2 << 20)
            | (1 << 15)
            | (0b000 << 12)
            | (imm4_1 << 8)
            | (imm11 << 7)
            | 0b1100011;
        match decode_32(word) {
            Instruction::Branch {
                rs1,
                rs2,
                imm,
                kind: BranchKind::Eq,
            } => {
                assert_eq!(rs1, 1);
                assert_eq!(rs2, 2);
                assert_eq!(imm, -4);
            }
            other => panic!("unexpected decode: {other:?}"),
        }
    }

    #[test]
    fn decodes_mul_and_divu() {
        let mul = (0b0000001 << 25) | (2 << 20) | (1 << 15) | (0b000 << 12) | (3 << 7) | 0b0110011;
        assert_eq!(
            decode_32(mul),
            Instruction::MulDiv {
                rd: 3,
                rs1: 1,
                rs2: 2,
                kind: MulDivOp::Mul
            }
        );
        let divu = (0b0000001 << 25) | (2 << 20) | (1 << 15) | (0b101 << 12) | (3 << 7) | 0b0110011;
        assert_eq!(
            decode_32(divu),
            Instruction::MulDiv {
                rd: 3,
                rs1: 1,
                rs2: 2,
                kind: MulDivOp::Divu
            }
        );
    }

    #[test]
    fn decodes_csrrw() {
        // csrrw x1, mtvec(0x305), x2
        let word = (0x305u32 << 20) | (2 << 15) | (0b001 << 12) | (1 << 7) | 0b1110011;
        match decode_32(word) {
            Instruction::Csr {
                rd,
                csr,
                src: CsrSrc::Reg(rs1),
                kind: CsrOp::Rw,
            } => {
                assert_eq!(rd, 1);
                assert_eq!(csr, 0x305);
                assert_eq!(rs1, 2);
            }
            other => panic!("unexpected decode: {other:?}"),
        }
    }

    #[test]
    fn decodes_csrrwi_csrrsi_csrrci() {
        // csrrwi x1, mscratch(0x340), uimm=5 -> funct3=101, rs1 field holds
        // the 5-bit immediate directly (no register read).
        let word = (0x340u32 << 20) | (5 << 15) | (0b101 << 12) | (1 << 7) | 0b1110011;
        match decode_32(word) {
            Instruction::Csr {
                rd,
                csr,
                src: CsrSrc::Imm(uimm),
                kind: CsrOp::Rw,
            } => {
                assert_eq!(rd, 1);
                assert_eq!(csr, 0x340);
                assert_eq!(uimm, 5);
            }
            other => panic!("unexpected decode: {other:?}"),
        }

        // csrrsi x2, mscratch(0x340), uimm=0x1f -> funct3=110
        let word = (0x340u32 << 20) | (0x1f << 15) | (0b110 << 12) | (2 << 7) | 0b1110011;
        match decode_32(word) {
            Instruction::Csr {
                rd,
                csr,
                src: CsrSrc::Imm(uimm),
                kind: CsrOp::Rs,
            } => {
                assert_eq!(rd, 2);
                assert_eq!(csr, 0x340);
                assert_eq!(uimm, 0x1f);
            }
            other => panic!("unexpected decode: {other:?}"),
        }

        // csrrci x3, mscratch(0x340), uimm=0x0a -> funct3=111
        let word = (0x340u32 << 20) | (0x0a << 15) | (0b111 << 12) | (3 << 7) | 0b1110011;
        match decode_32(word) {
            Instruction::Csr {
                rd,
                csr,
                src: CsrSrc::Imm(uimm),
                kind: CsrOp::Rc,
            } => {
                assert_eq!(rd, 3);
                assert_eq!(csr, 0x340);
                assert_eq!(uimm, 0x0a);
            }
            other => panic!("unexpected decode: {other:?}"),
        }
    }

    #[test]
    fn decodes_ecall_ebreak_mret() {
        assert_eq!(
            decode_32(0b0000000_00000_00000_000_00000_1110011),
            Instruction::Ecall
        );
        assert_eq!(
            decode_32(0b0000000_00001_00000_000_00000_1110011),
            Instruction::Ebreak
        );
        assert_eq!(
            decode_32(0b0011000_00010_00000_000_00000_1110011),
            Instruction::Mret
        );
    }

    // -------- 16-bit (compressed) decode --------

    #[test]
    fn decodes_c_nop_and_c_addi() {
        // C.NOP = 0x0001 (funct3=000, all other fields 0)
        assert_eq!(
            decode_16(0x0001),
            Instruction::OpImm {
                rd: 0,
                rs1: 0,
                imm: 0,
                kind: AluOp::Add
            }
        );
        // C.ADDI x8, 1 -> op=01 funct3=000 rd=8(bits11:7) imm[5]=0 imm[4:0]=00001
        let half: u16 = (0b000 << 13) | (8 << 7) | (0b0 << 12) | (0b00001 << 2) | 0b01;
        assert_eq!(
            decode_16(half),
            Instruction::OpImm {
                rd: 8,
                rs1: 8,
                imm: 1,
                kind: AluOp::Add
            }
        );
    }

    #[test]
    fn decodes_c_mv_and_c_jr() {
        // C.MV x8, x9: op=10 funct3=100 bit12=0 rd/rs1=8 rs2=9
        let half: u16 = (0b100 << 13) | (0 << 12) | (8 << 7) | (9 << 2) | 0b10;
        assert_eq!(
            decode_16(half),
            Instruction::Op {
                rd: 8,
                rs1: 0,
                rs2: 9,
                kind: AluOp::Add
            }
        );
        // C.JR x8: op=10 funct3=100 bit12=0 rd/rs1=8 rs2=0
        let half: u16 = (0b100 << 13) | (0 << 12) | (8 << 7) | (0 << 2) | 0b10;
        assert_eq!(
            decode_16(half),
            Instruction::Jalr {
                rd: 0,
                rs1: 8,
                imm: 0
            }
        );
    }

    #[test]
    fn decodes_c_lw_c_sw() {
        // C.LW x8, 4(x9): op=00 funct3=010 rs1'=9-8=1 rd'=8-8=0 imm=4 -> imm[2]=1 others 0
        // imm bits: bit5=imm[6], bits[12:10]=imm[5:3], bit6=imm[2]
        // imm=4 -> imm[2]=1, rest 0 => bit6=1, bits[12:10]=0
        let half: u16 =
            (0b010 << 13) | (0b000 << 10) | (1 << 7) | (1 << 6) | (0 << 5) | (0b000 << 2) | 0b00;
        assert_eq!(
            decode_16(half),
            Instruction::Load {
                rd: 8,
                rs1: 9,
                imm: 4,
                kind: LoadKind::W
            }
        );
    }

    #[test]
    fn decodes_c_sw() {
        // C.SW x9, 4(x10): rs1'=10-8=2, rs2'=9-8=1, imm=4 -> bit6=1, bits[12:10]=0, bit5=0
        let half: u16 =
            (0b110 << 13) | (0b000 << 10) | (2 << 7) | (1 << 6) | (0 << 5) | (0b001 << 2) | 0b00;
        assert_eq!(
            decode_16(half),
            Instruction::Store {
                rs1: 10,
                rs2: 9,
                imm: 4,
                kind: StoreKind::W
            }
        );
    }

    #[test]
    fn decodes_c_lui() {
        // C.LUI x5, 0x1 -> nzimm[17]=0, nzimm[16:12]=00001 -> imm = 1<<12
        let half: u16 = (0b011 << 13) | (0 << 12) | (5 << 7) | (0b00001 << 2) | 0b01;
        assert_eq!(
            decode_16(half),
            Instruction::Lui {
                rd: 5,
                imm: 1 << 12
            }
        );
    }

    #[test]
    fn decodes_c_j() {
        // C.J with all immediate bits 0 -> offset 0
        let half: u16 = (0b101 << 13) | 0b01;
        assert_eq!(decode_16(half), Instruction::Jal { rd: 0, imm: 0 });
    }

    #[test]
    fn decodes_c_swsp_and_c_lwsp() {
        // C.SWSP x5, 0(sp): rs2=5, imm=0
        let half: u16 = (0b110 << 13) | (5 << 2) | 0b10;
        assert_eq!(
            decode_16(half),
            Instruction::Store {
                rs1: 2,
                rs2: 5,
                imm: 0,
                kind: StoreKind::W
            }
        );
        // C.LWSP x5, 0(sp): rd=5, imm=0
        let half: u16 = (0b010 << 13) | (5 << 7) | 0b10;
        assert_eq!(
            decode_16(half),
            Instruction::Load {
                rd: 5,
                rs1: 2,
                imm: 0,
                kind: LoadKind::W
            }
        );
    }

    #[test]
    fn decodes_c_beqz() {
        // C.BEQZ x9, offset: try offset=0 -> all imm bits 0
        let half: u16 = (0b110 << 13) | (1 << 7) | 0b01;
        assert_eq!(
            decode_16(half),
            Instruction::Branch {
                rs1: 9,
                rs2: 0,
                imm: 0,
                kind: BranchKind::Eq
            }
        );
    }
}

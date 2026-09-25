//! A tiny `const fn` RV32IM *encoder*: the inverse of [`super::decode`], for
//! the handful of instruction forms this crate hand-assembles into
//! guest-executed code blobs (see `crate::rom`'s guest-executed ROM
//! routines, and `crate::mem::bus::RomCodeBlob`).
//!
//! Chip-agnostic, like the rest of `crate::cpu`: it knows the RISC-V base
//! encodings (RISC-V Unprivileged ISA spec, chapter "RV32I Base Integer
//! Instruction Set", §"Instruction Formats"/"Immediate Encoding Variants",
//! plus the "M" extension's `MUL`) and nothing about any SoC.
//!
//! Every function is `const`, so a blob built from them is a compile-time
//! `[u32; N]` constant — no external assembler, no build step. An
//! out-of-range immediate panics, which in a `const` context is a
//! *compile-time* error rather than a silently truncated encoding.
//! `crate::rom`'s tests additionally decode every emitted word through
//! [`super::decode::decode_32`] to prove the round trip.

// Register numbers (RISC-V psABI names), for readable blob listings.
pub const ZERO: u8 = 0;
pub const RA: u8 = 1;
pub const SP: u8 = 2;
pub const T0: u8 = 5;
pub const T1: u8 = 6;
pub const T2: u8 = 7;
pub const S0: u8 = 8;
pub const S1: u8 = 9;
pub const A0: u8 = 10;
pub const A1: u8 = 11;
pub const A2: u8 = 12;
pub const A3: u8 = 13;
pub const A4: u8 = 14;
pub const A5: u8 = 15;
pub const A6: u8 = 16;
pub const A7: u8 = 17;
pub const S2: u8 = 18;
pub const S3: u8 = 19;
pub const S4: u8 = 20;
pub const S5: u8 = 21;
pub const T3: u8 = 28;
pub const T4: u8 = 29;
pub const T5: u8 = 30;
pub const T6: u8 = 31;

const OP_LOAD: u32 = 0b000_0011;
const OP_IMM: u32 = 0b001_0011;
const OP_STORE: u32 = 0b010_0011;
const OP_REG: u32 = 0b011_0011;
const OP_LUI: u32 = 0b011_0111;
const OP_BRANCH: u32 = 0b110_0011;
const OP_JALR: u32 = 0b110_0111;
const OP_JAL: u32 = 0b110_1111;

const fn reg(r: u8) -> u32 {
    assert!(r < 32, "register number out of range");
    r as u32
}

const fn i_type(imm: i32, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
    assert!(imm >= -2048 && imm <= 2047, "I-type immediate out of range");
    ((imm as u32 & 0xfff) << 20) | (reg(rs1) << 15) | (funct3 << 12) | (reg(rd) << 7) | opcode
}

const fn s_type(imm: i32, rs2: u8, rs1: u8, funct3: u32) -> u32 {
    assert!(imm >= -2048 && imm <= 2047, "S-type immediate out of range");
    let imm = imm as u32 & 0xfff;
    ((imm >> 5) << 25)
        | (reg(rs2) << 20)
        | (reg(rs1) << 15)
        | (funct3 << 12)
        | ((imm & 0x1f) << 7)
        | OP_STORE
}

const fn r_type(funct7: u32, rs2: u8, rs1: u8, funct3: u32, rd: u8) -> u32 {
    (funct7 << 25) | (reg(rs2) << 20) | (reg(rs1) << 15) | (funct3 << 12) | (reg(rd) << 7) | OP_REG
}

const fn b_type(offset: i32, rs2: u8, rs1: u8, funct3: u32) -> u32 {
    assert!(
        offset >= -4096 && offset <= 4094 && offset % 2 == 0,
        "B-type offset out of range or misaligned"
    );
    let imm = offset as u32;
    (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3f) << 25)
        | (reg(rs2) << 20)
        | (reg(rs1) << 15)
        | (funct3 << 12)
        | (((imm >> 1) & 0xf) << 8)
        | (((imm >> 11) & 1) << 7)
        | OP_BRANCH
}

/// `addi rd, rs1, imm`
pub const fn addi(rd: u8, rs1: u8, imm: i32) -> u32 {
    i_type(imm, rs1, 0b000, rd, OP_IMM)
}

/// `lw rd, imm(rs1)`
pub const fn lw(rd: u8, rs1: u8, imm: i32) -> u32 {
    i_type(imm, rs1, 0b010, rd, OP_LOAD)
}

/// `lbu rd, imm(rs1)`
pub const fn lbu(rd: u8, rs1: u8, imm: i32) -> u32 {
    i_type(imm, rs1, 0b100, rd, OP_LOAD)
}

/// `sw rs2, imm(rs1)`
pub const fn sw(rs2: u8, rs1: u8, imm: i32) -> u32 {
    s_type(imm, rs2, rs1, 0b010)
}

/// `sb rs2, imm(rs1)`
pub const fn sb(rs2: u8, rs1: u8, imm: i32) -> u32 {
    s_type(imm, rs2, rs1, 0b000)
}

/// `add rd, rs1, rs2`
pub const fn add(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r_type(0b000_0000, rs2, rs1, 0b000, rd)
}

/// `sub rd, rs1, rs2`
pub const fn sub(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r_type(0b010_0000, rs2, rs1, 0b000, rd)
}

/// `slt rd, rs1, rs2`
pub const fn slt(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r_type(0b000_0000, rs2, rs1, 0b010, rd)
}

/// `mul rd, rs1, rs2` (M extension)
pub const fn mul(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r_type(0b000_0001, rs2, rs1, 0b000, rd)
}

/// `lui rd, imm20` — `imm20` is the raw 20-bit upper immediate (the value
/// that lands in `rd[31:12]`), as written in assembly.
pub const fn lui(rd: u8, imm20: u32) -> u32 {
    assert!(imm20 <= 0xf_ffff, "U-type immediate out of range");
    (imm20 << 12) | (reg(rd) << 7) | OP_LUI
}

/// `beq rs1, rs2, offset` (`offset` in bytes, relative to this instruction)
pub const fn beq(rs1: u8, rs2: u8, offset: i32) -> u32 {
    b_type(offset, rs2, rs1, 0b000)
}

/// `bne rs1, rs2, offset`
pub const fn bne(rs1: u8, rs2: u8, offset: i32) -> u32 {
    b_type(offset, rs2, rs1, 0b001)
}

/// `bge rs1, rs2, offset` (signed)
pub const fn bge(rs1: u8, rs2: u8, offset: i32) -> u32 {
    b_type(offset, rs2, rs1, 0b101)
}

/// `bgeu rs1, rs2, offset` (unsigned)
pub const fn bgeu(rs1: u8, rs2: u8, offset: i32) -> u32 {
    b_type(offset, rs2, rs1, 0b111)
}

/// `jal rd, offset` (`offset` in bytes, relative to this instruction)
pub const fn jal(rd: u8, offset: i32) -> u32 {
    assert!(
        offset >= -(1 << 20) && offset < (1 << 20) && offset % 2 == 0,
        "J-type offset out of range or misaligned"
    );
    let imm = offset as u32;
    (((imm >> 20) & 1) << 31)
        | (((imm >> 1) & 0x3ff) << 21)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 12) & 0xff) << 12)
        | (reg(rd) << 7)
        | OP_JAL
}

/// `jalr rd, imm(rs1)`
pub const fn jalr(rd: u8, rs1: u8, imm: i32) -> u32 {
    i_type(imm, rs1, 0b000, rd, OP_JALR)
}

#[cfg(test)]
mod tests {
    use super::super::decode::{
        decode_32, AluOp, BranchKind, Instruction, LoadKind, MulDivOp, StoreKind,
    };
    use super::*;

    #[test]
    fn every_encoder_round_trips_through_the_crates_own_decoder() {
        let cases: &[(u32, Instruction)] = &[
            (
                addi(SP, SP, -32),
                Instruction::OpImm {
                    rd: SP,
                    rs1: SP,
                    imm: -32,
                    kind: AluOp::Add,
                },
            ),
            (
                lw(RA, SP, 28),
                Instruction::Load {
                    rd: RA,
                    rs1: SP,
                    imm: 28,
                    kind: LoadKind::W,
                },
            ),
            (
                lbu(T2, T0, -1),
                Instruction::Load {
                    rd: T2,
                    rs1: T0,
                    imm: -1,
                    kind: LoadKind::Bu,
                },
            ),
            (
                sw(S5, SP, 4),
                Instruction::Store {
                    rs1: SP,
                    rs2: S5,
                    imm: 4,
                    kind: StoreKind::W,
                },
            ),
            (
                sb(T3, T0, -2048),
                Instruction::Store {
                    rs1: T0,
                    rs2: T3,
                    imm: -2048,
                    kind: StoreKind::B,
                },
            ),
            (
                add(S5, S0, S5),
                Instruction::Op {
                    rd: S5,
                    rs1: S0,
                    rs2: S5,
                    kind: AluOp::Add,
                },
            ),
            (
                sub(A0, S5, S2),
                Instruction::Op {
                    rd: A0,
                    rs1: S5,
                    rs2: S2,
                    kind: AluOp::Sub,
                },
            ),
            (
                slt(A0, T1, T0),
                Instruction::Op {
                    rd: A0,
                    rs1: T1,
                    rs2: T0,
                    kind: AluOp::Slt,
                },
            ),
            (
                mul(S5, S4, S2),
                Instruction::MulDiv {
                    rd: S5,
                    rs1: S4,
                    rs2: S2,
                    kind: MulDivOp::Mul,
                },
            ),
            (
                lui(T0, 0x3fc90),
                Instruction::Lui {
                    rd: T0,
                    imm: 0x3fc9_0000,
                },
            ),
            (
                beq(S5, S0, 64),
                Instruction::Branch {
                    rs1: S5,
                    rs2: S0,
                    imm: 64,
                    kind: BranchKind::Eq,
                },
            ),
            (
                bne(T0, S5, -24),
                Instruction::Branch {
                    rs1: T0,
                    rs2: S5,
                    imm: -24,
                    kind: BranchKind::Ne,
                },
            ),
            (
                bge(ZERO, A0, 4094),
                Instruction::Branch {
                    rs1: ZERO,
                    rs2: A0,
                    imm: 4094,
                    kind: BranchKind::Ge,
                },
            ),
            (
                bgeu(S4, S1, -4096),
                Instruction::Branch {
                    rs1: S4,
                    rs2: S1,
                    imm: -4096,
                    kind: BranchKind::Geu,
                },
            ),
            (
                jal(ZERO, 0x3_ebcc),
                Instruction::Jal {
                    rd: ZERO,
                    imm: 0x3_ebcc,
                },
            ),
            (
                jal(ZERO, -(1 << 20)),
                Instruction::Jal {
                    rd: ZERO,
                    imm: -(1 << 20),
                },
            ),
            (
                jalr(RA, S3, 0),
                Instruction::Jalr {
                    rd: RA,
                    rs1: S3,
                    imm: 0,
                },
            ),
        ];
        for (word, expected) in cases {
            assert_eq!(decode_32(*word), *expected, "word {word:#010x}");
        }
    }
}

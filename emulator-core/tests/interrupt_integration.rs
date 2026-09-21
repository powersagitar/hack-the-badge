//! End-to-end synthetic test for Task 3: a hand-written RV32IMC program that
//! configures the interrupt matrix + SYSTIMER purely through memory-mapped
//! writes (i.e. through `FirmwareBus`'s new SYSTIMER/INTERRUPT_CORE0
//! dispatch tiers, not by poking Rust structs directly), then is stepped via
//! `emulator_core::boot::step_with_interrupts` (the driving loop) until the
//! CPU actually takes the resulting interrupt.
//!
//! This proves the *whole chain* end-to-end, the same way Task 1's
//! `riscv_integration.rs` proved the CPU core against toy programs: SYSTIMER
//! peripheral -> interrupt matrix (`InterruptController::poll`) ->
//! `Cpu::raise_interrupt` -> Task 1's already-merged vectored trap dispatch
//! (`mtvec_base + 4*line`).
//!
//! The program (all addresses/values built at runtime via `lui`+`addi`, the
//! standard RISC-V 32-bit-constant idiom, so nothing here needs a `-0x800`
//! sign-extension carve-out):
//! 1. Sets `mtvec = 0x8000_0000 | 1` (vectored mode).
//! 2. Sets `mstatus.MIE` (realistic firmware behavior; Task 1's `enter_trap`
//!    doesn't currently gate interrupt delivery on it, so this isn't load-
//!    bearing for the test passing, but it's what real firmware would do,
//!    and this test should keep working if a future Task 1 fix adds that
//!    gating with `MIE` already set).
//! 3. Routes `SYSTIMER_TARGET0` to CPU interrupt line 5 via
//!    `SYSTIMER_TARGET0_INT_MAP_REG`.
//! 4. Enables that line in `CPU_INT_ENABLE_REG`.
//! 5. Configures `TARGET0_CONF_REG` for period mode with `PERIOD = 20`.
//! 6. Pulses `COMP0_LOAD_REG` to arm target0 (see
//!    `emulator_core::peripherals::systimer`'s module doc, "Period-mode
//!    arming," for why this is the step that actually gives period mode its
//!    first target).
//! 7. Read-modify-writes `SYSTIMER_CONF_REG` to set `TARGET0_WORK_EN`
//!    (preserving the reset-default `TIMER_UNIT0_WORK_EN`).
//! 8. Enables target0's interrupt in `SYSTIMER_INT_ENA_REG`.
//! 9. Pads out with NOPs so the CPU has valid instructions to keep fetching
//!    while the test harness steps it well past the point the interrupt
//!    should fire.

use emulator_core::boot::step_with_interrupts;
use emulator_core::cpu::{csr_addr, mstatus_bits, Cpu};
use emulator_core::mem::bus::FirmwareBus;
use emulator_core::mem::image::SegmentDescriptor;
use std::sync::Arc;

const PROGRAM_LOAD_ADDR: u32 = 0x4038_0000; // IRAM: RAM-copied, executable.
const SYSTIMER_BASE: u32 = 0x6002_3000;
const INTC_BASE: u32 = 0x600c_2000;
const TARGET_LINE: u32 = 5;
const PERIOD: u32 = 20;

// ---- Minimal RV32I(+Zicsr) encoders, mirroring emulator_core::cpu's own
// private test helpers (not reusable across crates, so reimplemented here).

fn lui(rd: u8, imm: u32) -> u32 {
    (imm & 0xffff_f000) | ((rd as u32) << 7) | 0b0110111
}

fn i_type(opcode: u32, funct3: u32, rd: u8, rs1: u8, imm: i32) -> u32 {
    (((imm as u32) & 0xfff) << 20)
        | ((rs1 as u32) << 15)
        | (funct3 << 12)
        | ((rd as u32) << 7)
        | opcode
}

fn addi(rd: u8, rs1: u8, imm: i32) -> u32 {
    i_type(0b0010011, 0b000, rd, rs1, imm)
}

fn lw(rd: u8, rs1: u8, imm: i32) -> u32 {
    i_type(0b0000011, 0b010, rd, rs1, imm)
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

fn sw(rs1: u8, rs2: u8, imm: i32) -> u32 {
    s_type(0b0100011, 0b010, rs1, rs2, imm)
}

fn r_type(opcode: u32, funct3: u32, funct7: u32, rd: u8, rs1: u8, rs2: u8) -> u32 {
    (funct7 << 25)
        | ((rs2 as u32) << 20)
        | ((rs1 as u32) << 15)
        | (funct3 << 12)
        | ((rd as u32) << 7)
        | opcode
}

fn or_(rd: u8, rs1: u8, rs2: u8) -> u32 {
    r_type(0b0110011, 0b110, 0, rd, rs1, rs2)
}

fn csrrw(rd: u8, csr: u16, rs1: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b001 << 12) | ((rd as u32) << 7) | 0b1110011
}

fn csrrs(rd: u8, csr: u16, rs1: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b010 << 12) | ((rd as u32) << 7) | 0b1110011
}

/// Standard "load an arbitrary 32-bit constant into `rd`" idiom: `lui` for
/// the upper 20 bits, `addi` (sign-extending) for the lower 12 -- computing
/// the `lui` operand so the two sum back to exactly `val` regardless of
/// whether the low 12 bits, viewed as signed, are negative.
fn load_imm32(rd: u8, val: u32) -> Vec<u32> {
    let low12 = (val & 0xfff) as i32;
    let low12_signed = if low12 >= 0x800 {
        low12 - 0x1000
    } else {
        low12
    };
    let upper = val.wrapping_sub(low12_signed as u32) & 0xffff_f000;
    vec![lui(rd, upper), addi(rd, rd, low12_signed)]
}

/// Builds the synthetic firmware program described in the module doc.
/// Returns the encoded instruction words and the (0-indexed) instruction
/// index of the `sw` that pulses `COMP0_LOAD_REG` -- the emulator's
/// `SysTimer::advance` sees `PERIOD` more ticks after this exact step, so
/// the caller can compute precisely which step the interrupt must fire on
/// rather than merely bounding it.
fn build_program() -> (Vec<u32>, usize) {
    let mut prog = Vec::new();

    // 1. mtvec = 0x8000_0000 | vectored(1)
    prog.extend(load_imm32(1, 0x8000_0001));
    prog.push(csrrw(0, csr_addr::MTVEC, 1));

    // 2. mstatus.MIE = 1 (not load-bearing today; see module doc)
    prog.push(addi(4, 0, mstatus_bits::MIE as i32));
    prog.push(csrrs(0, csr_addr::MSTATUS, 4));

    // 3. SYSTIMER_TARGET0_INT_MAP_REG = TARGET_LINE
    prog.extend(load_imm32(2, INTC_BASE + 0x094));
    prog.push(addi(1, 0, TARGET_LINE as i32));
    prog.push(sw(2, 1, 0));

    // 4. CPU_INT_ENABLE_REG = 1 << TARGET_LINE
    prog.extend(load_imm32(2, INTC_BASE + 0x104));
    prog.extend(load_imm32(1, 1 << TARGET_LINE));
    prog.push(sw(2, 1, 0));

    // 5. TARGET0_CONF_REG = PERIOD_MODE(bit30) | PERIOD
    prog.extend(load_imm32(2, SYSTIMER_BASE + 0x34));
    prog.extend(load_imm32(1, (1 << 30) | PERIOD));
    prog.push(sw(2, 1, 0));

    // 6. COMP0_LOAD_REG = 1 (arm target0 -- record this exact instruction's index)
    prog.extend(load_imm32(2, SYSTIMER_BASE + 0x50));
    prog.push(addi(1, 0, 1));
    let comp0_load_step = prog.len();
    prog.push(sw(2, 1, 0));

    // 7. SYSTIMER_CONF_REG |= TARGET0_WORK_EN (bit24), read-modify-write so
    //    the reset-default TIMER_UNIT0_WORK_EN (bit30) survives.
    prog.extend(load_imm32(2, SYSTIMER_BASE));
    prog.push(lw(3, 2, 0));
    prog.extend(load_imm32(1, 1 << 24));
    prog.push(or_(3, 3, 1));
    prog.push(sw(2, 3, 0));

    // 8. SYSTIMER_INT_ENA_REG = 1 (target0's line)
    prog.extend(load_imm32(2, SYSTIMER_BASE + 0x64));
    prog.push(addi(1, 0, 1));
    prog.push(sw(2, 1, 0));

    // 9. Padding: plenty of NOPs so the CPU has valid instructions to keep
    //    fetching for as long as the test needs to step past the interrupt.
    for _ in 0..64 {
        prog.push(addi(0, 0, 0));
    }

    (prog, comp0_load_step)
}

fn build_bus(prog: &[u32]) -> FirmwareBus {
    let mut bytes = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    let len = bytes.len();
    let flash: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
    let segments = [SegmentDescriptor {
        load_addr: PROGRAM_LOAD_ADDR,
        file_offset: 0,
        len,
    }];
    FirmwareBus::from_segments(flash, &segments)
}

#[test]
fn peripheral_to_interrupt_matrix_to_cpu_end_to_end() {
    let (prog, comp0_load_step) = build_program();
    let mut bus = build_bus(&prog);
    let mut cpu = Cpu::new();
    cpu.regs.pc = PROGRAM_LOAD_ADDR;

    // Per SysTimer::advance's edge-detection (see its module doc): the
    // COMP0_LOAD write at step `comp0_load_step` arms target0 =
    // counter-at-that-instant + PERIOD, where counter-at-that-instant
    // equals `comp0_load_step` (SysTimer::advance increments once per
    // step_with_interrupts call, so after N calls the counter is N; the
    // COMP0_LOAD write itself happens *during* cpu.step() at step index
    // `comp0_load_step`, i.e. before that step's own advance() call, so the
    // live counter it reads is exactly `comp0_load_step`). The comparator
    // then edge-fires (INT_RAW latched, poll() returns Some) on the step
    // where the counter reaches that target, i.e. step index
    // `comp0_load_step + PERIOD - 1`; `raise_interrupt` marks it pending,
    // taken at the very next step -- so the trap is taken at step index
    // `comp0_load_step + PERIOD`.
    let expected_trap_step = comp0_load_step + PERIOD as usize;

    const STEP_BUDGET: usize = 200;
    let mut trap_step: Option<usize> = None;
    for i in 0..STEP_BUDGET {
        let info = step_with_interrupts(&mut cpu, &mut bus);
        if info.trap_taken {
            trap_step = Some(i);
            break;
        }
    }

    let trap_step =
        trap_step.expect("expected the CPU to take an interrupt trap within the step budget");
    assert_eq!(
        trap_step, expected_trap_step,
        "interrupt must fire at the precisely-computed step, not merely \"eventually\""
    );

    // mcause: interrupt bit (31) set, low bits == the CPU line SYSTIMER_TARGET0 was routed to.
    assert_eq!(
        cpu.csr.mcause,
        0x8000_0000 | TARGET_LINE,
        "mcause must be the interrupt bit plus the exact CPU line number \
         SYSTIMER_TARGET0_INT_MAP_REG was routed to"
    );

    // pc: mtvec_base + 4*line, per the vectored dispatch scheme (Task 1,
    // confirmed this task against ESP-IDF's vectors_intc.S -- see the task
    // brief).
    assert_eq!(
        cpu.regs.pc,
        0x8000_0000u32.wrapping_add(4 * TARGET_LINE),
        "pc must land at mtvec_base + 4*line"
    );

    // And the peripheral-level state agrees with what actually happened:
    // the CPU took the trap, so by this point INT_RAW is latched (poll()
    // saw it asserted the step before) even though the CPU has since moved
    // on to the trap handler address.
    assert!(
        bus.systimer.target0_pending(),
        "SysTimer's own raw&ena state must still show the fired interrupt \
         (nothing in this test cleared it)"
    );

    // No trap should have fired even one step early.
    let mut bus2 = build_bus(&prog);
    let mut cpu2 = Cpu::new();
    cpu2.regs.pc = PROGRAM_LOAD_ADDR;
    for i in 0..expected_trap_step {
        let info = step_with_interrupts(&mut cpu2, &mut bus2);
        assert!(
            !info.trap_taken,
            "must not trap before step {expected_trap_step} (fired at step {i})"
        );
    }
}

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
//! `Cpu::set_pending_interrupts` -> Task 1's already-merged vectored trap
//! dispatch (`mtvec_base + 4*line`).
//!
//! Milestone 3 Task 4 added the level-delivery, threshold and SYSTEM
//! `FROM_CPU` software-interrupt tests at the end of this file, and the
//! program now also programs the line's priority (step 4b), as ESP-IDF
//! does.
//!
//! The program (all addresses/values built at runtime via `lui`+`addi`, the
//! standard RISC-V 32-bit-constant idiom, so nothing here needs a `-0x800`
//! sign-extension carve-out):
//! 1. Sets `mtvec = 0x8000_0000 | 1` (vectored mode).
//! 2. Sets `mstatus.MIE` (realistic firmware behavior; `Cpu::step` gates
//!    interrupt-taking on this bit, so it *is* load-bearing -- without it,
//!    the interrupt would stay pending forever and this test would time out
//!    against its step budget).
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

    // 4b. CPU_INT_PRI_<TARGET_LINE>_REG = 1, what ESP-IDF's
    //     esprv_intc_int_set_priority does for a level-1 allocation (and
    //     what the threshold test below raises THRESH above).
    prog.extend(load_imm32(2, INTC_BASE + 0x114 + 4 * TARGET_LINE));
    prog.push(addi(1, 0, 1));
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
    // then edge-fires (INT_RAW latched, the source asserted) in the tick
    // after step index `comp0_load_step + PERIOD - 1`, where the counter
    // reaches that target; the next step samples the asserted line at its
    // start (step_with_interrupts' level delivery) and takes it -- so the
    // trap is taken at step index `comp0_load_step + PERIOD`.
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

/// Level delivery (Task 4): once the ISR clears SYSTIMER's `INT_CLR`, the
/// line must de-assert -- re-enabling `MIE` must NOT take the interrupt a
/// second time from a stale sticky pending bit. The next alarm (period
/// mode, `PERIOD` ticks later) legitimately re-asserts it.
#[test]
fn cleared_source_is_not_retaken_but_the_next_alarm_is() {
    use emulator_core::mem::Bus;
    let (prog, comp0_load_step) = build_program();
    let mut bus = build_bus(&prog);
    let mut cpu = Cpu::new();
    cpu.regs.pc = PROGRAM_LOAD_ADDR;

    let expected_trap_step = comp0_load_step + PERIOD as usize;
    let mut taken_at = None;
    for i in 0..200 {
        if step_with_interrupts(&mut cpu, &mut bus).trap_taken {
            taken_at = Some(i);
            break;
        }
    }
    assert_eq!(taken_at, Some(expected_trap_step));
    assert_eq!(
        cpu.csr.mstatus & mstatus_bits::MIE,
        0,
        "trap entry clears MIE"
    );

    // "ISR" (test code): clear the source, return into the NOP padding,
    // and re-enable MIE (what `mret` would do).
    bus.write32(SYSTIMER_BASE + 0x6c, 1); // INT_CLR = target0
    assert!(
        !bus.systimer.target0_pending(),
        "INT_CLR de-asserts the source"
    );
    cpu.regs.pc = PROGRAM_LOAD_ADDR + 4 * (prog.len() as u32 - 64);
    cpu.csr.mstatus |= mstatus_bits::MIE;

    // Well before the next alarm (PERIOD ticks after the first): no trap.
    for i in 0..(PERIOD as usize - 5) {
        let info = step_with_interrupts(&mut cpu, &mut bus);
        assert!(
            !info.trap_taken,
            "level line re-taken at step {i} after clear"
        );
    }
    // The next period's alarm does fire.
    let mut second = false;
    for _ in 0..(2 * PERIOD as usize) {
        if step_with_interrupts(&mut cpu, &mut bus).trap_taken {
            second = true;
            break;
        }
    }
    assert!(second, "the next periodic alarm must raise the line again");
    assert_eq!(cpu.csr.mcause, 0x8000_0000 | TARGET_LINE);
}

/// Threshold masking (Task 4): raising `CPU_INT_THRESH_REG` above the line's
/// priority masks it even with `MIE` set (this is how the FreeRTOS RISC-V
/// port implements critical sections, and what `vectors.S` does on ISR
/// entry: `THRESH = PRI + 1`); lowering it back to the priority lets it
/// through (only priorities strictly below the threshold are masked).
#[test]
fn raised_threshold_masks_a_pending_line_until_lowered() {
    use emulator_core::mem::Bus;
    let (prog, comp0_load_step) = build_program();
    let mut bus = build_bus(&prog);
    let mut cpu = Cpu::new();
    cpu.regs.pc = PROGRAM_LOAD_ADDR;

    // Run the setup, then raise THRESH = 2 (line priority 1 + 1) right
    // before the alarm would fire.
    for _ in 0..(comp0_load_step + 2) {
        step_with_interrupts(&mut cpu, &mut bus);
    }
    bus.write32(INTC_BASE + 0x194, 2);
    for i in 0..(PERIOD as usize + 10) {
        let info = step_with_interrupts(&mut cpu, &mut bus);
        assert!(!info.trap_taken, "masked by threshold, fired at {i}");
    }
    assert_ne!(
        bus.read32(INTC_BASE + 0x110) & (1 << TARGET_LINE),
        0,
        "EIP_STATUS still shows the line pending (ungated)"
    );
    // Threshold == the line's priority: admitted.
    bus.write32(INTC_BASE + 0x194, 1);
    // Level sampling at the start of each step: the very next step takes it.
    assert!(
        step_with_interrupts(&mut cpu, &mut bus).trap_taken,
        "lowering the threshold lets the pending line through"
    );
    assert_eq!(cpu.csr.mcause, 0x8000_0000 | TARGET_LINE);
}

/// SYSTEM `FROM_CPU_0` software interrupt end to end (Task 4): a whole-word
/// store through the bus's byte-splitting path asserts the source, and the
/// first trap taken is on the line MAP[50] routes it to; writing 0 clears it.
#[test]
fn from_cpu_software_interrupt_is_taken_on_its_routed_line_and_clears() {
    use emulator_core::mem::Bus;
    const LINE: u32 = 3;
    let prog = vec![addi(0, 0, 0); 64];
    let mut bus = build_bus(&prog);
    let mut cpu = Cpu::new();
    cpu.regs.pc = PROGRAM_LOAD_ADDR;
    cpu.csr.mtvec = 0x8000_0001;
    cpu.csr.mstatus |= mstatus_bits::MIE;

    bus.write32(INTC_BASE + 50 * 4, LINE); // ETS_FROM_CPU_INTR0_SOURCE = 50
    bus.write32(INTC_BASE + 0x104, 1 << LINE);
    bus.write32(INTC_BASE + 0x114 + 4 * LINE, 1);
    for _ in 0..5 {
        assert!(!step_with_interrupts(&mut cpu, &mut bus).trap_taken);
    }
    bus.write32(0x600c_0028, 1); // vPortYield -> crosscore_int_ll_trigger_interrupt
    assert_eq!(bus.read32(0x600c_0028), 1, "get_state reads back");
    let mut taken = false;
    for _ in 0..4 {
        if step_with_interrupts(&mut cpu, &mut bus).trap_taken {
            taken = true;
            break;
        }
    }
    assert!(taken, "software interrupt must be taken");
    assert_eq!(cpu.csr.mcause, 0x8000_0000 | LINE);

    bus.write32(0x600c_0028, 0); // crosscore_int_ll_clear_interrupt
    cpu.regs.pc = PROGRAM_LOAD_ADDR;
    cpu.csr.mstatus |= mstatus_bits::MIE;
    for _ in 0..20 {
        assert!(!step_with_interrupts(&mut cpu, &mut bus).trap_taken);
    }
}

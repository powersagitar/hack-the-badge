//! End-to-end synthetic test for Task 4: a hand-written RV32IMC program that
//! configures the GPIO peripheral purely through memory-mapped `sw`/`lw`
//! instructions (i.e. through `FirmwareBus`'s new GPIO dispatch tier, not by
//! poking Rust structs directly), bit-bangs a `HC165_LOAD` pulse followed by
//! 7 `HC165_CLK` rising edges, and stores each `GPIO_IN_REG` snapshot it
//! reads to a scratch RAM buffer.
//!
//! This proves the *whole chain* end-to-end, the same way
//! `interrupt_integration.rs` did for Task 3: button-press injection (the
//! test harness calling `Gpio::hc165`'s setters, mirroring what later real
//! UI wiring will do) -> `Hc165` shift-register model -> `Gpio` peripheral's
//! `GPIO_IN_REG` computation -> `FirmwareBus`'s GPIO dispatch tier -> real
//! `lw`/`sw` instructions executed by the CPU core itself. The test asserts
//! against the values the CPU *stored back into RAM* after each read (not
//! against `bus.gpio`'s internal state directly), so a passing test can only
//! mean the CPU genuinely executed the MMIO reads and got the right answer.
//!
//! Pin roles (see `emulator_core::peripherals::gpio`'s module doc for the
//! full pin map and documented bit-order/edge-detection choices):
//! - GPIO20 (`HC165_LOAD`) / GPIO21 (`HC165_CLK`): configured as outputs.
//! - GPIO7 (`HC165_DATA`): left as an input (default), read via `GPIO_IN_REG`.
//!
//! The program stores 8 words to a scratch RAM buffer: the `GPIO_IN_REG`
//! snapshot read immediately after the `LOAD` pulse (before any `CLK` edge),
//! then one more after each of the 7 `CLK` rising edges — matching this
//! module's documented 8-bit shift-out sequence (7 button slots + 1 constant
//! bit) exactly.

use emulator_core::cpu::Cpu;
use emulator_core::mem::bus::FirmwareBus;
use emulator_core::mem::image::SegmentDescriptor;
use emulator_core::mem::Bus;
use std::sync::Arc;

const PROGRAM_LOAD_ADDR: u32 = 0x4038_0000; // IRAM: RAM-copied, executable.
const RAM_SCRATCH_BASE: u32 = 0x3fc9_9c00; // DRAM: separate scratch region.
const GPIO_BASE: u32 = 0x6000_4000;
const OUT_REG: u32 = GPIO_BASE + 0x04;
const ENABLE_REG: u32 = GPIO_BASE + 0x20;
const IN_REG: u32 = GPIO_BASE + 0x3C;
const LOAD_BIT: u32 = 1 << 20;
const CLK_BIT: u32 = 1 << 21;
const HC165_DATA_BIT: u32 = 1 << 7;

// ---- Minimal RV32I encoders, reimplemented locally (mirrors the private
// helpers `interrupt_integration.rs` already has for Task 3 -- not shared
// across test binaries, so each integration test file keeps its own copy).

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

/// Standard "load an arbitrary 32-bit constant into `rd`" idiom.
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

/// Registers: x1 = MMIO address scratch, x2 = value scratch, x3 = persistent
/// RAM-scratch write pointer.
fn build_program() -> Vec<u32> {
    let mut prog = Vec::new();

    // 1. GPIO_ENABLE_REG = (1<<20)|(1<<21): LOAD/CLK as outputs.
    prog.extend(load_imm32(1, ENABLE_REG));
    prog.extend(load_imm32(2, LOAD_BIT | CLK_BIT));
    prog.push(sw(1, 2, 0));

    // 2. GPIO_OUT_REG = LOAD_BIT (idle: LOAD high, CLK low).
    prog.extend(load_imm32(1, OUT_REG));
    prog.extend(load_imm32(2, LOAD_BIT));
    prog.push(sw(1, 2, 0));

    // 3. Pulse LOAD: low, then high (rising edge -> latch).
    prog.extend(load_imm32(2, 0));
    prog.push(sw(1, 2, 0));
    prog.extend(load_imm32(2, LOAD_BIT));
    prog.push(sw(1, 2, 0));

    // 4. x3 = RAM_SCRATCH_BASE.
    prog.extend(load_imm32(3, RAM_SCRATCH_BASE));

    // 5. Read GPIO_IN_REG (pre-clock bit) and store to ram[x3]; x3 += 4.
    prog.extend(load_imm32(1, IN_REG));
    prog.push(lw(4, 1, 0));
    prog.push(sw(3, 4, 0));
    prog.push(addi(3, 3, 4));

    // 6. 7x: toggle CLK low->high (rising edge), then read+store+advance.
    for _ in 0..7 {
        prog.extend(load_imm32(1, OUT_REG));
        prog.extend(load_imm32(2, LOAD_BIT)); // CLK low, LOAD still high
        prog.push(sw(1, 2, 0));
        prog.extend(load_imm32(2, LOAD_BIT | CLK_BIT)); // CLK rising edge
        prog.push(sw(1, 2, 0));

        prog.extend(load_imm32(1, IN_REG));
        prog.push(lw(4, 1, 0));
        prog.push(sw(3, 4, 0));
        prog.push(addi(3, 3, 4));
    }

    // Padding NOPs so stepping slightly past the last real instruction
    // (if the caller's step budget isn't pinned exactly) doesn't fault.
    for _ in 0..8 {
        prog.push(addi(0, 0, 0));
    }

    prog
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
    let mut bus = FirmwareBus::from_segments(flash, &segments);
    bus.add_scratch_ram(RAM_SCRATCH_BASE, 64);
    bus
}

#[test]
fn button_slots_to_shift_register_to_gpio_to_cpu_reads_end_to_end() {
    let prog = build_program();
    let mut bus = build_bus(&prog);
    let mut cpu = Cpu::new();
    cpu.regs.pc = PROGRAM_LOAD_ADDR;

    // Inject button slots via the same setters real UI wiring will later
    // use: press slots 1, 3, 5 (0-indexed); leave 0, 2, 4, 6 released.
    bus.gpio.hc165.set_button(1, true);
    bus.gpio.hc165.set_button(3, true);
    bus.gpio.hc165.set_button(5, true);

    // Step the CPU through the entire hand-written program (no branches, so
    // the exact instruction count is known statically).
    let program_len = prog.len();
    for i in 0..program_len {
        let info = cpu.step(&mut bus);
        assert!(
            !info.trap_taken,
            "synthetic program must execute without faulting (step {i})"
        );
    }

    // Read back the 8 stored GPIO_IN_REG snapshots -- proving the CPU
    // itself executed the MMIO reads, not just that the Rust model's
    // internal state is correct.
    let mut observed_bits = Vec::new();
    for i in 0..8u32 {
        let word = bus.read32(RAM_SCRATCH_BASE + i * 4);
        observed_bits.push(word & HC165_DATA_BIT != 0);
    }

    // Documented bit order (see `peripherals::gpio`'s module doc): slot0,
    // slot1, ..., slot6, then the constant bit. Active-low convention:
    // pressed -> false/low, released -> true/high. Slots 1,3,5 pressed.
    let expected = [true, false, true, false, true, false, true, true];
    assert_eq!(
        observed_bits, expected,
        "CPU-observed HC165_DATA bit sequence must match the injected \
         button slots in the documented ascending bit order, constant bit \
         last"
    );
}

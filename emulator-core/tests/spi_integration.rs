//! End-to-end synthetic test for Task 5: a hand-written RV32IMC program
//! that, purely through real CPU-executed `sw` instructions via
//! `FirmwareBus` (never by poking Rust peripheral structs directly, mirroring
//! `gpio_integration.rs`'s Task 4 approach), configures GPIO0 as an output
//! (the D/C line), drives it low/high around SPI2 register writes to send
//! `CASET`/`RASET`/`RAMWR` commands and a small 2x2 pixel payload, and
//! proves the *whole chain* — GPIO D/C level -> SPI2 `SPI_CMD_REG`'s
//! `SPI_USR` trigger -> `FirmwareBus`'s live GPIO0 read -> `Spi::
//! process_transaction` -> `St7789`'s command interpreter -> reconstructed
//! framebuffer — works end to end through actual CPU execution.
//!
//! Per the task brief, the final assertion reads
//! `bus.spi.framebuffer()`/`Spi::framebuffer()` directly on the Rust side
//! after the CPU has finished executing the program (the framebuffer isn't
//! itself a firmware-readable MMIO register — ST7789 is a write-only-from-
//! firmware's-perspective display), while every *write* that drives the
//! peripheral is a genuine CPU-executed `sw`.

use emulator_core::cpu::Cpu;
use emulator_core::mem::bus::FirmwareBus;
use emulator_core::mem::image::SegmentDescriptor;
use emulator_core::peripherals::spi::{SCREEN_WIDTH, W0_REG as SPI_W0_OFFSET};
use std::sync::Arc;

const PROGRAM_LOAD_ADDR: u32 = 0x4038_0000; // IRAM: RAM-copied, executable.
const GPIO_BASE: u32 = 0x6000_4000;
const GPIO_OUT_REG: u32 = GPIO_BASE + 0x04;
const GPIO_ENABLE_REG: u32 = GPIO_BASE + 0x20;
const DC_BIT: u32 = 1 << 0; // GPIO0, per Task 4's pin map (D/C line).

const SPI_BASE: u32 = 0x6002_4000;
const SPI_CMD_REG: u32 = SPI_BASE;
const SPI_MS_DLEN_REG: u32 = SPI_BASE + 0x1C;
const SPI_W0_REG: u32 = SPI_BASE + SPI_W0_OFFSET;
const SPI_USR_BIT: u32 = 1 << 24;

const CMD_CASET: u8 = 0x2A;
const CMD_RASET: u8 = 0x2B;
const CMD_RAMWR: u8 = 0x2C;

// ---- Minimal RV32I encoders, reimplemented locally (each integration test
// file keeps its own copy, mirroring gpio_integration.rs/
// interrupt_integration.rs's existing convention). ----

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

/// Registers: x1 = MMIO address scratch, x2 = value scratch.
struct ProgramBuilder {
    prog: Vec<u32>,
}

impl ProgramBuilder {
    fn new() -> Self {
        Self { prog: Vec::new() }
    }

    /// `*addr = val` via `x1`/`x2`.
    fn store_word(&mut self, addr: u32, val: u32) {
        self.prog.extend(load_imm32(1, addr));
        self.prog.extend(load_imm32(2, val));
        self.prog.push(sw(1, 2, 0));
    }

    /// Sets GPIO0 (D/C)'s driven output level.
    fn set_dc(&mut self, high: bool) {
        self.store_word(GPIO_OUT_REG, if high { DC_BIT } else { 0 });
    }

    /// Writes `bytes` (up to 4) into `SPI_W0_REG`'s little-endian-within-
    /// word packing (byte0 = LSB), sets `SPI_MS_DLEN_REG` for `bytes.len()`
    /// bytes, then triggers the transaction via `SPI_CMD_REG`'s `SPI_USR`
    /// bit.
    fn spi_transaction(&mut self, bytes: &[u8]) {
        assert!(bytes.len() <= 4, "this test helper only packs one W word");
        let mut w0 = 0u32;
        for (i, &b) in bytes.iter().enumerate() {
            w0 |= (b as u32) << (i * 8);
        }
        self.store_word(SPI_W0_REG, w0);
        let bit_count = (bytes.len() * 8) as u32;
        self.store_word(SPI_MS_DLEN_REG, bit_count - 1); // off-by-one convention
        self.store_word(SPI_CMD_REG, SPI_USR_BIT);
    }

    fn finish(mut self) -> Vec<u32> {
        // Padding NOPs so stepping slightly past the last real instruction
        // doesn't fault.
        for _ in 0..8 {
            self.prog.push(addi(0, 0, 0));
        }
        self.prog
    }
}

fn build_program() -> Vec<u32> {
    let mut b = ProgramBuilder::new();

    // 1. GPIO0 as output.
    b.store_word(GPIO_ENABLE_REG, DC_BIT);

    // 2. CASET: D/C low, command byte 0x2A.
    b.set_dc(false);
    b.spi_transaction(&[CMD_CASET]);
    // D/C high, params xs=0, xe=1 (big-endian 16-bit pairs).
    b.set_dc(true);
    b.spi_transaction(&[0x00, 0x00, 0x00, 0x01]);

    // 3. RASET: D/C low, command byte 0x2B.
    b.set_dc(false);
    b.spi_transaction(&[CMD_RASET]);
    // D/C high, params ys=0, ye=1.
    b.set_dc(true);
    b.spi_transaction(&[0x00, 0x00, 0x00, 0x01]);

    // 4. RAMWR: D/C low, command byte 0x2C.
    b.set_dc(false);
    b.spi_transaction(&[CMD_RAMWR]);
    // D/C high, pixel data streamed across two transactions (proving
    // cross-transaction RAMWR streaming end to end): (0,0)=red,(1,0)=green
    // in the first, (0,1)=blue,(1,1)=white in the second.
    b.set_dc(true);
    b.spi_transaction(&[0xF8, 0x00, 0x07, 0xE0]); // red, green
    b.spi_transaction(&[0x00, 0x1F, 0xFF, 0xFF]); // blue, white

    b.finish()
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
fn gpio_dc_to_spi_trigger_to_st7789_to_framebuffer_end_to_end() {
    let prog = build_program();
    let mut bus = build_bus(&prog);
    let mut cpu = Cpu::new();
    cpu.regs.pc = PROGRAM_LOAD_ADDR;

    let program_len = prog.len();
    for i in 0..program_len {
        let info = cpu.step(&mut bus);
        assert!(
            !info.trap_taken,
            "synthetic program must execute without faulting (step {i})"
        );
    }

    let fb = bus.spi.framebuffer();
    assert_eq!(fb[0], 0xF800, "(0,0) red");
    assert_eq!(fb[1], 0x07E0, "(1,0) green");
    assert_eq!(fb[SCREEN_WIDTH], 0x001F, "(0,1) blue");
    assert_eq!(fb[SCREEN_WIDTH + 1], 0xFFFF, "(1,1) white");
}

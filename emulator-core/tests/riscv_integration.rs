//! Toy-program integration tests: compile small `#![no_std] #![no_main]`
//! programs for `riscv32imc-unknown-none-elf` with the real `rustc`, load
//! the resulting machine code into a trivial `Bus` test double, run it
//! through `Cpu::step()`, and assert the final register/memory state. This
//! is the primary defense against decode bugs the hand-written unit tests
//! (in `emulator-core/src/cpu/*.rs`) might not exercise, since it's real
//! compiler-generated code — including RV32C opcodes chosen by the
//! compiler's own code-size optimization, not opcodes this test's author
//! picked.
//!
//! Requires `rustup target add riscv32imc-unknown-none-elf`. If it is
//! missing, these tests fail loudly with a message explaining what to
//! install, rather than silently skipping.

use emulator_core::cpu::Cpu;
use emulator_core::mem::Bus;
use object::elf::PT_LOAD;
use object::read::elf::{ElfFile32, ProgramHeader};
use object::{LittleEndian, Object};
use std::path::{Path, PathBuf};
use std::process::Command;

const RISCV_TARGET: &str = "riscv32imc-unknown-none-elf";

/// A flat `Vec<u8>`-backed `Bus`: byte 0 of the vec is address 0, etc. Reads
/// past the end return 0; writes past the end are silently dropped. This is
/// intentionally not a real memory map — just enough to load a flat binary
/// at address 0 and observe what the CPU does to a couple of fixed
/// "MMIO-ish" addresses.
struct FlatBus {
    mem: Vec<u8>,
}

impl FlatBus {
    fn new(size: usize) -> Self {
        Self { mem: vec![0; size] }
    }

    fn load_at(&mut self, addr: u32, bytes: &[u8]) {
        let start = addr as usize;
        self.mem[start..start + bytes.len()].copy_from_slice(bytes);
    }
}

impl Bus for FlatBus {
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

/// Flattens a linked ELF: each `PT_LOAD` segment's file bytes go at its
/// physical address, with any gaps zero-filled, so byte 0 of the image is
/// address 0. The image ends at the last file byte: a trailing `.bss`
/// (`p_memsz` past `p_filesz`) adds nothing, and is zero in a fresh
/// `FlatBus` anyway. Also returns the entry point, because the linker
/// script only pins the lowest address of `.text` to 0: rustc/LLVM are free
/// to order the functions within it (generated panic-handling helpers
/// sometimes land before user code), so `_start` need not be first.
fn flatten_elf(bytes: &[u8]) -> (Vec<u8>, u32) {
    let elf = ElfFile32::<LittleEndian>::parse(bytes).expect("parsing a little-endian ELF32");
    let endian = elf.endian();
    let mut image = Vec::new();
    for segment in elf.elf_program_headers() {
        if segment.p_type(endian) != PT_LOAD || segment.p_filesz(endian) == 0 {
            continue;
        }
        let start = segment.p_paddr(endian) as usize;
        let data = segment
            .data(endian, bytes)
            .expect("reading a PT_LOAD segment's file bytes");
        let end = start + data.len();
        if image.len() < end {
            image.resize(end, 0);
        }
        image[start..end].copy_from_slice(data);
    }
    (image, elf.entry() as u32)
}

/// Compiles `fixtures/<name>/src.rs` for `riscv32imc-unknown-none-elf`,
/// links it with `fixtures/<name>/link.ld` (which places `.text` at
/// address 0, entry `_start`), and returns the flat little-endian machine
/// code (see [`flatten_elf`]), ready to load into a `Bus` at address 0, plus
/// the entry point address to set `cpu.regs.pc` to before running.
fn compile_fixture(name: &str) -> (Vec<u8>, u32) {
    // Tests run in parallel and share one output path per fixture, so two
    // `rustc`s would delete each other's intermediate `.rcgu.o`. Serialize.
    static COMPILE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = COMPILE.lock().unwrap_or_else(|e| e.into_inner());

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_dir = manifest_dir.join("tests/fixtures").join(name);
    let src = fixture_dir.join("src.rs");
    let link_script = fixture_dir.join("link.ld");

    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let elf_path = out_dir.join(format!("{name}.elf"));

    let status = Command::new("rustc")
        .args([
            "--target",
            RISCV_TARGET,
            "-C",
            "panic=abort",
            "-C",
            "opt-level=2",
            "-C",
            &format!("link-arg=-T{}", link_script.display()),
            "-C",
            "link-arg=--nmagic",
            "-C",
            "link-arg=-e_start",
            "--crate-type",
            "bin",
            "--edition",
            "2021",
            "-o",
        ])
        .arg(&elf_path)
        .arg(&src)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn rustc: {e}"));
    assert!(
        status.success(),
        "rustc failed to compile fixture `{name}` for {RISCV_TARGET} \
         (is `rustup target add {RISCV_TARGET}` done?)"
    );

    flatten_elf(&std::fs::read(&elf_path).expect("reading the linked ELF"))
}

/// Runs `cpu.step()` until it observes an `EBREAK` trap (our fixtures'
/// convention for "computation done, stop here") or `max_steps` is
/// exceeded.
fn run_until_ebreak(cpu: &mut Cpu, bus: &mut FlatBus, max_steps: usize) {
    use emulator_core::cpu::exception_code;
    for _ in 0..max_steps {
        let info = cpu.step(bus);
        if info.trap_taken && cpu.csr.mcause == exception_code::BREAKPOINT {
            return;
        }
    }
    panic!(
        "program did not hit EBREAK within {max_steps} steps (pc=0x{:x})",
        cpu.regs.pc
    );
}

#[test]
fn toy_sum_loop_computes_correct_sum() {
    let (code, entry) = compile_fixture("toy_sum");

    let mut bus = FlatBus::new(0x3000);
    bus.load_at(0, &code);
    bus.load_at(0x1000, &10u32.to_le_bytes()); // input: n = 10

    let mut cpu = Cpu::new();
    cpu.regs.pc = entry;
    run_until_ebreak(&mut cpu, &mut bus, 10_000);

    let result = bus.read32(0x2000);
    assert_eq!(result, 55, "sum(1..=10) should be 55");
}

#[test]
fn toy_sum_loop_handles_zero_input() {
    let (code, entry) = compile_fixture("toy_sum");

    let mut bus = FlatBus::new(0x3000);
    bus.load_at(0, &code);
    bus.load_at(0x1000, &0u32.to_le_bytes());

    let mut cpu = Cpu::new();
    cpu.regs.pc = entry;
    run_until_ebreak(&mut cpu, &mut bus, 10_000);

    assert_eq!(bus.read32(0x2000), 0);
}

#[test]
fn toy_mul_loop_computes_factorial_and_rem() {
    let (code, entry) = compile_fixture("toy_mul");

    let mut bus = FlatBus::new(0x3000);
    bus.load_at(0, &code);
    bus.load_at(0x1000, &6u32.to_le_bytes()); // input: n = 6

    let mut cpu = Cpu::new();
    cpu.regs.pc = entry;
    run_until_ebreak(&mut cpu, &mut bus, 10_000);

    assert_eq!(
        bus.read32(0x2000),
        720,
        "6! should be 720 (exercises real MUL)"
    );
    assert_eq!(
        bus.read32(0x2004) as i32,
        6 % 3,
        "6 % 3 (exercises real REM)"
    );
}

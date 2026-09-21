// Toy RV32IMC integration-test program: reads an input word from a fixed
// "MMIO" address, computes the sum 1..=n via a genuine runtime loop (the
// input is read with a volatile load so the compiler can't constant-fold
// the whole computation away), writes the result to another fixed address,
// then spins on `ebreak` so the test harness has a clean stopping point.
//
// Compiled by `emulator-core/tests/riscv_integration.rs` for
// `riscv32imc-unknown-none-elf` at test time (not checked in as a binary).
// Deliberately exercises a JALR-based function call, a countdown loop
// (BEQZ/BGEU/branches), and — via normal `-O` compiler code-size
// optimization, not anything hand-picked — a good spread of RV32C
// (compressed) opcodes across all three quadrants: this is real
// compiler-generated machine code, not instructions the test author chose.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

const INPUT_ADDR: u32 = 0x1000;
const OUTPUT_ADDR: u32 = 0x2000;

#[no_mangle]
#[inline(never)]
pub extern "C" fn sum_to(n: u32) -> u32 {
    let mut sum: u32 = 0;
    let mut i: u32 = 1;
    while i <= n {
        sum = sum.wrapping_add(i);
        i += 1;
    }
    sum
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let n = unsafe { core::ptr::read_volatile(INPUT_ADDR as *const u32) };
    let result = sum_to(n);
    unsafe {
        core::ptr::write_volatile(OUTPUT_ADDR as *mut u32, result);
    }
    loop {
        unsafe {
            core::arch::asm!("ebreak");
        }
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

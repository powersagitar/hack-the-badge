// Toy RV32IMC integration-test program exercising the M extension: computes
// n! via a runtime loop (input read with a volatile load so the compiler
// can't constant-fold it), which the compiler lowers to MUL. Also computes
// a signed remainder with REM so both multiply and divide/remainder paths
// get exercised by real compiler output rather than hand-picked opcodes.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

const INPUT_ADDR: u32 = 0x1000;
const OUTPUT_MUL_ADDR: u32 = 0x2000;
const OUTPUT_REM_ADDR: u32 = 0x2004;

#[no_mangle]
#[inline(never)]
pub extern "C" fn factorial(n: u32) -> u32 {
    let mut result: u32 = 1;
    let mut i: u32 = 1;
    while i <= n {
        result = result.wrapping_mul(i);
        i += 1;
    }
    result
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn signed_rem(a: i32, b: i32) -> i32 {
    a % b
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let n = unsafe { core::ptr::read_volatile(INPUT_ADDR as *const u32) };
    let fact = factorial(n);
    let rem = signed_rem(n as i32, 3);
    unsafe {
        core::ptr::write_volatile(OUTPUT_MUL_ADDR as *mut u32, fact);
        core::ptr::write_volatile(OUTPUT_REM_ADDR as *mut i32, rem);
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

//! Generic high-level-emulation (HLE) stub mechanism for fixed-address ROM
//! calls.
//!
//! ## Why this exists
//!
//! ESP32 chips keep a large set of stable, fixed-address API functions in
//! on-chip **mask ROM** — silicon-resident code Espressif guarantees the
//! addresses of across chip revisions (that's what
//! `components/esp_rom/esp32c3/ld/esp32c3.rom.ld` in ESP-IDF *is*: a
//! linker script mapping ~1200 ROM API symbol names to absolute addresses).
//! Real ESP-IDF-compiled firmware therefore calls straight into those
//! addresses, and does so within its first handful of instructions
//! (`rtc_get_reset_reason` at `0x4000_0018`, for instance).
//!
//! We have the badge's *flash* contents (`frontend/public/firmware/factory.bin`), so
//! we can execute everything the app image itself carries. We do **not**
//! have the mask ROM's bytes, and never will via flash dumping — it isn't
//! in flash at all. Without help, the first such call runs `pc` into
//! genuinely unmapped space and (correctly, per Task 2.1) raises
//! `INSTRUCTION_ACCESS_FAULT`, ending boot ~20 instructions in.
//!
//! The standard emulator answer is high-level emulation: don't run the ROM's
//! machine code, *intercept the call* and simulate just enough of the
//! function's observable effect for the caller to proceed. That's what this
//! module provides.
//!
//! ## What this module is and isn't
//!
//! This module is deliberately **chip-agnostic**, matching `crate::cpu`'s
//! module-level rule that the CPU core knows nothing about ESP32-C3
//! specifics: it defines only the *shape* of a stub ([`RomStub`]) and the
//! address→stub table ([`RomStubTable`]). The actual ESP32-C3 addresses and
//! their chosen semantics live in `crate::rom`, one level up, alongside the
//! rest of this crate's SoC knowledge.
//!
//! ## Mechanism (see [`crate::cpu::Cpu::step`] for the call site)
//!
//! Right before `step()` fetches the instruction at `pc`, it looks `pc` up
//! in the CPU's stub table. On a match, **no instruction is fetched,
//! decoded or executed at all** for that step; instead:
//!
//! 1. The stub's [`RomStubEffect`] runs — typically "write a plausible return
//!    value into `a0`/`x10`", the RV32 ABI's integer return register;
//!    sometimes nothing at all (a `void` function); and for the one ROM
//!    function whose *actual* work matters ([`RomStubEffect::Memset`]), the
//!    real thing, through the bus.
//! 2. `pc` is set to `ra`/`x1` — the return address the caller's own
//!    `jal`/`jalr` already deposited there before transferring control.
//!    From the caller's point of view the callee has run and returned.
//!
//! That whole sequence is one step's worth of work: `step()` returns with
//! [`crate::cpu::StepInfo::rom_stub`] set to the intercepted address, and the
//! *next* `step()` resumes normal fetch/decode/execute at the return site.
//!
//! **Default-off, by construction.** [`Cpu::default`](crate::cpu::Cpu) starts
//! with an empty table, and an empty table short-circuits
//! ([`RomStubTable::lookup`] checks [`RomStubTable::is_empty`] first), so
//! every `Cpu` that doesn't explicitly opt in behaves exactly as it did
//! before this mechanism existed — same fetch path, same
//! `INSTRUCTION_ACCESS_FAULT` on unmapped fetches, same everything. Only a
//! caller that deliberately installs a table (see
//! [`crate::cpu::Cpu::set_rom_stubs`] and
//! `crate::boot::boot_from_factory_image_with_rom_stubs`) is affected.
//!
//! ## Interaction with pending interrupts
//!
//! The stub check happens *after* `step()`'s pending-interrupt check, not
//! before: a pending interrupt is architecturally delivered at an
//! instruction boundary, and the boundary immediately before a stubbed
//! "call" is a perfectly good one. So an interrupt raised while `pc` sits on
//! a stub address is taken first (with `mepc` = the stub address), and the
//! stub runs when the handler `mret`s back to it. That's the same ordering a
//! real interrupt-during-a-ROM-call would produce, modulo the ROM function
//! being atomic here.
//!
//! ## Known limitation: `ra` must be valid
//!
//! Redirecting to `ra` assumes control reached the stub address via a
//! standard ABI call (`jal ra, …` / `jalr ra, …`). A *tail* jump to a ROM
//! address (`j` with no link, `jalr x0, …`) would leave `ra` holding some
//! older frame's return address, and this mechanism would return there
//! instead of to the tail-caller's caller. In practice ESP-IDF calls ROM
//! functions normally, and the failure mode is loud rather than silent: a
//! bogus `ra` (`0`, say) sends `pc` somewhere unmapped and raises
//! `INSTRUCTION_ACCESS_FAULT` on the next step, with `mtval` pointing at the
//! bad address.

use std::collections::HashMap;

/// `x1`/`ra` — the RV32 ABI return-address register a stub redirects `pc` to.
pub const REG_RA: u8 = 1;
/// `x10`/`a0` — the RV32 ABI first argument / integer return value.
pub const REG_A0: u8 = 10;
/// `x11`/`a1` — the RV32 ABI second argument.
pub const REG_A1: u8 = 11;
/// `x12`/`a2` — the RV32 ABI third argument.
pub const REG_A2: u8 = 12;
/// `x13`/`a3` — the RV32 ABI fourth argument.
pub const REG_A3: u8 = 13;

/// Upper bound on how many bytes one memory-touching stub
/// ([`RomStubEffect::Memset`] and friends) will write in a single call.
///
/// The ESP32-C3 has 400 KiB of internal SRAM total, so any length beyond this
/// is certainly a garbage argument (a wild pointer, an uninitialized length)
/// rather than a real request. Capping matters because this code runs inside a
/// WASM module driving a browser tab: an unbounded firmware-supplied loop
/// count would hang the page instead of producing a diagnosable stall.
pub const MAX_STUB_MEMORY_BYTES: u32 = 8 * 1024 * 1024;

/// What a stub actually does before returning to `ra`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomStubEffect {
    /// Write this value into `a0`/`x10` — the RV32 ABI integer return
    /// register. `Return(0)` is this project's generic "call succeeded,
    /// returned zero" default.
    Return(u32),
    /// Touch nothing but `pc`. The right choice for a ROM function whose real
    /// signature is `void`, since clobbering `a0` there could corrupt a value
    /// the caller was keeping live across the call.
    Void,
    /// `void *memset(void *dst, int c, size_t n)`: writes the low byte of
    /// `a1` to `n = a2` bytes starting at `dst = a0`, through the bus, and
    /// returns `dst` — which is already sitting in `a0`, so nothing else
    /// needs writing. Length is clamped to [`MAX_STUB_MEMORY_BYTES`].
    ///
    /// Unlike the status-returning stubs, this one has to be *real*: a
    /// `memset` that returned a plausible value without writing the bytes
    /// wouldn't unblock the caller, it would silently corrupt it.
    Memset,
    /// One of libgcc's 64-bit integer helper routines — also *real*, for the
    /// same reason as [`RomStubEffect::Memset`]: these compute values the
    /// caller immediately uses, so a fabricated answer is worse than a fault.
    /// See [`Int64Op`] for the operand/result register convention.
    Int64(Int64Op),
}

/// The libgcc 64-bit integer helpers this project emulates.
///
/// **RV32 ABI**: a 64-bit value occupies an aligned register *pair*, low word
/// first. So the first `u64`/`i64` argument is `(a0, a1)`, the second is
/// `(a2, a3)`, and the 64-bit result is returned in `(a0, a1)`. The shift
/// helpers take a 64-bit value in `(a0, a1)` and a plain 32-bit shift count in
/// `a2`.
///
/// **Divide-by-zero** is undefined behavior in C, and libgcc's own ROM code
/// does whatever it does; this emulator instead mirrors the RISC-V
/// M-extension's *architectural* answers, which the rest of this CPU core
/// already implements for the 32-bit `DIVU`/`REMU` instructions: division by
/// zero yields all-ones, remainder by zero yields the dividend. Consistency
/// with the surrounding core beats matching an unspecified ROM behavior.
///
/// **Signed overflow** (`i64::MIN / -1`) uses wrapping semantics, again
/// matching the M-extension's defined answer (`i64::MIN`) and, critically, not
/// panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Int64Op {
    /// `__udivdi3`
    UDiv,
    /// `__umoddi3`
    UMod,
    /// `__divdi3`
    Div,
    /// `__moddi3`
    Mod,
    /// `__muldi3`
    Mul,
    /// `__ashldi3`
    Shl,
    /// `__lshrdi3`
    LShr,
    /// `__ashrdi3`
    AShr,
}

impl Int64Op {
    /// Applies this operation. `lhs` is the `(a0, a1)` pair; `rhs` is the
    /// `(a2, a3)` pair for the arithmetic ops, or `(shift_count, unused)` for
    /// the shifts.
    pub fn apply(self, lhs: u64, rhs: u64) -> u64 {
        // RISC-V only uses the low 6 bits of a 64-bit shift amount, and
        // Rust's shift operators panic past the width, so mask rather than
        // trusting the caller's value.
        let shamt = (rhs & 0x3f) as u32;
        match self {
            Int64Op::UDiv => lhs.checked_div(rhs).unwrap_or(u64::MAX),
            Int64Op::UMod => lhs.checked_rem(rhs).unwrap_or(lhs),
            Int64Op::Div => {
                if rhs == 0 {
                    u64::MAX
                } else {
                    (lhs as i64).wrapping_div(rhs as i64) as u64
                }
            }
            Int64Op::Mod => {
                if rhs == 0 {
                    lhs
                } else {
                    (lhs as i64).wrapping_rem(rhs as i64) as u64
                }
            }
            Int64Op::Mul => lhs.wrapping_mul(rhs),
            Int64Op::Shl => lhs << shamt,
            Int64Op::LShr => lhs >> shamt,
            Int64Op::AShr => ((lhs as i64) >> shamt) as u64,
        }
    }
}

/// One high-level-emulated ROM function: a name (for diagnostics) and an
/// effect.
///
/// Intentionally pure data — no closures, no `dyn Fn` — so [`RomStubTable`]
/// stays `Copy`/`Clone`/`Debug`/inspectable and the CPU core needs no extra
/// generic parameter. A stub whose effect isn't expressible as one of
/// [`RomStubEffect`]'s variants adds a variant rather than reshaping this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomStub {
    /// The ROM symbol name, exactly as ESP-IDF's `esp32c3.rom*.ld` scripts
    /// spell it. Carried purely for diagnostics/tracing — it has no effect on
    /// execution.
    pub name: &'static str,
    pub effect: RomStubEffect,
}

impl RomStub {
    /// A stub that returns `value` in `a0`.
    pub const fn returning(name: &'static str, value: u32) -> Self {
        Self {
            name,
            effect: RomStubEffect::Return(value),
        }
    }

    /// A stub for a `void` ROM function: returns to `ra` without touching
    /// `a0`.
    pub const fn void(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Void,
        }
    }

    /// A real high-level-emulated `memset` — see [`RomStubEffect::Memset`].
    pub const fn memset(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Memset,
        }
    }

    /// A real high-level-emulated libgcc 64-bit helper — see [`Int64Op`].
    pub const fn int64(name: &'static str, op: Int64Op) -> Self {
        Self {
            name,
            effect: RomStubEffect::Int64(op),
        }
    }
}

/// Address → [`RomStub`] table. An empty table (the default) makes the CPU's
/// stub check a single `is_empty()` branch, so a `Cpu` that never opts in
/// pays essentially nothing and behaves identically to one built before this
/// mechanism existed.
#[derive(Debug, Clone, Default)]
pub struct RomStubTable {
    entries: HashMap<u32, RomStub>,
}

impl RomStubTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `stub` at `addr`, replacing any previous entry there.
    /// Returns `&mut self` so a whole table can be built as one chained
    /// expression.
    pub fn insert(&mut self, addr: u32, stub: RomStub) -> &mut Self {
        self.entries.insert(addr, stub);
        self
    }

    /// The stub registered at `addr`, if any. Short-circuits on an empty
    /// table — see the type-level doc.
    pub fn lookup(&self, addr: u32) -> Option<RomStub> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.get(&addr).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every registered `(address, stub)` pair, sorted by address — for
    /// reporting/diagnostics (`HashMap` iteration order is deliberately
    /// unspecified, so this sorts to stay reproducible).
    pub fn entries_sorted(&self) -> Vec<(u32, RomStub)> {
        let mut out: Vec<(u32, RomStub)> = self.entries.iter().map(|(a, s)| (*a, *s)).collect();
        out.sort_by_key(|(a, _)| *a);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_never_matches() {
        let table = RomStubTable::new();
        assert!(table.is_empty());
        assert_eq!(table.lookup(0x4000_0018), None);
        assert_eq!(table.lookup(0), None);
    }

    #[test]
    fn insert_and_lookup_round_trip() {
        let mut table = RomStubTable::new();
        table.insert(0x4000_0018, RomStub::returning("rtc_get_reset_reason", 1));
        assert_eq!(table.len(), 1);
        let stub = table.lookup(0x4000_0018).expect("registered address");
        assert_eq!(stub.name, "rtc_get_reset_reason");
        assert_eq!(stub.effect, RomStubEffect::Return(1));
        assert_eq!(table.lookup(0x4000_001c), None, "neighbouring address");
    }

    #[test]
    fn constructors_map_to_the_expected_effects() {
        assert_eq!(RomStub::void("ets_delay_us").effect, RomStubEffect::Void);
        assert_eq!(RomStub::memset("memset").effect, RomStubEffect::Memset);
        assert_eq!(RomStub::returning("x", 7).effect, RomStubEffect::Return(7));
    }

    #[test]
    fn int64_ops_match_the_documented_semantics() {
        assert_eq!(Int64Op::UDiv.apply(100, 7), 14);
        assert_eq!(Int64Op::UMod.apply(100, 7), 2);
        assert_eq!(Int64Op::Div.apply(-100i64 as u64, 7), -14i64 as u64);
        assert_eq!(Int64Op::Mod.apply(-100i64 as u64, 7), -2i64 as u64);
        assert_eq!(Int64Op::Mul.apply(0x1_0000_0000, 3), 0x3_0000_0000);
        assert_eq!(Int64Op::Shl.apply(1, 40), 1u64 << 40);
        assert_eq!(Int64Op::LShr.apply(1u64 << 40, 40), 1);
        assert_eq!(Int64Op::AShr.apply(-1024i64 as u64, 4), -64i64 as u64);
    }

    #[test]
    fn int64_ops_are_panic_free_on_the_undefined_cases() {
        // Divide by zero: the RISC-V M-extension's architectural answers, per
        // `Int64Op`'s doc -- and, critically, no panic.
        assert_eq!(Int64Op::UDiv.apply(42, 0), u64::MAX);
        assert_eq!(Int64Op::Div.apply(42, 0), u64::MAX);
        assert_eq!(Int64Op::UMod.apply(42, 0), 42);
        assert_eq!(Int64Op::Mod.apply(42, 0), 42);
        // Signed overflow.
        assert_eq!(
            Int64Op::Div.apply(i64::MIN as u64, -1i64 as u64),
            i64::MIN as u64
        );
        assert_eq!(Int64Op::Mod.apply(i64::MIN as u64, -1i64 as u64), 0);
        // Over-wide shift counts are masked rather than panicking.
        assert_eq!(Int64Op::Shl.apply(1, 64), 1);
        assert_eq!(Int64Op::Shl.apply(1, u64::MAX), 1u64 << 63);
        assert_eq!(Int64Op::LShr.apply(u64::MAX, 100), u64::MAX >> 36);
    }

    #[test]
    fn entries_sorted_is_ordered_by_address() {
        let mut table = RomStubTable::new();
        table
            .insert(0x4000_0528, RomStub::returning("c", 0))
            .insert(0x4000_0018, RomStub::returning("a", 1))
            .insert(0x4000_0050, RomStub::returning("b", 0));
        let addrs: Vec<u32> = table.entries_sorted().iter().map(|(a, _)| *a).collect();
        assert_eq!(addrs, vec![0x4000_0018, 0x4000_0050, 0x4000_0528]);
    }
}

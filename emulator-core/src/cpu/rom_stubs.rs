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
//! ## What this mechanism cannot do: call back into firmware
//!
//! A stub's effect runs atomically inside one `step()`, so it can never run
//! guest code. A ROM function that must call a firmware-supplied function
//! pointer in the middle of its work (ROM libc `qsort` calling `compar`) can't
//! be a stub. Those are instead real, hand-assembled RV32 code the CPU
//! executes, mapped at fixed ROM addresses as read-only
//! `crate::mem::bus::RomCodeBlob`s (Milestone 3 Task D6; see `crate::rom`'s
//! module doc, entry 14). The two mechanisms are complementary: every ROM
//! address is a stub, a mapped code blob, or unmapped (fetch traps).
//!
//! ## Mechanism (see [`crate::cpu::Cpu::step`] for the call site)
//!
//! Right before `step()` fetches the instruction at `pc`, it looks `pc` up
//! in the CPU's stub table. On a match, **no instruction is fetched,
//! decoded or executed at all** for that step; instead:
//!
//! 1. The stub's [`RomStubEffect`] runs — typically "write a plausible return
//!    value into `a0`/`x10`", the RV32 ABI's integer return register;
//!    sometimes nothing at all (a `void` function); and for the ROM
//!    functions whose *actual* work matters, the real thing: a real
//!    register-only computation for [`RomStubEffect::Int64`] (and
//!    [`RomStubEffect::Int32Unary`]/[`RomStubEffect::SoftDouble`]), or a real
//!    effect through the bus for [`RomStubEffect::Memset`],
//!    [`RomStubEffect::Memcpy`], [`RomStubEffect::BusRegisterWrite`],
//!    [`RomStubEffect::BusRegisterWrites`] and [`RomStubEffect::StoreWords`]
//!    (plus the read-only libc effects `Strlen`/`Memcmp`/`Strncmp`/`Strcmp`/`Strspn`/`Strcspn`/`Strchr`/`DivT`, the writing
//!    `Strncpy`/`Strlcat`/`Strlcpy`/`Strcpy`, and [`RomStubEffect::Md5`], an MD5 whose context lives in guest
//!    memory).
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

use crate::mem::Bus;
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
/// `x14`/`a4` — the RV32 ABI fifth argument.
pub const REG_A4: u8 = 14;
/// `x15`/`a5` — the RV32 ABI sixth argument.
pub const REG_A5: u8 = 15;
/// `x16`/`a6` — the RV32 ABI seventh argument.
pub const REG_A6: u8 = 16;
/// `x17`/`a7` — the RV32 ABI eighth (and last register-passed) argument.
pub const REG_A7: u8 = 17;
/// `x2`/`sp` — the RV32 ABI stack pointer, where a call's 9th-and-later
/// arguments spill to (`sp`, `sp+4`, `sp+8`, ...).
pub const REG_SP: u8 = 2;

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
    /// One of libgcc's unary 32-bit bit-counting helpers: reads `a0`, writes
    /// the result to `a0`. See [`Int32UnaryOp`]. Real, not fabricated, for
    /// the same reason as [`RomStubEffect::Int64`].
    Int32Unary(Int32UnaryOp),
    /// One of libgcc's soft-float `double` helpers: reads its operand
    /// register pairs, writes the result to `a0` (and `a1` for a `double`).
    /// See [`SoftDoubleOp`]. Real, not fabricated, for the same reason as
    /// [`RomStubEffect::Int64`] (Milestone 4 Task D-M4-1).
    SoftDouble(SoftDoubleOp),
    /// One of libgcc's soft-float `float` helpers: reads `a0` (and `a1`),
    /// writes the result to `a0`. See [`SoftFloatOp`]. Real, for the same
    /// reason as [`RomStubEffect::SoftDouble`].
    SoftFloat(SoftFloatOp),
    /// `void *memcpy(void *dst, const void *src, size_t n)`: copies `n = a2`
    /// bytes from `src = a1` to `dst = a0`, through the bus, one byte at a
    /// time (this project's whole `Bus` interface is byte/half/word
    /// addressed with no bulk-copy primitive, and a byte-at-a-time copy is
    /// simplest and gives correct results regardless of alignment), and
    /// returns `dst` — already sitting in `a0`, so nothing else needs
    /// writing. Length is clamped to [`MAX_STUB_MEMORY_BYTES`], same as
    /// [`RomStubEffect::Memset`].
    ///
    /// Real `memcpy`'s behavior is undefined for overlapping `dst`/`src`
    /// ranges (that's what `memmove` is for), so a forward byte-by-byte copy
    /// is a conforming implementation regardless of whether the ranges
    /// happen to overlap.
    ///
    /// Real, not a generic status stub, for the same reason as
    /// [`RomStubEffect::Memset`]: a `memcpy` that returned a plausible value
    /// without copying the bytes wouldn't unblock the caller, it would
    /// silently corrupt it.
    Memcpy,
    /// A runtime-computed 32-bit peripheral-register write through the bus,
    /// driven entirely by the ROM call's own argument registers plus a base
    /// address the chip-specific table supplies. See [`BusRegisterWrite`]
    /// for the address/value computation and [`BusRegisterOp`] for the three
    /// write shapes.
    ///
    /// This is the mechanism for a ROM stub whose real effect is "write a
    /// register a peripheral model already exposes through `Bus`" rather
    /// than a status/data return or a raw memory copy — e.g. routing an
    /// interrupt-matrix MAP register, or setting/clearing bits in a shared
    /// enable/type register. Like [`RomStubEffect::Void`], it never touches
    /// `a0`: every ROM function using this effect has a `void` C signature,
    /// so a stray value in the caller's kept-alive `a0` must survive the
    /// call.
    ///
    /// Deliberately **chip-agnostic**: this variant and [`BusRegisterWrite`]
    /// only know "read some argument registers, compute an address and a new
    /// word, write it through the bus" — never which peripheral or SoC that
    /// resolves to. `crate::rom` supplies the actual ESP32-C3 base addresses
    /// (from `crate::mem::soc`/`crate::peripherals::intc` constants), per
    /// this module's chip-agnostic/chip-specific split.
    BusRegisterWrite(BusRegisterWrite),
    /// Several [`BusRegisterWrite`]s run in order as one atomic ROM call
    /// (Milestone 3 Task D11: `gpio_matrix_out` stores the pad's
    /// `GPIO_FUNCn_OUT_SEL_CFG_REG`, then sets its `GPIO_ENABLE_W1TS_REG`
    /// bit). **All-or-nothing on index bounds**: every indexed write's index
    /// is checked against its `index_limit` before any write happens, and if
    /// one is out of range the whole call is dropped -- no bus access at all,
    /// one count in `Cpu::rom_stub_index_drops`. That mirrors a ROM body
    /// whose early-return guard skips all of its stores, and means an
    /// unindexed later write (the W1TS store, whose bit comes from the same
    /// guarded register) is never half-applied. Never touches `a0`, like
    /// [`RomStubEffect::BusRegisterWrite`].
    BusRegisterWrites(&'static [BusRegisterWrite]),
    /// ROM libc's `char *itoa(int value, char *str, int base)`: writes the
    /// NUL-terminated string [`compute_itoa`] computes for `value = a0`/
    /// `base = a2` starting at `str = a1`, through the bus, then sets `a0`
    /// to `str` on a valid base or `0` (`NULL`) on an invalid one — see
    /// [`compute_itoa`]'s doc for the exact newlib semantics mirrored.
    ///
    /// Unlike [`RomStubEffect::Memset`]/[`RomStubEffect::Memcpy`], the
    /// return value is *not* already sitting in `a0` (the first argument is
    /// `value`, not `str`), so this effect writes `a0` explicitly. Real,
    /// not a generic status stub, for the same reason as `memset`/`memcpy`:
    /// the caller uses both the written string and the returned pointer.
    Itoa,
    /// `char *strcat(char *dst, const char *src)`: appends NUL-terminated
    /// `src = a1` onto the end of NUL-terminated `dst = a0` -- found by
    /// scanning `dst` for its own terminator, then copying `src` (including
    /// its own NUL) there, through the bus -- and returns `dst`, already
    /// sitting in `a0` (same as [`RomStubEffect::Memcpy`]/
    /// [`RomStubEffect::Memset`], so nothing else needs writing). Mirrors
    /// newlib's `strcat.c` slow/portable path (`newlib/libc/string/strcat.c`).
    ///
    /// Real, not a generic status stub, for the same reason as `memcpy`.
    /// Both the dst-NUL-scan and the src-copy are capped at
    /// [`MAX_STUB_MEMORY_BYTES`] bytes -- unlike the fixed-length stubs
    /// above, this one's "length" is data-dependent (found by scanning for
    /// a NUL, not given as an argument), so a corrupt/unterminated string
    /// could otherwise hang this emulator forever.
    Strcat,
    /// `size_t strlen(const char *s)`: scans NUL-terminated `s = a0` through
    /// the bus and writes the length to `a0`. The scan is capped at
    /// [`MAX_STUB_MEMORY_BYTES`], like [`RomStubEffect::Strcat`]'s. Real, not
    /// a generic status stub: the caller uses the length.
    Strlen,
    /// `int memcmp(const void *s1, const void *s2, size_t n)`: compares `n =
    /// a2` bytes of `s1 = a0` and `s2 = a1` as `unsigned char`, through the
    /// bus, returning in `a0` the difference of the first differing byte pair
    /// (`s1[i] - s2[i]`, sign-extended) or 0. Matches the ROM's own code
    /// (`sub a0, a5, a3` on two `lbu` values; its word-at-a-time fast path
    /// only skips equal words, so gives the same answer). `n` is capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real: the caller branches on the result.
    Memcmp,
    /// `int strncmp(const char *s1, const char *s2, size_t n)`: compares at
    /// most `n = a2` bytes of `s1 = a0`/`s2 = a1` as `unsigned char`,
    /// stopping after a differing pair or a shared NUL, and returns
    /// `s1[i] - s2[i]` for the last pair examined (0 if `n == 0` or the
    /// strings match), in `a0`. Mirrors the ROM's disassembly
    /// (`0x40058fa6`). `n` capped at [`MAX_STUB_MEMORY_BYTES`].
    Strncmp,
    /// `char *strncpy(char *dst, const char *src, size_t n)`: newlib's
    /// `strncpy` (`newlib/libc/string/strncpy.c`): copies bytes of `src = a1`
    /// to `dst = a0` through the bus until a NUL has been copied or `n = a2`
    /// bytes are written; if the NUL came before `n`, pads `dst` with NULs up
    /// to `n`. Does not terminate when `n <= strlen(src)`. Returns `dst`
    /// (already in `a0`). `n` capped at [`MAX_STUB_MEMORY_BYTES`]. Real: the
    /// ROM entry is a `j` trampoline to newlib code, same as `strncmp`.
    Strncpy,
    /// `int strcmp(const char *s1, const char *s2)`: compares `s1 = a0` and
    /// `s2 = a1` as `unsigned char` through the bus until they differ or both
    /// hit NUL, returning `s1[i] - s2[i]` of the last pair examined in `a0`
    /// (same convention as [`RomStubEffect::Strncmp`]). Scan capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real: the caller branches on the result.
    Strcmp,
    /// `int strcasecmp(const char *s1, const char *s2)`: as
    /// [`RomStubEffect::Strcmp`], but each byte is first lowered by the ROM's
    /// C-locale `tolower` (ROM ELF `strcasecmp` at `0x4005_8afa` inlines it:
    /// `_ctype_[c + 1] & 3 == _U` adds 0x20; the table marks only `A`..`Z`
    /// upper case). Returns the lowered `s1[i] - s2[i]` of the last pair
    /// examined, stopping at a difference or at `s2`'s NUL. Scan capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real: the firmware matches role names with
    /// it.
    Strcasecmp,
    /// `size_t strlcat(char *dst, const char *src, size_t siz)`: the BSD
    /// `strlcat` newlib ships (`newlib/libc/string/strlcat.c`, OpenBSD's):
    /// finds `dst = a0`'s NUL within its first `siz = a2` bytes, appends
    /// `src = a1` through the bus copying at most `siz - strlen(dst) - 1`
    /// bytes, and NUL-terminates unless `siz <= strlen(dst)` (no room, or no
    /// NUL within `siz`: nothing is written). Returns
    /// `min(siz, strlen(dst)) + strlen(src)` in `a0`, the length it tried to create. Every scan
    /// and the copy are capped at [`MAX_STUB_MEMORY_BYTES`]. Real: the ROM
    /// entry is a jump to newlib code, and the caller uses the string.
    Strlcat,
    /// `size_t strlcpy(char *dst, const char *src, size_t siz)`: the BSD
    /// `strlcpy` the ROM ships (ROM ELF `strlcpy` at `0x4005_8e4e`, OpenBSD's):
    /// copies at most `siz = a2` - 1 bytes of `src = a1` to `dst = a0` through
    /// the bus and NUL-terminates, unless `siz` is 0 (nothing is written).
    /// Returns `strlen(src)` in `a0`, the length it tried to create. The scan
    /// and the copy are capped at [`MAX_STUB_MEMORY_BYTES`]. Real, and a
    /// bus-effect stub rather than a guest code blob because it needs no
    /// callback into firmware, like `strlcat`: the console's line copy
    /// (`0x4206_0074`) uses the result and the copied bytes.
    Strlcpy,
    /// `char *strrchr(const char *s, int c)`: newlib's `strrchr` (ROM slot
    /// `__call_strrchr`, `esp32c3.rom.libc.ld` `strrchr = 0x40000408`).
    /// Returns the address of the last byte of `s = a0` equal to `(char)c =
    /// a1`, or 0 if none; the terminating NUL counts, so `c == 0` returns
    /// the end of the string. Scan capped at [`MAX_STUB_MEMORY_BYTES`].
    Strrchr,
    /// `long strtol(const char *nptr, char **endptr, int base)`: newlib's
    /// `strtol` (ROM slot `__call_strtol`, `esp32c3.rom.libc.ld`
    /// `strtol = 0x40000454`). Skips C-locale whitespace, an optional sign,
    /// and for base 0 or 16 a `0x`/`0X` prefix (base 0 then picks 8 for a
    /// leading `0`, else 10); accepts digits and letters below `base`;
    /// saturates at `LONG_MAX`/`LONG_MIN` on overflow. If no digit is
    /// consumed `*endptr = nptr` and 0 is returned; otherwise `*endptr`
    /// points past the last digit (written only when `endptr = a1` is
    /// non-NULL). `errno` (ERANGE/EINVAL) is not set: the ROM keeps it in a
    /// per-task reent struct the emulator does not model, and the observed
    /// caller (`put`'s size parse) checks only `endptr`. Scan capped at
    /// [`MAX_STUB_MEMORY_BYTES`].
    Strtol,
    /// `int atoi(const char *s)`: the ROM's newlib `atoi` (slot
    /// `__call_atoi`, `esp32c3.rom.newlib.ld` `atoi = 0x4000044c`), whose
    /// body (ROM ELF `0x4003_1dac`) is `strtol(s, NULL, 10)`: exactly
    /// [`RomStubEffect::Strtol`] with `endptr = NULL` and base 10 (so an
    /// out-of-range value saturates to `INT_MAX`/`INT_MIN`).
    Atoi,
    /// `size_t strspn(const char *s, const char *set)`: newlib's
    /// `strspn.c`: the length of the leading run of `s = a0` made only of
    /// bytes in NUL-terminated `set = a1`, in `a0`. Scans capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real: littlefs uses it to walk paths.
    Strspn,
    /// `size_t strcspn(const char *s, const char *set)`: newlib's
    /// `strcspn.c`: the length of the leading run of `s = a0` with no byte
    /// in `set = a1` (stops at `s`'s NUL), in `a0`. Scans capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real, like [`RomStubEffect::Strspn`].
    Strcspn,
    /// `char *strchr(const char *s, int c)`: C11 §7.24.5.2, as the ROM's
    /// newlib code (`0x40058bf2`) does it: `c` is cast to `unsigned char`,
    /// then `s = a0` is scanned through the bus for it, the terminating NUL
    /// included (so `c == 0` finds the NUL). Returns its address in `a0`,
    /// or 0 (NULL). Scan capped at [`MAX_STUB_MEMORY_BYTES`] (NULL past
    /// the cap). Real: the caller uses the pointer.
    Strchr,
    /// `char *strcpy(char *dst, const char *src)`: C11 §7.24.2.3, as the
    /// ROM's newlib code (`0x40058d2e`) does it: copies `src = a1` through
    /// the bus to `dst = a0` up to and including its NUL, returning `dst`
    /// (already in `a0`). Capped at [`MAX_STUB_MEMORY_BYTES`] bytes like
    /// [`RomStubEffect::Strcat`]'s copy. Real: the caller uses the string.
    Strcpy,
    /// `void *memchr(const void *s, int c, size_t n)`: scans `n = a2` bytes
    /// of `s = a0` through the bus for the byte `(unsigned char)c` (`c =
    /// a1`), returning in `a0` the address of the first match, or 0 (NULL)
    /// if none. Mirrors the ROM's newlib code (`0x40058758`: `zext.b a1,a1`,
    /// then a byte-at-a-time `lbu`/`beq` loop to `s + n`). `n` is capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real: the caller uses the pointer.
    Memchr,
    /// `void *memmove(void *dst, const void *src, size_t n)`: copies `n =
    /// a2` bytes from `src = a1` to `dst = a0` through the bus so that
    /// overlapping ranges come out right. Mirrors the ROM's newlib code
    /// (`0x40058870`): byte-at-a-time, backward when `src < dst < src + n`,
    /// forward otherwise. Returns `dst`, already in `a0`. `n` is capped at
    /// [`MAX_STUB_MEMORY_BYTES`]. Real, for the same reason as
    /// [`RomStubEffect::Memcpy`].
    Memmove,
    /// `div_t div(int numer, int denom)`: `a0 = numer`, `a1 = denom`; the
    /// 8-byte `div_t {int quot; int rem;}` is returned in `(a0, a1)` per the
    /// RV32 psABI. The ROM (`0x400319c6`) computes it with the M-extension
    /// `div`/`rem` and two fixups that are dead code under RISC-V's
    /// truncating semantics, so this is plain `div`/`rem` with the
    /// M-extension's defined edge cases (divide by zero: quot -1, rem
    /// numer; `i32::MIN / -1`: quot `i32::MIN`, rem 0), same as this CPU
    /// core's own `DIV`/`REM`.
    DivT,
    /// `int ets_printf(const char *fmt, ...)` (ROM's own vararg formatter,
    /// and the target of `esp_rom_printf`/every early-boot `ESP_EARLY_LOG*`
    /// line -- `esp32c3.rom.ld: ets_printf = 0x40000040;`,
    /// `esp32c3.rom.api.ld: PROVIDE(esp_rom_printf = ets_printf);`).
    ///
    /// Milestone 3 Task 7's orchestrator ruling: the firmware calls this
    /// address directly (confirmed by disassembly -- see `crate::rom`'s
    /// module doc, entry 7), so a `Return(0)` stub silently drops every
    /// early boot-log line rather than formatting and emitting it. This
    /// effect gives `ets_printf` a real HLE implementation
    /// ([`compute_printf`]): it formats `a0`'s C format string against the
    /// RV32 ILP32 varargs in `a1..a7`/the stack (see [`VarargCursor`]),
    /// writes each output byte through `bus.write8(sink_addr, ..)` -- the
    /// same path real FIFO/putc output already takes, so the console
    /// buffer, its cap, and the WASM passthrough all apply unchanged -- and
    /// returns the number of characters written in `a0`, matching
    /// `ets_sys.h`'s documented `int ets_printf(const char *fmt, ...)`
    /// signature ("@return int : the length printed to the output
    /// device.").
    ///
    /// `sink_addr` is chip-specific (the ESP32-C3's USB-Serial-JTAG EP1
    /// FIFO data register in this project), so it's supplied by
    /// `crate::rom`, not hardcoded here -- this effect's mechanism (parse a
    /// C format string, walk varargs per the RV32 ILP32 calling
    /// convention, write bytes through the bus) has nothing ESP32-C3-
    /// specific about it.
    Printf {
        /// The MMIO byte address each formatted output byte is written to
        /// via `bus.write8` -- one byte per write, matching how a real
        /// putc-style TX register is driven one character at a time.
        sink_addr: u32,
    },
    /// A `void` ROM function whose whole effect is storing words to fixed
    /// addresses, once per [`WordStore`] entry, in order, through the bus.
    /// Each word comes from an argument register, either the word it points
    /// at ([`WordSource::Pointee`], Milestone 3 Task D9:
    /// `esp_rom_newlib_init_common_mutexes(a0, a1)` copies `*a0` and `*a1`
    /// into two ROM-internal statics) or the register's own value
    /// ([`WordSource::Register`], Task D10: `ets_apb_backup_init_lock_func(a0,
    /// a1)` stores the two function pointers themselves). Like
    /// [`RomStubEffect::BusRegisterWrite`] it never touches `a0`: the real
    /// functions are `void`, and the state the ROM keeps (which later ROM
    /// code consumes) lands in ordinary emulated memory instead of being
    /// dropped. Chip-agnostic: `crate::rom` supplies the fixed addresses.
    /// (Named `LoadStoreWords` before Task D10 added the register-value
    /// source.)
    StoreWords(&'static [WordStore]),
    /// One of the three calls of an MD5 implementation whose context lives
    /// in guest memory as Colin Plumb's public-domain `struct MD5Context {
    /// uint32_t buf[4]; uint32_t bits[2]; uint8_t in[64]; }` (88 bytes),
    /// the shape the ESP32-C3 mask ROM's `MD5Init`/`MD5Update`/`MD5Final`
    /// use (Milestone 3 Task D13; `crate::rom` has the addresses and
    /// citations). See [`Md5Op`] for each call's registers, and
    /// [`apply_md5`] for how it runs: the whole context is loaded through
    /// the bus into a [`crate::md5::Md5Context`], the call is applied, and
    /// the bytes the real function writes are stored back. All state stays
    /// in guest memory, so a partial block buffered by one `MD5Update` is
    /// there for the next. All three are `void`: `a0` is left untouched.
    /// Chip-agnostic: nothing here knows which chip's ROM this is.
    Md5(Md5Op),
}

/// The three MD5 calls of [`RomStubEffect::Md5`], with Plumb's argument
/// order (`esp_rom_md5.h` declares the same order for `esp_rom_md5_*`, which
/// `esp32c3.rom.api.ld` aliases straight to these):
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Md5Op {
    /// `void MD5Init(struct MD5Context *ctx)`, `ctx = a0`: writes the RFC
    /// 1321 initial chaining value to `buf` and zeroes `bits`; `in` is not
    /// touched (the first 24 bytes only).
    Init,
    /// `void MD5Update(struct MD5Context *ctx, const unsigned char *buf,
    /// unsigned len)`, `ctx = a0`, `buf = a1`, `len = a2`: hashes `len`
    /// bytes read through the bus. `len` is clamped to
    /// [`MAX_STUB_MEMORY_BYTES`], like the libc stubs'.
    Update,
    /// `void MD5Final(unsigned char digest[16], struct MD5Context *ctx)`,
    /// `digest = a0`, `ctx = a1` (digest first): pads, writes the 16-byte
    /// digest, and zeroes all 88 bytes of the context.
    Final,
}

/// Runs one [`RomStubEffect::Md5`] call against guest memory: `a0`..`a2`
/// are the call's argument registers (see [`Md5Op`]).
pub fn apply_md5<B: Bus>(op: Md5Op, a0: u32, a1: u32, a2: u32, bus: &mut B) {
    use crate::md5::{Md5Context, CONTEXT_LEN, INIT_WRITE_LEN};
    let ctx_addr = match op {
        Md5Op::Init | Md5Op::Update => a0,
        Md5Op::Final => a1,
    };
    let mut raw = [0u8; CONTEXT_LEN];
    for (i, byte) in raw.iter_mut().enumerate() {
        *byte = bus.read8(ctx_addr.wrapping_add(i as u32));
    }
    let mut ctx = Md5Context::from_bytes(&raw);
    let written = match op {
        Md5Op::Init => {
            ctx.init();
            INIT_WRITE_LEN
        }
        Md5Op::Update => {
            // Feed the message a block at a time (MD5Update is split-
            // invariant), so a large `len` needs no host-side copy of it.
            let len = a2.min(MAX_STUB_MEMORY_BYTES);
            let mut chunk = [0u8; crate::md5::BLOCK_LEN];
            let mut done = 0u32;
            while done < len {
                let n = (len - done).min(chunk.len() as u32);
                for (i, byte) in chunk[..n as usize].iter_mut().enumerate() {
                    *byte = bus.read8(a1.wrapping_add(done).wrapping_add(i as u32));
                }
                ctx.update(&chunk[..n as usize]);
                done += n;
            }
            CONTEXT_LEN
        }
        Md5Op::Final => {
            let digest = ctx.finalize();
            for (i, byte) in digest.iter().enumerate() {
                bus.write8(a0.wrapping_add(i as u32), *byte);
            }
            CONTEXT_LEN
        }
    };
    for (i, byte) in ctx.to_bytes()[..written].iter().enumerate() {
        bus.write8(ctx_addr.wrapping_add(i as u32), *byte);
    }
}

/// One `*dst = <word>` store for [`RomStubEffect::StoreWords`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordStore {
    /// Where the stored word comes from.
    pub src: WordSource,
    /// The fixed guest address the word is stored to.
    pub dst: u32,
}

/// The source of one [`WordStore`]'s word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordSource {
    /// The word the register points at (`*a[reg]`; a `lw` from it).
    Pointee(u8),
    /// The register's own value (`a[reg]`).
    Register(u8),
}

/// The address/value computation for [`RomStubEffect::BusRegisterWrite`].
///
/// The word address touched is `base + a[index_reg] * 4` when `index_reg` is
/// `Some` (a ROM call that indexes into an array of same-sized registers,
/// one per source/line — e.g. one MAP register per interrupt source, one
/// `CPU_INT_PRI_<n>_REG` per CPU interrupt line), or just `base` when
/// `index_reg` is `None` (a single fixed-address register shared across
/// calls, e.g. `CPU_INT_ENABLE_REG`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusRegisterWrite {
    pub base: u32,
    pub index_reg: Option<u8>,
    /// Exclusive upper bound on `a[index_reg]` (the array's length in
    /// words). An index `>= limit` is a wild guest value: the call is
    /// dropped (no bus access) and counted in `Cpu::rom_stub_index_drops`.
    /// `None` = unbounded; ignored when `index_reg` is `None`.
    pub index_limit: Option<u32>,
    pub op: BusRegisterOp,
}

/// The register-write shapes [`BusRegisterWrite`] supports, chosen per-call
/// to match that ROM function's actual argument meaning (see `crate::rom`'s
/// citations for which shape each interrupt-controller and GPIO-matrix ROM
/// call uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusRegisterOp {
    /// Overwrite the whole word with `a[value_reg]` — used when the ROM
    /// call's argument *is* the register's new value outright (e.g.
    /// `intr_matrix_set`'s `intr_num`, `esprv_intc_int_set_priority`'s
    /// `priority`).
    Store { value_reg: u8 },
    /// Read-modify-write: OR the mask from `mask_reg` into the register if
    /// `set`, else AND its complement out — used for a call whose argument
    /// is a bitmask of several lines at once (`esprv_intc_int_enable`/
    /// `esprv_intc_int_disable`'s `mask`/`unmask`).
    UpdateMask { mask_reg: u8, set: bool },
    /// Read-modify-write a single bit at position `a[bit_reg]`: set it if
    /// `a[cond_reg]` is nonzero, clear it otherwise — used for a call that
    /// takes a bit index plus a boolean-ish flag for that one bit
    /// (`esprv_intc_int_set_type`'s `(intr_num, type)`, where `type`'s only
    /// architecturally meaningful states are `INTR_TYPE_LEVEL = 0` and
    /// `INTR_TYPE_EDGE = 1`).
    SetOrClearBit { bit_reg: u8, cond_reg: u8 },
    /// Overwrite the whole word with `a[value_reg]` OR-ed with each
    /// [`CondBits`] entry's `bits` whose condition holds -- used when the
    /// ROM function assembles a register's new value from one argument plus
    /// per-flag bits (`gpio_matrix_out`'s `signal_idx` plus `out_inv` ->
    /// bit 8 and `oen_inv` -> bit 10; `gpio_matrix_in`'s `gpio` plus `inv`
    /// -> bit 5). An outright store, not a read-modify-write, like
    /// [`BusRegisterOp::Store`].
    StoreComposed {
        value_reg: u8,
        flags: &'static [CondBits],
    },
    /// Overwrite the whole word with `1 << a[bit_reg]` -- the shape of a
    /// write-1-to-set/-clear register store, where the other bits written as
    /// 0 mean "leave alone" (`gpio_matrix_out`'s `GPIO_ENABLE_W1TS_REG`
    /// store). Not a read-modify-write. The bit index is masked to 5 bits,
    /// same as [`BusRegisterOp::SetOrClearBit`].
    StoreBit { bit_reg: u8 },
}

/// One conditional OR-in for [`BusRegisterOp::StoreComposed`]: if
/// `a[reg]` satisfies `cond`, `bits` is OR-ed into the stored word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondBits {
    pub reg: u8,
    pub cond: RegCond,
    pub bits: u32,
}

/// The test a [`CondBits`] applies to its register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegCond {
    /// `a[reg] != 0` -- a C `bool` argument tested the way RV32 code tests
    /// it (`beqz`), so any nonzero value counts as `true`.
    NonZero,
    /// `a[reg] != value` -- a comparison against a fixed constant
    /// (`gpio_matrix_in`'s `gpio != 0x3a`).
    NotEqual(u32),
}

impl RegCond {
    /// Whether a register holding `value` satisfies this condition.
    pub fn holds(self, value: u32) -> bool {
        match self {
            RegCond::NonZero => value != 0,
            RegCond::NotEqual(constant) => value != constant,
        }
    }
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

/// A unary 32-bit libgcc helper: `int f(unsigned int a)`, argument in `a0`,
/// result in `a0` (RV32 psABI).
///
/// Semantics come from the GCC internals manual, "Integer library routines".
/// Only routines a boot run has actually called are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Int32UnaryOp {
    /// `int __clzsi2 (unsigned int a)`: the number of leading 0-bits in `a`,
    /// starting at the most significant bit. The manual says the result is
    /// **undefined if `a` is 0**; this returns 32 (`u32::leading_zeros`), the
    /// natural "all bits are leading zeros" answer, which is also what the
    /// `32 - clz(x)` "bit length" idiom (TLSF's `fls`) wants for zero.
    Clz,
    /// `int __ffssi2 (int a)`: one plus the index of the least significant
    /// 1-bit of `a`, or 0 if `a` is 0 (fully defined, unlike `Clz`).
    Ffs,
    /// `int32_t __bswapsi2 (int32_t a)`: `a` with its four bytes reversed
    /// (byte 0 <-> byte 3, byte 1 <-> byte 2). Fully defined for every input.
    /// It is the libcall `__builtin_bswap32` lowers to on RV32IMC without
    /// Zbb (ESP-IDF's `HAL_SWAP32`,
    /// `components/hal/platform_port/include/hal/misc.h:15`).
    Bswap,
}

impl Int32UnaryOp {
    /// Applies this operation to the `a0` argument.
    pub fn apply(self, a: u32) -> u32 {
        match self {
            Int32UnaryOp::Clz => a.leading_zeros(),
            Int32UnaryOp::Ffs => match a {
                0 => 0,
                _ => a.trailing_zeros() + 1,
            },
            Int32UnaryOp::Bswap => a.swap_bytes(),
        }
    }
}

/// One of libgcc's soft-float `double` helpers (`esp32c3.rom.libgcc.ld`):
/// the ESP32-C3 has no FPU, so `double` arithmetic compiles into calls to
/// these. RV32 ilp32 psABI: a `double` argument or result occupies an
/// aligned register pair, low word first (`(a0, a1)`, then `(a2, a3)`); an
/// `unsigned int` uses `a0` alone.
///
/// Semantics: GCC's libgcc soft-fp (`libgcc/soft-fp/{floatunsidf,adddf3,
/// muldf3,divdf3,fixunsdfsi,fixdfsi,gedf2,ledf2}.c` over `op-common.h`), configured for RISC-V without
/// an FPU by `libgcc/config/riscv/sfp-machine.h`: round to nearest
/// (`FP_INIT_ROUNDMODE _frm = FP_RND_NEAREST` without `__riscv_flen`), full
/// subnormal support, and every NaN result the canonical quiet NaN
/// (`_FP_NANSIGN_D 0`, `_FP_NANFRAC_D _FP_QNANBIT_D, 0`,
/// `_FP_KEEPNANFRACP 0`), i.e. `0x7FF8_0000_0000_0000`. IEEE 754
/// round-to-nearest multiply and divide are exactly specified, so Rust's
/// `f64` operators give the same bits for every non-NaN result. Exception
/// flags are not observable without an FPU and are not modeled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftDoubleOp {
    /// `double __floatunsidf(unsigned int i)`: exact (every `u32` is
    /// representable).
    FloatUnsSi,
    /// `double __adddf3(double a, double b)`.
    Add,
    /// `double __muldf3(double a, double b)`.
    Mul,
    /// `double __divdf3(double a, double b)`.
    Div,
    /// `unsigned int __fixunsdfsi(double a)`: truncates toward zero.
    /// `_FP_TO_INT(..., rsigned = 0)` gives 0 for `|a| < 1` and for any
    /// negative `a`, and all-ones for a positive `a >= 2^32`, `+inf` or a
    /// NaN whose sign bit is clear (a NaN with the sign set reads as
    /// negative: 0). Only that NaN case differs from Rust's saturating
    /// `as u32`, which maps every NaN to 0.
    FixUnsSi,
    /// `int __gedf2(double a, double b)` (also the body of `__gtdf2`):
    /// soft-fp `FP_CMP_D(r, A, B, -2, 2)`, i.e. -1 if `a < b`, 0 if equal
    /// (`-0.0 == +0.0`), 1 if `a > b`, and -2 if either is a NaN, so a
    /// caller's `>= 0` / `> 0` test is false for unordered operands. The
    /// `int` result is returned in `a0` (sign-extended to the register).
    Ge,
    /// `int __ledf2(double a, double b)` (also the body of `__ltdf2`):
    /// soft-fp `FP_CMP_D(r, A, B, 2, 2)`; as [`SoftDoubleOp::Ge`] except
    /// that a NaN operand gives 2, so `<= 0` / `< 0` is false for unordered
    /// operands.
    Le,
    /// `int __fixdfsi(double a)`: truncates toward zero. `_FP_TO_INT(...,
    /// rsigned = 1)` saturates an out-of-range `a` (including +/-inf) to
    /// `INT_MAX` or `INT_MIN` by its sign, and treats a NaN the same way by
    /// its sign bit (sign clear: `INT_MAX`). Rust's saturating `as i32`
    /// matches except for NaN, which it maps to 0. The `int` result is
    /// returned in `a0`.
    FixSi,
}

/// A libgcc soft-float `float` helper (`esp32c3.rom.libgcc.ld`). RV32
/// ilp32 psABI: a `float` argument is one register (`a0`, then `a1`), and a
/// `float` result is `a0`. Semantics as [`SoftDoubleOp`]'s (same soft-fp
/// configuration: round to nearest, subnormals kept, every NaN result the
/// canonical quiet NaN `0x7FC0_0000`, `_FP_NANFRAC_S _FP_QNANBIT_S`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftFloatOp {
    /// `float __mulsf3(float a, float b)`: IEEE round-to-nearest, which
    /// Rust's `f32` multiply gives bit for bit for every non-NaN result.
    Mul,
    /// `float __addsf3(float a, float b)`.
    Add,
    /// `int __unordsf2(float a, float b)`: soft-fp
    /// `FP_CMP_UNORD_S(r, A, B, 1)`: 1 if either operand is a NaN, else 0.
    Unord,
    /// `int __ltsf2(float a, float b)` (libgcc builds it from the same
    /// source as `__lesf2`): soft-fp `FP_CMP_S(r, A, B, 2, 2)`: -1, 0 or 1
    /// by order (`-0.0 == +0.0`), 2 if either is a NaN, so `< 0` is false
    /// for unordered operands. The `int` result is returned in `a0`.
    Lt,
    /// `float __floatsisf(int a)`: `a0` read as a signed 32-bit integer,
    /// rounded to the nearest `float` (ties to even; `|a| > 2^24` can
    /// round), which Rust's `i32 as f32` gives bit for bit. `a1` is
    /// ignored.
    FloatSi,
    /// `float __subsf3(float a, float b)`: `a - b`, IEEE round-to-nearest.
    Sub,
    /// `int __gtsf2(float a, float b)` (libgcc builds it from the same
    /// source as `__gesf2`): soft-fp `FP_CMP_S(r, A, B, -2, 2)`: -1, 0 or 1
    /// by order, -2 if either is a NaN, so `> 0` is false for unordered
    /// operands. The `int` result is returned in `a0`.
    Gt,
}

/// The canonical quiet `float` NaN RISC-V soft-fp returns (see
/// [`SoftFloatOp`]).
pub const SOFT_FP_CANONICAL_NAN_F32: u32 = 0x7FC0_0000;

impl SoftFloatOp {
    /// Applies this operation to `a` (`a0`) and `b` (`a1`), returning the
    /// result bits for `a0`.
    pub fn apply(self, a: u32, b: u32) -> u32 {
        let canonical = |x: f32| {
            if x.is_nan() {
                SOFT_FP_CANONICAL_NAN_F32
            } else {
                x.to_bits()
            }
        };
        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
        match self {
            SoftFloatOp::Mul => canonical(x * y),
            SoftFloatOp::Add => canonical(x + y),
            SoftFloatOp::Unord => u32::from(x.is_nan() || y.is_nan()),
            SoftFloatOp::Lt => soft_fp_cmp(x.partial_cmp(&y), 2) as u32,
            SoftFloatOp::FloatSi => (a as i32 as f32).to_bits(),
            SoftFloatOp::Sub => canonical(x - y),
            SoftFloatOp::Gt => soft_fp_cmp(x.partial_cmp(&y), -2) as u32,
        }
    }
}

/// soft-fp's `FP_CMP` result: -1, 0 or 1 by order, `unordered` if either
/// operand is a NaN (`partial_cmp` gives `None`).
fn soft_fp_cmp(order: Option<std::cmp::Ordering>, unordered: i32) -> i32 {
    match order {
        Some(std::cmp::Ordering::Less) => -1,
        Some(std::cmp::Ordering::Equal) => 0,
        Some(std::cmp::Ordering::Greater) => 1,
        None => unordered,
    }
}

/// The canonical quiet NaN RISC-V soft-fp returns (see [`SoftDoubleOp`]).
pub const SOFT_FP_CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;

impl SoftDoubleOp {
    /// `true` if the result is a `double` (`(a0, a1)`), `false` if it is
    /// an `int` or `unsigned int` (`a0` only; `a1` is left untouched).
    pub fn returns_double(self) -> bool {
        !matches!(
            self,
            SoftDoubleOp::FixUnsSi | SoftDoubleOp::FixSi | SoftDoubleOp::Ge | SoftDoubleOp::Le
        )
    }

    /// Applies this operation. `a` is the `(a0, a1)` pair (for
    /// [`SoftDoubleOp::FloatUnsSi`], only its low word, `a0`, is read); `b`
    /// is `(a2, a3)`, read only by the binary ops. Returns the result bits
    /// (an `unsigned int` result is zero-extended).
    pub fn apply(self, a: u64, b: u64) -> u64 {
        let canonical = |x: f64| {
            if x.is_nan() {
                SOFT_FP_CANONICAL_NAN
            } else {
                x.to_bits()
            }
        };
        let (x, y) = (f64::from_bits(a), f64::from_bits(b));
        match self {
            SoftDoubleOp::FloatUnsSi => f64::from(a as u32).to_bits(),
            SoftDoubleOp::Add => canonical(x + y),
            SoftDoubleOp::Mul => canonical(x * y),
            SoftDoubleOp::Div => canonical(x / y),
            SoftDoubleOp::FixUnsSi => {
                if x.is_nan() {
                    // Raw exponent 0x7FF: "overflow", signed by the sign bit.
                    if x.is_sign_negative() {
                        0
                    } else {
                        u64::from(u32::MAX)
                    }
                } else {
                    // Saturating truncation: negative -> 0, >= 2^32 -> MAX.
                    u64::from(x as u32)
                }
            }
            SoftDoubleOp::FixSi => {
                let r = if x.is_nan() {
                    if x.is_sign_negative() {
                        i32::MIN
                    } else {
                        i32::MAX
                    }
                } else {
                    x as i32
                };
                u64::from(r as u32)
            }
            SoftDoubleOp::Ge => u64::from(soft_fp_cmp(x.partial_cmp(&y), -2) as u32),
            SoftDoubleOp::Le => u64::from(soft_fp_cmp(x.partial_cmp(&y), 2) as u32),
        }
    }
}

/// The digit alphabet newlib's `__utoa` uses
/// (`newlib/libc/stdlib/utoa.c`): lowercase, supporting bases up to 36.
const ITOA_DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Upper bound on the bytes [`compute_itoa`] can write: 1 sign byte (base
/// 10, negative values only) + 32 base-2 digits (the worst case: any 32-bit
/// value in base 2) + 1 NUL terminator.
pub const ITOA_MAX_LEN: usize = 34;

/// [`compute_itoa`]'s result: the exact bytes ROM's `itoa` would write to
/// `str` (`bytes[..len]`, NUL-terminated) and whether `base` was valid.
/// [`RomStubEffect::Itoa`]'s execution uses `valid_base` to decide whether
/// `a0` becomes the `str` pointer or `0`/`NULL`, matching `itoa.c`'s "return
/// NULL on an invalid base" behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItoaResult {
    pub bytes: [u8; ITOA_MAX_LEN],
    pub len: usize,
    pub valid_base: bool,
}

/// Pure re-implementation of ROM libc's `itoa(value, str, base)`, mirroring
/// newlib's own source byte-for-byte:
/// `newlib/libc/stdlib/itoa.c` (`__itoa`) composed with
/// `newlib/libc/stdlib/utoa.c` (`__utoa`) — fetched from
/// `https://sourceware.org/git/?p=newlib-cygwin.git;a=blob_plain;f=newlib/libc/stdlib/<itoa|utoa>.c;hb=HEAD`
/// for this task; see `crate::rom`'s module doc for the ROM address this
/// pairs with.
///
/// Semantics, straight from that source:
/// - **Invalid base** (`base < 2 || base > 36`): `str[0] = '\0'`, return
///   `NULL` (`0`). This is documented in `itoa.c` itself, not guessed.
/// - **Base 10 with a negative `value`**: write a leading `-`, then the
///   digits of `value`'s magnitude. `itoa.c` computes that magnitude as
///   `(unsigned)-value`; negating `i32::MIN` overflows and (two's
///   complement) wraps back to `i32::MIN`, whose bit pattern cast to
///   unsigned is `0x8000_0000` — numerically the correct magnitude despite
///   the C-level UB. `value.wrapping_neg() as u32` reproduces that exact
///   bit pattern.
/// - **Every other base**, regardless of sign: `value` is reinterpreted as
///   unsigned (`value as u32` — the same bit pattern C's implicit
///   `int`→`unsigned` conversion produces), so a negative `value` in, say,
///   base 16 prints the unsigned hex of its two's-complement bit pattern,
///   not a sign.
/// - Digits come from [`ITOA_DIGITS`], written least-significant first via
///   repeated mod/div then reversed in place — exactly `utoa.c`'s loop
///   shape (the sign byte, if any, is written by `itoa` before calling into
///   this digit loop and is never part of the reversed range, matching how
///   `__itoa` calls `__utoa(uvalue, &str[i], base)` at an offset past its
///   own sign byte).
/// - Always NUL-terminated, on both the valid- and invalid-base paths.
pub fn compute_itoa(value: i32, base: i32) -> ItoaResult {
    if !(2..=36).contains(&base) {
        let mut bytes = [0u8; ITOA_MAX_LEN];
        bytes[0] = 0;
        return ItoaResult {
            bytes,
            len: 1,
            valid_base: false,
        };
    }
    let base = base as u32;

    let mut bytes = [0u8; ITOA_MAX_LEN];
    let negative = base == 10 && value < 0;
    let digits_start = if negative {
        bytes[0] = b'-';
        1
    } else {
        0
    };
    let mut uvalue: u32 = if negative {
        // itoa.c: `uvalue = (unsigned)-value;` -- see this function's doc
        // for why wrapping_neg reproduces that exact (UB-but-consistent)
        // bit pattern for `i32::MIN` too.
        value.wrapping_neg() as u32
    } else {
        value as u32
    };

    let mut pos = digits_start;
    loop {
        let remainder = (uvalue % base) as usize;
        bytes[pos] = ITOA_DIGITS[remainder];
        pos += 1;
        uvalue /= base;
        if uvalue == 0 {
            break;
        }
    }
    bytes[pos] = 0; // NUL terminator, per utoa.c's `str[i] = '\0'`.
    bytes[digits_start..pos].reverse();

    ItoaResult {
        bytes,
        len: pos + 1,
        valid_base: true,
    }
}

// ---------------------------------------------------------------------
// `ets_printf` HLE formatter (see `RomStubEffect::Printf`).
// ---------------------------------------------------------------------

/// Upper bound on how many bytes one [`compute_printf`] call will scan out
/// of the format string (and any `%s` argument string) before giving up,
/// and on how many bytes of *output* it will produce. Same rationale as
/// [`MAX_STUB_MEMORY_BYTES`]: this runs inside a WASM module driving a
/// browser tab, so a garbage/unterminated pointer must not be able to hang
/// it. 1 KiB comfortably covers any real ESP-IDF early-boot log line (the
/// longest observed, `cpu_start`'s "Invalid app image header" line plus
/// its tag/timestamp, is well under 64 bytes) while still catching a
/// runaway format string loudly (truncated output) rather than silently.
pub const PRINTF_MAX_OUTPUT_BYTES: usize = 1024;
/// Companion cap on the format string's own scan length, and on how many
/// bytes of a `%s` argument are read -- kept equal to
/// [`PRINTF_MAX_OUTPUT_BYTES`] for the same "no runaway pointer hangs the
/// host" reasoning, not because the two must match in general.
pub const PRINTF_MAX_SCAN_BYTES: usize = PRINTF_MAX_OUTPUT_BYTES;

/// Tracks which 32-bit vararg "slot" [`compute_printf`] should read next,
/// and applies the RV32 ILP32 calling convention's alignment rule for a
/// 64-bit vararg.
///
/// **Source**: the RISC-V calling-convention spec
/// (`riscv-non-isa/riscv-elf-psabi-doc`, `riscv-cc.adoc`, "Integer Calling
/// Convention"): "Variadic arguments with 2×XLEN-bit alignment and size at
/// most 2×XLEN bits are passed in an *aligned* register pair (i.e., the
/// first register in the pair is even-numbered), or on the stack by value
/// if none is available. After a variadic argument has been passed on the
/// stack, all future arguments will also be passed on the stack (i.e. the
/// last argument register may be left unused due to the aligned register
/// pair rule)."
///
/// Slots are numbered from `1` (`ets_printf`'s fixed `fmt` parameter
/// itself occupies slot `0`/`a0`, already consumed before any
/// [`VarargCursor`] exists): slots `1..=7` are `a1..=a7` (`x11..=x17`),
/// and slot `8` onward is the caller's stack at `sp`, `sp+4`, `sp+8`, ...
/// A single monotonically increasing index can address both zones with
/// one alignment rule because the psABI's "even-numbered register" and
/// "8-byte-aligned stack slot" requirements are the same requirement
/// viewed in 4-byte units: slot `0` (`a0`) sits at relative byte offset
/// `0`, so every slot's relative byte offset is `slot * 4`, and "starts at
/// an even-numbered register" / "is 8-byte-aligned on the stack" both
/// reduce to "`slot` is even" under that numbering -- including right at
/// the register/stack boundary (slot `8`, the stack's first word, is
/// even, matching the ABI's own requirement that `sp` itself is 16-byte
/// -- hence 8-byte -- aligned).
#[derive(Debug, Clone, Copy, Default)]
pub struct VarargCursor {
    next_slot: u32,
}

impl VarargCursor {
    pub fn new() -> Self {
        Self { next_slot: 1 }
    }

    /// Consumes one 32-bit slot and returns its index.
    fn take_u32_slot(&mut self) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        slot
    }

    /// Consumes an aligned pair of slots -- skipping one slot of padding
    /// first if the next slot is odd, per this type's doc -- and returns
    /// `(low_slot, high_slot)`.
    fn take_u64_slots(&mut self) -> (u32, u32) {
        if !self.next_slot.is_multiple_of(2) {
            self.next_slot += 1; // padding: this slot is left unused.
        }
        let low = self.next_slot;
        let high = self.next_slot + 1;
        self.next_slot += 2;
        (low, high)
    }
}

/// What [`compute_printf`] needs from its caller: a way to read a byte of
/// memory (backing both the format-string scan and any `%s` argument
/// string) and a way to resolve a [`VarargCursor`] slot index to its
/// 32-bit value.
///
/// Deliberately abstract over *both* concerns in one trait rather than two
/// separate closures: the real HLE stub (`crate::cpu::Cpu::apply_rom_stub`)
/// backs both with the same live `Bus`, and a single mutable borrow of
/// that bus is all Rust's borrow checker will allow across one
/// [`compute_printf`] call -- two independent closures each independently
/// capturing the bus would be two live mutable borrows of the same value.
/// Unit tests below back this with a plain in-memory fake instead.
pub trait PrintfHost {
    /// Reads the byte at `addr`.
    fn read_byte(&mut self, addr: u32) -> u8;
    /// Resolves vararg slot `slot` (`1..=7` are `a1..=a7`; `8+` is the
    /// caller's stack, per [`VarargCursor`]'s doc) to its 32-bit value.
    fn slot(&mut self, slot: u32) -> u32;
}

/// [`compute_printf`]'s result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintfOutput {
    /// The formatted output bytes, already capped at
    /// [`PRINTF_MAX_OUTPUT_BYTES`].
    pub bytes: Vec<u8>,
    /// `a0`'s return value: the number of bytes actually emitted (i.e.
    /// `bytes.len()`) -- matching `ets_sys.h`'s documented
    /// `ets_printf`/`ets_vprintf` return, "the length printed to the
    /// output device", under this cap.
    pub chars_written: u32,
}

/// Appends `byte` to `out` unless [`PRINTF_MAX_OUTPUT_BYTES`] has already
/// been reached. Returns whether the byte was written -- callers use this
/// to stop early instead of doing pointless further work once the cap is
/// hit.
fn push_capped(out: &mut Vec<u8>, byte: u8) -> bool {
    if out.len() >= PRINTF_MAX_OUTPUT_BYTES {
        return false;
    }
    out.push(byte);
    true
}

/// Appends `text`, padded to at least `width` bytes with `pad_byte`
/// (space, or `'0'` for a zero-padded numeric conversion -- see
/// [`compute_printf`]'s flag handling), on the left unless `left_align`.
/// Stops early (silently) once [`PRINTF_MAX_OUTPUT_BYTES`] is hit, same as
/// [`push_capped`].
fn push_padded(out: &mut Vec<u8>, text: &[u8], width: usize, left_align: bool, pad_byte: u8) {
    let pad_len = width.saturating_sub(text.len());
    if !left_align {
        for _ in 0..pad_len {
            if !push_capped(out, pad_byte) {
                return;
            }
        }
    }
    for &b in text {
        if !push_capped(out, b) {
            return;
        }
    }
    if left_align {
        for _ in 0..pad_len {
            if !push_capped(out, pad_byte) {
                return;
            }
        }
    }
}

/// [`push_padded`] for a numeric conversion: when zero-padding, a leading
/// `-` goes before the zeros (`%05d` of -42 is `-0042`, as in C), not after.
fn push_number(out: &mut Vec<u8>, text: &[u8], width: usize, left_align: bool, pad_byte: u8) {
    match text.split_first() {
        Some((&b'-', digits)) if pad_byte == b'0' => {
            if push_capped(out, b'-') {
                push_padded(out, digits, width.saturating_sub(1), left_align, pad_byte);
            }
        }
        _ => push_padded(out, text, width, left_align, pad_byte),
    }
}

/// Formats `fmt_addr`'s C format string (read through `host`) against the
/// RV32 ILP32 varargs `host` resolves (`a1..a7`, then the stack -- see
/// [`VarargCursor`]), mirroring the ESP32-C3 ROM's own `ets_printf`
/// (`components/esp_rom/esp32c3/include/esp32c3/rom/ets_sys.h`:
/// `int ets_printf(const char *fmt, ...)`, "Printf the strings to uart or
/// other devices, similar with printf, simple than printf.").
///
/// **Supported conversions** (Milestone 3 Task 7's orchestrator ruling):
/// `%d %i %u %x %X %c %s %p %%`; the `l` modifier (a no-op on RV32
/// ILP32, where `long` is already 32 bits) and `ll` (consumes an aligned
/// 64-bit pair via [`VarargCursor::take_u64_slots`]); flags `-`
/// (left-align) and `0` (zero-pad); a decimal field width; and a decimal
/// precision on `%s` (truncates the string to at most that many bytes --
/// cheap to support since the string is already being scanned byte by
/// byte).
///
/// **Unsupported specifiers** (notably `%f`/`%e`/`%g` -- ROM's own
/// `ets_sys.h` documents `ets_printf` itself as unable to print
/// floating-point, "Can not print float point data format, or longlong
/// data format", though this stub does support `ll` per the orchestrator
/// ruling above) are echoed as just `%` followed by the conversion
/// character (e.g. `%f` prints the two characters `%f`; any flags, width,
/// precision or length modifier between them are parsed and dropped, so
/// `%-08.3lf` also prints `%f` -- not a fully verbatim echo) and consume
/// **no** vararg -- printing a wrong/misaligned value from the wrong slot
/// would be worse than printing the literal specifier. A lone `%` right
/// before the terminator (possibly with flags/width) prints nothing and
/// ends the string.
///
/// A NULL (`0`) `%s` pointer prints `(null)`, matching glibc/newlib's own
/// common convention for this ROM's non-standard printf (not documented in
/// `ets_sys.h`, which is silent on this case, but universal defensive
/// practice for a `%s` implementation and clearly preferable to dereferencing
/// a null pointer).
///
/// Every scan loop (literal text, flags, width, precision, `%s` argument)
/// is capped at [`PRINTF_MAX_SCAN_BYTES`] and every byte written at
/// [`PRINTF_MAX_OUTPUT_BYTES`] (the fixed-length `.`/`l`/`ll`/conversion
/// reads after a capped loop add at most four more bytes), so a
/// garbage/unterminated pointer can't hang the caller.
pub fn compute_printf(fmt_addr: u32, host: &mut impl PrintfHost) -> PrintfOutput {
    let mut out = Vec::new();
    let mut cursor = VarargCursor::new();
    let mut i: u32 = 0;

    loop {
        if i as usize >= PRINTF_MAX_SCAN_BYTES || out.len() >= PRINTF_MAX_OUTPUT_BYTES {
            break;
        }
        let c = host.read_byte(fmt_addr.wrapping_add(i));
        i += 1;
        if c == 0 {
            break;
        }
        if c != b'%' {
            if !push_capped(&mut out, c) {
                break;
            }
            continue;
        }

        // Flags: '-' (left-align) and '0' (zero-pad), in any order/repeat,
        // per this function's doc.
        let mut left_align = false;
        let mut zero_pad = false;
        while (i as usize) < PRINTF_MAX_SCAN_BYTES {
            match host.read_byte(fmt_addr.wrapping_add(i)) {
                b'-' => {
                    left_align = true;
                    i += 1;
                }
                b'0' => {
                    zero_pad = true;
                    i += 1;
                }
                _ => break,
            }
        }

        // Decimal field width.
        let mut width: usize = 0;
        while (i as usize) < PRINTF_MAX_SCAN_BYTES {
            let d = host.read_byte(fmt_addr.wrapping_add(i));
            if d.is_ascii_digit() {
                width = width.saturating_mul(10).saturating_add((d - b'0') as usize);
                i += 1;
            } else {
                break;
            }
        }

        // Decimal precision, `%s`-only per this function's doc.
        let mut precision: Option<usize> = None;
        if host.read_byte(fmt_addr.wrapping_add(i)) == b'.' {
            i += 1;
            let mut p: usize = 0;
            while (i as usize) < PRINTF_MAX_SCAN_BYTES {
                let d = host.read_byte(fmt_addr.wrapping_add(i));
                if d.is_ascii_digit() {
                    p = p.saturating_mul(10).saturating_add((d - b'0') as usize);
                    i += 1;
                } else {
                    break;
                }
            }
            precision = Some(p);
        }

        // Length modifier: 'l' (no-op on ILP32) or 'll' (a 64-bit vararg).
        let mut is_64 = false;
        if host.read_byte(fmt_addr.wrapping_add(i)) == b'l' {
            i += 1;
            if host.read_byte(fmt_addr.wrapping_add(i)) == b'l' {
                i += 1;
                is_64 = true;
            }
        }

        let conv = host.read_byte(fmt_addr.wrapping_add(i));
        i += 1;
        if conv == 0 {
            // A lone `%` (plus any flags/width) right before the terminator:
            // stop here rather than emit the NUL and scan past the string.
            break;
        }

        let zero_pad_byte = if zero_pad && !left_align { b'0' } else { b' ' };

        match conv {
            b'%' => {
                if !push_capped(&mut out, b'%') {
                    break;
                }
            }
            b'c' => {
                let slot = cursor.take_u32_slot();
                let v = host.slot(slot);
                if !push_capped(&mut out, v as u8) {
                    break;
                }
            }
            b'd' | b'i' => {
                let value: i64 = if is_64 {
                    let (lo, hi) = cursor.take_u64_slots();
                    let raw = u64::from(host.slot(lo)) | (u64::from(host.slot(hi)) << 32);
                    raw as i64
                } else {
                    let slot = cursor.take_u32_slot();
                    host.slot(slot) as i32 as i64
                };
                let text = value.to_string();
                push_number(&mut out, text.as_bytes(), width, left_align, zero_pad_byte);
            }
            b'u' => {
                let value: u64 = if is_64 {
                    let (lo, hi) = cursor.take_u64_slots();
                    u64::from(host.slot(lo)) | (u64::from(host.slot(hi)) << 32)
                } else {
                    let slot = cursor.take_u32_slot();
                    u64::from(host.slot(slot))
                };
                let text = value.to_string();
                push_number(&mut out, text.as_bytes(), width, left_align, zero_pad_byte);
            }
            b'x' | b'X' => {
                let value: u64 = if is_64 {
                    let (lo, hi) = cursor.take_u64_slots();
                    u64::from(host.slot(lo)) | (u64::from(host.slot(hi)) << 32)
                } else {
                    let slot = cursor.take_u32_slot();
                    u64::from(host.slot(slot))
                };
                let text = if conv == b'X' {
                    format!("{value:X}")
                } else {
                    format!("{value:x}")
                };
                push_number(&mut out, text.as_bytes(), width, left_align, zero_pad_byte);
            }
            b'p' => {
                let slot = cursor.take_u32_slot();
                let ptr = host.slot(slot);
                let text = format!("0x{ptr:x}");
                push_padded(&mut out, text.as_bytes(), width, left_align, b' ');
            }
            b's' => {
                let slot = cursor.take_u32_slot();
                let ptr = host.slot(slot);
                if ptr == 0 {
                    push_padded(&mut out, b"(null)", width, left_align, b' ');
                } else {
                    let mut s = Vec::new();
                    let mut j: u32 = 0;
                    loop {
                        if (j as usize) >= PRINTF_MAX_SCAN_BYTES {
                            break;
                        }
                        if let Some(p) = precision {
                            if s.len() >= p {
                                break;
                            }
                        }
                        let b = host.read_byte(ptr.wrapping_add(j));
                        if b == 0 {
                            break;
                        }
                        s.push(b);
                        j += 1;
                    }
                    push_padded(&mut out, &s, width, left_align, b' ');
                }
            }
            _ => {
                // Unsupported specifier (e.g. `%f`): emit the literal `%`
                // and conversion character, consuming no vararg -- see
                // this function's doc.
                if !push_capped(&mut out, b'%') {
                    break;
                }
                if !push_capped(&mut out, conv) {
                    break;
                }
            }
        }
    }

    let chars_written = out.len() as u32;
    PrintfOutput {
        bytes: out,
        chars_written,
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

    /// A real high-level-emulated `memcpy` — see [`RomStubEffect::Memcpy`].
    pub const fn memcpy(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Memcpy,
        }
    }

    /// A `void` ROM function that stores words taken from its argument
    /// registers to fixed addresses -- see [`RomStubEffect::StoreWords`].
    pub const fn store_words(name: &'static str, stores: &'static [WordStore]) -> Self {
        Self {
            name,
            effect: RomStubEffect::StoreWords(stores),
        }
    }

    /// A real high-level-emulated `itoa` — see [`RomStubEffect::Itoa`].
    pub const fn itoa(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Itoa,
        }
    }

    /// A real high-level-emulated `strlen` — see [`RomStubEffect::Strlen`].
    pub const fn strlen(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strlen,
        }
    }

    /// A real high-level-emulated `memcmp` — see [`RomStubEffect::Memcmp`].
    pub const fn memcmp(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Memcmp,
        }
    }

    /// A real high-level-emulated `strncmp` — see [`RomStubEffect::Strncmp`].
    pub const fn strncmp(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strncmp,
        }
    }

    /// A real high-level-emulated `strncpy` -- see [`RomStubEffect::Strncpy`].
    pub const fn strncpy(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strncpy,
        }
    }

    /// A real high-level-emulated `strcmp` -- see [`RomStubEffect::Strcmp`].
    pub const fn strcmp(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strcmp,
        }
    }

    /// A real high-level-emulated `strcasecmp` -- see
    /// [`RomStubEffect::Strcasecmp`].
    pub const fn strcasecmp(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strcasecmp,
        }
    }

    /// A real high-level-emulated `strlcpy` -- see [`RomStubEffect::Strlcpy`].
    pub const fn strlcpy(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strlcpy,
        }
    }

    /// A real high-level-emulated `strtol` -- see [`RomStubEffect::Strtol`].
    pub const fn strtol(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strtol,
        }
    }

    /// A real high-level-emulated `atoi` -- see [`RomStubEffect::Atoi`].
    pub const fn atoi(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Atoi,
        }
    }

    /// A real high-level-emulated `strrchr` -- see [`RomStubEffect::Strrchr`].
    pub const fn strrchr(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strrchr,
        }
    }

    /// A real high-level-emulated `strlcat` -- see [`RomStubEffect::Strlcat`].
    pub const fn strlcat(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strlcat,
        }
    }

    /// A real high-level-emulated `strspn` -- see [`RomStubEffect::Strspn`].
    pub const fn strspn(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strspn,
        }
    }

    /// A real high-level-emulated `strcspn` -- see [`RomStubEffect::Strcspn`].
    pub const fn strcspn(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strcspn,
        }
    }

    /// A real high-level-emulated `strchr` -- see [`RomStubEffect::Strchr`].
    pub const fn strchr(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strchr,
        }
    }

    /// A real high-level-emulated `strcpy` -- see [`RomStubEffect::Strcpy`].
    pub const fn strcpy(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strcpy,
        }
    }

    /// A real high-level-emulated `memchr` — see [`RomStubEffect::Memchr`].
    pub const fn memchr(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Memchr,
        }
    }

    /// A real high-level-emulated `memmove` — see [`RomStubEffect::Memmove`].
    pub const fn memmove(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Memmove,
        }
    }

    /// A real high-level-emulated `div` — see [`RomStubEffect::DivT`].
    pub const fn div_t(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::DivT,
        }
    }

    /// A real high-level-emulated MD5 call over a guest context — see
    /// [`RomStubEffect::Md5`].
    pub const fn md5(name: &'static str, op: Md5Op) -> Self {
        Self {
            name,
            effect: RomStubEffect::Md5(op),
        }
    }

    /// A real high-level-emulated `strcat` — see [`RomStubEffect::Strcat`].
    pub const fn strcat(name: &'static str) -> Self {
        Self {
            name,
            effect: RomStubEffect::Strcat,
        }
    }

    /// A real high-level-emulated `ets_printf`, writing formatted output
    /// bytes to `sink_addr` — see [`RomStubEffect::Printf`].
    pub const fn printf(name: &'static str, sink_addr: u32) -> Self {
        Self {
            name,
            effect: RomStubEffect::Printf { sink_addr },
        }
    }

    /// A real high-level-emulated libgcc unary 32-bit helper -- see
    /// [`Int32UnaryOp`].
    pub const fn int32_unary(name: &'static str, op: Int32UnaryOp) -> Self {
        RomStub {
            name,
            effect: RomStubEffect::Int32Unary(op),
        }
    }

    /// A real high-level-emulated libgcc 64-bit helper — see [`Int64Op`].
    pub const fn int64(name: &'static str, op: Int64Op) -> Self {
        Self {
            name,
            effect: RomStubEffect::Int64(op),
        }
    }

    /// A real libgcc soft-float `float` helper — see [`SoftFloatOp`].
    pub const fn soft_float(name: &'static str, op: SoftFloatOp) -> Self {
        Self {
            name,
            effect: RomStubEffect::SoftFloat(op),
        }
    }

    /// A real libgcc soft-float `double` helper — see [`SoftDoubleOp`].
    pub const fn soft_double(name: &'static str, op: SoftDoubleOp) -> Self {
        Self {
            name,
            effect: RomStubEffect::SoftDouble(op),
        }
    }

    /// A real high-level-emulated peripheral-register write — see
    /// [`RomStubEffect::BusRegisterWrite`].
    pub const fn bus_register_write(name: &'static str, write: BusRegisterWrite) -> Self {
        Self {
            name,
            effect: RomStubEffect::BusRegisterWrite(write),
        }
    }

    /// A real high-level-emulated sequence of peripheral-register writes —
    /// see [`RomStubEffect::BusRegisterWrites`].
    pub const fn bus_register_writes(
        name: &'static str,
        writes: &'static [BusRegisterWrite],
    ) -> Self {
        Self {
            name,
            effect: RomStubEffect::BusRegisterWrites(writes),
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
        assert_eq!(RomStub::memcpy("memcpy").effect, RomStubEffect::Memcpy);
        assert_eq!(RomStub::itoa("itoa").effect, RomStubEffect::Itoa);
        assert_eq!(RomStub::strcat("strcat").effect, RomStubEffect::Strcat);
        assert_eq!(RomStub::returning("x", 7).effect, RomStubEffect::Return(7));
        assert_eq!(
            RomStub::bus_register_write(
                "x",
                BusRegisterWrite {
                    base: 0x100,
                    index_reg: Some(REG_A0),
                    index_limit: Some(8),
                    op: BusRegisterOp::Store { value_reg: REG_A1 },
                }
            )
            .effect,
            RomStubEffect::BusRegisterWrite(BusRegisterWrite {
                base: 0x100,
                index_reg: Some(REG_A0),
                index_limit: Some(8),
                op: BusRegisterOp::Store { value_reg: REG_A1 },
            })
        );
    }

    #[test]
    fn soft_float_ops_match_libgcc_soft_fp() {
        use SoftFloatOp::*;
        let f = |x: f32| x.to_bits();
        let nan = SOFT_FP_CANONICAL_NAN_F32;
        // __mulsf3 / __addsf3: IEEE round-to-nearest in single precision.
        assert_eq!(Mul.apply(f(0.1), f(3.0)), f(0.1f32 * 3.0));
        assert_eq!(Add.apply(f(0.1), f(0.2)), f(0.1f32 + 0.2));
        assert_eq!(Mul.apply(f(f32::MIN_POSITIVE), f(0.5)), f(f32::MIN_POSITIVE / 2.0));
        assert_eq!(Mul.apply(f(f32::INFINITY), f(0.0)), nan);
        assert_eq!(Add.apply(0xFFC0_0001, f(1.0)), nan, "NaN in, canonical out");
        // __unordsf2.
        assert_eq!(Unord.apply(f(1.0), f(2.0)), 0);
        assert_eq!(Unord.apply(nan, f(2.0)), 1);
        assert_eq!(Unord.apply(f(2.0), 0xFFC0_0000), 1);
        // __ltsf2: -1/0/1, 2 for unordered.
        assert_eq!(Lt.apply(f(1.0), f(2.0)) as i32, -1);
        assert_eq!(Lt.apply(f(-0.0), f(0.0)), 0);
        assert_eq!(Lt.apply(f(3.0), f(2.0)), 1);
        assert_eq!(Lt.apply(nan, f(2.0)), 2);
        // __subsf3.
        assert_eq!(Sub.apply(f(1000.0), f(0.1)), f(1000.0f32 - 0.1));
        assert_eq!(Sub.apply(f(1.0), f(1.0)), 0, "+0.0");
        assert_eq!(Sub.apply(f(f32::INFINITY), f(f32::INFINITY)), nan);
        // __gtsf2: -1/0/1, -2 for unordered.
        assert_eq!(Gt.apply(f(1300.0), f(1200.0)), 1);
        assert_eq!(Gt.apply(f(0.0), f(-0.0)), 0);
        assert_eq!(Gt.apply(f(-1.0), f(1200.0)) as i32, -1);
        assert_eq!(Gt.apply(f(1.0), nan) as i32, -2);
        // __floatsisf: signed, exact below 2^24, round-to-nearest-even above.
        assert_eq!(FloatSi.apply(1000, 0xDEAD), f(1000.0));
        assert_eq!(FloatSi.apply(-2048i32 as u32, 0), f(-2048.0));
        assert_eq!(FloatSi.apply(0, 0), 0, "+0.0");
        assert_eq!(FloatSi.apply(16_777_217, 0), f(16_777_216.0), "tie to even");
        assert_eq!(FloatSi.apply(16_777_219, 0), f(16_777_220.0), "tie to even");
        assert_eq!(FloatSi.apply(i32::MIN as u32, 0), 0xCF00_0000, "-2^31");
        assert_eq!(FloatSi.apply(i32::MAX as u32, 0), 0x4F00_0000, "rounds to 2^31");
    }

    #[test]
    fn soft_double_ops_match_libgcc_soft_fp() {
        use SoftDoubleOp::*;
        let d = |x: f64| x.to_bits();
        // __floatunsidf: exact; the observed boot call converts 10,000,000.
        assert_eq!(FloatUnsSi.apply(10_000_000, 0), d(1e7));
        assert_eq!(FloatUnsSi.apply(10_000_000 | (0xDEAD << 32), 0), d(1e7));
        assert_eq!(FloatUnsSi.apply(0, 0), 0);
        assert_eq!(FloatUnsSi.apply(u64::from(u32::MAX), 0), d(4_294_967_295.0));
        // __muldf3 / __divdf3: IEEE round-to-nearest.
        assert_eq!(Mul.apply(d(1e7), d(2.5)), d(2.5e7));
        assert_eq!(Mul.apply(d(0.1), d(3.0)), d(0.1 * 3.0));
        assert_eq!(Div.apply(d(1e7), d(3.0)), d(1e7 / 3.0));
        assert_eq!(Div.apply(d(1.0), d(0.0)), d(f64::INFINITY));
        assert_eq!(Div.apply(d(-1.0), d(0.0)), d(f64::NEG_INFINITY));
        // Subnormals are kept, not flushed.
        assert_eq!(
            Mul.apply(d(f64::MIN_POSITIVE), d(0.5)),
            d(f64::MIN_POSITIVE / 2.0)
        );
        // NaN results are the canonical quiet NaN, sign clear.
        assert_eq!(Div.apply(d(0.0), d(0.0)), SOFT_FP_CANONICAL_NAN);
        assert_eq!(Mul.apply(d(f64::INFINITY), d(-0.0)), SOFT_FP_CANONICAL_NAN);
        assert_eq!(
            Mul.apply(0xFFF8_0000_0000_0001, d(1.0)),
            SOFT_FP_CANONICAL_NAN
        );
        // __fixunsdfsi: truncation and _FP_TO_INT's unsigned edge cases.
        assert_eq!(FixUnsSi.apply(d(3.9), 0), 3);
        assert_eq!(FixUnsSi.apply(d(4_294_967_295.9), 0), u64::from(u32::MAX));
        assert_eq!(FixUnsSi.apply(d(4_294_967_296.0), 0), u64::from(u32::MAX));
        assert_eq!(FixUnsSi.apply(d(f64::INFINITY), 0), u64::from(u32::MAX));
        assert_eq!(
            FixUnsSi.apply(SOFT_FP_CANONICAL_NAN, 0),
            u64::from(u32::MAX)
        );
        assert_eq!(FixUnsSi.apply(d(-0.5), 0), 0);
        assert_eq!(FixUnsSi.apply(d(-1.0), 0), 0);
        assert_eq!(FixUnsSi.apply(d(f64::NEG_INFINITY), 0), 0);
        assert_eq!(FixUnsSi.apply(0xFFF8_0000_0000_0000, 0), 0);
        // __adddf3.
        assert_eq!(Add.apply(d(0.1), d(0.2)), d(0.1 + 0.2));
        assert_eq!(Add.apply(d(f64::INFINITY), d(f64::NEG_INFINITY)), SOFT_FP_CANONICAL_NAN);
        // __fixdfsi: truncation toward zero, signed saturation, NaN by sign.
        let int = |x: i32| u64::from(x as u32);
        assert_eq!(FixSi.apply(d(-3.9), 0), int(-3));
        assert_eq!(FixSi.apply(d(1767225600.0), 0), int(1_767_225_600));
        assert_eq!(FixSi.apply(d(2_147_483_648.0), 0), int(i32::MAX));
        assert_eq!(FixSi.apply(d(-2_147_483_649.0), 0), int(i32::MIN));
        assert_eq!(FixSi.apply(d(f64::NEG_INFINITY), 0), int(i32::MIN));
        assert_eq!(FixSi.apply(SOFT_FP_CANONICAL_NAN, 0), int(i32::MAX));
        assert_eq!(FixSi.apply(0xFFF8_0000_0000_0000, 0), int(i32::MIN));
        // __gedf2 / __ledf2: -1/0/1, and the unordered results -2 / 2.
        for (op, nan) in [(Ge, -2), (Le, 2)] {
            assert_eq!(op.apply(d(1.0), d(2.0)), int(-1));
            assert_eq!(op.apply(d(2.0), d(2.0)), int(0));
            assert_eq!(op.apply(d(-0.0), d(0.0)), int(0), "-0 == +0");
            assert_eq!(op.apply(d(3.0), d(2.0)), int(1));
            assert_eq!(op.apply(SOFT_FP_CANONICAL_NAN, d(2.0)), int(nan));
            assert_eq!(op.apply(d(2.0), SOFT_FP_CANONICAL_NAN), int(nan));
            assert!(!op.returns_double());
        }
        assert!(!FixSi.returns_double() && Add.returns_double());
        assert!(!FixUnsSi.returns_double());
        assert!(FloatUnsSi.returns_double() && Mul.returns_double() && Div.returns_double());
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

    /// Decodes a [`compute_itoa`] result's written bytes (`bytes[..len]`,
    /// NUL-terminated) back to a `&str`, for readable test assertions.
    fn itoa_str(result: ItoaResult) -> String {
        String::from_utf8(result.bytes[..result.len - 1].to_vec()).expect("ascii digits")
    }

    #[test]
    fn compute_itoa_base16_matches_the_observed_boot_probe_call() {
        // itoa(value = 0x42001011, str, base = 0x10) -- Task D4's exact
        // observed stall (see the task brief/rom.rs's module doc).
        let result = compute_itoa(0x4200_1011u32 as i32, 16);
        assert!(result.valid_base);
        assert_eq!(itoa_str(result), "42001011");
    }

    #[test]
    fn compute_itoa_base10_negative_gets_a_minus_sign() {
        let result = compute_itoa(-123, 10);
        assert!(result.valid_base);
        assert_eq!(itoa_str(result), "-123");
    }

    #[test]
    fn compute_itoa_base16_negative_is_unsigned_twos_complement() {
        // itoa.c: "Negative numbers are only supported for decimal" -- every
        // other base treats `value` as unsigned, so this is the hex of
        // -1i32's bit pattern, not "-1".
        let result = compute_itoa(-1, 16);
        assert!(result.valid_base);
        assert_eq!(itoa_str(result), "ffffffff");
    }

    #[test]
    fn compute_itoa_i32_min_base10_matches_newlibs_wrapping_negate() {
        // itoa.c casts `(unsigned)-value`; negating i32::MIN overflows and
        // (two's complement) wraps back to i32::MIN, whose bit pattern cast
        // to unsigned is 0x8000_0000 = 2147483648 -- numerically correct
        // despite the C-level UB. `value.wrapping_neg() as u32` reproduces
        // that exact bit pattern.
        let result = compute_itoa(i32::MIN, 10);
        assert!(result.valid_base);
        assert_eq!(itoa_str(result), "-2147483648");
    }

    #[test]
    fn compute_itoa_zero_is_the_single_digit_zero() {
        let result = compute_itoa(0, 10);
        assert!(result.valid_base);
        assert_eq!(itoa_str(result), "0");
    }

    #[test]
    fn compute_itoa_base2_and_base36_use_correct_lowercase_digits() {
        let base2 = compute_itoa(10, 2);
        assert!(base2.valid_base);
        assert_eq!(itoa_str(base2), "1010");

        // 36^2 = 1296 -> "100" in base 36; and a value that exercises the
        // letter digits: 35 -> "z" (the last of the 36 symbols).
        let base36 = compute_itoa(1296, 36);
        assert!(base36.valid_base);
        assert_eq!(itoa_str(base36), "100");

        let base36_letters = compute_itoa(35, 36);
        assert!(base36_letters.valid_base);
        assert_eq!(itoa_str(base36_letters), "z");
    }

    #[test]
    fn compute_itoa_always_nul_terminates() {
        let result = compute_itoa(255, 16);
        assert_eq!(result.bytes[result.len - 1], 0);
    }

    #[test]
    fn compute_itoa_rejects_bases_outside_2_to_36() {
        // itoa.c: `if ((base < 2) || (base > 36)) { str[0] = '\0'; return
        // NULL; }`. Confirmed from newlib's own source
        // (newlib/libc/stdlib/itoa.c) -- not guessed.
        for bad_base in [-1, 0, 1, 37, 100] {
            let result = compute_itoa(42, bad_base);
            assert!(!result.valid_base, "base {bad_base} should be invalid");
            assert_eq!(result.len, 1, "only the NUL byte is written");
            assert_eq!(result.bytes[0], 0);
        }
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

    #[test]
    fn printf_constructor_maps_to_the_expected_effect() {
        assert_eq!(
            RomStub::printf("ets_printf", 0x6004_3000).effect,
            RomStubEffect::Printf {
                sink_addr: 0x6004_3000
            }
        );
    }

    // ---- `compute_printf` unit tests ----
    //
    // `TestHost` is a `PrintfHost` fake: memory is a plain byte map (built
    // by `write_cstr`), and vararg slots come from a flat `Vec<u32>`
    // indexed `slot - 1` (slot 1 is index 0). It deliberately does *not*
    // distinguish "register" from "stack" slots -- `VarargCursor`'s whole
    // point is that both zones share one alignment rule, so one flat
    // backing array exercises "slot <= 7" and "slot > 7" identically. The
    // real CPU-backed implementation (`crate::cpu::Cpu::apply_rom_stub`)
    // is the one that actually splits slots 1..=7 (registers) from 8+
    // (stack) -- see `crate::rom`'s full-execution test for that half.

    struct TestHost {
        mem: HashMap<u32, u8>,
        slots: Vec<u32>,
    }

    impl TestHost {
        fn new(slots: Vec<u32>) -> Self {
            Self {
                mem: HashMap::new(),
                slots,
            }
        }

        fn write_cstr(&mut self, addr: u32, s: &[u8]) {
            for (i, b) in s.iter().enumerate() {
                self.mem.insert(addr + i as u32, *b);
            }
            self.mem.insert(addr + s.len() as u32, 0);
        }
    }

    impl PrintfHost for TestHost {
        fn read_byte(&mut self, addr: u32) -> u8 {
            self.mem.get(&addr).copied().unwrap_or(0)
        }
        fn slot(&mut self, slot: u32) -> u32 {
            self.slots.get((slot - 1) as usize).copied().unwrap_or(0)
        }
    }

    const FMT_ADDR: u32 = 0x1000;

    /// Runs `compute_printf` on `fmt` against `slots`, with no `%s`
    /// arguments (see [`run_printf_with_strings`] for those).
    fn run_printf(fmt: &[u8], slots: Vec<u32>) -> (String, u32) {
        let mut host = TestHost::new(slots);
        host.write_cstr(FMT_ADDR, fmt);
        let result = compute_printf(FMT_ADDR, &mut host);
        (
            String::from_utf8(result.bytes).expect("ascii output"),
            result.chars_written,
        )
    }

    #[test]
    fn printf_supports_each_documented_conversion() {
        assert_eq!(run_printf(b"%d", vec![42]).0, "42");
        assert_eq!(run_printf(b"%i", vec![(-7i32) as u32]).0, "-7");
        assert_eq!(run_printf(b"%u", vec![u32::MAX]).0, "4294967295");
        assert_eq!(run_printf(b"%x", vec![0xdead_beef]).0, "deadbeef");
        assert_eq!(run_printf(b"%X", vec![0xdead_beef]).0, "DEADBEEF");
        assert_eq!(run_printf(b"%c", vec![b'Q' as u32]).0, "Q");
        assert_eq!(run_printf(b"100%%", vec![]).0, "100%");
        assert_eq!(run_printf(b"%p", vec![0x1234]).0, "0x1234");
    }

    #[test]
    fn printf_return_value_is_the_number_of_bytes_written() {
        let (text, chars_written) = run_printf(b"x=%d!", vec![42]);
        assert_eq!(text, "x=42!");
        assert_eq!(chars_written, text.len() as u32);
    }

    #[test]
    fn printf_percent_s_reads_a_nul_terminated_string_through_the_host() {
        let mut host = TestHost::new(vec![0x2000]);
        host.write_cstr(FMT_ADDR, b"%s");
        host.write_cstr(0x2000, b"hi");
        let result = compute_printf(FMT_ADDR, &mut host);
        assert_eq!(String::from_utf8(result.bytes).unwrap(), "hi");
    }

    #[test]
    fn printf_percent_s_precision_truncates() {
        let mut host = TestHost::new(vec![0x2000]);
        host.write_cstr(FMT_ADDR, b"%.2s");
        host.write_cstr(0x2000, b"hello");
        let result = compute_printf(FMT_ADDR, &mut host);
        assert_eq!(String::from_utf8(result.bytes).unwrap(), "he");
    }

    #[test]
    fn printf_null_percent_s_prints_null_literal() {
        // No documented ets_sys.h behavior for this case; universal
        // defensive convention for a %s implementation, and clearly
        // better than dereferencing a null pointer -- see this function's
        // doc.
        assert_eq!(run_printf(b"%s", vec![0]).0, "(null)");
    }

    #[test]
    fn printf_field_width_and_flags() {
        assert_eq!(run_printf(b"[%5d]", vec![42]).0, "[   42]");
        assert_eq!(run_printf(b"[%-5d]", vec![42]).0, "[42   ]");
        assert_eq!(run_printf(b"[%05d]", vec![42]).0, "[00042]");
        // `0` is ignored once `-` is also given (left-align wins), same as
        // standard C printf.
        assert_eq!(run_printf(b"[%-05d]", vec![42]).0, "[42   ]");
    }

    #[test]
    fn printf_ll_reads_an_aligned_64_bit_pair_and_skips_a_padding_slot() {
        // The first vararg is a 64-bit one: per VarargCursor's doc, slot 1
        // (a1) is odd and gets skipped as padding, so the pair comes from
        // slots 2:3 (a2:a3), not 1:2. Seed slot 1 with a value that would
        // produce a wildly different (huge) result if the cursor
        // incorrectly failed to skip it and used slots 1:2 as the
        // low/high pair instead -- so this test actually discriminates a
        // broken alignment rule from a correct one.
        let (text, _) = run_printf(b"%llu", vec![0x00ba_dbad, 1, 0]);
        assert_eq!(
            text, "1",
            "expected the aligned pair (slots 2:3 = 1,0), not slots 1:2"
        );
    }

    #[test]
    fn printf_ll_high_word_is_significant() {
        // slot 1 (padding, skipped), slot 2 = low = 0x11112222, slot 3 =
        // high = 0x33334444 -> the full 64-bit value, not just the low
        // word a plain %x/%u (32-bit) read would have produced.
        let (text, _) = run_printf(b"%llx", vec![0, 0x1111_2222, 0x3333_4444]);
        assert_eq!(text, "3333444411112222");
    }

    #[test]
    fn printf_llu_after_a_32_bit_vararg_still_aligns() {
        // slot 1 (a1, odd) holds an ordinary %u; the %llu that follows
        // must then skip slot 2 (a2 -- even, but the *next* slot after
        // slot 1 is already even, so no padding is needed here) -- this
        // exercises the "next slot already even" branch, complementing
        // the padding-needed case above.
        let (text, _) = run_printf(b"%u %llu", vec![7, 9, 0]);
        assert_eq!(text, "7 9");
    }

    #[test]
    fn printf_more_than_eight_varargs_keeps_resolving_sequentially() {
        // Slots 1..=7 would be a1..=a7 in the real CPU-backed host; slots
        // 8+ would be the stack. `compute_printf`/`VarargCursor` don't
        // themselves know which is which (that split lives in the real
        // host -- see crate::rom's full-execution test) -- this test
        // proves the *cursor* still resolves nine sequential 32-bit
        // varargs, i.e. slots 1..=9, in strict order regardless.
        let (text, _) = run_printf(
            b"%d %d %d %d %d %d %d %d %d",
            vec![10, 20, 30, 40, 50, 60, 70, 80, 90],
        );
        assert_eq!(text, "10 20 30 40 50 60 70 80 90");
    }

    #[test]
    fn printf_unsupported_specifier_is_emitted_verbatim_and_consumes_no_arg() {
        // %f is unsupported (see this function's doc, citing ets_sys.h's
        // own "cannot print floating point" caveat); it must print the
        // literal two characters "%f" and NOT consume a vararg, so the
        // %d that follows must still read slot 1, not slot 2.
        let (text, _) = run_printf(b"%f%d", vec![111, 222]);
        assert_eq!(text, "%f111");
    }

    #[test]
    fn printf_unsupported_specifier_drops_its_flags_and_width() {
        // Only `%` and the conversion character are echoed; whatever sat
        // between them is dropped (see this function's doc).
        let (text, _) = run_printf(b"[%-08.3lf]", vec![]);
        assert_eq!(text, "[%f]");
    }

    #[test]
    fn printf_trailing_percent_stops_at_the_terminator() {
        // A format string ending in a lone `%` must not emit `%` + NUL nor
        // keep scanning the bytes past its own terminator.
        let mut host = TestHost::new(vec![]);
        host.write_cstr(FMT_ADDR, b"50%");
        host.write_cstr(FMT_ADDR + 4, b"XYZ");
        let result = compute_printf(FMT_ADDR, &mut host);
        assert_eq!(result.bytes, b"50");
        assert_eq!(result.chars_written, 2);
    }

    #[test]
    fn printf_flag_scan_is_bounded_by_the_scan_cap() {
        // A `%` followed by an endless run of `0` flags (e.g. a garbage
        // pointer into a zero-filled-with-'0' buffer) must terminate.
        struct EndlessZeros {
            reads: usize,
        }
        impl PrintfHost for EndlessZeros {
            fn read_byte(&mut self, addr: u32) -> u8 {
                self.reads += 1;
                assert!(
                    self.reads <= 4 * PRINTF_MAX_SCAN_BYTES,
                    "flag scan ran past the cap"
                );
                if addr == FMT_ADDR {
                    b'%'
                } else {
                    b'0'
                }
            }
            fn slot(&mut self, _slot: u32) -> u32 {
                0
            }
        }
        let mut host = EndlessZeros { reads: 0 };
        let result = compute_printf(FMT_ADDR, &mut host);
        assert!(result.bytes.len() <= PRINTF_MAX_OUTPUT_BYTES);
    }

    #[test]
    fn printf_zero_padding_goes_after_the_sign() {
        assert_eq!(run_printf(b"%05d", vec![(-42i32) as u32]).0, "-0042");
        assert_eq!(run_printf(b"%05d", vec![42]).0, "00042");
        assert_eq!(run_printf(b"%-5d|", vec![(-42i32) as u32]).0, "-42  |");
        assert_eq!(run_printf(b"%03d", vec![(-1234i32) as u32]).0, "-1234");
        assert_eq!(
            run_printf(b"%06lld", vec![0, (-5i64) as u32, ((-5i64) >> 32) as u32]).0,
            "-00005"
        );
    }

    #[test]
    fn printf_stops_at_the_output_cap_without_hanging() {
        // A firmware format string that's just a huge repeated literal (no
        // conversions, so no vararg host calls needed) must still be
        // capped, not grown without bound.
        let fmt = vec![b'A'; PRINTF_MAX_OUTPUT_BYTES * 4];
        let (text, chars_written) = run_printf(&fmt, vec![]);
        assert_eq!(text.len(), PRINTF_MAX_OUTPUT_BYTES);
        assert_eq!(chars_written as usize, PRINTF_MAX_OUTPUT_BYTES);
    }
}

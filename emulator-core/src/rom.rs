//! The ESP32-C3's mask-ROM API surface, as a high-level-emulation (HLE) stub
//! table.
//!
//! The *mechanism* lives in `crate::cpu::rom_stubs` (chip-agnostic, per
//! `crate::cpu`'s own "knows nothing about the ESP32-C3" rule). This module
//! holds the chip-specific half: which fixed ROM addresses we intercept, and
//! what each one pretends to have done.
//!
//! ## Where the addresses come from
//!
//! ESP-IDF ships a set of linker scripts under
//! `components/esp_rom/esp32c3/ld/` that map stable mask-ROM symbol names to
//! absolute addresses — which is precisely how ESP-IDF-compiled firmware ends
//! up calling into ROM at hardcoded addresses in the first place. Every
//! address below was read out of one of them at tag `v5.5.3`, not guessed, and
//! each entry carries the symbol name the script gives it:
//!
//! - `esp32c3.rom.ld` (~1200 symbols: the `rtc_*`, `ets_*`, `Cache_*`,
//!   `rom_i2c_*` ROM API)
//! - `esp32c3.rom.libc.ld` (`memset`, `strlen`, …)
//! - `esp32c3.rom.libgcc.ld` (`__udivdi3`, `__muldi3`, …)
//! - `esp32c3.rom.api.ld` (the `esp_rom_*` aliases ESP-IDF code actually
//!   calls, e.g. `esp_rom_printf = ets_printf`)
//!
//! Fetched with, e.g.:
//! `gh api "repos/espressif/esp-idf/contents/components/esp_rom/esp32c3/ld/esp32c3.rom.ld?ref=v5.5.3" --jq '.content' | base64 -d`
//!
//! ## How this list was arrived at
//!
//! Empirically, not speculatively: boot the real image, see which address the
//! PC faults on, look that address up in the linker scripts, add the smallest
//! stub that lets boot proceed, repeat. `tests/rom_stub_boot.rs` records the
//! resulting call sequence. Nothing is stubbed "just in case" — an unstubbed
//! ROM address faults loudly with the address in `mtval`, which is a far better
//! failure than a stub quietly returning a wrong answer, so the table stays as
//! small as the firmware allows.
//!
//! ## Which functions are stubbed, and why these semantics
//!
//! 1. **`rtc_get_reset_reason` (`0x4000_0018`)** — the very first ROM call
//!    the badge's firmware makes (~20 instructions after `entry_addr`), and
//!    therefore the exact wall Task 2.1's boot run hit. ESP-IDF's
//!    `esp_system` startup path calls it to report why the chip restarted.
//!    We return **`1` = `POWERON_RESET`** (`RESET_REASON` enum,
//!    `components/esp_rom/include/esp32c3/rom/rtc.h`), because a cold
//!    power-on is exactly the scenario `crate::boot`'s shortcut boot models:
//!    a freshly-powered chip whose app image was just handed control. A
//!    deep-sleep or panic-reset code would send startup down a materially
//!    different path (restoring RTC state, dumping a stored backtrace) that
//!    nothing in this emulator has set up.
//!
//! 2. **The whole `Cache_*` family (`0x4000_04b0`..=`0x4000_057c`)** —
//!    flash-cache/MMU management. `crate::mem::bus::FirmwareBus` models no
//!    instruction or data cache at all: XIP flash reads are served directly
//!    out of the image bytes, unconditionally and always coherent (Task 2).
//!    So invalidating, suspending, resuming, locking, freezing or
//!    re-mapping a cache that doesn't exist has nothing to do, and every one
//!    of these is stubbed as an unconditional success returning `0`. Per the
//!    task brief this family is deliberately *not* individually researched:
//!    with no cache model, per-function fidelity would buy nothing. The
//!    known soft spot is the handful of `Cache_Get_*` accessors
//!    (`Cache_Get_ICache_Line_Size`, `Cache_Get_Mode`, …) whose real return
//!    value is a *datum* rather than a status — `0` is a guess there. They're
//!    left at the family default until a boot run demonstrably misbehaves on
//!    one, at which point that single function is worth researching (the
//!    brief's explicit rule).
//!
//! 3. **`ets_delay_us` (`0x4000_0050`)** — ROM's microsecond busy-wait,
//!    reached from the firmware's own trap/panic path and from its peripheral
//!    bring-up. Stubbed as a **`void` no-op**: this emulator has no wall-clock
//!    model to wait against (`crate::peripherals::systimer` advances on
//!    `step()` calls, not on real time), and a busy-wait's only observable
//!    effect is the passage of time. Returning immediately is therefore the
//!    *correct* HLE, not a shortcut — and `Void` rather than `Return(0)`
//!    because the real signature is `void ets_delay_us(uint32_t us)`, so
//!    clobbering `a0` could destroy a live value.
//!
//! 4. **`memset` (`0x4000_0354`, from `esp32c3.rom.libc.ld`)** — this
//!    firmware really does link libc's `memset` out of ROM (observed
//!    empirically: boot jumps to `0x4000_0354` with a valid `ra` 33 steps in,
//!    while clearing memory during early startup). This one is stubbed with a
//!    **real implementation** ([`crate::cpu::rom_stubs::RomStubEffect::Memset`]),
//!    not the generic zero return, and it's the one place the brief's "only
//!    research a function's real semantics if the generic no-op demonstrably
//!    doesn't work" rule bites: a `memset` that returns a plausible value
//!    without writing the bytes doesn't *stall* the caller, it silently
//!    corrupts it, which is strictly worse than faulting. So it gets the real
//!    behavior: fill `a2` bytes at `a0` with `a1`'s low byte, return `a0`.
//!
//! 5. **The `rom_i2c_*Reg*` family (`0x4000_1954`..=`0x4000_1960`)** — ROM's
//!    accessors for the on-chip *analog* register bus (ESP-IDF calls it
//!    `regi2c`), used to program the BBPLL, regulators and RTC oscillators.
//!    None of that analog domain is modeled here, so the two `write*`
//!    functions are `void` no-ops and the two `read*` functions return the
//!    generic `0`. **`0` is a guess for the reads**, flagged rather than
//!    buried: it's what made `rtc_clk_init` log
//!    `"invalid RTC_XTAL_FREQ_REG value, assume 40MHz"` and fall back to
//!    40 MHz — which is, as it happens, the badge's real crystal, so the
//!    fallback lands on the right answer by a coincidence worth knowing about.
//!
//! 6. **`ets_get_cpu_frequency` (`0x4000_0584`)** — returns the CPU clock in
//!    MHz. Stubbed as [`CPU_FREQ_MHZ`] = **160**, the ESP32-C3's maximum and
//!    ESP-IDF's own default (`CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ`). This is the
//!    clearest example of why the generic `0` default can't be universal: a
//!    frequency of zero is a divisor in the caller's tick-rate math.
//!    `ets_update_cpu_frequency` (`0x4000_0588`), its `void` setter, is a
//!    no-op.
//!
//! 7. **`ets_printf` (`0x4000_0040`)** — every line of ESP-IDF's early boot
//!    log, via the `esp_rom_printf` alias. Stubbed as `Return(0)`: there's no
//!    UART model for the output to go to, and the return value (characters
//!    written) is discarded by ESP-IDF's logging macros. Note that the format
//!    string is still readable from `a0` at the moment of the call, which is
//!    how `tests/rom_stub_boot.rs`'s trace recovers boot-log text without any
//!    of this code needing an output sink.
//!
//! 8. **libgcc's 64-bit integer helpers** ([`LIBGCC_INT64_FAMILY`]) — 32-bit
//!    RISC-V has no 64-bit divide instruction, so `uint64_t` arithmetic
//!    compiles into calls to these, and this firmware links them from ROM.
//!    **Real implementations**, for `memset`'s reason: their results feed
//!    straight into the caller's next computation.
//!
//! Anything added here later follows the same default:
//! `a0 = 0` ("succeeded, returned zero"), `pc = ra`, unless a specific
//! function's real semantics demonstrably matter — in which case *why* gets
//! documented next to it, as above.
//!
//! ## What is NOT stubbed, on purpose
//!
//! The rest of ROM libc/newlib (`memcpy`, `strlen`, `qsort`, …) and the float
//! half of ROM libgcc are **absent by design**, for the same reason `memset`
//! and `__udivdi3` are special-cased rather than defaulted: a generic
//! "return 0, do nothing" stub for a function whose *output* the caller uses
//! silently corrupts it. Each one gets a real HLE implementation when — and
//! only when — a boot run is actually observed to call it. Faulting on an
//! unstubbed ROM address is a loud, diagnosable outcome; a wrong answer from a
//! stub is not.
//!
//! ## Where this gets boot to
//!
//! With this table installed, the real `factory.bin` runs 2,000,000
//! instructions with zero traps of any kind (versus faulting ~20 instructions
//! in without it), getting through `.bss` clear, flash cache/MMU bring-up,
//! analog/PLL config and its first log line, and stalls inside SoC clock
//! initialization — spinning on a TIMERGROUP0 calibration register that no
//! peripheral module models yet. `tests/rom_stub_boot.rs` pins down both the
//! progress and the stall.

use crate::cpu::rom_stubs::{Int64Op, RomStub, RomStubTable};

/// `rtc_get_reset_reason`'s return value: `POWERON_RESET` from ESP-IDF's
/// `RESET_REASON` enum (`esp32c3/rom/rtc.h`). See the module doc for why a
/// cold power-on is the right answer for `crate::boot`'s shortcut boot.
pub const POWERON_RESET: u32 = 1;

/// `rtc_get_reset_reason`'s fixed ROM address.
pub const RTC_GET_RESET_REASON: u32 = 0x4000_0018;

/// `ets_delay_us`'s fixed ROM address (`esp32c3.rom.ld`).
pub const ETS_DELAY_US: u32 = 0x4000_0050;

/// ROM libc `memset`'s fixed address (`esp32c3.rom.libc.ld`).
pub const MEMSET: u32 = 0x4000_0354;

/// `ets_get_cpu_frequency`'s fixed ROM address (`esp32c3.rom.ld`).
pub const ETS_GET_CPU_FREQUENCY: u32 = 0x4000_0584;

/// `ets_printf`'s fixed ROM address (`esp32c3.rom.ld`). This is what
/// `esp_rom_printf` resolves to (`esp32c3.rom.api.ld`), i.e. every line of
/// ESP-IDF's early boot log.
pub const ETS_PRINTF: u32 = 0x4000_0040;

/// The CPU frequency (MHz) [`ETS_GET_CPU_FREQUENCY`]'s stub reports. 160 MHz
/// is the ESP32-C3's maximum and ESP-IDF's default
/// (`CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ`). See the module doc for why this can't
/// just be the generic `0`.
pub const CPU_FREQ_MHZ: u32 = 160;

/// Every stub with an individually-chosen name and semantics, in address
/// order. The family tables below cover the bulk-stubbed groups.
const NAMED_STUBS: &[(u32, RomStub)] = &[
    (
        RTC_GET_RESET_REASON,
        RomStub::returning("rtc_get_reset_reason", POWERON_RESET),
    ),
    (ETS_PRINTF, RomStub::returning("ets_printf", 0)),
    (ETS_DELAY_US, RomStub::void("ets_delay_us")),
    (MEMSET, RomStub::memset("memset")),
    (
        ETS_GET_CPU_FREQUENCY,
        RomStub::returning("ets_get_cpu_frequency", CPU_FREQ_MHZ),
    ),
    (0x4000_0588, RomStub::void("ets_update_cpu_frequency")),
];

/// libgcc's 64-bit integer helpers, which this firmware also links out of ROM
/// (`esp32c3.rom.libgcc.ld`) — 32-bit RISC-V has no 64-bit divide, so any
/// `uint64_t` arithmetic (`esp_timer`'s microsecond clock, for one) becomes a
/// call to one of these. Each gets a **real** implementation; see
/// [`Int64Op`] for the register-pair convention and the divide-by-zero
/// choice.
///
/// Only the routines observed in a boot run, plus their obvious siblings
/// (signed/unsigned, the three shift directions), are listed — the float and
/// bit-counting halves of `libgcc.ld` are left to fault loudly until something
/// actually calls them.
const LIBGCC_INT64_FAMILY: &[(u32, &str, Int64Op)] = &[
    (0x4000_077c, "__ashldi3", Int64Op::Shl),
    (0x4000_0780, "__ashrdi3", Int64Op::AShr),
    (0x4000_07b4, "__divdi3", Int64Op::Div),
    (0x4000_0830, "__lshrdi3", Int64Op::LShr),
    (0x4000_083c, "__moddi3", Int64Op::Mod),
    (0x4000_084c, "__muldi3", Int64Op::Mul),
    (0x4000_08ac, "__udivdi3", Int64Op::UDiv),
    (0x4000_08bc, "__umoddi3", Int64Op::UMod),
];

/// The `rom_i2c_*Reg*` analog-register accessors (`esp32c3.rom.ld`), the ROM
/// side of what ESP-IDF calls `regi2c` — see the module doc.
const REGI2C_FAMILY: &[(u32, &str, bool)] = &[
    // (address, symbol, returns_a_value)
    (0x4000_1954, "rom_i2c_readReg", true),
    (0x4000_1958, "rom_i2c_readReg_Mask", true),
    (0x4000_195c, "rom_i2c_writeReg", false),
    (0x4000_1960, "rom_i2c_writeReg_Mask", false),
];

/// Every `Cache_*` symbol in `esp32c3.rom.ld`, in address order. Stubbed as a
/// family — see the module doc.
const CACHE_FAMILY: &[(u32, &str)] = &[
    (0x4000_04b0, "Cache_Get_ICache_Line_Size"),
    (0x4000_04b4, "Cache_Get_Mode"),
    (0x4000_04b8, "Cache_Address_Through_IBus"),
    (0x4000_04bc, "Cache_Address_Through_DBus"),
    (0x4000_04c0, "Cache_Set_Default_Mode"),
    (0x4000_04c4, "Cache_Enable_Defalut_ICache_Mode"),
    (0x4000_04cc, "Cache_Invalidate_ICache_Items"),
    (0x4000_04d0, "Cache_Op_Addr"),
    (0x4000_04d4, "Cache_Invalidate_Addr"),
    (0x4000_04d8, "Cache_Invalidate_ICache_All"),
    (0x4000_04dc, "Cache_Mask_All"),
    (0x4000_04e0, "Cache_UnMask_Dram0"),
    (0x4000_04e4, "Cache_Suspend_ICache_Autoload"),
    (0x4000_04e8, "Cache_Resume_ICache_Autoload"),
    (0x4000_04ec, "Cache_Start_ICache_Preload"),
    (0x4000_04f0, "Cache_ICache_Preload_Done"),
    (0x4000_04f4, "Cache_End_ICache_Preload"),
    (0x4000_04f8, "Cache_Config_ICache_Autoload"),
    (0x4000_04fc, "Cache_Enable_ICache_Autoload"),
    (0x4000_0500, "Cache_Disable_ICache_Autoload"),
    (0x4000_0504, "Cache_Enable_ICache_PreLock"),
    (0x4000_0508, "Cache_Disable_ICache_PreLock"),
    (0x4000_050c, "Cache_Lock_ICache_Items"),
    (0x4000_0510, "Cache_Unlock_ICache_Items"),
    (0x4000_0514, "Cache_Lock_Addr"),
    (0x4000_0518, "Cache_Unlock_Addr"),
    (0x4000_051c, "Cache_Disable_ICache"),
    (0x4000_0520, "Cache_Enable_ICache"),
    (0x4000_0524, "Cache_Suspend_ICache"),
    (0x4000_0528, "Cache_Resume_ICache"),
    (0x4000_052c, "Cache_Freeze_ICache_Enable"),
    (0x4000_0530, "Cache_Freeze_ICache_Disable"),
    (0x4000_0534, "Cache_Pms_Lock"),
    (0x4000_0538, "Cache_Ibus_Pms_Set_Addr"),
    (0x4000_053c, "Cache_Ibus_Pms_Set_Attr"),
    (0x4000_0540, "Cache_Dbus_Pms_Set_Addr"),
    (0x4000_0544, "Cache_Dbus_Pms_Set_Attr"),
    (0x4000_0548, "Cache_Set_IDROM_MMU_Size"),
    (0x4000_054c, "Cache_Get_IROM_MMU_End"),
    (0x4000_0550, "Cache_Get_DROM_MMU_End"),
    (0x4000_0554, "Cache_Owner_Init"),
    (0x4000_0558, "Cache_Occupy_ICache_MEMORY"),
    (0x4000_055c, "Cache_MMU_Init"),
    (0x4000_0560, "Cache_Ibus_MMU_Set"),
    (0x4000_0564, "Cache_Dbus_MMU_Set"),
    (0x4000_0568, "Cache_Count_Flash_Pages"),
    (0x4000_056c, "Cache_Travel_Tag_Memory"),
    (0x4000_0570, "Cache_Get_Virtual_Addr"),
    (0x4000_0574, "Cache_Get_Memory_BaseAddr"),
    (0x4000_0578, "Cache_Get_Memory_Addr"),
    (0x4000_057c, "Cache_Get_Memory_value"),
];

/// Builds the ESP32-C3 mask-ROM HLE stub table described in this module's
/// doc, ready to hand to [`crate::cpu::Cpu::set_rom_stubs`].
///
/// `crate::boot::boot_from_factory_image_with_rom_stubs` is the normal way to
/// get a `Cpu` with this installed; constructing it directly is for tests and
/// for callers that want to extend it.
pub fn esp32c3_rom_stubs() -> RomStubTable {
    let mut table = RomStubTable::new();
    for (addr, stub) in NAMED_STUBS {
        table.insert(*addr, *stub);
    }
    for (addr, name) in CACHE_FAMILY {
        table.insert(*addr, RomStub::returning(name, 0));
    }
    for (addr, name, op) in LIBGCC_INT64_FAMILY {
        table.insert(*addr, RomStub::int64(name, *op));
    }
    for (addr, name, returns_a_value) in REGI2C_FAMILY {
        let stub = if *returns_a_value {
            RomStub::returning(name, 0)
        } else {
            RomStub::void(name)
        };
        table.insert(*addr, stub);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::rom_stubs::RomStubEffect;

    #[test]
    fn reset_reason_stub_returns_poweron() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(RTC_GET_RESET_REASON).expect("registered");
        assert_eq!(stub.name, "rtc_get_reset_reason");
        assert_eq!(stub.effect, RomStubEffect::Return(POWERON_RESET));
    }

    #[test]
    fn delay_is_a_void_noop_and_memset_is_a_real_implementation() {
        let table = esp32c3_rom_stubs();
        assert_eq!(
            table.lookup(ETS_DELAY_US).expect("registered").effect,
            RomStubEffect::Void,
            "a busy-wait's only effect is elapsed time; returning immediately \
             is correct HLE, and a void signature must not clobber a0"
        );
        assert_eq!(
            table.lookup(MEMSET).expect("registered").effect,
            RomStubEffect::Memset,
            "memset must really write the bytes -- a zero return would \
             silently corrupt the caller"
        );
    }

    #[test]
    fn whole_cache_family_is_stubbed_as_return_zero() {
        let table = esp32c3_rom_stubs();
        for (addr, name) in CACHE_FAMILY {
            let stub = table
                .lookup(*addr)
                .unwrap_or_else(|| panic!("{name} at 0x{addr:08x} should be stubbed"));
            assert_eq!(stub.name, *name);
            assert_eq!(stub.effect, RomStubEffect::Return(0), "{name}");
        }
    }

    #[test]
    fn libgcc_and_regi2c_families_get_the_right_kind_of_stub() {
        let table = esp32c3_rom_stubs();
        for (addr, name, op) in LIBGCC_INT64_FAMILY {
            let stub = table
                .lookup(*addr)
                .unwrap_or_else(|| panic!("{name} at 0x{addr:08x} should be stubbed"));
            assert_eq!(stub.name, *name);
            assert_eq!(stub.effect, RomStubEffect::Int64(*op), "{name}");
        }
        for (addr, name, returns_a_value) in REGI2C_FAMILY {
            let stub = table
                .lookup(*addr)
                .unwrap_or_else(|| panic!("{name} at 0x{addr:08x} should be stubbed"));
            let expected = if *returns_a_value {
                RomStubEffect::Return(0)
            } else {
                RomStubEffect::Void
            };
            assert_eq!(stub.effect, expected, "{name}");
        }
    }

    #[test]
    fn no_two_stub_groups_claim_the_same_address() {
        // Each group is built by a separate loop into one table, so a
        // copy-paste slip could have one silently overwrite another.
        let mut seen = std::collections::BTreeSet::new();
        let named = NAMED_STUBS.iter().map(|(a, _)| *a);
        let cache = CACHE_FAMILY.iter().map(|(a, _)| *a);
        let libgcc = LIBGCC_INT64_FAMILY.iter().map(|(a, _, _)| *a);
        let regi2c = REGI2C_FAMILY.iter().map(|(a, _, _)| *a);
        let mut total = 0usize;
        for addr in named.chain(cache).chain(libgcc).chain(regi2c) {
            assert!(seen.insert(addr), "0x{addr:08x} is listed twice");
            total += 1;
        }
        assert_eq!(
            esp32c3_rom_stubs().len(),
            total,
            "every listed address should reach the built table"
        );
    }

    #[test]
    fn cache_family_addresses_are_unique_and_ascending() {
        let mut prev = 0u32;
        for (addr, name) in CACHE_FAMILY {
            assert!(
                *addr > prev,
                "{name} at 0x{addr:08x} breaks ascending order"
            );
            prev = *addr;
        }
    }

    #[test]
    fn known_cache_family_spelling_matches_the_linker_script() {
        // Two entries worth pinning literally: `Cache_Resume_ICache` at
        // 0x40000528 is the exact address Task 2.1 observed re-faulting, and
        // `Cache_Enable_Defalut_ICache_Mode`'s spelling is a real typo in
        // Espressif's own linker script -- not ours to "fix", since the name
        // is only meaningful if it matches the source it was read from.
        let table = esp32c3_rom_stubs();
        assert_eq!(
            table.lookup(0x4000_0528).unwrap().name,
            "Cache_Resume_ICache"
        );
        assert_eq!(
            table.lookup(0x4000_04c4).unwrap().name,
            "Cache_Enable_Defalut_ICache_Mode"
        );
    }

    #[test]
    fn no_rom_libc_function_carries_a_generic_zero_return_stub() {
        // Guard for the module doc's "NOT stubbed, on purpose" section: a
        // zero-returning `strlen`/`memcpy` would silently corrupt boot rather
        // than unblock it, so ROM libc entries are either absent or carry a
        // real implementation -- never the generic default. Addresses from
        // esp32c3.rom.libc.ld.
        let table = esp32c3_rom_stubs();
        for addr in [
            0x4000_0374u32, /* strlen */
            0x4000_03c8,    /* memchr */
        ] {
            assert_eq!(
                table.lookup(addr),
                None,
                "ROM libc address 0x{addr:08x} must not carry a generic stub"
            );
        }
        assert_eq!(
            table.lookup(MEMSET).unwrap().effect,
            RomStubEffect::Memset,
            "the one ROM libc function that IS stubbed must be a real one"
        );
    }
}

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
//! The *driving loop* was empirical, not speculative: boot the real image,
//! see which address the PC faults on, look that address up in the linker
//! scripts, add the smallest stub that lets boot proceed, repeat.
//! `tests/rom_stub_boot.rs` records the resulting call sequence, and each
//! `NAMED_STUBS` entry (`rtc_get_reset_reason`, `ets_delay_us`, `memset`,
//! `memcpy`, `ets_efuse_get_spiconfig`, `ets_efuse_get_wp_pad`,
//! `uart_tx_wait_idle`, `intr_matrix_set`, the `esprv_intc_int_*` family,
//! `ets_get_cpu_frequency` + its setter, `ets_printf`, …) was chosen
//! because the real boot run was observed calling that exact address and the
//! generic zero-return default either stalled it or would have silently
//! corrupted a caller. The bulk of the table's ~69 entries, though — the
//! whole `Cache_*` family, the `rom_i2c_*` family, and (to a lesser degree,
//! since each does get real arithmetic semantics rather than a generic
//! zero) the libgcc 64-bit integer family — were added preemptively in one
//! shot once their linker-script *range* was known, on the documented
//! reasoning that this emulator models no cache and no analog register bus
//! at all (see "Which functions are stubbed, and why these semantics"
//! below), not because each individual address in those families was
//! independently observed being called during that boot run; most never
//! were. What *is* true of the whole table, addresses-observed or not: an
//! unstubbed ROM address faults loudly with the address in `mtval` rather
//! than a stub quietly returning a wrong answer, so adding a family in one
//! shot is a defensible way to stay ahead of the boot run's next fault
//! without individually re-deriving each member's justification.
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
//! 9. **`memcpy` (`0x4000_0358`, from
//!    `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`, not the plain
//!    `esp32c3.rom.libc.ld` — see below)** — Milestone 3 Task D2's stall:
//!    real boot faults on an unmapped instruction fetch here at step 401,761
//!    (a fixed, deterministic call target reached via `auipc`+`jalr` with a
//!    valid `ra`, confirmed by inspecting CPU state at the fault: `a0 =
//!    0x3fcdc67c` (`dst`), `a1 = 0x3fcdc670` (`src`), `a2 = 3` (`n`) — a
//!    small, in-RAM, unaligned 3-byte copy). This address is **not** in the
//!    "well-labeled" `esp32c3.rom.libc.ld` (which defines `strlen`/`strstr`/
//!    `bzero` at `0x4000_0374`/`0x378`/`0x37c` but nothing at `0x358`);
//!    it's in the sibling
//!    `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld` script, which
//!    ESP-IDF's `esp_rom`
//!    `CMakeLists.txt` links instead whenever
//!    `CONFIG_ESP_ROM_HAS_SUBOPTIMAL_NEWLIB_ON_MISALIGNED_MEMORY` is set and
//!    `CONFIG_LIBC_OPTIMIZED_MISALIGNED_ACCESS` is *not* — the ESP32-C3's
//!    default configuration — and which defines `memcpy = 0x4000_0358`
//!    exactly (plus `memmove`/`memcmp`/`strcpy`/`strncpy`/`strcmp`/`strncmp`
//!    contiguously after it; see [`MEMCPY`]'s doc). Like `memset`, this is
//!    stubbed with a **real implementation**
//!    ([`crate::cpu::rom_stubs::RomStubEffect::Memcpy`]): a generic
//!    zero/status return would silently corrupt whatever the copied bytes
//!    were meant to become, which is worse than the loud fault it replaces.
//!    The sibling functions in that same linker script (`memmove`, `memcmp`,
//!    `strcpy`, `strncpy`, `strcmp`, `strncmp`) were checked against a
//!    post-fix boot-probe re-run and were **not** observed being called
//!    within the tested budget — see `tests/rom_stub_boot.rs` and this
//!    task's report for the exact re-probe evidence — so, per this module's
//!    own "only stub what's observed" rule, they remain unstubbed for now.
//!
//! 10. **Milestone 3 Task D3's chain of six ROM calls** — boot's next stall
//!     after `memcpy`, and the five it exposed one at a time once each was
//!     unblocked in turn (this task's brief authorized exactly this:
//!     "if the next stall is another unstubbed ROM function call whose
//!     semantics are documented in a header, implement it too... repeat"):
//!     - **[`ETS_EFUSE_GET_SPICONFIG`] (`0x4000_071c`)** — returns
//!       [`EFUSE_SPICONFIG_DEFAULT_SPI_PINS`] = **`0`**, the documented
//!       "default SPI pins" value (`esp32c3/rom/efuse.h`), per the
//!       orchestrator's explicit ruling: not the generic zero-return
//!       default by coincidence, but the correct answer for a chip (like
//!       the badge) whose flash sits on the default SPI pads. The eFuse
//!       block itself remains entirely unmodeled.
//!     - **[`ETS_EFUSE_GET_WP_PAD`] (`0x4000_072c`)** — returns
//!       [`EFUSE_WP_PAD_INVALID`] = **`0x3f`**, the documented "invalid"
//!       sentinel (`esp32c3/rom/efuse.h`: "0x3f for invalid, 0~46 is
//!       valid") for "no WP pad override has been fused" — again a
//!       documented value, not a guessed pad number.
//!     - **[`UART_TX_WAIT_IDLE`] (`0x4000_0084`)** — a **`void` no-op**.
//!       `uart_tx_wait_idle(uint8_t uart_no)` busy-waits for real UART
//!       hardware to finish transmitting
//!       (`components/esp_rom/esp32c3/include/esp32c3/rom/uart.h`); this
//!       emulator models no UART TX-busy state to wait on, so — same
//!       reasoning as `ets_delay_us` — returning immediately is correct
//!       HLE, not a shortcut.
//!     - **[`INTR_MATRIX_SET`], [`ESPRV_INTC_INT_DISABLE`],
//!       [`ESPRV_INTC_INT_SET_TYPE`], [`ESPRV_INTC_INT_SET_PRIORITY`]
//!       (`0x4000_05e0`..=`0x4000_05f4`)** — four **`void` no-ops**, all
//!       from ESP-IDF's interrupt-controller bring-up
//!       (`components/riscv/include/esp_private/interrupt_deprecated.h`
//!       for the `esprv_intc_int_*` family; `intr_matrix_set` from
//!       `esp32c3/rom/ets_sys.h`). Each really does write a register
//!       `crate::peripherals::intc::InterruptController` models (MAP
//!       registers, `CPU_INT_TYPE_REG`, `CPU_INT_PRI_<n>_REG`), so this
//!       isn't the generic "no effect" case by default — it's justified
//!       per-function: a throwaway, uncommitted boot-probe (this task's
//!       report has the trace) observed `intr_matrix_set` called 62 times
//!       in a loop as `intr_matrix_set(0, model_num, 0)` for
//!       `model_num` = 0..=0x3d, `esprv_intc_int_disable(1 << 25)`, and
//!       `esprv_intc_int_set_type(25, INTR_TYPE_LEVEL)` once each — in
//!       every one of these calls the register being written is still at
//!       its power-on-reset value (`0`) at that point in boot, so a real
//!       write and a no-op are byte-identical for the *observed* calls,
//!       not merely assumed to be. `esprv_intc_int_set_priority(25, 4)` is
//!       different — its write value (`4`) is not `0`, so a real write and
//!       a no-op genuinely differ in what ends up stored — but
//!       `InterruptController`'s own module doc already documents
//!       `CPU_INT_PRI_<n>_REG` as real storage this emulator's interrupt
//!       arbitration doesn't consult in v1 (only `SYSTIMER_TARGET0` has a
//!       real signal wired), so the divergence has no effect this
//!       emulator's current fidelity observes — the same "we model no
//!       consumer for this yet" reasoning the `Cache_Get_*` family already
//!       relies on, not a fresh guess.
//!
//!     **Where this chain stops, and why**: the very next call at the same
//!     address family, `esprv_intc_int_enable(1 << 25)` (`0x4000_05e8`),
//!     is qualitatively different from its four siblings above: it *sets*
//!     bit 25 of `CPU_INT_ENABLE_REG`, a register
//!     `InterruptController::poll` genuinely *does* consult (`self.cpu_int_enable
//!     & (1 << line) != 0`) to decide whether a pending, routed source is
//!     allowed through to the CPU. Skipping this write is not a
//!     provably-inert no-op the way its four siblings are (their target
//!     registers all stayed at `0`, this one is asking to leave a register
//!     at `0` a real call would set to a nonzero value with a documented
//!     purpose), and doing the write for real would mean reaching into an
//!     already-modeled peripheral's register from a stub effect — new
//!     mechanism work this task's brief scopes out ("peripherals are out
//!     of scope here"). This is also the boundary the brief's own stop
//!     condition names directly ("a peripheral register"). So Task D3
//!     stops here and leaves `esprv_intc_int_enable` unstubbed for a later
//!     task's judgment call — see `tests/rom_stub_boot.rs`'s
//!     `boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call`
//!     and this task's report.
//!
//! Anything added here later follows the same default:
//! `a0 = 0` ("succeeded, returned zero"), `pc = ra`, unless a specific
//! function's real semantics demonstrably matter — in which case *why* gets
//! documented next to it, as above.
//!
//! ## What is NOT stubbed, on purpose
//!
//! The rest of ROM libc/newlib (`memmove`, `memcmp`, `strcpy`, `strncpy`,
//! `strcmp`, `strncmp`, `strlen`, `qsort`, …) and the float half of ROM
//! libgcc are **absent by design**, for the same reason `memset` and
//! `__udivdi3` are special-cased rather than defaulted: a generic
//! "return 0, do nothing" stub for a function whose *output* the caller uses
//! silently corrupts it. Each one gets a real HLE implementation when — and
//! only when — a boot run is actually observed to call it. Faulting on an
//! unstubbed ROM address is a loud, diagnosable outcome; a wrong answer from a
//! stub is not.
//!
//! ## Where this gets boot to
//!
//! With this table installed (as of Milestone 3 Task D3), the real
//! `factory.bin` runs past the mask-ROM wall, `.bss` clear, flash cache/MMU
//! bring-up, analog/PLL config, SoC clock init, RTC_CNTL's RTC-timer delay
//! loop (Task D1), the `memcpy` call (Task D2), prints a full ESP-IDF
//! "Guru Meditation Error" boot log, and now runs straight through that
//! panic dump's own eFuse queries, UART flush, and the start of
//! interrupt-controller bring-up (entry 10's six-function chain above),
//! stalling at step 405,806 on `esprv_intc_int_enable` (`0x4000_05e8`,
//! `unmask = 1 << 25` observed) — the first call in that chain whose real
//! effect this emulator's own peripheral model (`InterruptController::poll`)
//! actually consults, so it's deliberately left unstubbed rather than
//! guessed at. See `tests/rom_stub_boot.rs`'s
//! `boot_currently_stalls_on_the_unstubbed_esprv_intc_int_enable_rom_call`
//! and `docs/firmware-emulator-notes.md`'s "Known limitations" for the full
//! story.

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

/// ROM libc `memcpy`'s fixed address (`esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`
/// — the variant this firmware links, per that script's build-config gate;
/// see the module doc's entry 9 below). That same script defines
/// `memmove = 0x4000_035c`, `memcmp = 0x4000_0360`, `strcpy = 0x4000_0364`,
/// `strncpy = 0x4000_0368`, `strcmp = 0x4000_036c`, `strncmp = 0x4000_0370`
/// contiguously after it — none observed called in this task's boot-probe
/// re-run, so none are stubbed (see the module doc's "What is NOT stubbed"
/// section).
pub const MEMCPY: u32 = 0x4000_0358;

/// `ets_efuse_get_spiconfig`'s fixed ROM address (`esp32c3.rom.ld`).
pub const ETS_EFUSE_GET_SPICONFIG: u32 = 0x4000_071c;

/// [`ETS_EFUSE_GET_SPICONFIG`]'s stub return value: `0`, the documented
/// "default SPI pins" sentinel from `ets_efuse_get_spiconfig`'s doc comment
/// (`components/esp_rom/esp32c3/include/esp32c3/rom/efuse.h`: "0 for default
/// SPI pins. 1 for default HSPI pins. Other values define a custom pin
/// configuration mask."). See the module doc's entry 10 for why this is a
/// chosen, cited value rather than the generic zero-return default landing
/// here by coincidence.
pub const EFUSE_SPICONFIG_DEFAULT_SPI_PINS: u32 = 0;

/// `ets_efuse_get_wp_pad`'s fixed ROM address (`esp32c3.rom.ld`). The very
/// next ROM call boot makes after [`ETS_EFUSE_GET_SPICONFIG`] returns (see
/// the module doc's entry 10).
pub const ETS_EFUSE_GET_WP_PAD: u32 = 0x4000_072c;

/// [`ETS_EFUSE_GET_WP_PAD`]'s stub return value: `0x3f`, the documented
/// "invalid" sentinel from `ets_efuse_get_wp_pad`'s doc comment
/// (`components/esp_rom/esp32c3/include/esp32c3/rom/efuse.h`: "0x3f for
/// invalid. 0~46 is valid."). This emulator models no eFuse block, so "no WP
/// pad override has been fused" (the invalid sentinel) is the correct
/// answer, not a guessed pad number.
pub const EFUSE_WP_PAD_INVALID: u32 = 0x3f;

/// `uart_tx_wait_idle`'s fixed ROM address (`esp32c3.rom.ld`). Called by the
/// firmware's panic handler to flush its crash dump before rebooting (see
/// the module doc's entry 10).
pub const UART_TX_WAIT_IDLE: u32 = 0x4000_0084;

/// `intr_matrix_set`'s fixed ROM address (`esp32c3.rom.ld`). Routes a
/// peripheral interrupt source onto a CPU interrupt line by writing the
/// source's MAP register in `crate::peripherals::intc::InterruptController`
/// (see the module doc's entry 10 for why this is a `void` no-op rather
/// than a real register write).
pub const INTR_MATRIX_SET: u32 = 0x4000_05f4;

/// `esprv_intc_int_disable`'s fixed ROM address (`esp32c3.rom.ld`). Clears
/// bits in `CPU_INT_ENABLE_REG`
/// (`crate::peripherals::intc::InterruptController`); see the module doc's
/// entry 10 for why this is a `void` no-op rather than a real register
/// write.
pub const ESPRV_INTC_INT_DISABLE: u32 = 0x4000_05ec;

/// `esprv_intc_int_set_type`'s fixed ROM address (`esp32c3.rom.ld`). Sets or
/// clears a bit in `CPU_INT_TYPE_REG`
/// (`crate::peripherals::intc::InterruptController`); see the module doc's
/// entry 10 for why this is a `void` no-op rather than a real register
/// write.
pub const ESPRV_INTC_INT_SET_TYPE: u32 = 0x4000_05f0;

/// `esprv_intc_int_set_priority`'s fixed ROM address (`esp32c3.rom.ld`).
/// Writes a `CPU_INT_PRI_<n>_REG` entry
/// (`crate::peripherals::intc::InterruptController`); see the module doc's
/// entry 10 for why this is a `void` no-op (that register is real storage
/// but not yet consulted by this emulator's interrupt arbitration).
pub const ESPRV_INTC_INT_SET_PRIORITY: u32 = 0x4000_05e0;

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
    (MEMCPY, RomStub::memcpy("memcpy")),
    (
        ETS_EFUSE_GET_SPICONFIG,
        RomStub::returning("ets_efuse_get_spiconfig", EFUSE_SPICONFIG_DEFAULT_SPI_PINS),
    ),
    (
        ETS_EFUSE_GET_WP_PAD,
        RomStub::returning("ets_efuse_get_wp_pad", EFUSE_WP_PAD_INVALID),
    ),
    (UART_TX_WAIT_IDLE, RomStub::void("uart_tx_wait_idle")),
    (INTR_MATRIX_SET, RomStub::void("intr_matrix_set")),
    (
        ESPRV_INTC_INT_DISABLE,
        RomStub::void("esprv_intc_int_disable"),
    ),
    (
        ESPRV_INTC_INT_SET_TYPE,
        RomStub::void("esprv_intc_int_set_type"),
    ),
    (
        ESPRV_INTC_INT_SET_PRIORITY,
        RomStub::void("esprv_intc_int_set_priority"),
    ),
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
        assert_eq!(
            table.lookup(MEMCPY).expect("registered").effect,
            RomStubEffect::Memcpy,
            "memcpy must really copy the bytes -- a zero return would \
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
        // zero-returning `strlen`/`strcpy` would silently corrupt boot rather
        // than unblock it, so ROM libc entries are either absent or carry a
        // real implementation -- never the generic default. Addresses from
        // esp32c3.rom.libc.ld / esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld.
        let table = esp32c3_rom_stubs();
        for addr in [
            0x4000_0374u32, /* strlen */
            0x4000_03c8,    /* memchr */
            0x4000_035c,    /* memmove */
            0x4000_0360,    /* memcmp */
            0x4000_0364,    /* strcpy */
            0x4000_0368,    /* strncpy */
            0x4000_036c,    /* strcmp */
            0x4000_0370,    /* strncmp */
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
            "one of the ROM libc functions that IS stubbed must be a real one"
        );
        assert_eq!(
            table.lookup(MEMCPY).unwrap().effect,
            RomStubEffect::Memcpy,
            "one of the ROM libc functions that IS stubbed must be a real one"
        );
    }

    #[test]
    fn efuse_get_spiconfig_stub_returns_default_spi_pins() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(ETS_EFUSE_GET_SPICONFIG).expect("registered");
        assert_eq!(stub.name, "ets_efuse_get_spiconfig");
        assert_eq!(
            stub.effect,
            RomStubEffect::Return(EFUSE_SPICONFIG_DEFAULT_SPI_PINS),
            "0 is the documented 'default SPI pins' return value \
             (esp32c3/rom/efuse.h), not the generic zero-return default by \
             coincidence -- the badge's flash sits on the default SPI pads"
        );
    }

    #[test]
    fn efuse_get_wp_pad_stub_returns_the_documented_invalid_sentinel() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(ETS_EFUSE_GET_WP_PAD).expect("registered");
        assert_eq!(stub.name, "ets_efuse_get_wp_pad");
        assert_eq!(
            stub.effect,
            RomStubEffect::Return(EFUSE_WP_PAD_INVALID),
            "0x3f is the documented 'invalid' sentinel (esp32c3/rom/efuse.h) \
             for 'no WP pad override fused' -- the correct answer for an \
             unmodeled eFuse block, not a guess at a pad number"
        );
    }

    #[test]
    fn intr_matrix_set_is_a_void_noop() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(INTR_MATRIX_SET).expect("registered");
        assert_eq!(stub.name, "intr_matrix_set");
        assert_eq!(
            stub.effect,
            RomStubEffect::Void,
            "intr_matrix_set(cpu_no, model_num, intr_num) routes a \
             peripheral interrupt source onto a CPU interrupt line by \
             writing that source's MAP register in the interrupt matrix \
             (esp32c3/rom/ets_sys.h; register layout confirmed against \
             crate::peripherals::intc); this task's boot-probe observed the \
             sole call within budget as intr_matrix_set(0, 0, 0), which \
             writes 0 into a MAP register whose reset value is already 0 \
             -- a real write and a no-op are byte-identical for this \
             observed call, so this is an exact match, not a guess. See the \
             module doc's entry 10 for the caveat if a later, non-trivial \
             call is ever observed."
        );
    }

    #[test]
    fn esprv_intc_int_disable_is_a_void_noop() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(ESPRV_INTC_INT_DISABLE).expect("registered");
        assert_eq!(stub.name, "esprv_intc_int_disable");
        assert_eq!(
            stub.effect,
            RomStubEffect::Void,
            "esprv_intc_int_disable(mask) clears bits in \
             CPU_INT_ENABLE_REG, a register crate::peripherals::intc models \
             (components/riscv/include/esp_private/interrupt_deprecated.h). \
             This task's boot-probe observed the sole call within budget as \
             esprv_intc_int_disable(1 << 25), clearing a bit in a register \
             that is still at its reset value (0) at this point in boot --  \
             a real clear-bit and a no-op are byte-identical for this \
             observed call."
        );
    }

    #[test]
    fn esprv_intc_int_set_type_is_a_void_noop() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(ESPRV_INTC_INT_SET_TYPE).expect("registered");
        assert_eq!(stub.name, "esprv_intc_int_set_type");
        assert_eq!(
            stub.effect,
            RomStubEffect::Void,
            "esprv_intc_int_set_type(intr_num, type) sets or clears \
             intr_num's bit in CPU_INT_TYPE_REG, a register \
             crate::peripherals::intc models \
             (components/riscv/include/esp_private/interrupt_deprecated.h). \
             This task's boot-probe observed the sole call within budget as \
             esprv_intc_int_set_type(25, INTR_TYPE_LEVEL=0), clearing a bit \
             in a register still at its reset value (0) -- a real clear-bit \
             and a no-op are byte-identical for this observed call."
        );
    }

    #[test]
    fn esprv_intc_int_set_priority_is_a_void_noop() {
        let table = esp32c3_rom_stubs();
        let stub = table
            .lookup(ESPRV_INTC_INT_SET_PRIORITY)
            .expect("registered");
        assert_eq!(stub.name, "esprv_intc_int_set_priority");
        assert_eq!(
            stub.effect,
            RomStubEffect::Void,
            "esprv_intc_int_set_priority(rv_int_num, priority) writes a \
             CPU_INT_PRI_<n>_REG entry \
             (components/riscv/include/esp_private/interrupt_deprecated.h). \
             This task's boot-probe observed the sole call within budget as \
             esprv_intc_int_set_priority(25, 4) -- a non-trivial write \
             (unlike this task's other no-ops, the stored value really \
             would differ from a skipped write), but \
             crate::peripherals::intc::InterruptController's own module doc \
             already documents CPU_INT_PRI_<n>_REG as real storage that \
             `poll()`'s arbitration doesn't consult in v1 (only one source, \
             systimer target0, is wired to a real signal), so skipping the \
             store has no effect this emulator's current fidelity level \
             observes -- the same reasoning already used for the bulk \
             Cache_* family, not a guess at what the value should be."
        );
    }

    #[test]
    fn uart_tx_wait_idle_is_a_void_noop() {
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(UART_TX_WAIT_IDLE).expect("registered");
        assert_eq!(stub.name, "uart_tx_wait_idle");
        assert_eq!(
            stub.effect,
            RomStubEffect::Void,
            "uart_tx_wait_idle(uint8_t uart_no) busy-waits for real UART \
             hardware to finish transmitting (esp32c3/rom/uart.h); this \
             emulator models no UART TX-busy state to wait on, so returning \
             immediately without touching a0 is correct HLE, same reasoning \
             as ets_delay_us"
        );
    }
}

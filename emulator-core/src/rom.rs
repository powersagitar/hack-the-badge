//! The ESP32-C3's mask-ROM API surface, as a high-level-emulation (HLE) stub
//! table.
//!
//! The *mechanism* lives in `crate::cpu::rom_stubs` (chip-agnostic, per
//! `crate::cpu`'s own "knows nothing about the ESP32-C3" rule). This module
//! holds the chip-specific half: which fixed ROM addresses we intercept, and
//! what each one pretends to have done. It also holds the few guest-executed
//! ROM code blobs (entry 14) and ROM data tables (entry 15) that a stub
//! cannot replace.
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
//! `ets_get_cpu_frequency` + its setter, `ets_printf`, `itoa`, `strcat`, …)
//! was chosen
//! because the real boot run was observed calling that exact address and the
//! generic zero-return default either stalled it or would have silently
//! corrupted a caller. The bulk of the table's ~80 entries, though — the
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
//!    log, via the `esp_rom_printf` alias (`esp32c3.rom.api.ld`:
//!    `PROVIDE(esp_rom_printf = ets_printf)`). **Milestone 3 Task 7**: this
//!    used to be `Return(0)` (see "History" below) — a status stub that
//!    dropped every line, including a real `E (...) cpu_start: Invalid app
//!    image header` error (entry 12). Task 7's boot-probe run confirmed
//!    the firmware calls this ROM formatting entry point *directly*
//!    (`cpu_start` at `0x42001004`–`0x42001010`, and other early sites), so
//!    `esp_rom_printf` does not format IDF-side and call a ROM putc the way
//!    some other chips' ROMs do — the plan's original "don't implement
//!    printf in the stub" clause assumed otherwise and was overridden by
//!    that task's orchestrator ruling. `ets_printf` is now
//!    [`RomStubEffect::Printf`](crate::cpu::rom_stubs::RomStubEffect::Printf),
//!    a real HLE formatter
//!    ([`crate::cpu::rom_stubs::compute_printf`] — see that function's doc
//!    for the exact conversions supported, the RV32 ILP32 vararg-slot
//!    convention cited from the RISC-V calling-convention spec, and the
//!    output/scan caps): it formats `a0`'s C string against the varargs in
//!    `a1..a7`/the stack and writes each output byte through
//!    `bus.write8(USB_SERIAL_JTAG_RANGE.start, ..)` — [`USB_SERIAL_JTAG_RANGE`]'s
//!    first byte is `USB_SERIAL_JTAG_EP1_REG`'s byte-0 lane
//!    (`crate::peripherals::usb_serial_jtag`), the exact same path real
//!    FIFO TX output already takes, so every early boot-log line now
//!    reaches [`crate::peripherals::console::Console`] and therefore
//!    [`crate::runtime::FirmwareRuntime::console_output`] — see this
//!    module's tests for the full-execution proof (a real `ets_printf`
//!    call through a real `FirmwareBus`, asserting the console text) and
//!    `tests/boot_progress.rs`/`tests/rom_stub_boot.rs` for what boot's
//!    console now actually shows.
//!
//!    **History (superseded by the above)**: until Task 7, this was
//!    `Return(0)` — no UART/USB-Serial-JTAG model existed for the output to
//!    go to yet, and the return value (characters written) is discarded by
//!    ESP-IDF's logging macros regardless, so a status stub looked
//!    sufficient at the time. Task D4 fix round 1 (finding I2) flagged the
//!    resulting load-bearing caveat: because that stub never touched
//!    `Console`, *every* `ets_printf`/`ESP_EARLY_LOG*` call was invisible to
//!    `console_output`, so console emptiness at any point during boot was
//!    not evidence that nothing had been logged. That caveat no longer
//!    applies now that `ets_printf` is a real formatter.
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
//!    within the tested budget (`strncpy` and `strcmp` were later observed,
//!    after Milestone 4's flash MMU, and `strcpy` in Milestone 4 Task D-M4-2,
//!    all stubbed for real) — see `tests/rom_stub_boot.rs` and this
//!    task's report for the exact re-probe evidence — so, per this module's
//!    own "only stub what's observed" rule, they remain unstubbed for now.
//!
//! 10. **Milestone 3 Task D3's chain of seven ROM calls** — boot's next
//!     stall after `memcpy`, and the six more it exposed one at a time once
//!     each was unblocked in turn (this task's brief authorized exactly this:
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
//!       (`0x4000_05e0`..=`0x4000_05f4`)** — originally landed as four
//!       **`void` no-ops**, all from ESP-IDF's interrupt-controller
//!       bring-up (`components/riscv/include/esp_private/interrupt_deprecated.h`
//!       for the `esprv_intc_int_*` family; `intr_matrix_set` from
//!       `esp32c3/rom/ets_sys.h` — note that name, unlike the other three,
//!       is *not* one of the `esprv_intc_int_*` family, even though this
//!       task grouped all four together). Each really does write a register
//!       `crate::peripherals::intc::InterruptController` models (MAP
//!       registers, `CPU_INT_TYPE_REG`, `CPU_INT_PRI_<n>_REG`), so
//!       "no-op" wasn't the generic "no effect" case by default — it was
//!       justified per-function on the *observed* boot-probe evidence: a
//!       throwaway, uncommitted probe (this task's report has the trace)
//!       observed `intr_matrix_set` called 62 times in a loop as
//!       `intr_matrix_set(0, model_num, 0)` for `model_num` = 0..=0x3d,
//!       `esprv_intc_int_disable(1 << 25)`, and
//!       `esprv_intc_int_set_type(25, INTR_TYPE_LEVEL)` once each, with the
//!       register being written still at its power-on-reset value (`0`) at
//!       that point in boot for every one of those calls — so a real write
//!       and a no-op were byte-identical *for the calls observed*, not
//!       merely assumed to be. `esprv_intc_int_set_priority(25, 4)` was the
//!       one exception (its write value, `4`, is not `0`), justified instead
//!       on `InterruptController`'s "v1 scope" doc: `CPU_INT_PRI_<n>_REG` is
//!       real storage this emulator's interrupt arbitration didn't consult
//!       at the time. **Superseded by Fix round 1 (entry 11 below)**, which
//!       replaced all four no-ops with real register writes — the
//!       "byte-identical for the observed calls" argument was never a
//!       reason these four were safe *in general* (a later, non-trivial
//!       call, or a later task that starts consulting `CPU_INT_PRI_<n>_REG`,
//!       would have silently exposed the gap), only a reason boot could
//!       proceed for now.
//!
//!     **Where this chain stopped, and why**: the very next call at the same
//!     address family, `esprv_intc_int_enable(1 << 25)` (`0x4000_05e8`),
//!     was qualitatively different from its four siblings above: it *sets*
//!     bit 25 of `CPU_INT_ENABLE_REG`, a register
//!     `InterruptController::poll` genuinely *does* consult (`self.cpu_int_enable
//!     & (1 << line) != 0`) to decide whether a pending, routed source is
//!     allowed through to the CPU. Skipping this write was not a
//!     provably-inert no-op the way its four siblings were (their target
//!     registers all stayed at `0`; this one was asking to leave a register
//!     at `0` a real call would set to a nonzero value with a documented
//!     purpose), and doing the write for real needed new stub-mechanism
//!     plumbing this task's brief scoped out ("peripherals are out of scope
//!     here") — exactly the boundary the brief's own stop condition named
//!     ("a peripheral register"). So Task D3 stopped here and left
//!     `esprv_intc_int_enable` unstubbed for a later task's judgment
//!     call — resolved by Fix round 1, entry 11 below.
//!
//! 11. **Fix round 1: real register writes for all five interrupt-controller
//!     calls, via a new generic effect** — a review of Task D3 found the
//!     four `void` no-ops above (FINDING I1) genuinely risky: ESP-IDF's real
//!     `esp_intr_alloc` -> `esp_rom_route_intr_matrix` path calls the very
//!     same ROM `intr_matrix_set`, and once a later task wires up
//!     `CPU_INT_PRI_<n>_REG`/priority arbitration or a second real interrupt
//!     source, a firmware `intr_matrix_set`/`esprv_intc_int_set_priority`
//!     call with a *non-zero* argument would be silently dropped — plus
//!     `esprv_intc_int_enable` (`0x4000_05e8`, [`ESPRV_INTC_INT_ENABLE`])
//!     itself was still unstubbed, the exact stall Task D3 left behind.
//!     This fix round adds [`crate::cpu::rom_stubs::RomStubEffect::BusRegisterWrite`]
//!     (chip-agnostic mechanism in `cpu/rom_stubs.rs`: compute an address
//!     from a base plus an optional argument-register-indexed offset, then
//!     either overwrite the word, OR-in/AND-out a mask, or set/clear one
//!     bit — see [`crate::cpu::rom_stubs::BusRegisterOp`]) and rewires all
//!     five calls onto it, with addresses computed from
//!     [`crate::mem::soc::INTERRUPT_CORE0_RANGE`] and
//!     `crate::peripherals::intc`'s own register-offset constants (both
//!     already cited there against
//!     `components/soc/esp32c3/register/soc/interrupt_core0_reg.h`):
//!     - `intr_matrix_set(cpu_no, model_num, intr_num)` (`esp32c3/rom/ets_sys.h`)
//!       writes `intr_num` into the MAP register at
//!       `INTERRUPT_CORE0_RANGE.start + model_num * 4` — the MAP region
//!       starts at that range's offset `0x000`, one word per source, per
//!       the header's `..._MAP_REG` list.
//!     - `esprv_intc_int_disable(mask)`/`esprv_intc_int_enable(unmask)`
//!       (`components/riscv/include/esp_private/interrupt_deprecated.h`)
//!       AND-out/OR-in `mask`/`unmask` at `CPU_INT_ENABLE_REG`.
//!     - `esprv_intc_int_set_type(intr_num, type)` (same header; `enum
//!       intr_type { INTR_TYPE_LEVEL = 0, INTR_TYPE_EDGE = 1 }` per
//!       `components/riscv/include/riscv/interrupt.h`) sets `intr_num`'s bit
//!       in `CPU_INT_TYPE_REG` when `type` is nonzero, clears it otherwise.
//!     - `esprv_intc_int_set_priority(rv_int_num, priority)` (same header)
//!       writes `priority` into the `CPU_INT_PRI_<n>_REG` at
//!       `CPU_INT_PRI_BASE_REG + rv_int_num * 4`.
//!
//!     (Task 4: the two indexed calls are bounds-checked via
//!     `BusRegisterWrite::index_limit` -- `model_num < 64`,
//!     `rv_int_num < 32` -- and an out-of-range index drops the write and is
//!     counted in `Cpu::rom_stub_index_drops`.)
//!
//!     Every one of these five is still a `void` C function (see the
//!     deprecated-header signatures above), so `a0` is left untouched, same
//!     as [`crate::cpu::rom_stubs::RomStubEffect::Void`] — only the register
//!     write is new. With `esprv_intc_int_enable` now performing a real
//!     write, boot runs straight past the old `0x4000_05e8` fault and hits
//!     a **new, later, genuinely different** unstubbed ROM call: `itoa`
//!     (`0x4000_0448`, `esp32c3.rom.libc.ld`) — a ROM libc function, not an
//!     interrupt-controller one, so per this fix round's own iteration
//!     ruling ("STOP at the next stall that is not one of these five
//!     functions") this is exactly where the round stops. Boot-probe
//!     evidence: `itoa(value = 0x4200_1011, buf = 0x3fcd_c694, base = 0x10)`
//!     at step 407,471 — see `tests/rom_stub_boot.rs`'s
//!     `boot_currently_stalls_on_the_unstubbed_itoa_rom_call` and this fix
//!     round's report section for the full trace.
//!
//! 12. **Milestone 3 Task D4: ROM libc `itoa` and `strcat`** — Fix round 1's
//!     stall. Step 1 of this task's own brief required first establishing
//!     whether the `itoa` call sat on the normal boot path or an
//!     already-active error path. **Corrected in Task D4 fix round 1**
//!     (a review found the original conclusion wrong — see below): it is
//!     the *latter*. Disassembling the call site (IRAM, `0x40397262`) and
//!     its caller chain, cross-checked against `factory.bin`'s own bytes at
//!     every address, shows this is newlib's `abort()`
//!     (`components/newlib/abort.c`, ESP-IDF's override, not the ROM
//!     libc): `mv s0, ra` captures `abort()`'s own return address, then
//!     `itoa(s0 - 3, addr_buf, 16)` at `0x40397262` formats "the calling
//!     instruction's address" (`s0 - 3`, a backtrace convention) and a
//!     second `itoa(0, core_buf, 10)` at `0x40397272` formats the core ID —
//!     both *before* the loop, not inside it. The loop that follows is
//!     `strcat`-only: it concatenates a fixed 4-piece message —
//!     `"abort() was called at PC 0x"`, `addr_buf`, `" on core "`,
//!     `core_buf` (all four confirmed by reading the actual string bytes
//!     out of `factory.bin` at their literal addresses) — then calls
//!     `esp_system_abort` → `panic_abort` (the `c.unimp` trap described
//!     below).
//!
//!     `abort()`'s own caller, at `0x42001004`–`0x42001010` (confirmed via
//!     `auipc`+`jalr` target computation against the same image bytes), is
//!     `ets_printf(0x40000040)` called with the format string
//!     `"E (%lu) %s: Invalid app image header\n"` and the tag `"cpu_start"`
//!     (again, both read directly out of the image at their literal
//!     addresses) — i.e. **`cpu_start`'s own app-image-header validation
//!     rejected this image and is logging that fact before aborting.**
//!     The console showed nothing at the moment of the `itoa` call not
//!     because nothing had been logged yet, but because [`ETS_PRINTF`]'s
//!     stub is `Return(0)` and never reaches [`crate::peripherals::console::Console`]
//!     (see entry 7's caveat, added in this same fix round) — so the
//!     `cpu_start` error line was logged and silently dropped. **Boot has
//!     therefore been aborting on this check since at least Task D3 fix
//!     round 1** (whichever fix first let execution reach `cpu_start`'s
//!     header check); Task D4 did not move this wall, it only let the
//!     *existing* abort's message finish formatting instead of faulting
//!     mid-format.
//!
//!     [`crate::cpu::rom_stubs::RomStubEffect::Itoa`] gives `itoa`
//!     (`0x4000_0448`, [`ITOA`]) a real implementation
//!     ([`crate::cpu::rom_stubs::compute_itoa`], mirroring newlib's
//!     `itoa.c`+`utoa.c` byte-for-byte — see that function's doc for the
//!     exact source cited), and [`crate::cpu::rom_stubs::RomStubEffect::Strcat`]
//!     does the same for `strcat` (`0x4000_03d8`, [`STRCAT`], also
//!     `esp32c3.rom.libc.ld` — the very next unstubbed ROM libc call once
//!     `itoa` was unblocked, so per this task's own iteration ruling it was
//!     fixed in the same task). Both stubs are correct and needed
//!     regardless of this narrative correction — newlib's `abort()` calls
//!     them on real hardware too, and any later boot path (a *successful*
//!     header check, a different assertion, a hacker app's own `abort()`)
//!     would hit the exact same two calls.
//!
//!     With both unblocked, the `abort()` call above runs to completion
//!     (instead of faulting mid-format) and reaches the trap it was always
//!     going to reach: a real `ILLEGAL_INSTRUCTION` exception (RISC-V cause
//!     2) at `0x4038e4fa`, inside the app's own IRAM code — a
//!     compiler-emitted `c.unimp` (the RVC extension's all-zero 16-bit
//!     encoding, reserved to always trap), immediately preceded by a store
//!     of `g_panic_abort = true` and `g_panic_abort_details = <the message
//!     just built>` to two fixed addresses. That's `esp_system_abort` →
//!     `panic_abort()`'s mechanism (`components/esp_system/panic.c`'s
//!     `g_panic_abort`/`g_panic_abort_details`) — a deliberate,
//!     hardware-standard trap, not a decode gap. Per Task D4's own stop
//!     condition ("anything other than an unstubbed ROM libc/string call"),
//!     that task stopped there rather than chasing this further. See
//!     [`STRCAT`]'s and [`ITOA`]'s doc for the exact addresses and
//!     `tests/rom_stub_boot.rs`'s
//!     `boot_currently_aborts_reaching_the_panic_handlers_reboot_message`
//!     for the full post-abort trace: the panic handler now completes a
//!     real crash report (this abort's message) for the first time, prints
//!     ESP-IDF's generic pre-restart text, then tries to reboot via the
//!     still-unstubbed `software_reset_cpu`, faults again, and loops. That
//!     text is **panic output for `cpu_start`'s abort, not evidence of
//!     boot progress past it** — the header-check wall has not moved.
//!     **Unverified hypothesis for the actual next blocker** (flagged as
//!     such, not confirmed): `cpu_start` reads the image header through
//!     `SOC_DROM_LOW` (`0x3c00_0000`) via the flash cache/`memcpy`, and
//!     `crate::boot`'s shortcut-boot mapping may not expose the header
//!     bytes at that address the way real flash-cache bring-up would — this
//!     is a plausible next investigation, not a finding.
//!
//! 13. **Milestone 3 Task 7: `ets_printf` becomes a real formatter** — see
//!     entry 7 above for the full account. In one line: boot-probe
//!     confirmed the firmware calls `ets_printf` directly (not through an
//!     IDF-side formatter that calls a ROM putc), so `ets_printf` is now
//!     [`RomStubEffect::Printf`](crate::cpu::rom_stubs::RomStubEffect::Printf)
//!     ([`crate::cpu::rom_stubs::compute_printf`]), writing formatted bytes
//!     through the bus to [`USB_SERIAL_JTAG_RANGE`]'s FIFO register — the
//!     same path real TX output takes. This doesn't move the header-check
//!     wall (still entry 12's blocker, unresolved — that's Task D5's job,
//!     explicitly out of scope here), but every early-boot log line up to
//!     and including `cpu_start`'s own `E (%lu) %s: Invalid app image
//!     header\n` call now reaches the console instead of being silently
//!     dropped. Confirmed by this task's boot-probe re-run (report has the
//!     full console text): the spec's original ladder rungs (`cpu_start:
//!     Pro cpu start user code`, `cpu_start: cpu freq:`) do **not** appear
//!     — `cpu_start`'s header check runs and fails before either would
//!     print — so `tests/boot_progress.rs`'s new rung asserts the line
//!     boot *does* reach instead
//!     (`boot_reaches_cpu_starts_own_header_check_error_line`).
//!
//! 14. **Milestone 3 Task D6: ROM libc `qsort`, the first *guest-executed*
//!     ROM routine** — Task D5's stall: an instruction-access fault at
//!     `0x4000_0434` (`esp32c3.rom.libc.ld`: `qsort = 0x40000434;`) at step
//!     408,481. Step 1 of that task captured the call: `a0 = 0x3fcdc4d0`
//!     (a stack array), `a1 = 5` (`nmemb`), `a2 = 8` (`size`), `a3 =
//!     0x420029bc` (`compar`), `ra = 0x42002a18`. The comparator's bytes in
//!     `factory.bin` are `c.lw a0,0(a0); c.lw a5,0(a1); c.sub a0,a5;
//!     c.ret`, and the 5 elements are `{start, end}` address pairs. That
//!     matches ESP-IDF v5.5.3's `s_prepare_reserved_regions()`
//!     (`components/heap/port/memory_layout_utils.c`): `qsort(reserved,
//!     count, sizeof(soc_reserved_region_t), s_compare_reserved_regions)`,
//!     where the comparator returns `(int)r_a->start - (int)r_b->start`.
//!     Task D5's unconfirmed guess (`do_system_init_fn()`) was wrong.
//!
//!     **Why this is not a [`RomStubEffect`](crate::cpu::rom_stubs::RomStubEffect)**:
//!     every stub runs atomically inside one `step()` and then jumps to
//!     `ra`. `qsort` has to call `compar`, which is *firmware* code, many
//!     times in the middle of the sort. An atomic stub cannot run guest
//!     code, and computing the comparison in Rust would mean PC-intercepting
//!     an ESP-IDF function, which this emulator never does. So `qsort` is
//!     real RV32 machine code that the CPU executes like any other code,
//!     calling `compar` with an ordinary `jalr`:
//!     - `0x4000_0434` itself is a jump-table slot (the `esp32c3.rom*.ld`
//!       symbols are 4 bytes apart). It holds exactly one instruction,
//!       [`QSORT_SLOT`] = `jal x0, QSORT_BODY_ADDR`, so execution can never
//!       run on into `rand_r`'s slot at `0x4000_0438`. That slot and every
//!       other unstubbed ROM address still traps on fetch, as before.
//!     - The body, [`QSORT_BODY`], sits at [`QSORT_BODY_ADDR`] =
//!       `0x4003_f000`, inside [`ROM_CODE_FREE_RANGE`]. That range's doc has
//!       the evidence that no symbol in any of the 21 `esp32c3.rom*.ld`
//!       scripts points into it.
//!     - The body is a byte-swapping insertion sort, assembled at compile
//!       time with `crate::cpu::encode`'s `const fn`s (no external
//!       assembler). A test decodes every word through this crate's own
//!       decoder. See [`QSORT_BODY`]'s doc for the calling convention and
//!       the note on equal elements. That note does not matter for this
//!       caller: its regions have distinct `start`s (the function itself
//!       `assert`s `reserved[i + 1].start > reserved[i].start`), so there
//!       are no equal elements to order.
//!     - Mapping: the generic, chip-agnostic mechanism is
//!       [`RomCodeBlob`]/[`FirmwareBus::map_rom_code`], a read-only
//!       executable region in `FirmwareBus`'s ordered region list. The
//!       ESP32-C3 data is [`ESP32C3_ROM_CODE`] here, installed by
//!       [`install_esp32c3_rom_code`] from
//!       `crate::boot::boot_from_factory_image_with_rom_stubs`.
//!
//!     With `qsort` running, the call returns to the caller (step 408,906)
//!     with the array correctly sorted. **The next stall is not a ROM libc
//!     call.** The array's entry 0 is `{ets_rom_layout_p->
//!     dram0_rtos_reserved_start, SOC_DIRAM_DRAM_HIGH}`
//!     (`ESP_ROM_HAS_LAYOUT_TABLE` is set for the ESP32-C3). But
//!     `ets_rom_layout_p` is a ROM *data* pointer (`esp32c3.rom.ld`:
//!     `ets_rom_layout_p = 0x3ff1fffc;`) that nothing here backs, so it
//!     reads `0` from the catch-all, and so does the field load at `NULL +
//!     4`. The region becomes `0x00000000 - 0x3fce0000`, which overlaps the
//!     next sorted region. The firmware logs `E (0) memory_layout:
//!     SOC_RESERVE_MEMORY_REGION region range 0x00000000 - 0x3fce0000
//!     overlaps with 0x3fc80000 - 0x3fc99c00` and calls `abort()` (step
//!     408,970, then `panic_abort()`'s `ILLEGAL_INSTRUCTION` at step
//!     409,071). That is a missing ROM *data table*, outside Task D6's
//!     "atomic ROM libc call" continuation rule, so the task stopped there.
//!
//! 15. **Milestone 3 Task D7: ROM *data*, the layout table
//!     (`ets_rom_layout_p`)** — entry 14's stall. This is the first ROM
//!     *data* this emulator backs; everything above is ROM code.
//!
//!     **Which code reads it, and which fields** (disassembled from
//!     `factory.bin`'s IROM segment): `s_prepare_reserved_regions()` at
//!     `0x4200_29c4` does `lui a5, 0x3ff20; lw a5, -4(a5)` (loads
//!     `ets_rom_layout_p` at `0x3ff1_fffc`), then `lw a5, 4(a5)` (loads the
//!     struct's second field, byte offset 4) and stores it as
//!     `reserved[0].start`. `reserved[0].end` is the immediate `0x3fce0000`
//!     (`SOC_DIRAM_DRAM_HIGH`, `components/soc/esp32c3/include/soc/soc.h`).
//!     This matches ESP-IDF v5.5.3's
//!     `components/heap/port/memory_layout_utils.c`
//!     (`reserved[0].start = (intptr_t)layout->dram0_rtos_reserved_start;`,
//!     under `ESP_ROM_HAS_LAYOUT_TABLE`, which
//!     `components/esp_rom/esp32c3/esp_rom_caps.h` sets). Per
//!     `components/esp_rom/esp32c3/include/esp32c3/rom/rom_layout.h`,
//!     offset 4 of `ets_rom_layout_t` is `dram0_rtos_reserved_start` (the
//!     second of 40 `void *` fields, with `SUPPORT_BTDM` = `SUPPORT_WIFI` =
//!     1 and `SUPPORT_USB_DWCOTG` = 0). That instruction pair is the only
//!     `lui` of `0x3ff20` in the image's IROM and IRAM segments. A 12-bit
//!     load offset from a `lui 0x3ff1f` base can reach at most `0x3ff1_f7ff`.
//!     No `auipc` in either segment targets the DROM mask, and no aligned
//!     word anywhere in the image equals `0x3ff1fffc` or `0x3ff1be3c`. So
//!     nothing else in this firmware reads `ets_rom_layout_p`, and so no
//!     other field of the table is ever read.
//!
//!     **Where the values come from**: the mask ROM, not IDF source. They
//!     were read from Espressif's published ROM ELF, `esp32c3_rev3_rom.elf`,
//!     in `espressif/esp-rom-elfs` release `20241011`
//!     (`esp-rom-elfs-20241011.tar.gz`, SHA-256
//!     `921f000164a421c7628fbfee55b173384aafaa51883adc65cd27bf9b0af9e9a9`,
//!     the version ESP-IDF v5.5.3's `tools/tools.json` pins; the ELF itself
//!     has SHA-256
//!     `19ac22e08707df926fb0cf4c54795d4067b983fae8635f396ded173a6d78fc3c`).
//!     The rev3 ELF is the right one for this badge: its chip is rev v0.4
//!     (ECO3 silicon), and the firmware's minimum is v0.3. In that ELF:
//!     - `ets_rom_layout_p` (`0x3ff1fffc`, in `.rodata.interface`) holds
//!       `0x3ff1be3c`, the address of the 160-byte local object
//!       `ets_rom_layout` (in `.rodata`): [`ETS_ROM_LAYOUT`].
//!     - Its field at offset 4 is `0x3fcdf060`, equal to the ELF's own
//!       `_dram0_rtos_reserved_start` symbol: [`DRAM0_RTOS_RESERVED_START`].
//!       (The rev0 ELF differs: it has `0x3ff1be30` and `0x3fcdf260`.)
//!
//!     Only that one consumed field carries its real value. The other 39
//!     fields are `0` in [`ETS_ROM_LAYOUT_TABLE`], because the evidence above
//!     shows this firmware never reads them. The table is mapped at its
//!     real address and full size. Neither blob clashes with anything: both
//!     lie in the DROM mask (`SOC_DROM_MASK_LOW..SOC_DROM_MASK_HIGH`,
//!     [`crate::mem::soc::DROM_MASK_RANGE`]), which nothing else on the bus
//!     maps.
//!
//!     **Mechanism**: [`RomDataBlob`]/[`FirmwareBus::map_rom_data`], the
//!     data twin of entry 14's [`RomCodeBlob`]. It is a read-only region in
//!     `FirmwareBus`'s ordered region list, and it is **never fetchable**:
//!     a jump into ROM data still traps, like unmapped space. The ESP32-C3
//!     data is [`ESP32C3_ROM_DATA`], installed by
//!     [`install_esp32c3_rom_data`] from
//!     `crate::boot::boot_from_factory_image_with_rom_stubs`.
//!
//!     With it, the overlap check passes: entry 0 becomes `0x3fcdf060 -
//!     0x3fce0000` and sorts last. Boot prints `I (0) heap_init:
//!     Initializing. RAM available for dynamic allocation:`. It then faults
//!     on the 409,759th step, on the unstubbed libgcc `__clzsi2`
//!     (`0x4000_079c`, `esp32c3.rom.libgcc.ld`), called from `0x4212_9480`
//!     as `32 - __clzsi2(size)` while registering a heap region. That is a
//!     libgcc bit-count helper, not ROM data or a ROM libc/string call, so
//!     Task D7 stopped there.
//!
//! 16. **libgcc's unary 32-bit bit-count helpers** ([`LIBGCC_INT32_FAMILY`],
//!     `esp32c3.rom.libgcc.ld`) — Milestone 3 Task D8's stall. (Task D12 adds
//!     `__bswapsi2` to the same family; see entry 23.)
//!     `__clzsi2` (`0x4000_079c`) is TLSF's `fls()` (`32 - clz(size)`,
//!     observed `a0 = 0x2e6c`, result 18); `__ffssi2` (`0x4000_07d4`) is
//!     `ffs()` (observed `a0 = 0x200`, result 10, caller `0x4039_5c08`).
//!     Both are **real** ([`RomStubEffect::Int32Unary`]/[`Int32UnaryOp`]): a
//!     value in `a0`, a value out in `a0`, `pc = ra`. Semantics are from the
//!     GCC internals manual, "Integer library routines". `__clzsi2(0)` is
//!     *undefined* there; the stub returns 32 (`leading_zeros`), the natural
//!     "every bit is a leading zero" answer, and the one that makes the
//!     `32 - clz(x)` bit-length idiom return 0. `__ffssi2(0)` is defined
//!     (0). With both backed, `heap_init` prints all four `heap_init: At
//!     ...` region lines (step ~415,621). The next stall is on the 417,992nd
//!     step: ROM `esp_rom_newlib_init_common_mutexes` (`0x4000_0350`,
//!     `esp32c3.rom.libc.ld`), which Task D9 (entry 17) backed.
//!
//! 17. **Newlib-init hooks and atomic ROM libc calls** — Milestone 3 Task D9.
//!     - `esp_rom_newlib_init_common_mutexes(_LOCK_T, _LOCK_T)`
//!       ([`ESP_ROM_NEWLIB_INIT_COMMON_MUTEXES`], `0x4000_0350`;
//!       `esp32c3.rom.libc.ld`; called by `esp_newlib_locks_init()` in
//!       IDF v5.5.3's `components/newlib/src/locks.c` with two copies of a
//!       "magic" `_LOCK_T` -- the ROM has retargetable locking with no
//!       exported lock symbols on the C3). The ROM ELF
//!       (`esp32c3_rev3_rom.elf`, esp-rom-elfs 20241011; local-only) shows
//!       `0x40000350` is a `j 0x4005260e` trampoline to a 7-instruction body:
//!       `lw a4,0(a0); sw a4,0x660(0x3fcdf000); lw a4,0(a1);
//!       sw a4,0x65c(0x3fcdf000); ret`. It stores the *pointed-at words*
//!       (`*a0`, `*a1`), not the pointers, into the ROM statics
//!       `common_recursive_mutex` (`0x3fcd_f660`) and `common_mutex`
//!       (`0x3fcd_f65c`) (`llvm-nm` names). Modeled faithfully as
//!       [`RomStubEffect::StoreWords`] (then named `LoadStoreWords`) with
//!       [`NEWLIB_COMMON_MUTEX_COPIES`]:
//!       the stores go through the bus into the DRAM aperture (which
//!       `crate::boot` backs with scratch RAM, including the ROM-reserved
//!       `0x3fcd_f060..` window), leaving `a0` alone (a `void` function).
//!     - Atomic ROM libc calls hit next, each real ([`RomStubEffect`]):
//!       `strlen` (`0x4000_0374`, [`STRLEN`]), `memcmp` (`0x4000_0360`,
//!       [`MEMCMP`]), `strncmp` (`0x4000_0370`, [`STRNCMP`]), `div`
//!       (`0x4000_0428`, [`DIV`]; returns `div_t` in `a0`/`a1`). Addresses are
//!       from `esp32c3.rom.libc.ld`; each is a `j` trampoline to newlib code
//!       in the ROM ELF whose semantics the doc comment of its effect states
//!       (`memcmp`/`strncmp` return the unsigned-byte difference; `div` is
//!       truncating division, its two fixups being dead code under RISC-V
//!       `div`/`rem`). Not stubbed until observed:
//!       `strstr`, `bzero`, `ldiv`, ... (`memmove` and `memchr`
//!       came in Task 8, entry 18; `strncpy`, `0x4000_0368`, [`STRNCPY`],
//!       and `strcmp`, `0x4000_036c`, [`STRCMP`], came in Milestone 4:
//!       newlib semantics, `strncpy` copies to NUL then NUL-pads to `n`,
//!       `strcmp` returns the unsigned-byte difference at the first mismatch);
//!       `strlcat`, `0x4000_03ec`, [`STRLCAT`], came in Milestone 4 Task 5:
//!       BSD/newlib semantics, append within `siz`, return the length tried;
//!       `strspn`, `0x4000_0410`, [`STRSPN`], and `strcspn`, `0x4000_03e4`,
//!       [`STRCSPN`], came right after it, for littlefs's path walk).
//!
//!     With these, boot runs fault-free to step 442,140. The next stall is
//!     *not* ROM: ESP-IDF's flash-chip detection reads the JEDEC ID through
//!     the SPI1 flash controller (`memspi_host_read_id_hs`,
//!     `components/spi_flash/memspi_host_driver.c`), gets 0 from the
//!     unmodeled peripheral and logs `E (0) memspi: no response` (step
//!     441,439), then fails an `assert` and `abort()`s (ILLEGAL_INSTRUCTION,
//!     step 442,141). (Task 8 modeled that controller: see
//!     `crate::peripherals::flash`.)
//!
//! 18. **Atomic ROM libc calls after flash-chip detection** — Milestone 3
//!     Task 8. With the SPI1 flash controller modeled, boot reaches two more
//!     ROM libc calls, each now real ([`crate::cpu::rom_stubs::RomStubEffect`]): `memchr`
//!     (`0x4000_03c8`, [`MEMCHR`], `esp32c3.rom.libc.ld`; first reached on
//!     step 490,128 from `0x4211_8854` with `a1 = '\n'`, `a2 = 3`) and
//!     `memmove` (`0x4000_035c`, [`MEMMOVE`],
//!     `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`; step 490,143).
//!     Both are `j` trampolines to newlib code in the ROM ELF (`0x40058758`,
//!     `0x40058870`) whose semantics their effects' doc comments state. The
//!     next stall is on the 493,861st step: ROM
//!     `ets_apb_backup_init_lock_func` (`0x4000_0060`, `esp32c3.rom.ld`),
//!     neither libc nor flash, so Task 8 stopped there. (Task D10 backed it:
//!     entry 19.)
//!
//! 19. **`ets_apb_backup_init_lock_func`** ([`ETS_APB_BACKUP_INIT_LOCK_FUNC`],
//!     `0x4000_0060`, `esp32c3.rom.ld`) — Milestone 3 Task D10. Called once,
//!     from `esp_apb_backup_dma_lock_init()`
//!     (`components/esp_system/port/soc/esp32c3/apb_backup_dma.c`, run by
//!     `ESP_SYSTEM_INIT_FN(init_apb_dma, SECONDARY, BIT(0), 203)` in
//!     `components/esp_system/startup_funcs.c`) with the IRAM functions
//!     `apb_backup_dma_lock`/`_unlock` (observed on step 493,812 with
//!     `a0 = 0x4038_0512`, `a1 = 0x4038_04f8`, caller RA `0x4200_155c`;
//!     493,861 before entry 20's seeding shortened flash init by 49
//!     steps). The ROM ELF body (`0x40045fe8`: `sw a0,0x654(0x3fcdf000); sw
//!     a1,0x658(0x3fcdf000); ret`) stores the two pointers *themselves* into the ROM statics
//!     `_rom_apb_backup_lock`/`_rom_apb_backup_unlock` (`0x3fcd_f654`/
//!     `0x3fcd_f658`), the lock hooks the ROM's APB backup-DMA code uses. It
//!     is modeled faithfully, not as a `void` no-op (the D3 precedent), as
//!     [`RomStubEffect::StoreWords`] with [`WordSource::Register`]
//!     ([`APB_BACKUP_LOCK_FUNC_STORES`]); D9's pointee copy is the same
//!     effect with [`WordSource::Pointee`].
//!
//! 20. **ROM writable `.data`: the SPI-flash legacy data** — Milestone 3
//!     Task D10. `rom_spiflash_legacy_data` ([`ROM_SPIFLASH_LEGACY_DATA`],
//!     `0x3fcd_fff0`, `esp32c3.rom.ld`) is a ROM `.data` pointer, and ESP-IDF
//!     reads its flash chip's `device_id`/`chip_size` through it
//!     (`g_rom_flashchip` is `rom_spiflash_legacy_data->chip`,
//!     `components/esp_rom/include/esp_rom_spiflash.h`). On real hardware
//!     the mask ROM's reset code copies its `.data` initializer (the pointer
//!     to [`ROM_DEFAULT_SPIFLASH_LEGACY_DATA`], `0x3fcd_f5c0`, and that
//!     struct's defaults) into DRAM, and the 2nd-stage bootloader then calls
//!     `esp_rom_spiflash_config_param()` with the real chip size. The
//!     shortcut boot skips both, so before this task the pointer read 0:
//!     `esp_flash` saw `chip_size` 0 and printed `Detected size(4096k)
//!     larger than the size in the binary image header(0k)` (a line the
//!     real badge's log does not have), and the firmware's own
//!     `bootloader_flash_update_id()` stored the device ID at address 0.
//!     [`esp32c3_rom_ram_initializers`] now supplies those writes and
//!     `crate::boot::apply_ram_initializers` applies them before the first
//!     instruction; its doc cites every value. Values: from the ROM ELF
//!     (esp-rom-elfs 20241011 `esp32c3_rev3_rom.elf`, the SHA-256 recorded in
//!     entry 15), `llvm-nm` symbols `rom_spiflash_legacy_data`
//!     (`0x3fcdfff0`, section `.data.interface.spiflash_legacy`, initializer
//!     `c0 f5 cd 3f`) and `rom_default_spiflash_legacy_data` (`0x3fcdf5c0`,
//!     28 bytes, section `.data_spi_flash`); field layout
//!     `esp_rom_spiflash_legacy_data_t` / `esp_rom_spiflash_chip_t`
//!     (`esp_rom_spiflash.h`); the bootloader's overwrite from
//!     `bootloader_flash_config_esp32c3.c` `update_flash_config()` and the
//!     ROM body of `esp_rom_spiflash_config_param` (`0x4004e22e`, six `sw`
//!     through the pointer); `chip_size` from factory.bin's own header
//!     (byte 3 `0x2f`: `ESP_IMAGE_FLASH_SIZE_4MB`, `esp_app_format.h`),
//!     `device_id` `0x464016` from `bootloader_read_flash_id()`'s byte swap
//!     of the badge's JEDEC ID. With this, the warning is gone and no
//!     unmapped access below `0x100` happens during boot (before, the NULL
//!     pointer caused reads at `0x3`/`0x7`/`0x19` and the store at `0x0`).
//!
//!     **Nothing else is seeded, on purpose.** The ROM ELF has many more
//!     writable `.data`/`.bss` sections (BT/Wi-Fi/PHY state,
//!     `ets_ops_table_ptr`, `rom_spiflash_legacy_funcs`, cache state, the
//!     newlib/APB statics above, ...). Boot has not been observed to read
//!     any of them before writing it. Evidence: a temporary (uncommitted)
//!     bus trace of every read of the ROM-reserved DRAM `0x3fcd_f060..0x3fce_0000` not preceded by a
//!     write, run to the Task D10 stall, found only this struct's pointer
//!     (without the seeding) and `g_coa_funcs_p`/`coexist_funcs`
//!     (`0x3fcd_f838..0x3fcd_f840`, section `.bss.interface.rom_coexist`,
//!     all-zero in the ELF), which the coexistence library reads (and, for
//!     `coexist_funcs`, fills after finding it NULL). The whole DRAM aperture
//!     is backed with zeroed RAM (`crate::boot`), so that `.bss` is already
//!     correct. Bulk-copying the ELF's `.data` would add bytes with no
//!     observed consumer; each one is added when a probe shows a read, as
//!     here.
//!
//! 21. **After the APB call: coexistence version, interrupt threshold** —
//!     Milestone 3 Task D10. With entries 19 and 20, boot reaches two more
//!     atomic ROM calls:
//!     - `esp_coex_rom_version_get` ([`ESP_COEX_ROM_VERSION_GET`],
//!       `0x4000_18ac`, step 498,924), from the closed coexistence library
//!       during `ESP_SYSTEM_INIT_FN(init_coexist, ..., 204)`
//!       (`startup_funcs.c`: `esp_coex_adapter_register()` then
//!       `coex_pre_init()`); the caller passes the result to
//!       `coexist_printf("coexist rom version %s\n", ...)`
//!       (`components/esp_coex/src/lib_printf.c`). The ROM body (`lui
//!       a5,0x3fcdf; lw a0,0xd0(a5); ret`) returns the ROM `.data` word
//!       `coexist_rom_version` (`0x3fcd_f0d0`), whose ELF initializer
//!       `0x3ff1_b74c` points at the string `"9387209"` in ROM `.rodata`.
//!       None of the 21 `components/esp_rom/esp32c3/ld/*.ld` scripts exports
//!       that word, so firmware cannot name it; the stub returns its
//!       initializer, [`COEXIST_ROM_VERSION_STR`]. The string is backed as
//!       ROM data ([`COEXIST_ROM_VERSION_WORDS`], the D7 mechanism) because
//!       the caller formats it. (The line itself does
//!       not reach the console, in the emulator or in the real badge's log.)
//!     - `esprv_intc_int_set_threshold` ([`ESPRV_INTC_INT_SET_THRESHOLD`],
//!       `0x4000_05e4`, step 528,694), aliased as `esprv_int_set_threshold` by
//!       `components/riscv/ld/rom.api.ld` and called by FreeRTOS's
//!       `xPortStartScheduler()` (`components/freertos/FreeRTOS-Kernel/
//!       portable/riscv/port.c`) with `RVHAL_INTR_ENABLE_THRESH` = 1. The ROM
//!       body is `sw a0,0x194(0x600c2000)`, a store to `CPU_INT_THRESH_REG`,
//!       so it is a [`RomStubEffect::BusRegisterWrite`] like its entry-11
//!       siblings. (`crate::peripherals::intc` stores the threshold but does
//!       not yet consult it; see its module doc.)
//!
//!     The next stall is not a ROM call: `vPortYield()` requests the first
//!     context switch through the unmodeled SYSTEM cross-core software
//!     interrupt, so the scheduler never starts (see "Where this gets boot
//!     to").
//!
//! 22. **ROM GPIO-matrix routing: `gpio_matrix_out` and `gpio_matrix_in`**
//!     ([`GPIO_MATRIX_OUT`], [`GPIO_MATRIX_IN`]) — Milestone 3 Task D11.
//!     Inside `app_main`, `spi_bus_initialize()` ->
//!     `spicommon_bus_initialize_io()` (`components/esp_driver_spi/src/
//!     gpspi/spi_common.c:650-705`, v5.5.3) routes SPI2 (FSPI) onto the
//!     badge's display pads with `esp_rom_gpio_connect_out_signal()`/
//!     `esp_rom_gpio_connect_in_signal()`, which `esp32c3.rom.api.ld`
//!     aliases to these two ROM functions (`esp32c3.rom.ld`: `gpio_matrix_in
//!     = 0x400005a0; gpio_matrix_out = 0x400005a4;`). The first call (the
//!     571,713th step from a cold boot, caller RA `0x420e_e7b8`) is line
//!     653's `esp_rom_gpio_connect_out_signal(mosi_io_num = 10,
//!     FSPID_OUT_IDX = 65, false, false)`; with it stubbed, line 657's
//!     `esp_rom_gpio_connect_in_signal(10, FSPID_IN_IDX = 65, false)`
//!     (`gpio_matrix_in`, step 571,725, RA `0x420e_e7da`) was the next
//!     fault. Both are atomic, `void` and touch only GPIO registers. Their
//!     bodies, disassembled from the rev3 ROM ELF (each `0x4000_05xx` slot
//!     is a `j` to the body):
//!     - `gpio_matrix_out(gpio, signal_idx, out_inv, oen_inv)` (body
//!       `0x4005_1ab8`): returns at once if `gpio > 25`; otherwise stores
//!       `signal_idx | (out_inv ? 0x100 : 0) | (oen_inv ? 0x400 : 0)` to
//!       `0x6000_4554 + 4 * gpio` (`GPIO_FUNCn_OUT_SEL_CFG_REG`; bit 8
//!       `OUT_INV_SEL`, bit 10 `OEN_INV_SEL`, `OEN_SEL` left 0), then
//!       `1 << gpio` to `0x6000_4024` (`GPIO_ENABLE_W1TS_REG`). This is a
//!       [`RomStubEffect::BusRegisterWrites`] pair: the first write's
//!       `index_limit` of 26 is the ROM's own guard, and the effect's
//!       all-or-nothing bound skips the `W1TS` store with it, as the ROM
//!       does.
//!     - `gpio_matrix_in(gpio, signal_idx, inv)` (body `0x4005_1a94`):
//!       stores `gpio | (inv ? 0x20 : 0) | (gpio != 0x3a ? 0x40 : 0)` to
//!       `0x6000_4154 + 4 * signal_idx` (`GPIO_FUNCn_IN_SEL_CFG_REG`; bit 5
//!       `IN_INV_SEL`, bit 6 `SIG_IN_SEL` = "through the matrix"). The ROM
//!       neither masks `gpio` to `IN_SEL`'s 5 bits nor bounds `signal_idx`;
//!       the stub mirrors the former (the value is stored as computed) but
//!       bounds the latter at the 128 `IN_SEL_CFG` registers, dropping and
//!       counting a wild index like entry 11's indexed intc calls.
//!
//!     The values come from argument registers through
//!     [`BusRegisterOp::StoreComposed`] (a register plus per-flag bits) and
//!     [`BusRegisterOp::StoreBit`], and land in
//!     `crate::peripherals::gpio`'s new `FUNCn_OUT_SEL_CFG`/
//!     `FUNCn_IN_SEL_CFG` storage (store-only so far; see that module's doc).
//!     The rest of the `gpio_*` ROM group (`gpio_pad_select_gpio`,
//!     `gpio_pad_*`, `gpio_output_set`, ...) is not observed, so it stays
//!     unstubbed.
//!
//! 23. **libgcc `__bswapsi2`, and the bootloader's `RTC_XTAL_FREQ_REG`
//!     store** — Milestone 3 Task D12.
//!     - `__bswapsi2` (`0x4000_0788`, `esp32c3.rom.libgcc.ld`; GCC internals
//!       "Integer library routines": `int32_t __bswapsi2 (int32_t a)`
//!       returns `a` with its bytes reversed) joins entry 16's
//!       [`LIBGCC_INT32_FAMILY`] as [`Int32UnaryOp::Bswap`]. Its caller is
//!       `spi_ll_set_command()` (`hal/esp32c3/include/hal/spi_ll.h:1018-1030`,
//!       RA `0x4039_45fa`): the MSB-first branch's `HAL_SPI_SWAP_DATA_TX` is
//!       a `HAL_SWAP32` = `__builtin_bswap32` (`hal/misc.h`), a libcall on
//!       RV32IMC without Zbb. Observed once, on step 593,018, with `a0 = 0`
//!       (the first SPI2 transaction's command is 0). `__bswapdi2`
//!       (`0x4000_0784`) is not observed, so it stays unstubbed.
//!     - `RTC_XTAL_FREQ_REG` ([`RTC_XTAL_FREQ_REG`]) is `RTC_CNTL_STORE4_REG`
//!       (`0x6000_8000 + 0xb8`; `components/esp_rom/esp32c3/include/
//!       esp32c3/rom/rtc.h`, `soc/rtc_cntl_reg.h`). On real hardware the
//!       2nd-stage bootloader writes it: `bootloader_init()`
//!       (`bootloader_support/src/esp32c3/bootloader_esp32c3.c:154`) ->
//!       `bootloader_clock_configure()` (`bootloader_clock_init.c:83`) ->
//!       `rtc_clk_init(RTC_CLK_CONFIG_DEFAULT())` (`esp_hw_support/port/
//!       esp32c3/rtc_clk_init.c:50-52`) -> `rtc_clk_xtal_freq_update()`
//!       (`rtc_clk.c:368-371`) -> `clk_ll_xtal_store_freq_mhz(40)`
//!       (`hal/esp32c3/include/hal/clk_tree_ll.h:626-635`). The encoding is
//!       the MHz value in both 16-bit halves, with bit 0 of each half
//!       (`RTC_DISABLE_ROM_LOG`) kept only if already set:
//!       [`BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE`] = `0x0028_0028`.
//!       `clk_ll_xtal_load_freq_mhz()` (same header, `:645-655`) accepts a
//!       value iff the halves match and it is neither 0 nor all-ones. The
//!       shortcut boot skipped the store, so the register read its reset
//!       value 0, and both `rtc_clk_xtal_freq_get()` (`rtc_clk.c:358-364`,
//!       17 times in early boot) and `clk_hal_xtal_get_freq_mhz()`
//!       (`hal/esp32c3/clk_tree_hal.c:76-84`, from the SPI clock setup)
//!       logged an "invalid RTC_XTAL_FREQ_REG value, assume 40MHz"
//!       warning the real badge's log does not have. Seeded now through
//!       entry 20's mechanism ([`esp32c3_rom_ram_initializers`], item 3):
//!       the write goes through the bus into the RTC_CNTL model's plain
//!       storage (`crate::peripherals::rtc_cntl`'s module doc, "STORE4").
//!       The warnings were the first callers of `ets_get_cpu_frequency`/
//!       `ets_printf`, so dropping them moves the whole timeline earlier
//!       (296 steps at `qsort`'s return, 629 at the first yield).
//!
//! 24. **ROM MD5: `MD5Init`, `MD5Update`, `MD5Final`** ([`MD5_INIT`],
//!     [`MD5_UPDATE`], [`MD5_FINAL`]) — Milestone 3 Task D13. Addresses
//!     from `esp32c3.rom.ld` (`MD5Init = 0x40000614; MD5Update =
//!     0x40000618; MD5Final = 0x4000061c;`, next to `md5_vector` and the
//!     `hmac_md5*` pair, which are not stubbed); `esp32c3.rom.api.ld`
//!     aliases them as `esp_rom_md5_init`/`_update`/`_final`. No other
//!     `esp32c3.rom*.ld` file (including
//!     `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`) names an MD5
//!     symbol. The context type is `md5_context_t` from
//!     `components/esp_rom/include/esp_rom_md5.h` (v5.5.3): for every
//!     target but the ESP32-C2 it is `struct MD5Context { uint32_t buf[4];
//!     uint32_t bits[2]; uint8_t in[64]; }`, 88 bytes, and the header
//!     declares `esp_rom_md5_final(uint8_t *digest, md5_context_t
//!     *context)`, digest first. The ROM ELF (`esp32c3_rev3_rom.elf`) shows
//!     each address is a `j` trampoline to Colin Plumb's public-domain MD5
//!     (`MD5Init` `0x400369d8`, `MD5Update` `0x40036a0a`, `MD5Final`
//!     `0x40036ad2`, `MD5Transform` `0x400360d0`): `MD5Init` stores the RFC
//!     1321 IV and zeroes `bits` only; `MD5Update` adds `len << 3` to
//!     `bits[0]` with a carry and `len >> 29` to `bits[1]`, then buffers in
//!     `in`; `MD5Final` pads, copies the digest out, and ends with
//!     `memset(ctx, 0, 0x58)`, the whole context. These are real effects
//!     ([`crate::cpu::rom_stubs::RomStubEffect::Md5`] over
//!     [`crate::md5::Md5Context`]), keeping all state in the guest's
//!     context, because the caller compares the digest. The caller is
//!     `load_partitions()` (`components/esp_partition/partition.c:107-250`,
//!     with `CONFIG_PARTITION_TABLE_MD5`): `MD5Init` on step 5,555,258
//!     (`a0 = 0x3fcb_fd30`, the stack context; RA `0x420f_a6ea`), then
//!     again on step 5,583,744 (`a0 = 0x3fcb_fd40`) when the first load
//!     fails. `MD5Update`/`MD5Final` are not reached on the boot yet (see
//!     "Where this gets boot to"); they are stubbed with `MD5Init` because
//!     the three are always called together, and a unit test drives
//!     `load_partitions()`'s call pattern through all three over the
//!     synthesized partition table and gets its stored digest.
//!
//! 25. **After the RMT transmission: `strdup`, `strchr`, `strcpy`** —
//!     Milestone 4 Task D-M4-2 (ruling R10: an observed, faulting ROM libc
//!     call gets a real implementation in the task that hits it). Right
//!     after `hal_sleep` reports ready, ROM newlib `strdup` (`0x4000_03dc`,
//!     `esp32c3.rom.newlib.ld`, [`STRDUP`]) faulted (return address
//!     `0x420f_1214`). `strdup` allocates, so like `qsort` (entry 14) it is
//!     **guest-executed code**, not an atomic stub: a one-instruction slot
//!     ([`STRDUP_SLOT`]) jumps to [`STRDUP_BODY`] in
//!     [`ROM_CODE_FREE_RANGE`], which does what the rev3 ROM ELF's
//!     `strdup`/`_strdup_r` (`0x40058db2`/`0x40058dc4`) do: `__getreent()`
//!     and `_malloc_r(reent, strlen(s) + 1)` through slots 0 and 1 of the
//!     firmware's `struct syscall_stub_table` at `*`[`SYSCALL_TABLE_PTR`]
//!     (`0x3fcd_ffe0`, which ESP-IDF's `esp_newlib_init` sets; the ROM's
//!     own `__getreent`/`_malloc_r` trampolines read the same word), then
//!     `strlen` and `memcpy` through their stubbed ROM entries. Then ROM
//!     libc `strchr` (`0x4000_03e0`, [`STRCHR`]; return address
//!     `0x420f_7ee4`) and, once the app registry launches its first app,
//!     `strcpy` (`0x4000_0364`, [`STRCPY`], from the
//!     `libc-suboptimal_for_misaligned_mem` script; return address
//!     `0x4206_1f20`) faulted; both are atomic real stubs
//!     ([`RomStubEffect::Strchr`](crate::cpu::rom_stubs::RomStubEffect::Strchr),
//!     [`RomStubEffect::Strcpy`](crate::cpu::rom_stubs::RomStubEffect::Strcpy)),
//!     with C11 semantics matching the ROM's newlib code (`0x40058bf2`,
//!     `0x40058d2e`).
//!
//! Anything added here later follows the same default:
//! `a0 = 0` ("succeeded, returned zero"), `pc = ra`, unless a specific
//! function's real semantics demonstrably matter — in which case *why* gets
//! documented next to it, as above.
//!
//! ## What is NOT stubbed, on purpose
//!
//! The rest of ROM libc/newlib (`strstr`, `atoi`, …) and the rest of ROM
//! libgcc's float half (all but the four soft-double helpers in
//! [`LIBGCC_SOFT_DOUBLE_FAMILY`]) are **absent by design**, for the same
//! reason `memset` and `__udivdi3` are special-cased rather than defaulted:
//! a generic "return 0, do nothing" stub for a function whose *output* the
//! caller uses silently corrupts it. Each one gets a real HLE implementation when — and
//! only when — a boot run is actually observed to call it. Faulting on an
//! unstubbed ROM address is a loud, diagnosable outcome; a wrong answer from a
//! stub is not.
//!
//! ## Where this gets boot to
//!
//! **As of Milestone 5 Task 3**: 112 stubs: `strlcpy` (`0x4000_03f0`,
//! [`STRLCPY`]), `strtol` (`0x4000_0454`, [`STRTOL`]) and `strrchr`
//! (`0x4000_0408`, [`STRRCHR`]), each a real HLE observed faulting on the
//! console REPL's first `prov show` / `put` (decision R-T3-1; addresses from
//! `esp32c3.rom.libc.ld`, semantics from the ROM ELF's newlib code).
//!
//! **As of the end of Milestone 4**: 109 stubs and two guest-executed ROM
//! routines (`qsort`, `strdup`), unchanged since Task D-M4-2. Boot reaches
//! the first app's stable first-run screen and its START-launched
//! self-test without needing another ROM call (`tests/boot_progress.rs`'s
//! `boots_to_first_run_screen` and `first_run_screen_responds_to_start`;
//! the notes' "Current state").
//!
//! **As of Milestone 4 Task D-M4-2** (history): 109 stubs (`strchr` and `strcpy`)
//! and a second guest-executed ROM routine, `strdup` (entry 25). With RMT
//! modeled, boot launches its first app and draws its screen, with no
//! exception through step 30,000,000 (the notes' "Milestone 4 Task
//! D-M4-2" entry).
//!
//! **As of Milestone 4 Task D-M4-1** (history): 107 stubs: the four libgcc soft-double
//! helpers ([`LIBGCC_SOFT_DOUBLE_FAMILY`]) the first function after the I2C0
//! accelerometer probe calls. Boot takes no exception (checked to step
//! 30,000,000) and goes silent in the RMT driver (the notes' "Milestone 4
//! Task D-M4-1" entry).
//!
//! **As of Milestone 4 Task 5** (history): 103 stubs (Task D13's 98 plus `strncpy` and
//! `strcmp`, Milestone 4 Task 2, and `strlcat`/`strspn`/`strcspn`, Milestone 4
//! Task 5). With the flash MMU modeled (Milestone 4 Task 2),
//! `load_partitions()` reads the synthesized partition table through its own
//! `spi_flash_mmap` window and uses the ROM MD5 stubs (entry 24) end to
//! end: `MD5Init`, one `MD5Update` per table entry (4 observed), `MD5Final`,
//! a digest match, and `ESP_OK`. `tests/boot_progress.rs`'s
//! `load_partitions_accepts_the_synthesized_table_through_the_flash_mmu`
//! pins it. Boot takes no exception (checked to step 12,000,000) and then
//! stalls in a flash-controller poll, not a ROM call: see the notes'
//! "Milestone 4 Task 3" history entry.
//!
//! **As of Task D13** (history): 98 stubs (Task D12's 95 plus `MD5Init`,
//! `MD5Update` and `MD5Final`, entry 24). The `MD5Init` call on step
//! 5,555,258 returned, and boot then took no exception but stalled on the
//! unmodeled flash MMU: `load_partitions()` read zeros through its
//! `spi_flash_mmap` window and returned `ESP_ERR_NOT_FOUND` before ever
//! calling `MD5Update`. Milestone 4 Task 2 modeled the MMU.
//!
//! The Task 6 paragraph below is kept as history.
//!
//! **As of Task 6** (history; CPU and interrupt changes, no stub changes): still 95
//! stubs. `wfi` is now a real wait-for-interrupt with SYSTIMER fast-forward
//! while idle, so the idle task's `wfi` (step 596,609) retires and the
//! FreeRTOS tick wakes it on the next step. Boot then runs, with no
//! exception, through 42 FreeRTOS ticks while `app_main` renders with LVGL
//! and flushes over SPI2 with DMA (GDMA is unmodeled, so nothing is drawn),
//! until a missing stub again: on the 5,555,258th step
//! `load_partitions()` (`components/esp_partition/partition.c:107-125`)
//! calls ROM `MD5Init` (`0x4000_0614`, `esp32c3.rom.ld`; RA
//! `0x420f_a6ea`) through `esp_rom_md5_init()`. The panic handler's reboot
//! then faults on `software_reset_cpu` (step 5,795,474). See
//! `tests/rom_stub_boot.rs`'s
//! `boot_idles_through_freertos_ticks_then_faults_on_the_unstubbed_rom_md5init`.
//!
//! **As of Task 10** (history; GDMA, no stub changes): still 95 stubs, and the same
//! `MD5Init` fault on the same step. The frames LVGL flushes now reach the
//! ST7789 model through GDMA, so the framebuffer holds the boot splash by
//! then.
//!
//! **As of Task D12** (history): 95 stubs (Task 9's 94 plus `__bswapsi2`, entry 23),
//! and `RTC_XTAL_FREQ_REG` is seeded as the skipped bootloader leaves it
//! (entry 23), so no "invalid RTC_XTAL_FREQ_REG" warning prints and the
//! timeline is ~629 steps earlier by the first yield. `spi_ll_set_command()`'s
//! `__bswapsi2` call (step 593,018) returns, one SPI2 transaction completes,
//! and `app_main`'s task blocks. Then the stall is not a ROM call: the
//! FreeRTOS IDLE task's `esp_cpu_wait_for_intr()`
//! (`components/esp_hw_support/cpu.c:52-64`, from
//! `esp_vApplicationIdleHook()`) executes `wfi` at `0x4038_b8bc`, which the
//! core does not implement yet (plan Task 6), so step 596,609 takes an
//! `ILLEGAL_INSTRUCTION` exception. The panic handler's reboot then faults on
//! `software_reset_cpu` (step 836,487). Nothing is drawn. See
//! `tests/rom_stub_boot.rs`'s
//! `boot_currently_takes_an_illegal_instruction_on_the_idle_tasks_wfi` (since
//! renamed and re-pointed by Task 6, above). The Task 9 paragraph below is
//! kept as history.
//!
//! **As of Task 9** (history; SPI2 register fidelity, no stub changes): still 94
//! stubs. With SPI2's `SPI_UPDATE` self-clearing, the `spi_hal_init()` poll
//! below exits at once, and boot faults on the 602,868th step on a missing
//! stub: the libgcc ROM helper `__bswapsi2` (`0x4000_0788`,
//! `esp32c3.rom.libgcc.ld`), called from `spi_ll_set_command()`
//! (`spi_ll.h:1018-1030`; `HAL_SWAP32` = `__builtin_bswap32`) while the
//! first SPI2 transaction is set up. The panic handler's reboot then faults
//! on `software_reset_cpu`. See `tests/rom_stub_boot.rs`'s
//! `boot_currently_faults_on_the_unstubbed_rom_bswapsi2_in_spi_ll_set_command`.
//! The Task D11 paragraph below is kept as history.
//!
//! **As of Task D11** (history): 94 stubs (Task 4's 92 plus `gpio_matrix_out` and
//! `gpio_matrix_in`, entry 22). `app_main`'s SPI bus setup routes SPI2 onto
//! its pads through the GPIO matrix, and boot runs on with no exception and
//! no panic. It then spins on a peripheral, not a ROM call:
//! `spi_hal_init()` (`components/hal/spi_hal.c`) ends with
//! `spi_ll_apply_config()` (`hal/esp32c3/include/hal/spi_ll.h:264-267`),
//! which sets SPI2's `SPI_UPDATE` (`SPI_CMD_REG` bit 23) and polls it until
//! the hardware clears it. The SPI2 model stores that bit inertly, so from
//! step 584,618 the CPU spins on `0x420f_d6fc..=0x420f_d702` (plan Task 9:
//! "UPDATE self-clear"). `main_task: Calling app_main()` is still the newest
//! console line. (That pinned-stall test was re-pointed by Task 9.) The
//! Task 4 paragraph below is kept as history.
//!
//! **As of Task 4** (history; interrupt matrix + SYSTEM software
//! interrupts): still 92 stubs. `intr_matrix_set` and `esprv_intc_int_set_priority` now bound
//! their guest index (`BusRegisterWrite::index_limit`: 64 MAP registers, 32
//! lines), so a wild index is dropped and counted
//! (`Cpu::rom_stub_index_drops`) instead of landing on a neighbouring
//! register. With the SYSTEM `FROM_CPU` interrupt modeled, the yield below
//! is taken, the first task runs, and boot prints `main_task: Calling
//! app_main()`. The next stall is a ROM call again: inside `app_main`, on
//! the 571,713th step, the unstubbed `gpio_matrix_out` (`0x4000_05a4`,
//! `esp32c3.rom.ld`) faults, and the panic path's reboot then faults on the
//! unstubbed `software_reset_cpu` (`0x4000_0094`, step 812,080). See
//! `tests/rom_stub_boot.rs`'s
//! `boot_currently_faults_in_app_main_on_the_unstubbed_gpio_matrix_out_rom_call`.
//! The Task D10 paragraph below is kept as history.
//!
//! **As of Task D10** (history): 92 stubs (Task 8's 89 plus
//! `ets_apb_backup_init_lock_func`, `esp_coex_rom_version_get` and
//! `esprv_intc_int_set_threshold`, entries 19 and 21), three ROM data blobs
//! (entries 15 and 21) and the seeded SPI-flash legacy data (entry 20). The
//! `(0k)` flash-size warning is gone, and boot runs with **zero traps**
//! through FreeRTOS's scheduler start. There it stalls on a peripheral, not
//! a ROM call: `vPortYield()` requests the first context switch by writing
//! `SYSTEM_CPU_INTR_FROM_CPU_0_REG` (`0x600c_0028`, `soc/system_reg.h`) on
//! the 528,777th step; the SYSTEM peripheral is unmodeled, so no software
//! interrupt fires, `vTaskStartScheduler()` returns, and from step 528,805
//! the CPU spins on a `j .` at `0x4200_0cd2`. No task runs, so the real
//! badge's next line (`main_task: Started on CPU0`) never prints. (Pinned
//! then by `tests/rom_stub_boot.rs`'s
//! `boot_currently_spins_after_vtaskstartscheduler_returns_because_the_from_cpu_0_yield_interrupt_is_unmodeled`,
//! replaced in Task 4.) The Task 8 paragraph below is kept as history.
//!
//! **As of Task 8** (history): 89 stubs (D9's 87 plus `memchr` and `memmove`, entry
//! 18). The SPI1 flash controller is modeled (`crate::peripherals::flash`),
//! so flash-chip detection succeeds (`I (0) spi_flash: detected chip:
//! generic`, step ~446,991). Boot runs fault-free to step 493,860, then
//! faults on the unstubbed ROM `ets_apb_backup_init_lock_func`
//! (`0x4000_0060`, 493,861st step). The panic handler prints "Guru
//! Meditation Error" (~494,744), "Rebooting..." (~733,938), faults on
//! `software_reset_cpu` (734,553rd step) and loops. See
//! `tests/rom_stub_boot.rs`'s
//! `boot_currently_faults_on_the_unstubbed_ets_apb_backup_init_lock_func_call_and_reaches_the_panic_handlers_reboot_message`.
//! The Task D9 paragraph below is kept as history.
//!
//! **As of Task D9** (history): 87 stubs (D8's 82 plus `esp_rom_newlib_init_common_mutexes`,
//! `strlen`, `memcmp`, `strncmp`, `div`, entry 17). Boot runs fault-free
//! through `esp_newlib_init`'s ROM calls to step 442,140, prints `E (0)
//! memspi: no response` (step 441,439, an *error*: the SPI1 flash
//! controller is unmodeled, so the JEDEC-ID read returns 0), and aborts on
//! a failed `assert` (ILLEGAL_INSTRUCTION, 442,141st step). The panic
//! handler prints "assert failed" (~443,599), "Rebooting..." (~680,821),
//! faults on `software_reset_cpu` (681,436th step), prints "Guru Meditation
//! Error" (~682,319) and loops. (Its pinned test was re-pointed in Task
//! 8.) The Task D8 paragraph below is kept as history.
//!
//! **As of Task D8** (history): 82 stubs (D7's 80 plus `__clzsi2` and `__ffssi2`, entry
//! 16). Boot gets through `heap_init`'s whole region list and prints all
//! four `heap_init: At ...` lines, then faults on the unstubbed ROM
//! `esp_rom_newlib_init_common_mutexes` (`0x4000_0350`, step 417,992). The
//! panic handler prints "Guru Meditation Error" (step ~419,414),
//! "Rebooting..." (~658,042), faults on `software_reset_cpu` (658,657th
//! step) and loops. (Its pinned test was re-pointed in Task D9.) The Task D7
//! paragraph below is kept as history.
//!
//! **As of Task D7** (this table, 80 stubs, one guest-executed routine,
//! `qsort`, from [`ESP32C3_ROM_CODE`], and the ROM layout table from
//! [`ESP32C3_ROM_DATA`]): boot passes `cpu_start`'s header check (Task
//! D5), prints the full `cpu_start`/`app_init`/`efuse_init` log up to
//! `efuse_init: Chip rev: v0.0`, runs ROM `qsort` (entry 14), passes the
//! reserved-region check (entry 15), and prints `heap_init: Initializing.
//! RAM available for dynamic allocation:`. It then faults on the unstubbed
//! libgcc `__clzsi2` (`0x4000_079c`). The panic handler prints a "Guru
//! Meditation Error" report, faults on the still-unstubbed
//! `software_reset_cpu` and loops. **The actual blocker is now the
//! unstubbed `__clzsi2`.** See `tests/rom_stub_boot.rs`'s
//! `boot_currently_faults_on_the_unstubbed_clzsi2_call_and_reaches_the_panic_handlers_reboot_message`.
//! The rest of this section is the Task 7 snapshot, kept as history.
//!
//! With this table installed (as of Task 7), the real `factory.bin` runs
//! past the mask-ROM wall, `.bss` clear, flash cache/MMU bring-up, analog/
//! PLL config, SoC clock init, RTC_CNTL's RTC-timer delay loop (Task D1),
//! the `memcpy` call (Task D2), the eFuse queries, the UART flush, and the
//! interrupt-controller bring-up (entries 10/11) — **all of that runs
//! fault-free**. But that is *not* the same as "boot is proceeding
//! normally": `cpu_start` (ESP-IDF's early startup) rejects this image's
//! header and calls `ets_printf` — **now console-visible** (entry 13; no
//! longer entry 7's old invisibility caveat) — then `abort()` — see entry
//! 12 for the full, corrected narrative and why the original "this is
//! normal, pre-panic boot" conclusion was wrong. With `itoa`/`strcat`
//! real, that pre-existing `abort()` call runs to completion instead of
//! faulting mid-format, reaching a real, hardware-standard
//! `ILLEGAL_INSTRUCTION` trap at ESP-IDF's own `panic_abort()` (entry 12's
//! `0x4038e4fa`, *not* a ROM address) — the firmware genuinely,
//! deliberately triggering its own panic path over the header-check
//! failure, same as real hardware would. The panic handler runs to
//! completion for the first time (itoa/strcat let it actually build this
//! abort's message), prints ESP-IDF's generic pre-restart text (panic
//! output, not boot progress), then tries to reboot via the
//! still-unstubbed `software_reset_cpu` (`0x4000_0094`) — which faults
//! (this *is* an unstubbed ROM call, but only reached via the panic path,
//! so per this module's own scoping it stays unstubbed), re-entering the
//! panic handler's re-entrancy guard and retrying forever. See
//! `tests/rom_stub_boot.rs`'s
//! `boot_currently_aborts_reaching_the_panic_handlers_reboot_message` and
//! `docs/firmware-emulator-notes.md`'s "Known limitations" for the full
//! story. **The actual blocker remains `cpu_start`'s app-image-header
//! check**, unresolved by this task — Task 7 only made the boot log
//! leading up to it, and the abort/panic path following it, actually
//! readable.

use crate::boot::RamInitializer;
use crate::cpu::encode::{
    add, addi, beq, bge, bgeu, bne, jal, jalr, lbu, lui, lw, mul, sb, sub, sw, A0, A1, A2, A3, RA,
    S0, S1, S2, S3, S4, S5, SP, T0, T1, T2, T3, ZERO,
};
use crate::cpu::rom_stubs::{
    BusRegisterOp, BusRegisterWrite, CondBits, Int32UnaryOp, Int64Op, Md5Op, RegCond, RomStub,
    RomStubTable, SoftDoubleOp, WordSource, WordStore, REG_A0, REG_A1, REG_A2, REG_A3,
};
use crate::mem::bus::{FirmwareBus, RomCodeBlob, RomDataBlob};
use crate::mem::image::ImageHeader;
use crate::mem::soc::{GPIO_RANGE, INTERRUPT_CORE0_RANGE, USB_SERIAL_JTAG_RANGE};
use crate::peripherals::gpio::{
    ENABLE_W1TS_REG, FUNC0_IN_SEL_CFG_REG, FUNC0_OUT_SEL_CFG_REG, IN_SEL_CFG_COUNT,
    OUT_SEL_CFG_COUNT,
};
use crate::peripherals::intc::{
    CPU_INT_ENABLE_REG, CPU_INT_PRI_BASE_REG, CPU_INT_THRESH_REG, CPU_INT_TYPE_REG, LINE_COUNT,
    MAP_SOURCE_COUNT,
};

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
/// contiguously after it — at this task's boot-probe re-run none were observed
/// called; `strncmp`, `memmove`, `memcmp` and (Milestone 4) `strncpy`/`strcmp` have since
/// become real stubs, and the rest are not stubbed (see the module doc's "What is NOT stubbed"
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

/// `uart_tx_wait_idle`'s fixed ROM address (`esp32c3.rom.ld`). Called during
/// normal boot (UART flush ahead of a clock change), well before the panic
/// this chain eventually leads to (see the module doc's entry 10 and Fix
/// round 1's note on the corrected boot/panic ordering).
pub const UART_TX_WAIT_IDLE: u32 = 0x4000_0084;

/// `intr_matrix_set`'s fixed ROM address (`esp32c3.rom.ld`). Routes a
/// peripheral interrupt source onto a CPU interrupt line by writing that
/// source's MAP register in `crate::peripherals::intc::InterruptController`
/// — a **real** register write as of Fix round 1
/// ([`BusRegisterOp::Store`], see the module doc's entry 11). The source
/// index (`a1`) is bounded by `intc::MAP_SOURCE_COUNT` (Task 4); an
/// out-of-range one is dropped, never written past the MAP array.
pub const INTR_MATRIX_SET: u32 = 0x4000_05f4;

/// `esprv_intc_int_disable`'s fixed ROM address (`esp32c3.rom.ld`). Clears
/// bits in `CPU_INT_ENABLE_REG`
/// (`crate::peripherals::intc::InterruptController`) — a **real**
/// read-modify-write as of Fix round 1 ([`BusRegisterOp::UpdateMask`], see
/// the module doc's entry 11).
pub const ESPRV_INTC_INT_DISABLE: u32 = 0x4000_05ec;

/// `esprv_intc_int_enable`'s fixed ROM address (`esp32c3.rom.ld`). Sets bits
/// in `CPU_INT_ENABLE_REG`
/// (`crate::peripherals::intc::InterruptController`) — the boot stall Task
/// D3 left unstubbed and Fix round 1 resolves with a **real**
/// read-modify-write ([`BusRegisterOp::UpdateMask`], see the module doc's
/// entry 11).
pub const ESPRV_INTC_INT_ENABLE: u32 = 0x4000_05e8;

/// `esprv_intc_int_set_type`'s fixed ROM address (`esp32c3.rom.ld`). Sets or
/// clears a bit in `CPU_INT_TYPE_REG`
/// (`crate::peripherals::intc::InterruptController`) — a **real**
/// read-modify-write as of Fix round 1 ([`BusRegisterOp::SetOrClearBit`],
/// see the module doc's entry 11).
pub const ESPRV_INTC_INT_SET_TYPE: u32 = 0x4000_05f0;

/// `esprv_intc_int_set_threshold`'s fixed ROM address (`esp32c3.rom.ld`:
/// `esprv_intc_int_set_threshold = 0x400005e4;`). The ROM ELF shows a `j
/// 0x400529a2` trampoline to `lui a5,0x600c2; sw a0,0x194(a5); ret`: a plain
/// store of `a0` to `CPU_INT_THRESH_REG` (`INTERRUPT_CORE0` base
/// `0x600c_2000` + `0x194`, `soc/interrupt_core0_reg.h`). Module doc, entry
/// 21.
pub const ESPRV_INTC_INT_SET_THRESHOLD: u32 = 0x4000_05e4;

/// `esprv_intc_int_set_priority`'s fixed ROM address (`esp32c3.rom.ld`).
/// Writes a `CPU_INT_PRI_<n>_REG` entry
/// (`crate::peripherals::intc::InterruptController`) — a **real** register
/// write as of Fix round 1 ([`BusRegisterOp::Store`], see the module doc's
/// entry 11). Since Milestone 3 Task 4 the interrupt matrix consults it: a
/// line fires only if its priority is `>=` `CPU_INT_THRESH_REG` (see
/// `InterruptController`'s module doc). The line index (`a0`) is bounded by
/// `intc::LINE_COUNT`; an out-of-range one is dropped, never written past
/// the array.
pub const ESPRV_INTC_INT_SET_PRIORITY: u32 = 0x4000_05e0;

/// ROM `gpio_matrix_in(gpio, signal_idx, inv)` (`esp32c3.rom.ld`:
/// `gpio_matrix_in = 0x400005a0;`), aliased as
/// `esp_rom_gpio_connect_in_signal` by `esp32c3.rom.api.ld`. See the module
/// doc, entry 22.
pub const GPIO_MATRIX_IN: u32 = 0x4000_05a0;

/// `gpio_matrix_in`'s flag bits in `GPIO_FUNCn_IN_SEL_CFG_REG` (`gpio_reg.h`),
/// per the ROM body (module doc, entry 22): `inv` (`a2`, tested with `beqz`)
/// sets `GPIO_FUNC0_IN_INV_SEL` (bit 5), and every `gpio` except `0x3a` sets
/// `GPIO_SIG0_IN_SEL` (bit 6, "route through the matrix").
const GPIO_MATRIX_IN_FLAGS: &[CondBits] = &[
    CondBits {
        reg: REG_A2,
        cond: RegCond::NonZero,
        bits: 1 << 5,
    },
    CondBits {
        reg: REG_A0,
        cond: RegCond::NotEqual(0x3a),
        bits: 1 << 6,
    },
];

/// ROM `gpio_matrix_out(gpio, signal_idx, out_inv, oen_inv)`
/// (`esp32c3.rom.ld`: `gpio_matrix_out = 0x400005a4;`), aliased as
/// `esp_rom_gpio_connect_out_signal` by `esp32c3.rom.api.ld`. See the module
/// doc, entry 22.
pub const GPIO_MATRIX_OUT: u32 = 0x4000_05a4;

/// `gpio_matrix_out`'s two flag bits in `GPIO_FUNCn_OUT_SEL_CFG_REG`
/// (`gpio_reg.h`): `out_inv` (`a2`) sets `GPIO_FUNC0_OUT_INV_SEL` (bit 8),
/// `oen_inv` (`a3`) sets `GPIO_FUNC0_OEN_INV_SEL` (bit 10). The ROM tests
/// each with `beqz`, so any nonzero value counts.
const GPIO_MATRIX_OUT_FLAGS: &[CondBits] = &[
    CondBits {
        reg: REG_A2,
        cond: RegCond::NonZero,
        bits: 1 << 8,
    },
    CondBits {
        reg: REG_A3,
        cond: RegCond::NonZero,
        bits: 1 << 10,
    },
];

/// `gpio_matrix_out`'s two stores, in the ROM body's order (module doc,
/// entry 22): the pad's `GPIO_FUNCn_OUT_SEL_CFG_REG`, then its bit in
/// `GPIO_ENABLE_W1TS_REG`. The first write's `index_limit` is the ROM's own
/// `gpio > 25` early return, which skips both.
const GPIO_MATRIX_OUT_WRITES: &[BusRegisterWrite] = &[
    BusRegisterWrite {
        base: GPIO_RANGE.start + FUNC0_OUT_SEL_CFG_REG,
        index_reg: Some(REG_A0), // gpio
        index_limit: Some(OUT_SEL_CFG_COUNT),
        op: BusRegisterOp::StoreComposed {
            value_reg: REG_A1, // signal_idx
            flags: GPIO_MATRIX_OUT_FLAGS,
        },
    },
    BusRegisterWrite {
        base: GPIO_RANGE.start + ENABLE_W1TS_REG,
        index_reg: None,
        index_limit: None,
        op: BusRegisterOp::StoreBit { bit_reg: REG_A0 }, // 1 << gpio
    },
];

/// `ets_get_cpu_frequency`'s fixed ROM address (`esp32c3.rom.ld`).
pub const ETS_GET_CPU_FREQUENCY: u32 = 0x4000_0584;

/// `ets_printf`'s fixed ROM address (`esp32c3.rom.ld`). This is what
/// `esp_rom_printf` resolves to (`esp32c3.rom.api.ld`), i.e. every line of
/// ESP-IDF's early boot log.
pub const ETS_PRINTF: u32 = 0x4000_0040;

/// ROM libc `itoa`'s fixed address (`esp32c3.rom.libc.ld`: `itoa =
/// 0x40000448;`, immediately after `utoa = 0x40000444;`). See the module
/// doc's entry 12 for the boot-probe evidence and why this is a real HLE
/// implementation ([`crate::cpu::rom_stubs::RomStubEffect::Itoa`]), not a
/// generic status stub.
pub const ITOA: u32 = 0x4000_0448;

/// ROM libc `strcat`'s fixed address (`esp32c3.rom.libc.ld`: `strcat =
/// 0x400003d8;`). The very next ROM call boot makes once `itoa` returns
/// (see the module doc's entry 12) -- a real HLE implementation
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strcat`]), same reasoning.
pub const STRCAT: u32 = 0x4000_03d8;

/// `esp_rom_newlib_init_common_mutexes`'s fixed address
/// (`esp32c3.rom.libc.ld`: `esp_rom_newlib_init_common_mutexes = 0x40000350;`).
/// A real HLE effect ([`crate::cpu::rom_stubs::RomStubEffect::StoreWords`]),
/// see the module doc's entry 17 and [`NEWLIB_COMMON_MUTEX_COPIES`].
pub const ESP_ROM_NEWLIB_INIT_COMMON_MUTEXES: u32 = 0x4000_0350;

/// The two ROM-internal statics `esp_rom_newlib_init_common_mutexes` stores
/// into, from the ROM ELF's symbol table (`llvm-nm`): `0x3fcd_f65c` is
/// `common_mutex` (also aliased as `__lock___tz_mutex` etc.) and
/// `0x3fcd_f660` is `common_recursive_mutex` (`__lock___malloc_recursive_mutex`
/// etc.).
pub const ROM_COMMON_MUTEX: u32 = 0x3fcd_f65c;
/// See [`ROM_COMMON_MUTEX`].
pub const ROM_COMMON_RECURSIVE_MUTEX: u32 = 0x3fcd_f660;

/// The ROM's own disassembly (`4005260e <esp_rom_newlib_init_common_mutexes>`,
/// reached from the `0x40000350` `j` trampoline `__call_...`): `lw a4,0(a0);
/// sw a4,0x660(0x3fcdf000); lw a4,0(a1); sw a4,0x65c(0x3fcdf000); ret` --
/// i.e. `*a0 -> 0x3fcdf660`, `*a1 -> 0x3fcdf65c`, in that order.
const NEWLIB_COMMON_MUTEX_COPIES: &[WordStore] = &[
    WordStore {
        src: WordSource::Pointee(REG_A0),
        dst: ROM_COMMON_RECURSIVE_MUTEX,
    },
    WordStore {
        src: WordSource::Pointee(REG_A1),
        dst: ROM_COMMON_MUTEX,
    },
];

/// `ets_apb_backup_init_lock_func`'s fixed address (`esp32c3.rom.ld`:
/// `ets_apb_backup_init_lock_func = 0x40000060;`). A real HLE effect
/// ([`crate::cpu::rom_stubs::RomStubEffect::StoreWords`] with
/// [`WordSource::Register`]); see the module doc's entry 19 and
/// [`APB_BACKUP_LOCK_FUNC_STORES`].
pub const ETS_APB_BACKUP_INIT_LOCK_FUNC: u32 = 0x4000_0060;

/// The two ROM-internal statics `ets_apb_backup_init_lock_func` stores into,
/// from the ROM ELF's symbol table (`llvm-nm`, both local `.bss` symbols of
/// the `ets_apb_backup` group, `_bss_start_ets_apb_backup` = `0x3fcd_f654`):
/// `_rom_apb_backup_lock` (`0x3fcd_f654`) and `_rom_apb_backup_unlock`
/// (`0x3fcd_f658`).
pub const ROM_APB_BACKUP_LOCK: u32 = 0x3fcd_f654;
/// See [`ROM_APB_BACKUP_LOCK`].
pub const ROM_APB_BACKUP_UNLOCK: u32 = 0x3fcd_f658;

/// The ROM's own disassembly (`40045fe8 <ets_apb_backup_init_lock_func>`,
/// reached from the `0x40000060` `j` trampoline
/// `__call_ets_apb_backup_init_lock_func`): `lui a5,0x3fcdf; sw
/// a0,0x654(a5); lui a5,0x3fcdf; sw a1,0x658(a5); ret` -- i.e. the register
/// *values* `a0 -> 0x3fcdf654`, `a1 -> 0x3fcdf658`, in that order. No load:
/// the arguments are the lock/unlock function pointers themselves
/// (`void ets_apb_backup_init_lock_func(void(*)(void), void(*)(void))`,
/// `esp32c3/rom/apb_backup_dma.h`).
const APB_BACKUP_LOCK_FUNC_STORES: &[WordStore] = &[
    WordStore {
        src: WordSource::Register(REG_A0),
        dst: ROM_APB_BACKUP_LOCK,
    },
    WordStore {
        src: WordSource::Register(REG_A1),
        dst: ROM_APB_BACKUP_UNLOCK,
    },
];

/// ROM libc `strlen`'s fixed address (`esp32c3.rom.libc.ld`: `strlen =
/// 0x40000374;`). A real HLE implementation
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strlen`]); the ROM ELF shows
/// `0x40000374` is a `j 0x40058e8c <strlen>` trampoline. See the module doc's
/// entry 17.
pub const STRLEN: u32 = 0x4000_0374;

/// ROM libc `memcmp`'s fixed address (`esp32c3.rom.libc.ld`: `memcmp =
/// 0x40000360;`; the ROM ELF shows a `j 0x40058772 <memcmp>` trampoline). Real
/// HLE ([`crate::cpu::rom_stubs::RomStubEffect::Memcmp`]); module doc entry 17.
pub const MEMCMP: u32 = 0x4000_0360;

/// ROM libc `strncmp`'s fixed address (`esp32c3.rom.libc.ld`: `strncmp =
/// 0x40000370;`; trampoline to `0x40058fa6 <strncmp>`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strncmp`]); module doc entry 17.
pub const STRNCMP: u32 = 0x4000_0370;

/// ROM libc `strncpy`'s fixed address (`esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`:
/// `strncpy = 0x40000368;`, the variant this firmware links, same as
/// [`MEMCPY`]). Real HLE ([`crate::cpu::rom_stubs::RomStubEffect::Strncpy`]):
/// newlib `strncpy` semantics, entry 17. First called by `load_partitions()`
/// right after the first `MD5Init` (about step 5.56M).
pub const STRNCPY: u32 = 0x4000_0368;

/// ROM libc `strcmp`'s fixed address (`esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`:
/// `strcmp = 0x4000036c;`, right after [`STRNCPY`]). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strcmp`]), added in Milestone 4:
/// first called from the partition-lookup code after `load_partitions()`.
pub const STRCMP: u32 = 0x4000_036c;

/// ROM libc `strlcat`'s fixed address (`esp32c3.rom.libc.ld`: `strlcat =
/// 0x400003ec;`; the ROM ELF names that slot `__call_strlcat`, a jump to
/// newlib's `strlcat` at `0x4005_8dfa`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strlcat`]), added in Milestone 4
/// Task 5: first called by `esp_vfs_littlefs_register` after the littlefs
/// format.
pub const STRLCAT: u32 = 0x4000_03ec;

/// ROM libc `strlcpy`'s fixed address (`esp32c3.rom.libc.ld`: `strlcpy =
/// 0x400003f0;`, ROM ELF `__call_strlcpy`, a jump to newlib's `strlcpy` at
/// `0x4005_8e4e`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strlcpy`]), Milestone 5 Task 3:
/// the console REPL's first command (`0x4206_0074`, size 256) calls it.
pub const STRLCPY: u32 = 0x4000_03f0;

/// ROM libc `strtol`'s fixed address (`esp32c3.rom.libc.ld`: `strtol =
/// 0x40000454;`, ROM ELF `__call_strtol`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strtol`]), Milestone 5 Task 3:
/// the console's `put <path> <size>` parses its size with it.
pub const STRTOL: u32 = 0x4000_0454;

/// ROM libc `strrchr`'s fixed address (`esp32c3.rom.libc.ld`: `strrchr =
/// 0x40000408;`, ROM ELF `__call_strrchr`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strrchr`]), Milestone 5 Task 3:
/// `put` looks for the last `/` of its path to create parent directories.
pub const STRRCHR: u32 = 0x4000_0408;

/// ROM libc `strcspn`'s fixed address (`esp32c3.rom.libc.ld`: `strcspn =
/// 0x400003e4;`, ROM ELF `__call_strcspn`, newlib code at `0x4005_8d90`).
/// Real HLE ([`crate::cpu::rom_stubs::RomStubEffect::Strcspn`]), Milestone 4
/// Task 5: littlefs's path walk (`0x420f_71ea`) calls it right after
/// [`STRSPN`].
pub const STRCSPN: u32 = 0x4000_03e4;

/// ROM libc `strchr`'s fixed address (`esp32c3.rom.libc.ld`: `strchr =
/// 0x400003e0;`, ROM ELF `__call_strchr`, a `j 0x40058bf2 <strchr>`
/// trampoline). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strchr`]), Milestone 4 Task
/// D-M4-2: called (return address `0x420f_7ee4`) right after `hal_sleep`
/// reports the sleep manager ready.
pub const STRCHR: u32 = 0x4000_03e0;

/// ROM libc `strcpy`'s fixed address
/// (`esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`: `strcpy =
/// 0x40000364;`, the script ESP-IDF links on the ESP32-C3 by default, see
/// [`MEMCPY`]; ROM ELF `j 0x40058d2e <strcpy>`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::Strcpy`]), Milestone 4 Task
/// D-M4-2: called (return address `0x4206_1f20`) after the app registry
/// launches its first app.
pub const STRCPY: u32 = 0x4000_0364;

/// ROM libc `strspn`'s fixed address (`esp32c3.rom.libc.ld`: `strspn =
/// 0x40000410;`, ROM ELF `__call_strspn`, newlib code at `0x4005_90b0`).
/// Real HLE ([`crate::cpu::rom_stubs::RomStubEffect::Strspn`]), Milestone 4
/// Task 5: first called by littlefs's path walk (return address
/// `0x420f_721c`) after `hal_fs` mounts the filesystem.
pub const STRSPN: u32 = 0x4000_0410;

/// ROM libc `div`'s fixed address (`esp32c3.rom.libc.ld`: `div =
/// 0x40000428;`; trampoline to `0x400319c6 <div>`). Real HLE
/// ([`crate::cpu::rom_stubs::RomStubEffect::DivT`]); module doc entry 17.
pub const DIV: u32 = 0x4000_0428;

/// ROM libc `memchr`'s fixed address (`esp32c3.rom.libc.ld`: `memchr =
/// 0x400003c8;`; the ROM ELF shows a `j 0x40058758 <memchr>` trampoline).
/// Real HLE ([`crate::cpu::rom_stubs::RomStubEffect::Memchr`]); module doc
/// entry 18.
pub const MEMCHR: u32 = 0x4000_03c8;

/// ROM libc `memmove`'s fixed address
/// (`esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`: `memmove =
/// 0x4000035c;`; the ROM ELF shows a `j 0x40058870 <memmove>` trampoline).
/// Real HLE ([`crate::cpu::rom_stubs::RomStubEffect::Memmove`]); module doc
/// entry 18.
pub const MEMMOVE: u32 = 0x4000_035c;

/// ROM `MD5Init`'s fixed address (`esp32c3.rom.ld`: `MD5Init =
/// 0x40000614;`; `esp32c3.rom.api.ld`: `PROVIDE ( esp_rom_md5_init =
/// MD5Init );`). The ROM ELF shows a `j 0x400369d8 <MD5Init>` trampoline.
/// Real HLE ([`crate::cpu::rom_stubs::RomStubEffect::Md5`]); module doc
/// entry 24.
pub const MD5_INIT: u32 = 0x4000_0614;

/// ROM `MD5Update`'s fixed address (`esp32c3.rom.ld`: `MD5Update =
/// 0x40000618;`; aliased by `esp_rom_md5_update` in `esp32c3.rom.api.ld`;
/// trampoline to `0x40036a0a <MD5Update>`). Real HLE; module doc entry 24.
pub const MD5_UPDATE: u32 = 0x4000_0618;

/// ROM `MD5Final`'s fixed address (`esp32c3.rom.ld`: `MD5Final =
/// 0x4000061c;`; aliased by `esp_rom_md5_final` in `esp32c3.rom.api.ld`;
/// trampoline to `0x40036ad2 <MD5Final>`). Real HLE; module doc entry 24.
pub const MD5_FINAL: u32 = 0x4000_061c;

/// ROM libc `qsort`'s fixed address (`esp32c3.rom.libc.ld`: `qsort =
/// 0x40000434;`, between `ldiv = 0x40000430;` in the same script and
/// `rand_r = 0x40000438;`, which is in `esp32c3.rom.newlib.ld`, not
/// `esp32c3.rom.libc.ld`).
/// Unlike every [`NAMED_STUBS`] entry, this is **not** an HLE stub: it is a
/// 4-byte jump-table slot holding one real `jal x0, <body>` instruction
/// ([`QSORT_SLOT`]) into a guest-executed body at [`QSORT_BODY_ADDR`] — see
/// the module doc's entry 14 for why.
pub const QSORT: u32 = 0x4000_0434;

/// ROM newlib `strdup`'s fixed address (`esp32c3.rom.newlib.ld`: `strdup =
/// 0x400003dc;`, before `strndup = 0x40000400;`). Like [`QSORT`], a
/// guest-executed jump-table slot, not an HLE stub (module doc, entry 25):
/// `strdup` allocates through the firmware's `_malloc_r`.
pub const STRDUP: u32 = 0x4000_03dc;

/// `syscall_table_ptr`, the ROM `.bss` word (`esp32c3.rom.libc.ld`, line
/// 57; `0x3fcdffe0` in `esp32c3_rev3_rom.elf`, section
/// `.bss.interface.newlib`) through which ROM newlib reaches the
/// firmware's `struct syscall_stub_table` (`esp_rom/include/esp32c3/rom/
/// libc_stubs.h`): `__getreent` at offset 0, `_malloc_r` at offset 4.
/// ESP-IDF's newlib init stores it.
pub const SYSCALL_TABLE_PTR: u32 = 0x3fcd_ffe0;

/// The ROM addresses guest-executed ROM routine *bodies* may occupy
/// (Milestone 3 Task D6). Chosen so that no symbol in any of ESP-IDF
/// v5.5.3's 21 `components/esp_rom/esp32c3/ld/esp32c3.rom*.ld` scripts
/// points into it, and no [`esp32c3_rom_stubs`] entry uses it:
///
/// - Every IROM-mask symbol across all 21 scripts (including
///   `esp32c3.rom.libc-suboptimal_for_misaligned_mem.ld`, the `eco3`/`eco7`
///   patch scripts and all the `bt`/`ble_*` scripts) lies in
///   `0x4000_0000..=0x4000_4680`; the two highest are
///   `r_lld_res_list_clear = 0x40004638` and `r_lld_res_list_rem =
///   0x40004680` (`esp32c3.rom.eco7_bt_funcs.ld`). This range starts ~235 KiB
///   past the last of them.
/// - The only other ROM-aperture symbols are data in the DROM mask
///   (`SOC_DROM_MASK_LOW..SOC_DROM_MASK_HIGH` = `0x3ff0_0000..0x3ff2_0000`,
///   `components/soc/esp32c3/include/soc/soc.h`), all within
///   `0x3ff1_ee3c..=0x3ff1_fffc`. The range ends at `0x4004_0000`, so it
///   stays clear of the top 128 KiB of the IROM mask
///   (`SOC_IROM_MASK_LOW..SOC_IROM_MASK_HIGH` = `0x4000_0000..0x4006_0000`)
///   whether or not that part aliases the DROM mask.
/// - Every stub address in this module is below `0x4000_2000`
///   (`rom_code_never_overlaps_a_stub_and_stays_inside_the_free_rom_range`
///   checks this).
///
/// On silicon these addresses hold *some* real ROM code (the mask ROM is
/// full of unexported internals); they're "free" only in the sense that
/// matters here: nothing ESP-IDF links can reach them by name, so firmware
/// never jumps there except through our own slot.
pub const ROM_CODE_FREE_RANGE: core::ops::Range<u32> = 0x4003_f000..0x4004_0000;

/// Where [`QSORT_BODY`] is mapped: the start of [`ROM_CODE_FREE_RANGE`].
pub const QSORT_BODY_ADDR: u32 = ROM_CODE_FREE_RANGE.start;

/// `qsort`'s jump-table slot: exactly one 4-byte instruction, so linear
/// execution can never run on into `rand_r`'s slot at `0x4000_0438`.
pub const QSORT_SLOT: [u32; 1] = [jal(ZERO, (QSORT_BODY_ADDR - QSORT) as i32)];

// Instruction indices of the branch targets in QSORT_BODY (each
// instruction is 4 bytes, so a branch at index `i` to label `L` has byte
// offset `(L - i) * 4`).
const QS_OUTER: i32 = 13;
const QS_INNER: i32 = 16;
const QS_SWAP: i32 = 23;
const QS_NEXT_I: i32 = 32;
const QS_DONE: i32 = 34;

/// Byte offset from instruction index `from` to label `to`.
const fn rel(from: i32, to: i32) -> i32 {
    (to - from) * 4
}

/// `void qsort(void *base, size_t nmemb, size_t size,
///             int (*compar)(const void *, const void *))`
///
/// A straight insertion sort, run by the CPU as ordinary guest code (see the
/// module doc's entry 14). Correct per C11 §7.22.5.2: afterwards the array
/// is in ascending order according to `compar`, which is called only with
/// pointers to elements of the array. Not the newlib ROM's algorithm, so
/// elements that compare equal may end up in a different relative order
/// than on real silicon (`qsort` is not stable either way; this insertion
/// sort happens to be).
///
/// Elements are swapped byte by byte, so any `size` works (`size == 0` or
/// `nmemb < 2` returns without calling `compar` or writing the array).
/// Calling convention (RISC-V psABI, ILP32): `s0..s5` and `ra` are saved
/// in a 32-byte frame (keeping `sp` 16-byte aligned) and restored; only
/// `t0..t3` and `a0/a1` are used as scratch, and never across a `compar`
/// call, which may clobber any caller-saved register. Returns `void`.
///
/// Register roles: `s0` = base, `s1` = nmemb, `s2` = size, `s3` = compar,
/// `s4` = i (1..nmemb), `s5` = p = &base[j] (the element being sunk).
pub const QSORT_BODY: [u32; 43] = [
    // 0: prologue
    addi(SP, SP, -32), // 0  addi sp, sp, -32
    sw(RA, SP, 28),    // 1  sw   ra, 28(sp)
    sw(S0, SP, 24),    // 2  sw   s0, 24(sp)
    sw(S1, SP, 20),    // 3  sw   s1, 20(sp)
    sw(S2, SP, 16),    // 4  sw   s2, 16(sp)
    sw(S3, SP, 12),    // 5  sw   s3, 12(sp)
    sw(S4, SP, 8),     // 6  sw   s4, 8(sp)
    sw(S5, SP, 4),     // 7  sw   s5, 4(sp)
    addi(S0, A0, 0),   // 8  mv   s0, a0        # base
    addi(S1, A1, 0),   // 9  mv   s1, a1        # nmemb
    addi(S2, A2, 0),   // 10 mv   s2, a2        # size
    addi(S3, A3, 0),   // 11 mv   s3, a3        # compar
    addi(S4, ZERO, 1), // 12 li   s4, 1         # i = 1
    // OUTER:
    bgeu(S4, S1, rel(13, QS_DONE)), // 13 bgeu s4, s1, DONE  # i >= nmemb
    mul(S5, S4, S2),                // 14 mul  s5, s4, s2
    add(S5, S0, S5),                // 15 add  s5, s0, s5    # p = &base[i]
    // INNER:
    beq(S5, S0, rel(16, QS_NEXT_I)), // 16 beq s5, s0, NEXT_I  # p == base
    sub(A0, S5, S2),                 // 17 sub  a0, s5, s2    # &p[-1]
    addi(A1, S5, 0),                 // 18 mv   a1, s5        # p
    jalr(RA, S3, 0),                 // 19 jalr ra, 0(s3)     # compar(p-1, p)
    bge(ZERO, A0, rel(20, QS_NEXT_I)), // 20 blez a0, NEXT_I  # in order
    sub(T0, S5, S2),                 // 21 sub  t0, s5, s2    # left = p - size
    addi(T1, S5, 0),                 // 22 mv   t1, s5        # right = p
    // SWAP: (left runs up to p)
    lbu(T2, T0, 0),                // 23 lbu  t2, 0(t0)
    lbu(T3, T1, 0),                // 24 lbu  t3, 0(t1)
    sb(T3, T0, 0),                 // 25 sb   t3, 0(t0)
    sb(T2, T1, 0),                 // 26 sb   t2, 0(t1)
    addi(T0, T0, 1),               // 27 addi t0, t0, 1
    addi(T1, T1, 1),               // 28 addi t1, t1, 1
    bne(T0, S5, rel(29, QS_SWAP)), // 29 bne  t0, s5, SWAP
    sub(S5, S5, S2),               // 30 sub  s5, s5, s2    # p -= size
    jal(ZERO, rel(31, QS_INNER)),  // 31 j    INNER
    // NEXT_I:
    addi(S4, S4, 1),              // 32 addi s4, s4, 1     # i++
    jal(ZERO, rel(33, QS_OUTER)), // 33 j    OUTER
    // DONE: epilogue
    lw(RA, SP, 28),    // 34 lw   ra, 28(sp)
    lw(S0, SP, 24),    // 35 lw   s0, 24(sp)
    lw(S1, SP, 20),    // 36 lw   s1, 20(sp)
    lw(S2, SP, 16),    // 37 lw   s2, 16(sp)
    lw(S3, SP, 12),    // 38 lw   s3, 12(sp)
    lw(S4, SP, 8),     // 39 lw   s4, 8(sp)
    lw(S5, SP, 4),     // 40 lw   s5, 4(sp)
    addi(SP, SP, 32),  // 41 addi sp, sp, 32
    jalr(ZERO, RA, 0), // 42 ret
];

// The label indices above must match the listing (a compile-time check, so
// an edit that shifts instructions can't silently mis-target a branch).
const _: () = {
    assert!(QSORT_BODY[QS_OUTER as usize] == bgeu(S4, S1, rel(QS_OUTER, QS_DONE)));
    assert!(QSORT_BODY[QS_INNER as usize] == beq(S5, S0, rel(QS_INNER, QS_NEXT_I)));
    assert!(QSORT_BODY[QS_SWAP as usize] == lbu(T2, T0, 0));
    assert!(QSORT_BODY[QS_NEXT_I as usize] == addi(S4, S4, 1));
    assert!(QSORT_BODY[QS_DONE as usize] == lw(RA, SP, 28));
    assert!(QSORT_BODY_ADDR + 4 * QSORT_BODY.len() as u32 <= ROM_CODE_FREE_RANGE.end);
};

/// Where [`STRDUP_BODY`] is mapped: right after [`QSORT_BODY`], inside
/// [`ROM_CODE_FREE_RANGE`].
pub const STRDUP_BODY_ADDR: u32 = QSORT_BODY_ADDR + 4 * QSORT_BODY.len() as u32;

/// `strdup`'s jump-table slot: one 4-byte `jal x0`, so execution can never
/// run on into the next slot (as on silicon, where the slot is `j
/// 0x40058db2 <strdup>`).
pub const STRDUP_SLOT: [u32; 1] = [jal(ZERO, (STRDUP_BODY_ADDR - STRDUP) as i32)];

// Instruction indices in STRDUP_BODY that the encoding below refers to.
const SD_STRLEN_CALL: i32 = 13;
const SD_NULL_CHECK: i32 = 20;
const SD_MEMCPY_CALL: i32 = 23;
const SD_DONE: i32 = 24;

/// Byte offset from instruction index `from` of [`STRDUP_BODY`] to the
/// absolute ROM address `target` (a stub's fixed address).
const fn sd_rel_abs(from: i32, target: u32) -> i32 {
    target.wrapping_sub(STRDUP_BODY_ADDR + 4 * from as u32) as i32
}

/// `char *strdup(const char *s)`, as ROM newlib implements it (the rev3
/// ROM ELF's `strdup` at `0x40058db2` tail-calls `_strdup_r(__getreent(),
/// s)`, which calls `strlen`, `_malloc_r(reent, len + 1)` and, if that
/// succeeded, `memcpy(copy, s, len + 1)`). `__getreent` and `_malloc_r`
/// are reached the way the ROM's own trampolines reach them: through
/// `*`[`SYSCALL_TABLE_PTR`], slots 0 and 1 of the firmware's `struct
/// syscall_stub_table` (`esp32c3/rom/libc_stubs.h`). `strlen` and
/// `memcpy` are the ROM's own exported entries ([`STRLEN`], [`MEMCPY`]),
/// which this module stubs. Returns the copy, or `NULL` if `_malloc_r`
/// returned `NULL`. Module doc, entry 25.
///
/// Calling convention (RISC-V psABI, ILP32): `ra` and `s0..s3` are saved
/// in a 32-byte frame and restored; nothing caller-saved is kept across a
/// call. Register roles: `s0` = reent, then the copy; `s1` = `s`; `s2` =
/// the syscall table; `s3` = `len + 1`.
pub const STRDUP_BODY: [u32; 32] = [
    addi(SP, SP, -32),                                            // 0  addi sp, sp, -32
    sw(RA, SP, 28),                                               // 1  sw   ra, 28(sp)
    sw(S0, SP, 24),                                               // 2  sw   s0, 24(sp)
    sw(S1, SP, 20),                                               // 3  sw   s1, 20(sp)
    sw(S2, SP, 16),                                               // 4  sw   s2, 16(sp)
    sw(S3, SP, 12),                                               // 5  sw   s3, 12(sp)
    addi(S1, A0, 0),                            // 6  mv   s1, a0               # s
    lui(S2, (SYSCALL_TABLE_PTR + 0x800) >> 12), // 7  lui s2, %hi(syscall_table_ptr)
    lw(S2, S2, ((SYSCALL_TABLE_PTR & 0xFFF) as i32) << 20 >> 20), // 8  lw s2, %lo(..)(s2)
    lw(T0, S2, 0),                              // 9  lw   t0, 0(s2)            # ->__getreent
    jalr(RA, T0, 0),                            // 10 jalr t0                   # a0 = reent
    addi(S0, A0, 0),                            // 11 mv   s0, a0
    addi(A0, S1, 0),                            // 12 mv   a0, s1
    jal(RA, sd_rel_abs(SD_STRLEN_CALL, STRLEN)), // 13 jal strlen
    addi(S3, A0, 1),                            // 14 addi s3, a0, 1            # len + 1
    addi(A0, S0, 0),                            // 15 mv   a0, s0               # reent
    addi(A1, S3, 0),                            // 16 mv   a1, s3
    lw(T0, S2, 4),                              // 17 lw   t0, 4(s2)            # ->_malloc_r
    jalr(RA, T0, 0),                            // 18 jalr t0                   # a0 = copy
    addi(S0, A0, 0),                            // 19 mv   s0, a0
    beq(A0, ZERO, rel(SD_NULL_CHECK, SD_DONE)), // 20 beqz a0, DONE
    addi(A1, S1, 0),                            // 21 mv   a1, s1
    addi(A2, S3, 0),                            // 22 mv   a2, s3
    jal(RA, sd_rel_abs(SD_MEMCPY_CALL, MEMCPY)), // 23 jal memcpy
    // DONE:
    addi(A0, S0, 0),   // 24 mv   a0, s0
    lw(RA, SP, 28),    // 25 lw   ra, 28(sp)
    lw(S0, SP, 24),    // 26 lw   s0, 24(sp)
    lw(S1, SP, 20),    // 27 lw   s1, 20(sp)
    lw(S2, SP, 16),    // 28 lw   s2, 16(sp)
    lw(S3, SP, 12),    // 29 lw   s3, 12(sp)
    addi(SP, SP, 32),  // 30 addi sp, sp, 32
    jalr(ZERO, RA, 0), // 31 ret
];

const _: () = {
    assert!(STRDUP_BODY[SD_STRLEN_CALL as usize] == jal(RA, sd_rel_abs(SD_STRLEN_CALL, STRLEN)));
    assert!(STRDUP_BODY[SD_NULL_CHECK as usize] == beq(A0, ZERO, rel(SD_NULL_CHECK, SD_DONE)));
    assert!(STRDUP_BODY[SD_MEMCPY_CALL as usize] == jal(RA, sd_rel_abs(SD_MEMCPY_CALL, MEMCPY)));
    assert!(STRDUP_BODY[SD_DONE as usize] == addi(A0, S0, 0));
    assert!(STRDUP_BODY_ADDR + 4 * STRDUP_BODY.len() as u32 <= ROM_CODE_FREE_RANGE.end);
};

/// Every guest-executed ROM code blob this module maps (see the module
/// doc's entries 14 and 25), installed by [`install_esp32c3_rom_code`].
pub const ESP32C3_ROM_CODE: &[RomCodeBlob] = &[
    RomCodeBlob {
        base: QSORT,
        words: &QSORT_SLOT,
    },
    RomCodeBlob {
        base: QSORT_BODY_ADDR,
        words: &QSORT_BODY,
    },
    RomCodeBlob {
        base: STRDUP,
        words: &STRDUP_SLOT,
    },
    RomCodeBlob {
        base: STRDUP_BODY_ADDR,
        words: &STRDUP_BODY,
    },
];

/// Maps every [`ESP32C3_ROM_CODE`] blob onto `bus` as read-only executable
/// memory ([`FirmwareBus::map_rom_code`]). The ROM-code counterpart of
/// [`esp32c3_rom_stubs`]; `crate::boot::boot_from_factory_image_with_rom_stubs`
/// installs both.
pub fn install_esp32c3_rom_code(bus: &mut FirmwareBus) {
    for blob in ESP32C3_ROM_CODE {
        bus.map_rom_code(*blob);
    }
}

/// `ets_rom_layout_p`: the ROM *data* word holding a pointer to the ROM's
/// `ets_rom_layout_t` table (`esp32c3.rom.ld`: `ets_rom_layout_p =
/// 0x3ff1fffc;`). See the module doc's entry 15.
pub const ETS_ROM_LAYOUT_P: u32 = 0x3ff1_fffc;

/// Where the ROM's `ets_rom_layout_t` table itself lives: the value of the
/// `ets_rom_layout_p` word (and the local symbol `ets_rom_layout`, 160
/// bytes) in `esp32c3_rev3_rom.elf` from `espressif/esp-rom-elfs` release
/// `20241011` (module doc, entry 15).
pub const ETS_ROM_LAYOUT: u32 = 0x3ff1_be3c;

/// `sizeof(ets_rom_layout_t)` in words: 40 `void *` fields per
/// `components/esp_rom/esp32c3/include/esp32c3/rom/rom_layout.h`
/// (`SUPPORT_BTDM` = `SUPPORT_WIFI` = 1, `SUPPORT_USB_DWCOTG` = 0), matching
/// the ELF's 160-byte `ets_rom_layout` symbol.
const ETS_ROM_LAYOUT_WORDS: usize = 40;

/// Word index of `dram0_rtos_reserved_start` in `ets_rom_layout_t` (the
/// second field, byte offset 4). The only field this firmware reads.
pub const DRAM0_RTOS_RESERVED_START_FIELD: usize = 1;

/// `ets_rom_layout_t::dram0_rtos_reserved_start` in `esp32c3_rev3_rom.elf`
/// (equal to that ELF's `_dram0_rtos_reserved_start` linker symbol): the
/// start of the DRAM the ROM keeps for its own `.data`/`.bss`, which
/// ESP-IDF reserves up to `SOC_DIRAM_DRAM_HIGH` (`0x3fce0000`).
pub const DRAM0_RTOS_RESERVED_START: u32 = 0x3fcd_f060;

/// The `ets_rom_layout_p` word: a pointer to [`ETS_ROM_LAYOUT_TABLE`].
pub const ETS_ROM_LAYOUT_P_WORD: [u32; 1] = [ETS_ROM_LAYOUT];

/// The `ets_rom_layout_t` table, full size. Only the field this firmware
/// consumes ([`DRAM0_RTOS_RESERVED_START_FIELD`]) carries its real value;
/// every other field is `0`, because nothing reads it (module doc, entry
/// 15, has the evidence). A future consumer of another field must back it
/// from the same ELF, not rely on the `0`.
pub const ETS_ROM_LAYOUT_TABLE: [u32; ETS_ROM_LAYOUT_WORDS] = {
    let mut table = [0; ETS_ROM_LAYOUT_WORDS];
    table[DRAM0_RTOS_RESERVED_START_FIELD] = DRAM0_RTOS_RESERVED_START;
    table
};

/// `esp_coex_rom_version_get`'s fixed address (`esp32c3.rom.ld`:
/// `esp_coex_rom_version_get = 0x400018ac;`). The ROM ELF shows a `j
/// 0x40045c1a` trampoline to `lui a5,0x3fcdf; lw a0,0xd0(a5); ret`: it
/// returns the ROM `.data` word `coexist_rom_version` (`0x3fcd_f0d0`,
/// section `.data_coexist_rom`), whose initializer is
/// [`COEXIST_ROM_VERSION_STR`]. Module doc, entry 21.
pub const ESP_COEX_ROM_VERSION_GET: u32 = 0x4000_18ac;

/// Where the ROM's coexistence-library version string lives: the ELF
/// `.data` initializer of `coexist_rom_version` (`4c b7 f1 3f`), inside the
/// ROM ELF's `.rodata` (`0x3ff19c00..0x3ff1eac8`). Module doc, entry 21.
pub const COEXIST_ROM_VERSION_STR: u32 = 0x3ff1_b74c;

/// The NUL-terminated string at [`COEXIST_ROM_VERSION_STR`] in the rev3
/// ROM ELF: `"9387209\0"` (`39 33 38 37 32 30 39 00`), as little-endian
/// words.
pub const COEXIST_ROM_VERSION_WORDS: [u32; 2] = [0x3738_3339, 0x0039_3032];

/// Every ROM data blob this module maps (module doc, entries 15 and 21), installed
/// by [`install_esp32c3_rom_data`].
pub const ESP32C3_ROM_DATA: &[RomDataBlob] = &[
    RomDataBlob {
        base: ETS_ROM_LAYOUT_P,
        words: &ETS_ROM_LAYOUT_P_WORD,
    },
    RomDataBlob {
        base: ETS_ROM_LAYOUT,
        words: &ETS_ROM_LAYOUT_TABLE,
    },
    RomDataBlob {
        base: COEXIST_ROM_VERSION_STR,
        words: &COEXIST_ROM_VERSION_WORDS,
    },
];

/// Maps every [`ESP32C3_ROM_DATA`] blob onto `bus` as read-only,
/// non-executable memory ([`FirmwareBus::map_rom_data`]). The ROM-data
/// counterpart of [`install_esp32c3_rom_code`];
/// `crate::boot::boot_from_factory_image_with_rom_stubs` installs both.
pub fn install_esp32c3_rom_data(bus: &mut FirmwareBus) {
    for blob in ESP32C3_ROM_DATA {
        bus.map_rom_data(*blob);
    }
}

/// `rom_spiflash_legacy_data`: the ROM's writable `.data` *pointer* to its
/// SPI-flash legacy data (`esp32c3.rom.ld`: `PROVIDE( rom_spiflash_legacy_data
/// = 0x3fcdfff0 );`; ESP-IDF's `g_rom_flashchip` is
/// `rom_spiflash_legacy_data->chip`, `components/esp_rom/include/esp_rom_spiflash.h`).
/// Module doc, entry 20.
pub const ROM_SPIFLASH_LEGACY_DATA: u32 = 0x3fcd_fff0;

/// `rom_default_spiflash_legacy_data`: the ROM's own
/// `esp_rom_spiflash_legacy_data_t` (28 bytes) that
/// [`ROM_SPIFLASH_LEGACY_DATA`] initially points at (`llvm-nm` on the ROM
/// ELF: `3fcdf5c0 0000001c ? rom_default_spiflash_legacy_data`, in section
/// `.data_spi_flash`). Module doc, entry 20.
pub const ROM_DEFAULT_SPIFLASH_LEGACY_DATA: u32 = 0x3fcd_f5c0;

/// The ROM ELF's `.data` initializer of [`ROM_DEFAULT_SPIFLASH_LEGACY_DATA`]:
/// section `.data_spi_flash` (`0x3fcdf5bc`, 0x20 bytes) bytes 4..32
/// (`llvm-objdump -s`), as `esp_rom_spiflash_legacy_data_t` fields
/// (`esp_rom_spiflash.h`): `chip = { device_id 0x1540ef, chip_size
/// 0x200000, block_size 0x10000, sector_size 0x1000, page_size 0x100,
/// status_mask 0xffff }`, then `dummy_len_plus[3] = {0,0,0}`, `sig_matrix =
/// 0`.
pub const ROM_DEFAULT_SPIFLASH_LEGACY_DATA_INIT: [u8; 28] = [
    0xef, 0x40, 0x15, 0x00, // chip.device_id   = 0x001540ef
    0x00, 0x00, 0x20, 0x00, // chip.chip_size   = 0x00200000
    0x00, 0x00, 0x01, 0x00, // chip.block_size  = 0x00010000
    0x00, 0x10, 0x00, 0x00, // chip.sector_size = 0x00001000
    0x00, 0x01, 0x00, 0x00, // chip.page_size   = 0x00000100
    0xff, 0xff, 0x00, 0x00, // chip.status_mask = 0x0000ffff
    0x00, 0x00, 0x00, 0x00, // dummy_len_plus[3], sig_matrix
];

/// The flash size the 2nd-stage bootloader assumes when the image header's
/// size nibble is not a known `esp_image_flash_size_t`
/// (`bootloader_flash_config_esp32c3.c` `update_flash_config`: `default:
/// size = 2;`, in MB).
pub const BOOTLOADER_DEFAULT_FLASH_SIZE: u32 = 2 * 0x10_0000;

/// The `device_id` the 2nd-stage bootloader stores in `g_rom_flashchip`:
/// `bootloader_read_flash_id()` (`bootloader_support/bootloader_flash/src/bootloader_flash.c`)
/// reads RDID's 3 bytes into the low 24 bits of `W0` and byte-swaps them as
/// `((id & 0xff) << 16) | ((id >> 16) & 0xff) | (id & 0xff00)`, i.e.
/// manufacturer, memory type, capacity from most to least significant. For
/// the badge's [`crate::peripherals::flash::JEDEC_ID`] `0x46 0x40 0x16` that
/// is `0x464016`.
pub const BOOTLOADER_FLASH_DEVICE_ID: u32 = {
    let id = crate::peripherals::flash::JEDEC_ID;
    ((id[0] as u32) << 16) | ((id[1] as u32) << 8) | id[2] as u32
};

/// `RTC_XTAL_FREQ_REG` = `RTC_CNTL_STORE4_REG`
/// (`components/esp_rom/esp32c3/include/esp32c3/rom/rtc.h`;
/// `DR_REG_RTCCNTL_BASE` `0x6000_8000` + `0xb8`, `soc/rtc_cntl_reg.h`).
/// Module doc, entry 23.
pub const RTC_XTAL_FREQ_REG: u32 =
    crate::mem::soc::RTC_CNTL_RANGE.start + crate::peripherals::rtc_cntl::STORE4_REG;

/// The value the 2nd-stage bootloader leaves in [`RTC_XTAL_FREQ_REG`]:
/// `clk_ll_xtal_store_freq_mhz(40)` (`hal/esp32c3/include/hal/clk_tree_ll.h`)
/// writes `(f & 0xffff) | ((f & 0xffff) << 16)`, both halves the MHz value,
/// with bit 0 of each half (`RTC_DISABLE_ROM_LOG`) set only when the ROM log
/// was already disabled, which it is not on this badge (the ROM prints its
/// banner). `clk_ll_xtal_load_freq_mhz()` accepts it as 40 MHz. 40 is the
/// only `CONFIG_XTAL_FREQ` the ESP32-C3 offers
/// (`components/esp_hw_support/port/esp32c3/Kconfig.xtal`), passed via
/// `RTC_CLK_CONFIG_DEFAULT()` (`soc/rtc.h`). Module doc, entry 23.
pub const BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE: u32 = (40 << 16) | 40;

/// The ROM writable-`.data` state the shortcut boot must seed, standing in
/// for the two steps it skips (module doc, entry 20), in application order:
///
/// 1. The mask ROM's reset-time `.data` copy, for the only ROM `.data` boot
///    is observed to consume: [`ROM_SPIFLASH_LEGACY_DATA`] (its ELF
///    initializer, section `.data.interface.spiflash_legacy`, is `c0 f5 cd
///    3f` = [`ROM_DEFAULT_SPIFLASH_LEGACY_DATA`]) and the struct it points
///    at ([`ROM_DEFAULT_SPIFLASH_LEGACY_DATA_INIT`]).
/// 2. The 2nd-stage bootloader's `update_flash_config()`
///    (`components/bootloader_support/bootloader_flash/src/bootloader_flash_config_esp32c3.c`,
///    from `bootloader_init_spi_flash()`), which calls
///    `esp_rom_spiflash_config_param(g_rom_flashchip.device_id, size *
///    0x100000, 0x10000, 0x1000, 0x100, 0xffff)`. The ROM body
///    (`4004e22e`) stores its six arguments over the six `chip` words
///    through the pointer. `device_id` was set just before by
///    `bootloader_flash_update_id()` ([`BOOTLOADER_FLASH_DEVICE_ID`]).
///    `size` is decoded from `header`'s flash-size nibble
///    ([`ImageHeader::flash_size_bytes`]), falling back to
///    [`BOOTLOADER_DEFAULT_FLASH_SIZE`] as the bootloader does. (The real
///    bootloader reads its *own* image header; esptool writes the same
///    flash size into both, and on this badge both say 4 MB.)
/// 3. The 2nd-stage bootloader's XTAL-frequency store (module doc, entry
///    23): [`RTC_XTAL_FREQ_REG`] = [`BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE`].
///    This one is a peripheral register, not RAM: the write goes through
///    the bus into the RTC_CNTL model's plain register storage
///    ([`crate::peripherals::rtc_cntl::STORE4_REG`]), which is where the
///    real bootloader's `WRITE_PERI_REG` lands too.
pub fn esp32c3_rom_ram_initializers(header: &ImageHeader) -> Vec<RamInitializer> {
    let chip_size = header
        .flash_size_bytes()
        .unwrap_or(BOOTLOADER_DEFAULT_FLASH_SIZE);
    // esp_rom_spiflash_config_param's six arguments, in `chip` field order.
    let config_param = [
        BOOTLOADER_FLASH_DEVICE_ID,
        chip_size,
        0x10000, // block_size
        0x1000,  // sector_size
        0x100,   // page_size
        0xffff,  // status_mask
    ];
    vec![
        // 1. Mask-ROM .data init.
        RamInitializer {
            addr: ROM_SPIFLASH_LEGACY_DATA,
            bytes: ROM_DEFAULT_SPIFLASH_LEGACY_DATA.to_le_bytes().to_vec(),
        },
        RamInitializer {
            addr: ROM_DEFAULT_SPIFLASH_LEGACY_DATA,
            bytes: ROM_DEFAULT_SPIFLASH_LEGACY_DATA_INIT.to_vec(),
        },
        // 2. The bootloader's esp_rom_spiflash_config_param() effect.
        RamInitializer {
            addr: ROM_DEFAULT_SPIFLASH_LEGACY_DATA,
            bytes: config_param.iter().flat_map(|w| w.to_le_bytes()).collect(),
        },
        // 3. The bootloader's clk_ll_xtal_store_freq_mhz(40) effect.
        RamInitializer {
            addr: RTC_XTAL_FREQ_REG,
            bytes: BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE.to_le_bytes().to_vec(),
        },
    ]
}

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
    (
        ETS_PRINTF,
        RomStub::printf("ets_printf", USB_SERIAL_JTAG_RANGE.start),
    ),
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
    (
        INTR_MATRIX_SET,
        RomStub::bus_register_write(
            "intr_matrix_set",
            BusRegisterWrite {
                // MAP registers start at INTERRUPT_CORE0_RANGE's offset
                // 0x000, one per source, indexed by `model_num` -- see
                // interrupt_core0_reg.h's MAC_INTR_MAP_REG..
                // CACHE_CORE0_ACS_INT_MAP_REG run and the module doc's
                // entry 11.
                base: INTERRUPT_CORE0_RANGE.start,
                index_reg: Some(REG_A1), // model_num
                index_limit: Some(MAP_SOURCE_COUNT),
                op: BusRegisterOp::Store { value_reg: REG_A2 }, // intr_num
            },
        ),
    ),
    (
        ESPRV_INTC_INT_DISABLE,
        RomStub::bus_register_write(
            "esprv_intc_int_disable",
            BusRegisterWrite {
                base: INTERRUPT_CORE0_RANGE.start + CPU_INT_ENABLE_REG,
                index_reg: None,
                index_limit: None,
                op: BusRegisterOp::UpdateMask {
                    mask_reg: REG_A0, // mask
                    set: false,
                },
            },
        ),
    ),
    (
        ESPRV_INTC_INT_ENABLE,
        RomStub::bus_register_write(
            "esprv_intc_int_enable",
            BusRegisterWrite {
                base: INTERRUPT_CORE0_RANGE.start + CPU_INT_ENABLE_REG,
                index_reg: None,
                index_limit: None,
                op: BusRegisterOp::UpdateMask {
                    mask_reg: REG_A0, // unmask
                    set: true,
                },
            },
        ),
    ),
    (
        ESPRV_INTC_INT_SET_TYPE,
        RomStub::bus_register_write(
            "esprv_intc_int_set_type",
            BusRegisterWrite {
                base: INTERRUPT_CORE0_RANGE.start + CPU_INT_TYPE_REG,
                index_reg: None,
                index_limit: None,
                op: BusRegisterOp::SetOrClearBit {
                    bit_reg: REG_A0,  // intr_num
                    cond_reg: REG_A1, // type: INTR_TYPE_LEVEL=0, INTR_TYPE_EDGE=1
                },
            },
        ),
    ),
    (
        ESPRV_INTC_INT_SET_PRIORITY,
        RomStub::bus_register_write(
            "esprv_intc_int_set_priority",
            BusRegisterWrite {
                base: INTERRUPT_CORE0_RANGE.start + CPU_INT_PRI_BASE_REG,
                index_reg: Some(REG_A0), // rv_int_num
                index_limit: Some(LINE_COUNT),
                op: BusRegisterOp::Store { value_reg: REG_A1 }, // priority
            },
        ),
    ),
    (
        ESPRV_INTC_INT_SET_THRESHOLD,
        RomStub::bus_register_write(
            "esprv_intc_int_set_threshold",
            BusRegisterWrite {
                base: INTERRUPT_CORE0_RANGE.start + CPU_INT_THRESH_REG,
                index_reg: None,
                index_limit: None,
                op: BusRegisterOp::Store { value_reg: REG_A0 }, // priority_threshold
            },
        ),
    ),
    (
        GPIO_MATRIX_IN,
        RomStub::bus_register_write(
            "gpio_matrix_in",
            BusRegisterWrite {
                base: GPIO_RANGE.start + FUNC0_IN_SEL_CFG_REG,
                index_reg: Some(REG_A1), // signal_idx
                // The ROM has no bound; 128 keeps a wild signal inside the
                // FUNCn_IN_SEL_CFG array (module doc, entry 22).
                index_limit: Some(IN_SEL_CFG_COUNT),
                op: BusRegisterOp::StoreComposed {
                    value_reg: REG_A0, // gpio
                    flags: GPIO_MATRIX_IN_FLAGS,
                },
            },
        ),
    ),
    (
        GPIO_MATRIX_OUT,
        RomStub::bus_register_writes("gpio_matrix_out", GPIO_MATRIX_OUT_WRITES),
    ),
    (
        ETS_GET_CPU_FREQUENCY,
        RomStub::returning("ets_get_cpu_frequency", CPU_FREQ_MHZ),
    ),
    (0x4000_0588, RomStub::void("ets_update_cpu_frequency")),
    (ITOA, RomStub::itoa("itoa")),
    (STRCAT, RomStub::strcat("strcat")),
    (STRLEN, RomStub::strlen("strlen")),
    (MEMCMP, RomStub::memcmp("memcmp")),
    (STRNCMP, RomStub::strncmp("strncmp")),
    (STRNCPY, RomStub::strncpy("strncpy")),
    (STRCMP, RomStub::strcmp("strcmp")),
    (STRLCAT, RomStub::strlcat("strlcat")),
    (STRLCPY, RomStub::strlcpy("strlcpy")),
    (STRTOL, RomStub::strtol("strtol")),
    (STRRCHR, RomStub::strrchr("strrchr")),
    (STRSPN, RomStub::strspn("strspn")),
    (STRCSPN, RomStub::strcspn("strcspn")),
    (STRCHR, RomStub::strchr("strchr")),
    (STRCPY, RomStub::strcpy("strcpy")),
    (MEMCHR, RomStub::memchr("memchr")),
    (MEMMOVE, RomStub::memmove("memmove")),
    (DIV, RomStub::div_t("div")),
    (MD5_INIT, RomStub::md5("MD5Init", Md5Op::Init)),
    (MD5_UPDATE, RomStub::md5("MD5Update", Md5Op::Update)),
    (MD5_FINAL, RomStub::md5("MD5Final", Md5Op::Final)),
    (
        ESP_ROM_NEWLIB_INIT_COMMON_MUTEXES,
        RomStub::store_words(
            "esp_rom_newlib_init_common_mutexes",
            NEWLIB_COMMON_MUTEX_COPIES,
        ),
    ),
    (
        ETS_APB_BACKUP_INIT_LOCK_FUNC,
        RomStub::store_words("ets_apb_backup_init_lock_func", APB_BACKUP_LOCK_FUNC_STORES),
    ),
    (
        ESP_COEX_ROM_VERSION_GET,
        // Returns the ROM .data word coexist_rom_version at its ELF
        // initializer value (module doc, entry 21).
        RomStub::returning("esp_coex_rom_version_get", COEXIST_ROM_VERSION_STR),
    ),
];

/// libgcc's 64-bit integer helpers, which this firmware also links out of ROM
/// (`esp32c3.rom.libgcc.ld`) — 32-bit RISC-V has no 64-bit divide, so any
/// `uint64_t` arithmetic (`esp_timer`'s microsecond clock, for one) becomes a
/// call to one of these. Each gets a **real** implementation; see
/// [`Int64Op`] for the register-pair convention and the divide-by-zero
/// choice.
///
/// Only the routines observed in a boot run, plus their obvious siblings
/// (signed/unsigned, the three shift directions), are listed — the float half
/// (bar [`LIBGCC_SOFT_DOUBLE_FAMILY`]) and the bit-counting half (bar
/// [`LIBGCC_INT32_FAMILY`]) of `libgcc.ld` are left to fault loudly until something
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

/// libgcc's unary 32-bit helpers (`esp32c3.rom.libgcc.ld`), real
/// implementations -- see [`Int32UnaryOp`]. `__clzsi2` is TLSF's `fls()` in
/// the heap allocator (Task D8's stall); `__ffssi2` is the next observed call (a0 = 0x200);
/// `__bswapsi2` is `spi_ll_set_command()`'s `HAL_SWAP32` (Task D12, module
/// doc entry 23). Others such as `__ctzsi2` are left to fault loudly until
/// something calls them.
const LIBGCC_INT32_FAMILY: &[(u32, &str, Int32UnaryOp)] = &[
    (0x4000_0788, "__bswapsi2", Int32UnaryOp::Bswap),
    (0x4000_079c, "__clzsi2", Int32UnaryOp::Clz),
    (0x4000_07d4, "__ffssi2", Int32UnaryOp::Ffs),
];

/// libgcc's soft-float `double` helpers (`esp32c3.rom.libgcc.ld`), real
/// implementations -- see [`SoftDoubleOp`]. Milestone 4 Task D-M4-1: once
/// I2C0 lets boot past the accelerometer probe, one firmware function
/// (`0x420e_d7f0` onward) converts a clock frequency (`a0 = 10_000_000`) to
/// `double` and scales it with these four in sequence. The other float
/// helpers are left to fault loudly until something calls them.
const LIBGCC_SOFT_DOUBLE_FAMILY: &[(u32, &str, SoftDoubleOp)] = &[
    (0x4000_07b0, "__divdf3", SoftDoubleOp::Div),
    (0x4000_07e8, "__fixunsdfsi", SoftDoubleOp::FixUnsSi),
    (0x4000_080c, "__floatunsidf", SoftDoubleOp::FloatUnsSi),
    (0x4000_0848, "__muldf3", SoftDoubleOp::Mul),
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
    for (addr, name, op) in LIBGCC_INT32_FAMILY {
        table.insert(*addr, RomStub::int32_unary(name, *op));
    }
    for (addr, name, op) in LIBGCC_SOFT_DOUBLE_FAMILY {
        table.insert(*addr, RomStub::soft_double(name, *op));
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
        for (addr, name, op) in LIBGCC_INT32_FAMILY {
            let stub = table.lookup(*addr).expect("stubbed");
            assert_eq!(stub.name, *name);
            assert_eq!(stub.effect, RomStubEffect::Int32Unary(*op), "{name}");
        }
        for (addr, name, op) in LIBGCC_SOFT_DOUBLE_FAMILY {
            let stub = table.lookup(*addr).expect("stubbed");
            assert_eq!(stub.name, *name);
            assert_eq!(stub.effect, RomStubEffect::SoftDouble(*op), "{name}");
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
        let libgcc32 = LIBGCC_INT32_FAMILY.iter().map(|(a, _, _)| *a);
        let softdf = LIBGCC_SOFT_DOUBLE_FAMILY.iter().map(|(a, _, _)| *a);
        let regi2c = REGI2C_FAMILY.iter().map(|(a, _, _)| *a);
        let mut total = 0usize;
        for addr in named
            .chain(cache)
            .chain(libgcc)
            .chain(libgcc32)
            .chain(softdf)
            .chain(regi2c)
        {
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
        // Task D9 gave strlen/memcmp/strncmp real effects, Milestone 4 Task
        // D-M4-2 strcpy (checked below).
        assert_eq!(
            table.lookup(0x4000_0378), // strstr (esp32c3.rom.libc.ld)
            None,
            "ROM libc strstr must not carry a generic stub"
        );
        for (addr, effect) in [
            (STRLEN, RomStubEffect::Strlen),
            (MEMCMP, RomStubEffect::Memcmp),
            (STRNCMP, RomStubEffect::Strncmp),
            (STRNCPY, RomStubEffect::Strncpy),
            (STRCMP, RomStubEffect::Strcmp),
            (STRLCAT, RomStubEffect::Strlcat),
            (STRLCPY, RomStubEffect::Strlcpy),
            (STRTOL, RomStubEffect::Strtol),
            (STRRCHR, RomStubEffect::Strrchr),
            (STRSPN, RomStubEffect::Strspn),
            (STRCSPN, RomStubEffect::Strcspn),
            (STRCHR, RomStubEffect::Strchr),
            (STRCPY, RomStubEffect::Strcpy),
            (MEMCHR, RomStubEffect::Memchr),
            (MEMMOVE, RomStubEffect::Memmove),
        ] {
            assert_eq!(table.lookup(addr).unwrap().effect, effect);
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

    // ---- Fix round 1 (FINDING I1): the five interrupt-controller ROM
    // calls now perform real register writes, not void no-ops. Each test
    // below runs the actual stub table through `Cpu::step` against a real
    // `FirmwareBus` (so a real `InterruptController` backs the registers),
    // sets the exact argument registers a real firmware call would carry,
    // and asserts both the resulting register state (read back through the
    // bus, at `InterruptController`'s real offsets, not re-derived) and that
    // control returns to `ra`.

    use crate::cpu::rom_stubs::{
        REG_A0, REG_A1, REG_A2, REG_A3, REG_A4, REG_A5, REG_A6, REG_A7, REG_RA, REG_SP,
    };
    use crate::cpu::Cpu;
    use crate::mem::bus::FirmwareBus;
    use crate::mem::Bus;
    use std::sync::Arc;

    /// An empty-flash `FirmwareBus` -- no XIP/RAM segments, just the real
    /// peripheral models (including `InterruptController`) every SoC
    /// address range dispatches to regardless of what's loaded. Sufficient
    /// for these tests, which only touch `INTERRUPT_CORE0` registers.
    fn empty_firmware_bus() -> FirmwareBus {
        FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[])
    }

    /// Runs one ROM stub call: installs the real ESP32-C3 table, points
    /// `pc` at `addr` with `ra` and the given argument registers set, steps
    /// once, and returns the bus for the caller to inspect.
    fn run_stub_call(addr: u32, args: &[(u8, u32)]) -> (Cpu, FirmwareBus) {
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000); // arbitrary but distinctive return address
        for (reg, val) in args {
            cpu.regs.write(*reg, *val);
        }
        cpu.regs.pc = addr;
        let mut bus = empty_firmware_bus();
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken, "a stub call must never trap");
        assert_eq!(
            info.rom_stub,
            Some(addr),
            "expected {addr:#x} to be stubbed"
        );
        assert_eq!(cpu.regs.pc, 0x4000_1000, "pc must return to ra");
        (cpu, bus)
    }

    #[test]
    fn intr_matrix_set_writes_the_indexed_map_register() {
        // intr_matrix_set(cpu_no = 0, model_num = 5, intr_num = 3): writes
        // 3 into the 6th MAP register (interrupt_core0_reg.h's MAP region
        // starts at INTERRUPT_CORE0_RANGE's offset 0x000, one word per
        // source).
        let (_cpu, mut bus) =
            run_stub_call(INTR_MATRIX_SET, &[(REG_A0, 0), (REG_A1, 5), (REG_A2, 3)]);
        assert_eq!(bus.read32(INTERRUPT_CORE0_RANGE.start + 5 * 4), 3);
        // A neighbouring MAP register must be untouched.
        assert_eq!(bus.read32(INTERRUPT_CORE0_RANGE.start + 4 * 4), 0);
    }

    #[test]
    fn intc_indexed_stubs_drop_out_of_range_indices_without_touching_neighbours() {
        // intr_matrix_set(_, model_num = 64, _): one past the last MAP slot.
        // Unbounded, this would land on 0x100 = CPU_INT_ENABLE_REG.
        let (cpu, mut bus) = run_stub_call(
            INTR_MATRIX_SET,
            &[(REG_A0, 0), (REG_A1, MAP_SOURCE_COUNT), (REG_A2, 0x1F)],
        );
        assert_eq!(cpu.rom_stub_index_drops(), 1);
        assert_eq!(
            bus.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_ENABLE_REG),
            0,
            "must not spill into CPU_INT_ENABLE_REG"
        );
        // esprv_intc_int_set_priority(32, 7): one past the last line; would
        // land on CPU_INT_THRESH_REG.
        let (cpu, mut bus2) = run_stub_call(
            ESPRV_INTC_INT_SET_PRIORITY,
            &[(REG_A0, LINE_COUNT), (REG_A1, 7)],
        );
        assert_eq!(cpu.rom_stub_index_drops(), 1);
        assert_eq!(
            bus2.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_THRESH_REG),
            0,
            "must not spill into CPU_INT_THRESH_REG"
        );
        // Wild (wrapping) values are dropped too, and the last valid slots work.
        let (cpu, _) = run_stub_call(INTR_MATRIX_SET, &[(REG_A1, 0xFFFF_FFFF), (REG_A2, 1)]);
        assert_eq!(cpu.rom_stub_index_drops(), 1);
        let (cpu, mut bus3) = run_stub_call(
            INTR_MATRIX_SET,
            &[(REG_A1, MAP_SOURCE_COUNT - 1), (REG_A2, 6)],
        );
        assert_eq!(cpu.rom_stub_index_drops(), 0);
        assert_eq!(
            bus3.read32(INTERRUPT_CORE0_RANGE.start + (MAP_SOURCE_COUNT - 1) * 4),
            6
        );
    }

    #[test]
    fn esprv_intc_int_disable_clears_only_the_masked_bits() {
        let addr = INTERRUPT_CORE0_RANGE.start + CPU_INT_ENABLE_REG;
        let mut seed = empty_firmware_bus();
        seed.write32(addr, 0b0110); // lines 1 and 2 enabled
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, 1 << 1); // mask: disable line 1 only
        cpu.regs.pc = ESPRV_INTC_INT_DISABLE;
        let info = cpu.step(&mut seed);

        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(ESPRV_INTC_INT_DISABLE));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(seed.read32(addr), 0b0100, "line 2 must stay enabled");
    }

    #[test]
    fn esprv_intc_int_enable_sets_only_the_unmasked_bits() {
        // This is the address Task D3 left unstubbed and this fix round
        // resolves -- see the module doc's entry 11.
        let addr = INTERRUPT_CORE0_RANGE.start + CPU_INT_ENABLE_REG;
        let mut seed = empty_firmware_bus();
        seed.write32(addr, 0b0100); // line 2 already enabled
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, 1 << 25); // the real boot-probe's observed unmask
        cpu.regs.pc = ESPRV_INTC_INT_ENABLE;
        let info = cpu.step(&mut seed);

        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(ESPRV_INTC_INT_ENABLE));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(
            seed.read32(addr),
            0b0100 | (1 << 25),
            "line 2 must stay enabled while line 25 becomes enabled too"
        );
    }

    #[test]
    fn esprv_intc_int_set_type_sets_the_bit_for_edge() {
        // INTR_TYPE_EDGE = 1 (riscv/interrupt.h) sets intr_num's bit.
        let (_cpu, mut bus) = run_stub_call(ESPRV_INTC_INT_SET_TYPE, &[(REG_A0, 25), (REG_A1, 1)]);
        assert_eq!(
            bus.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_TYPE_REG),
            1 << 25
        );
    }

    #[test]
    fn esprv_intc_int_set_type_clears_the_bit_for_level() {
        let addr = INTERRUPT_CORE0_RANGE.start + CPU_INT_TYPE_REG;
        let mut bus = empty_firmware_bus();
        bus.write32(addr, (1 << 25) | (1 << 3)); // line 25 edge, line 3 edge

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, 25); // intr_num
        cpu.regs.write(REG_A1, 0); // INTR_TYPE_LEVEL = 0
        cpu.regs.pc = ESPRV_INTC_INT_SET_TYPE;
        let info = cpu.step(&mut bus);

        assert!(!info.trap_taken);
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(
            bus.read32(addr),
            1 << 3,
            "line 25's bit must clear while line 3's stays set"
        );
    }

    #[test]
    fn esprv_intc_int_set_threshold_writes_cpu_int_thresh_reg() {
        // xPortStartScheduler's esprv_int_set_threshold(RVHAL_INTR_ENABLE_THRESH = 1).
        let stub = esp32c3_rom_stubs()
            .lookup(ESPRV_INTC_INT_SET_THRESHOLD)
            .expect("registered");
        assert_eq!(stub.name, "esprv_intc_int_set_threshold");
        let (cpu, mut bus) = run_stub_call(ESPRV_INTC_INT_SET_THRESHOLD, &[(REG_A0, 1)]);
        assert_eq!(
            bus.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_THRESH_REG),
            1
        );
        assert_eq!(cpu.regs.read(REG_A0), 1, "void: a0 untouched");
        // A neighbouring register (the last priority register) is untouched.
        assert_eq!(
            bus.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_PRI_BASE_REG + 31 * 4),
            0
        );
    }

    #[test]
    fn esprv_intc_int_set_priority_writes_the_indexed_priority_register() {
        let (_cpu, mut bus) =
            run_stub_call(ESPRV_INTC_INT_SET_PRIORITY, &[(REG_A0, 25), (REG_A1, 4)]);
        assert_eq!(
            bus.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_PRI_BASE_REG + 25 * 4),
            4
        );
        // A neighbouring priority register must be untouched.
        assert_eq!(
            bus.read32(INTERRUPT_CORE0_RANGE.start + CPU_INT_PRI_BASE_REG + 24 * 4),
            0
        );
    }

    // ---- Milestone 3 Task D11: ROM GPIO-matrix routing ----

    use crate::mem::soc::GPIO_RANGE;
    use crate::peripherals::gpio::{
        ENABLE_REG, FUNC0_IN_SEL_CFG_REG, FUNC0_OUT_SEL_CFG_REG, FUNC_OUT_SEL_CFG_RESET,
        IN_SEL_CFG_COUNT, OUT_SEL_CFG_COUNT,
    };

    #[test]
    fn gpio_matrix_out_routes_the_signal_and_enables_the_pad_output() {
        // The observed boot call: spicommon_bus_initialize_io's
        // esp_rom_gpio_connect_out_signal(mosi_io_num = 10, FSPID_OUT_IDX =
        // 0x41, false, false).
        let stub = esp32c3_rom_stubs()
            .lookup(GPIO_MATRIX_OUT)
            .expect("registered");
        assert_eq!(stub.name, "gpio_matrix_out");
        let (cpu, mut bus) = run_stub_call(
            GPIO_MATRIX_OUT,
            &[(REG_A0, 10), (REG_A1, 0x41), (REG_A2, 0), (REG_A3, 0)],
        );
        assert_eq!(
            bus.read32(GPIO_RANGE.start + FUNC0_OUT_SEL_CFG_REG + 10 * 4),
            0x41
        );
        assert_eq!(bus.read32(GPIO_RANGE.start + ENABLE_REG), 1 << 10);
        // Neighbouring pins keep their reset routing.
        assert_eq!(bus.gpio.func_out_sel_cfg(9), Some(FUNC_OUT_SEL_CFG_RESET));
        assert_eq!(bus.gpio.func_out_sel_cfg(11), Some(FUNC_OUT_SEL_CFG_RESET));
        assert_eq!(cpu.regs.read(REG_A0), 10, "void: a0 untouched");
    }

    #[test]
    fn gpio_matrix_out_sets_the_inversion_bits_for_nonzero_flags() {
        // out_inv -> bit 8 (GPIO_FUNC0_OUT_INV_SEL), oen_inv -> bit 10
        // (GPIO_FUNC0_OEN_INV_SEL); the ROM tests each for nonzero.
        let (_cpu, bus) = run_stub_call(
            GPIO_MATRIX_OUT,
            &[(REG_A0, 3), (REG_A1, 0x41), (REG_A2, 1), (REG_A3, 5)],
        );
        assert_eq!(bus.gpio.func_out_sel_cfg(3), Some(0x41 | 0x100 | 0x400));
        let (_cpu, bus) = run_stub_call(
            GPIO_MATRIX_OUT,
            &[(REG_A0, 3), (REG_A1, 0x41), (REG_A2, 0), (REG_A3, 1)],
        );
        assert_eq!(bus.gpio.func_out_sel_cfg(3), Some(0x41 | 0x400));
    }

    #[test]
    fn gpio_matrix_out_ignores_a_gpio_past_the_last_pad_like_the_rom() {
        // The ROM body's `li a5,25; bltu a5,a0,ret` guard: gpio 26 writes
        // nothing at all -- no OUT_SEL_CFG word, no ENABLE bit.
        let (cpu, mut bus) = run_stub_call(
            GPIO_MATRIX_OUT,
            &[(REG_A0, OUT_SEL_CFG_COUNT), (REG_A1, 0x41)],
        );
        assert_eq!(bus.read32(GPIO_RANGE.start + ENABLE_REG), 0);
        assert_eq!(
            bus.read32(GPIO_RANGE.start + FUNC0_OUT_SEL_CFG_REG + OUT_SEL_CFG_COUNT * 4),
            0
        );
        assert_eq!(cpu.rom_stub_index_drops(), 1);
        // The last real pad still works.
        let (cpu, bus) = run_stub_call(
            GPIO_MATRIX_OUT,
            &[(REG_A0, OUT_SEL_CFG_COUNT - 1), (REG_A1, 0x41)],
        );
        assert_eq!(bus.gpio.func_out_sel_cfg(OUT_SEL_CFG_COUNT - 1), Some(0x41));
        assert_eq!(cpu.rom_stub_index_drops(), 0);
    }

    #[test]
    fn gpio_matrix_in_routes_the_pad_to_the_peripheral_input_through_the_matrix() {
        // The observed boot call: spicommon_bus_initialize_io's
        // esp_rom_gpio_connect_in_signal(mosi_io_num = 10, FSPID_IN_IDX =
        // 0x41, false): GPIO_FUNC65_IN_SEL_CFG_REG = 10 | SIG_IN_SEL (0x40).
        let stub = esp32c3_rom_stubs()
            .lookup(GPIO_MATRIX_IN)
            .expect("registered");
        assert_eq!(stub.name, "gpio_matrix_in");
        let (cpu, mut bus) =
            run_stub_call(GPIO_MATRIX_IN, &[(REG_A0, 10), (REG_A1, 0x41), (REG_A2, 0)]);
        assert_eq!(
            bus.read32(GPIO_RANGE.start + FUNC0_IN_SEL_CFG_REG + 0x41 * 4),
            0x40 | 10
        );
        assert_eq!(bus.gpio.func_in_sel_cfg(0x40), Some(0));
        assert_eq!(bus.gpio.func_in_sel_cfg(0x42), Some(0));
        // No output-enable side effect, unlike gpio_matrix_out.
        assert_eq!(bus.read32(GPIO_RANGE.start + ENABLE_REG), 0);
        assert_eq!(cpu.regs.read(REG_A0), 10, "void: a0 untouched");
    }

    #[test]
    fn gpio_matrix_in_sets_inv_for_a_nonzero_flag_and_skips_sig_in_sel_for_0x3a() {
        // inv -> GPIO_FUNC0_IN_INV_SEL (bit 5), tested with beqz.
        let (_cpu, bus) = run_stub_call(GPIO_MATRIX_IN, &[(REG_A0, 7), (REG_A1, 3), (REG_A2, 2)]);
        assert_eq!(bus.gpio.func_in_sel_cfg(3), Some(0x40 | 0x20 | 7));
        // The ROM body leaves SIG_IN_SEL (bit 6) clear only for gpio ==
        // 0x3a, stored as given (the ROM does not mask gpio to IN_SEL's 5
        // bits).
        let (_cpu, bus) =
            run_stub_call(GPIO_MATRIX_IN, &[(REG_A0, 0x3a), (REG_A1, 3), (REG_A2, 0)]);
        assert_eq!(bus.gpio.func_in_sel_cfg(3), Some(0x3a));
    }

    #[test]
    fn gpio_matrix_in_drops_a_signal_past_the_last_in_sel_cfg_register() {
        // The ROM has no bound on signal_idx; signal 128 would land on
        // GPIO_FUNC0_OUT_SEL_CFG_REG's neighbourhood (0x354). Dropped and
        // counted instead, like the intc stubs' wild indices.
        let (cpu, mut bus) = run_stub_call(
            GPIO_MATRIX_IN,
            &[(REG_A0, 10), (REG_A1, IN_SEL_CFG_COUNT), (REG_A2, 0)],
        );
        assert_eq!(cpu.rom_stub_index_drops(), 1);
        assert_eq!(
            bus.read32(GPIO_RANGE.start + FUNC0_IN_SEL_CFG_REG + IN_SEL_CFG_COUNT * 4),
            0
        );
    }

    #[test]
    fn itoa_stub_writes_the_hex_string_through_a_real_firmware_bus() {
        // The exact observed boot-probe call (Task D4 brief):
        // itoa(value = 0x42001011, str = 0x3fcdc694, base = 0x10). RAM
        // addresses (0x3fcd_xxxx is DRAM) are real, dispatchable
        // `FirmwareBus` regions, unlike `run_stub_call`'s helper's empty-flash
        // bus used for the INTERRUPT_CORE0-only tests above -- so this test
        // seeds a `FirmwareBus` with a DRAM segment covering the target
        // string buffer.
        use crate::mem::soc::DRAM_RANGE;
        let str_addr = 0x3fcd_c694u32;
        assert!(
            DRAM_RANGE.contains(&str_addr),
            "the observed str pointer must fall inside DRAM"
        );
        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        // The plain no-segments bus (see `empty_firmware_bus` above) leaves
        // DRAM unmapped, and this bus's catch-all silently drops writes to
        // unmapped data addresses -- real, so this test needs a real
        // writable region at the target buffer to observe anything.
        bus.add_scratch_ram(str_addr, 16);

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, 0x4200_1011); // value
        cpu.regs.write(REG_A1, str_addr); // str
        cpu.regs.write(REG_A2, 0x10); // base
        cpu.regs.pc = ITOA;
        let info = cpu.step(&mut bus);

        assert!(!info.trap_taken, "a stub call must never trap");
        assert_eq!(info.rom_stub, Some(ITOA));
        assert_eq!(cpu.regs.pc, 0x4000_1000, "pc must return to ra");
        assert_eq!(cpu.regs.read(REG_A0), str_addr, "itoa returns str in a0");

        let mut written = Vec::new();
        for i in 0..9u32 {
            written.push(bus.read8(str_addr + i));
        }
        assert_eq!(&written, b"42001011\0");
    }

    #[test]
    fn strcat_stub_appends_through_a_real_firmware_bus() {
        // Same call shape as the real boot-probe evidence (Task D4 report):
        // strcat(dst = 0x3fcdc65c-ish DRAM buffer, src = a literal string in
        // DRAM) -- both real, dispatchable `FirmwareBus` addresses seeded
        // with scratch RAM the same way the itoa test above does.
        use crate::mem::soc::DRAM_RANGE;
        let dst = 0x3fcd_c65cu32;
        let src = 0x3fca_7614u32;
        assert!(DRAM_RANGE.contains(&dst) && DRAM_RANGE.contains(&src));

        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.add_scratch_ram(dst, 32);
        bus.add_scratch_ram(src, 32);
        for (i, b) in b"Hello\0".iter().enumerate() {
            bus.write8(dst + i as u32, *b);
        }
        for (i, b) in b"World\0".iter().enumerate() {
            bus.write8(src + i as u32, *b);
        }

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, dst);
        cpu.regs.write(REG_A1, src);
        cpu.regs.pc = STRCAT;
        let info = cpu.step(&mut bus);

        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(STRCAT));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(cpu.regs.read(REG_A0), dst, "strcat returns dst");

        let mut written = Vec::new();
        for i in 0..11u32 {
            written.push(bus.read8(dst + i));
        }
        assert_eq!(&written, b"HelloWorld\0");
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

    #[test]
    fn newlib_init_common_mutexes_copies_the_pointed_at_words_into_rom_statics() {
        // esp_rom_newlib_init_common_mutexes(a0 = &recursive, a1 = &plain)
        // must store *a0 at 0x3fcdf660 and *a1 at 0x3fcdf65c -- the pointed-at
        // words, not the pointers -- and leave a0 alone (void function).
        let table = esp32c3_rom_stubs();
        let stub = table
            .lookup(ESP_ROM_NEWLIB_INIT_COMMON_MUTEXES)
            .expect("registered");
        assert_eq!(stub.name, "esp_rom_newlib_init_common_mutexes");

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        // boot.rs backs the whole DRAM aperture (which includes the
        // ROM-reserved 0x3fcdf060.. window) with scratch RAM; mirror the
        // relevant parts here.
        bus.add_scratch_ram(0x3fc9_0100, 16);
        bus.add_scratch_ram(0x3fcd_f650, 0x20);
        bus.write32(0x3fc9_0100, 0xdead_beef);
        bus.write32(0x3fc9_0104, 0x1234_5678);
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, 0x3fc9_0100);
        cpu.regs.write(REG_A1, 0x3fc9_0104);
        cpu.regs.pc = ESP_ROM_NEWLIB_INIT_COMMON_MUTEXES;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(ESP_ROM_NEWLIB_INIT_COMMON_MUTEXES));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(cpu.regs.read(REG_A0), 0x3fc9_0100, "void: a0 untouched");
        assert_eq!(bus.read32(0x3fcd_f660), 0xdead_beef);
        assert_eq!(bus.read32(0x3fcd_f65c), 0x1234_5678);
    }

    fn header_with_speed_size(spi_speed_size: u8) -> ImageHeader {
        let mut bytes = vec![0u8; crate::mem::image::IMAGE_HEADER_LEN];
        bytes[0] = crate::mem::image::IMAGE_MAGIC;
        bytes[3] = spi_speed_size;
        crate::mem::image::parse_image(&bytes)
            .expect("zero-segment header parses")
            .header
    }

    /// Applies `inits` to a flat little-endian word map (address -> word),
    /// in order, so the test checks the *net* effect, not the list shape.
    fn net_words(inits: &[RamInitializer]) -> std::collections::BTreeMap<u32, u32> {
        let mut bytes = std::collections::BTreeMap::new();
        for init in inits {
            for (i, b) in init.bytes.iter().enumerate() {
                bytes.insert(init.addr + i as u32, *b);
            }
        }
        let mut words = std::collections::BTreeMap::new();
        for (&addr, _) in bytes.iter().filter(|(a, _)| *a % 4 == 0) {
            let w = (0..4)
                .map(|i| u32::from(bytes[&(addr + i)]) << (8 * i))
                .sum();
            words.insert(addr, w);
        }
        words
    }

    #[test]
    fn rom_ram_initializers_seed_the_spiflash_legacy_data_as_rom_then_bootloader_leave_it() {
        // factory.bin's header byte 3 is 0x2f (ESP_IMAGE_FLASH_SIZE_4MB).
        let words = net_words(&esp32c3_rom_ram_initializers(&header_with_speed_size(0x2f)));
        let expected: std::collections::BTreeMap<u32, u32> = [
            // rom_spiflash_legacy_data -> rom_default_spiflash_legacy_data
            (0x3fcd_fff0, 0x3fcd_f5c0),
            // chip, as esp_rom_spiflash_config_param(0x464016, 4 MiB, ...) left it
            (0x3fcd_f5c0, 0x0046_4016), // device_id (bootloader RDID)
            (0x3fcd_f5c4, 0x0040_0000), // chip_size (header: 4 MB)
            (0x3fcd_f5c8, 0x0001_0000), // block_size
            (0x3fcd_f5cc, 0x0000_1000), // sector_size
            (0x3fcd_f5d0, 0x0000_0100), // page_size
            (0x3fcd_f5d4, 0x0000_ffff), // status_mask
            (0x3fcd_f5d8, 0),           // dummy_len_plus[3], sig_matrix (ROM .data)
            // RTC_XTAL_FREQ_REG, as clk_ll_xtal_store_freq_mhz(40) left it (Task D12)
            (0x6000_80b8, 0x0028_0028),
        ]
        .into_iter()
        .collect();
        assert_eq!(words, expected);
    }

    #[test]
    fn rom_ram_initializers_take_chip_size_from_the_header_with_the_bootloaders_fallback() {
        let size_of = |b: u8| {
            net_words(&esp32c3_rom_ram_initializers(&header_with_speed_size(b)))
                [&(ROM_DEFAULT_SPIFLASH_LEGACY_DATA + 4)]
        };
        assert_eq!(size_of(0x1f), 0x0020_0000); // 2 MB
        assert_eq!(size_of(0x3f), 0x0080_0000); // 8 MB
                                                // Not an esp_image_flash_size_t: update_flash_config's `default:
                                                // size = 2;`.
        assert_eq!(size_of(0x8f), BOOTLOADER_DEFAULT_FLASH_SIZE);
    }

    #[test]
    fn rom_ram_initializers_seed_only_the_rom_data_boot_consumes() {
        // Guard against bulk-copying ROM .data: every byte seeded lies in
        // the legacy-data pointer word, the 28-byte struct, or (Task D12)
        // the one RTC_XTAL_FREQ_REG word.
        for init in esp32c3_rom_ram_initializers(&header_with_speed_size(0x2f)) {
            let end = init.addr + init.bytes.len() as u32;
            let in_ptr =
                init.addr >= ROM_SPIFLASH_LEGACY_DATA && end <= ROM_SPIFLASH_LEGACY_DATA + 4;
            let in_struct = init.addr >= ROM_DEFAULT_SPIFLASH_LEGACY_DATA
                && end <= ROM_DEFAULT_SPIFLASH_LEGACY_DATA + 28;
            let in_xtal = init.addr == RTC_XTAL_FREQ_REG && init.bytes.len() == 4;
            assert!(
                in_ptr || in_struct || in_xtal,
                "unexpected seed at 0x{:08x}",
                init.addr
            );
        }
    }

    #[test]
    fn bswapsi2_is_a_real_int32_unary_stub_at_its_libgcc_ld_address() {
        // esp32c3.rom.libgcc.ld: `__bswapsi2 = 0x40000788;`.
        let table = esp32c3_rom_stubs();
        let stub = table.lookup(0x4000_0788).expect("__bswapsi2 is stubbed");
        assert_eq!(stub.name, "__bswapsi2");
        assert_eq!(stub.effect, RomStubEffect::Int32Unary(Int32UnaryOp::Bswap));
    }

    /// `clk_ll_xtal_load_freq_mhz()` (`hal/esp32c3/include/hal/clk_tree_ll.h`),
    /// transcribed: both 16-bit halves equal, not 0 and not all-ones, then
    /// the low half minus `RTC_DISABLE_ROM_LOG`; otherwise 0 ("invalid").
    fn clk_ll_xtal_load_freq_mhz(reg: u32) -> u32 {
        const RTC_DISABLE_ROM_LOG: u32 = (1 << 0) | (1 << 16);
        if (reg & 0xffff) == ((reg >> 16) & 0xffff) && reg != 0 && reg != u32::MAX {
            reg & !RTC_DISABLE_ROM_LOG & 0xffff
        } else {
            0
        }
    }

    #[test]
    fn rom_ram_initializers_seed_rtc_xtal_freq_reg_as_the_bootloader_stores_40mhz() {
        assert_eq!(RTC_XTAL_FREQ_REG, 0x6000_80b8, "RTC_CNTL_STORE4_REG");
        let words = net_words(&esp32c3_rom_ram_initializers(&header_with_speed_size(0x2f)));
        let reg = *words
            .get(&RTC_XTAL_FREQ_REG)
            .expect("RTC_XTAL_FREQ_REG is seeded");
        // clk_ll_xtal_store_freq_mhz(40) with the ROM log enabled.
        assert_eq!(reg, 0x0028_0028);
        assert_eq!(clk_ll_xtal_load_freq_mhz(reg), 40, "a valid 40 MHz");
        // The unseeded reset value is what clk_hal calls invalid.
        assert_eq!(clk_ll_xtal_load_freq_mhz(0), 0);
    }

    #[test]
    fn bootloader_device_id_is_the_byte_swapped_jedec_id() {
        // bootloader_read_flash_id() on W0 = 0x00164046 (Task 8's RDID).
        let w0: u32 = 0x0016_4046;
        let id = ((w0 & 0xff) << 16) | ((w0 >> 16) & 0xff) | (w0 & 0xff00);
        assert_eq!(BOOTLOADER_FLASH_DEVICE_ID, id);
        assert_eq!(BOOTLOADER_FLASH_DEVICE_ID, 0x0046_4016);
    }

    #[test]
    fn apb_backup_init_lock_func_stores_the_register_values_into_rom_statics() {
        // ets_apb_backup_init_lock_func(a0 = lock_fn, a1 = unlock_fn) must
        // store the pointers *themselves* (not the words they point at) at
        // 0x3fcdf654 / 0x3fcdf658, and leave a0 alone (void function).
        let table = esp32c3_rom_stubs();
        let stub = table
            .lookup(ETS_APB_BACKUP_INIT_LOCK_FUNC)
            .expect("registered");
        assert_eq!(stub.name, "ets_apb_backup_init_lock_func");

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(0x3fcd_f650, 0x20);
        // Function pointers in IRAM, as the real caller passes (a0 =
        // 0x40380512, a1 = 0x403804f8 at the observed call). Back IRAM with
        // a distinct word at each so a pointee-load would be caught.
        bus.add_scratch_ram(0x4038_04f8, 0x20);
        bus.write32(0x4038_04f8, 0x1111_1111);
        bus.write32(0x4038_0510, 0x2222_2222);
        cpu.regs.write(REG_RA, 0x4200_155c);
        cpu.regs.write(REG_A0, 0x4038_0512);
        cpu.regs.write(REG_A1, 0x4038_04f8);
        cpu.regs.pc = ETS_APB_BACKUP_INIT_LOCK_FUNC;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(ETS_APB_BACKUP_INIT_LOCK_FUNC));
        assert_eq!(cpu.regs.pc, 0x4200_155c);
        assert_eq!(cpu.regs.read(REG_A0), 0x4038_0512, "void: a0 untouched");
        assert_eq!(bus.read32(ROM_APB_BACKUP_LOCK), 0x4038_0512);
        assert_eq!(bus.read32(ROM_APB_BACKUP_UNLOCK), 0x4038_04f8);
    }

    #[test]
    fn strlen_stub_returns_the_length_of_a_nul_terminated_string() {
        let str_addr = 0x3fc9_0200;
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(str_addr, 16);
        for (i, b) in b"hello\0zz".iter().enumerate() {
            bus.write8(str_addr + i as u32, *b);
        }
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, str_addr);
        cpu.regs.pc = STRLEN;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(STRLEN));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(cpu.regs.read(REG_A0), 5);
        // Empty string.
        cpu.regs.write(REG_A0, str_addr + 5);
        cpu.regs.pc = STRLEN;
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.read(REG_A0), 0);
    }

    #[test]
    fn memchr_stub_returns_a_pointer_to_the_first_match_or_null() {
        let buf = 0x3fc9_0280;
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(buf, 16);
        for (i, b) in b"ab\ncd\n\xff".iter().enumerate() {
            bus.write8(buf + i as u32, *b);
        }
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, c: u32, n: u32| {
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, buf);
            cpu.regs.write(REG_A1, c);
            cpu.regs.write(REG_A2, n);
            cpu.regs.pc = MEMCHR;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(MEMCHR));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            cpu.regs.read(REG_A0)
        };
        assert_eq!(
            call(&mut cpu, &mut bus, b'\n' as u32, 7),
            buf + 2,
            "first match"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, b'\n' as u32, 2),
            0,
            "outside n: NULL"
        );
        assert_eq!(call(&mut cpu, &mut bus, b'z' as u32, 7), 0, "absent: NULL");
        assert_eq!(call(&mut cpu, &mut bus, b'a' as u32, 0), 0, "n = 0: NULL");
        // c is converted to unsigned char (the ROM's `zext.b a1, a1`).
        assert_eq!(call(&mut cpu, &mut bus, 0xFFFF_FFFF, 7), buf + 6);
        assert_eq!(call(&mut cpu, &mut bus, 0x100 | b'c' as u32, 7), buf + 3);
    }

    #[test]
    fn memmove_stub_copies_overlapping_ranges_correctly_and_returns_dst() {
        let buf = 0x3fc9_02c0;
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(buf, 16);
        let fill = |bus: &mut FirmwareBus| {
            for (i, b) in b"abcdefgh".iter().enumerate() {
                bus.write8(buf + i as u32, *b);
            }
        };
        let read =
            |bus: &mut FirmwareBus| -> Vec<u8> { (0..8).map(|i| bus.read8(buf + i)).collect() };
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, dst: u32, src: u32, n: u32| {
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, dst);
            cpu.regs.write(REG_A1, src);
            cpu.regs.write(REG_A2, n);
            cpu.regs.pc = MEMMOVE;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(MEMMOVE));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            assert_eq!(cpu.regs.read(REG_A0), dst, "memmove returns dst");
        };
        // dst above src, overlapping: a naive forward copy would smear "a".
        fill(&mut bus);
        call(&mut cpu, &mut bus, buf + 2, buf, 5);
        assert_eq!(read(&mut bus), b"ababcdeh");
        // dst below src, overlapping.
        fill(&mut bus);
        call(&mut cpu, &mut bus, buf, buf + 2, 5);
        assert_eq!(read(&mut bus), b"cdefgfgh");
        // n = 0 copies nothing.
        fill(&mut bus);
        call(&mut cpu, &mut bus, buf, buf + 4, 0);
        assert_eq!(read(&mut bus), b"abcdefgh");
    }

    /// Guest addresses for the ROM MD5 tests: an 88-byte `md5_context_t`, a
    /// message buffer and a 16-byte digest buffer, all in DRAM scratch.
    const MD5_CTX: u32 = 0x3fc9_1000;
    const MD5_MSG: u32 = 0x3fc9_1100;
    const MD5_DIGEST: u32 = 0x3fc9_1300;
    const MD5_RA: u32 = 0x4200_2000;

    fn md5_bus() -> FirmwareBus {
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(MD5_CTX, 0x400);
        bus
    }

    /// One stubbed ROM call: `pc = addr` with the given argument registers.
    /// Checks it ran as a stub, returned to `ra`, and left `a0` alone (all
    /// three ROM MD5 functions are `void`).
    fn md5_call(cpu: &mut Cpu, bus: &mut FirmwareBus, addr: u32, args: &[u32]) {
        for (i, v) in args.iter().enumerate() {
            cpu.regs.write(REG_A0 + i as u8, *v);
        }
        cpu.regs.write(REG_RA, MD5_RA);
        cpu.regs.pc = addr;
        let info = cpu.step(bus);
        assert!(!info.trap_taken, "0x{addr:08x} trapped");
        assert_eq!(info.rom_stub, Some(addr));
        assert_eq!(cpu.regs.pc, MD5_RA);
        assert_eq!(cpu.regs.read(REG_A0), args[0], "void: a0 untouched");
    }

    /// `MD5Init(ctx)`, one `MD5Update(ctx, msg + at, n)` per chunk, then
    /// `MD5Final(digest, ctx)`, all through the stubs; returns the digest.
    fn md5_via_rom(cpu: &mut Cpu, bus: &mut FirmwareBus, msg: &[u8], chunks: &[usize]) -> String {
        for (i, b) in msg.iter().enumerate() {
            bus.write8(MD5_MSG + i as u32, *b);
        }
        md5_call(cpu, bus, MD5_INIT, &[MD5_CTX]);
        let mut at = 0u32;
        for n in chunks {
            md5_call(cpu, bus, MD5_UPDATE, &[MD5_CTX, MD5_MSG + at, *n as u32]);
            at += *n as u32;
        }
        assert_eq!(at as usize, msg.len(), "chunks cover the message");
        md5_call(cpu, bus, MD5_FINAL, &[MD5_DIGEST, MD5_CTX]);
        (0..16)
            .map(|i| format!("{:02x}", bus.read8(MD5_DIGEST + i)))
            .collect()
    }

    #[test]
    fn rom_md5_stubs_are_registered_at_the_linker_script_addresses() {
        let table = esp32c3_rom_stubs();
        for (addr, name, op) in [
            (0x4000_0614, "MD5Init", Md5Op::Init),
            (0x4000_0618, "MD5Update", Md5Op::Update),
            (0x4000_061c, "MD5Final", Md5Op::Final),
        ] {
            let stub = table.lookup(addr).expect("registered");
            assert_eq!(stub.name, name);
            assert_eq!(stub.effect, RomStubEffect::Md5(op));
        }
        assert_eq!(
            (MD5_INIT, MD5_UPDATE, MD5_FINAL),
            (0x4000_0614, 0x4000_0618, 0x4000_061c)
        );
    }

    #[test]
    fn rom_md5_stubs_give_the_rfc_1321_digests() {
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = md5_bus();
        for (msg, want) in [
            (&b""[..], "d41d8cd98f00b204e9800998ecf8427e"),
            (b"a", "0cc175b9c0f1b6a831c399e269772661"),
            (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
            (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            let got = md5_via_rom(&mut cpu, &mut bus, msg, &[msg.len()]);
            assert_eq!(got, want, "{:?}", String::from_utf8_lossy(msg));
        }
    }

    #[test]
    fn rom_md5_update_split_at_odd_sizes_keeps_the_partial_block_in_guest_memory() {
        // 1 + 63 + 65 + 7 = 136 bytes: a 1-byte partial block, topped up to
        // exactly one block, then a block plus one byte, then 7 more. Only
        // the guest context carries the buffered bytes between calls.
        let msg: Vec<u8> = (0..136u32).map(|i| (i * 31 + 7) as u8).collect();
        let want: String = crate::md5::md5(&msg)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        for chunks in [
            &[1usize, 63, 65, 7][..],
            &[136],
            &[0, 55, 1, 8, 72],
            &[64, 64, 8],
        ] {
            let mut cpu = Cpu::new();
            cpu.set_rom_stubs(esp32c3_rom_stubs());
            let mut bus = md5_bus();
            assert_eq!(
                md5_via_rom(&mut cpu, &mut bus, &msg, chunks),
                want,
                "{chunks:?}"
            );
        }
        // Mid-stream, the guest context holds the bit count and the tail:
        // after 1 + 63 + 65 bytes, bits = 129 * 8 and in[0] = msg[128].
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = md5_bus();
        for (i, b) in msg.iter().enumerate() {
            bus.write8(MD5_MSG + i as u32, *b);
        }
        md5_call(&mut cpu, &mut bus, MD5_INIT, &[MD5_CTX]);
        let mut at = 0;
        for n in [1u32, 63, 65] {
            md5_call(&mut cpu, &mut bus, MD5_UPDATE, &[MD5_CTX, MD5_MSG + at, n]);
            at += n;
        }
        assert_eq!(bus.read32(MD5_CTX + 16), 129 * 8, "bits[0]");
        assert_eq!(bus.read32(MD5_CTX + 20), 0, "bits[1]");
        assert_eq!(bus.read8(MD5_CTX + 24), msg[128], "in[0]");
    }

    #[test]
    fn rom_md5_init_leaves_in_alone_and_final_zeroes_the_whole_context() {
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = md5_bus();
        for i in 0..0x60 {
            bus.write8(MD5_CTX + i, 0xA5);
        }
        md5_call(&mut cpu, &mut bus, MD5_INIT, &[MD5_CTX]);
        assert_eq!(bus.read32(MD5_CTX), 0x6745_2301);
        assert_eq!(bus.read32(MD5_CTX + 12), 0x1032_5476);
        assert_eq!(bus.read32(MD5_CTX + 16), 0, "bits[0]");
        assert_eq!(bus.read32(MD5_CTX + 20), 0, "bits[1]");
        assert!(
            (24..88).all(|i| bus.read8(MD5_CTX + i) == 0xA5),
            "in[] untouched"
        );

        bus.write8(MD5_MSG, b'a');
        md5_call(&mut cpu, &mut bus, MD5_UPDATE, &[MD5_CTX, MD5_MSG, 1]);
        md5_call(&mut cpu, &mut bus, MD5_FINAL, &[MD5_DIGEST, MD5_CTX]);
        assert!(
            (0..88).all(|i| bus.read8(MD5_CTX + i) == 0),
            "all 88 bytes of the context zeroed"
        );
        assert_eq!(bus.read8(MD5_CTX + 88), 0xA5, "nothing past the context");
        assert_eq!(bus.read8(MD5_DIGEST), 0x0c, "digest written to a0");
    }

    /// The check `load_partitions()` makes (`components/esp_partition/
    /// partition.c:107-250`), driven through the stubs over the emulator's
    /// own synthesized partition table: `esp_rom_md5_init`, one
    /// `esp_rom_md5_update(&context, &entry, 32)` per `0x50AA` entry until
    /// the `0xEBEB` MD5 entry, then `esp_rom_md5_final` and a compare with
    /// the 16 bytes stored `ESP_PARTITION_MD5_OFFSET` into that entry. This
    /// is the unit test of the stubs; the end-to-end proof is boot_progress.rs's
    /// `load_partitions_accepts_the_synthesized_table_through_the_flash_mmu`.
    #[test]
    fn rom_md5_stubs_accept_the_synthesized_partition_tables_md5_entry() {
        use crate::peripherals::flash::{EmulatedFlash, PARTITION_TABLE_OFFSET};
        let flash = EmulatedFlash::from_app_image(&[]);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = md5_bus();
        md5_call(&mut cpu, &mut bus, MD5_INIT, &[MD5_CTX]);
        let mut entry_off = PARTITION_TABLE_OFFSET;
        let mut entries = 0;
        let stored = loop {
            let magic = u16::from_le_bytes([flash.read(entry_off), flash.read(entry_off + 1)]);
            if magic == 0xEBEB {
                break (0..16)
                    .map(|k| flash.read(entry_off + 16 + k))
                    .collect::<Vec<u8>>();
            }
            assert_eq!(magic, 0x50AA, "entry at 0x{entry_off:x}");
            // memcpy(&entry, p_entry, 32) into a stack copy, then update.
            for k in 0..32 {
                bus.write8(MD5_MSG + k, flash.read(entry_off + k));
            }
            md5_call(&mut cpu, &mut bus, MD5_UPDATE, &[MD5_CTX, MD5_MSG, 32]);
            entries += 1;
            entry_off += 32;
        };
        assert_eq!(entries, 4, "nvs, phy_init, factory, storage");
        md5_call(&mut cpu, &mut bus, MD5_FINAL, &[MD5_DIGEST, MD5_CTX]);
        let calc: Vec<u8> = (0..16).map(|k| bus.read8(MD5_DIGEST + k)).collect();
        assert_eq!(calc, stored, "load_partitions() would accept the table");
    }

    #[test]
    fn memcmp_stub_returns_the_first_differing_byte_difference() {
        let (a, b) = (0x3fc9_0300, 0x3fc9_0320);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(a, 16);
        bus.add_scratch_ram(b, 16);
        for (i, x) in [1u8, 2, 0x90, 4].iter().enumerate() {
            bus.write8(a + i as u32, *x);
        }
        for (i, x) in [1u8, 2, 0x10, 9].iter().enumerate() {
            bus.write8(b + i as u32, *x);
        }
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, n: u32| {
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, a);
            cpu.regs.write(REG_A1, b);
            cpu.regs.write(REG_A2, n);
            cpu.regs.pc = MEMCMP;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(MEMCMP));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            cpu.regs.read(REG_A0)
        };
        // Bytes are compared as unsigned char: 0x90 - 0x10 = +0x80.
        assert_eq!(call(&mut cpu, &mut bus, 4), 0x80);
        assert_eq!(call(&mut cpu, &mut bus, 2), 0, "equal prefix");
        assert_eq!(call(&mut cpu, &mut bus, 0), 0, "n = 0");
        // Swap operands' roles: negative difference, as a wrapped i32.
        bus.write8(a + 2, 0x10);
        bus.write8(b + 2, 0x90);
        assert_eq!(call(&mut cpu, &mut bus, 4) as i32, -0x80);
    }

    #[test]
    fn strncmp_stub_stops_at_nul_or_n_and_returns_the_byte_difference() {
        let (a, b) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(a, 16);
        bus.add_scratch_ram(b, 16);
        for (i, x) in b"abcX\0".iter().enumerate() {
            bus.write8(a + i as u32, *x);
        }
        for (i, x) in b"abcY\0".iter().enumerate() {
            bus.write8(b + i as u32, *x);
        }
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, n: u32| {
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, a);
            cpu.regs.write(REG_A1, b);
            cpu.regs.write(REG_A2, n);
            cpu.regs.pc = STRNCMP;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(STRNCMP));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            cpu.regs.read(REG_A0) as i32
        };
        assert_eq!(call(&mut cpu, &mut bus, 0), 0, "n = 0");
        assert_eq!(call(&mut cpu, &mut bus, 3), 0, "equal within n");
        assert_eq!(call(&mut cpu, &mut bus, 4), -1, "'X' - 'Y'");
        // Equal strings stop at the shared NUL even if n is larger.
        bus.write8(a + 3, 0);
        bus.write8(b + 3, 0);
        bus.write8(a + 4, b'p');
        bus.write8(b + 4, b'q');
        assert_eq!(call(&mut cpu, &mut bus, 16), 0, "stops at NUL");
    }

    #[test]
    fn strncpy_stub_copies_to_nul_pads_and_respects_n() {
        let (src, dst) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(src, 32);
        bus.add_scratch_ram(dst, 32);
        for (i, x) in b"abc\0zzz".iter().enumerate() {
            bus.write8(src + i as u32, *x);
        }
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, n: u32| {
            for i in 0..16 {
                bus.write8(dst + i, 0x55);
            }
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, dst);
            cpu.regs.write(REG_A1, src);
            cpu.regs.write(REG_A2, n);
            cpu.regs.pc = STRNCPY;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(STRNCPY));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            assert_eq!(cpu.regs.read(REG_A0), dst, "returns dst");
            (0..8).map(|i| bus.read8(dst + i)).collect::<Vec<u8>>()
        };
        assert_eq!(
            call(&mut cpu, &mut bus, 0),
            [0x55; 8],
            "n = 0 writes nothing"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, 2),
            [b'a', b'b', 0x55, 0x55, 0x55, 0x55, 0x55, 0x55],
            "n < strlen: no terminator"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, 3),
            [b'a', b'b', b'c', 0x55, 0x55, 0x55, 0x55, 0x55],
            "n == strlen: no terminator"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, 6),
            [b'a', b'b', b'c', 0, 0, 0, 0x55, 0x55],
            "NUL copied, then padded with NULs up to n only"
        );
    }

    #[test]
    fn strlcat_stub_appends_within_siz_terminates_and_returns_the_tried_length() {
        let (src, dst) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(src, 32);
        bus.add_scratch_ram(dst, 32);
        for (i, x) in b"xyz\0".iter().enumerate() {
            bus.write8(src + i as u32, *x);
        }
        // dst starts as "ab" (or unterminated within siz), rest 0x55.
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, init: &[u8], siz: u32| {
            for i in 0..16 {
                bus.write8(dst + i, 0x55);
            }
            for (i, x) in init.iter().enumerate() {
                bus.write8(dst + i as u32, *x);
            }
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, dst);
            cpu.regs.write(REG_A1, src);
            cpu.regs.write(REG_A2, siz);
            cpu.regs.pc = STRLCAT;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(STRLCAT));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            let ret = cpu.regs.read(REG_A0);
            (ret, (0..8).map(|i| bus.read8(dst + i)).collect::<Vec<u8>>())
        };
        assert_eq!(
            call(&mut cpu, &mut bus, b"ab\0", 16),
            (5, b"abxyz\0\x55\x55".to_vec()),
            "fits: full append, terminated"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, b"ab\0", 4),
            (5, b"abx\0\x55\x55\x55\x55".to_vec()),
            "truncated to siz - 1, terminated, returns the length tried"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, b"ab\0", 3),
            (5, b"ab\0\x55\x55\x55\x55\x55".to_vec()),
            "no room: dst untouched"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, b"ab", 2),
            (5, b"ab\x55\x55\x55\x55\x55\x55".to_vec()),
            "no NUL within siz: siz + strlen(src), nothing written"
        );
        assert_eq!(
            call(&mut cpu, &mut bus, b"", 0),
            (3, [0x55; 8].to_vec()),
            "siz 0 writes nothing"
        );
    }

    /// Runs ROM `addr` as `size_t f(const char *s, const char *set)`.
    fn span_call(addr: u32, s: &[u8], set: &[u8]) -> u32 {
        let (sa, seta) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(sa, 32);
        bus.add_scratch_ram(seta, 32);
        for (i, x) in s.iter().enumerate() {
            bus.write8(sa + i as u32, *x);
        }
        for (i, x) in set.iter().enumerate() {
            bus.write8(seta + i as u32, *x);
        }
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, sa);
        cpu.regs.write(REG_A1, seta);
        cpu.regs.pc = addr;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(addr));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        cpu.regs.read(REG_A0)
    }

    #[test]
    fn strlcpy_stub_is_bsd_strlcpy() {
        let (src, dst) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(src, 32);
        bus.add_scratch_ram(dst, 32);
        for (i, x) in b"xyz\0".iter().enumerate() {
            bus.write8(src + i as u32, *x);
        }
        let mut call = |siz: u32| {
            for i in 0..8 {
                bus.write8(dst + i, 0x55);
            }
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, dst);
            cpu.regs.write(REG_A1, src);
            cpu.regs.write(REG_A2, siz);
            cpu.regs.pc = STRLCPY;
            let info = cpu.step(&mut bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(STRLCPY));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            (cpu.regs.read(REG_A0), (0..8).map(|i| bus.read8(dst + i)).collect::<Vec<u8>>())
        };
        assert_eq!(call(16), (3, b"xyz\0\x55\x55\x55\x55".to_vec()), "fits");
        assert_eq!(call(4), (3, b"xyz\0\x55\x55\x55\x55".to_vec()), "exact fit");
        assert_eq!(call(3), (3, b"xy\0\x55\x55\x55\x55\x55".to_vec()), "truncated, terminated");
        assert_eq!(call(1), (3, b"\0\x55\x55\x55\x55\x55\x55\x55".to_vec()), "only the NUL");
        assert_eq!(call(0), (3, [0x55; 8].to_vec()), "siz 0 writes nothing");
    }

    #[test]
    fn strtol_stub_parses_like_newlib_and_sets_endptr() {
        let (s, endp) = (0x3fc9_0400, 0x3fc9_0440);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(s, 64);
        bus.add_scratch_ram(endp, 8);
        let mut call = |text: &[u8], base: u32, want_end: bool| {
            for i in 0..32 {
                bus.write8(s + i, 0);
            }
            for (i, x) in text.iter().enumerate() {
                bus.write8(s + i as u32, *x);
            }
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, s);
            cpu.regs.write(REG_A1, if want_end { endp } else { 0 });
            cpu.regs.write(REG_A2, base);
            cpu.regs.pc = STRTOL;
            let info = cpu.step(&mut bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(STRTOL));
            let end = (0..4).fold(0u32, |a, k| a | (u32::from(bus.read8(endp + k)) << (8 * k)));
            (cpu.regs.read(REG_A0) as i32, end.wrapping_sub(s))
        };
        assert_eq!(call(b"320", 10, true), (320, 3));
        assert_eq!(call(b"  -12x", 10, true), (-12, 5));
        assert_eq!(call(b"0x1F", 16, true), (31, 4));
        assert_eq!(call(b"0x1F", 0, true), (31, 4));
        assert_eq!(call(b"017", 0, true), (15, 3));
        assert_eq!(call(b"abc", 10, true), (0, 0), "no digits: endptr = nptr");
        assert_eq!(call(b"99999999999", 10, true).0, i32::MAX, "saturates");
        assert_eq!(call(b"-99999999999", 10, true).0, i32::MIN, "saturates");
        assert_eq!(call(b"7", 10, false).0, 7, "NULL endptr is allowed");
    }

    #[test]
    fn strrchr_stub_finds_the_last_match_and_the_terminator() {
        let s = 0x3fc9_0400;
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(s, 32);
        for (i, x) in b"/a/b.json\0".iter().enumerate() {
            bus.write8(s + i as u32, *x);
        }
        let mut call = |c: u32| {
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, s);
            cpu.regs.write(REG_A1, c);
            cpu.regs.pc = STRRCHR;
            let info = cpu.step(&mut bus);
            assert_eq!(info.rom_stub, Some(STRRCHR));
            cpu.regs.read(REG_A0)
        };
        assert_eq!(call(u32::from(b'/')), s + 2);
        assert_eq!(call(u32::from(b'z')), 0);
        assert_eq!(call(0), s + 9, "the terminator counts");
    }

    #[test]
    fn strspn_stub_counts_the_leading_bytes_in_the_set() {
        assert_eq!(span_call(STRSPN, b"//a/b ", b"/ "), 2);
        assert_eq!(span_call(STRSPN, b"abc ", b"/ "), 0);
        assert_eq!(span_call(STRSPN, b"abc ", b"cba "), 3, "stops at NUL");
        assert_eq!(span_call(STRSPN, b"abc ", b" "), 0, "empty set");
    }

    /// `char *strcpy(char *dst, const char *src)` (C11 §7.24.2.3, the ROM's
    /// newlib code at `0x40058d2e`): copies `src` and its NUL, returns `dst`.
    #[test]
    fn strcpy_stub_copies_through_the_terminator_and_returns_dst() {
        let (dst, src) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(dst, 32);
        bus.add_scratch_ram(src, 32);
        for i in 0..32 {
            bus.write8(dst + i, 0x55);
        }
        for (i, x) in b"My Badge\0".iter().enumerate() {
            bus.write8(src + i as u32, *x);
        }
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, dst);
        cpu.regs.write(REG_A1, src);
        cpu.regs.pc = STRCPY;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(STRCPY));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        assert_eq!(cpu.regs.read(REG_A0), dst);
        let out: Vec<u8> = (0..10).map(|i| bus.read8(dst + i)).collect();
        assert_eq!(&out, b"My Badge\0\x55", "NUL copied, nothing after it");
    }

    /// Runs ROM `strchr(s, c)` with `s` at `0x3fc9_0400`.
    fn strchr_call(s: &[u8], c: u32) -> u32 {
        let sa = 0x3fc9_0400;
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(sa, 32);
        for (i, x) in s.iter().enumerate() {
            bus.write8(sa + i as u32, *x);
        }
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, sa);
        cpu.regs.write(REG_A1, c);
        cpu.regs.pc = STRCHR;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(STRCHR));
        assert_eq!(cpu.regs.pc, 0x4000_1000);
        cpu.regs.read(REG_A0)
    }

    /// C11 §7.24.5.2 and the ROM's newlib code (`0x40058bf2`): the first
    /// `(unsigned char)c` in `s`, the terminating NUL included; else NULL.
    #[test]
    fn strchr_stub_finds_the_first_byte_including_the_terminator() {
        let sa = 0x3fc9_0400;
        assert_eq!(strchr_call(b"a/b/c\0", u32::from(b'/')), sa + 1);
        assert_eq!(strchr_call(b"abc\0", u32::from(b'z')), 0);
        assert_eq!(strchr_call(b"abc\0", 0), sa + 3, "c = 0 finds the NUL");
        assert_eq!(strchr_call(b"\0", u32::from(b'a')), 0);
        assert_eq!(
            strchr_call(b"x/\0", 0x100 | u32::from(b'/')),
            sa + 1,
            "c is cast to unsigned char"
        );
    }

    #[test]
    fn strcspn_stub_counts_the_leading_bytes_not_in_the_set() {
        assert_eq!(span_call(STRCSPN, b"littlefs/x ", b"/ "), 8);
        assert_eq!(span_call(STRCSPN, b"/x ", b"/ "), 0);
        assert_eq!(span_call(STRCSPN, b"abc ", b"/ "), 3, "stops at NUL");
        assert_eq!(span_call(STRCSPN, b"abc ", b" "), 3, "empty set");
    }

    #[test]
    fn strcmp_stub_returns_the_unsigned_byte_difference_at_first_mismatch_or_nul() {
        let (a, b) = (0x3fc9_0400, 0x3fc9_0420);
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        let mut bus = empty_firmware_bus();
        bus.add_scratch_ram(a, 16);
        bus.add_scratch_ram(b, 16);
        let call = |cpu: &mut Cpu, bus: &mut FirmwareBus, x: &[u8], y: &[u8]| {
            for (i, v) in x.iter().enumerate() {
                bus.write8(a + i as u32, *v);
            }
            for (i, v) in y.iter().enumerate() {
                bus.write8(b + i as u32, *v);
            }
            cpu.regs.write(REG_RA, 0x4000_1000);
            cpu.regs.write(REG_A0, a);
            cpu.regs.write(REG_A1, b);
            cpu.regs.pc = STRCMP;
            let info = cpu.step(bus);
            assert!(!info.trap_taken);
            assert_eq!(info.rom_stub, Some(STRCMP));
            assert_eq!(cpu.regs.pc, 0x4000_1000);
            cpu.regs.read(REG_A0) as i32
        };
        assert_eq!(call(&mut cpu, &mut bus, b"abc\0", b"abc\0"), 0, "equal");
        assert_eq!(call(&mut cpu, &mut bus, b"abX\0", b"abY\0"), -1);
        assert_eq!(
            call(&mut cpu, &mut bus, b"ab\0", b"abc\0"),
            -0x63,
            "shorter < longer"
        );
        assert_eq!(call(&mut cpu, &mut bus, b"abc\0", b"ab\0"), 0x63);
        assert_eq!(
            call(&mut cpu, &mut bus, b"\xff\0", b"\x01\0"),
            0xfe,
            "unsigned bytes"
        );
    }

    #[test]
    fn div_stub_returns_the_quot_rem_pair_in_a0_a1() {
        let run = |num: i32, den: i32| {
            let (cpu, _bus) = run_stub_call(DIV, &[(REG_A0, num as u32), (REG_A1, den as u32)]);
            (cpu.regs.read(REG_A0) as i32, cpu.regs.read(REG_A1) as i32)
        };
        assert_eq!(run(0x50, 0x50), (1, 0), "the observed boot call");
        assert_eq!(run(17, 5), (3, 2));
        assert_eq!(run(-17, 5), (-3, -2), "C truncation toward zero");
        assert_eq!(run(17, -5), (-3, 2));
        assert_eq!(run(5, 0), (-1, 5), "M-extension divide by zero");
        assert_eq!(run(i32::MIN, -1), (i32::MIN, 0), "M-extension overflow");
    }

    // ---- Milestone 3 Task 7: `ets_printf` full-execution tests ----
    //
    // These exercise the real stub end to end through a real `FirmwareBus`
    // (so a real `UsbSerialJtag`/`Console` backs the output), unlike
    // `cpu::rom_stubs::tests`' `compute_printf` unit tests, which use a
    // plain in-memory fake and never touch `Cpu::apply_rom_stub`'s
    // register/stack-splitting `CpuPrintfHost` adapter at all.

    #[test]
    fn ets_printf_stub_writes_the_observed_boot_log_line_through_a_real_bus() {
        // The exact call shape `cpu_start` makes right before `abort()`
        // (see this module's doc, entry 12, and this task's boot-probe
        // report): `ets_printf("E (%lu) %s: Invalid app image header\n",
        // <timestamp>, "cpu_start")`.
        use crate::mem::soc::DRAM_RANGE;
        let fmt_addr = 0x3fcd_0000u32;
        let tag_addr = 0x3fcd_0100u32;
        assert!(DRAM_RANGE.contains(&fmt_addr) && DRAM_RANGE.contains(&tag_addr));

        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.add_scratch_ram(fmt_addr, 64);
        bus.add_scratch_ram(tag_addr, 16);
        for (i, b) in b"E (%lu) %s: Invalid app image header\n\0"
            .iter()
            .enumerate()
        {
            bus.write8(fmt_addr + i as u32, *b);
        }
        for (i, b) in b"cpu_start\0".iter().enumerate() {
            bus.write8(tag_addr + i as u32, *b);
        }

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, fmt_addr);
        cpu.regs.write(REG_A1, 0); // timestamp
        cpu.regs.write(REG_A2, tag_addr);
        cpu.regs.pc = ETS_PRINTF;
        let info = cpu.step(&mut bus);

        assert!(!info.trap_taken, "a stub call must never trap");
        assert_eq!(info.rom_stub, Some(ETS_PRINTF));
        assert_eq!(cpu.regs.pc, 0x4000_1000, "pc must return to ra");

        let expected = "E (0) cpu_start: Invalid app image header\n";
        assert_eq!(bus.console.text(), expected);
        assert_eq!(
            cpu.regs.read(REG_A0),
            expected.len() as u32,
            "a0 must be the number of characters written, per ets_sys.h"
        );
    }

    #[test]
    fn ets_printf_stub_reads_stack_spilled_varargs_through_a_real_bus() {
        // Nine `%d` conversions: the first seven come from a1..a7, the
        // last two spill to the stack at sp+0/sp+4 -- this is the one
        // thing the pure `compute_printf` unit tests (cpu::rom_stubs::
        // tests, backed by a flat fake with no register/stack distinction)
        // cannot exercise, since that split is `CpuPrintfHost`'s job, not
        // `compute_printf`'s.
        use crate::mem::soc::DRAM_RANGE;
        let fmt_addr = 0x3fcd_0200u32;
        let sp = 0x3fcd_0300u32;
        assert!(DRAM_RANGE.contains(&fmt_addr) && DRAM_RANGE.contains(&sp));

        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.add_scratch_ram(fmt_addr, 64);
        bus.add_scratch_ram(sp, 16);
        for (i, b) in b"%d %d %d %d %d %d %d %d %d\0".iter().enumerate() {
            bus.write8(fmt_addr + i as u32, *b);
        }
        bus.write32(sp, 80); // 8th vararg -- stack word 0
        bus.write32(sp + 4, 90); // 9th vararg -- stack word 1

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4000_1000);
        cpu.regs.write(REG_A0, fmt_addr);
        cpu.regs.write(REG_A1, 10);
        cpu.regs.write(REG_A2, 20);
        cpu.regs.write(REG_A3, 30);
        cpu.regs.write(REG_A4, 40);
        cpu.regs.write(REG_A5, 50);
        cpu.regs.write(REG_A6, 60);
        cpu.regs.write(REG_A7, 70);
        cpu.regs.write(REG_SP, sp);
        cpu.regs.pc = ETS_PRINTF;
        let info = cpu.step(&mut bus);

        assert!(!info.trap_taken);
        assert_eq!(info.rom_stub, Some(ETS_PRINTF));
        assert_eq!(bus.console.text(), "10 20 30 40 50 60 70 80 90");
    }

    // ---- Milestone 3 Task D6: guest-executed ROM `qsort` ----
    //
    // These run the real blob through a real `Cpu` + `FirmwareBus`,
    // installed exactly the way `crate::boot` installs it, against a
    // comparator that is itself guest code in RAM -- so every `compar` call
    // is a genuine `jalr` into firmware-style code and back, never a Rust
    // callback.

    use crate::cpu::encode::*;
    use crate::cpu::exception_code;

    /// Guest comparator code (IRAM scratch).
    const CMP_ADDR: u32 = 0x4038_0000;
    /// `ra` handed to `qsort`; reaching it means `qsort` returned. Never
    /// executed.
    const RET_SENTINEL: u32 = 0x4038_0f00;
    /// Incremented by every comparator call (DRAM scratch).
    const CALLS_ADDR: u32 = 0x3fc9_0000;
    const ARRAY_ADDR: u32 = 0x3fc9_1000;
    const STACK_TOP: u32 = 0x3fc9_8000;

    /// Bumps the call counter at [`CALLS_ADDR`] (`0x3fc9_0000`).
    const COUNT_CALL: [u32; 4] = [
        lui(T0, 0x3fc90), // t0 = &calls
        lw(T1, T0, 0),    // t1 = calls
        addi(T1, T1, 1),  // t1 += 1
        sw(T1, T0, 0),    // calls = t1
    ];

    /// Then trashes every caller-saved register the psABI lets a callee
    /// trash (except a0, the result, and ra, needed to return) and returns.
    const CLOBBER_AND_RET: [u32; 15] = [
        addi(A1, ZERO, -1),
        addi(A2, ZERO, -1),
        addi(A3, ZERO, -1),
        addi(A4, ZERO, -1),
        addi(A5, ZERO, -1),
        addi(A6, ZERO, -1),
        addi(A7, ZERO, -1),
        addi(T0, ZERO, -1),
        addi(T1, ZERO, -1),
        addi(T2, ZERO, -1),
        addi(T3, ZERO, -1),
        addi(T4, ZERO, -1),
        addi(T5, ZERO, -1),
        addi(T6, ZERO, -1),
        jalr(ZERO, RA, 0), // ret
    ];

    /// `int cmp(const int32_t *a, const int32_t *b)` returning
    /// `(*a > *b) - (*a < *b)` -- overflow-free, unlike `*a - *b`.
    fn int32_comparator() -> Vec<u32> {
        let mut code = COUNT_CALL.to_vec();
        code.extend([
            lw(T0, A0, 0),   // t0 = *a
            lw(T1, A1, 0),   // t1 = *b
            slt(A0, T1, T0), // a0 = *b < *a
            slt(T2, T0, T1), // t2 = *a < *b
            sub(A0, A0, T2), // a0 = (a>b) - (a<b)
        ]);
        code.extend(CLOBBER_AND_RET);
        code
    }

    /// Same, but on each element's *first byte* only (unsigned), for the
    /// odd-`size` test: the other bytes are payload that must travel with
    /// their key.
    fn first_byte_comparator() -> Vec<u32> {
        let mut code = COUNT_CALL.to_vec();
        code.extend([
            lbu(T0, A0, 0),
            lbu(T1, A1, 0),
            slt(A0, T1, T0),
            slt(T2, T0, T1),
            sub(A0, A0, T2),
        ]);
        code.extend(CLOBBER_AND_RET);
        code
    }

    enum Comparator {
        /// 32-bit words, written into RAM as-is.
        Words(Vec<u32>),
        /// Raw 16-bit (RVC) halfwords, as they appear in `factory.bin`.
        Halfwords(Vec<u16>),
    }

    struct QsortRun {
        array: Vec<u8>,
        calls: u32,
    }

    /// Calls ROM `qsort(ARRAY_ADDR, nmemb, size, CMP_ADDR)` on `bytes` and
    /// runs it to completion, asserting on the way the psABI contract:
    /// no trap, `sp` 16-byte aligned at every comparator entry, and on
    /// return `pc == ra`, `sp` and `s0..s11` (plus `gp`/`tp`) unchanged.
    fn run_qsort(bytes: &[u8], nmemb: u32, size: u32, cmp: Comparator) -> QsortRun {
        use crate::cpu::rom_stubs::{REG_A3, REG_RA, REG_SP};

        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.add_scratch_ram(CALLS_ADDR, (STACK_TOP - CALLS_ADDR) as usize);
        bus.add_scratch_ram(CMP_ADDR, 0x1000);
        install_esp32c3_rom_code(&mut bus);
        match cmp {
            Comparator::Words(words) => {
                for (i, w) in words.iter().enumerate() {
                    bus.write32(CMP_ADDR + 4 * i as u32, *w);
                }
            }
            Comparator::Halfwords(halves) => {
                for (i, h) in halves.iter().enumerate() {
                    bus.write16(CMP_ADDR + 2 * i as u32, *h);
                }
            }
        }
        for (i, b) in bytes.iter().enumerate() {
            bus.write8(ARRAY_ADDR + i as u32, *b);
        }

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        // Distinctive values in every register qsort must preserve.
        const PRESERVED: [u8; 14] = [3, 4, 8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
        for r in PRESERVED {
            cpu.regs.write(r, 0x5a00_0000 | u32::from(r));
        }
        cpu.regs.write(REG_A0, ARRAY_ADDR);
        cpu.regs.write(REG_A1, nmemb);
        cpu.regs.write(REG_A2, size);
        cpu.regs.write(REG_A3, CMP_ADDR);
        cpu.regs.write(REG_RA, RET_SENTINEL);
        cpu.regs.write(REG_SP, STACK_TOP);
        cpu.regs.pc = QSORT;

        let mut steps = 0u32;
        while cpu.regs.pc != RET_SENTINEL {
            assert!(steps < 200_000, "qsort did not return within 200,000 steps");
            if cpu.regs.pc == CMP_ADDR {
                assert_eq!(
                    cpu.regs.read(REG_SP) % 16,
                    0,
                    "sp must be 16-byte aligned at every compar call"
                );
            }
            let info = cpu.step(&mut bus);
            assert!(
                !info.trap_taken,
                "qsort trapped at {:#x} (mcause {}, mtval {:#x})",
                info.pc_before, cpu.csr.mcause, cpu.csr.mtval
            );
            steps += 1;
        }

        assert_eq!(cpu.regs.read(REG_SP), STACK_TOP, "sp must be restored");
        for r in PRESERVED {
            assert_eq!(
                cpu.regs.read(r),
                0x5a00_0000 | u32::from(r),
                "callee-saved/reserved x{r} must be preserved"
            );
        }

        let array = (0..bytes.len() as u32)
            .map(|i| bus.read8(ARRAY_ADDR + i))
            .collect();
        QsortRun {
            array,
            calls: bus.read32(CALLS_ADDR),
        }
    }

    fn i32_bytes(values: &[i32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn sort_i32(values: &[i32]) -> (Vec<i32>, u32) {
        let run = run_qsort(
            &i32_bytes(values),
            values.len() as u32,
            4,
            Comparator::Words(int32_comparator()),
        );
        let sorted = run
            .array
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes(*c))
            .collect();
        (sorted, run.calls)
    }

    #[test]
    fn qsort_sorts_int32_ascending() {
        let (sorted, calls) = sort_i32(&[5, -3, 17, 0, -3_000_000, 42, 7, i32::MIN, i32::MAX]);
        assert_eq!(
            sorted,
            [i32::MIN, -3_000_000, -3, 0, 5, 7, 17, 42, i32::MAX]
        );
        assert!(calls > 0);
    }

    #[test]
    fn qsort_with_zero_or_one_element_never_calls_compar_or_writes_the_array() {
        let bytes = i32_bytes(&[0x1122_3344, 0x5566_7788]);
        for nmemb in [0, 1] {
            let run = run_qsort(&bytes, nmemb, 4, Comparator::Words(int32_comparator()));
            assert_eq!(run.calls, 0, "nmemb {nmemb}: compar must not be called");
            assert_eq!(run.array, bytes, "nmemb {nmemb}: array must be untouched");
        }
    }

    #[test]
    fn qsort_with_zero_element_size_never_calls_compar_or_writes_the_array() {
        let bytes = i32_bytes(&[0x1122_3344, 0x5566_7788, 0x0102_0304]);
        let run = run_qsort(&bytes, 3, 0, Comparator::Words(int32_comparator()));
        assert_eq!(run.calls, 0, "size 0: compar must not be called");
        assert_eq!(run.array, bytes, "size 0: array must be untouched");
    }

    #[test]
    fn qsort_handles_already_sorted_and_reverse_sorted_input() {
        let ascending: Vec<i32> = (0..12).collect();
        let (sorted, calls) = sort_i32(&ascending);
        assert_eq!(sorted, ascending);
        assert_eq!(
            calls, 11,
            "sorted input: one comparison per element after the first"
        );

        let descending: Vec<i32> = (0..12).rev().collect();
        let (sorted, _) = sort_i32(&descending);
        assert_eq!(sorted, ascending);
    }

    #[test]
    fn qsort_handles_duplicates() {
        let (sorted, _) = sort_i32(&[3, 1, 3, 2, 1, 3, -1, 2]);
        assert_eq!(sorted, [-1, 1, 1, 2, 2, 3, 3, 3]);
    }

    #[test]
    fn qsort_sorts_the_observed_boot_call_with_the_firmwares_own_comparator() {
        // The exact call boot makes (Task D6 Step 1): ESP-IDF v5.5.3's
        // `s_prepare_reserved_regions()` (components/heap/port/
        // memory_layout_utils.c) sorting 5 `soc_reserved_region_t {start,
        // end}` (size 8) with `s_compare_reserved_regions`, whose compiled
        // body in factory.bin at 0x420029bc is these four RVC halfwords:
        // `c.lw a0,0(a0); c.lw a5,0(a1); c.sub a0,a5; c.ret` --
        // `(int)r_a->start - (int)r_b->start`. The input is the array's
        // exact contents at the observed call.
        let regions: [(u32, u32); 5] = [
            (0x0000_0000, 0x3fce_0000),
            (0x5000_1fe8, 0x5000_2000),
            (0x5000_0000, 0x5000_0020),
            (0x3fc8_0000, 0x3fc9_9c00),
            (0x3fc9_9c00, 0x3fcb_d180),
        ];
        let bytes: Vec<u8> = regions
            .iter()
            .flat_map(|(s, e)| s.to_le_bytes().into_iter().chain(e.to_le_bytes()))
            .collect();
        let run = run_qsort(
            &bytes,
            5,
            8,
            Comparator::Halfwords(vec![0x4108, 0x419c, 0x8d1d, 0x8082]),
        );
        let sorted: Vec<(u32, u32)> = run
            .array
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| {
                (
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                )
            })
            .collect();
        assert_eq!(
            sorted,
            [
                (0x0000_0000, 0x3fce_0000),
                (0x3fc8_0000, 0x3fc9_9c00),
                (0x3fc9_9c00, 0x3fcb_d180),
                (0x5000_0000, 0x5000_0020),
                (0x5000_1fe8, 0x5000_2000),
            ],
            "each 8-byte element must move whole, `end` travelling with `start`"
        );
    }

    #[test]
    fn qsort_swaps_bytewise_for_an_odd_element_size() {
        // size 3: [key, payload, payload].
        let bytes = [
            9, 0x90, 0x91, //
            2, 0x20, 0x21, //
            7, 0x70, 0x71, //
            0, 0x00, 0x01, //
            5, 0x50, 0x51,
        ];
        let run = run_qsort(&bytes, 5, 3, Comparator::Words(first_byte_comparator()));
        assert_eq!(
            run.array,
            [
                0, 0x00, 0x01, //
                2, 0x20, 0x21, //
                5, 0x50, 0x51, //
                7, 0x70, 0x71, //
                9, 0x90, 0x91,
            ]
        );
    }

    #[test]
    fn neighbouring_rom_libc_slots_still_fault_exactly_as_before() {
        // qsort's slot is one 4-byte jump; its neighbours `ldiv`
        // (0x40000430) and `rand_r` (0x40000438) are still unstubbed and
        // unbacked, so fetching them must still raise
        // INSTRUCTION_ACCESS_FAULT with the address in mtval.
        for addr in [0x4000_0430u32, 0x4000_0438] {
            let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
            install_esp32c3_rom_code(&mut bus);
            let mut cpu = Cpu::new();
            cpu.set_rom_stubs(esp32c3_rom_stubs());
            cpu.csr.mtvec = 0x4038_0000;
            cpu.regs.pc = addr;
            let info = cpu.step(&mut bus);
            assert!(info.trap_taken, "{addr:#x} must still trap");
            assert_eq!(cpu.csr.mcause, exception_code::INSTRUCTION_ACCESS_FAULT);
            assert_eq!(cpu.csr.mtval, addr);
        }
    }

    #[test]
    fn every_rom_code_word_is_a_32_bit_instruction_this_crates_decoder_accepts() {
        assert!(!ESP32C3_ROM_CODE.is_empty());
        for blob in ESP32C3_ROM_CODE {
            for (i, word) in blob.words.iter().enumerate() {
                let addr = blob.base + 4 * i as u32;
                assert_eq!(
                    word & 0b11,
                    0b11,
                    "{addr:#x}: {word:#010x} is not a 32-bit encoding"
                );
                assert!(
                    !matches!(
                        crate::cpu::decode_32(*word),
                        crate::cpu::Instruction::Illegal(_)
                    ),
                    "{addr:#x}: {word:#010x} does not decode"
                );
            }
        }
    }

    #[test]
    fn qsort_slot_is_a_single_jump_to_the_body() {
        let slot = ESP32C3_ROM_CODE
            .iter()
            .find(|b| b.base == QSORT)
            .expect("qsort's jump-table slot is mapped");
        assert_eq!(
            slot.words.len(),
            1,
            "exactly one 4-byte slot, never spilling into 0x40000438"
        );
        assert_eq!(
            crate::cpu::decode_32(slot.words[0]),
            crate::cpu::Instruction::Jal {
                rd: 0,
                imm: (QSORT_BODY_ADDR - QSORT) as i32
            }
        );
        assert!(ESP32C3_ROM_CODE.iter().any(|b| b.base == QSORT_BODY_ADDR));
    }

    #[test]
    fn rom_code_never_overlaps_a_stub_and_stays_inside_the_free_rom_range() {
        let table = esp32c3_rom_stubs();
        for (addr, _) in table.entries_sorted() {
            assert!(
                addr < 0x4000_2000,
                "stub {addr:#x} is not below 0x4000_2000"
            );
        }
        for blob in ESP32C3_ROM_CODE {
            let end = blob.base + 4 * blob.words.len() as u32;
            for addr in (blob.base..end).step_by(2) {
                assert_eq!(
                    table.lookup(addr),
                    None,
                    "{addr:#x} is both stubbed and ROM code"
                );
            }
            if blob.base != QSORT && blob.base != STRDUP {
                assert!(
                    blob.base >= ROM_CODE_FREE_RANGE.start && end <= ROM_CODE_FREE_RANGE.end,
                    "blob at {:#x} is outside ROM_CODE_FREE_RANGE",
                    blob.base
                );
            }
        }
    }

    // ---- Milestone 3 Task D7: ROM layout data (`ets_rom_layout_p`) ----

    /// An empty-flash bus with only `crate::rom`'s ROM data installed, the
    /// way `crate::boot` installs it.
    fn bus_with_rom_data() -> FirmwareBus {
        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        install_esp32c3_rom_data(&mut bus);
        bus
    }

    #[test]
    fn ets_rom_layout_p_points_at_the_rom_layout_table() {
        let mut bus = bus_with_rom_data();
        assert_eq!(bus.read32(0x3ff1_fffc), 0x3ff1_be3c);
    }

    #[test]
    fn the_rom_layout_tables_dram0_rtos_reserved_start_is_the_rev3_rom_value() {
        let mut bus = bus_with_rom_data();
        // Exactly the firmware's own access sequence (0x420029d6..de):
        // `lw a5, -4(0x3ff20000)`, then `lw a5, 4(a5)`.
        let layout = bus.read32(0x3ff2_0000 - 4);
        assert_eq!(bus.read32(layout + 4), 0x3fcd_f060);
        assert!(
            bus.unmapped_log().is_empty(),
            "both loads hit mapped ROM data, not the catch-all"
        );
    }

    #[test]
    fn coexist_rom_version_string_is_backed_with_the_rev3_rom_bytes() {
        let mut bus = bus_with_rom_data();
        let s: Vec<u8> = (0..8).map(|i| bus.read8(0x3ff1_b74c + i)).collect();
        assert_eq!(&s, b"9387209\0");
        assert!(bus.unmapped_log().is_empty());
    }

    #[test]
    fn esp_coex_rom_version_get_returns_the_rom_version_string_pointer() {
        let stub = esp32c3_rom_stubs()
            .lookup(ESP_COEX_ROM_VERSION_GET)
            .expect("registered");
        assert_eq!(stub.name, "esp_coex_rom_version_get");
        let mut bus = bus_with_rom_data();
        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        cpu.regs.write(REG_RA, 0x4210_5722);
        cpu.regs.pc = ESP_COEX_ROM_VERSION_GET;
        let info = cpu.step(&mut bus);
        assert!(!info.trap_taken);
        assert_eq!(cpu.regs.pc, 0x4210_5722);
        assert_eq!(cpu.regs.read(REG_A0), 0x3ff1_b74c);
    }

    #[test]
    fn unconsumed_rom_layout_fields_read_zero() {
        let mut bus = bus_with_rom_data();
        for field in (0..40u32).filter(|&i| i != 1) {
            assert_eq!(bus.read32(0x3ff1_be3c + 4 * field), 0, "field {field}");
        }
    }

    #[test]
    fn rom_layout_data_is_not_executable() {
        for addr in [0x3ff1_fffcu32, 0x3ff1_be3c, 0x3ff1_be40] {
            let mut bus = bus_with_rom_data();
            let mut cpu = Cpu::new();
            cpu.set_rom_stubs(esp32c3_rom_stubs());
            cpu.csr.mtvec = 0x4038_0000;
            cpu.regs.pc = addr;
            let info = cpu.step(&mut bus);
            assert!(info.trap_taken, "{addr:#x} must trap");
            assert_eq!(cpu.csr.mcause, exception_code::INSTRUCTION_ACCESS_FAULT);
            assert_eq!(cpu.csr.mtval, addr);
        }
    }

    #[test]
    fn neighbouring_drom_mask_addresses_still_read_zero() {
        let mut bus = bus_with_rom_data();
        // Just below the pointer word, just past the table, and just below
        // the table: all still unmapped catch-all space.
        for addr in [0x3ff1_fff8u32, 0x3ff1_bedc, 0x3ff1_be38] {
            assert_eq!(bus.read32(addr), 0, "{addr:#x}");
            assert!(
                bus.unmapped_log().iter().any(|a| a.addr == addr),
                "{addr:#x} is logged as unmapped"
            );
        }
    }

    #[test]
    fn rom_data_lies_inside_the_drom_mask_and_clear_of_rom_code() {
        use crate::mem::soc::DROM_MASK_RANGE;
        assert!(!ESP32C3_ROM_DATA.is_empty());
        for blob in ESP32C3_ROM_DATA {
            let end = blob.base as u64 + 4 * blob.words.len() as u64;
            assert!(
                DROM_MASK_RANGE.contains(&blob.base) && end <= DROM_MASK_RANGE.end as u64,
                "blob at {:#x} is outside the DROM mask",
                blob.base
            );
        }
        // Installing ROM code and ROM data together must not trip
        // `FirmwareBus`'s overlap assertion.
        let mut bus = bus_with_rom_data();
        install_esp32c3_rom_code(&mut bus);
    }

    // ---- Milestone 4 Task D-M4-2: guest-executed ROM newlib `strdup` ----

    const SD_GETREENT_ADDR: u32 = 0x4038_0000;
    const SD_MALLOC_ADDR: u32 = 0x4038_0100;
    /// `_malloc_r` records its `(reent, size)` arguments here.
    const SD_RECORD_ADDR: u32 = 0x3fc9_0000;
    const SD_STR_ADDR: u32 = 0x3fc9_1000;
    const SD_TABLE_ADDR: u32 = 0x3fc9_2000;
    const SD_REENT: u32 = 0x3fc9_3000;
    const SD_HEAP: u32 = 0x3fc9_4000;

    struct StrdupRun {
        ret: u32,
        heap: Vec<u8>,
        malloc_reent: u32,
        malloc_size: u32,
    }

    /// Calls ROM `strdup(SD_STR_ADDR)` with a firmware-style syscall table
    /// (guest `__getreent` returning `SD_REENT`, guest `_malloc_r`
    /// recording its arguments and returning `heap_ret`) and runs it to
    /// completion, checking the psABI contract like `run_qsort`.
    fn run_strdup(s: &[u8], heap_ret: u32) -> StrdupRun {
        use crate::cpu::rom_stubs::{REG_RA, REG_SP};

        let mut bus = FirmwareBus::from_segments(Arc::from(Vec::new().into_boxed_slice()), &[]);
        bus.add_scratch_ram(SD_RECORD_ADDR, (STACK_TOP - SD_RECORD_ADDR) as usize);
        bus.add_scratch_ram(SD_GETREENT_ADDR, 0x1000);
        bus.add_scratch_ram(SYSCALL_TABLE_PTR & !0xFFF, 0x1000);
        install_esp32c3_rom_code(&mut bus);
        let mut getreent = vec![lui(A0, SD_REENT >> 12)];
        getreent.extend(CLOBBER_AND_RET);
        let mut malloc = vec![
            lui(T0, SD_RECORD_ADDR >> 12),
            sw(A0, T0, 0),
            sw(A1, T0, 4),
            lui(A0, heap_ret >> 12),
        ];
        malloc.extend(CLOBBER_AND_RET);
        for (base, code) in [(SD_GETREENT_ADDR, &getreent), (SD_MALLOC_ADDR, &malloc)] {
            for (i, w) in code.iter().enumerate() {
                bus.write32(base + 4 * i as u32, *w);
            }
        }
        bus.write32(SD_TABLE_ADDR, SD_GETREENT_ADDR); // .__getreent
        bus.write32(SD_TABLE_ADDR + 4, SD_MALLOC_ADDR); // ._malloc_r
        bus.write32(SYSCALL_TABLE_PTR, SD_TABLE_ADDR);
        for (i, b) in s.iter().enumerate() {
            bus.write8(SD_STR_ADDR + i as u32, *b);
        }
        for i in 0..64 {
            bus.write8(SD_HEAP + i, 0xAA);
        }

        let mut cpu = Cpu::new();
        cpu.set_rom_stubs(esp32c3_rom_stubs());
        const PRESERVED: [u8; 14] = [3, 4, 8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
        for r in PRESERVED {
            cpu.regs.write(r, 0x5a00_0000 | u32::from(r));
        }
        cpu.regs.write(REG_A0, SD_STR_ADDR);
        cpu.regs.write(REG_RA, RET_SENTINEL);
        cpu.regs.write(REG_SP, STACK_TOP);
        cpu.regs.pc = STRDUP;
        let mut steps = 0u32;
        while cpu.regs.pc != RET_SENTINEL {
            assert!(steps < 10_000, "strdup did not return within 10,000 steps");
            let info = cpu.step(&mut bus);
            assert!(
                !info.trap_taken,
                "strdup trapped at {:#x} (mcause {}, mtval {:#x})",
                info.pc_before, cpu.csr.mcause, cpu.csr.mtval
            );
            steps += 1;
        }
        assert_eq!(cpu.regs.read(REG_SP), STACK_TOP, "sp must be restored");
        for r in PRESERVED {
            assert_eq!(
                cpu.regs.read(r),
                0x5a00_0000 | u32::from(r),
                "callee-saved/reserved x{r} must be preserved"
            );
        }
        StrdupRun {
            ret: cpu.regs.read(REG_A0),
            heap: (0..64).map(|i| bus.read8(SD_HEAP + i)).collect(),
            malloc_reent: bus.read32(SD_RECORD_ADDR),
            malloc_size: bus.read32(SD_RECORD_ADDR + 4),
        }
    }

    /// newlib `_strdup_r` (the body the ROM's slot jumps to, via `strdup`
    /// = `_strdup_r(__getreent(), s)`): `_malloc_r(reent, strlen(s) + 1)`,
    /// then `memcpy` of the string and its NUL.
    #[test]
    fn strdup_copies_into_memory_from_the_syscall_tables_malloc_r() {
        let run = run_strdup(b"hal_sleep\0", SD_HEAP);
        assert_eq!(run.ret, SD_HEAP);
        assert_eq!(run.malloc_reent, SD_REENT);
        assert_eq!(run.malloc_size, 10);
        assert_eq!(&run.heap[..10], b"hal_sleep\0");
        assert_eq!(run.heap[10], 0xAA, "nothing past the NUL is written");
    }

    #[test]
    fn strdup_returns_null_when_malloc_fails() {
        let run = run_strdup(b"x\0", 0);
        assert_eq!(run.ret, 0);
        assert_eq!(run.malloc_size, 2);
        assert!(run.heap.iter().all(|b| *b == 0xAA));
    }

    #[test]
    fn strdup_of_the_empty_string_allocates_one_byte() {
        let run = run_strdup(b"\0", SD_HEAP);
        assert_eq!(run.ret, SD_HEAP);
        assert_eq!(run.malloc_size, 1);
        assert_eq!(run.heap[0], 0);
        assert_eq!(run.heap[1], 0xAA);
    }
}

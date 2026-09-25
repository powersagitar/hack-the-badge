import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { initSync, add } from "../src/cpu/wasm-pkg/emulator_wasm.js";
import { createFirmwareEmulator, initCpuWasmSync } from "../src/cpu/bridge";

describe("emulator-wasm round trip", () => {
  test("Rust -> WASM -> TS add() matches native behavior", () => {
    const wasmBytes = readFileSync(
      new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
    );
    initSync({ module: wasmBytes });
    expect(add(2, 3)).toBe(5);
  });
});

describe("emulator-wasm firmware-boot boundary", () => {
  // The plan's "Verification strategy" section calls for a minimal
  // integration check of the wasm-bindgen boundary beyond the trivial add()
  // round-trip above -- this is that check, exercising the actual point of
  // this whole branch: booting the real dumped firmware through the WASM CPU
  // emulator and reading back a run summary + framebuffer through the same
  // `src/cpu/bridge.ts` surface `src/runtime/firmware-runtime.ts` uses.
  test("boots factory.bin, runs a bounded budget, and exposes a full framebuffer", () => {
    const wasmBytes = readFileSync(
      new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
    );
    initCpuWasmSync(wasmBytes);

    const image = readFileSync(new URL("../public/firmware/factory.bin", import.meta.url));
    const handle = createFirmwareEmulator(new Uint8Array(image));
    try {
      const report = handle.run(500_000);

      // Milestone 3 Task D1: this used to assert 0 traps. Modeling
      // RTC_CNTL's RTC timer (emulator-core/src/peripherals/rtc_cntl.rs)
      // let boot's rtc_cntl_ll_get_rtc_time()-based busy-wait actually
      // terminate, so boot now runs past a spin loop that previously kept
      // it fault-free forever, and hits a genuine ROM-call fault.
      //
      // Milestone 3 Task D2: ROM libc memcpy is now HLE-stubbed
      // (emulator-core/src/cpu/rom_stubs.rs's RomStubEffect::Memcpy), which
      // unblocked the Task-D1-era fault (at step 401,761) -- boot now runs
      // further and hits a *different* unstubbed ROM call
      // (ets_efuse_get_spiconfig, step 402,113), still well within this
      // 500,000-step budget.
      //
      // Milestone 3 Task D3: ets_efuse_get_spiconfig and six more ROM
      // calls it led to are now HLE-stubbed (emulator-core/src/rom.rs's
      // module doc, entry 10), which unblocked the Task-D2-era fault --
      // boot now runs further still and hits a *different* unstubbed ROM
      // call (esprv_intc_int_enable, step 405,806), still well within this
      // 500,000-step budget, so the trap count here stays 1.
      //
      // Milestone 3 Task D3 fix round 1: esprv_intc_int_enable and its four
      // siblings now perform real InterruptController register writes
      // (emulator-core/src/rom.rs's module doc, entry 11), which unblocked
      // the Task-D3-era fault -- boot now runs further still and hits a
      // *different* unstubbed ROM call (itoa, step 407,471).
      //
      // Milestone 3 Task D4: itoa, and the very next unstubbed ROM libc
      // call it led to (strcat, step ~407,498), are now HLE-stubbed
      // (emulator-core/src/rom.rs's module doc, entry 12). With both real,
      // boot no longer stalls on an unmapped ROM-address fetch at all --
      // it hits a *qualitatively different* fault: a real
      // ILLEGAL_INSTRUCTION trap (step 407,549) at ESP-IDF's own
      // panic_abort(), still well within this 500,000-step budget, so the
      // trap count here stays 1 (the second fault -- the still-unstubbed
      // software_reset_cpu the panic handler's own reboot attempt calls --
      // isn't reached until step 645,410, past this budget).
      //
      // Task D4 fix round 1 (correction, not a code change): itoa/strcat
      // are called from newlib's abort(), itself called because cpu_start
      // (ESP-IDF's early startup) rejects this image's header and aborts --
      // *not* from "normal boot progress" as originally (incorrectly)
      // documented. This trap is panic_abort() reached via that
      // pre-existing abort() call, not evidence of progress past the
      // header check. See docs/firmware-emulator-notes.md for the full
      // corrected story (superseded by Task D5 below).
      //
      // Milestone 3 Task D5: emulator-core/src/mem/bus.rs's
      // FirmwareBus::from_segments now widens each XIP (DROM/IROM) segment
      // to its containing 64 KiB flash-cache MMU page, matching what the
      // real 2nd-stage bootloader's set_cache_and_start_app() +
      // mmu_hal_map_region() actually expose. cpu_start's app-image-header
      // check reads the header from exactly this newly-exposed leading page
      // gap, so it now reads the real magic byte and passes for real --
      // abort() is never called any more, and boot runs much further (a
      // full app_init/efuse_init log block that never printed before) --
      // before hitting a new, unrelated stall: an unstubbed ROM qsort call
      // (0x4000_0434, step 408,481). That INSTRUCTION_ACCESS_FAULT is still
      // well within this 500,000-step budget, so the trap count here stays
      // 1 (the second fault -- the still-unstubbed software_reset_cpu the
      // panic handler's own reboot attempt calls -- isn't reached until
      // ~step 648,457, past this budget). See
      // emulator-core/tests/rom_stub_boot.rs's
      // boot_currently_faults_on_the_unstubbed_qsort_call_and_reaches_the_panic_handlers_reboot_message
      // (renamed in Task D6, below) and docs/firmware-emulator-notes.md for
      // the full story.
      //
      // Milestone 3 Task D6: ROM qsort is now real guest-executed RV32 code
      // mapped into the ROM address space (emulator-core/src/rom.rs's
      // module doc, entry 14), so the step-408,481 fault is gone. Its
      // caller, ESP-IDF's s_prepare_reserved_regions(), then finds that
      // the sorted reserved-region list overlaps. Entry 0 comes from the
      // unbacked ROM layout table (ets_rom_layout_p reads 0). The firmware
      // logs the overlap and calls abort(), which reaches panic_abort()'s
      // ILLEGAL_INSTRUCTION trap at step 409,071. That is still the only
      // trap within this 500,000-step budget. The software_reset_cpu
      // reboot-retry fault follows at step 647,238, past this budget. See
      // emulator-core/tests/rom_stub_boot.rs's
      // boot_currently_aborts_on_the_unbacked_rom_layout_reserved_region_overlap_and_reaches_the_panic_handlers_reboot_message.
      expect(report.traps).toBe(1);

      const fb = handle.framebuffer();
      expect(fb.length).toBe(320 * 240);
      expect(fb.length).toBe(76_800);
    } finally {
      handle.dispose();
    }
  });
});

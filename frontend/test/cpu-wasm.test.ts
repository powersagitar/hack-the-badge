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
      // it fault-free forever, and hits a genuine (and, for now,
      // deliberately not yet stubbed -- see the module doc there) ROM-call
      // fault at step 401,761. See
      // emulator-core/tests/rom_stub_boot.rs's
      // boot_currently_stalls_retrying_reboot_via_an_unidentified_unstubbed_rom_call
      // and docs/firmware-emulator-notes.md for the full story.
      expect(report.traps).toBe(1);

      const fb = handle.framebuffer();
      expect(fb.length).toBe(320 * 240);
      expect(fb.length).toBe(76_800);
    } finally {
      handle.dispose();
    }
  });
});

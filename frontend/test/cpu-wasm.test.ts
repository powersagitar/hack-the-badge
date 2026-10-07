import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { initSync, add, FirmwareEmulator } from "../src/cpu/wasm-pkg/emulator_wasm.js";
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

      // Boot takes zero traps within this budget (the full stall-by-stall
      // history is in docs/firmware-emulator-notes.md).
      expect(report.traps).toBe(0);

      const fb = handle.framebuffer();
      expect(fb.length).toBe(320 * 240);
      expect(fb.length).toBe(76_800);
    } finally {
      handle.dispose();
    }
  });

  // The WASM-path twin of emulator-core/tests/boot_progress.rs's
  // boots_to_first_real_frame: same 250,000-step chunks to the same
  // 6,750,000-step horizon, same FNV-1a hash over each pixel's
  // little-endian bytes. Uses the raw wasm-bindgen class (not the bridge
  // handle) because consoleOutput() is not part of the bridge surface.
  test(
    "boots factory.bin to the same first real frame as the native finish-line test",
    () => {
      const wasmBytes = readFileSync(
        new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
      );
      initSync({ module: wasmBytes });
      const image = readFileSync(new URL("../public/firmware/factory.bin", import.meta.url));
      const emu = new FirmwareEmulator(new Uint8Array(image));
      try {
        for (let done = 0; done < 6_750_000; done += 250_000) {
          const report = emu.run(250_000);
          const fault = report.lastInstructionFault;
          report.free();
          // As natively: faults are checked up to SPLASH_FAULT_FREE_STEPS
          // (the current stall, a ROM strlcat fault, follows the splash).
          if (done + 250_000 <= 5_500_000) {
            expect(fault).toBeUndefined();
          }
        }
        const fb = emu.framebuffer();
        expect(new Set(fb).size).toBeGreaterThan(1);
        let h = 0xcbf29ce484222325n;
        for (const px of fb) {
          for (const b of [px & 0xff, px >> 8]) {
            h ^= BigInt(b);
            h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
          }
        }
        expect(h).toBe(0x5599c270ab0429fan);
        expect(emu.consoleOutput().length).toBeGreaterThan(0);
      } finally {
        emu.free();
      }
    },
    30_000,
  );
});

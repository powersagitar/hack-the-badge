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

/** FNV-1a (64-bit) over each pixel's little-endian bytes, as boot_progress.rs's framebuffer_fnv1a. */
function fnv1a(fb: Uint16Array): bigint {
  let h = 0xcbf29ce484222325n;
  for (const px of fb) {
    for (const b of [px & 0xff, px >> 8]) {
      h ^= BigInt(b);
      h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
    }
  }
  return h;
}

/** boot_progress.rs's SPLASH_HASH (boots_to_first_real_frame). */
const SPLASH_HASH = 0x5599c270ab0429fan;
/** boot_progress.rs's FIRST_RUN_HASH (boots_to_first_run_screen). */
const FIRST_RUN_HASH = 0x8c5027cec0f79490n;

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
          expect(fault).toBeUndefined();
        }
        const fb = emu.framebuffer();
        expect(new Set(fb).size).toBeGreaterThan(1);
        expect(fnv1a(fb)).toBe(SPLASH_HASH);
        expect(emu.consoleOutput().length).toBeGreaterThan(0);
      } finally {
        emu.free();
      }
    },
    30_000,
  );

  // The WASM-path twin of boot_progress.rs's boots_to_first_run_screen
  // (its run_until_stable_frame): 250,000-step chunks, stopping at the
  // first frame that is not uniform, not the splash, and unchanged for
  // 1,000,000 steps, capped at 17,500,000 steps; same hash.
  test(
    "boots factory.bin to the same first-run screen as the native finish-line test",
    () => {
      const wasmBytes = readFileSync(
        new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
      );
      initSync({ module: wasmBytes });
      const image = readFileSync(new URL("../public/firmware/factory.bin", import.meta.url));
      const emu = new FirmwareEmulator(new Uint8Array(image));
      try {
        let hash = fnv1a(emu.framebuffer());
        let hashSince = 0;
        let stable: bigint | undefined;
        for (let done = 0; done < 17_500_000 && stable === undefined; ) {
          const report = emu.run(250_000);
          const fault = report.lastInstructionFault;
          report.free();
          expect(fault).toBeUndefined();
          done += 250_000;
          const fb = emu.framebuffer();
          const h = fnv1a(fb);
          if (h !== hash) {
            hash = h;
            hashSince = done;
          } else if (
            done - hashSince >= 1_000_000 &&
            fb.some((px) => px !== fb[0]) &&
            h !== SPLASH_HASH
          ) {
            stable = h;
          }
        }
        expect(stable).toBe(FIRST_RUN_HASH);
        expect(emu.consoleOutput()).toContain("app_reg: launched My Badge");
      } finally {
        emu.free();
      }
    },
    60_000,
  );
});

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { initSync, add, FirmwareEmulator } from "../src/cpu/wasm-pkg/emulator_wasm.js";
import { createFirmwareEmulator, initCpuWasmSync } from "../src/cpu/bridge";
import { createProvisioner, type ProvisionState } from "../src/runtime/provisioner";
import {
  AFTER_PRESS_STABLE_FOR,
  AFTER_SHAKE_PAGES,
  ANIMATED_PAGE_STEPS,
  BOOT_STABLE_FOR,
  FIRST_RUN_HASH,
  FIRST_RUN_MAX_STEPS,
  fnv1a,
  LAUNCHER_HASH,
  LAUNCHER_RESPONSE_STEPS,
  ONBOARDING_RESPONSE_MARGIN,
  OWN_ROLE,
  OWN_ROLE_PAGE1_HASH,
  OWN_ROLE_REGISTERED_HASH,
  press,
  pressAndSettle,
  PROVISION_DEADLINE,
  REGISTERED_RESPONSE_STEPS,
  runForUs,
  runUntilStable,
  SHAKE_HALF_US,
  SHAKEN_AT_REST_HASH,
  SPLASH_HASH,
  TO_LAUNCHER_SLOT,
  TO_SHAKE_PAGE,
} from "../src/bench/walk";

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
  test("boots firmware.bin, runs a bounded budget, and exposes a full framebuffer", () => {
    const wasmBytes = readFileSync(
      new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
    );
    initCpuWasmSync(wasmBytes);

    const image = readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url));
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
  // 5,500,000-step horizon, same FNV-1a hash over each pixel's
  // little-endian bytes. Uses the raw wasm-bindgen class (not the bridge
  // handle), which also covers the generated glue directly.
  test(
    "boots firmware.bin to the same first real frame as the native finish-line test",
    () => {
      const wasmBytes = readFileSync(
        new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
      );
      initSync({ module: wasmBytes });
      const image = readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url));
      const emu = new FirmwareEmulator(new Uint8Array(image));
      try {
        for (let done = 0; done < 5_500_000; done += 250_000) {
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
  // 1,000,000 steps, capped at 16,500,000 steps; same hash.
  test(
    "boots firmware.bin to the same first-run screen as the native finish-line test",
    () => {
      const wasmBytes = readFileSync(
        new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
      );
      initSync({ module: wasmBytes });
      const image = readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url));
      const emu = new FirmwareEmulator(new Uint8Array(image));
      try {
        let hash = fnv1a(emu.framebuffer());
        let hashSince = 0;
        let stable: bigint | undefined;
        for (let done = 0; done < 16_500_000 && stable === undefined; ) {
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

  test("runUntil stops at the target emulated time or the step cap", () => {
    const wasmBytes = readFileSync(
      new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
    );
    initCpuWasmSync(wasmBytes);
    const image = readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url));
    const handle = createFirmwareEmulator(new Uint8Array(image));
    try {
      expect(handle.emulatedTimeUs()).toBe(0);
      const s = handle.runUntil(1_000, 1_000_000);
      expect(handle.emulatedTimeUs()).toBeGreaterThanOrEqual(1_000);
      expect(s.steps).toBe(handle.totalSteps());
      expect(handle.runUntil(1e12, 500).steps).toBe(500);
    } finally {
      handle.dispose();
    }
  });
});

describe("emulator-wasm provisioned boot", () => {
  // The WASM-path twin of boot_progress.rs's boots_to_launcher, provisioned
  // by the frontend's own provisioner (src/runtime/provisioner.ts) rather
  // than the Rust test helper, so it also proves the provisioner against
  // the real firmware. About 343M steps.
  test(
    "provisions the test identity through the console, walks onboarding, and reaches the same launcher as the native finish line",
    () => {
      initCpuWasmSync(
        readFileSync(new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url)),
      );
      const image = readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url));
      const handle = createFirmwareEmulator(new Uint8Array(image));
      try {
        expect(runUntilStable(handle, FIRST_RUN_MAX_STEPS, BOOT_STABLE_FOR, [SPLASH_HASH])).toBe(
          FIRST_RUN_HASH,
        );

        const json = readFileSync(
          new URL(`../public/firmware/test-identities/${OWN_ROLE}.json`, import.meta.url),
          "utf8",
        );
        const p = createProvisioner(handle, json);
        let state: ProvisionState = p.poll();
        while (
          (state.kind === "waiting" || state.kind === "typing") &&
          handle.totalSteps() < PROVISION_DEADLINE
        ) {
          expect(handle.run(50_000).lastInstructionFault).toBeUndefined();
          state = p.poll();
        }
        expect(state).toMatchObject({ kind: "ok" });
        expect((state as { line: string }).line).toStartWith("PROV OK id=test-");

        let cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
        let previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [FIRST_RUN_HASH]);
        expect(previous).toBe(OWN_ROLE_PAGE1_HASH);
        for (const [slot, expected] of TO_SHAKE_PAGE) {
          previous = pressAndSettle(handle, slot, previous);
          expect(previous).toBe(expected);
        }
        for (let k = 0; k < 6; k++) {
          const s = k % 2 === 0 ? 1 : -1;
          handle.setAcceleration(s * 2000, s * 2000, 1000);
          runForUs(handle, SHAKE_HALF_US);
        }
        handle.setAcceleration(0, 0, 1000);
        cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
        previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous]);
        expect(previous).toBe(SHAKEN_AT_REST_HASH);
        for (const [slot, expected] of AFTER_SHAKE_PAGES) {
          if (expected === null) {
            press(handle, slot);
            expect(handle.run(ANIMATED_PAGE_STEPS).lastInstructionFault).toBeUndefined();
          } else {
            previous = pressAndSettle(handle, slot, previous);
            expect(previous).toBe(expected);
          }
        }
        press(handle, 0); // START finishes Setup
        cap = handle.totalSteps() + REGISTERED_RESPONSE_STEPS;
        previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous]);
        expect(previous).toBe(OWN_ROLE_REGISTERED_HASH);

        press(handle, TO_LAUNCHER_SLOT);
        cap = handle.totalSteps() + LAUNCHER_RESPONSE_STEPS;
        expect(runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous])).toBe(LAUNCHER_HASH);
        expect(handle.consoleOutput()).toContain("launched Launcher");
      } finally {
        handle.dispose();
      }
    },
    600_000,
  );
});

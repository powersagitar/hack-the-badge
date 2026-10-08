import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { initSync, add, FirmwareEmulator } from "../src/cpu/wasm-pkg/emulator_wasm.js";
import {
  createFirmwareEmulator,
  initCpuWasmSync,
  type FirmwareEmulatorHandle,
} from "../src/cpu/bridge";
import { createProvisioner, type ProvisionState } from "../src/runtime/provisioner";

describe("emulator-wasm round trip", () => {
  test("Rust -> WASM -> TS add() matches native behavior", () => {
    const wasmBytes = readFileSync(
      new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
    );
    initSync({ module: wasmBytes });
    expect(add(2, 3)).toBe(5);
  });
});

/**
 * FNV-1a (64-bit) over each pixel's little-endian bytes, as boot_progress.rs's
 * framebuffer_fnv1a. Computed in 32-bit halves (BigInt per byte is too slow
 * for the provisioned twin's ~1,400 samples): the prime is 2^40 + 0x1b3, so
 * h * prime = h * 0x1b3 + (lo << 40).
 */
function fnv1a(fb: Uint16Array): bigint {
  let hi = 0xcbf29ce4;
  let lo = 0x84222325;
  const step = (b: number) => {
    lo = (lo ^ b) >>> 0;
    const p = lo * 0x1b3;
    hi = (Math.imul(hi, 0x1b3) + Math.floor(p / 0x1_0000_0000) + (lo << 8)) >>> 0;
    lo = p >>> 0;
  };
  for (const px of fb) {
    step(px & 0xff);
    step(px >> 8);
  }
  return (BigInt(hi) << 32n) | BigInt(lo);
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
  // handle), which also covers the generated glue directly.
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

/*
 * boot_progress.rs's Milestone 5 constants, copied (same names and values;
 * see that file for where each was measured). The finish line walks
 * OWN_ROLE from boot through provisioning and onboarding to the launcher.
 */
const FIRST_RUN_MAX_STEPS = 17_500_000;
const BOOT_STABLE_FOR = 1_000_000;
const AFTER_PRESS_STABLE_FOR = 8_000_000;
const PRESS_HOLD_STEPS = 1_600_000;
const PROVISION_DEADLINE = 38_000_000;
const ONBOARDING_RESPONSE_MARGIN = 16_000_000;
const OWN_ROLE = "hacker";
/** ROLE_ROWS' hacker row: onboarding page 1, and the registered screen. */
const OWN_ROLE_PAGE1_HASH = 0xd36476617b1a89b5n;
const OWN_ROLE_REGISTERED_HASH = 0xbd6d761308a7bf24n;
const TO_SHAKE_PAGE: Array<[number, bigint]> = [
  [1, 0xe788a78193281b08n], // A: page 2 "This is you"
  [1, 0x9621b8aa8b040d38n], // A: page 3 "Try every button"
  [1, 0xa283f1b3a862abffn], // A lit
  [2, 0x8522bf8dfccb6084n], // B
  [7, 0xb9e7b94ddd899859n], // UP
  [5, 0x362f61206bea4a4fn], // LEFT
  [6, 0x16eb495bb805251an], // RIGHT
  [4, 0x8a7a4dff645446d3n], // DOWN
  [8, 0xc6fba44d81322795n], // Aux1
  [3, 0x824afde3f915a562n], // HOME (captured on page 3)
  [0, 0x3a725bc10083f74an], // START: all nine done
  [1, 0xc1ad004ba5f77b78n], // A: page 4 "Lights"
  [1, 0x7598def30e838832n], // A: page 5 "Tap to unlock"
  [1, 0x4a54e5a40d669898n], // A: page 6 "Shake it!"
];
const SHAKE_HALF_STEPS = 1_000_000;
const SHAKEN_AT_REST_HASH = 0xa80a279b127e1bb1n;
/** `null`: page 8, animated, not pinned (see boot_progress.rs). */
const AFTER_SHAKE_PAGES: Array<[number, bigint | null]> = [
  [1, 0x110e8782bc4e5ff0n], // A: page 7 "Badge Connect"
  [1, null], // A: page 8 "Bump to connect", animated
  [1, 0x0831b6b675e6fe01n], // A: page 9 "HEAT SAFETY"
  [1, 0x21e9c7e1b05439e3n], // A: page 10 "Go explore"
  [1, 0x7dd45fd39362faacn], // A: page 11 "Badge rules", unticked
  [1, 0xe8c67af97ac7968en], // A: ticked, "Finished setup  Start"
];
const ANIMATED_PAGE_STEPS = 9_000_000;
const REGISTERED_RESPONSE_STEPS = AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
const TO_LAUNCHER_SLOT = 3;
const LAUNCHER_RESPONSE_STEPS = 33_000_000;
const LAUNCHER_HASH = 0xb694a61febfd62ffn;

/** boot_progress.rs's run_until_stable_frame (250,000-step chunks), over the bridge. */
function runUntilStable(
  handle: FirmwareEmulatorHandle,
  maxSteps: number,
  stableFor: number,
  skip: bigint[],
): bigint {
  let hash = fnv1a(handle.framebuffer());
  let since = handle.totalSteps();
  while (handle.totalSteps() < maxSteps) {
    expect(handle.run(250_000).lastInstructionFault).toBeUndefined();
    const fb = handle.framebuffer();
    const h = fnv1a(fb);
    if (h !== hash) {
      hash = h;
      since = handle.totalSteps();
    } else if (
      handle.totalSteps() - since >= stableFor &&
      fb.some((px) => px !== fb[0]) &&
      !skip.includes(h)
    ) {
      return hash;
    }
  }
  throw new Error(`no stable frame by ${maxSteps}; last hash ${hash.toString(16)}`);
}

function press(handle: FirmwareEmulatorHandle, slot: number): void {
  handle.setRawButton(slot, true);
  expect(handle.run(PRESS_HOLD_STEPS).lastInstructionFault).toBeUndefined();
  handle.setRawButton(slot, false);
}

/** boot_progress.rs's press_and_settle. */
function pressAndSettle(handle: FirmwareEmulatorHandle, slot: number, previous: bigint): bigint {
  press(handle, slot);
  const cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
  return runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [previous]);
}

describe("emulator-wasm provisioned boot", () => {
  // The WASM-path twin of boot_progress.rs's boots_to_launcher, provisioned
  // by the frontend's own provisioner (src/runtime/provisioner.ts) rather
  // than the Rust test helper, so it also proves the provisioner against
  // the real firmware. About 340M steps.
  test(
    "provisions the test identity through the console, walks onboarding, and reaches the same launcher as the native finish line",
    () => {
      initCpuWasmSync(
        readFileSync(new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url)),
      );
      const image = readFileSync(new URL("../public/firmware/factory.bin", import.meta.url));
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
          expect(handle.run(SHAKE_HALF_STEPS).lastInstructionFault).toBeUndefined();
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

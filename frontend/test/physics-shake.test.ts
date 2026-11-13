/**
 * Step 6 of Milestone 14 Task 9: the lanyard's sensor against the real
 * firmware's "Shake it!" page (Setup page 6), through the WASM twin.
 *
 * One walk (provision, then onboarding pages 1 to 6, as
 * test/cpu-wasm.test.ts walks them) serves both tests: the gentle drag
 * runs first and must leave the page unshaken, then the vigorous drag
 * must shake it. Every 1/60 s the lanyard steps one frame and its
 * reading goes to `setAcceleration`, and the emulator runs to the
 * matching emulated time.
 *
 * The verdict is A: once shaken, the page's footer becomes "A / START:
 * next" and A opens page 7 (`AFTER_SHAKE_PAGES[0]`); unshaken, A does
 * not. The frame alone cannot tell: the page prints the live `G-force`
 * and its `Peak`, so any motion changes it.
 */
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { createFirmwareEmulator, initCpuWasmSync, type FirmwareEmulatorHandle } from "../src/cpu/bridge";
import { createProvisioner, type ProvisionState } from "../src/runtime/provisioner";
import {
  AFTER_PRESS_STABLE_FOR,
  AFTER_SHAKE_PAGES,
  BOOT_STABLE_FOR,
  FIRST_RUN_HASH,
  FIRST_RUN_MAX_STEPS,
  fnv1a,
  HOLD_MAX_STEPS,
  ONBOARDING_RESPONSE_MARGIN,
  OWN_ROLE,
  OWN_ROLE_PAGE1_HASH,
  press,
  pressAndSettle,
  PROVISION_DEADLINE,
  runUntilStable,
  SPLASH_HASH,
  TO_SHAKE_PAGE,
} from "../src/bench/walk";
import { createLanyard, GAIN_MG_PER_PX_S2 } from "../src/motion/lanyard";
import type { Mg } from "../src/motion/source";

const FRAME_US = 1_000_000 / 60;
const PAGE7_HASH = AFTER_SHAKE_PAGES[0][1]!;
const A_SLOT = 1;

let handle: FirmwareEmulatorHandle;
let page6 = 0n;

/** Runs the emulator to `targetUs`, throwing on a fault or a missed target. */
function runTo(targetUs: number): void {
  const r = handle.runUntil(Math.round(targetUs), HOLD_MAX_STEPS);
  if (r.lastInstructionFault !== undefined) throw new Error("fault");
  if (handle.emulatedTimeUs() < Math.round(targetUs)) throw new Error("time not reached");
}

/**
 * Grabs the badge at its centre and moves the pointer ±`amp` px
 * horizontally at `hz` for 2 s, then lets go for 1 s; every frame's
 * reading drives the emulated accelerometer. Returns the readings.
 */
function scriptedDrag(amp: number, hz: number): Mg[] {
  const l = createLanyard({ width: 1000, height: 780, layout: "desktop" });
  const readings: Mg[] = [];
  l.start((mg) => {
    readings.push(mg);
    handle.setAcceleration(mg[0], mg[1], mg[2]);
  });
  const t0 = handle.emulatedTimeUs();
  let frame = 0;
  const tick = () => {
    l.step(FRAME_US / 1000);
    frame++;
    runTo(t0 + frame * FRAME_US);
  };
  for (let i = 0; i < 30; i++) tick(); // hang at rest for 0.5 s
  const b = l.pose().badge;
  const c = { x: b.x, y: b.y };
  if (!l.grab(c)) throw new Error("grab missed the badge");
  for (let i = 1; i <= 120; i++) {
    l.drag({ x: c.x + amp * Math.sin((2 * Math.PI * hz * i) / 60), y: c.y });
    tick();
  }
  l.release();
  for (let i = 0; i < 60; i++) tick();
  l.stop();
  return readings;
}

/** Presses A and returns the stable frame after it. */
function pressA(): bigint {
  const before = fnv1a(handle.framebuffer());
  press(handle, A_SLOT);
  const cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
  try {
    return runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [before]);
  } catch {
    return fnv1a(handle.framebuffer()); // nothing changed: still the page
  }
}

const peak = (rs: Mg[]) => Math.max(...rs.map((v) => Math.hypot(...v)));

describe("the lanyard shakes the real firmware", () => {
  beforeAll(() => {
    initCpuWasmSync(
      readFileSync(new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url)),
    );
    const image = readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url));
    handle = createFirmwareEmulator(new Uint8Array(image));
    if (runUntilStable(handle, FIRST_RUN_MAX_STEPS, BOOT_STABLE_FOR, [SPLASH_HASH]) !== FIRST_RUN_HASH) {
      throw new Error("first-run screen not reached");
    }
    const json = readFileSync(
      new URL(`../public/firmware/test-identities/${OWN_ROLE}.json`, import.meta.url),
      "utf8",
    );
    const p = createProvisioner(handle, json);
    let state: ProvisionState = p.poll();
    while ((state.kind === "waiting" || state.kind === "typing") && handle.totalSteps() < PROVISION_DEADLINE) {
      handle.run(50_000);
      state = p.poll();
    }
    if (state.kind !== "ok") throw new Error(`provisioning: ${state.kind}`);
    const cap = handle.totalSteps() + AFTER_PRESS_STABLE_FOR + ONBOARDING_RESPONSE_MARGIN;
    let previous = runUntilStable(handle, cap, AFTER_PRESS_STABLE_FOR, [FIRST_RUN_HASH]);
    if (previous !== OWN_ROLE_PAGE1_HASH) throw new Error("page 1 not reached");
    for (const [slot, expected] of TO_SHAKE_PAGE) {
      previous = pressAndSettle(handle, slot, previous);
      if (previous !== expected) throw new Error(`walk diverged at slot ${slot}`);
    }
    page6 = previous;
  }, 120_000);

  afterAll(() => handle?.dispose());

  test("a gentle drag does not", () => {
    expect(page6).toBe(TO_SHAKE_PAGE.at(-1)![1]);
    const readings = scriptedDrag(20, 1);
    console.log(`gentle drag: gain ${GAIN_MG_PER_PX_S2}, peak |a| ${Math.round(peak(readings))} mg`);
    expect(pressA()).not.toBe(PAGE7_HASH);
  }, 120_000);

  test("a vigorous drag passes Shake it!", () => {
    const readings = scriptedDrag(120, 3);
    console.log(`vigorous drag: gain ${GAIN_MG_PER_PX_S2}, peak |a| ${Math.round(peak(readings))} mg`);
    expect(pressA()).toBe(PAGE7_HASH);
  }, 120_000);
});

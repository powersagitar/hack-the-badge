/**
 * Thin TS wrapper over the generated wasm-bindgen glue in `wasm-pkg/`,
 * analogous in role to how `src/lua/vm.ts` wraps Fengari for Lua mode — much
 * narrower in scope, though: this is just the CPU/bus surface, not a general
 * value bridge.
 *
 * `wasm-pkg/` is **generated and gitignored**. Run `bun run build:wasm` after
 * any change under `emulator-core/` or `emulator-wasm/`, and before
 * `bun run dev` / `bun run build` / `bun test` / `bunx tsc --noEmit` —
 * nothing else regenerates it.
 *
 * ## What this layer adds over the raw glue
 *
 * 1. **One-time init, either way round.** wasm-bindgen's `--target web` output
 *    needs an explicit init before any export is callable: async (fetches the
 *    `.wasm` next to the JS module) in a browser, or synchronous (from bytes
 *    you already have) anywhere `fetch` isn't appropriate — which is how
 *    `bun test` uses it. Both are exposed, and both are idempotent.
 * 2. **No leaked WASM handles.** `RunReport` is a wasm-bindgen *class*: it owns
 *    memory on the Rust side that only `free()` releases. A render loop calling
 *    `run()` 60 times a second and dropping the result would leak steadily, so
 *    [`FirmwareEmulatorHandle.run`] copies the fields into a plain object and
 *    frees the handle immediately. Callers never see a `RunReport`.
 * 3. **A named interface** the rest of the app depends on instead of the
 *    generated `.d.ts`, so `src/runtime/firmware-runtime.ts` can be unit-tested
 *    against a hand-written fake with no WASM involved.
 *
 * ## Framebuffer aliasing safety
 *
 * `framebuffer()` returns a fresh, JS-owned `Uint16Array` that has no
 * relationship to WASM linear memory — safe to keep, safe to hold across a
 * `run()` call. That is a property of the Rust signature (`-> Vec<u16>`, an
 * *owned* vector), not a convention this file maintains: wasm-bindgen's glue
 * for an owned vector return copies the bytes out (`.slice()`) and frees the
 * Rust allocation before returning. A borrowed `&[u16]` return would instead
 * have produced a *view*, which any later WASM heap growth detaches — see
 * `emulator-wasm/src/lib.rs`'s module doc for the full argument.
 *
 * This module therefore deliberately does **not** cache or re-wrap the array,
 * because there is nothing to defend against; it just passes it through to
 * `blitFramebuffer`.
 */
import init, {
  initSync,
  FirmwareEmulator,
  type RunReport,
} from "./wasm-pkg/emulator_wasm.js";

/** Plain-data form of the WASM `RunReport` — see this module's doc for why. */
export interface CpuRunSummary {
  /** Instruction steps executed. */
  steps: number;
  /** How many of those steps took a trap instead of completing an instruction. */
  traps: number;
  /** How many of those steps intercepted a mask-ROM HLE stub. */
  romStubCalls: number;
  /** `pc` after the run. */
  pc: number;
  /**
   * Faulting address of the most recent unmapped instruction fetch, or
   * `undefined` if there wasn't one. Anything other than `undefined` normally
   * means a mask-ROM function that still needs a stub — look the address up in
   * ESP-IDF's `esp32c3.rom*.ld` and see `emulator-core/src/rom.rs`.
   */
  lastInstructionFault: number | undefined;
}

/** The CPU-emulator surface `src/runtime/firmware-runtime.ts` programs against. */
export interface FirmwareEmulatorHandle {
  /** Runs `budget` instruction steps. */
  run(budget: number): CpuRunSummary;
  /** Re-boots from the same image, keeping held buttons. */
  reset(): void;
  /**
   * Presses/releases a raw button slot. Slot 0 is `PIN_START` (GPIO9); slots
   * 1..8 are the 74HC165's bits. The human-name→slot table lives in
   * `src/runtime/firmware-runtime.ts`.
   */
  setRawButton(slot: number, pressed: boolean): void;
  /** Current framebuffer: RGB565, row-major, `width * height` elements. */
  framebuffer(): Uint16Array;
  readonly screenWidth: number;
  readonly screenHeight: number;
  /** Current `pc`, for a diagnostics HUD. */
  pc(): number;
  /** Steps run since construction or the last `reset()`. */
  totalSteps(): number;
  /** Releases the WASM-side emulator. Calling anything else afterwards throws. */
  dispose(): void;
}

let initPromise: Promise<unknown> | null = null;
let ready = false;

/**
 * Initializes the WASM module by fetching `emulator_wasm_bg.wasm` from next to
 * the generated JS (the browser path). Safe to call repeatedly — concurrent
 * callers share one in-flight promise.
 */
export async function initCpuWasm(): Promise<void> {
  if (ready) return;
  initPromise ??= init();
  await initPromise;
  ready = true;
}

/**
 * Initializes the WASM module from bytes already in hand, synchronously — for
 * `bun test` and any other non-browser caller, where there's no meaningful
 * `fetch` for the glue's default path to use. Idempotent.
 */
export function initCpuWasmSync(wasmBytes: BufferSource): void {
  if (ready) return;
  initSync({ module: wasmBytes });
  ready = true;
}

/** `true` once either initializer has completed. */
export function isCpuWasmReady(): boolean {
  return ready;
}

/**
 * Boots `image` (the bytes of `public/firmware/factory.bin`) on the emulated
 * ESP32-C3. Throws if the WASM module hasn't been initialized, or if the bytes
 * aren't a parseable ESP-IDF app image.
 */
export function createFirmwareEmulator(image: Uint8Array): FirmwareEmulatorHandle {
  if (!ready) {
    throw new Error(
      "cpu/bridge: initCpuWasm() (or initCpuWasmSync()) must complete before creating an emulator",
    );
  }

  const wasm = new FirmwareEmulator(image);
  const screenWidth = wasm.screenWidth;
  const screenHeight = wasm.screenHeight;
  let disposed = false;

  function assertLive(): void {
    if (disposed) throw new Error("cpu/bridge: emulator has been disposed");
  }

  return {
    screenWidth,
    screenHeight,
    run(budget: number): CpuRunSummary {
      assertLive();
      const report: RunReport = wasm.run(budget);
      try {
        return {
          steps: report.steps,
          traps: report.traps,
          romStubCalls: report.romStubCalls,
          pc: report.pc,
          lastInstructionFault: report.lastInstructionFault,
        };
      } finally {
        // See this module's doc: `RunReport` is a WASM-owned handle, and a
        // per-frame caller would otherwise leak one every frame.
        report.free();
      }
    },
    reset(): void {
      assertLive();
      wasm.reset();
    },
    setRawButton(slot: number, pressed: boolean): void {
      assertLive();
      wasm.setButton(slot, pressed);
    },
    framebuffer(): Uint16Array {
      assertLive();
      return wasm.framebuffer();
    },
    pc(): number {
      assertLive();
      return wasm.pc;
    },
    totalSteps(): number {
      assertLive();
      return wasm.totalSteps;
    },
    dispose(): void {
      if (disposed) return;
      disposed = true;
      wasm.free();
    },
  };
}

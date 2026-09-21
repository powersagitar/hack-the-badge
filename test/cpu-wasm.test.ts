import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { initSync, add } from "../src/cpu/wasm-pkg/emulator_wasm.js";

describe("emulator-wasm round trip", () => {
  test("Rust -> WASM -> TS add() matches native behavior", () => {
    const wasmBytes = readFileSync(
      new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url),
    );
    initSync({ module: wasmBytes });
    expect(add(2, 3)).toBe(5);
  });
});

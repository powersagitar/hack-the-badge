/**
 * `bun run bench:wasm`: the WASM redraw benchmark under Bun
 * (JavaScriptCore). Runs `runRedrawBench` (src/bench/redraw-bench.ts) three
 * times, each on a freshly booted emulator, and prints each run's lines and
 * a median summary. Not part of `bun test`. Needs `bun run build:wasm`.
 */
import { readFileSync } from "node:fs";
import os from "node:os";
import { createFirmwareEmulator, initCpuWasmSync } from "../src/cpu/bridge";
import { type BenchResult, runRedrawBench, summarizeRuns } from "../src/bench/redraw-bench";
import { OWN_ROLE } from "../src/bench/walk";

const RUNS = 3;

/** The commit (with `+dirty` for uncommitted changes) and the release opt-level. */
export function buildInfo(): string {
  const git = (...args: string[]) =>
    Bun.spawnSync(["git", ...args], { cwd: import.meta.dir }).stdout.toString().trim();
  const dirty = git("status", "--porcelain") === "" ? "" : "+dirty";
  const cargo = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
  const opt = /\[profile\.release\][^[]*?opt-level\s*=\s*("?\w+"?)/.exec(cargo)?.[1] ?? "default";
  return `commit=${git("rev-parse", "--short", "HEAD")}${dirty} profile=release opt-level=${opt}`;
}

if (import.meta.main) {
  initCpuWasmSync(
    readFileSync(new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url)),
  );
  const image = new Uint8Array(
    readFileSync(new URL("../public/firmware/firmware.bin", import.meta.url)),
  );
  const identity = readFileSync(
    new URL(`../public/firmware/test-identities/${OWN_ROLE}.json`, import.meta.url),
    "utf8",
  );
  const host = `Bun ${Bun.version} (JavaScriptCore), ${os.cpus()[0]?.model ?? "?"}, ${os.platform()} ${os.arch()}`;
  console.log(`BENCH host="${host}" ${buildInfo()}`);
  const results: BenchResult[] = [];
  for (let run = 1; run <= RUNS; run++) {
    console.log(`run ${run}/${RUNS}`);
    const handle = createFirmwareEmulator(image);
    try {
      results.push(runRedrawBench(handle, identity, () => performance.now(), console.log));
    } finally {
      handle.dispose();
    }
  }
  for (const line of summarizeRuns(results)) console.log(line);
}

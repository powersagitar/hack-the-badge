// Regression guard for the dev server's "process is not defined" (Milestone
// 7, Task 9a). Vite's dev dependency pre-bundle defines only
// process.env.NODE_ENV, so fengari's luaconf.js, which reads
// process.env.FENGARICONF at module top level, threw in the browser.
// vite.config.ts gives the pre-bundle the production build's
// `process.env` -> `{}` define. This test runs the real dep optimizer with
// that config into a temporary cache directory and evaluates the pre-bundled
// fengari in a context with no `process` global, as a browser would.
import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import vm from "node:vm";
import { optimizeDeps, resolveConfig } from "vite";

const FRONTEND = resolve(import.meta.dir, "..");

test("pre-bundled fengari evaluates without a process global", async () => {
  const cacheDir = mkdtempSync(join(tmpdir(), "htb-vite-deps-"));
  try {
    const config = await resolveConfig(
      {
        root: FRONTEND,
        configFile: join(FRONTEND, "vite.config.ts"),
        cacheDir,
        logLevel: "silent",
        optimizeDeps: { include: ["fengari"], noDiscovery: true },
      },
      "serve",
    );
    // Vite's deprecated manual entry point runs the same optimizer the dev
    // server does (prepareRolldownOptimizerRun), with the config's
    // optimizeDeps.rolldownOptions.
    await optimizeDeps(config, true);
    let src = readFileSync(join(cacheDir, "deps", "fengari.js"), "utf8");

    // A boolean, so a failure does not print the 1.7 MB bundle.
    expect(/\bprocess\.env\b/.test(src)).toBe(false);

    src = src
      .replace(/\/\/# sourceMappingURL=.*$/m, "")
      .replace(/^export default (.*);$/m, "globalThis.__fengari = $1;");
    const ctx = vm.createContext({ TextEncoder, TextDecoder });
    vm.runInContext(src, ctx);
    const fengari = (ctx as { __fengari?: Record<string, unknown> }).__fengari;
    expect(vm.runInContext("typeof process", ctx)).toBe("undefined");
    // luaconf resolves as in the production build: FENGARICONF is unset.
    expect(fengari?.luaconf).toBeDefined();
    expect(Object.keys(fengari ?? {})).toEqual(
      expect.arrayContaining(["lua", "lauxlib", "lualib", "to_luastring"]),
    );
  } finally {
    rmSync(cacheDir, { recursive: true, force: true });
  }
});

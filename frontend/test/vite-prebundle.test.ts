// Regression guard for the dev server's "process is not defined" (Milestone
// 7, Task 9a). Vite's dev dependency pre-bundle defines only
// process.env.NODE_ENV, so a browser dependency that reads `process` at
// module top level throws in the browser (fengari's luaconf.js did, until
// the Lua sandbox was removed in Milestone 14 Task 4). This test runs the
// real dep optimizer with vite.config.ts into a temporary cache directory
// and evaluates the pre-bundled valibot, the page's remaining browser
// dependency, in a context with no `process` global, as a browser would.
import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import vm from "node:vm";
import { optimizeDeps, resolveConfig } from "vite";

const FRONTEND = resolve(import.meta.dir, "..");

test("pre-bundled valibot evaluates without a process global", async () => {
  const cacheDir = mkdtempSync(join(tmpdir(), "htb-vite-deps-"));
  try {
    const config = await resolveConfig(
      {
        root: FRONTEND,
        configFile: join(FRONTEND, "vite.config.ts"),
        cacheDir,
        logLevel: "silent",
        optimizeDeps: { include: ["valibot"], noDiscovery: true },
      },
      "serve",
    );
    // Vite's deprecated manual entry point runs the same optimizer the dev
    // server does (prepareRolldownOptimizerRun), with the resolved config.
    await optimizeDeps(config, true);
    let src = readFileSync(join(cacheDir, "deps", "valibot.js"), "utf8");

    // A boolean, so a failure does not print the bundle.
    expect(/\bprocess\.env\b/.test(src)).toBe(false);

    // valibot is ESM with named exports: collect them onto a global.
    src = src
      .replace(/\/\/# sourceMappingURL=.*$/m, "")
      .replace(/^export\s*\{([^}]*)\};?\s*$/m, (_, names: string) => {
        const props = names
          .split(",")
          .map((n) => n.trim())
          .filter(Boolean)
          .map((n) => {
            const [local, exported] = n.split(/\s+as\s+/);
            return `${JSON.stringify(exported ?? local)}: ${local}`;
          });
        return `globalThis.__valibot = { ${props.join(", ")} };`;
      });
    const ctx = vm.createContext({});
    vm.runInContext(src, ctx);
    const valibot = (ctx as { __valibot?: Record<string, unknown> }).__valibot;
    expect(vm.runInContext("typeof process", ctx)).toBe("undefined");
    expect(Object.keys(valibot ?? {})).toEqual(expect.arrayContaining(["object", "string", "safeParse"]));
  } finally {
    rmSync(cacheDir, { recursive: true, force: true });
  }
});

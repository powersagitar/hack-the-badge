import { describe, expect, test } from "bun:test";
import {
  createSandboxedEnv,
  runMainScript,
  MAX_MAIN_SOURCE_BYTES,
  MAX_REQUIRE_DEPTH,
  MAX_REQUIRE_MODULES,
  type FileLoader,
} from "../src/lua/vm";

function makeLoader(files: Record<string, string>): FileLoader {
  return (path: string) => (Object.prototype.hasOwnProperty.call(files, path) ? files[path] : null);
}

function run(source: string, files: Record<string, string> = {}): void {
  const env = createSandboxedEnv("/app", makeLoader(files));
  runMainScript(env, source);
}

describe("sandbox global shape", () => {
  test("allowed libraries are present with the right kind", () => {
    expect(() =>
      run(`
        assert(type(table) == "table", "table missing")
        assert(type(string) == "table", "string missing")
        assert(type(math) == "table", "math missing")
        assert(type(utf8) == "table", "utf8 missing")
        assert(type(require) == "function", "require missing")
      `),
    ).not.toThrow();
  });

  test("forbidden libraries are entirely absent", () => {
    expect(() =>
      run(`
        assert(os == nil, "os should not exist")
        assert(io == nil, "io should not exist")
        assert(package == nil, "package should not exist")
        assert(debug == nil, "debug should not exist")
        assert(coroutine == nil, "coroutine should not exist")
      `),
    ).not.toThrow();
  });

  test("base entry points that are normally present get nil'd out", () => {
    expect(() =>
      run(`
        assert(dofile == nil, "dofile should be nil")
        assert(loadfile == nil, "loadfile should be nil")
        assert(load == nil, "load should be nil")
        assert(pcall == nil, "pcall should be nil")
        assert(xpcall == nil, "xpcall should be nil")
        assert(setmetatable == nil, "setmetatable should be nil")
      `),
    ).not.toThrow();
  });

  test("ordinary base functions that ARE allowed still work", () => {
    expect(() =>
      run(`
        assert(type(1) == "number")
        assert(tostring(5) == "5")
        assert(tonumber("5") == 5)
        assert(assert(true) == true)
        local t = {1, 2, 3}
        local n = 0
        for _ in ipairs(t) do n = n + 1 end
        assert(n == 3)
      `),
    ).not.toThrow();
  });
});

describe("require()", () => {
  test("loads <appdir>/<pkg>/<mod>.lua for a dotted module name", () => {
    expect(() =>
      run(
        `
          local greet = require("pkg.mod")
          assert(greet == "hello from pkg/mod")
        `,
        { "pkg/mod.lua": `return "hello from pkg/mod"` },
      ),
    ).not.toThrow();
  });

  test("caches modules: requiring twice does not re-execute the module body", () => {
    expect(() =>
      run(
        `
          local a = require("counter")
          local b = require("counter")
          assert(load_count == 1, "expected load_count == 1, got " .. tostring(load_count))
          assert(a == b, "expected the same cached table both times")
        `,
        { "counter.lua": `load_count = (load_count or 0) + 1\nreturn { tag = "counter" }` },
      ),
    ).not.toThrow();
  });

  test("detects a require cycle and errors instead of hanging", () => {
    const files = {
      "cyc_a.lua": `return require("cyc_b")`,
      "cyc_b.lua": `return require("cyc_a")`,
    };
    expect(() => run(`require("cyc_a")`, files)).toThrow(/cycle detected/);
  });

  test("enforces the require-depth-8 limit on a genuinely deep chain", () => {
    // d0 -> d1 -> d2 -> ... -> d9 (10 links): must blow the depth-8 cap.
    const files: Record<string, string> = {};
    const depth = 10;
    for (let i = 0; i < depth; i++) {
      files[`d${i}.lua`] = i === depth - 1 ? `return "leaf"` : `return require("d${i + 1}")`;
    }
    expect(() => run(`require("d0")`, files)).toThrow(/max require depth/);
  });

  test("a chain within the depth limit loads fine", () => {
    const files: Record<string, string> = {};
    const depth = MAX_REQUIRE_DEPTH; // exactly at the cap should still succeed
    for (let i = 0; i < depth; i++) {
      files[`s${i}.lua`] = i === depth - 1 ? `return "leaf"` : `return require("s${i + 1}")`;
    }
    expect(() => run(`local v = require("s0"); assert(v == "leaf")`, files)).not.toThrow();
  });

  test("enforces the 16-modules-per-app cap on a genuinely wide require graph", () => {
    const files: Record<string, string> = {};
    const count = MAX_REQUIRE_MODULES + 1; // 17 distinct modules
    const requires: string[] = [];
    for (let i = 0; i < count; i++) {
      files[`w${i}.lua`] = `return ${i}`;
      requires.push(`require("w${i}")`);
    }
    expect(() => run(requires.join("\n"), files)).toThrow(/too many modules/);
  });

  test("exactly MAX_REQUIRE_MODULES distinct modules loads fine", () => {
    const files: Record<string, string> = {};
    const requires: string[] = [];
    for (let i = 0; i < MAX_REQUIRE_MODULES; i++) {
      files[`v${i}.lua`] = `return ${i}`;
      requires.push(`require("v${i}")`);
    }
    expect(() => run(requires.join("\n"), files)).not.toThrow();
  });

  test("a missing module produces a clear error, not a crash", () => {
    expect(() => run(`require("does.not.exist")`)).toThrow(/not found/);
  });
});

describe("main.lua size cap", () => {
  test("source over 64 KiB is rejected", () => {
    const big = "-- " + "a".repeat(MAX_MAIN_SOURCE_BYTES + 1);
    const env = createSandboxedEnv("/app", makeLoader({}));
    expect(() => runMainScript(env, big)).toThrow(/exceeding/);
  });

  test("source right at the cap is accepted", () => {
    // Build a comment line padded to exactly MAX_MAIN_SOURCE_BYTES bytes.
    const prefix = "-- ";
    const padded = prefix + "a".repeat(MAX_MAIN_SOURCE_BYTES - prefix.length);
    expect(padded.length).toBe(MAX_MAIN_SOURCE_BYTES);
    const env = createSandboxedEnv("/app", makeLoader({}));
    expect(() => runMainScript(env, padded)).not.toThrow();
  });
});

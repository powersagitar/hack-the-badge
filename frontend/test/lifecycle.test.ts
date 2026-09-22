import { describe, expect, test } from "bun:test";
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { lua, to_luastring } from "fengari";
import { luaToJs } from "../src/lua/interop";
import { BUTTON_IDS, KIND } from "../src/badge/input";
import { getStorage } from "../src/badge/storage";
import { createAppRuntime, loadApp, type AppManifest, type AppRuntime } from "../src/runtime/lifecycle";
import type { FileLoader } from "../src/lua/vm";

function makeLoader(files: Record<string, string>): FileLoader {
  return (path: string) => (Object.prototype.hasOwnProperty.call(files, path) ? files[path] : null);
}

function getLuaGlobal(runtime: AppRuntime, name: string): unknown {
  const { L } = runtime.env;
  lua.lua_getglobal(L, to_luastring(name));
  const v = luaToJs(L, -1);
  lua.lua_pop(L, 1);
  return v;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const BASE_MANIFEST: AppManifest = { slug: "lifecycle_test_app", name: "Lifecycle Test" };

describe("runtime/lifecycle end-to-end (fake in-memory fileLoader)", () => {
  test("loadApp calls on_enter exactly once, even if start() is called twice", () => {
    const files = {
      "manifest.cfg": "slug=enter_once\nname=Enter Once",
      "main.lua": `
        enter_count = 0
        function on_enter(root)
          enter_count = enter_count + 1
        end
      `,
    };
    const runtime = loadApp("/app", makeLoader(files));
    try {
      runtime.start(1000);
      expect(getLuaGlobal(runtime, "enter_count")).toBe(1);
      runtime.start(1000); // calling start() again must not re-fire on_enter
      expect(getLuaGlobal(runtime, "enter_count")).toBe(1);
    } finally {
      runtime.stop();
    }
  });

  test("the tick loop invokes on_tick", async () => {
    const runtime = createAppRuntime(
      BASE_MANIFEST,
      "/app",
      makeLoader({
        "main.lua": `
          tick_count = 0
          function on_tick()
            tick_count = tick_count + 1
          end
        `,
      }),
    );
    try {
      runtime.start(5);
      await sleep(60);
      const count = getLuaGlobal(runtime, "tick_count") as number;
      expect(count).toBeGreaterThan(0);
    } finally {
      runtime.stop();
    }
  });

  test("injecting a button press invokes on_button with the right args", async () => {
    const runtime = createAppRuntime(
      BASE_MANIFEST,
      "/app",
      makeLoader({
        "main.lua": `
          last_button = nil
          last_kind = nil
          function on_button(button, kind)
            last_button = button
            last_kind = kind
          end
        `,
      }),
    );
    try {
      runtime.start(5);
      runtime.injectButton("A", "pressed");
      await sleep(30);
      expect(getLuaGlobal(runtime, "last_button")).toBe(BUTTON_IDS.A);
      expect(getLuaGlobal(runtime, "last_kind")).toBe(KIND.PRESSED);

      runtime.injectButton("A", "released");
      await sleep(30);
      expect(getLuaGlobal(runtime, "last_button")).toBe(BUTTON_IDS.A);
      expect(getLuaGlobal(runtime, "last_kind")).toBe(KIND.RELEASED);
    } finally {
      runtime.stop();
    }
  });

  test("on_exit fires when the app is unloaded via stop()", () => {
    const runtime = createAppRuntime(
      BASE_MANIFEST,
      "/app",
      makeLoader({
        "main.lua": `
          exit_count = 0
          function on_exit()
            exit_count = exit_count + 1
          end
        `,
      }),
    );
    runtime.start(1000);
    expect(getLuaGlobal(runtime, "exit_count")).toBe(0);
    runtime.stop();
    expect(getLuaGlobal(runtime, "exit_count")).toBe(1);
  });

  test("3 consecutive on_tick failures suspend further ticks", async () => {
    const runtime = createAppRuntime(
      BASE_MANIFEST,
      "/app",
      makeLoader({
        "main.lua": `
          tick_calls = 0
          function on_tick()
            tick_calls = tick_calls + 1
            error("on_tick always fails")
          end
        `,
      }),
    );
    const originalConsoleError = console.error;
    console.error = () => {}; // keep test output clean; the errors are expected
    try {
      runtime.start(5);
      await sleep(150); // many more than 3 tick intervals worth of time
      const calls = getLuaGlobal(runtime, "tick_calls") as number;
      expect(calls).toBe(3);

      // Confirm it really has stopped, not just slowed: wait again and
      // verify the count did not move.
      await sleep(100);
      const callsAfterMore = getLuaGlobal(runtime, "tick_calls") as number;
      expect(callsAfterMore).toBe(3);
    } finally {
      console.error = originalConsoleError;
      runtime.stop();
    }
  });

  test("1 on_button failure suspends further button dispatch", async () => {
    const runtime = createAppRuntime(
      BASE_MANIFEST,
      "/app",
      makeLoader({
        "main.lua": `
          button_calls = 0
          function on_button(button, kind)
            button_calls = button_calls + 1
            error("on_button always fails")
          end
        `,
      }),
    );
    const originalConsoleError = console.error;
    console.error = () => {};
    try {
      runtime.start(5);
      runtime.injectButton("A", "pressed");
      await sleep(30);
      expect(getLuaGlobal(runtime, "button_calls")).toBe(1);

      runtime.injectButton("B", "pressed");
      await sleep(30);
      // Dispatch should now be suspended: still just the 1 call from before.
      expect(getLuaGlobal(runtime, "button_calls")).toBe(1);
    } finally {
      console.error = originalConsoleError;
      runtime.stop();
    }
  });

  // Regression test for a code-review finding: stop() used to leave
  // `entered`/tick-and-button-suspension state untouched, so restarting the
  // same AppRuntime after a stop() silently skipped on_enter (since
  // `entered` was still true) and could inherit a stale tick suspension from
  // before the stop. stop() now resets this state so start() after stop()
  // behaves like a fresh run.
  test("stop() then start() again re-fires on_enter and clears a prior tick suspension", async () => {
    const runtime = createAppRuntime(
      BASE_MANIFEST,
      "/app",
      makeLoader({
        "main.lua": `
          enter_count = 0
          fail_tick = true
          tick_calls = 0
          function on_enter(root)
            enter_count = enter_count + 1
          end
          function on_tick()
            tick_calls = tick_calls + 1
            if fail_tick then error("on_tick fails until told otherwise") end
          end
        `,
      }),
    );
    const originalConsoleError = console.error;
    console.error = () => {};
    try {
      runtime.start(5);
      expect(getLuaGlobal(runtime, "enter_count")).toBe(1);
      await sleep(60); // long enough to exhaust the 3-failure suspension
      expect(getLuaGlobal(runtime, "tick_calls")).toBe(3);
      runtime.stop();

      // Flip the Lua-side flag so ticks would succeed if actually dispatched,
      // then restart: on_enter must re-fire, and ticking must resume (not
      // stay suspended from before the stop()).
      runtime.start(5);
      expect(getLuaGlobal(runtime, "enter_count")).toBe(2);
      const L = runtime.env.L;
      lua.lua_pushboolean(L, false);
      lua.lua_setglobal(L, to_luastring("fail_tick"));
      await sleep(60);
      const callsAfterRestart = getLuaGlobal(runtime, "tick_calls") as number;
      expect(callsAfterRestart).toBeGreaterThan(3);
    } finally {
      console.error = originalConsoleError;
      runtime.stop();
    }
  });

  test("parseManifest + createAppRuntime together via loadApp with a synthetic app", () => {
    const files = {
      "manifest.cfg": "slug=synthetic\nname=Synthetic App",
      "main.lua": `did_run = true`,
    };
    const runtime = loadApp("/app", makeLoader(files));
    expect(runtime.manifest.slug).toBe("synthetic");
    expect(runtime.manifest.name).toBe("Synthetic App");
    expect(getLuaGlobal(runtime, "did_run")).toBe(true);
    runtime.stop();
  });

  test("loadApp throws a clear error when manifest.cfg is missing", () => {
    expect(() => loadApp("/app", makeLoader({ "main.lua": "" }))).toThrow(/manifest\.cfg not found/);
  });

  test("createAppRuntime throws a clear error when main.lua is missing", () => {
    expect(() =>
      createAppRuntime(BASE_MANIFEST, "/app", makeLoader({})),
    ).toThrow(/main\.lua not found/);
  });
});

describe("integration: the real public/apps/smoke-test app", () => {
  test("loads and runs end-to-end through loadApp", async () => {
    const appDir = join(import.meta.dir, "..", "public", "apps", "smoke-test");
    expect(existsSync(join(appDir, "main.lua"))).toBe(true);
    expect(existsSync(join(appDir, "manifest.cfg"))).toBe(true);

    const fileLoader: FileLoader = (path: string) => {
      const full = join(appDir, path);
      if (!existsSync(full)) return null;
      return readFileSync(full, "utf8");
    };

    // The smoke-test app is scoped to slug "smoke_test" in badge.store; clear
    // shared in-memory storage first so this test doesn't depend on state
    // left over from a previous run.
    getStorage().clear();

    const runtime = loadApp(appDir, fileLoader);
    try {
      expect(runtime.manifest.slug).toBe("smoke_test");
      expect(runtime.manifest.name).toBe("Smoke Test");

      runtime.start(5);
      // on_enter should have already built the widget tree synchronously.
      expect(runtime.ui.root.children.length).toBeGreaterThan(0);

      runtime.injectButton("A", "pressed");
      await sleep(30);

      const [r, g, b] = runtime.led.getColors()[0]!;
      expect([r, g, b]).toEqual([0, 255, 80]);

      await sleep(20);
    } finally {
      runtime.stop();
    }
  });
});

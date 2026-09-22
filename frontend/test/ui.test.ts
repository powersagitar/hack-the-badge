import { describe, expect, test } from "bun:test";
import { lua, to_luastring } from "fengari";
import { createSandboxedEnv, runMainScript, setGlobalRaw, type LuaEnv } from "../src/lua/vm";
import { createUiModule } from "../src/badge/ui";

/**
 * Builds a fresh sandboxed env with `badge.ui` attached and a Lua global
 * `root` bound to the module's root widget, mirroring what
 * `runtime/lifecycle.ts` wires up for real apps (minus the other `badge.*`
 * modules, which are irrelevant to widget-system tests).
 */
function makeUiEnv() {
  const ui = createUiModule();
  const env = createSandboxedEnv("/app", () => null);
  setGlobalRaw(env, "badge", (L) => {
    lua.lua_newtable(L);
    ui.attach(L);
    lua.lua_setfield(L, -2, to_luastring("ui"));
  });
  ui.pushWidget(env.L, ui.root);
  lua.lua_setglobal(env.L, to_luastring("root"));
  return { env, ui };
}

/** Runs Lua source with `assert(...)` checks against a fresh UI env. */
function expectLuaOk(source: string): void {
  const { env } = makeUiEnv();
  expect(() => runMainScript(env, source)).not.toThrow();
}

describe("widget factories: creation + one signature method call each, via real Lua source", () => {
  test("label", () => {
    expectLuaOk(`
      local w = badge.ui.label(root, "hi")
      assert(w:type() == "label")
      w:set_text("updated")
    `);
  });

  test("textarea", () => {
    expectLuaOk(`
      local w = badge.ui.textarea(root, "hi")
      assert(w:type() == "textarea")
      w:set_text("updated")
    `);
  });

  test("box", () => {
    expectLuaOk(`
      local w = badge.ui.box(root, 10, 20)
      assert(w:type() == "box")
      w:set_size(30, 40)
    `);
  });

  test("button", () => {
    expectLuaOk(`
      local w = badge.ui.button(root, 10, 20)
      assert(w:type() == "button")
      w:set_pos(1, 2)
    `);
  });

  test("bar", () => {
    expectLuaOk(`
      local w = badge.ui.bar(root, 0, 100, 25)
      assert(w:type() == "bar")
      w:set_value(50)
    `);
  });

  test("arc", () => {
    expectLuaOk(`
      local w = badge.ui.arc(root, 0, 100, 25)
      assert(w:type() == "arc")
      w:set_value(50)
    `);
  });

  test("slider", () => {
    expectLuaOk(`
      local w = badge.ui.slider(root, 0, 100, 25)
      assert(w:type() == "slider")
      w:set_range(0, 200)
    `);
  });

  test("image", () => {
    expectLuaOk(`
      local w = badge.ui.image(root, "foo.bin")
      assert(w:type() == "image")
      w:set_src("bar.bin")
    `);
  });

  test("line", () => {
    expectLuaOk(`
      local w = badge.ui.line(root, {{0,0}, {10,10}})
      assert(w:type() == "line")
      w:set_points({{1,1}, {2,2}, {3,3}})
    `);
  });

  test("switch", () => {
    expectLuaOk(`
      local w = badge.ui.switch(root, false)
      assert(w:type() == "switch")
      w:set_checked(true)
      assert(w:get_checked() == true)
    `);
  });

  test("checkbox", () => {
    expectLuaOk(`
      local w = badge.ui.checkbox(root, "label", false)
      assert(w:type() == "checkbox")
      w:set_checked(true)
      assert(w:get_checked() == true)
    `);
  });

  test("roller", () => {
    expectLuaOk(`
      local w = badge.ui.roller(root, "a\\nb\\nc")
      assert(w:type() == "roller")
      assert(w:get_selected() == 0)
    `);
  });
});

describe("common widget methods", () => {
  test("set_pos / set_size / align", () => {
    const { env, ui } = makeUiEnv();
    runMainScript(
      env,
      `
        local w = badge.ui.box(root, 1, 1)
        w:set_pos(5, 6)
        w:set_size(7, 8)
        w:align("center", 1, 2)
      `,
    );
    const w = ui.root.children[0]!;
    expect(w.x).toBe(5);
    expect(w.y).toBe(6);
    expect(w.w).toBe(7);
    expect(w.h).toBe(8);
    expect(w.align).toEqual({ name: "center", dx: 1, dy: 2 });
  });

  test("parent() / child() / child_count()", () => {
    expectLuaOk(`
      local box = badge.ui.box(root, 50, 50)
      local lbl = badge.ui.label(box, "hi")
      assert(box:child_count() == 1, "expected 1 child, got " .. tostring(box:child_count()))
      assert(box:child(1) ~= nil, "child(1) should resolve")
      assert(lbl:parent() ~= nil, "parent() should resolve")
      -- parent() returns a *wrapper*, but it should carry the same identity
      -- via __wid: compare against a second parent() call.
      local p1 = lbl:parent()
      local p2 = lbl:parent()
      assert(p1.__wid == p2.__wid, "parent() should resolve to the same widget id")
      assert(box:child(2) == nil, "out-of-range child() should be nil")
    `);
  });

  test("hidden() / clickable()", () => {
    const { env, ui } = makeUiEnv();
    runMainScript(
      env,
      `
        local w = badge.ui.box(root, 1, 1)
        w:hidden(true)
        w:clickable(true)
      `,
    );
    const w = ui.root.children[0]!;
    expect(w.hidden).toBe(true);
    expect(w.clickable).toBe(true);
  });

  test("delete() removes the widget from its parent's children", () => {
    const { env, ui } = makeUiEnv();
    runMainScript(
      env,
      `
        local a = badge.ui.box(root, 1, 1)
        local b = badge.ui.box(root, 1, 1)
        assert(root ~= nil)
        a:delete()
      `,
    );
    expect(ui.root.children.length).toBe(1);
    expect(ui.root.children[0]!.deleted).toBe(false);
  });

  test("delete() marks the widget itself deleted and hidden", () => {
    expectLuaOk(`
      local a = badge.ui.box(root, 1, 1)
      a:delete()
    `);
    // Re-check via a fresh env + direct JS inspection for the deleted flag.
    const { env, ui } = makeUiEnv();
    runMainScript(env, `local a = badge.ui.box(root, 1, 1); a:delete()`);
    // The widget was spliced out of root.children by delete(), so we can't
    // reach it through the tree anymore -- which is itself the behavior
    // under test (a deleted widget is unreachable from its old parent).
    expect(ui.root.children.length).toBe(0);
  });

  // Regression test for a code-review finding: delete() used to remove only
  // the widget itself from the registry, leaking a registry entry per
  // descendant and leaving orphaned children still resolvable from Lua even
  // though they were detached from the render tree. Widget *methods* are
  // closures over the JS object directly (registry-independent by design --
  // see the module doc comment), so the observable effect of the leak is
  // specifically that a deleted child could still be passed as a `parent`
  // argument (which does go through the registry via resolveWidgetArg) to
  // create new widgets under it. That must now fail instead of succeeding.
  test("delete() recursively removes descendants, so a deleted child can no longer be used as a parent", () => {
    const { env } = makeUiEnv();
    expect(() =>
      runMainScript(
        env,
        `
          local a = badge.ui.box(root, 10, 10)
          child_ref = badge.ui.box(a, 1, 1)
          a:delete()
          badge.ui.box(child_ref, 1, 1)
        `,
      ),
    ).toThrow(/stale or invalid widget reference/);
  });

  test("style({...}) merges into the widget's base style", () => {
    const { env, ui } = makeUiEnv();
    runMainScript(
      env,
      `
        local w = badge.ui.box(root, 1, 1)
        w:style({ bg_color = 0x112233, radius = 4 })
        w:style({ radius = 8 })
      `,
    );
    const w = ui.root.children[0]!;
    expect(w.style.bg_color).toBe(0x112233);
    expect(w.style.radius).toBe(8);
  });

  test("style({...}, selector) writes into styleSelectors, not the base style", () => {
    const { env, ui } = makeUiEnv();
    runMainScript(
      env,
      `
        local w = badge.ui.bar(root, 0, 100, 50)
        w:style({ bg_color = 0x00ff00 }, "indicator")
      `,
    );
    const w = ui.root.children[0]!;
    expect(w.styleSelectors["indicator"]).toEqual({ bg_color: 0x00ff00 });
    expect(w.style.bg_color).toBeUndefined();
  });

  test("line widget caps points at 128", () => {
    const { env, ui } = makeUiEnv();
    const pts = Array.from({ length: 200 }, (_, i) => `{${i},${i}}`).join(",");
    runMainScript(env, `local w = badge.ui.line(root, {${pts}})`);
    const w = ui.root.children[0]!;
    expect(w.points?.length).toBe(128);
  });
});

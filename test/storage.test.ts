import { beforeEach, describe, expect, test } from "bun:test";
import { createSandboxedEnv, runMainScript, setGlobalRaw } from "../src/lua/vm";
import { createStoreModule, MAX_KEYS, MAX_VALUE_BYTES } from "../src/badge/store";
import { createFsModule, MAX_FILE_BYTES, MAX_PATH_BYTES, MAX_PATH_DEPTH, MAX_TOTAL_BYTES } from "../src/badge/fs";
import { getStorage } from "../src/badge/storage";

// Confirms which backend bun test actually exercises: real DOM localStorage
// does not exist under bun, so the in-memory fallback in storage.ts is what
// badge.store/badge.fs run against here.
test("getStorage() falls back cleanly to the in-memory shim under bun (no DOM localStorage)", () => {
  expect(typeof (globalThis as Record<string, unknown>).localStorage).toBe("undefined");
  const s = getStorage();
  s.setItem("probe", "1");
  expect(s.getItem("probe")).toBe("1");
  s.removeItem("probe");
});

// The in-memory fallback (`storage.ts`'s `memoryFallback`) is a module-level
// singleton standing in for the browser's shared localStorage, so every test
// below clears it first for isolation between cases.
beforeEach(() => {
  getStorage().clear();
});

/** Binds a single badge.* module's Lua table directly as global `name`. */
function bindModule(env: ReturnType<typeof createSandboxedEnv>, name: string, mod: { attach(L: any): void }): void {
  setGlobalRaw(env, name, (L) => mod.attach(L));
}

function storeEnv(...slugs: string[]) {
  const env = createSandboxedEnv("/app", () => null);
  const names = ["store_a", "store_b", "store_c"];
  slugs.forEach((slug, i) => bindModule(env, names[i]!, createStoreModule(slug)));
  return env;
}

function fsEnv(...slugs: string[]) {
  const env = createSandboxedEnv("/app", () => null);
  const names = ["fs_a", "fs_b", "fs_c"];
  slugs.forEach((slug, i) => bindModule(env, names[i]!, createFsModule(slug)));
  return env;
}

describe("badge.store", () => {
  test("round-trips int and string values via set/get, set_int/get_int, set_str/get_str", () => {
    const env = storeEnv("app_rt");
    expect(() =>
      runMainScript(
        env,
        `
          store_a.set_int("k1", 42)
          assert(store_a.get_int("k1") == 42)

          store_a.set_str("k2", "hello")
          assert(store_a.get_str("k2") == "hello")

          store_a.set("k3", 7)
          assert(store_a.get("k3") == 7)

          store_a.set("k4", "world")
          assert(store_a.get("k4") == "world")

          assert(store_a.get("no_such_key") == nil)
        `,
      ),
    ).not.toThrow();
  });

  test("key pattern is enforced: [A-Za-z0-9_] only, and rejects (throws) on violation", () => {
    const env = storeEnv("app_key");
    expect(() => runMainScript(env, `store_a.set_int("bad-key!", 1)`)).toThrow(/invalid key/);
  });

  test("key length is capped at 24 bytes: 24 is fine, 25 throws", () => {
    const ok = storeEnv("app_key_ok");
    expect(() =>
      runMainScript(ok, `store_a.set_int(string.rep("k", 24), 1)`),
    ).not.toThrow();

    const bad = storeEnv("app_key_bad");
    expect(() =>
      runMainScript(bad, `store_a.set_int(string.rep("k", 25), 1)`),
    ).toThrow(/invalid key/);
  });

  test(`string value length is capped at ${MAX_VALUE_BYTES} bytes: at-limit is fine, over throws`, () => {
    const ok = storeEnv("app_val_ok");
    expect(() =>
      runMainScript(ok, `store_a.set_str("v", string.rep("x", ${MAX_VALUE_BYTES}))`),
    ).not.toThrow();

    const bad = storeEnv("app_val_bad");
    expect(() =>
      runMainScript(bad, `store_a.set_str("v", string.rep("x", ${MAX_VALUE_BYTES + 1}))`),
    ).toThrow(/exceeds/);
  });

  test(`key count is capped at ${MAX_KEYS}: exactly the cap is fine, one more throws`, () => {
    const ok = storeEnv("app_cap_ok");
    expect(() =>
      runMainScript(
        ok,
        `for i = 1, ${MAX_KEYS} do store_a.set_int("k" .. i, i) end`,
      ),
    ).not.toThrow();

    const bad = storeEnv("app_cap_bad");
    expect(() =>
      runMainScript(
        bad,
        `for i = 1, ${MAX_KEYS + 1} do store_a.set_int("k" .. i, i) end`,
      ),
    ).toThrow(/key limit/);
  });

  test("overwriting an existing key does not count against the key cap", () => {
    const env = storeEnv("app_overwrite");
    expect(() =>
      runMainScript(
        env,
        `
          for i = 1, ${MAX_KEYS} do store_a.set_int("k" .. i, i) end
          -- re-set an existing key many times; should never hit the cap
          for i = 1, 10 do store_a.set_int("k1", i) end
          assert(store_a.get_int("k1") == 10)
        `,
      ),
    ).not.toThrow();
  });

  test("storage is scoped per app slug: two slugs don't see each other's data", () => {
    const env = storeEnv("app_scope_a", "app_scope_b");
    expect(() =>
      runMainScript(
        env,
        `
          store_a.set_int("shared_key", 111)
          store_b.set_int("shared_key", 222)
          assert(store_a.get_int("shared_key") == 111)
          assert(store_b.get_int("shared_key") == 222)
          assert(store_a.get("only_in_a") == nil)
          store_a.set_str("only_in_a", "x")
          assert(store_b.get("only_in_a") == nil, "app_b should not see app_a's key")
        `,
      ),
    ).not.toThrow();
  });
});

describe("badge.fs", () => {
  test("round-trips a file write/read and reports existence", () => {
    const env = fsEnv("fsapp_rt");
    expect(() =>
      runMainScript(
        env,
        `
          assert(fs_a.exists("notes.txt") == false)
          fs_a.write("notes.txt", "hello")
          assert(fs_a.read("notes.txt") == "hello")
          assert(fs_a.exists("notes.txt") == true)
        `,
      ),
    ).not.toThrow();
  });

  test("append() concatenates onto an existing file", () => {
    const env = fsEnv("fsapp_append");
    expect(() =>
      runMainScript(
        env,
        `
          fs_a.write("log.txt", "a")
          fs_a.append("log.txt", "b")
          fs_a.append("log.txt", "c")
          assert(fs_a.read("log.txt") == "abc")
        `,
      ),
    ).not.toThrow();
  });

  test("remove() deletes a file", () => {
    const env = fsEnv("fsapp_remove");
    expect(() =>
      runMainScript(
        env,
        `
          fs_a.write("gone.txt", "x")
          assert(fs_a.exists("gone.txt") == true)
          fs_a.remove("gone.txt")
          assert(fs_a.exists("gone.txt") == false)
          assert(fs_a.read("gone.txt") == nil)
        `,
      ),
    ).not.toThrow();
  });

  test("mkdir() + list() surface an empty directory via its .dir marker", () => {
    const env = fsEnv("fsapp_mkdir");
    expect(() =>
      runMainScript(
        env,
        `
          fs_a.mkdir("emptydir")
          assert(fs_a.exists("emptydir") == true)
          fs_a.write("datadir/a.txt", "1")
          fs_a.write("datadir/b.txt", "2")
          local names = fs_a.list("datadir")
          assert(#names == 2, "expected 2 entries, got " .. tostring(#names))
          local top = fs_a.list()
          local has_emptydir, has_datadir = false, false
          for _, n in ipairs(top) do
            if n == "emptydir" then has_emptydir = true end
            if n == "datadir" then has_datadir = true end
          end
          assert(has_emptydir and has_datadir)
        `,
      ),
    ).not.toThrow();
  });

  test(`path depth is capped at ${MAX_PATH_DEPTH} segments: at-limit is fine, one more throws`, () => {
    const ok = fsEnv("fsapp_depth_ok");
    expect(() => runMainScript(ok, `fs_a.write("a/b/c/d", "x")`)).not.toThrow();

    const bad = fsEnv("fsapp_depth_bad");
    expect(() => runMainScript(bad, `fs_a.write("a/b/c/d/e", "x")`)).toThrow(/depth/);
  });

  test(`path length is capped at ${MAX_PATH_BYTES} bytes: at-limit is fine, one more throws`, () => {
    const ok = fsEnv("fsapp_len_ok");
    expect(() =>
      runMainScript(ok, `fs_a.write(string.rep("x", ${MAX_PATH_BYTES}), "v")`),
    ).not.toThrow();

    const bad = fsEnv("fsapp_len_bad");
    expect(() =>
      runMainScript(bad, `fs_a.write(string.rep("x", ${MAX_PATH_BYTES + 1}), "v")`),
    ).toThrow(/exceeds/);
  });

  test("rejects absolute paths and '..' segments", () => {
    const env = fsEnv("fsapp_traversal");
    expect(() => runMainScript(env, `fs_a.write("/abs.txt", "v")`)).toThrow(/relative/);
    const env2 = fsEnv("fsapp_traversal2");
    expect(() => runMainScript(env2, `fs_a.write("a/../b.txt", "v")`)).toThrow(/relative/);
  });

  test(`per-file quota is capped at ${MAX_FILE_BYTES} bytes: at-limit is fine, one more throws`, () => {
    const ok = fsEnv("fsapp_file_ok");
    expect(() =>
      runMainScript(ok, `fs_a.write("big.txt", string.rep("x", ${MAX_FILE_BYTES}))`),
    ).not.toThrow();

    const bad = fsEnv("fsapp_file_bad");
    expect(() =>
      runMainScript(bad, `fs_a.write("big.txt", string.rep("x", ${MAX_FILE_BYTES + 1}))`),
    ).toThrow(/per-file cap/);
  });

  test(`total app quota is capped at ${MAX_TOTAL_BYTES} bytes across files`, () => {
    const filesToFill = MAX_TOTAL_BYTES / MAX_FILE_BYTES; // exactly fills the quota
    const env = fsEnv("fsapp_quota");
    const writes = Array.from(
      { length: filesToFill },
      (_, i) => `fs_a.write("f${i}.txt", string.rep("x", ${MAX_FILE_BYTES}))`,
    ).join("\n");
    expect(() => runMainScript(env, writes)).not.toThrow();

    const env2 = fsEnv("fsapp_quota_over");
    const writesOver =
      writes + `\nfs_a.write("one_more.txt", "x")`;
    expect(() => runMainScript(env2, writesOver)).toThrow(/app quota/);
  });

  test("storage is scoped per app slug: two slugs don't see each other's files", () => {
    const env = fsEnv("fsapp_scope_a", "fsapp_scope_b");
    expect(() =>
      runMainScript(
        env,
        `
          fs_a.write("shared.txt", "from a")
          fs_b.write("shared.txt", "from b")
          assert(fs_a.read("shared.txt") == "from a")
          assert(fs_b.read("shared.txt") == "from b")
          assert(fs_b.exists("only_in_a.txt") == false)
        `,
      ),
    ).not.toThrow();
  });
});

import { describe, expect, test } from "bun:test";
import { parseManifest } from "../src/runtime/lifecycle";

describe("parseManifest", () => {
  test("parses required and optional keys", () => {
    const m = parseManifest(
      [
        "slug=smoke_test",
        "name=Smoke Test",
        "icon=icon.bin",
        "api=1",
        "heap_kb=64",
        "wake_lock=false",
        "home_button=true",
        "confirm_home=false",
        "version=0.1.0",
        "author=hack-the-badge",
      ].join("\n"),
    );
    expect(m).toEqual({
      slug: "smoke_test",
      name: "Smoke Test",
      icon: "icon.bin",
      api: "1",
      heap_kb: 64,
      wake_lock: false,
      home_button: "true",
      confirm_home: false,
      version: "0.1.0",
      author: "hack-the-badge",
    });
  });

  test("throws when required key 'slug' is missing", () => {
    expect(() => parseManifest("name=Only Name")).toThrow(/slug/);
  });

  test("throws when required key 'name' is missing", () => {
    expect(() => parseManifest("slug=only_slug")).toThrow(/name/);
  });

  test("optional keys are left undefined when absent", () => {
    const m = parseManifest("slug=bare\nname=Bare App");
    expect(m.icon).toBeUndefined();
    expect(m.heap_kb).toBeUndefined();
    expect(m.wake_lock).toBeUndefined();
    expect(m.confirm_home).toBeUndefined();
  });

  test("blank lines and comment lines are ignored without throwing", () => {
    const m = parseManifest(
      [
        "",
        "  ",
        "# a full-line comment",
        "; a semicolon comment",
        "slug=commented",
        "name=Commented App",
        "",
      ].join("\n"),
    );
    expect(m.slug).toBe("commented");
    expect(m.name).toBe("Commented App");
  });

  test("a line with no '=' is silently skipped rather than throwing", () => {
    const m = parseManifest(["slug=noeq", "name=No Eq App", "this line has no equals sign"].join("\n"));
    expect(m.slug).toBe("noeq");
    expect(m.name).toBe("No Eq App");
  });

  test("unknown keys are silently ignored (not surfaced on the manifest)", () => {
    const m = parseManifest(["slug=unk", "name=Unknown Key App", "totally_unknown_key=some value"].join("\n"));
    expect(m.slug).toBe("unk");
    expect((m as Record<string, unknown>).totally_unknown_key).toBeUndefined();
  });

  test("boolean parsing accepts 1/true/yes case-insensitively, else false", () => {
    expect(parseManifest("slug=b1\nname=B\nwake_lock=1").wake_lock).toBe(true);
    expect(parseManifest("slug=b2\nname=B\nwake_lock=TRUE").wake_lock).toBe(true);
    expect(parseManifest("slug=b3\nname=B\nwake_lock=yes").wake_lock).toBe(true);
    expect(parseManifest("slug=b4\nname=B\nwake_lock=no").wake_lock).toBe(false);
    expect(parseManifest("slug=b5\nname=B\nwake_lock=0").wake_lock).toBe(false);
  });

  test("heap_kb parses to an integer, empty value stays undefined", () => {
    expect(parseManifest("slug=h1\nname=H\nheap_kb=128").heap_kb).toBe(128);
    expect(parseManifest("slug=h2\nname=H\nheap_kb=").heap_kb).toBeUndefined();
  });

  test("values and keys are trimmed of surrounding whitespace", () => {
    const m = parseManifest("slug =  padded_slug  \n  name=  Padded Name  ");
    expect(m.slug).toBe("padded_slug");
    expect(m.name).toBe("Padded Name");
  });
});

import { describe, expect, test } from "bun:test";
import { createConsoleLog, REBOOT_MARKER, type ConsoleEntry } from "../src/ui/console-log";

const view = (entries: readonly ConsoleEntry[]) => entries.map((e) => [e.source, e.text]);

describe("console log", () => {
  test("splits lines and holds the partial tail", () => {
    const log = createConsoleLog();
    log.appendSerial("I (12) boot: ok\r\nI (13) app: st", 0);
    expect(view(log.entries())).toEqual([["serial", "I (12) boot: ok"]]);
    log.appendSerial("art\r", 0);
    expect(log.entries()).toHaveLength(1);
    log.appendSerial("\n\nhack> ", 0);
    expect(view(log.entries())).toEqual([
      ["serial", "I (12) boot: ok"],
      ["serial", "I (13) app: start"],
      ["serial", ""],
    ]);
    // Ids are unique and increasing.
    const ids = log.entries().map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect([...ids].sort((a, b) => a - b)).toEqual(ids);
  });

  test("marks a reboot when boot changes", () => {
    const log = createConsoleLog();
    log.appendSerial("first boot\nhalf", 0);
    log.appendSerial("", 0);
    expect(view(log.entries())).toEqual([["serial", "first boot"]]);
    log.appendSerial("second boot\n", 1);
    expect(REBOOT_MARKER).toBe("— badge rebooted —");
    // The old boot's partial line is flushed before the marker, never
    // glued to the new boot's first line.
    expect(view(log.entries())).toEqual([
      ["serial", "first boot"],
      ["serial", "half"],
      ["page", REBOOT_MARKER],
      ["serial", "second boot"],
    ]);
    log.appendSerial("more\n", 1);
    expect(log.entries().filter((e) => e.text === REBOOT_MARKER)).toHaveLength(1);
  });

  test("caps lines and tolerates partial and invalid sequences", () => {
    const log = createConsoleLog();
    let notified = 0;
    log.subscribe(() => notified++);
    const flood = Array.from({ length: 10_000 }, (_, i) => `line ${i}`).join("\n") + "\n";
    log.appendSerial(flood, 0);
    const kept = log.entries();
    expect(kept).toHaveLength(2000);
    expect(kept[0]!.text).toBe("line 8000");
    expect(kept.at(-1)!.text).toBe("line 9999");
    // One notification per append, not one per line.
    expect(notified).toBe(1);

    // Many small appends stay bounded too.
    for (let i = 0; i < 3000; i++) log.appendSerial(`x${i}\n`, 0);
    expect(log.entries()).toHaveLength(2000);
    expect(log.entries().at(-1)!.text).toBe("x2999");

    // U+FFFD (what a decoder makes of invalid bytes), a lone surrogate
    // and control bytes pass through unchanged.
    const odd = "bad �� byte \uD800 \x1b[0;32mI\x1b[0m \0end";
    log.appendSerial(`${odd}\n`, 0);
    expect(log.entries().at(-1)!.text).toBe(odd);

    // A flood with no newline at all stays bounded: the held tail is
    // flushed as a line once it is too long.
    const small = createConsoleLog(10);
    expect(() => small.appendSerial("z".repeat(1_000_000), 0)).not.toThrow();
    expect(small.entries().length).toBeLessThanOrEqual(10);
    for (const e of small.entries()) expect(e.text.length).toBeLessThanOrEqual(4096);
  });

  test("page entries interleave in arrival order", () => {
    const log = createConsoleLog();
    log.appendSerial("one\ntw", 0);
    log.appendPage("provisioning…");
    log.appendSerial("o\nthree\n", 0);
    log.appendPage("provisioned");
    expect(view(log.entries())).toEqual([
      ["serial", "one"],
      ["page", "provisioning…"],
      ["serial", "two"],
      ["serial", "three"],
      ["page", "provisioned"],
    ]);
  });

  test("snapshot is stable between changes and subscribers unsubscribe", () => {
    const log = createConsoleLog();
    const a = log.getSnapshot();
    expect(log.getSnapshot()).toBe(a);
    let calls = 0;
    const off = log.subscribe(() => calls++);
    log.appendSerial("partial", 0); // no complete line: no change
    expect(calls).toBe(0);
    expect(log.getSnapshot()).toBe(a);
    log.appendPage("hello");
    expect(calls).toBe(1);
    const b = log.getSnapshot();
    expect(b).not.toBe(a);
    expect(log.getSnapshot()).toBe(b);
    expect(b).toEqual(log.entries());
    off();
    log.appendPage("again");
    expect(calls).toBe(1);
  });
});

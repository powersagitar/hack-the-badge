import { describe, expect, test } from "bun:test";
import { buildBadgeNotice, buildProvisionControls, roleLabel } from "../src/ui/shell";

describe("roleLabel", () => {
  test("shows a firmware role string as words", () => {
    expect(roleLabel("hacker")).toBe("hacker");
    expect(roleLabel("workshop_lead")).toBe("workshop lead");
  });
});

/**
 * Just enough of the DOM for the shell's builders: `bun test` has no
 * `document`. Elements keep their children, text, class, type, value,
 * style and click listeners.
 */
class FakeElement {
  children: FakeElement[] = [];
  className = "";
  type = "";
  value = "";
  style: Record<string, string> = {};
  private ownText = "";
  private listeners = new Map<string, Array<() => void>>();
  constructor(readonly tagName: string) {}
  get textContent(): string {
    return this.ownText + this.children.map((c) => c.textContent).join("");
  }
  set textContent(t: string) {
    this.ownText = t;
    this.children = [];
  }
  set innerHTML(_: string) {
    this.ownText = "";
    this.children = [];
  }
  append(...els: FakeElement[]): void {
    this.children.push(...els);
  }
  appendChild(el: FakeElement): FakeElement {
    this.children.push(el);
    return el;
  }
  addEventListener(kind: string, cb: () => void): void {
    this.listeners.set(kind, [...(this.listeners.get(kind) ?? []), cb]);
  }
  click(): void {
    for (const cb of this.listeners.get("click") ?? []) cb();
  }
  /** Every descendant, depth first. */
  all(): FakeElement[] {
    return this.children.flatMap((c) => [c, ...c.all()]);
  }
}

const fakeDoc = { createElement: (tag: string) => new FakeElement(tag.toUpperCase()) } as unknown as Document;
const container = () => new FakeElement("DIV");
const asEl = (e: FakeElement) => e as unknown as HTMLElement;
const buttons = (root: FakeElement) => root.all().filter((e) => e.tagName === "BUTTON");

describe("provision controls", () => {
  test("a Bump button sits next to Shake, and each calls its own handler", () => {
    const root = container();
    const calls: string[] = [];
    buildProvisionControls(
      asEl(root),
      fakeDoc,
      ["hacker"],
      (role) => calls.push(`provision ${role}`),
      () => calls.push("shake"),
      () => calls.push("bump"),
    );
    const labels = buttons(root).map((b) => b.textContent);
    expect(labels).toContain("Shake");
    expect(labels.indexOf("Bump")).toBe(labels.indexOf("Shake") + 1);
    buttons(root).find((b) => b.textContent === "Bump")!.click();
    buttons(root).find((b) => b.textContent === "Shake")!.click();
    expect(calls).toEqual(["bump", "shake"]);
  });
});

describe("badge notice", () => {
  test("says the badge runs in another tab, and Run here takes it over", () => {
    const root = container();
    let takeOvers = 0;
    const notice = buildBadgeNotice(asEl(root), fakeDoc, () => takeOvers++);
    expect(root.textContent).toContain("Badge running in another tab");
    const run = buttons(root).find((b) => b.textContent === "Run here");
    expect(run).toBeDefined();
    // Hidden until the badge is found taken.
    expect(root.style.display).toBe("none");
    notice.setVisible(true);
    expect(root.style.display).toBe("");
    run!.click();
    expect(takeOvers).toBe(1);
    notice.setVisible(false);
    expect(root.style.display).toBe("none");
  });
});

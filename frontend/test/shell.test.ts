import { describe, expect, test } from "bun:test";
import { buildBadgeNotice, buildLinkPanel, buildProvisionControls, roleLabel } from "../src/ui/shell";

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

describe("link panel", () => {
  function build() {
    const root = container();
    const calls: string[] = [];
    const panel = buildLinkPanel(asEl(root), fakeDoc, {
      onHost: () => calls.push("host"),
      onJoin: (code) => calls.push(`join ${code}`),
      onManualCreate: () => calls.push("manual create"),
      onManualUse: (code) => calls.push(`manual use ${code}`),
      onRemove: (id) => calls.push(`remove ${id}`),
    });
    const byClass = (cls: string) => root.all().find((e) => e.className.split(" ").includes(cls))!;
    return { root, calls, panel, byClass };
  }

  test("Host link, then the join code input and Join link, then Manual, in that order", () => {
    const { root, calls, byClass } = build();
    const labels = buttons(root).map((b) => b.textContent);
    expect(labels.slice(0, 3)).toEqual(["Host link", "Join link", "Manual"]);
    buttons(root).find((b) => b.textContent === "Host link")!.click();
    const input = byClass("link-code");
    expect(input.tagName).toBe("INPUT");
    input.value = "abcd-efgh";
    buttons(root).find((b) => b.textContent === "Join link")!.click();
    expect(calls).toEqual(["host", "join abcd-efgh"]);
  });

  test("the status shows a hosted room code", () => {
    const { panel, byClass } = build();
    panel.setStatus("Room ABCDEFGH: waiting for a peer to join…");
    expect(byClass("link-status").textContent).toContain("ABCDEFGH");
  });

  test("the Manual toggle shows the copy-paste codes", () => {
    const { root, calls, panel, byClass } = build();
    const manual = byClass("link-manual");
    expect(manual.style.display).toBe("none");
    const toggle = buttons(root).find((b) => b.textContent === "Manual")!;
    toggle.click();
    expect(manual.style.display).toBe("");
    expect(toggle.className).toContain("active");
    buttons(root).find((b) => b.textContent === "Create code")!.click();
    panel.setManualOutput("offer:abc");
    expect(byClass("link-manual-out").value).toBe("offer:abc");
    byClass("link-manual-in").value = "answer:xyz";
    buttons(root).find((b) => b.textContent === "Use code")!.click();
    expect(calls).toEqual(["manual create", "manual use answer:xyz"]);
    toggle.click();
    expect(manual.style.display).toBe("none");
    expect(toggle.className).not.toContain("active");
  });

  test("the link list shows each link's state, with Remove", () => {
    const { root, calls, panel, byClass } = build();
    panel.setLinks([
      { id: "link-1", label: "Link 1 (room ABCDEFGH)", state: "open" },
      { id: "link-2", label: "Link 2 (manual)", state: "connecting" },
    ]);
    const list = byClass("link-list");
    expect(list.children.length).toBe(2);
    expect(list.children[0]!.textContent).toContain("Link 1 (room ABCDEFGH)");
    expect(list.children[0]!.textContent).toContain("connected");
    expect(list.children[1]!.textContent).toContain("connecting");
    buttons(list.children[1]!).find((b) => b.textContent === "Remove")!.click();
    expect(calls).toEqual(["remove link-2"]);
    panel.setLinks([]);
    expect(list.children.length).toBe(0);
    expect(root.textContent).not.toContain("Link 1");
  });

  test("hides with the provisioning controls", () => {
    const { root, panel } = build();
    panel.setVisible(false);
    expect(root.style.display).toBe("none");
    panel.setVisible(true);
    expect(root.style.display).toBe("");
  });
});

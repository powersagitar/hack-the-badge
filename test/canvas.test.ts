import { describe, expect, test } from "bun:test";
import { renderFrame } from "../src/render/canvas";
import type { Widget, WidgetType } from "../src/badge/ui";

let idSeq = 0;

/** Minimal hand-built Widget fixture, matching the real factories' defaults. */
function makeWidget(type: WidgetType, parent: Widget | null, over: Partial<Widget> = {}): Widget {
  const w: Widget = {
    id: ++idSeq,
    type,
    parent,
    children: [],
    x: 0,
    y: 0,
    w: 0,
    h: 0,
    align: null,
    hidden: false,
    clickable: false,
    deleted: false,
    style: {},
    styleSelectors: {},
    fontSize: "normal",
    borderColor: 0,
    borderWidth: 0,
    ...over,
  };
  if (parent) parent.children.push(w);
  return w;
}

/**
 * A fake CanvasRenderingContext2D that records only the drawing primitives
 * `src/render/canvas.ts` actually calls (verified by reading the source),
 * so we can assert on interop without pulling in a real DOM/canvas.
 */
function makeStubCtx() {
  const calls: string[] = [];
  const record =
    (name: string) =>
    (...args: unknown[]) => {
      calls.push(name);
      return undefined as unknown;
    };

  const ctx = {
    save: record("save"),
    restore: record("restore"),
    clearRect: record("clearRect"),
    fillRect: record("fillRect"),
    strokeRect: record("strokeRect"),
    beginPath: record("beginPath"),
    closePath: record("closePath"),
    moveTo: record("moveTo"),
    lineTo: record("lineTo"),
    arcTo: record("arcTo"),
    arc: record("arc"),
    fill: record("fill"),
    stroke: record("stroke"),
    fillText: (text: string) => {
      calls.push("fillText");
      return undefined;
    },
    measureText: (text: string) => ({ width: String(text).length * 6 }) as TextMetrics,
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    lineCap: "butt",
    globalAlpha: 1,
    font: "",
    textAlign: "left",
    textBaseline: "alphabetic",
  };

  return { ctx: ctx as unknown as CanvasRenderingContext2D, calls };
}

describe("renderFrame", () => {
  test("does not throw for an empty root", () => {
    const root = makeWidget("root", null);
    const { ctx } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
  });

  test("paints a rect-like widget (box) via fill, not throwing", () => {
    const root = makeWidget("root", null);
    makeWidget("box", root, { w: 20, h: 10, style: { bg_color: 0x112233 } });
    const { ctx, calls } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
    expect(calls).toContain("fill");
    expect(calls).toContain("beginPath");
  });

  test("paints a text-like widget (label) via fillText", () => {
    const root = makeWidget("root", null);
    makeWidget("label", root, { w: 50, h: 12, text: "hello" });
    const { ctx, calls } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
    expect(calls).toContain("fillText");
  });

  test("paints a line-like widget (line) via moveTo/lineTo/stroke", () => {
    const root = makeWidget("root", null);
    makeWidget("line", root, {
      points: [
        [0, 0],
        [10, 10],
        [20, 0],
      ],
    });
    const { ctx, calls } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
    expect(calls).toContain("moveTo");
    expect(calls).toContain("lineTo");
    expect(calls).toContain("stroke");
  });

  test("a line widget with fewer than 2 points draws nothing (no crash)", () => {
    const root = makeWidget("root", null);
    makeWidget("line", root, { points: [[0, 0]] });
    const { ctx } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
  });

  test("skips hidden and deleted widgets entirely", () => {
    const root = makeWidget("root", null);
    makeWidget("label", root, { hidden: true, text: "should not paint" });
    makeWidget("label", root, { deleted: true, text: "should not paint either" });
    const { ctx, calls } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
    expect(calls).not.toContain("fillText");
  });

  test("renders one of each visually-distinct widget kind without throwing", () => {
    const root = makeWidget("root", null);
    makeWidget("box", root, { w: 10, h: 10 });
    makeWidget("button", root, { w: 10, h: 10 });
    makeWidget("bar", root, { w: 20, h: 8, min: 0, max: 100, value: 30 });
    makeWidget("slider", root, { w: 20, h: 8, min: 0, max: 100, value: 30 });
    makeWidget("arc", root, { w: 20, h: 20, min: 0, max: 100, value: 50 });
    makeWidget("label", root, { text: "hi" });
    makeWidget("textarea", root, { text: "hi" });
    makeWidget("checkbox", root, { text: "check", checked: true });
    makeWidget("switch", root, { checked: false });
    makeWidget("roller", root, { options: ["a", "b"], selected: 0 });
    makeWidget("image", root, { src: "icon.bin" });
    makeWidget("line", root, {
      points: [
        [0, 0],
        [5, 5],
      ],
    });
    const { ctx } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
  });

  test("nested children are painted relative to their parent", () => {
    const root = makeWidget("root", null);
    const box = makeWidget("box", root, { x: 10, y: 10, w: 100, h: 100 });
    makeWidget("label", box, { x: 5, y: 5, text: "nested" });
    const { ctx, calls } = makeStubCtx();
    expect(() => renderFrame(ctx, root)).not.toThrow();
    expect(calls).toContain("fillText");
  });
});

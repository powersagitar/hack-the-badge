/**
 * Walks the widget tree from `src/badge/ui.ts` and paints it onto a 320x240
 * 2D canvas. Deliberately not pixel-perfect LVGL — filled rects/arcs/lines,
 * legible text, and best-effort `align()` interpretation, per the spec.
 */
import type { Widget, WidgetStyle } from "../badge/ui";
import { SCREEN_HEIGHT, SCREEN_WIDTH } from "../badge/ui";

const DEFAULT_FONT_FAMILY = "monospace";

export function renderFrame(ctx: CanvasRenderingContext2D, root: Widget): void {
  ctx.save();
  ctx.clearRect(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
  ctx.fillStyle = "#000000";
  ctx.fillRect(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
  for (const child of root.children) {
    paintWidget(ctx, child, 0, 0, SCREEN_WIDTH, SCREEN_HEIGHT);
  }
  ctx.restore();
}

function paintWidget(
  ctx: CanvasRenderingContext2D,
  w: Widget,
  parentAbsX: number,
  parentAbsY: number,
  parentW: number,
  parentH: number,
): void {
  if (w.hidden || w.deleted) return;

  const { x, y } = resolvePosition(w, parentAbsX, parentAbsY, parentW, parentH);
  const width = w.w;
  const height = w.h;
  const style = w.style;

  switch (w.type) {
    case "box":
    case "button":
      paintBox(ctx, x, y, width, height, style);
      break;
    case "bar":
    case "slider":
      paintBox(ctx, x, y, width, height, style);
      paintValueFill(ctx, x, y, width, height, w, style);
      break;
    case "arc":
      paintArc(ctx, x, y, width, height, w, style);
      break;
    case "label":
      paintText(ctx, x, y, width, height, w.text ?? "", style, false);
      break;
    case "textarea":
      paintText(ctx, x, y, width, height, w.text ?? "", style, true);
      break;
    case "checkbox":
      paintCheckbox(ctx, x, y, width, height, w, style);
      break;
    case "switch":
      paintSwitch(ctx, x, y, width, height, w, style);
      break;
    case "roller":
      paintRoller(ctx, x, y, width, height, w, style);
      break;
    case "image":
      paintImagePlaceholder(ctx, x, y, width, height, w);
      break;
    case "line":
      paintLine(ctx, x, y, w, style);
      break;
    case "root":
    default:
      break;
  }

  for (const child of w.children) {
    paintWidget(ctx, child, x, y, width, height);
  }
}

function resolvePosition(
  w: Widget,
  parentAbsX: number,
  parentAbsY: number,
  parentW: number,
  parentH: number,
): { x: number; y: number } {
  if (!w.align) {
    return { x: parentAbsX + w.x, y: parentAbsY + w.y };
  }
  const anchor = alignAnchor(w.align.name, parentW, parentH, w.w, w.h);
  return { x: parentAbsX + anchor.x + w.align.dx, y: parentAbsY + anchor.y + w.align.dy };
}

function alignAnchor(
  name: string,
  pw: number,
  ph: number,
  ww: number,
  wh: number,
): { x: number; y: number } {
  switch (name) {
    case "center":
      return { x: (pw - ww) / 2, y: (ph - wh) / 2 };
    case "top_left":
      return { x: 0, y: 0 };
    case "top_mid":
      return { x: (pw - ww) / 2, y: 0 };
    case "top_right":
      return { x: pw - ww, y: 0 };
    case "left_mid":
      return { x: 0, y: (ph - wh) / 2 };
    case "right_mid":
      return { x: pw - ww, y: (ph - wh) / 2 };
    case "bottom_left":
      return { x: 0, y: ph - wh };
    case "bottom_mid":
      return { x: (pw - ww) / 2, y: ph - wh };
    case "bottom_right":
      return { x: pw - ww, y: ph - wh };
    default:
      // Unknown/unsupported align name — best-effort fallback to top-left.
      return { x: 0, y: 0 };
  }
}

function cssColor(n: unknown, fallback: string): string {
  if (typeof n !== "number" || Number.isNaN(n)) return fallback;
  const v = Math.max(0, Math.min(0xffffff, Math.trunc(n)));
  return "#" + v.toString(16).padStart(6, "0");
}

function opaAlpha(opa: unknown, fallback = 255): number {
  const n = typeof opa === "number" ? opa : fallback;
  return Math.max(0, Math.min(255, n)) / 255;
}

function fontPx(fontSize: unknown): number {
  if (typeof fontSize === "string") {
    switch (fontSize) {
      case "small":
        return 11;
      case "large":
        return 18;
      case "xlarge":
        return 22;
      case "normal":
        return 14;
      default: {
        const n = Number(fontSize);
        if (Number.isFinite(n) && n > 0) return n;
      }
    }
  }
  return 14;
}

function roundRectPath(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  if (w <= 0 || h <= 0) {
    ctx.beginPath();
    return;
  }
  const rr = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.lineTo(x + w - rr, y);
  ctx.arcTo(x + w, y, x + w, y + rr, rr);
  ctx.lineTo(x + w, y + h - rr);
  ctx.arcTo(x + w, y + h, x + w - rr, y + h, rr);
  ctx.lineTo(x + rr, y + h);
  ctx.arcTo(x, y + h, x, y + h - rr, rr);
  ctx.lineTo(x, y + rr);
  ctx.arcTo(x, y, x + rr, y, rr);
  ctx.closePath();
}

function paintBox(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, style: WidgetStyle): void {
  if (w <= 0 || h <= 0) return;
  const radius = Math.max(0, Number(style.radius) || 0);

  ctx.save();
  ctx.globalAlpha = opaAlpha(style.bg_opa);
  ctx.fillStyle = cssColor(style.bg_color, "#222222");
  roundRectPath(ctx, x, y, w, h, radius);
  ctx.fill();
  ctx.restore();

  const borderWidth = Number(style.border_width) || 0;
  if (borderWidth > 0) {
    ctx.save();
    ctx.globalAlpha = opaAlpha(style.border_opa);
    ctx.strokeStyle = cssColor(style.border_color, "#888888");
    ctx.lineWidth = borderWidth;
    roundRectPath(
      ctx,
      x + borderWidth / 2,
      y + borderWidth / 2,
      Math.max(0, w - borderWidth),
      Math.max(0, h - borderWidth),
      radius,
    );
    ctx.stroke();
    ctx.restore();
  }
}

function paintValueFill(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  widget: Widget,
  style: WidgetStyle,
): void {
  if (w <= 0 || h <= 0) return;
  const min = widget.min ?? 0;
  const max = widget.max ?? 100;
  const value = widget.value ?? min;
  const frac = max > min ? Math.max(0, Math.min(1, (value - min) / (max - min))) : 0;
  const indicatorStyle = widget.styleSelectors["indicator"] || {};
  const color = cssColor(indicatorStyle.bg_color ?? style.color, "#3399ff");

  ctx.save();
  ctx.fillStyle = color;
  const fillW = Math.max(0, w * frac);
  roundRectPath(ctx, x, y, fillW, h, Math.min(Number(style.radius) || 0, h / 2));
  ctx.fill();
  ctx.restore();

  if (widget.type === "slider") {
    const knobR = Math.min(h, 10);
    ctx.save();
    ctx.fillStyle = "#ffffff";
    ctx.beginPath();
    ctx.arc(x + fillW, y + h / 2, knobR / 2 + 2, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

function paintArc(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  widget: Widget,
  style: WidgetStyle,
): void {
  if (w <= 0 || h <= 0) return;
  const cx = x + w / 2;
  const cy = y + h / 2;
  const radius = Math.max(2, Math.min(w, h) / 2 - 4);
  const min = widget.min ?? 0;
  const max = widget.max ?? 100;
  const value = widget.value ?? min;
  const frac = max > min ? Math.max(0, Math.min(1, (value - min) / (max - min))) : 0;
  const startAngle = -Math.PI / 2;
  const endAngle = startAngle + frac * Math.PI * 2;
  const lineWidth = Number(style.arc_width) || 6;

  ctx.save();
  ctx.lineCap = "round";
  ctx.lineWidth = lineWidth;
  ctx.strokeStyle = "#333333";
  ctx.beginPath();
  ctx.arc(cx, cy, radius, 0, Math.PI * 2);
  ctx.stroke();

  ctx.strokeStyle = cssColor(style.arc_color, "#3399ff");
  ctx.beginPath();
  ctx.arc(cx, cy, radius, startAngle, endAngle);
  ctx.stroke();
  ctx.restore();
}

function wrapAndDrawText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  maxWidth: number,
  lineHeight: number,
): void {
  const lines: string[] = [];
  if (!maxWidth || maxWidth <= 0) {
    lines.push(text);
  } else {
    let line = "";
    for (const word of String(text).split(/\s+/)) {
      const test = line ? `${line} ${word}` : word;
      if (line && ctx.measureText(test).width > maxWidth) {
        lines.push(line);
        line = word;
      } else {
        line = test;
      }
    }
    if (line) lines.push(line);
  }
  let cy = y;
  for (const l of lines) {
    ctx.fillText(l, x, cy);
    cy += lineHeight + 2;
  }
}

function paintText(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  text: string,
  style: WidgetStyle,
  boxed: boolean,
): void {
  const px = fontPx(style.text_font);
  if (boxed) {
    paintBox(ctx, x, y, w || 100, h || px + 8, {
      bg_color: style.bg_color ?? 0x111111,
      radius: style.radius ?? 4,
      border_width: style.border_width ?? 1,
      border_color: style.border_color ?? 0x555555,
    });
  }

  ctx.save();
  ctx.fillStyle = cssColor(style.text_color, "#ffffff");
  ctx.font = `${px}px ${DEFAULT_FONT_FAMILY}`;
  ctx.textBaseline = "top";

  let drawX = x + 2;
  if (style.text_align === "center") {
    ctx.textAlign = "center";
    drawX = x + (w || ctx.measureText(text).width) / 2;
  } else if (style.text_align === "right") {
    ctx.textAlign = "right";
    drawX = x + (w || 0) - 2;
  } else {
    ctx.textAlign = "left";
  }
  wrapAndDrawText(ctx, text, drawX, y + 2, w, px);
  ctx.restore();
}

function paintCheckbox(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  widget: Widget,
  style: WidgetStyle,
): void {
  const boxSize = Math.min(h || 16, 16);
  const boxY = y + ((h || boxSize) - boxSize) / 2;

  ctx.save();
  ctx.strokeStyle = cssColor(style.border_color ?? style.text_color, "#ffffff");
  ctx.lineWidth = 1;
  ctx.strokeRect(x + 0.5, boxY + 0.5, boxSize, boxSize);
  if (widget.checked) {
    ctx.fillStyle = cssColor(style.bg_color, "#3399ff");
    ctx.fillRect(x + 2, boxY + 2, Math.max(0, boxSize - 4), Math.max(0, boxSize - 4));
  }
  ctx.fillStyle = cssColor(style.text_color, "#ffffff");
  ctx.font = `${fontPx(style.text_font)}px ${DEFAULT_FONT_FAMILY}`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "left";
  ctx.fillText(widget.text ?? "", x + boxSize + 6, y + (h || boxSize) / 2);
  ctx.restore();
}

function paintSwitch(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  widget: Widget,
  style: WidgetStyle,
): void {
  const width = w || 40;
  const height = h || 20;
  ctx.save();
  ctx.fillStyle = widget.checked ? cssColor(style.bg_color, "#33cc66") : "#555555";
  roundRectPath(ctx, x, y, width, height, height / 2);
  ctx.fill();

  const knobR = height / 2 - 2;
  const knobX = widget.checked ? x + width - height / 2 : x + height / 2;
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.arc(knobX, y + height / 2, Math.max(1, knobR), 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function paintRoller(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  widget: Widget,
  style: WidgetStyle,
): void {
  paintBox(ctx, x, y, w || 100, h || 24, {
    bg_color: style.bg_color ?? 0x1a1a1a,
    radius: style.radius ?? 4,
  });
  const options = widget.options ?? [];
  const selected = widget.selected ?? 0;
  ctx.save();
  ctx.fillStyle = cssColor(style.text_color, "#ffffff");
  ctx.font = `${fontPx(style.text_font)}px ${DEFAULT_FONT_FAMILY}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(options[selected] ?? "", x + (w || 100) / 2, y + (h || 24) / 2);
  ctx.restore();
}

function paintImagePlaceholder(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  widget: Widget,
): void {
  const width = w || 32;
  const height = h || 32;
  ctx.save();
  ctx.fillStyle = "#333333";
  ctx.fillRect(x, y, width, height);
  ctx.strokeStyle = "#888888";
  ctx.strokeRect(x + 0.5, y + 0.5, Math.max(0, width - 1), Math.max(0, height - 1));
  ctx.fillStyle = "#cccccc";
  ctx.font = `10px ${DEFAULT_FONT_FAMILY}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  const label = widget.src ? widget.src.split("/").pop() || widget.src : "image";
  ctx.fillText(label, x + width / 2, y + height / 2);
  ctx.restore();
}

function paintLine(ctx: CanvasRenderingContext2D, x: number, y: number, widget: Widget, style: WidgetStyle): void {
  const pts = widget.points ?? [];
  if (pts.length < 2) return;
  ctx.save();
  ctx.strokeStyle = cssColor(style.line_color, "#ffffff");
  ctx.lineWidth = Number(style.line_width) || 1;
  ctx.beginPath();
  pts.forEach(([px, py], i) => {
    if (i === 0) ctx.moveTo(x + px, y + py);
    else ctx.lineTo(x + px, y + py);
  });
  ctx.stroke();
  ctx.restore();
}

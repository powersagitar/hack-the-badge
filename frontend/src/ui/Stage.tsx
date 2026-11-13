/**
 * The stage (spec "The badge", "Physics"): the dotted panel the badge
 * hangs in from its lanyard, "Reset swing", the hint, and the one frame
 * loop that drives them.
 *
 * - **The lanyard** (`../motion/lanyard.ts`) is created here, from the
 *   stage's size and the layout, and handed out through `onLanyard` (the
 *   controller subscribes its impacts and bumps it). A layout change
 *   builds a new one (its badge-size clamps differ); a resize relays it
 *   out (`ResizeObserver`).
 * - **The frame loop** (`requestAnimationFrame`) steps the lanyard by the
 *   frame's wall time, then writes the badge's transform, the rope's SVG
 *   paths (the design's `tick`), the LED glows from the client's latest
 *   `onLeds` frame, and the power switch's `.is-down` while the badge
 *   reboots. The lanyard sends its reading to `sink` (`start(sink)`) on
 *   every step that ran.
 * - **LEDs.** Chain index i lights `LED_SPOTS[LED_CHAIN_TO_SPOT[i]]`. The
 *   glow's opacity is `max(r,g,b)/255` (off is 0), and its `--rgb` is the
 *   colour scaled to full brightness, so the brightness is counted once.
 * - **Width.** On desktop the stage is at most {@link STAGE_MAX_W} px
 *   wide: past about that, the rope can't swing the badge into a wall
 *   (Task 9), and drags would never bump.
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { createLanyard, lanyardGeometry, type Lanyard } from "../motion/lanyard";
import type { Mg, MotionSource } from "../motion/source";
import { Button } from "@/components/ui/button";
import { Badge, type BadgeClient, type BadgeParts, type BadgeSize, type Layout } from "./Badge";
import { LED_CHAIN_TO_SPOT, LED_SPOTS } from "./badge-geometry";

/** The desktop stage's width cap (px): the design's stage at 1440 px is about 1016. */
export const STAGE_MAX_W = 1016;

/** The breakpoint between the two layouts (Tailwind's `md`). */
export const DESKTOP_QUERY = "(min-width: 768px)";

const HINT: Record<Layout, string> = {
  desktop: "DRAG THE BADGE TO SWING IT · PRESS THE BUTTONS ON THE BOARD",
  mobile: "DRAG TO SWING · TAP THE BUTTONS",
};

/** The layout from {@link DESKTOP_QUERY}, following the window as it changes. */
export function useLayout(): Layout {
  const subscribe = useCallback((cb: () => void) => {
    if (typeof window === "undefined" || !window.matchMedia) return () => {};
    const mq = window.matchMedia(DESKTOP_QUERY);
    mq.addEventListener("change", cb);
    return () => mq.removeEventListener("change", cb);
  }, []);
  const get = () =>
    typeof window === "undefined" || !window.matchMedia || window.matchMedia(DESKTOP_QUERY).matches
      ? "desktop"
      : "mobile";
  return useSyncExternalStore(subscribe, get, () => "desktop");
}

export type StageLanyard = Lanyard & MotionSource;

export interface StageProps {
  /** The firmware client; without one the board's controls do nothing. */
  client?: BadgeClient | null;
  /** Where the lanyard's readings go (the active motion sink); none stops them. */
  sink?: ((mg: Mg) => void) | null;
  /** Hears the lanyard as it is built (and null when it goes). */
  onLanyard?: (lanyard: StageLanyard | null) => void;
  /** Receives the badge's screen canvas. */
  onCanvas?: (canvas: HTMLCanvasElement | null) => void;
}

/** The rope's SVG path (the design's `tick`): smoothed through the nodes, ending 12 px above the ring. */
export function ropePath(rope: readonly { x: number; y: number }[], centre: { x: number; y: number }): string {
  const n = rope.length - 1;
  const f = (v: number) => v.toFixed(1);
  let d = `M${f(rope[0].x)} ${f(rope[0].y)}`;
  for (let i = 1; i < n; i++) {
    const mx = (rope[i].x + rope[i + 1].x) / 2;
    const my = (rope[i].y + rope[i + 1].y) / 2;
    d += ` Q${f(rope[i].x)} ${f(rope[i].y)} ${f(mx)} ${f(my)}`;
  }
  const P = rope[n];
  const ux = centre.x - P.x;
  const uy = centre.y - P.y;
  const ul = Math.hypot(ux, uy) || 1;
  return `${d} L${f(P.x - (ux / ul) * 12)} ${f(P.y - (uy / ul) * 12)}`;
}

/** One LED's glow: `--rgb` at full brightness and the opacity, or off. */
export function ledGlow(r: number, g: number, b: number): { rgb: string; opacity: number } {
  const max = Math.max(r, g, b);
  if (!(max > 0)) return { rgb: "255,255,255", opacity: 0 };
  const k = 255 / max;
  return { rgb: `${Math.round(r * k)},${Math.round(g * k)},${Math.round(b * k)}`, opacity: max / 255 };
}

function sizeOf(width: number, height: number, layout: Layout): BadgeSize {
  const { bw, bh, clip } = lanyardGeometry(width, height, layout);
  return { bw, bh, clip };
}

export function Stage({ client = null, sink = null, onLanyard, onCanvas }: StageProps) {
  const layout = useLayout();
  const stageRef = useRef<HTMLDivElement | null>(null);
  const ribbon = useRef<SVGPathElement | null>(null);
  const stitch = useRef<SVGPathElement | null>(null);
  const shadow = useRef<SVGPathElement | null>(null);
  const parts = useRef<BadgeParts>({ root: null, leds: [], power: null }).current;
  const [lanyard, setLanyard] = useState<StageLanyard | null>(null);
  const [size, setSize] = useState<BadgeSize>(() => sizeOf(1, 1, layout));
  const leds = useRef<{ rgb: Uint8Array | null; dirty: boolean }>({ rgb: null, dirty: false });

  // The lanyard: one per layout, relaid out as the stage resizes.
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const l = createLanyard({ width: el.clientWidth, height: el.clientHeight, layout });
    setLanyard(l);
    const relayout = () => l.relayout(el.clientWidth, el.clientHeight);
    const ro = typeof window.ResizeObserver !== "undefined" ? new window.ResizeObserver(relayout) : null;
    ro?.observe(el);
    if (!ro) window.addEventListener("resize", relayout);
    return () => {
      ro?.disconnect();
      if (!ro) window.removeEventListener("resize", relayout);
      l.release();
      l.stop();
      setLanyard(null);
    };
  }, [layout]);

  useEffect(() => {
    onLanyard?.(lanyard);
  }, [lanyard, onLanyard]);

  useEffect(() => {
    if (!lanyard) return;
    if (sink) lanyard.start(sink);
    else lanyard.stop();
    return () => lanyard.stop();
  }, [lanyard, sink]);

  // The firmware's LED frames, kept for the next animation frame.
  useEffect(() => {
    const unsubscribe = client?.onLeds((rgb) => {
      leds.current = { rgb, dirty: true };
    });
    leds.current = { rgb: null, dirty: true };
    return unsubscribe;
  }, [client]);

  // The frame loop.
  useEffect(() => {
    if (!lanyard) return;
    let raf = 0;
    let last: number | null = null;
    let drawn: BadgeSize | null = null;
    const frame = (now: number) => {
      raf = window.requestAnimationFrame(frame);
      if (last !== null) lanyard.step(Math.min(now - last, 100));
      last = now;
      const { rope, badge } = lanyard.pose();
      if (!drawn || Math.abs(drawn.bh - badge.h) > 0.5) {
        drawn = { bw: badge.w, bh: badge.h, clip: Math.round(badge.w * 0.2) };
        setSize(drawn);
      }
      if (parts.root) {
        parts.root.style.transform = `translate(${badge.x - badge.w / 2}px,${badge.y - badge.h / 2}px) rotate(${badge.angle}rad)`;
      }
      const d = ropePath(rope, badge);
      ribbon.current?.setAttribute("d", d);
      stitch.current?.setAttribute("d", d);
      shadow.current?.setAttribute("d", d);
      parts.power?.classList.toggle("is-down", client?.rebooting() ?? false);
      if (leds.current.dirty) {
        leds.current.dirty = false;
        const rgb = leds.current.rgb;
        LED_SPOTS.forEach((_, chain) => {
          const el = parts.leds[LED_CHAIN_TO_SPOT[chain]];
          if (!el) return;
          const o = chain * 3;
          const glow = rgb && rgb.length >= o + 3 ? ledGlow(rgb[o], rgb[o + 1], rgb[o + 2]) : ledGlow(0, 0, 0);
          el.style.setProperty("--rgb", glow.rgb);
          el.style.opacity = glow.opacity.toFixed(3);
        });
      }
    };
    raf = window.requestAnimationFrame(frame);
    return () => window.cancelAnimationFrame(raf);
  }, [lanyard, client, parts]);

  const mobile = layout === "mobile";
  return (
    <div
      ref={stageRef}
      data-slot="stage"
      className="stage-grid relative h-[640px] w-full min-w-0 overflow-hidden rounded-panel-mobile border border-border bg-card md:h-[780px] md:w-auto md:max-w-[1016px] md:flex-[999_1_560px] md:rounded-panel"
    >
      <svg
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 h-full w-full overflow-visible"
      >
        <path ref={shadow} d="" transform="translate(3 6)" fill="none" stroke="rgba(0,0,0,.55)" strokeWidth={20} strokeLinecap="round" strokeLinejoin="round" />
        <path ref={ribbon} d="" fill="none" style={{ stroke: "var(--lanyard)" }} strokeWidth={17} strokeLinecap="round" strokeLinejoin="round" />
        <path ref={stitch} d="" fill="none" stroke="rgba(255,255,255,.22)" strokeWidth={1.2} strokeDasharray="5 4" strokeLinecap="round" />
      </svg>
      <Badge
        layout={layout}
        size={size}
        client={client}
        lanyard={lanyard}
        stageRef={stageRef}
        parts={parts}
        onCanvas={onCanvas}
      />
      <Button type="button" className="absolute top-3 right-3" onClick={() => lanyard?.reset()}>
        Reset swing
      </Button>
      <div
        className={
          mobile
            ? "pointer-events-none absolute right-3 bottom-4 left-3 flex items-center justify-center gap-2 text-center font-mono text-[10.5px] leading-[1.4] font-medium tracking-[.04em] text-muted-foreground"
            : "pointer-events-none absolute right-4 bottom-5 left-4 flex items-center justify-center gap-2 text-center font-mono text-[11px] leading-[1.4] font-medium tracking-[.04em] text-muted-foreground"
        }
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M8 13V5.5a1.5 1.5 0 0 1 3 0V12" />
          <path d="M11 11.5v-2a1.5 1.5 0 0 1 3 0V12" />
          <path d="M14 10.5a1.5 1.5 0 0 1 3 0V12" />
          <path d="M17 11.5a1.5 1.5 0 0 1 3 0V16a6 6 0 0 1-6 6h-2a6 6 0 0 1-5-2.7l-3-4.6a1.6 1.6 0 0 1 2.6-1.8L8 15" />
        </svg>
        <span>{HINT[layout]}</span>
      </div>
    </div>
  );
}

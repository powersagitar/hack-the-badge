/**
 * The badge (spec "The badge"): the board photo with the firmware's
 * screen in its LCD, the six LED glows, the hit targets over the board's
 * buttons, the power switch, and the split rings and clasp the lanyard
 * hangs from (the design's markup, `Main.dc.html`'s badge `div`).
 *
 * - **Buttons.** Pointer down presses (`injectButton(name, "pressed",
 *   "pad")`), and that pointer's up or cancel anywhere releases it (one
 *   window listener, as the design's `onUp`, but by `pointerId`: on
 *   multi-touch, a second finger lifting off leaves the thumb's D-pad
 *   press held; a button two fingers hold is released when both lift).
 *   A hit button never stops a pointerup: a mouse gets no implicit
 *   capture, so a press released over another hit target must still
 *   reach the window. A keyboard
 *   click (`detail === 0`) taps: pressed, then released 150 ms later; the
 *   runtime's tap queue holds it long enough for the firmware.
 * - **AUX1** is the side slide switch: a press (or Space) toggles it, on
 *   holding the slot and off releasing it. Both ways go through the pad
 *   source, so the runtime holds it exactly while it reads on.
 * - **Keyboard** ({@link keyAction}): held keys press with the `key`
 *   source; a window blur releases them (a keyup that never arrives
 *   would strand the button). S and K are impulses on the lanyard.
 * - **Power** calls `client.powerCycle()`; the runtime ignores a press
 *   while it reboots. The stage's frame loop shows `.is-down` on it
 *   while `client.rebooting()`.
 * - **Drag.** Pointer down on the body (not on a button) grabs the
 *   lanyard at that point; moves drag it, and pointerup, pointercancel,
 *   lostpointercapture (all on the window) and the page going hidden
 *   release it (Review Focus 1).
 *
 * The stage owns the frame loop: it writes the badge's transform, the
 * LED glows and the power switch's state through {@link BadgeParts}.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, RefObject } from "react";
import type { Lanyard, Point } from "../motion/lanyard";
import type { ButtonName, ButtonSource } from "../runtime/buttons";
import { BUTTON_ORDER, BUTTON_SPOTS, HOLES, LED_SPOTS, PHOTO, POWER, SCREEN } from "./badge-geometry";
import { keyAction } from "./keyboard";

/** What the badge needs from the firmware client (`WorkerFirmwareRuntime` has it all). */
export interface BadgeClient {
  injectButton(button: ButtonName, kind: "pressed" | "released", source: ButtonSource): void;
  powerCycle(): void;
  rebooting(): boolean;
  onLeds(cb: (rgb: Uint8Array) => void): () => void;
}

/** The elements the stage's frame loop writes to; the badge fills them in. */
export interface BadgeParts {
  root: HTMLDivElement | null;
  /** By LED_SPOTS index. */
  leds: (HTMLDivElement | null)[];
  power: HTMLButtonElement | null;
}

export type Layout = "desktop" | "mobile";

/** The badge's drawn size (px): the lanyard's `bw`, `bh` and `clip`. */
export interface BadgeSize {
  bw: number;
  bh: number;
  clip: number;
}

export interface BadgeProps {
  layout: Layout;
  size: BadgeSize;
  client: BadgeClient | null;
  lanyard: Pick<Lanyard, "grab" | "drag" | "release" | "impulse"> | null;
  /** The stage, for pointer coordinates. */
  stageRef: RefObject<HTMLElement | null>;
  parts: BadgeParts;
  /**
   * Receives the 320×240 screen canvas (the client blits frames into it),
   * once per canvas element; it need not be a stable function.
   */
  onCanvas?: (canvas: HTMLCanvasElement | null) => void;
}

/** How long a keyboard click holds a button (the design's 150 ms). */
export const TAP_MS = 150;

/** A client pointer position in the stage's (unscaled) coordinates, as the design's `toStage`. */
export function toStage(stage: HTMLElement, e: { clientX: number; clientY: number }): Point {
  const r = stage.getBoundingClientRect();
  const sx = stage.offsetWidth ? r.width / stage.offsetWidth : 1;
  const sy = stage.offsetHeight ? r.height / stage.offsetHeight : 1;
  return {
    x: (e.clientX - r.left) / (sx || 1) - stage.clientLeft,
    y: (e.clientY - r.top) / (sy || 1) - stage.clientTop,
  };
}

/** The split rings and clasp's geometry (the design's `makeGeo`). */
function ringGeometry({ bw, bh, clip }: BadgeSize) {
  const hy = clip + (bh * (HOLES[0][1] + HOLES[1][1])) / 200;
  const ringR = bw * 0.031;
  return {
    svgTop: -clip,
    svgH: clip + bh * 0.08,
    px: bw / 2,
    claspX: bw / 2 - 8,
    h1x: (bw * HOLES[0][0]) / 100,
    h2x: (bw * HOLES[1][0]) / 100,
    hy,
    ringY: hy - ringR * 0.3,
    ringR,
  };
}

type Held = Map<ButtonName, Set<ButtonSource>>;

export function Badge({ layout, size, client, lanyard, stageRef, parts, onCanvas }: BadgeProps) {
  const [down, setDown] = useState<ReadonlySet<ButtonName>>(new Set());
  const [aux, setAux] = useState(false);
  const [grabbing, setGrabbing] = useState(false);

  // The latest props, for the window listeners and timers.
  const live = useRef({ client, lanyard });
  live.current = { client, lanyard };
  const held = useRef<Held>(new Map());
  /** Which pad button each pointer holds down (by `pointerId`). */
  const pointers = useRef(new Map<number, ButtonName>());
  const auxOn = useRef(false);
  const dragging = useRef(false);
  const taps = useRef(new Set<ReturnType<typeof setTimeout>>());

  const geo = useMemo(() => ringGeometry(size), [size]);

  // A new lanyard (a layout flip) mid-drag: the old one lets go, and so does the badge.
  useEffect(
    () => () => {
      if (!dragging.current) return;
      dragging.current = false;
      lanyard?.release();
      setGrabbing(false);
    },
    [lanyard],
  );

  // The screen canvas goes to `onCanvas` once per element, through a stable
  // ref callback: an inline `onCanvas` never sees a null/canvas churn.
  const onCanvasLatest = useRef(onCanvas);
  onCanvasLatest.current = onCanvas;
  const canvas = useMemo(() => {
    let el: HTMLCanvasElement | null = null;
    let delivered: HTMLCanvasElement | null | undefined;
    const deliver = () => {
      const cb = onCanvasLatest.current;
      if (!cb || delivered === el) return;
      delivered = el;
      cb(el);
    };
    const ref = (next: HTMLCanvasElement | null) => {
      el = next;
      deliver();
    };
    return { ref, deliver };
  }, []);
  // A callback that arrives after the canvas mounted still gets it.
  useEffect(() => canvas.deliver());

  // Press/release by source; `down` (the drawn state) is any source holding.
  const api = useMemo(() => {
    const sync = () => setDown(new Set(held.current.keys()));
    const press = (name: ButtonName, source: ButtonSource) => {
      const sources = held.current.get(name) ?? new Set<ButtonSource>();
      if (sources.has(source)) return;
      sources.add(source);
      held.current.set(name, sources);
      live.current.client?.injectButton(name, "pressed", source);
      sync();
    };
    const release = (name: ButtonName, source: ButtonSource) => {
      const sources = held.current.get(name);
      if (!sources?.has(source)) return;
      sources.delete(source);
      if (sources.size === 0) held.current.delete(name);
      live.current.client?.injectButton(name, "released", source);
      sync();
    };
    const releaseAll = (source: ButtonSource) => {
      for (const [name, sources] of [...held.current]) if (sources.has(source)) release(name, source);
    };
    const pressPointer = (name: ButtonName, pointerId: number) => {
      pointers.current.set(pointerId, name);
      press(name, "pad");
    };
    /** A pointer lifted: its button lets go, unless another pointer still holds it. */
    const releasePointer = (pointerId: number) => {
      const name = pointers.current.get(pointerId);
      if (name === undefined) return;
      pointers.current.delete(pointerId);
      if (![...pointers.current.values()].includes(name)) release(name, "pad");
    };
    const toggleAux = () => {
      auxOn.current = !auxOn.current;
      live.current.client?.injectButton("AUX1", auxOn.current ? "pressed" : "released", "pad");
      setAux(auxOn.current);
    };
    const tap = (name: ButtonName) => {
      press(name, "pad");
      const t = setTimeout(() => {
        taps.current.delete(t);
        release(name, "pad");
      }, TAP_MS);
      taps.current.add(t);
    };
    const endDrag = () => {
      if (!dragging.current) return;
      dragging.current = false;
      live.current.lanyard?.release();
      setGrabbing(false);
    };
    return { press, release, releaseAll, pressPointer, releasePointer, toggleAux, tap, endDrag };
  }, []);

  // Window listeners: the drag, pad releases, the keyboard and visibility.
  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const stage = stageRef.current;
      if (!dragging.current || !stage) return;
      live.current.lanyard?.drag(toStage(stage, e));
    };
    const onUp = (e: PointerEvent) => {
      api.endDrag();
      api.releasePointer(e.pointerId);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const action = keyAction(e.code, e.target);
      if (!action) return;
      e.preventDefault();
      if (e.repeat) return;
      if ("button" in action) api.press(action.button, "key");
      else if ("toggle" in action) api.toggleAux();
      else live.current.lanyard?.impulse(action.impulse);
    };
    const onKeyUp = (e: KeyboardEvent) => {
      const action = keyAction(e.code, e.target);
      if (action && "button" in action) api.release(action.button, "key");
    };
    const onBlur = () => api.releaseAll("key");
    const onVisibility = () => {
      if (document.visibilityState === "hidden") api.endDrag();
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    window.addEventListener("lostpointercapture", api.endDrag);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", onVisibility);
    const pending = taps.current;
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
      window.removeEventListener("lostpointercapture", api.endDrag);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", onVisibility);
      for (const t of pending) clearTimeout(t);
      pending.clear();
      api.endDrag();
    };
  }, [api, stageRef]);

  const onBodyDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if ((e.target as Element).closest?.("button")) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    const stage = stageRef.current;
    if (!stage || !lanyard) return;
    if (!lanyard.grab(toStage(stage, e))) return;
    e.preventDefault();
    dragging.current = true;
    setGrabbing(true);
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // No capture (a synthetic event): the window listeners still end the drag.
    }
  };

  return (
    <div
      ref={(el) => {
        parts.root = el;
      }}
      className={grabbing ? "badge grabbing" : "badge"}
      onPointerDown={onBodyDown}
      style={{ position: "absolute", left: 0, top: 0, width: size.bw, height: size.bh, transformOrigin: "50% 50%" }}
    >
      <div aria-hidden="true" style={{ position: "absolute", inset: -28, borderRadius: 32 }} />
      <svg
        aria-hidden="true"
        style={{
          position: "absolute",
          left: 0,
          top: geo.svgTop,
          width: "100%",
          height: geo.svgH,
          overflow: "visible",
          pointerEvents: "none",
        }}
      >
        {[geo.h1x, geo.h2x].map((hx) => (
          <line key={`s${hx}`} x1={geo.px} y1={0} x2={hx} y2={geo.hy} stroke="#1a1a1a" strokeWidth={3.2} strokeLinecap="round" />
        ))}
        {[geo.h1x, geo.h2x].map((hx) => (
          <line key={`w${hx}`} x1={geo.px} y1={0} x2={hx} y2={geo.hy} stroke="#9a9aa2" strokeWidth={1.4} strokeLinecap="round" />
        ))}
        {[geo.h1x, geo.h2x].map((hx) => (
          <circle key={`r${hx}`} cx={hx} cy={geo.ringY} r={geo.ringR} fill="none" stroke="#c7c7cf" strokeWidth={2.4} />
        ))}
        <rect x={geo.claspX} y={-15} width={16} height={13} rx={3} fill="#2b2b30" stroke="#55555c" strokeWidth={1} />
        <circle cx={geo.px} cy={0} r={6} fill="none" stroke="#d8d8de" strokeWidth={2.6} />
      </svg>
      <img
        src={PHOTO.src}
        alt="Hack the North 2026 hacker badge, front"
        draggable={false}
        style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", pointerEvents: "none", userSelect: "none" }}
      />
      <canvas
        ref={canvas.ref}
        width={320}
        height={240}
        aria-label="Badge screen"
        role="img"
        style={{
          position: "absolute",
          left: `${SCREEN.left}%`,
          top: `${SCREEN.top}%`,
          width: `${SCREEN.width}%`,
          height: `${SCREEN.height}%`,
          imageRendering: "pixelated",
          pointerEvents: "none",
          background: "#000",
        }}
      />
      {LED_SPOTS.map(([x, y], i) => (
        <div
          key={`led${i}`}
          ref={(el) => {
            parts.leds[i] = el;
          }}
          className="led"
          style={{ left: `${x}%`, top: `${y}%` }}
        />
      ))}
      {BUTTON_ORDER.map((name) => {
        const spot = BUTTON_SPOTS[name];
        const isAux = name === "AUX1";
        const on = isAux ? aux : down.has(name);
        return (
          <button
            key={name}
            type="button"
            className={`hit${isAux ? " side" : ""}${on ? " is-down" : ""}`}
            style={{ left: `${spot.x}%`, top: `${spot.y}%`, width: `${spot.s[layout]}%` }}
            aria-label={isAux ? `AUX1 switch, ${aux ? "on" : "off"}` : `Badge button ${name}`}
            aria-pressed={isAux ? aux : undefined}
            title={isAux ? `AUX1 ${aux ? "ON" : "OFF"}` : name}
            onPointerDown={(e) => {
              e.stopPropagation();
              if (e.pointerType === "mouse" && e.button !== 0) return;
              if (isAux) api.toggleAux();
              else api.pressPointer(name, e.pointerId);
            }}
            onClick={(e) => {
              if (e.detail !== 0) return;
              if (isAux) api.toggleAux();
              else api.tap(name);
            }}
          />
        );
      })}
      <button
        ref={(el) => {
          parts.power = el;
        }}
        type="button"
        className="hit power"
        style={{ left: `${POWER.left}%`, top: `${POWER.top}%`, width: `${POWER.width}%`, height: `${POWER.height}%` }}
        aria-label="Power switch: reboot the badge"
        title="Power · reboot"
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          client?.powerCycle();
        }}
      />
    </div>
  );
}

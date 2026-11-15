import { afterAll, afterEach, beforeAll, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Point } from "../src/motion/lanyard";
import type { ButtonName, ButtonSource } from "../src/runtime/buttons";
import type { BadgeClient, BadgeParts } from "../src/ui/Badge";
import { SCREEN } from "../src/ui/badge-geometry";

/**
 * The badge and the stage on a happy-dom `Window`, installed as the global
 * `window` and `document` for this file only (as app-shell.test.tsx does).
 */
const GLOBALS = ["window", "document", "navigator", "IS_REACT_ACT_ENVIRONMENT"] as const;

let win: Window;
const saved = new Map<string, PropertyDescriptor | undefined>();
let rtl: typeof import("@testing-library/react/pure");
let Badge: typeof import("../src/ui/Badge").Badge;
let Stage: typeof import("../src/ui/Stage").Stage;
let ledGlow: typeof import("../src/ui/Stage").ledGlow;
let LED_GAMMA: number;
let LED_MIN_OPACITY: number;

beforeAll(async () => {
  win = new Window({ url: "http://localhost:5173/", width: 1440, height: 900 });
  const values: Record<(typeof GLOBALS)[number], unknown> = {
    window: win,
    document: win.document,
    navigator: win.navigator,
    IS_REACT_ACT_ENVIRONMENT: true,
  };
  for (const name of GLOBALS) {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value: values[name], configurable: true, writable: true });
  }
  rtl = await import("@testing-library/react/pure");
  ({ Badge } = await import("../src/ui/Badge"));
  ({ Stage, ledGlow, LED_GAMMA, LED_MIN_OPACITY } = await import("../src/ui/Stage"));
});

afterEach(() => {
  rtl.cleanup();
});

afterAll(async () => {
  for (const name of GLOBALS) {
    const d = saved.get(name);
    if (d) Object.defineProperty(globalThis, name, d);
    else delete (globalThis as Record<string, unknown>)[name];
  }
  await win.happyDOM.close();
});

type Call = [ButtonName, "pressed" | "released", ButtonSource];

function fakeClient() {
  const calls: Call[] = [];
  const ledListeners = new Set<(rgb: Uint8Array) => void>();
  let powerCycles = 0;
  let rebooting = false;
  const client: BadgeClient = {
    injectButton: (b, k, s) => void calls.push([b, k, s]),
    powerCycle: () => void powerCycles++,
    rebooting: () => rebooting,
    onLeds(cb) {
      ledListeners.add(cb);
      return () => ledListeners.delete(cb);
    },
  };
  return {
    client,
    calls,
    powerCycles: () => powerCycles,
    setRebooting: (v: boolean) => (rebooting = v),
    emitLeds: (rgb: Uint8Array) => ledListeners.forEach((cb) => cb(rgb)),
  };
}

function fakeLanyard() {
  const log: string[] = [];
  return {
    log,
    lanyard: {
      grab: (p: Point) => (log.push(`grab ${p.x},${p.y}`), true),
      drag: (p: Point) => void log.push(`drag ${p.x},${p.y}`),
      release: () => void log.push("release"),
      impulse: (k: "shake" | "knock") => void log.push(`impulse ${k}`),
    },
  };
}

function renderBadge(layout: "desktop" | "mobile" = "desktop") {
  const fc = fakeClient();
  const fl = fakeLanyard();
  const stage = document.createElement("div");
  document.body.appendChild(stage);
  const parts: BadgeParts = { root: null, leds: [], power: null };
  const view = rtl.render(
    <Badge
      layout={layout}
      size={{ bw: 354, bh: 550, clip: 71 }}
      client={fc.client}
      lanyard={fl.lanyard}
      stageRef={{ current: stage }}
      parts={parts}
    />,
    { container: stage },
  );
  return { ...fc, ...fl, view, parts };
}

const pointer = (type: string, init: Record<string, unknown> = {}) =>
  new win.PointerEvent(type, { bubbles: true, cancelable: true, pointerType: "mouse", button: 0, pointerId: 1, ...init });

describe("Badge", () => {
  test("a held board button presses and releases through the pad", () => {
    const b = renderBadge();
    const a = b.view.getByRole("button", { name: "Badge button A" });
    rtl.act(() => void a.dispatchEvent(pointer("pointerdown") as unknown as Event));
    expect(b.calls).toEqual([["A", "pressed", "pad"]]);
    expect(a.className).toContain("is-down");
    rtl.act(() => void a.dispatchEvent(pointer("pointerup") as unknown as Event));
    expect(b.calls).toEqual([
      ["A", "pressed", "pad"],
      ["A", "released", "pad"],
    ]);
    expect(a.className).not.toContain("is-down");
  });

  test("a button released over another hit target still lets go", () => {
    const b = renderBadge();
    const up = b.view.getByRole("button", { name: "Badge button UP" });
    const left = b.view.getByRole("button", { name: "Badge button LEFT" });
    rtl.act(() => void up.dispatchEvent(pointer("pointerdown") as unknown as Event));
    rtl.act(() => void left.dispatchEvent(pointer("pointerup") as unknown as Event));
    expect(b.calls).toEqual([
      ["UP", "pressed", "pad"],
      ["UP", "released", "pad"],
    ]);
    expect(up.className).not.toContain("is-down");
  });

  test("a new lanyard mid-drag ends the drag", () => {
    const fl2 = fakeLanyard();
    const stage = document.createElement("div");
    document.body.appendChild(stage);
    const parts: BadgeParts = { root: null, leds: [], power: null };
    const first = fakeLanyard();
    const props = { layout: "desktop" as const, size: { bw: 354, bh: 550, clip: 71 }, client: null, stageRef: { current: stage }, parts };
    const view = rtl.render(<Badge {...props} lanyard={first.lanyard} />, { container: stage });
    rtl.act(() => void parts.root!.dispatchEvent(pointer("pointerdown") as unknown as Event));
    expect(parts.root!.className).toContain("grabbing");
    view.rerender(<Badge {...props} lanyard={fl2.lanyard} />);
    expect(first.log.at(-1)).toBe("release");
    expect(parts.root!.className).not.toContain("grabbing");
    // Moves no longer drag either lanyard.
    rtl.act(() => void window.dispatchEvent(pointer("pointermove", { clientX: 5, clientY: 5 }) as unknown as Event));
    expect(fl2.log).toEqual([]);
    expect(first.log.filter((l) => l.startsWith("drag"))).toEqual([]);
  });

  test("an inline onCanvas gets the canvas once, not on every render", () => {
    const stage = document.createElement("div");
    document.body.appendChild(stage);
    const parts: BadgeParts = { root: null, leds: [], power: null };
    const seen: (HTMLCanvasElement | null)[] = [];
    const props = { layout: "desktop" as const, client: null, lanyard: null, stageRef: { current: stage }, parts };
    const view = rtl.render(<Badge {...props} size={{ bw: 354, bh: 550, clip: 71 }} onCanvas={(c) => seen.push(c)} />, {
      container: stage,
    });
    view.rerender(<Badge {...props} size={{ bw: 300, bh: 466, clip: 60 }} onCanvas={(c) => seen.push(c)} />);
    view.rerender(<Badge {...props} size={{ bw: 300, bh: 466, clip: 60 }} onCanvas={(c) => seen.push(c)} />);
    expect(seen.length).toBe(1);
    expect(seen[0]?.tagName).toBe("CANVAS");
  });

  test("a pointer up elsewhere releases a held button", () => {
    const b = renderBadge();
    const up = b.view.getByRole("button", { name: "Badge button UP" });
    rtl.act(() => void up.dispatchEvent(pointer("pointerdown") as unknown as Event));
    rtl.act(() => void window.dispatchEvent(pointer("pointercancel") as unknown as Event));
    expect(b.calls).toEqual([
      ["UP", "pressed", "pad"],
      ["UP", "released", "pad"],
    ]);
  });

  test("a second finger lifting off releases only its own button", () => {
    const b = renderBadge();
    const up = b.view.getByRole("button", { name: "Badge button UP" });
    const a = b.view.getByRole("button", { name: "Badge button A" });
    const touch = (type: string, id: number) => pointer(type, { pointerType: "touch", pointerId: id });
    rtl.act(() => void up.dispatchEvent(touch("pointerdown", 7) as unknown as Event));
    rtl.act(() => void a.dispatchEvent(touch("pointerdown", 8) as unknown as Event));
    // A third finger taps the photo and lifts: nothing it pressed.
    rtl.act(() => void window.dispatchEvent(touch("pointerup", 9) as unknown as Event));
    rtl.act(() => void window.dispatchEvent(touch("pointerup", 8) as unknown as Event));
    expect(b.calls).toEqual([
      ["UP", "pressed", "pad"],
      ["A", "pressed", "pad"],
      ["A", "released", "pad"],
    ]);
    expect(up.className).toContain("is-down");
    rtl.act(() => void window.dispatchEvent(touch("pointercancel", 7) as unknown as Event));
    expect(b.calls.at(-1)).toEqual(["UP", "released", "pad"]);
  });

  test("two fingers on one button hold it until both lift", () => {
    const b = renderBadge();
    const up = b.view.getByRole("button", { name: "Badge button UP" });
    const touch = (type: string, id: number) => pointer(type, { pointerType: "touch", pointerId: id });
    rtl.act(() => void up.dispatchEvent(touch("pointerdown", 1) as unknown as Event));
    rtl.act(() => void up.dispatchEvent(touch("pointerdown", 2) as unknown as Event));
    rtl.act(() => void window.dispatchEvent(touch("pointerup", 1) as unknown as Event));
    expect(b.calls).toEqual([["UP", "pressed", "pad"]]);
    rtl.act(() => void window.dispatchEvent(touch("pointerup", 2) as unknown as Event));
    expect(b.calls).toEqual([
      ["UP", "pressed", "pad"],
      ["UP", "released", "pad"],
    ]);
  });

  test("a hidden page or a window blur releases pad buttons a lost touch-up left held", () => {
    for (const lose of ["hidden", "blur"] as const) {
      const b = renderBadge();
      const up = b.view.getByRole("button", { name: "Badge button UP" });
      const touch = (type: string, id: number) => pointer(type, { pointerType: "touch", pointerId: id });
      // Finger 3's up and cancel both get lost.
      rtl.act(() => void up.dispatchEvent(touch("pointerdown", 3) as unknown as Event));
      if (lose === "hidden") {
        Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
        rtl.act(() => void document.dispatchEvent(new win.Event("visibilitychange") as unknown as Event));
        delete (document as unknown as Record<string, unknown>).visibilityState;
      } else {
        rtl.act(() => void window.dispatchEvent(new win.Event("blur") as unknown as Event));
      }
      expect(b.calls).toEqual([
        ["UP", "pressed", "pad"],
        ["UP", "released", "pad"],
      ]);
      expect(up.className).not.toContain("is-down");
      // A new finger presses and lifts as normal: the stale pointer is gone.
      rtl.act(() => void up.dispatchEvent(touch("pointerdown", 4) as unknown as Event));
      rtl.act(() => void window.dispatchEvent(touch("pointerup", 4) as unknown as Event));
      expect(b.calls.slice(2)).toEqual([
        ["UP", "pressed", "pad"],
        ["UP", "released", "pad"],
      ]);
      expect(up.className).not.toContain("is-down");
      rtl.cleanup();
    }
  });

  test("the badge's accessible description lists the keyboard shortcuts", () => {
    const b = renderBadge();
    const group = b.view.getByRole("group", { name: "Badge" });
    const id = group.getAttribute("aria-describedby");
    expect(id).toBeTruthy();
    const desc = document.getElementById(id!)!;
    expect(desc.className).toContain("sr-only");
    for (const k of ["arrow keys", "Z presses A", "X presses B", "Enter", "Escape", "Space", "AUX1", "S shakes", "K knocks"]) {
      expect(desc.textContent).toContain(k);
    }
  });

  test("with phone motion on, the description leaves out S and K", () => {
    const stage = document.createElement("div");
    document.body.appendChild(stage);
    const parts: BadgeParts = { root: null, leds: [], power: null };
    const view = rtl.render(
      <Badge
        layout="mobile"
        size={{ bw: 354, bh: 550, clip: 71 }}
        client={null}
        lanyard={null}
        stageRef={{ current: stage }}
        parts={parts}
        phoneMotion
      />,
      { container: stage },
    );
    const group = view.getByRole("group", { name: "Badge" });
    const desc = document.getElementById(group.getAttribute("aria-describedby")!)!;
    expect(desc.textContent).toContain("Escape presses HOME");
    expect(desc.textContent).not.toContain("S shakes");
    expect(desc.textContent).not.toContain("K knocks");
  });

  test("AUX1 toggles and reports its state", () => {
    const b = renderBadge();
    const aux = b.view.getByRole("button", { name: "AUX1 switch, off" });
    expect(aux.getAttribute("aria-pressed")).toBe("false");
    rtl.act(() => void aux.dispatchEvent(pointer("pointerdown") as unknown as Event));
    rtl.act(() => void aux.dispatchEvent(pointer("pointerup") as unknown as Event));
    expect(aux.getAttribute("aria-pressed")).toBe("true");
    expect(aux.getAttribute("aria-label")).toBe("AUX1 switch, on");
    rtl.act(() => void aux.dispatchEvent(pointer("pointerdown") as unknown as Event));
    expect(aux.getAttribute("aria-pressed")).toBe("false");
    expect(b.calls).toEqual([
      ["AUX1", "pressed", "pad"],
      ["AUX1", "released", "pad"],
    ]);
  });

  test("a keyboard click taps the button", async () => {
    const b = renderBadge();
    const start = b.view.getByRole("button", { name: "Badge button START" });
    rtl.act(() => void start.dispatchEvent(new win.MouseEvent("click", { bubbles: true, detail: 0 }) as unknown as Event));
    expect(b.calls).toEqual([["START", "pressed", "pad"]]);
    await rtl.act(() => new Promise((r) => setTimeout(r, 200)));
    expect(b.calls).toEqual([
      ["START", "pressed", "pad"],
      ["START", "released", "pad"],
    ]);
  });

  test("power reboots the badge", () => {
    const b = renderBadge();
    const power = b.view.getByRole("button", { name: "Power switch: reboot the badge" });
    rtl.act(() => void power.dispatchEvent(new win.MouseEvent("click", { bubbles: true, detail: 1 }) as unknown as Event));
    expect(b.powerCycles()).toBe(1);
  });

  test("dragging the body grabs the lanyard and pointercancel or lostpointercapture lets go", () => {
    const b = renderBadge();
    const root = b.parts.root!;
    rtl.act(() => void root.dispatchEvent(pointer("pointerdown", { clientX: 100, clientY: 200 }) as unknown as Event));
    expect(b.log[0]).toStartWith("grab");
    expect(root.className).toContain("grabbing");
    rtl.act(() => void window.dispatchEvent(pointer("pointermove", { clientX: 120, clientY: 210 }) as unknown as Event));
    expect(b.log[1]).toStartWith("drag");
    rtl.act(() => void window.dispatchEvent(pointer("lostpointercapture") as unknown as Event));
    expect(b.log.at(-1)).toBe("release");
    expect(root.className).not.toContain("grabbing");

    rtl.act(() => void root.dispatchEvent(pointer("pointerdown") as unknown as Event));
    rtl.act(() => void window.dispatchEvent(pointer("pointercancel") as unknown as Event));
    expect(b.log.filter((l) => l === "release").length).toBe(2);
    // A button press never grabs.
    const n = b.log.length;
    const a = b.view.getByRole("button", { name: "Badge button A" });
    rtl.act(() => void a.dispatchEvent(pointer("pointerdown") as unknown as Event));
    expect(b.log.length).toBe(n);
  });

  test("the keyboard presses buttons, toggles AUX1 and pushes the lanyard", () => {
    const b = renderBadge();
    const key = (type: string, code: string, target: EventTarget = window as unknown as EventTarget) =>
      rtl.act(() => void target.dispatchEvent(new win.KeyboardEvent(type, { code, bubbles: true, cancelable: true }) as unknown as Event));
    key("keydown", "KeyZ");
    key("keyup", "KeyZ");
    key("keydown", "Space");
    key("keydown", "KeyS");
    key("keydown", "KeyK");
    expect(b.calls).toEqual([
      ["A", "pressed", "key"],
      ["A", "released", "key"],
      ["AUX1", "pressed", "pad"],
    ]);
    expect(b.log).toEqual(["impulse shake", "impulse knock"]);
    const input = document.createElement("input");
    document.body.appendChild(input);
    key("keydown", "KeyZ", input as unknown as EventTarget);
    expect(b.calls.length).toBe(3);
    input.remove();
  });

  test("the screen canvas is drawn level in the LCD (Ruling R18)", () => {
    const b = renderBadge();
    const canvas = b.view.getByRole("img", { name: "Badge screen" });
    expect(canvas.style.transform).toBe("");
    expect(canvas.style.left).toBe(`${SCREEN.left}%`);
    expect("rotateDeg" in SCREEN).toBe(false);
  });

  test("mobile hit targets are bigger", () => {
    const desk = renderBadge("desktop");
    const dw = desk.view.getByRole("button", { name: "Badge button A" }).style.width;
    rtl.cleanup();
    const mob = renderBadge("mobile");
    const mw = mob.view.getByRole("button", { name: "Badge button A" }).style.width;
    expect(dw).toBe("10.38%");
    expect(mw).toBe("12.59%");
  });
});

describe("Stage", () => {
  test("LED glows follow the firmware's frames, and power shows the reboot", async () => {
    const fc = fakeClient();
    const view = rtl.render(<Stage client={fc.client} />);
    const leds = [...view.container.querySelectorAll<HTMLElement>(".led")];
    expect(leds.length).toBe(6);
    // Chain 0 green at 0x42, chain 2 off, the rest dark.
    const rgb = new Uint8Array(18);
    rgb[1] = 0x42;
    rgb[6] = 0x30;
    rtl.act(() => fc.emitLeds(rgb));
    fc.setRebooting(true);
    await rtl.act(() => new Promise((r) => setTimeout(r, 100)));
    expect(leds[0].style.opacity).toBe(ledGlow(0, 0x42, 0).opacity.toFixed(3));
    expect(leds[0].style.opacity).toBe("0.702");
    expect(leds[0].style.getPropertyValue("--rgb")).toBe("0,255,0");
    // Chain 2 is LED_SPOTS[3] (the mid-right spot).
    expect(leds[3].style.opacity).toBe(ledGlow(0, 0x30, 0).opacity.toFixed(3));
    expect(leds[1].style.opacity).toBe("0.000");
    expect(view.getByRole("button", { name: "Power switch: reboot the badge" }).className).toContain("is-down");
    expect(view.getByText("DRAG THE BADGE TO SWING IT · PRESS THE BUTTONS ON THE BOARD")).toBeTruthy();
    expect(view.getByRole("button", { name: "Reset swing" })).toBeTruthy();
    // The desktop cap, so the rope can still reach the walls.
    expect(view.container.querySelector<HTMLElement>("[data-slot=stage]")!.style.maxWidth).toBe("1016px");
  });

  test("the hint lists the shortcuts, and phone motion has its own hint without S and K", () => {
    const drag = rtl.render(<Stage />);
    const keys = drag.container.querySelector("[data-slot=shortcuts]")!;
    expect(keys.textContent).toContain("Z/X A/B");
    expect(keys.textContent).toContain("S SHAKE");
    expect(keys.textContent).toContain("K KNOCK");
    rtl.cleanup();
    const phone = rtl.render(<Stage phoneMotion />);
    expect(phone.queryByText(/DRAG/)).toBeNull();
    expect(phone.getByText("MOVE THE DEVICE TO SWING IT · PRESS THE BUTTONS ON THE BOARD")).toBeTruthy();
    const phoneKeys = phone.container.querySelector("[data-slot=shortcuts]")!;
    expect(phoneKeys.textContent).toContain("SPACE AUX1");
    expect(phoneKeys.textContent).not.toContain("SHAKE");
    expect(phoneKeys.textContent).not.toContain("KNOCK");
    const desc = document.getElementById(phone.getByRole("group", { name: "Badge" }).getAttribute("aria-describedby")!)!;
    expect(desc.textContent).not.toContain("S shakes");
  });

  test("ledGlow scales the colour to full and boosts the brightness for the eye (R17)", () => {
    expect(ledGlow(0, 0, 0)).toEqual({ rgb: "255,255,255", opacity: 0 });
    const g = ledGlow(0x0c, 0x42, 0x0e);
    expect(g.rgb).toBe("46,255,54");
    expect(g.opacity).toBeCloseTo(LED_MIN_OPACITY + (1 - LED_MIN_OPACITY) * (0x42 / 255) ** (1 / LED_GAMMA), 12);
    // The firmware's real levels all read clearly lit: prov apply's 0x06
    // flash, the role colour's 0x1F, Mood LEDs' 0x21, Setup's 0x42.
    expect(ledGlow(0, 0x06, 0).opacity).toBeGreaterThan(0.45);
    expect(ledGlow(6, 0x1f, 6).opacity).toBeGreaterThan(0.55);
    expect(ledGlow(0, 0x42, 0).opacity).toBeCloseTo(0.7, 2);
    expect(ledGlow(255, 255, 255).opacity).toBe(1);
    // Brighter is never dimmer.
    let prev = 0;
    for (let v = 1; v <= 255; v++) {
      const o = ledGlow(v, 0, 0).opacity;
      expect(o).toBeGreaterThan(prev);
      prev = o;
    }
  });
});

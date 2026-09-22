/**
 * `badge.led` — 6-LED strip stub. Keeps an in-memory RGB array the UI shell
 * can render as small colored dots; `show()` is a no-op since the emulator
 * has no separate "commit to hardware" step (the array itself is the
 * displayed state).
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";

export const LED_COUNT = 6;
export type Rgb = [number, number, number];

export interface LedModule {
  attach(L: LuaState): void;
  getColors(): Rgb[];
}

function clamp255(n: number): number {
  const v = Math.round(Number(n) || 0);
  return Math.max(0, Math.min(255, v));
}

export function createLedModule(): LedModule {
  const colors: Rgb[] = Array.from({ length: LED_COUNT }, () => [0, 0, 0]);

  const api = {
    set(index: number, r: number, g: number, b: number) {
      const i = Math.trunc(Number(index) || 0) - 1;
      if (i < 0 || i >= LED_COUNT) return;
      colors[i] = [clamp255(r), clamp255(g), clamp255(b)];
    },
    set_all(r: number, g: number, b: number) {
      const c: Rgb = [clamp255(r), clamp255(g), clamp255(b)];
      for (let i = 0; i < LED_COUNT; i++) colors[i] = [...c];
    },
    clear() {
      for (let i = 0; i < LED_COUNT; i++) colors[i] = [0, 0, 0];
    },
    show() {
      // No-op: the emulator has no hardware "commit" step.
    },
    count() {
      return LED_COUNT;
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
    getColors() {
      return colors.map((c) => [...c] as Rgb);
    },
  };
}

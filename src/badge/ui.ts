/**
 * `badge.ui` — a retained-mode LVGL-ish widget tree.
 *
 * Widgets are plain JS objects (see `Widget`) forming a tree. Each widget is
 * exposed to Lua as a fresh table-of-closures (per the milestone spec: "use
 * Fengari's metatable/interop support, or a table-of-closures-per-instance
 * approach, whichever is more robust" — we use the latter, since it needs no
 * metatable machinery and every method closes directly over its JS widget).
 *
 * A widget's Lua table carries one plain field, `__wid` (its numeric id), so
 * that *other* API calls which take a widget as an argument (e.g.
 * `label(parent, text)`) can resolve the Lua table back to the JS `Widget`
 * instance via `registry`. Method calls don't need this: `w:set_text(...)`
 * ignores the Lua `self` argument entirely and uses the JS closure's own
 * `widget` reference instead.
 *
 * This module also exports the plain widget-tree data (`UiModule.root`) so
 * `src/render/canvas.ts` can walk and paint it without touching Lua at all.
 */
import { lua, lauxlib, to_luastring } from "fengari";
import type { LuaState } from "../lua/interop";
import { luaToJs } from "../lua/interop";

export const SCREEN_WIDTH = 320;
export const SCREEN_HEIGHT = 240;
const MAX_LINE_POINTS = 128;

export type WidgetType =
  | "root"
  | "label"
  | "textarea"
  | "box"
  | "button"
  | "bar"
  | "arc"
  | "slider"
  | "image"
  | "line"
  | "switch"
  | "checkbox"
  | "roller";

/**
 * Style properties from the spec. Anything not explicitly listed (e.g. any
 * `pad_*`/`shadow_*` key) is still accepted and stored verbatim thanks to
 * the index signature — `w:style({...})` never rejects an unknown key, it
 * just may not be rendered with special fidelity by canvas.ts.
 */
export interface WidgetStyle {
  bg_color?: number;
  bg_opa?: number;
  color?: number;
  opa?: number;
  radius?: number;
  border_color?: number;
  border_opa?: number;
  border_width?: number;
  text_color?: number;
  text_opa?: number;
  text_font?: string;
  text_align?: string;
  arc_color?: number;
  arc_opa?: number;
  arc_width?: number;
  line_color?: number;
  line_opa?: number;
  line_width?: number;
  flex_flow?: string;
  [key: string]: unknown;
}

export interface WidgetAlign {
  name: string;
  dx: number;
  dy: number;
}

export interface Widget {
  id: number;
  type: WidgetType;
  parent: Widget | null;
  children: Widget[];
  x: number;
  y: number;
  w: number;
  h: number;
  align: WidgetAlign | null;
  hidden: boolean;
  clickable: boolean;
  deleted: boolean;
  style: WidgetStyle;
  styleSelectors: Record<string, WidgetStyle>;
  fontSize: string;
  borderColor: number;
  borderWidth: number;

  // type-specific (all optional; presence depends on `type`)
  text?: string;
  min?: number;
  max?: number;
  value?: number;
  src?: string;
  points?: Array<[number, number]>;
  checked?: boolean;
  options?: string[];
  selected?: number;
}

export interface UiModule {
  root: Widget;
  /** Pushes the `badge.ui` table onto the Lua stack top. */
  attach(L: LuaState): void;
  /** Pushes a Lua wrapper table for an arbitrary widget (used for `on_enter(root)`). */
  pushWidget(L: LuaState, widget: Widget): void;
}

let widgetIdSeq = 0;

function makeWidget(type: WidgetType, parent: Widget | null): Widget {
  const w: Widget = {
    id: ++widgetIdSeq,
    type,
    parent,
    children: [],
    x: 0,
    y: 0,
    w: 0,
    h: 0,
    align: null,
    hidden: false,
    clickable: type === "button" || type === "switch" || type === "checkbox" || type === "roller" || type === "slider",
    deleted: false,
    style: {},
    styleSelectors: {},
    fontSize: "normal",
    borderColor: 0x000000,
    borderWidth: 0,
  };
  if (parent) parent.children.push(w);
  return w;
}

function argNum(L: LuaState, idx: number): number {
  const n = lua.lua_tonumber(L, idx);
  return typeof n === "number" && !Number.isNaN(n) ? n : 0;
}
function argInt(L: LuaState, idx: number): number {
  const n = lua.lua_tointeger(L, idx);
  return typeof n === "number" && !Number.isNaN(n) ? n : 0;
}
function argStr(L: LuaState, idx: number): string {
  const s = lua.lua_tojsstring(L, idx);
  return typeof s === "string" ? s : "";
}
function argBool(L: LuaState, idx: number): boolean {
  return lua.lua_toboolean(L, idx) !== 0;
}
function luaErrorStr(L: LuaState, msg: string): number {
  return lauxlib.luaL_error(L, to_luastring("%s"), to_luastring(msg));
}

function normalizePoints(raw: unknown): Array<[number, number]> {
  if (!Array.isArray(raw)) return [];
  const pts: Array<[number, number]> = [];
  for (const p of raw.slice(0, MAX_LINE_POINTS)) {
    if (Array.isArray(p) && p.length >= 2) {
      pts.push([Number(p[0]) || 0, Number(p[1]) || 0]);
    }
  }
  return pts;
}

export function createUiModule(): UiModule {
  const root = makeWidget("root", null);
  root.w = SCREEN_WIDTH;
  root.h = SCREEN_HEIGHT;

  /** Every widget that has been handed to Lua at least once, keyed by id. */
  const registry = new Map<number, Widget>();
  registry.set(root.id, root);

  /** Recursively removes a widget and all its descendants from `registry`. */
  function removeSubtreeFromRegistry(w: Widget): void {
    registry.delete(w.id);
    for (const child of w.children) removeSubtreeFromRegistry(child);
  }

  function resolveWidgetArg(L: LuaState, idx: number): Widget {
    if (lua.lua_type(L, idx) !== lua.LUA_TTABLE) {
      luaErrorStr(L, "expected a widget (table with __wid) argument");
    }
    lua.lua_getfield(L, idx, to_luastring("__wid"));
    const wid = lua.lua_tointeger(L, -1);
    lua.lua_pop(L, 1);
    const widget = registry.get(wid);
    if (!widget) {
      luaErrorStr(L, `stale or invalid widget reference (id ${wid})`);
    }
    return widget as Widget;
  }

  function pushWidget(L: LuaState, widget: Widget): void {
    registry.set(widget.id, widget);
    lua.lua_newtable(L);
    lua.lua_pushinteger(L, widget.id);
    lua.lua_setfield(L, -2, to_luastring("__wid"));

    const method = (name: string, fn: (L: LuaState, w: Widget) => number) => {
      lua.lua_pushjsfunction(L, (L2: LuaState) => fn(L2, widget));
      lua.lua_setfield(L, -2, to_luastring(name));
    };

    method("set_pos", (L, w) => {
      w.x = argNum(L, 2);
      w.y = argNum(L, 3);
      return 0;
    });
    method("set_size", (L, w) => {
      w.w = argNum(L, 2);
      w.h = argNum(L, 3);
      return 0;
    });
    method("align", (L, w) => {
      w.align = { name: argStr(L, 2), dx: argNum(L, 3), dy: argNum(L, 4) };
      return 0;
    });
    method("parent", (L, w) => {
      if (w.parent) pushWidget(L, w.parent);
      else lua.lua_pushnil(L);
      return 1;
    });
    method("child", (L, w) => {
      const i = argInt(L, 2);
      const child = w.children[i - 1];
      if (child) pushWidget(L, child);
      else lua.lua_pushnil(L);
      return 1;
    });
    method("child_count", (L, w) => {
      lua.lua_pushinteger(L, w.children.length);
      return 1;
    });
    method("type", (L, w) => {
      lua.lua_pushstring(L, to_luastring(w.type));
      return 1;
    });
    method("hidden", (L, w) => {
      w.hidden = argBool(L, 2);
      return 0;
    });
    method("clickable", (L, w) => {
      w.clickable = argBool(L, 2);
      return 0;
    });
    method("bring_to_front", (L, w) => {
      if (w.parent) {
        const i = w.parent.children.indexOf(w);
        if (i >= 0) {
          w.parent.children.splice(i, 1);
          w.parent.children.push(w);
        }
      }
      return 0;
    });
    method("delete", (L, w) => {
      w.deleted = true;
      w.hidden = true;
      if (w.parent) {
        const i = w.parent.children.indexOf(w);
        if (i >= 0) w.parent.children.splice(i, 1);
      }
      removeSubtreeFromRegistry(w);
      return 0;
    });
    method("set_text", (L, w) => {
      w.text = argStr(L, 2);
      return 0;
    });
    method("set_value", (L, w) => {
      w.value = argNum(L, 2);
      return 0;
    });
    method("set_range", (L, w) => {
      w.min = argNum(L, 2);
      w.max = argNum(L, 3);
      return 0;
    });
    method("set_src", (L, w) => {
      w.src = argStr(L, 2);
      return 0;
    });
    method("set_points", (L, w) => {
      w.points = normalizePoints(luaToJs(L, 2));
      return 0;
    });
    method("set_checked", (L, w) => {
      w.checked = argBool(L, 2);
      return 0;
    });
    method("get_checked", (L, w) => {
      lua.lua_pushboolean(L, w.checked ? 1 : 0);
      return 1;
    });
    method("set_options", (L, w) => {
      w.options = argStr(L, 2)
        .split("\n")
        .filter((s) => s.length > 0);
      if (w.selected === undefined) w.selected = 0;
      return 0;
    });
    method("get_selected", (L, w) => {
      lua.lua_pushinteger(L, w.selected ?? 0);
      return 1;
    });
    method("set_color", (L, w) => {
      const c = argInt(L, 2);
      if (w.type === "line") w.style.line_color = c;
      else if (w.type === "arc") w.style.arc_color = c;
      // bar/slider render their value indicator from `style.color` (see
      // canvas.ts's `paintValueFill`), not `style.bg_color` (which is the
      // track background painted underneath it via `paintBox`).
      else if (w.type === "bar" || w.type === "slider") w.style.color = c;
      else w.style.bg_color = c;
      return 0;
    });
    method("set_border", (L, w) => {
      w.borderColor = argInt(L, 2);
      w.borderWidth = argNum(L, 3);
      w.style.border_color = w.borderColor;
      w.style.border_width = w.borderWidth;
      return 0;
    });
    method("set_font_size", (L, w) => {
      const s = argStr(L, 2);
      if (s) {
        w.fontSize = s;
        // `fontPx()` in canvas.ts reads `style.text_font`, not the
        // standalone `fontSize` field -- keep both in sync so this
        // actually changes the rendered text size.
        w.style.text_font = s;
      }
      return 0;
    });
    method("style", (L, w) => {
      const styleObj = (luaToJs(L, 2) as WidgetStyle) || {};
      const argc = lua.lua_gettop(L);
      if (argc >= 3 && lua.lua_type(L, 3) === lua.LUA_TSTRING) {
        const selector = argStr(L, 3);
        w.styleSelectors[selector] = { ...(w.styleSelectors[selector] || {}), ...styleObj };
      } else {
        w.style = { ...w.style, ...styleObj };
      }
      return 0;
    });
  }

  function attach(L: LuaState): void {
    lua.lua_newtable(L);
    lua.lua_pushinteger(L, SCREEN_WIDTH);
    lua.lua_setfield(L, -2, to_luastring("screen_width"));
    lua.lua_pushinteger(L, SCREEN_HEIGHT);
    lua.lua_setfield(L, -2, to_luastring("screen_height"));

    const factory = (name: string, build: (L: LuaState) => Widget) => {
      lua.lua_pushjsfunction(L, (L2: LuaState) => {
        const w = build(L2);
        pushWidget(L2, w);
        return 1;
      });
      lua.lua_setfield(L, -2, to_luastring(name));
    };

    factory("label", (L) => {
      const w = makeWidget("label", resolveWidgetArg(L, 1));
      w.text = argStr(L, 2);
      return w;
    });
    factory("textarea", (L) => {
      const w = makeWidget("textarea", resolveWidgetArg(L, 1));
      w.text = argStr(L, 2);
      return w;
    });
    factory("box", (L) => {
      const w = makeWidget("box", resolveWidgetArg(L, 1));
      w.w = argNum(L, 2);
      w.h = argNum(L, 3);
      return w;
    });
    factory("button", (L) => {
      const w = makeWidget("button", resolveWidgetArg(L, 1));
      w.w = argNum(L, 2);
      w.h = argNum(L, 3);
      return w;
    });
    factory("bar", (L) => {
      const w = makeWidget("bar", resolveWidgetArg(L, 1));
      w.min = argNum(L, 2);
      w.max = argNum(L, 3);
      w.value = argNum(L, 4);
      return w;
    });
    factory("arc", (L) => {
      const w = makeWidget("arc", resolveWidgetArg(L, 1));
      w.min = argNum(L, 2);
      w.max = argNum(L, 3);
      w.value = argNum(L, 4);
      return w;
    });
    factory("slider", (L) => {
      const w = makeWidget("slider", resolveWidgetArg(L, 1));
      w.min = argNum(L, 2);
      w.max = argNum(L, 3);
      w.value = argNum(L, 4);
      return w;
    });
    factory("image", (L) => {
      const w = makeWidget("image", resolveWidgetArg(L, 1));
      w.src = argStr(L, 2);
      return w;
    });
    factory("line", (L) => {
      const w = makeWidget("line", resolveWidgetArg(L, 1));
      w.points = normalizePoints(luaToJs(L, 2));
      return w;
    });
    factory("switch", (L) => {
      const w = makeWidget("switch", resolveWidgetArg(L, 1));
      w.checked = argBool(L, 2);
      return w;
    });
    factory("checkbox", (L) => {
      const w = makeWidget("checkbox", resolveWidgetArg(L, 1));
      w.text = argStr(L, 2);
      w.checked = argBool(L, 3);
      return w;
    });
    factory("roller", (L) => {
      const w = makeWidget("roller", resolveWidgetArg(L, 1));
      w.options = argStr(L, 2)
        .split("\n")
        .filter((s) => s.length > 0);
      w.selected = 0;
      return w;
    });
  }

  return { root, attach, pushWidget };
}

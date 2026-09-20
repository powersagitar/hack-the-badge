/**
 * Fengari ships no TypeScript types. These ambient declarations are
 * intentionally loose (`any`-typed) — we lean on our own wrapper functions
 * in `interop.ts` and `vm.ts` for type safety at the boundary instead of
 * trying to model the full Lua C API in TS.
 */
declare module "fengari" {
  export const lua: any;
  export const lauxlib: any;
  export const lualib: any;
  export const luaconf: any;
  export function to_luastring(s: string): Uint8Array;
  export function to_jsstring(s: Uint8Array | string): string;
}

declare module "fengari-interop" {
  export function luaopen_js(L: any): number;
  export function push(L: any, value: any): void;
  export function pushjs(L: any, value: any): void;
  export function tojs(L: any, idx: number): any;
  export function checkjs(L: any, idx: number): any;
  export function testjs(L: any, idx: number): any;
}

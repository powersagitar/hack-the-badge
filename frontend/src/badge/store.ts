/**
 * `badge.store` — small app-scoped key/value persistence backed by
 * `localStorage` (see `storage.ts` for the fallback used outside a
 * browser), namespaced per app slug so different apps can't see each
 * other's data.
 *
 * Limits per spec: 32 keys max, key names `[A-Za-z0-9_]` up to 24 bytes,
 * string values up to 128 bytes. Values are tagged with a 1-byte-cheap
 * prefix (`n:`/`s:`) so the polymorphic `get()` can tell numbers from
 * strings apart again.
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";
import { byteLength, getStorage } from "./storage";

const KEY_RE = /^[A-Za-z0-9_]{1,24}$/;
export const MAX_KEYS = 32;
export const MAX_VALUE_BYTES = 128;

export function validateStoreKey(key: string): void {
  if (typeof key !== "string" || !KEY_RE.test(key)) {
    throw new Error(`badge.store: invalid key '${key}' (must match [A-Za-z0-9_]{1,24})`);
  }
}

export function createStoreModule(appSlug: string) {
  const prefix = `badge:store:${appSlug}:`;
  const fullKey = (key: string) => prefix + key;

  function listKeys(): string[] {
    const storage = getStorage();
    const keys: string[] = [];
    for (let i = 0; i < storage.length; i++) {
      const k = storage.key(i);
      if (k && k.startsWith(prefix)) keys.push(k.slice(prefix.length));
    }
    return keys;
  }

  function ensureCapacityFor(key: string): void {
    const existing = listKeys();
    if (!existing.includes(key) && existing.length >= MAX_KEYS) {
      throw new Error(`badge.store: key limit of ${MAX_KEYS} reached`);
    }
  }

  function writeTagged(key: string, tagged: string): void {
    validateStoreKey(key);
    if (byteLength(tagged) > MAX_VALUE_BYTES + 2) {
      throw new Error(`badge.store: value for '${key}' exceeds ${MAX_VALUE_BYTES} bytes`);
    }
    ensureCapacityFor(key);
    getStorage().setItem(fullKey(key), tagged);
  }

  function readTagged(key: string): string | null {
    validateStoreKey(key);
    return getStorage().getItem(fullKey(key));
  }

  const api = {
    set(key: string, value: unknown) {
      if (typeof value === "number") writeTagged(key, `n:${value}`);
      else writeTagged(key, `s:${String(value ?? "")}`);
    },
    get(key: string) {
      const raw = readTagged(key);
      if (raw === null) return null;
      if (raw.startsWith("n:")) return Number(raw.slice(2));
      if (raw.startsWith("s:")) return raw.slice(2);
      return raw;
    },
    set_int(key: string, value: number) {
      writeTagged(key, `n:${Math.trunc(Number(value) || 0)}`);
    },
    get_int(key: string) {
      const raw = readTagged(key);
      if (raw === null) return null;
      const stripped =
        raw.startsWith("n:") || raw.startsWith("s:") ? raw.slice(2) : raw;
      const n = Number(stripped);
      return Number.isFinite(n) ? Math.trunc(n) : 0;
    },
    set_str(key: string, value: string) {
      writeTagged(key, `s:${String(value ?? "")}`);
    },
    get_str(key: string) {
      const raw = readTagged(key);
      if (raw === null) return null;
      if (raw.startsWith("s:") || raw.startsWith("n:")) return raw.slice(2);
      return raw;
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

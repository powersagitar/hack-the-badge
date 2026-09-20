/**
 * `badge.fs` — sandboxed per-app virtual filesystem backed by `localStorage`
 * (see `storage.ts`). Paths are relative to the app dir, use the
 * `appdata/`-prefixed key convention internally, and are capped at 64 bytes
 * / 4 path segments deep. 64 KiB total quota per app, 16 KiB per file.
 *
 * Directories are implicit (a "directory" exists if any file lives under
 * it), except `mkdir()` also drops a hidden `<dir>/.dir` marker so an empty
 * directory still `exists()`/shows up via `list()` on its parent.
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";
import { byteLength, getStorage } from "./storage";

export const MAX_PATH_BYTES = 64;
export const MAX_PATH_DEPTH = 4;
export const MAX_FILE_BYTES = 16 * 1024;
export const MAX_TOTAL_BYTES = 64 * 1024;

const DIR_MARKER = ".dir";

export function validateFsPath(path: string): string[] {
  if (typeof path !== "string" || path.length === 0) {
    throw new Error("badge.fs: path must be a non-empty string");
  }
  if (byteLength(path) > MAX_PATH_BYTES) {
    throw new Error(`badge.fs: path exceeds ${MAX_PATH_BYTES} bytes`);
  }
  if (path.startsWith("/") || path.split("/").includes("..")) {
    throw new Error("badge.fs: path must be relative and may not contain '..'");
  }
  const segments = path.split("/").filter((s) => s.length > 0);
  if (segments.length === 0 || segments.length > MAX_PATH_DEPTH) {
    throw new Error(`badge.fs: path depth must be 1-${MAX_PATH_DEPTH} segments`);
  }
  return segments;
}

export function createFsModule(appSlug: string) {
  const prefix = `badge:fs:${appSlug}:appdata/`;
  const storage = () => getStorage();
  const keyFor = (segments: string[]) => prefix + segments.join("/");

  function totalBytesExcluding(excludeKey: string | null): number {
    const s = storage();
    let total = 0;
    for (let i = 0; i < s.length; i++) {
      const k = s.key(i);
      if (k && k.startsWith(prefix) && k !== excludeKey && !k.endsWith(`/${DIR_MARKER}`)) {
        total += byteLength(s.getItem(k) || "");
      }
    }
    return total;
  }

  function writeFile(path: string, data: unknown, append: boolean): void {
    const segs = validateFsPath(path);
    const key = keyFor(segs);
    const base = append ? storage().getItem(key) || "" : "";
    const next = base + String(data ?? "");
    const nextBytes = byteLength(next);
    if (nextBytes > MAX_FILE_BYTES) {
      throw new Error(`badge.fs: '${path}' would exceed the ${MAX_FILE_BYTES}-byte per-file cap`);
    }
    const otherTotal = totalBytesExcluding(key);
    if (otherTotal + nextBytes > MAX_TOTAL_BYTES) {
      throw new Error(`badge.fs: writing '${path}' would exceed the ${MAX_TOTAL_BYTES}-byte app quota`);
    }
    storage().setItem(key, next);
  }

  function readFile(path: string): string | null {
    const segs = validateFsPath(path);
    return storage().getItem(keyFor(segs));
  }

  function exists(path: string): boolean {
    const segs = validateFsPath(path);
    const key = keyFor(segs);
    if (storage().getItem(key) !== null) return true;
    if (storage().getItem(`${key}/${DIR_MARKER}`) !== null) return true;
    const dirPrefix = `${key}/`;
    const s = storage();
    for (let i = 0; i < s.length; i++) {
      const k = s.key(i);
      if (k && k.startsWith(dirPrefix)) return true;
    }
    return false;
  }

  function remove(path: string): void {
    const segs = validateFsPath(path);
    const key = keyFor(segs);
    storage().removeItem(key);
    storage().removeItem(`${key}/${DIR_MARKER}`);
  }

  function list(subdir?: string): string[] {
    const segs = subdir ? validateFsPath(subdir) : [];
    const dirPrefix = segs.length ? `${keyFor(segs)}/` : prefix;
    const names = new Set<string>();
    const s = storage();
    for (let i = 0; i < s.length; i++) {
      const k = s.key(i);
      if (k && k.startsWith(dirPrefix)) {
        const rest = k.slice(dirPrefix.length);
        if (rest === DIR_MARKER) continue;
        const name = rest.split("/")[0];
        if (name) names.add(name);
      }
    }
    return Array.from(names).sort();
  }

  function mkdir(subdir: string): void {
    const segs = validateFsPath(subdir);
    storage().setItem(`${keyFor(segs)}/${DIR_MARKER}`, "");
  }

  const api = {
    write(path: string, data: string) {
      writeFile(path, data, false);
    },
    append(path: string, data: string) {
      writeFile(path, data, true);
    },
    read(path: string) {
      return readFile(path);
    },
    exists(path: string) {
      return exists(path);
    },
    remove(path: string) {
      remove(path);
    },
    list(subdir?: string) {
      return list(subdir);
    },
    mkdir(subdir: string) {
      mkdir(subdir);
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

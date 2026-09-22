/**
 * Storage backend shared by `badge.store` and `badge.fs`.
 *
 * Prefers the browser's real `localStorage`. Falls back to an in-memory
 * `Storage`-shaped shim when `localStorage` isn't available (e.g. running
 * under `bun test` with no DOM) so these modules stay independently
 * testable without a browser.
 */
class MemoryStorage implements Storage {
  private map = new Map<string, string>();

  get length(): number {
    return this.map.size;
  }
  clear(): void {
    this.map.clear();
  }
  getItem(key: string): string | null {
    return this.map.has(key) ? (this.map.get(key) as string) : null;
  }
  key(index: number): string | null {
    return Array.from(this.map.keys())[index] ?? null;
  }
  removeItem(key: string): void {
    this.map.delete(key);
  }
  setItem(key: string, value: string): void {
    this.map.set(key, String(value));
  }
}

let memoryFallback: MemoryStorage | null = null;

export function getStorage(): Storage {
  if (typeof localStorage !== "undefined") return localStorage;
  if (!memoryFallback) memoryFallback = new MemoryStorage();
  return memoryFallback;
}

export function byteLength(s: string): number {
  return new TextEncoder().encode(s).length;
}

/**
 * One badge per browser (Milestone 10 Task 9). Every tab of this origin
 * would be the same badge, with the same factory MAC from
 * {@link loadOrCreateMac}, so only one may run it. The tab that runs the
 * firmware holds the Web Lock {@link BADGE_LOCK_NAME}, exclusive, for as
 * long as it runs. A tab that finds it held shows "Badge running in
 * another tab" with a take-over button; {@link takeOver} steals the lock,
 * which ends the old holder's request with an `AbortError`, so the old
 * tab's `onLost` stops its emulator. The new tab then acquires the lock
 * and starts its own.
 *
 * Without the Web Locks API (or when it throws or rejects, as a sandboxed
 * frame may), the badge runs here: nothing is gained by refusing to run,
 * and two tabs then simply act as two badges with one MAC (ruling
 * R-T9-5).
 */

export const BADGE_LOCK_NAME = "htn-badge";

/** The `localStorage` key of this browser's factory MAC. */
export const MAC_STORAGE_KEY = "htn-badge-mac";

/** The part of `LockManager` used here; injectable for tests. */
export interface LockManagerLike {
  request(
    name: string,
    options: { ifAvailable?: boolean; steal?: boolean },
    cb: (lock: { name: string } | null) => unknown,
  ): Promise<unknown>;
}

export interface BadgeHold {
  /** Frees the badge. Idempotent; `onLost` does not run afterwards. */
  release(): void;
}

/** `navigator.locks`, or `null` where there is none. */
function defaultLocks(): LockManagerLike | null {
  try {
    const nav = (globalThis as { navigator?: { locks?: LockManagerLike } }).navigator;
    return nav?.locks ?? null;
  } catch {
    return null;
  }
}

const NO_LOCK: BadgeHold = { release() {} };

/**
 * Takes the badge if no other tab holds it: a hold, or `null` when it is
 * taken. `onLost` runs once if another tab takes the badge over while this
 * one holds it. Never rejects.
 */
export function acquireBadge(
  onLost: () => void,
  locks: LockManagerLike | null = defaultLocks(),
): Promise<BadgeHold | null> {
  if (!locks) return Promise.resolve(NO_LOCK);
  return new Promise((resolve) => {
    let granted = false;
    let released = false;
    let request: Promise<unknown>;
    try {
      request = locks.request(BADGE_LOCK_NAME, { ifAvailable: true }, (lock) => {
        if (!lock) {
          resolve(null);
          return undefined;
        }
        granted = true;
        // Held until release(): the lock lasts as long as this promise is pending.
        return new Promise<void>((free) => {
          resolve({
            release() {
              if (released) return;
              released = true;
              free();
            },
          });
        });
      });
    } catch {
      resolve(NO_LOCK);
      return;
    }
    Promise.resolve(request).then(
      () => {},
      () => {
        if (!granted) {
          // The lock manager failed before granting anything: run here.
          resolve(NO_LOCK);
          return;
        }
        // Stolen (an AbortError) while held.
        if (released) return;
        released = true;
        try {
          onLost();
        } catch (e) {
          console.error("badge lock: onLost failed:", e);
        }
      },
    );
  });
}

/**
 * Takes the badge from whichever tab holds it: steals the lock (the old
 * holder's `onLost` runs) and frees it at once, for {@link acquireBadge}
 * to take next. Never rejects.
 */
export async function takeOver(locks: LockManagerLike | null = defaultLocks()): Promise<void> {
  if (!locks) return;
  try {
    await locks.request(BADGE_LOCK_NAME, { steal: true }, () => undefined);
  } catch {
    // Stolen in turn, or no lock manager after all: acquireBadge decides.
  }
}

const MAC_TEXT = /^[0-9a-f]{2}(?::[0-9a-f]{2}){5}$/;

/** A locally administered unicast MAC: bit 1 of the first byte set, bit 0 clear. */
function isLocalUnicast(mac: Uint8Array): boolean {
  return mac.length === 6 && (mac[0]! & 0x03) === 0x02;
}

function formatMac(mac: Uint8Array): string {
  return [...mac].map((b) => b.toString(16).padStart(2, "0")).join(":");
}

function parseMac(text: string | null): Uint8Array | null {
  if (text === null || !MAC_TEXT.test(text)) return null;
  const mac = new Uint8Array(text.split(":").map((h) => parseInt(h, 16)));
  return isLocalUnicast(mac) ? mac : null;
}

function randomMac(): Uint8Array {
  const mac = new Uint8Array(6);
  crypto.getRandomValues(mac);
  mac[0] = (mac[0]! & 0xfc) | 0x02;
  return mac;
}

/**
 * This browser's factory MAC: the one stored under {@link MAC_STORAGE_KEY},
 * or a new random locally administered unicast MAC, stored there. A
 * malformed or non-local stored value is replaced. Every storage access is
 * guarded; with no usable storage the MAC is random per load.
 */
export function loadOrCreateMac(storage: Storage | null): Uint8Array {
  let stored: string | null = null;
  try {
    stored = storage?.getItem(MAC_STORAGE_KEY) ?? null;
  } catch {
    stored = null;
  }
  const kept = parseMac(stored);
  if (kept) return kept;
  const mac = randomMac();
  try {
    storage?.setItem(MAC_STORAGE_KEY, formatMac(mac));
  } catch {
    // Storage is full or blocked: random per load.
  }
  return mac;
}

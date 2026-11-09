import type { LockManagerLike } from "../src/radio/lock";

/**
 * A fake of the Web Locks API's exclusive mode for one lock name at a
 * time, enough for `ifAvailable` and `steal`: a stolen holder's request
 * promise rejects with an `AbortError`, as the spec says, and the lock
 * is held until the callback's promise settles.
 */
export class FakeLockManager implements LockManagerLike {
  readonly requests: Array<{ name: string; options: { ifAvailable?: boolean; steal?: boolean } }> = [];
  private holder: { reject: (e: unknown) => void; token: object } | null = null;
  private waiting: Array<() => void> = [];

  request(
    name: string,
    options: { ifAvailable?: boolean; steal?: boolean },
    cb: (lock: { name: string } | null) => unknown,
  ): Promise<unknown> {
    this.requests.push({ name, options });
    return new Promise((resolve, reject) => {
      const grant = () => {
        const token = {};
        this.holder = { reject, token };
        Promise.resolve()
          .then(() => cb({ name }))
          .then(
            (v) => {
              if (this.holder?.token === token) this.release();
              resolve(v);
            },
            (e) => {
              if (this.holder?.token === token) this.release();
              reject(e);
            },
          );
      };
      if (options.steal) {
        if (this.holder) {
          const old = this.holder;
          this.holder = null;
          old.reject(new DOMException("stolen", "AbortError"));
        }
        grant();
        return;
      }
      if (this.holder) {
        if (options.ifAvailable) {
          Promise.resolve()
            .then(() => cb(null))
            .then(resolve, reject);
          return;
        }
        this.waiting.push(grant);
        return;
      }
      grant();
    });
  }

  held(): boolean {
    return this.holder !== null;
  }

  private release(): void {
    this.holder = null;
    this.waiting.shift()?.();
  }
}

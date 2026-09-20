/**
 * `badge.contacts` — the emulator simulates no other badges, so there are
 * never any contacts.
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";

export function createContactsModule() {
  const api = {
    count() {
      return 0;
    },
    get(_i: number) {
      return null;
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

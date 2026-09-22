/**
 * `badge.nfc` — no real NFC hardware in the emulator; all stubs.
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";

export function createNfcModule() {
  const api = {
    enable() {
      return true;
    },
    disable() {
      // No-op.
    },
    card() {
      return null;
    },
    read_text() {
      return null;
    },
    clear() {
      // No-op.
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

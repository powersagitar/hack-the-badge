/**
 * `badge.me` — stubbed hacker identity (the emulator has no provisioning
 * flow, so these are plausible fixed values).
 */
import type { LuaState } from "../lua/interop";
import { multi, pushNamespace } from "../lua/interop";

export function createMeModule() {
  const api = {
    name() {
      return "Emulated Hacker";
    },
    role() {
      return "hacker";
    },
    role_name() {
      return "Hacker";
    },
    color() {
      return multi(0x33, 0x99, 0xff);
    },
    badge_id() {
      return null;
    },
    provisioned() {
      return false;
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

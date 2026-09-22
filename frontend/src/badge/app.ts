/**
 * `badge.app` — identity of the currently loaded app (from its manifest),
 * plus `exit()` to signal "return to launcher". This milestone has no
 * launcher to return to, so `exit()` just logs and calls the optional
 * `onExit` hook the runtime can supply.
 */
import type { LuaState } from "../lua/interop";
import { pushNamespace } from "../lua/interop";

export interface AppIdentity {
  slug: string;
  name: string;
}

export function createAppModule(identity: AppIdentity, onExit?: () => void) {
  const api = {
    slug() {
      return identity.slug;
    },
    name() {
      return identity.name;
    },
    exit() {
      if (onExit) {
        onExit();
      } else {
        console.log("[badge.app.exit] no launcher wired up in this milestone; ignoring");
      }
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

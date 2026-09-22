/**
 * `badge.sensor` — the emulator has no real accelerometer.
 *
 * Design choice (spec offered either option): `accel()` returns a fixed
 * `(0, 0, 1000)` reading — milli-g units, flat on a table (1g straight down
 * the Z axis) — rather than `nil, "unavailable"`, so smoke-test apps that
 * unconditionally destructure the 3 return values don't need extra nil
 * checks. `shake()`/`tap()` always report false; `orientation()` is fixed.
 */
import type { LuaState } from "../lua/interop";
import { multi, pushNamespace } from "../lua/interop";

export function createSensorModule() {
  const api = {
    accel() {
      return multi(0, 0, 1000);
    },
    shake() {
      return false;
    },
    tap() {
      return false;
    },
    orientation() {
      return "flat_up";
    },
  };

  return {
    attach(L: LuaState) {
      pushNamespace(L, api);
    },
  };
}

# Milestone 13: skip Setup, and the ride-alongs

Date: 2026-11-11. Status: approved by the human partner (2026-11-11).

## Why

After "Provision test badge", the firmware hands the badge to its
onboarding app ("Setup", 11 pages: the name, press all 8 buttons, a
shake, an animation, tick "I agree", START). The human partner walks it
by hand on every page load, and so does anyone they demo the emulator
to, who only wants to reach the apps. They want one button that skips
all of it.

## What success looks like

On a fresh page, in firmware mode: Provision (unchanged, its own
button) → Setup appears, and with it a **Skip setup** button → one
press → about two seconds later the badge shows My Badge's registered
screen, the same screen Setup's last START lands on, one HOME press
from the launcher. The button is gone once Setup is gone.

## Goals

1. A Skip setup control in the page chrome (`ui/shell.ts`), visible
   **only while Setup is on screen** (the human partner's call).
2. The skip uses the firmware's own console, the way the provisioner
   types the identity. It needs no emulator-core change.
3. The ride-alongs below: "everything from the Milestone 13 backlog that
   fits" (the human partner, approving the list in the conversation).

## Non-goals

- Folding the skip into Provision. Provision stays a separate step (the
  human partner's call).
- Changing the boot ratchet's walk. Milestone 5's R-T4-2 ("onboarding is
  not bypassed") stands for the tests: the rungs keep walking all 11
  pages, as a browser user can. The skip is an extra path, with its own
  rung.
- A skip in Lua mode (there is no Setup there).
- Changing the firmware's behaviour on re-provisioning.

## Anti-goals

- A skip that lands somewhere other than the hand walk's end. A frame
  that differs from the walked registered screen is a failure, even if
  the launcher works.
- Moving any existing pin. Every step count, needle, emulated time and
  hash in today's suites stays byte-identical (the skip is new input on
  a new path; nothing existing should change).
- A button that can fire twice, or fire outside Setup, and leave the
  badge in a half state.

## How the skip works (decided)

Found in the public firmware source
(github.com/hackthenorth/badge-firmware, "Open source firmware release",
`23c66f0`). It is not labelled v392, so the builder confirms each step
against v392's image. The strings `usage: config <slug> <key> <value>`,
`OK %s.%s=%s`, `onboard_build`, `rebooting...`, `Setup` and
`I (%lu) %s: launched %s` are all in `firmware.bin`.

- `main/apps/app_onboarding.cpp`: `badge_onboarding_required()` is
  `hal_identity_is_provisioned() && hal_config_get_int("system",
  "onboard_build", -1) < BADGE_BUILD_NUMBER`. `finish()` writes
  `onboard_build = BADGE_BUILD_NUMBER`, sets the launcher and launches
  My Badge. `BADGE_BUILD_NUMBER` comes from the `BADGE_BUILD` env at
  build time (`main/CMakeLists.txt`), and v392's value is unknown.
- `main/hal/hal_console.cpp` `cmd_config`: `config <slug> <key>
  <value>` → `hal_config_set_string`, prints `OK system.onboard_build=…`.
- Setup runs as the shell and never re-checks the setting, so writing
  it alone leaves Setup on screen. `cmd_reboot` (`reboot`) prints
  `rebooting...` and calls `esp_restart()`.
- `main/main.cpp` (~line 196): on boot, a provisioned badge with
  onboarding not required takes the normal path to My Badge.

**The sequence:** type `config system onboard_build 2147483647`, wait
for `OK system.onboard_build=2147483647` and the prompt, then type
`reboot`. 2147483647 (INT32_MAX) is at or above any build number, so
v392's value doesn't matter. The reboot goes through the emulated
software reset Milestone 9/11 already models (466 ms held screen, then
the boot). LittleFS lives in the emulated flash, which a reset keeps.

**Fallback if v392 disagrees** (for example `config` writes the string
where `hal_config_get_int` can't read it, or the reboot routes
elsewhere): stop and bring it to the human partner. Don't fall back to
a scripted walk without asking. The scripted fast-forward walk was the
alternative considered, and the human partner asked to try the console
command first.

## When the button shows (decided: only during Setup; detection is the builder's, with this default)

The default: the Worker watches the current boot's console. The
registry logs `app_reg: launched <name>` (`app_registry.cpp:186`) on
every launch, and Setup's `name()` is `"Setup"`.

- Show on `app_reg: launched Setup`.
- Hide on any later `app_reg: launched <other>`, on a soft reset, on a
  mode switch and on a stopped badge.
- Hide while a skip runs, so a second press can't send it twice.

Both ways into Setup log the line in the source.
`app_registry_start` (`app_registry.cpp:259`), the boot-time path for a
badge that boots provisioned but not onboarded, calls
`app_registry_launch`. So does My Badge's hand-off after `prov apply`.
Confirm both on v392. If the boot-time path is silent there, add a
second signal for it and record it.

**Why the skipped and walked screens should match:** Setup's only config
writes are `onboard_build` (`app_onboarding.cpp:47`, `:388`), and
`finish()` does what a normal boot does (it sets the launcher and
launches My Badge, as `main.cpp`'s `app_registry_start(launcher,
badge_id)` does). The reboot adds only the boot splash first. The upload
token doesn't change the registered or launcher hashes (Milestone 12's
measurement 3), so the rung doesn't depend on whether it sets one.

This covers re-provisioning too: the firmware prints `tutorial=reset`,
the launcher stays until the next press, then Setup launches and the
button appears.

The state reaches the page through the existing Worker `status`
message (`firmware-worker-protocol.ts`), and the press goes in as a new
message beside provisioning's. Message names and shapes are the
builder's.

## Where the code goes (builder's, within the repo's rules)

- A small console-typing routine for the two lines, beside
  `runtime/provisioner.ts`. Reuse its line typing, prompt matching and
  boot-scoped console start; don't fork them. `config …` is 38 bytes,
  under the 64-byte chunk.
- The Worker owns the skip, as it owns provisioning: one at a time, and
  refused while a provision is typing.
- `ui/shell.ts`: the button, in the provisioning controls.
  `ui/badge-controller.ts`: the wiring.

## Tests and checks

- **A new boot-ratchet rung** (`emulator-core/tests/boot_progress.rs`,
  with a console helper in `tests/common/` mirroring the TS routine, as
  `provision.rs` mirrors `provisioner.ts`): provision role hacker, wait
  for Setup's first page, type the two lines, and assert
  `OK system.onboard_build=2147483647`, `rebooting...`, one soft reset,
  no fault, and a stable frame equal to the walked registered-screen
  hash for hacker. Then HOME reaches the launcher hash. Pin its step
  count and emulated time like the other rungs. It shares checkpoints
  through the existing `OnceLock`s.
- **The equivalence rungs** (`execution_digest_is_unchanged`,
  `hacker_walk_totals_are_unchanged`) and every existing pin stay
  unchanged.
- **Frontend tests:** the show/hide rules (the console needles, soft
  reset, mode switch); a double press sends once; refused during a
  provision. Optionally the WASM twin can take the skip path.
- **The human partner's browser check:** Provision → Skip setup →
  registered screen → HOME → launcher, in the prod build.

## Ride-alongs

From `docs/milestone-12-decisions.md`, "Milestone 13 backlog", as the
human partner approved them in the conversation. Each closes with a
test where it can, and the rest of the backlog carries forward word for
word.

1. **The fetch timeout ends at the headers.** In
   `radio/signal-client.ts` `call` (~line 108), keep the timer running
   until the body is read (the `res.json()` at ~154 and ~243), so a
   stalled body hits `SIGNAL_FETCH_TIMEOUT_MS`. Add a test with a body
   that never ends.
2. **Re-provisioning's tutorial reset.** No code change. Record it as
   handled by Skip setup. The behaviour stays the firmware's.
3. **The provisioner around reboots** (Milestone 11's carried items).
   (a) A console at its 256 KiB cap plus a soft reset leaves the
   boot-scoped prompt scan never matching (R-T13-3). (b) A provision
   still typing when the badge reboots loses bytes in the new REPL's
   terminal probe, then times out. The skip's typing shares this code,
   so fix both there. The *how* is the builder's. The bar: (a) the
   prompt is found; (b) a provision interrupted by a reboot ends in a
   clear failure state, or restarts cleanly, rather than waiting out
   the timeout. Which of the two is the builder's call, recorded.
4. **The speed readout through the reboot's pause** (R-T8-2): it reads
   about 1x while the screen is held. Show the pause as what it is, not
   a speed.
5. **`status.softResets` unused by the Worker client.** Use it, or
   remove it. Skip setup's hide-on-reset is a natural user.
6. **The host tab's stale "Room …: enter this code" line after Remove.**
   Clear it.
7. **No feedback for a refused Bump.** Show a short reason (for
   example "badge not running" or "page hidden", from the bump gate).
8. **Milestone 12's deferred review minors**, all of the list under
   "Milestone 12's other deferred review minors": Task 1's test-wording,
   citation-range and dash nits; Task 2's console-snapshot, deadline-doc,
   redundant-assert and needle-order nits; Task 3's provisioner-test
   end state, `uploadToken` validation, `as string` cast and long line;
   Task 4's `lib`/`target` mismatch and the commented cast; Task 5's
   missing `close` tests and the silent timed-out `close`. Each one must
   leave every pin unchanged.
9. **The smoke-test manifest** (R-T6-2): give
   `frontend/public/apps/smoke-test/manifest.cfg` a `heap_kb` of 48 or
   96 and a `wake_lock` of 0 or 1, *if* Lua mode's manifest parser
   (`runtime/lifecycle.ts`) still accepts and runs it unchanged. If Lua
   mode would read it differently, leave it and record why.

**Out of scope** (stays in the backlog): Share between two browsers,
the fake sync station, the Sync stopwatch (needs a real badge), a cold
power-on's 466 ms (moves every pin), and the reset-model, radio-model
and Milestone 8 carried items.

## Constraints (from CLAUDE.md, restated where they bite)

- emulator-core needs no change for the skip. If one turns out to be
  needed, it keeps the core chip-agnostic and every pin unchanged.
- Bun only. Pin the exact version of any new dependency and tell the
  human partner about it.
- The only committed identity data is the fake identities and
  `FAKE_UPLOAD_TOKEN`.
- Docs: a `docs/milestone-13-decisions.md` (BASE, rulings, a Milestone
  14 backlog). Update CLAUDE.md's "Current state" and the notes'
  onboarding section with the skip.

## Left to the builder

Message names, the routine's module, the button's label and styling
(default "Skip setup"), the rung's name, how 3(a)/3(b) are fixed, the
speed readout's wording during the pause, and the second Setup signal if
the boot-time path is silent. Record each in
the decisions doc.

# Milestone 13: decisions and the Milestone 14 backlog

Milestone 13 adds a **Skip setup** button: while the onboarding app
("Setup") is on screen, one press types `config system onboard_build
2147483647` and `reboot` into the badge's console, and the new boot
lands on My Badge's registered screen. It also carries the Milestone 12
backlog items that fit (nine ride-alongs).
Spec: `docs/superpowers/specs/2026-11-11-milestone13-skip-setup-design.md`;
plan: `docs/superpowers/plans/2026-11-11-milestone13-skip-setup.md`.

`BASE` (the branch point for this milestone's work, `milestone-13`):
`7772e37` ("docs: Milestone 13 plan"; on `main`, after the spec's
`be6e18e`).

## Why

After "Provision test badge" the firmware hands the badge to Setup, 11
pages: the name, press all 8 buttons, a shake, an animation, tick "I
agree", START. The human partner walked it by hand on every page load,
and so did anyone they showed the emulator to who only wanted the apps.
They asked for one button that skips it, kept separate from Provision,
landing where Setup ends (My Badge's registered screen), and shown only
while Setup is on screen. They asked to look for a console command
first, before a scripted walk (2026-11-11). The public firmware source
(`hackthenorth/badge-firmware` @ `23c66f0`, not labelled v392) had one:
`badge_onboarding_required()` reads `system.onboard_build`, the
console's `config` writes it, and `reboot` restarts into the normal path
(the spec, "How the skip works").

## Human decisions (2026-11-11)

- A Skip setup button; provisioning stays its own button; the skip lands
  where Setup ends, on My Badge's registered screen.
- A console command over a scripted walk ("see if there's a console
  command that does that first").
- The button shows only during Setup.
- Ride-alongs: "everything that fits" (the list approved in chat, now in
  the spec's "Ride-alongs").
- The spec "lgtm", then subagent-driven development, with subagent
  reviews.

## Measurements

### The skip on v392 (Task 1, the gate)

Two rungs in `emulator-core/tests/boot_progress.rs`, with the helper
`skip_setup` in `tests/common/provision.rs` (release, role hacker):

- **`skipping_setup_lands_on_the_registered_screen`.** From
  `provisioned(OWN_ROLE)` (Setup's page 1 stable at 36,542,000 total
  steps): `OK system.onboard_build=2147483647`, no `usage:`, then
  `rebooting...` and exactly one soft reset, seen at 38,392,000 (1.85M
  steps later, sampled every 50,000; the `config` line, its OK, the
  prompt and `reboot`). The new boot logs `app_reg: launched My Badge`
  and no `launched Setup`. The registered screen is first stable at
  76,392,000 and confirmed at 84,392,000 (46.0M after the reset), hash
  `0xbd6d_7613_08a7_bf24`, the frame the 11-page walk ends on
  (`role_row(OWN_ROLE).1`). HOME then opens the launcher
  (`0x94d7_a7a8_5fa9_3ea3`). Pinned: `SKIP_REGISTERED_AT_STEPS` =
  84,392,000 and `SKIP_REGISTERED_AT_US` = 8,855,536 (emulated time
  counts through the reset's 466 ms).
- **`setup_logs_its_launch_after_a_reboot`.** From the same checkpoint,
  `reboot` alone: the reset by 36,792,000; `app_reg: launched Setup` by
  47,642,000; Setup's page 1 (`0xbf9c_cb7e_80b4_4b1a`,
  `role_row(OWN_ROLE).0`) first stable at 51,642,000 and confirmed at
  59,642,000 (+22.85M), 12,630,966 µs (not pinned). So the boot-time
  path (`app_registry_start`) logs the launch on v392 too, and the
  watcher needs only the one needle (R-T1-1).
- Budgets, at the file's rule (measured + 25 %, rounded up to
  1,000,000):

  | Budget | Measured | Value |
  |---|---|---|
  | `SKIP_SETUP_DEADLINE_STEPS` | 1,850,000 | 3,000,000 |
  | `SKIP_REGISTERED_STEPS` | 46,000,000 | 58,000,000 |
  | `SETUP_AFTER_REBOOT_STEPS` | 22,850,000 | 29,000,000 |

- No existing pin moved; `execution_digest_is_unchanged` and
  `hacker_walk_totals_are_unchanged` pass untouched. Ratchet: 90 → 92
  rungs, 18.39 s.
- The skip sets no upload token (`provisioned()` sets none), and the
  registered hash still matches, as the spec expected.

### The console is not ASCII (Task 2)

By 5.5M steps v392 has printed an em dash (U+2014): the decoded text is
1,726 characters for 1,728 bytes written. The plan's `consoleSince`, a
character slice by `consoleWritten()`, would have been off by two bytes
per such character, so the slice moved into the core (R-T2-1).

### The skip in the WASM twin and in Chrome (Tasks 4 and 6)

- `test/cpu-wasm.test.ts`, "skips Setup to the registered screen": the
  provisioned hacker's Setup page 1, `setupOnScreen` true, the skipper
  driven in 50,000-step runs to `done` within 3,000,000 steps, one soft
  reset, the registered hash `0xbd6d761308a7bf24` within 58,000,000
  steps, and `launched My Badge` with no `launched Setup` in the new
  boot. It adds about 2.05 s to `bun test`.
- The controller's smoke check (Task 6; Chrome through chrome-devtools,
  dev server): Provision hacker; Skip setup hidden while the provision
  ran, shown once `PROV OK …` read and Setup was up. A scripted double
  click sent one skip: `skipping setup…` (Skip hidden, Provision
  disabled) at 0.00 s, `rebooting…` at 0.21 s, `1.0x · PROV OK …` at
  0.67 s, then the registered screen ("Test Hacker", "Badge ID:
  test-fake-badge-0001") and HOME to the 16-app launcher.

### The build (Task 8)

`tsconfig.json`'s `target` ES2020 → ES2022: `bun run build`'s `dist/`
is byte-identical (the same SHA-1 for every file). Raising Vite's
`build.target` to es2022 as well changed the shipped JS (new hashes,
about 1 kB smaller), so it was reverted (R-T8-3).

### Suites per task (from the task reports)

| After | Release `boot_progress` | `bun test` |
|---|---|---|
| `BASE` | 90 + 1 ignored, 18.48 s | 339, 13.12 s |
| Task 1 (`8068c52`) | 92 + 1 ignored, 18.39 s | — |
| Task 2 (`6b83c63`) | 92 + 1 ignored | 340 |
| Task 3 (`fea23b8`) | — | 352, 12.74 s |
| Task 4 (`ca9ed1c`) | — | 372, 14.62 s |
| Task 5 (`30d2a66`) | — | 376, 14.7 s |
| Task 6 (`b60e4ed`) | — | 384, 14.72 s |
| Task 7 (`3867f6f`) | — | 385, 14.8 s |
| Task 8 (`c2bdfaf`) | 92 + 1 ignored, 18.23 s | 392, 15.5 s |
| Final review (`689911e`) | — (no Rust change) | 395, 15.39 s |

Clippy and `bun run typecheck` were clean after every task.

## Re-provisioning's tutorial reset: handled by Skip setup

Milestone 12 recorded, at its Gate 3, that `prov apply` on a registered
badge prints `tutorial=reset`: the launcher stays on screen until the
next button press, which drops into onboarding. The human partner did
not rule on it. Milestone 13 closes it as **handled by Skip setup**
(ride-along 2): the firmware's behaviour is unchanged, and once Setup
launches after that press, the button appears and skips it again. The
button follows the current boot's last `app_reg: launched` line, so it
appears at Setup's launch, not at `PROV OK` (Review Focus 4; Task 4's
`setupOnScreen follows the last launch line` test).

## Gates and the human partner's answers

- **Gate 1** (Task 1, the go/no-go): passed. v392 agrees with the public
  source: `config` writes the setting, `reboot` takes the normal path,
  and the skipped boot lands on the walked registered screen.
- **Gate 2** (Task 6): no scope surprise; ride-alongs 4, 6 and 7 went
  in.
- **Gate 3, the human partner's browser check: pending.** To be run on
  the prod build (`bun run build && bun run preview`): provision hacker,
  then the Skip setup button appears with Setup; press it, then
  `rebooting…`, the registered screen, HOME, the launcher; re-provision
  from the launcher, press a button, and Setup and the button come back.
  Their result goes here, in their words.

## Dependencies added

None. `git diff 7772e37..HEAD` touches no `package.json`, `Cargo.toml`,
`Cargo.lock` or `bun.lock`. Task 8's tests use `spyOn` from `bun:test`.

## Rulings

The controller's rulings keep their ledger numbers with an `R-` prefix
(P1 → R-P1, T2-1 → R-T2-1, and so on). The plan's decisions and the
implementers' choices are numbered after them within each task.

Ruling R-P1: Task 2's additive `Console::written` / `FirmwareRuntime::console_written` core change is allowed, though the spec's Goal 2 says the skip "needs no emulator-core change" — why: the spec's Constraints allow a needed core change that keeps the core chip-agnostic and every pin unchanged, and this is a counter with no emulated effect, so it cannot move a pin — cost if wrong: a small revert, with the browser falling back to length arithmetic over `consoleOutput()`.

Ruling R-T1-1: the boot-time path into Setup logs `app_reg: launched Setup` on v392 (`setup_logs_its_launch_after_a_reboot`; a mid-Setup `reboot` comes back to page 1), so the watcher uses that one needle and no second Setup signal — why: the spec asked to confirm both ways into Setup log the line, and to add a second signal only if the boot-time path were silent — cost if wrong: none found; a later firmware that launched Setup without the line would leave the button hidden, never shown wrongly.

Ruling R-T1-2: both rungs measure the new boot's console from just past `rebooting...` (`boot_start_after`), not from the console length when the reset is seen — why: the 50,000-step chunk that holds the reset can already hold the new boot's first lines, so the length would skip them; past `rebooting...` is exact either way, and keeps the old boot's `launched Setup` out of the negative check — cost if wrong: none to behaviour; a firmware that printed after `rebooting...` before the reset would have those lines counted as the new boot's.

Ruling R-T1-3: `setup_logs_its_launch_after_a_reboot` bounds its reset wait with `SKIP_SETUP_DEADLINE_STEPS` (3,000,000) and leaves its emulated time (12,630,966 µs) unpinned — why: the reboot alone is shorter than the skip that budget was measured on, and the plan asked only for the hash — cost if wrong: a loose bound (the reset came within one 250,000-step chunk); a change to the reboot's timing there shows only through the step budget and the hash.

Ruling R-T2-1: the console slice is byte-exact in the core: `Console::text_since(at)` (lossy UTF-8 of the ring's bytes from absolute offset `at`; the whole ring if `at` is older than the ring, "" at or past `written`), `FirmwareRuntime::console_since`, the wasm one-line passthrough `consoleSince(at: f64)` (`at as u64`, saturating, as `run_until` does) and `FirmwareEmulatorHandle.consoleSince`; `console-io.ts`'s `consoleSince(handle, at)` is a thin call to it — why: v392's console is not ASCII (Task 2's em dash), so a slice can be exact only where the bytes are; it also copies only the current boot instead of the whole 256 KiB ring — cost if wrong: one more passthrough to remove; a slice that starts mid-character or at a wrapped ring's start begins with one U+FFFD, harmless for needle scans.

Ruling R-T3-1: ride-along 3(b), decided in the plan: a provision **fails** with `the badge rebooted during provisioning` (`REBOOTED_REASON`) when `softResetCount()` changes after it left its `prompt` phase, and types nothing more; a reboot while it still waits for the first prompt stays fine — why (inferred: the plan says only "decided"; this reason is ours): the new REPL's 500 ms terminal probe eats bytes, so a retry would have to restart from `put` after re-reading the new boot, while a clear failure ends at once and the user presses Provision again; the old behaviour waited out the 60M-step timeout — cost if wrong: an interrupted provision needs a second click instead of resuming by itself. The check is `!==`, not "a rise": a cold `reset()` zeroes the count but drops the provisioner anyway, so `!==` is the safe superset.

Ruling R-T3-2: ride-along 3(a): the provisioner's markers (`marker`, `provOkEnd`, `bootConsoleStart()`) are absolute byte offsets read through `consoleSince`, and `firmware-runtime.ts` sets `bootConsoleStart` to `consoleWritten()` at a soft reset (was `consoleOutput().length`) — why: an index into `consoleOutput()` shifts once the 256 KiB ring is full, which made Milestone 11's R-T13-3 boot-scoped prompt scan never match after a reset in a full console; an absolute offset does not shift — cost if wrong: none found; the WASM twin's provisioned walk is unchanged.

Ruling R-T3-3: `console-io.ts` also exports `consoleOffsetOf(handle, text, i)`, the absolute offset of `text[i]` counted back from `consoleWritten()` by the UTF-8 length of `text.slice(i)` — why: T2-1's forward formula (`at` plus the bytes before `i`) is wrong when the slice was the whole ring or began with U+FFFD; counting from the end stays exact there, because `i` sits on an ASCII needle — cost if wrong: invalid or truncated UTF-8 after `i` makes the result early by up to 2 bytes per U+FFFD (its doc says so); the firmware prints valid UTF-8.

Ruling R-T4-1: Skip setup is hidden while a provision is in flight (`provisionState().kind` `waiting` or `typing`), in addition to the plan's conditions (firmware mode, Setup on screen, no skip typing) — why: `setupOnScreen()` is true during the token phase, which runs after Setup appears, but `skipSetup()` refuses then, so a visible button would be a dead click (Review Focus 1) — cost if wrong: the button appears about 0.3 s after Setup; trivially reverted.

Ruling R-T4-2: `provision()` is refused while a skip is in flight, at the runtime: its state becomes `{ kind: "failed", reason: "a setup skip is in progress" }` (`SKIP_IN_PROGRESS_REASON`) and nothing is typed; the page hides nothing extra for it — why: the Task 4 reviewer found a provision could type `put` between `config` and `reboot`, interleaving lines, which the runtime's own doc forbids; this mirrors Review Focus 1 the other way — cost if wrong: one guard and one test to remove. A refused provision overwrites an earlier `done` provision state with `failed` (a deferred minor).

Ruling R-T4-3: a soft reset before `reboot` is typed (while the skipper waits for its prompt or for the config's reply) fails the skip with `the badge rebooted during the skip` and types nothing more — why: it mirrors R-T3-1; and unlike the provisioner it fails even before the first prompt, because a reboot then means the screen may no longer be Setup, which `skipSetup()` checked only at the press — cost if wrong: a skip that could have gone on needs a second press.

Ruling R-T4-4: `setupOnScreen(bootText)` reads the current boot's last `app_reg: launched ` line, and if that line is still unterminated, the one before it; it is true only when the name is exactly `Setup` — why: a launch line half-printed at the slice's end would otherwise be read with a truncated name — cost if wrong: the button shows or hides one status interval (250 ms) late.

Ruling R-T5-1: the Provision button is disabled while `skipState()?.kind === "typing"` — why: the client's optimistic `provision()` shows `waiting` for up to 250 ms before the Worker's refusal (R-T4-2) arrives, and disabling avoids that misleading flash — cost if wrong: a disabled button for about 2 s during a skip.

Ruling R-T5-2: ride-along 5, decided in the plan: `softResets` is removed from the Worker's `status`, which is now `{ kind, speed, rebooting, provision, skip, setupOnScreen }`; the Worker keeps its internal count for the immediate post on a reset, and the page gets `rebooting` instead. `setupOnScreen` is computed only when a status is built (at most every 250 ms, or at a reset's immediate post) — why: no page code read `softResets`, and `setupOnScreen` decodes up to the whole boot's console, too costly per slice — cost if wrong: a page that later needs the count adds the field back; `rebooting` going false shows up to 250 ms late.

Ruling R-T5-3: the Worker client still omits `skipSetup` from its `FirmwareRuntime` interface (`Omit<FirmwareRuntime, "skipSetup">`) — why: the client's `skipSetup()` returns `void` (it posts a message; the plan's signature) and the runtime's returns `boolean`, which TypeScript does not allow as an override; the client sends only while the last status shows Setup, then shows the skip as `typing` until the next status — cost if wrong: the page cannot learn synchronously that the Worker refused a press; the next status shows it.

Ruling R-T6-1: the status line shows whichever the user started last, the skip or the provision (`lastStarted` in `badge-controller.ts`): `skipping setup…` and `skip failed: <reason>` only while the skip was started last, the provision's own state otherwise; a `done` skip adds nothing, so the older `PROV OK` line shows again — why: the first version let a failed skip's line hide every later provision's status (the Task 6 reviewer's Important finding) — cost if wrong: display only; `lastStarted` is not reset on a mode switch or `reset()` (a deferred minor).

Ruling R-T6-2: the speed readout's place shows `rebooting…` while `rebooting()` is true (the reboot's wall-clock debt unpaid), and `N.Nx` otherwise (ride-along 4; the wording was the builder's) — why: through the pause the readout read about 1x, a speed for a screen that is not running (Milestone 11's R-T8-2) — cost if wrong: none to behaviour.

Ruling R-T6-3: ride-along 6 clears the hosted room's code line on Remove only while the room is still hosted (before it links); a linked room's `Room X: linked.` line stays after Remove — why: the backlog item named the "enter this code" line, and the Task 6 reviewer agreed the linked line is outside it — cost if wrong: a stale `linked.` line after Remove (kept in the backlog).

Ruling R-T6-4: a refused Bump shows one note, `bump: the badge isn't running`, for `BUMP_NOTE_MS` (3 s) — why: the hub's gate does not say whether the badge was stopped or the page hidden, and a hidden page cannot be clicked, so one reason covers what a user can see — cost if wrong: the note does not distinguish the two cases.

Ruling R-T7-1: `call` in `signal-client.ts` reads the reply's body (`res.json()`) inside the same `try` as the fetch, raced against the abort, before its `finally` clears the timer, and returns `{ res, body }` — why: ride-along 1, a body that stalls after the headers hung the joiner's offer read past `SIGNAL_FETCH_TIMEOUT_MS`; racing against the abort ends even a body that ignores the signal — cost if wrong: every reply's body is now read, non-2xx ones too (harmless, and timed).

Ruling R-T8-1: Task 9 runs `cargo fmt` over Task 2's leftover long lines, format-only — why: one formatter pass is a nit, cheaper than a final-review fix round — cost if wrong: none (format-only, no pin can move). `cargo fmt --all -- --check` showed the same leftover in `runtime.rs`'s test as well as `console.rs:113`, so both went in one commit (`style(core): cargo fmt console.rs`).

Ruling R-T8-2: ride-along 9, the manifest outcome: applied. `frontend/public/apps/smoke-test/manifest.cfg` has `heap_kb=96` (was 64) and `wake_lock=0` (was `false`), within v392's loader rules (Milestone 12's R-T6-2) — why: Lua mode reads it the same way (`lifecycle.ts`'s `toBool` is `/^(1|true|yes)$/i`, so `0` gives `false`, as `false` did), and `heap_kb` only sets `badge.sys`'s reported `lua_limit`, which the app never reads — cost if wrong: the smoke-test app reports a 96 KiB Lua limit instead of 64; `manifest.test.ts` pins the real file's values.

Ruling R-T8-3: the tsconfig choice: `target` raised from ES2020 to ES2022, matching `lib` (Milestone 12's R-T4-1); Vite's `build.target` stays es2020 — why: the other option, `lib` back to ES2020, means rewriting 50 `.at()` calls and one `.replaceAll()`; Bun ignores `target`, Vite does not read it, and `dist/` is byte-identical; raising Vite's own target changed the shipped JS, which would change the browsers the bundle supports, out of scope — cost if wrong: R-T4-1's cost stands: `src/` can type-check an ES2021/ES2022 library call that an ES2020-only browser lacks, since `build.target` lowers syntax and adds no polyfills.

Ruling R-T8-4: a caller's `uploadToken` that is not exactly 64 bytes in UTF-8 fails the provision at the first poll with `upload token must be 64 bytes` (`BAD_TOKEN_REASON`) and types nothing; `null` still skips the token phase — why: Milestone 12's deferred minor, a wrong-length value waited out the timeout — cost if wrong: none today; the browser and the twin use the default and the benchmark passes `null`.

Ruling R-T8-5: a `close` that gets no reply (a timeout or a network failure) logs one `console.warn(CLOSE_FAILED_WARNING, room)`, `"signal: close got no reply; the room ends at its TTL:"`, with the room code and never the token; a non-2xx reply is still ignored silently — why: Milestone 12's deferred minor, a timed-out `close` was dropped silently — cost if wrong: the page's only signaling log line; the room code is short-lived and not secret.

Ruling R-T8-6: `wait_for_any` (`tests/common/provision.rs`) returns the needle printed first (the smallest console position; on a tie the earlier list entry), not the first in list order — why: Milestone 12's deferred minor; within one chunk, list order could report a later line — cost if wrong: none to pins; it still returns as soon as any needle appears, so no step count moves.

Ruling R-T8-7: Milestone 12's R-T1-1 wording was corrected in `docs/milestone-12-decisions.md` itself, with a Milestone 13 history note (the old reference repeated the stub's loop; Task 8 made it table-driven and checked it against `0xCBF43926`), rather than only here — why: the ruling's text overstated what the test checked, and a reader finds it there — cost if wrong: a later milestone's note inside Milestone 12's record; the ruling itself is unchanged.

## Final review

A whole-branch review approved the branch with fixes; the controller's
final fixer made them (`ce01928`, `689911e` and these docs). No Rust
changed.

Ruling R-FR-1: Skip setup hides on a stopped badge: `badge-controller.ts` keeps a `running` flag, set beside each `hub.setRunning(true/false)` (the start, a switch to Lua mode, and `onBadgeLost()`, which now also refreshes the status), the button shows only while it is set, and the click handler returns at once unless it is — why: the spec hides the button "on a stopped badge", but a badge lost to another tab only stopped the runtime, which posts no more statuses, so the button stayed up from the last one, and a click left the page on `skipping setup…` with Provision disabled until Run here — cost if wrong: none found; the button comes back with the next status after the badge runs again (`Skip setup hides on a badge lost to another tab, and a click sends nothing`).

Ruling R-FR-2: the provisioning hint's Skip setup sentence follows the setup's sentence and names its object: "…on “Shake it!” press Shake. Press Skip setup to jump past the setup. In Connect, press Bump." — why: after "In Connect, press Bump.", "jump past it" read as skipping Connect (a plan defect) — cost if wrong: text only (`shell.test.ts` pins the order).

Ruling R-FR-3: `lastStarted = "skip"` is set only for a skip that was sent: the handler returns before it unless the badge runs and the client's `setupOnScreen()` is true (the client's own guard) — why: a refused click set it anyway, which could bring back an old `skip failed: …` line over a newer provision's status — cost if wrong: display only (`a refused Skip setup click keeps the newer provision's status`).

The review's test minors went in with them: the mid-character
`consoleOffsetOf` test is renamed to what it pins (it rules out a
forward count, not a char-count tail); the Worker's extra-fields
`skipSetup` is now the first skip and types exactly one `config`;
the hung-body bound is 500 ms; and a new test covers the `whenAborted`
arm of `call`'s race (a `json()` that never settles and ignores the
signal still rejects with the "Could not reach the signaling server"
`SignalError` by the timeout). `bun test`: 392 → 395 (three new
tests), 15.39 s.

## Final suites

At `9e4f886` (the `cargo fmt` commit, after Task 8's `c2bdfaf`; the
commit after it is these docs only), run in Task 9:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean (after `9e4f886`) |
| `cargo test --workspace` | 764 passed, 0 failed, 1 ignored, 29.79 s wall: `emulator-core` lib 641; debug `boot_progress` 92 + 1 ignored in 27.03 s; the other integration binaries 31 |
| `cargo test -p emulator-core --release --test boot_progress` | 92 passed, 1 ignored, 17.99 s |
| `cargo clippy --workspace --all-targets` | 0 warnings |
| `bun run build:wasm` | OK |
| `bun test` | 392 pass, 0 fail, 19,213 `expect()` calls, 21 files, 15.56 s |
| `bun run typecheck` | 0 errors |
| `bun run build` | OK |

Against Milestone 12's final suites: release `boot_progress` 90 → 92
(Task 1's two rungs); `cargo test --workspace` 757 → 764 (the two rungs
and Task 2's five console tests: lib 636 → 641); `bun test` 339 → 392
(Tasks 2 to 8), then 395 after the final review (395 pass, 0 fail,
19,231 `expect()` calls, 21 files, 15.39 s; see "Final review").

`bun run build`'s output (bytes, `ls -l`): `index.html` 984,
`bridge-DKQjtxhY.js` 6,241, `firmware-worker-B3_rqCz3.js` 10,837,
`emulator_wasm_bg-BxQCJ19A.wasm` 143,645 (Vite's gzip 63.26 kB),
`index-XRjmGPO9.css` 2,962, `index-D-uZ9DUE.js` 262,923 (gzip
87.07 kB). Against Milestone 12's record: the bridge +322 B and the
wasm +1,375 B (`consoleWritten`, `consoleSince`), the Worker +1,891 B
(the skipper and the status fields), `index-*.js` +1,134 B (the button,
the notes, the signaling body read).

## Milestone 14 backlog

### New in Milestone 13

The deferred review minors from the controller's ledger. The final
review may close some of these; what it closes comes off this list.

- **Task 1 (the rungs):** `skip_setup` can panic through `type_line`'s
  assert on a refused queue while it returns `Err` for `reboot`'s
  refusal (an inconsistent `Result` contract); the reboot rung
  re-implements `skip_setup`'s reset-wait loop (`FRAME_CHUNK` against
  `CHUNK`; a `reboot_and_wait` helper could serve both); it borrows
  `SKIP_SETUP_DEADLINE_STEPS` for the reboot alone (R-T1-3); the two
  rungs sit after the `provisions_role!` list, not near
  `walk_to_registered_from`; the reboot rung's emulated time
  (12,630,966 µs) is unpinned.
- **Task 2 (the console offset):** `cpu-wasm.test.ts`'s no-wrap guard
  (~:390) compares JS string length, not bytes (use `consoleWritten()`);
  `text_since` silently returns the whole ring for an `at` older than
  the ring (documented; a caller can detect `written - at >
  CONSOLE_CAPACITY`).
- **Task 3 (the provisioner):** `!==` against the doc's "a rise" for the
  reboot check (a safe superset, R-T3-1).
- **Task 4 (the skipper):** `setup-skip.ts`'s `usage:` match is not
  anchored to a line start or to before the OK (the firmware's help text
  has "Heap usage: …").
- **Task 5 (the Worker):** no test pins `setupOnScreen` being
  computed only when a status is built (verified by reading
  `firmware-worker.ts`); a refused provision overwrites an earlier
  `done` state with `failed` (R-T4-2).
- **Task 6 (the page):** `shell.ts`'s no-op fallback literal is about
  120 characters on one line; a `done` skip falls back to the older
  `PROV OK` line (R-T6-1); `lastStarted` is not reset on a mode switch
  or `reset()` (display only); Remove leaves a linked room's
  `Room X: linked.` line (R-T6-3); a refused Bump always says "the badge
  isn't running" (R-T6-4).
- **Task 8 (the minors):** `manifest.test.ts` (~:88-90) repeats the
  assertion at ~:85; `signal-client.test.ts` (~:318-329) restores the
  `warn` spy through a catch-rethrow and an after-loop, where
  `try`/`finally` is simpler.
- **The final review:** `setupOnScreen` decodes the whole boot's
  console, up to 256 KiB, four times a second (once per status); it
  could scan only from the last `app_reg: launched` offset.

### Carried from Milestone 12's backlog (still open)

Word for word in `docs/milestone-12-decisions.md`, "Milestone 13
backlog", "New in Milestone 12", minus what this milestone closed: the
fetch timeout at the headers (Task 7), re-provisioning's tutorial reset
(handled by Skip setup, above), the smoke-test manifest (Task 8) and
Milestone 12's other deferred review minors (Task 8, all nine).

- **Share between two browsers** (Task 6: the core and the controller
  model need no change). Three pieces: a two-badge Share rung pinning
  the probe's end; a browser control that installs a Lua app over the
  console, as the provisioner types; a two-browser check of a transfer
  over WebRTC, where the Worker posts the advertisement at most at
  10 Hz and a DATA frame dwells only 60 to 90 ms (STAT repair should
  cover the frames it misses; untested).
- **A fake sync station.** The protocol is advertisement-only (v392's
  NimBLE links no connect or GATT procedure, Task 6's trace; Sync's
  OPEN goes out through `send_open`), so a station is possible on
  today's model. The human partner chose
  not to build one this milestone (2026-11-10).
- **A stopwatch check of Sync's 20 s on a v392 badge** (R-T6-1): from A
  to "No sync station in range", about 20 s (the emulator's 750 ticks of
  20 or 30 ms) against about 15 s (every tick 20 ms).

### Carried from Milestone 11's backlog (still open)

Word for word in `docs/milestone-12-decisions.md`, "Carried from
Milestone 11's backlog (still open)", minus what this milestone closed:
the provisioner's two items (Task 3), and from the browser item, no
feedback for a refused Bump, the host tab's stale "Room …: enter this
code" line after Remove, the speed readout through the reboot's pause
(R-T8-2) and `status.softResets` (Tasks 5 and 6).

- **A cold power-on's 466 ms.** The shortcut boot starts the app at
  0 µs; the badge takes about 466 ms. Adding it would move every pin for
  no visible gain (Milestone 11's spec, "Non-goals").
- **A joiner who answered before a cancel waits out `waitOpen`.** A
  joiner who arrives after the cancel gets the 404 at once. Covering the
  earlier one needs a new poll, which costs commands (Milestone 11's
  spec, Part 4.6).
- **The reset model:** the CPU-reset table's silent rows (INTC, R-T1-2;
  the external chips); the core reset narrower than the TRM; the SYSTEM
  clock-gate and reset bits not modeled; DRAM `.noinit` not kept; the
  RTC and timer-group watchdogs inert; three reboot apps' 465,999 µs
  deltas, untraced; v335's 0.2M steps after `PROV FAIL`, not
  established.
- **The radio model:** `host_rcv_pkt` blocking on the real firmware is
  not established (Milestone 10's R-T6-2, half open); LE Set Extended
  Scan Enable's other parameter rules (Core v5.4, 7.8.65).
- **The browser:** Provision in a tab whose badge was since freed; a
  Provision and a `startFirmware` racing on a claim; an advertisement
  change just before a Worker stop never posted; `inject` feeding a
  stopped runtime behind the "another tab" notice; a shake or bump
  across a reboot losing its tail (R-T8-3).
- **Carried from Milestone 8's backlog**, as `milestone-11-decisions.md`
  lists them: Milestone 7's final-review items; speed and timing (Snake's
  late game-over after an eat, CPI 1, zero-latency RMT, I2C0 and
  SPIMEM1, PCER/PCMR, `PROVISION_TIMEOUT_STEPS` in steps); apps and UI
  (no app's deeper screens tested beyond Connect's pairing and Sync's
  "not ready" and "no station", a browser RNG seed, a console pane,
  `DeviceMotionEvent`, Milestone 5 Task 7's minors, Task 7's small
  cleanups, only role hacker's frames compared with hardware);
  peripherals (RMT, NFC, the busy phase after START, SC7A20H, I2C0's
  empty-FIFO `OP_WRITE`, the `put` task's CPU time, `R_M5_3`'s closure,
  `strtol`/`atoi` and `errno`, the 8 MiB stub cap, the idle unmapped
  accesses); build and tests (the release suite's one chain, Milestone
  6's test minors); and Milestones 3 and 4 (`MADCTL`, GPIO0 routing, the
  eFuse block beyond the factory MAC, flagged ROM-stub guesses, edge
  interrupts, GDMA's corners, the 250,000-step chunks, small
  duplications).

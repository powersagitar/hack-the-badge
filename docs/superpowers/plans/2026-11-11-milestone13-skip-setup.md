# Milestone 13: Skip Setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Skip setup" button, shown only while the firmware's Setup app is on screen. It types `config system onboard_build 2147483647` and `reboot` into the badge's console and lands on My Badge's registered screen. Plus the approved backlog ride-alongs.

**Architecture:** First, a Rust rung proves the two console lines on v392 (Task 1 is the go/no-go gate). The browser side then mirrors it: an absolute console offset from the core (Task 2), shared console helpers with the provisioner fixed around reboots (Task 3), a setup skipper and a Setup watcher in the Worker's runtime (Task 4), new status fields and a `skipSetup` message (Task 5), and the button (Task 6). Tasks 7 and 8 are independent ride-alongs. Task 9 is the docs and the human partner's browser check.

**Tech Stack:** Rust (emulator-core, wasm-bindgen shim), TypeScript, Bun test, Vite.

**Spec:** `docs/superpowers/specs/2026-11-11-milestone13-skip-setup-design.md`

## Global Constraints

- Every existing step count, console needle, emulated time and frame hash stays byte-identical. `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged` must pass untouched. A moved pin is a bug in the change.
- The rungs keep walking all 11 Setup pages (R-T4-2 stands for tests).
- `emulator-core/src/cpu/` and `clock.rs` hold no chip or v392 constant. `emulator-core` takes no wasm-bindgen dependency. `emulator-wasm` has one-line passthroughs only.
- Unmapped data access never panics. Browser, peer and network input never panics the emulator.
- Bun only (`bun test`, `bun run …`), never npm/node. Run Cargo from the repo root. Re-run `bun run build:wasm` after any change under `emulator-core/` or `emulator-wasm/`.
- Pin the exact version of any new dependency and tell the human partner. None is expected.
- The skip's two lines, exactly: `config system onboard_build 2147483647` and `reboot`. The replies to wait for: `OK system.onboard_build=2147483647`, then `rebooting...`.
- Setup's launch needle: `app_reg: launched Setup`. Any other launch: `app_reg: launched `.
- If v392 disagrees with the public source (Task 1 fails for a reason other than a pin to measure), **stop and report to the human partner**. Do not fall back to a scripted walk.
- `cargo clippy --workspace --all-targets` and `bun run typecheck` stay clean.
- Work on branch `milestone-13` in a worktree (superpowers:using-git-worktrees). Record `BASE` (this plan's commit) in `docs/milestone-13-decisions.md`.

## Review Focus

1. **Skip pressed during a provision still typing** (the token phase runs after Setup appears): the press must be refused, never interleave lines. Test in Task 4 (`skipSetup refuses while a provision is in flight`).
2. **A double click, or a click landing just as the status updates**: exactly one `config` and one `reboot` are typed. Test in Task 4 (`a second skipSetup while one runs is refused`) and Task 6 (the button hides while `skip` is in flight).
3. **The page reloaded or the mode switched mid-skip** (after `config`, before `reboot`): the next boot is still Setup, the button reappears, and a fresh skip works. Test in Task 4 (`a reset drops the skip; setupOnScreen recomputes from the new boot`).
4. **Re-provisioning from the launcher**: Setup comes back on the next press, and the button must appear then, not at `PROV OK`. Test in Task 4 (`setupOnScreen follows the last launch line of the current boot`, with a "launched Launcher" then "launched Setup" sequence).
5. **A console that has wrapped 256 KiB** (a long session) still finds the prompt and the launch line. Test in Task 3 (`consoleSince is exact past a wrap`) and Task 4 (the watcher reads through `consoleSince`).

---

### Task 1: The skip on v392, as boot rungs (the gate)

**Files:**
- Modify: `emulator-core/tests/common/provision.rs` (append)
- Modify: `emulator-core/tests/boot_progress.rs` (two rungs near `walk_to_registered_from`, ~line 1445)

**Interfaces:**
- Produces (test-only, Rust):
  - `pub const SKIP_SETUP_CONFIG: &str = "config system onboard_build 2147483647";`
  - `pub const SKIP_SETUP_CONFIG_OK: &str = "OK system.onboard_build=2147483647";`
  - `pub const REBOOT: &str = "reboot";`
  - `pub const REBOOTING: &str = "rebooting...";`
  - `pub const SETUP_LAUNCHED: &str = "app_reg: launched Setup";`
  - `pub fn skip_setup(rt: &mut FirmwareRuntime, deadline: u64) -> Result<String, String>`: types `SKIP_SETUP_CONFIG` with `type_line`, checks for `SKIP_SETUP_CONFIG_OK` (an `Err` with the console text if missing or on `usage:`), then types `REBOOT` and runs (with `wait_for_any` semantics, `CHUNK` steps, asserting no fault) until `rt.soft_reset_count()` has risen by one. Returns the console text from the `config` line on.
- Uses `provisioned(OWN_ROLE)`, `run_until_stable_frame`, `press`, `role_row`, `SPLASH_HASH`, `LAUNCHER_HASH`, `TO_LAUNCHER_SLOT`, `LAUNCHER_RESPONSE_STEPS`, `AFTER_PRESS_STABLE_FOR`, `assert_no_panic_text` from `boot_progress.rs`.

- [ ] **Step 1: Write rung `skipping_setup_lands_on_the_registered_screen`**

From `provisioned(OWN_ROLE)` (Setup page 1, stable):
```rust
assert!(rt.console_output().contains(provision::SETUP_LAUNCHED));
let resets = rt.soft_reset_count();
let text = provision::skip_setup(&mut rt, rt.total_steps() + SKIP_SETUP_DEADLINE_STEPS).unwrap();
assert!(text.contains(provision::SKIP_SETUP_CONFIG_OK) && text.contains(provision::REBOOTING));
assert_eq!(rt.soft_reset_count(), resets + 1);
let boot_start = /* console length right after the reset */;
let (hash, step) = run_until_stable_frame(&mut rt, rt.total_steps() + SKIP_REGISTERED_STEPS,
    AFTER_PRESS_STABLE_FOR, &[page1, SPLASH_HASH]);
assert_eq!(hash, role_row(OWN_ROLE).1 /* 0xbd6d_7613_08a7_bf24 */, "registered after skip, step {step}");
let new_boot = &rt.console_output()[boot_start..];
assert!(new_boot.contains("app_reg: launched My Badge") && !new_boot.contains(provision::SETUP_LAUNCHED));
assert_eq!((rt.total_steps(), rt.emulated_time_us()), (SKIP_REGISTERED_AT_STEPS, SKIP_REGISTERED_AT_US));
press(&mut rt, TO_LAUNCHER_SLOT);
// then the launcher, as walk_to_launcher does: LAUNCHER_HASH within LAUNCHER_RESPONSE_STEPS
assert_no_panic_text(&rt);
```
The new constants (`SKIP_SETUP_DEADLINE_STEPS`, `SKIP_REGISTERED_STEPS`, `SKIP_REGISTERED_AT_STEPS`, `SKIP_REGISTERED_AT_US`) start as placeholders. Measure them on the first run, then set the deadline and response budgets as the file's other budgets are set (measured + 25 %, rounded up to 1,000,000). The two `AT_` values are exact pins. Each gets a doc comment with its measurement.

- [ ] **Step 2: Write rung `setup_logs_its_launch_after_a_reboot`**

From `provisioned(OWN_ROLE)`: type `REBOOT` alone (`type_line` won't see a prompt in time, so send it with `serial_input` and run until `soft_reset_count` rises), then run until the new boot's console (from the reset's console length) contains `SETUP_LAUNCHED`, and the frame is stable. Pin that hash as measured. Setup's page 1 (`role_row(OWN_ROLE).0`) is expected. If it differs, record what the screen shows. This confirms the boot-time path (`app_registry_start`) logs the launch on v392, so the Setup watcher needs only the one needle.

- [ ] **Step 3: Run them**

Run: `cargo test -p emulator-core --release --test boot_progress skipping_setup setup_logs_its_launch -- --nocapture`
Expected the first time: the placeholder pins fail with measured values. Set them and re-run until both PASS.
**Gate:** if `SKIP_SETUP_CONFIG_OK` never appears, `usage:` appears, the reboot lands anywhere but the registered screen, or the boot after a mid-Setup reboot doesn't log `launched Setup`, stop and report to the human partner with the console text.

- [ ] **Step 4: Full ratchet and clippy**

Run: `cargo test -p emulator-core --release --test boot_progress` → 92 passed, 1 ignored (90 + 2). Run `cargo clippy --workspace --all-targets` → no warnings.

- [ ] **Step 5: Commit**

```bash
git add emulator-core/tests/common/provision.rs emulator-core/tests/boot_progress.rs
git commit -m "test(boot): skip Setup through the console on v392"
```

### Task 2: An absolute console offset

**Files:**
- Modify: `emulator-core/src/peripherals/console.rs`, `emulator-core/src/runtime.rs` (beside `console_output`, ~line 560)
- Modify: `emulator-wasm/src/lib.rs` (beside `consoleOutput`, ~line 194)
- Modify: `frontend/src/cpu/bridge.ts` (`FirmwareEmulatorHandle`, ~line 97, and its impl ~243)
- Test: `console.rs` tests, `frontend/test/cpu-wasm.test.ts`

**Interfaces:**
- Produces:
  - `Console::written(&self) -> u64`: bytes ever pushed since construction. It isn't reset by the ring's cap, and it survives a soft reset with the console.
  - `FirmwareRuntime::console_written(&self) -> u64`.
  - wasm getter `consoleWritten: f64` (as `totalSteps`).
  - TS `FirmwareEmulatorHandle.consoleWritten(): number`.
- Note: a cold `reset()` makes a new console, so `written` is 0 again. Check how `reset()` treats the console and match it.

- [ ] **Step 1: Failing Rust test `written_counts_past_the_cap`**: push `CONSOLE_CAPACITY + 10` bytes. Then `written() == CONSOLE_CAPACITY as u64 + 10` and `bytes().len() == CONSOLE_CAPACITY`. Add a runtime test: `console_written` keeps its count across `SOFTWARE_RESET_CPU` (beside `console_output_survives_a_soft_reset`).
- [ ] **Step 2: Run** `cargo test -p emulator-core console` → FAIL (no method).
- [ ] **Step 3: Implement** a `written: u64` field incremented in `push`, plus the three passthroughs.
- [ ] **Step 4: Run** `cargo test -p emulator-core` → PASS. Run the ratchet (`--release --test boot_progress`) → unchanged. Run `bun run build:wasm`. Add a `cpu-wasm.test.ts` case: after a bounded boot, `consoleWritten() === consoleOutput().length` (ASCII console, no wrap). Run `bun test test/cpu-wasm.test.ts` → PASS.
- [ ] **Step 5: Commit** `feat(core): console_written, an absolute console offset`.

### Task 3: Shared console helpers; the provisioner around reboots (ride-along 3)

**Files:**
- Create: `frontend/src/runtime/console-io.ts`
- Modify: `frontend/src/runtime/provisioner.ts`, `frontend/src/runtime/firmware-runtime.ts` (`bootConsoleStart` → absolute)
- Test: `frontend/test/provisioner.test.ts`, `frontend/test/firmware-runtime.test.ts`

**Interfaces:**
- Consumes: `FirmwareEmulatorHandle.consoleWritten()` (Task 2).
- Produces (`console-io.ts`):
  - `export function consoleSince(handle: FirmwareEmulatorHandle, at: number): string`. The console text from absolute offset `at` on: `text.slice(Math.max(0, text.length - (handle.consoleWritten() - at)))`. If `at` is older than the ring, it returns the whole ring. This assumes one byte per char, which holds for the ASCII console. Say so in the doc comment.
  - `export function completeLine(text: string, at: number): string | undefined` (moved from `provisioner.ts`).
  - `export function typeLine(handle: FirmwareEmulatorHandle, line: string): number | null`. It sends `line + LINE_END` and returns the absolute offset before the line was sent (`consoleWritten()`), or `null` if `serialInput` refused bytes.
  - `PROMPT`, `LINE_END` move here. `provisioner.ts` re-exports them so existing imports still work.
- `createProvisioner`'s `bootConsoleStart: () => number` now returns an **absolute** offset. `firmware-runtime.ts` sets it to `handle.consoleWritten()` at a reset (was `consoleOutput().length`). `marker` and `provOkEnd` become absolute too, read through `consoleSince`.
- 3(b), decided: a provision **fails** with reason `the badge rebooted during provisioning` when `handle.softResetCount()` rises after the provisioner left its `prompt` phase. A reboot while it still waits for the first prompt stays fine, as today. Record this as a ruling in Task 9.

- [ ] **Step 1: Failing tests.** In `provisioner.test.ts`, the fake handle gains `consoleWritten` and a ring cap:
  - `consoleSince is exact past a wrap`: a fake with a 64-char ring has had 200 chars written. `consoleSince(h, 190)` is the last 10.
  - `finds the first prompt in a full console after a soft reset` (R-T13-3): the ring is full, a reset at absolute 300,000, then `badge> ` arrives. The provisioner types `put …`.
  - `fails when the badge reboots mid-provision`: after `put` is typed, bump `softResetCount`. Then `poll()` is `{ kind: "failed", reason: "the badge rebooted during provisioning" }`, and nothing more is typed.
  - The existing provisioner tests pass unchanged, with `consoleWritten` added to their fakes.
- [ ] **Step 2: Run** `bun test test/provisioner.test.ts` → the new ones FAIL.
- [ ] **Step 3: Implement** `console-io.ts`, and move the provisioner onto it (absolute markers, reboot check). Switch `firmware-runtime.ts`'s `bootConsoleStart` to `consoleWritten()`.
- [ ] **Step 4: Run** `bun test` → all pass (339 + new), including `cpu-wasm.test.ts`'s provisioned walk unchanged. Run `bun run typecheck` → clean.
- [ ] **Step 5: Commit** `fix(provisioner): absolute console offsets; fail on a reboot mid-provision`.

### Task 4: The setup skipper and the Setup watcher, in the runtime

**Files:**
- Create: `frontend/src/runtime/setup-skip.ts`
- Modify: `frontend/src/runtime/firmware-runtime.ts`
- Test: create `frontend/test/setup-skip.test.ts`; modify `frontend/test/firmware-runtime.test.ts`, `frontend/test/cpu-wasm.test.ts`

**Interfaces:**
- Consumes: `consoleSince`, `typeLine`, `completeLine`, `PROMPT` (Task 3).
- Produces (`setup-skip.ts`):
  - `export const SKIP_SETUP_CONFIG = "config system onboard_build 2147483647"`, `SKIP_SETUP_CONFIG_OK = "OK system.onboard_build=2147483647"`, `REBOOT_LINE = "reboot"`, `SETUP_LAUNCHED = "app_reg: launched Setup"`, `LAUNCHED = "app_reg: launched "`, `SKIP_SETUP_TIMEOUT_STEPS = 60_000_000`.
  - `export type SkipState = { kind: "typing" } | { kind: "done" } | { kind: "failed"; reason: string }`.
  - `export function setupOnScreen(bootText: string): boolean`: true iff the **last** complete `app_reg: launched <name>` line in `bootText` names `Setup`.
  - `export interface SetupSkipper { poll(): SkipState }`.
  - `export function createSetupSkipper(handle: FirmwareEmulatorHandle, bootConsoleStart: () => number): SetupSkipper`. Phases: wait for `PROMPT` in the current boot (`consoleSince(handle, bootConsoleStart())`), then type the config. Wait for `SKIP_SETUP_CONFIG_OK` and a prompt after it (a `usage:` line before it means failed, with that line). Type `REBOOT_LINE`. Done when `softResetCount()` rises.
- `FirmwareRuntime` gains:
  - `skipSetup(): boolean`. It starts a skipper and returns true only if `setupOnScreen()` is true, no provisioner is in flight and no skip is in flight. Otherwise it returns false and does nothing.
  - `skipState(): SkipState | null`. The last state. `null` before any skip and after `reset()`.
  - `setupOnScreen(): boolean`. `setupOnScreen(consoleSince(handle, bootConsoleStart))`, false while a skip is in flight.
  - `rebooting(): boolean`. `rebootDebtUs > 0`. Used by Task 5/6 (ride-along 4).
  - The skipper is polled in `loop` beside `pollProvisioner`, with the same steps timeout (`failed: timed out`). `reset()` drops it.

- [ ] **Step 1: Failing tests (`setup-skip.test.ts`, fake handle as in `provisioner.test.ts`):**
  - `setupOnScreen follows the last launch line of the current boot`: `""` → false. `"…launched My Badge\n…launched Setup\n"` → true. `"…launched Setup\n…launched Launcher\n"` → false. `"…launched Launcher\n…launched Setup\n"` → true (re-provision). An unterminated `"…launched Se"` doesn't count.
  - `types config, then reboot, and is done at the reset`: it asserts the exact bytes typed: `config system onboard_build 2147483647\n`, then (only after `OK system.onboard_build=2147483647` and `badge> `) `reboot\n`. Then `softResetCount` +1 → `{ kind: "done" }`.
  - `fails on a usage reply`: `usage: config <slug> <key> <value>` → `{ kind: "failed", reason: "usage: config <slug> <key> <value>" }`, and `reboot` is never typed.
  - In `firmware-runtime.test.ts`:
    - `skipSetup refuses while a provision is in flight`;
    - `a second skipSetup while one runs is refused`;
    - `skipSetup refuses when Setup is not on screen`;
    - `a reset drops the skip; setupOnScreen recomputes from the new boot`;
    - `rebooting is true while the reboot's debt is unpaid`.
- [ ] **Step 2: Run** `bun test test/setup-skip.test.ts test/firmware-runtime.test.ts` → FAIL.
- [ ] **Step 3: Implement** `setup-skip.ts` and the runtime methods.
- [ ] **Step 4: WASM twin.** In `cpu-wasm.test.ts`'s provisioned-boot describe, add `skips Setup to the registered screen`. Provision role hacker through `createProvisioner`, run to Setup page 1, then drive `createSetupSkipper` to `done` (resetting the boot start at the soft reset, as the runtime does). Run to a stable frame, then check `framebufferHash` (or the file's existing hash helper) equals `0xbd6d_7613_08a7_bf24`'s TS form, as the file pins the launcher. Run `bun run build:wasm && bun test` → all pass. Keep the added time under ~5 s and note it.
- [ ] **Step 5: Commit** `feat(runtime): skip Setup through the console; Setup watcher`.

### Task 5: Worker, protocol and client (ride-alongs 4 and 5)

**Files:**
- Modify: `frontend/src/runtime/firmware-worker-protocol.ts`, `firmware-worker.ts`, `firmware-worker-client.ts`
- Test: `frontend/test/firmware-worker.test.ts`

**Interfaces:**
- Consumes: Task 4's runtime methods and `SkipState`.
- Produces:
  - `ToWorker` gains `{ kind: "skipSetup" }`. The Worker calls `rt.skipSetup()`.
  - The `status` message becomes `{ kind: "status"; speed: number; rebooting: boolean; provision: ProvisionState | null; skip: SkipState | null; setupOnScreen: boolean }`. **`softResets` is removed** from the message (ride-along 5, decided: the Worker keeps its internal count for the immediate post, and the page now gets `rebooting` instead). `setupOnScreen` is computed only when a `status` is built (at most every 250 ms), because it copies the console, which is too costly per slice.
  - `WorkerFirmwareRuntime` gains `skipSetup(): void`, `skipState(): SkipState | null`, `setupOnScreen(): boolean`, `rebooting(): boolean`, read from the last status. `skipSetup()` optimistically sets the local `skipState` to `{ kind: "typing" }` and `setupOnScreen` to false until the next status, so a second click can't send twice.

- [ ] **Step 1: Failing tests** in `firmware-worker.test.ts`:
  - `skipSetup reaches the runtime`;
  - `status carries rebooting, skip and setupOnScreen, and no softResets`;
  - client: `skipSetup hides the skip until the next status`.
- [ ] **Step 2: Run** `bun test test/firmware-worker.test.ts` → FAIL.
- [ ] **Step 3: Implement.** Validate the message's kind only (it has no fields).
- [ ] **Step 4: Run** `bun test` and `bun run typecheck` → pass.
- [ ] **Step 5: Commit** `feat(worker): skipSetup message; status carries Setup, skip and rebooting`.

### Task 6: The page (the button; ride-alongs 4, 6, 7)

**Files:**
- Modify: `frontend/src/ui/shell.ts` (`buildProvisionControls`, ~line 114; `ProvisionControls`, ~56), `frontend/src/ui/badge-controller.ts` (`refreshStatus`, ~309; the Bump handler, ~305; `remove`, ~479)
- Test: `frontend/test/shell.test.ts`

**Interfaces:**
- Consumes: Task 5's client methods. `hub.bump(): boolean` (false when the bump gate is closed).
- Produces:
  - `buildProvisionControls(…, onSkipSetup: () => void)`, with a new trailing parameter. It also threads through `ShellHandles.mountProvisionControls`.
  - A button labelled `Skip setup`, placed after `Provision test badge`.
  - `ProvisionControls.setSkipVisible(visible: boolean): void` (the `hidden` attribute).
- `refreshStatus` rules:
  - Skip visible iff `mode === "firmware"`, the runtime exists, `setupOnScreen()` is true, and `skipState()?.kind !== "typing"`.
  - Status text: a skip in flight → `skipping setup…`. Skip `failed` → `skip failed: <reason>`. Skip `done` shows nothing new; the badge's screen shows the result.
  - Speed prefix: `rebooting…` while `rebooting()` is true, else the `1.0x` reading as today (ride-along 4).
- Ride-along 7: when `hub.bump()` returns false, show `bump: the badge isn't running` in the status for 3 s (a `bumpNote` like `fetchError`, cleared by a timer).
- Ride-along 6: in `remove(id)`, when the removed entry is the hosted room's link, also `panel.setStatus("")`.
- The hint text drops nothing. Append `Press Skip setup to jump past it.` to it.

- [ ] **Step 1: Failing tests** in `shell.test.ts`, with the controller's existing fakes:
  - `Skip setup shows only while Setup is on screen`: fake runtime `setupOnScreen` true → visible. false → hidden. Lua mode → hidden.
  - `Skip setup hides while a skip runs, and a click calls skipSetup once`.
  - `the speed reads rebooting… during the reboot's pause`.
  - `a refused bump says why`.
  - `removing the hosted room clears its code line`.
- [ ] **Step 2: Run** `bun test test/shell.test.ts` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `bun test`, `bun run typecheck`, `bun run build` → pass. Smoke-check in `bun run dev`: Provision hacker → Skip setup appears → press → `rebooting…` → registered screen → HOME → launcher. Note the result for Task 9 (the human partner's check is separate).
- [ ] **Step 5: Commit** `feat(ui): Skip setup button; rebooting readout; bump and room fixes`.

### Task 7: The fetch timeout covers the body (ride-along 1)

**Files:**
- Modify: `frontend/src/radio/signal-client.ts` (`call`, ~108; its callers' `res.json()` at ~154 and ~243)
- Test: `frontend/test/signal-client.test.ts`

**Interfaces:**
- `call` becomes `async function call(d: Deps, init: RequestInit, query?: string, signal?: AbortSignal): Promise<{ res: Response; body: unknown }>`. It reads `await res.json()` (a parse failure gives `body: null`) **inside** the `try`, before the `finally` clears the timer. An abort during the body read throws the same `SignalError` as a fetch failure. Callers use `body` in place of their own `res.json()`, keeping their current handling of a `null`/non-object body.

- [ ] **Step 1: Failing test** `a body that never ends times out`: the fake `fetch` resolves a `Response` whose `ReadableStream` body never closes (but errors when the signal aborts). With `fetchTimeoutMs: 50`, `hostRoom` rejects with the "Could not reach the signaling server" `SignalError` within ~100 ms.
- [ ] **Step 2: Run** `bun test test/signal-client.test.ts` → FAIL (hangs to the test timeout).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `bun test test/signal-client.test.ts test/signal-handler.test.ts` → PASS.
- [ ] **Step 5: Commit** `fix(signal): the fetch timeout covers the reply's body`.

### Task 8: The smoke-test manifest and Milestone 12's review minors (ride-alongs 8 and 9)

**Files:**
- Modify: `frontend/public/apps/smoke-test/manifest.cfg`, `frontend/test/manifest.test.ts` (if it reads the real file; its literal fixture stays)
- Modify: as each minor in `docs/milestone-12-decisions.md`, "Milestone 12's other deferred review minors", names (`emulator-core/src/cpu/rom_stubs.rs`, `emulator-core/src/rom.rs`, `emulator-core/tests/common/provision.rs`, `emulator-core/tests/boot_progress.rs`, `frontend/src/runtime/provisioner.ts`, `frontend/tsconfig.json`, `frontend/test/shell.test.ts`, `frontend/test/provisioner.test.ts`, `frontend/test/signal-client.test.ts`, the decisions doc)

- [ ] **Step 1: Manifest.** Set `heap_kb=96` and `wake_lock=0`. Add a `manifest.test.ts` case: `parseManifest` of `wake_lock=0` gives `false`. If `toBool("0")` isn't false, revert the manifest, record why, and skip the rest of this step. Run `bun test test/manifest.test.ts test/lifecycle.test.ts` → PASS, including "integration: the real public/apps/smoke-test app".
- [ ] **Step 2: The minors**, one commit per milestone task group (Tasks 1–5 of Milestone 12), each leaving every pin unchanged:
  - **T1:**
    - make the wild-buffer test's reference independent: a table-driven CRC-32, or a published check value such as `crc32("123456789") = 0xCBF43926`, adjusted to `crc32_le`'s seed convention;
    - fix R-T1-1's wording;
    - align the `esp_rom_crc.h` line range in `rom_stubs.rs` and `rom.rs` to the header's real lines;
    - change the `--` to `—`.
  - **T2:**
    - snapshot the console after the `complete=1` line ends, not at the prefix;
    - say in `PROVISION_DEADLINE`'s doc that it also covers the token's ~0.29M steps;
    - drop the redundant `TOKEN_OK` assert;
    - check `set_upload_token`'s needles in console order.
  - **T3:**
    - make "reports PROV OK" end in `ok`, by running the token phase in the fake;
    - validate `uploadToken`: not exactly 64 bytes fails at once with `upload token must be 64 bytes`;
    - remove the `token as string` cast;
    - wrap the long line.
  - **T4:**
    - set `lib` to match `target`, or raise `target` to ES2022 if Bun and Vite allow it (record which);
    - remove the commented cast in `shell.test.ts`.
  - **T5:**
    - add `close` tests for `waitOpen`'s deadline and for a bad answer;
    - log (`console.warn`) a timed-out `close` instead of dropping it silently.
- [ ] **Step 3: Run** the whole suite: `cargo test --workspace`, `cargo test -p emulator-core --release --test boot_progress`, `cargo clippy --workspace --all-targets`, `bun run build:wasm && bun test`, `bun run typecheck` → all pass, clippy clean.
- [ ] **Step 4: Commit** each group as `chore: Milestone 12 review minors (Task N)`. The manifest is `fix(apps): smoke-test manifest within v392's loader rules`.

### Task 9: Docs, and the human partner's check

**Files:**
- Create: `docs/milestone-13-decisions.md`
- Modify: `CLAUDE.md` ("Current state", the rung count 90 → 92, the test count, `runtime/provisioner.ts` and the new `setup-skip.ts`/`console-io.ts` in "Architecture", `console.rs`'s `written`), `docs/firmware-emulator-notes.md` (the onboarding section ~line 727; "Known limitations" item 11 on re-provisioning; the provisioner items)

- [ ] **Step 1: Decisions doc**, in Milestone 12's shape. It records:
  - Why, and BASE;
  - Task 1's measurements and pins;
  - the rulings: 3(b) fails rather than retries; `softResets` removed; the manifest outcome; the tsconfig choice; and that the boot-time Setup launch logs (Task 1 Step 2);
  - re-provisioning's tutorial reset, closed as "handled by Skip setup";
  - a Milestone 14 backlog: Milestone 12's list minus what this milestone closed, word for word, plus anything new.
- [ ] **Step 2: CLAUDE.md and the notes.** Run the commands they quote to confirm the counts (`bun test` total and time; the ratchet's rung count and time).
- [ ] **Step 3: The human partner's browser check** (prod build: `bun run build && bun run preview`):
  - Provision hacker → the Skip setup button appears with Setup;
  - press it → `rebooting…` → the registered screen → HOME → the launcher;
  - re-provision from the launcher → press a button → Setup and the button come back.

  Record their result in the decisions doc, in their words.
- [ ] **Step 4: Commit** `docs: Milestone 13 decisions, notes and CLAUDE.md`.

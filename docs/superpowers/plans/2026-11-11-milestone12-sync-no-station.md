# Milestone 12: Sync to "No sync station in range" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sync, provisioned through the browser, runs past "Badge not ready to sync" to "No sync station in range". Three ride-alongs come with it: three signaling fixes, type-checked test files, and a recorded trace of Share's transfer.

**Architecture:**
- **The core** gains one ROM HLE stub, `crc32_le` at `0x4000_0628`. Its address lives in `rom.rs`. Its effect is a generic CRC-32 in `cpu/`.
- **Both provisioners** type the firmware's own `badge_token set` after `prov apply`, with one fixed fake token. The Rust helper does it only when a rung asks; the browser does it on every Provision.
- **New rungs** pin Sync's path with a credential. No existing rung changes.

**Tech Stack:** Rust (`emulator-core`), wasm-pack, Bun/TypeScript (`frontend/`), the Vercel function over `@upstash/redis` 1.41.0. No new dependencies are planned. If one is added anyway, it is named to the human partner.

**Spec:** `docs/superpowers/specs/2026-11-10-milestone12-sync-no-station-design.md` (approved 2026-11-11). Read "Facts read" and "Measured" before Tasks 1–3.

## Global Constraints

- **Pins.** No Milestone 11 pin moves: every step count, console needle, emulated time and frame hash in today's suites stays byte-identical.
  - `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged` guard this.
  - The Rust `provision::provision` is unchanged. Only new rungs set the token.
  - The WASM twin (`frontend/test/cpu-wasm.test.ts`) pins frames only. It takes the token through the browser provisioner's default, and its frame pins must still hold.
  - A moved pin is a bug in the change, never a re-pin.
- **The token:** `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`, one constant for every role.
  - It is named `FAKE_UPLOAD_TOKEN` in `emulator-core/tests/common/provision.rs` and in `frontend/src/runtime/provisioner.ts`.
  - It never goes into the identity JSON files, which stay byte-identical.
- **ROM:** `CRC32_LE = 0x4000_0628`, cited to ESP-IDF v5.5.3's `components/esp_rom/esp32c3/ld/esp32c3.rom.ld:415`.
  - Semantics are from ESP-IDF v5.5.3's `components/esp_rom/include/esp_rom_crc.h`, fetched and cited: zlib CRC-32, polynomial `0xEDB88320`, the value inverted in and out, chainable.
- **Firmware strings** (v392, present in `firmware.bin`): `BADGE_TOKEN READY`, `BADGE_TOKEN OK`, `BADGE_TOKEN FAIL`, `usage: badge_token`, "No sync station in range", "Move closer to the sync desk", "A: try again   -   HOME: home".
- **The core's structural rules:**
  - `cpu/` and `clock.rs` hold no ESP32-C3 or v392 constant;
  - `FirmwareBus` is unchanged;
  - `emulator-core` takes no wasm-bindgen dependency;
  - `emulator-wasm` is untouched.
- **Input safety.** Unmapped data access never panics, and an unmapped instruction fetch always traps. A stub reading a wild pointer or a huge length must not panic: it reads through the bus and is capped at `MAX_STUB_MEMORY_BYTES`.
- **Servers.** Only `/api/signal` and its Upstash store. `iceServers: []`. No other third-party service.
- **Rulings** go in `docs/milestone-12-decisions.md` with their task's commit, as `Ruling R-T<n>-<k>: <decision> — cost if wrong: <...>`. The Milestone 3 to 11 decisions stand.
- **Data handling:**
  - never commit, quote or print anything from `local/full_flash_dump.bin`, `local/boot_log.txt`, `local/m6-badge/` or `local/m11-badge/`; give paths only;
  - committed tests use only `firmware.bin`, the eleven fake identities and `FAKE_UPLOAD_TOKEN`;
  - the fake MACs are `02:00:00:00:10:01` (hacker) and `02:00:00:00:10:02` (organizer).
- **Tools:**
  - use `/usr/bin/grep`;
  - use `/Users/powersagitar/Developer/powersagitar/hack-the-badge/local/.venv/bin/python -I` for any Python, never the system `python3`;
  - use Bun, never npm or node;
  - fetch external sources into the session scratchpad with one plain `curl -sfL -o <scratchpad>/<file> <url>` per file, never into the repo;
  - scratch probes (an `#[ignore]` test appended to `boot_progress.rs`) are removed before committing.
- **The full check list** runs before each commit that touches code, and everything must pass:
  ```
  cargo fmt --all && cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck && bun run build
  ```
  - Clippy and `bun run build` stay at 0 warnings.
  - The release `boot_progress` suite stays under 90 s, and its time is recorded in the ledger.
  - The baseline at `35a6c37`: release `boot_progress` 88 pass and 1 ignored, about 15 s; `cargo test --workspace` 751 pass and 1 ignored, about 23 s; `bun test` 319 pass.
- **Commits** end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` only. Pushing, merging and deploying are the human partner's call.

## Review Focus

1. **Provision on a badge that already has an identity** (from the launcher, for another role, or twice). The new identity gets its own token: a token set against the old identity is invalid by design. The provisioner must not match a prompt printed before `PROV OK`, even if `prov apply` reboots. Pinned by Task 3's re-provision probe and its `provisioner_sets_the_token_after_a_prov_ok_reboot` (or `…_without_a_reboot`, whichever the probe finds).
2. **The badge refuses the token, or the command is missing** (`BADGE_TOKEN FAIL`, `usage:`, or no `BADGE_TOKEN READY` at all). The provision fails with that line, or times out through `PROVISION_TIMEOUT_STEPS`. It never hangs in `typing`, and it never reports `ok`. Pinned by Task 3's `provisioner_fails_on_badge_token_fail` and `provisioner_fails_on_badge_token_usage`.
3. **`crc32_le` given a length of 0, a huge length, or an unmapped buffer.** A length of 0 returns the seed unchanged. A huge length is capped, and an unmapped buffer reads 0s. Neither panics. Pinned by Task 1's `crc32_le_stub_zero_length_returns_the_seed` and `crc32_le_stub_wild_buffer_does_not_panic`.
4. **A joiner whose signaling fetch hangs.** The joiner has no cancel, so the request aborts after the fetch timeout. The joiner then gets "Could not reach the signaling server", well inside its 120 s budget plus one timeout. Pinned by Task 5's `call_aborts_a_hung_fetch`.
5. **A host that fails and is then cancelled**, or is cancelled and then fails. Either way `close` goes out exactly once. Pinned by Task 5's `host_failure_then_cancel_sends_close_once`.

---

## File Structure

- `emulator-core/src/rom.rs`: `CRC32_LE`, its table entry, its module-doc entry, and the ROM-libc guard test (Task 1).
- `emulator-core/src/cpu/rom_stubs.rs`: `RomStubEffect::Crc32Le`, and `RomStub::crc32_le` (Task 1).
- `emulator-core/src/cpu/mod.rs`: the effect's arm (Task 1).
- `emulator-core/tests/common/provision.rs`: `FAKE_UPLOAD_TOKEN`, `set_upload_token` (Task 2).
- `emulator-core/tests/boot_progress.rs`: the credential checkpoint and the Sync rungs (Task 2).
- `frontend/src/runtime/provisioner.ts`, `frontend/test/provisioner.test.ts`: the token phases and the skip option (Task 3).
- `frontend/src/bench/redraw-bench.ts`: skips the token (Task 3).
- `frontend/test/test-identities.test.ts`: the two token copies match (Task 3).
- `frontend/tsconfig.json`, `frontend/test/*.test.ts`: type-checked tests (Task 4).
- `frontend/src/radio/signal-client.ts`, `frontend/api/signal.ts`, `frontend/test/signal-client.test.ts`, `frontend/test/signal-handler.test.ts`: the signaling fixes (Task 5).
- `docs/milestone-12-decisions.md`: created in Task 1 and grown by every task. `CLAUDE.md` and `docs/firmware-emulator-notes.md`: Task 7.

---

### Task 1: The decisions doc, `BASE`, and the `crc32_le` stub

**Files:**
- Create: `docs/milestone-12-decisions.md`
- Modify: `emulator-core/src/rom.rs` (constants near `MEMCMP` at about line 1532; table near line 2468; guard test near line 2907; module doc)
- Modify: `emulator-core/src/cpu/rom_stubs.rs` (`RomStubEffect` near `Memcmp` at about line 271; constructor near line 1541)
- Modify: `emulator-core/src/cpu/mod.rs` (effect arms near `Memcmp` at about line 764)

**Interfaces:**
- Produces: `crate::rom::CRC32_LE: u32 = 0x4000_0628`; `RomStubEffect::Crc32Le`; `RomStub::crc32_le(name: &'static str) -> RomStub`.

- [ ] **Step 1: Create `docs/milestone-12-decisions.md`.**
  - It gets the sections "Why", "Measurements", "Traces", "Dependencies added", "Rulings", "Final suites" and "Milestone 13 backlog" (empty for now).
  - Record `BASE` = `git rev-parse HEAD` in the ledger and the doc.
  - Copy the spec's "Measured" probe numbers into "Measurements", marked as from the spec.

- [ ] **Step 2: Fetch the two ESP-IDF sources** into the session scratchpad:
  - `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/components/esp_rom/esp32c3/ld/esp32c3.rom.ld`
  - `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/components/esp_rom/include/esp_rom_crc.h`

  Confirm line 415 reads `crc32_le = 0x40000628;`. Note the header's line numbers for the semantics, to cite them.

- [ ] **Step 3: Write the failing tests** in `rom.rs`'s test module, following the other effect tests there (for example `memcmp_stub_returns_the_first_differing_byte_difference`):
  - `crc32_le_stub_matches_the_check_value`: seed 0 over `b"123456789"` in RAM → `a0 == 0xCBF4_3926`; `pc == ra`.
  - `crc32_le_stub_zero_length_returns_the_seed`: seed `0xDEAD_BEEF`, len 0 → `0xDEAD_BEEF`.
  - `crc32_le_stub_chains`: `crc(crc(0, b"1234"), b"56789") == 0xCBF4_3926`.
  - `crc32_le_stub_wild_buffer_does_not_panic`: buffer `0x1000_0000` (unmapped), len `u32::MAX` → returns some value, no panic, and runs in bounded time (capped at `MAX_STUB_MEMORY_BYTES`).
  - In `no_rom_libc_function_carries_a_generic_zero_return_stub`'s list: `(CRC32_LE, RomStubEffect::Crc32Le)`.

- [ ] **Step 4: Run the tests and see them fail.**
  Run: `cargo test -p emulator-core crc32_le`
  Expected: compile errors (`CRC32_LE` and `Crc32Le` are not defined).

- [ ] **Step 5: Implement the stub.**
  - `RomStubEffect::Crc32Le` takes `a0` = crc, `a1` = buf and `a2` = len, capped at `MAX_STUB_MEMORY_BYTES`. It reads each byte with `bus.read8` and returns `!crc'` in `a0`, where `crc'` is the reflected update with `0xEDB8_8320` starting from `!a0`. Its doc comment cites `esp_rom_crc.h`.
  - Add `RomStub::crc32_le`.
  - In `rom.rs`, add `pub const CRC32_LE`, with a doc comment citing `esp32c3.rom.ld:415` and the header, and the table entry `(CRC32_LE, RomStub::crc32_le("crc32_le"))`.
  - Add a module-doc entry: first reached when NVS writes the upload credential (`badge_token set`), and again in Sync's snapshot CRC; the spec's probe 1.
  - Use no new dependency.

- [ ] **Step 6: Run the tests and see them pass.**
  Run: `cargo test -p emulator-core crc32_le`
  Expected: PASS (5 tests, counting the guard test).

- [ ] **Step 7: Run the full check list.** Every pin must be unchanged. Nothing reached `0x4000_0628` before (spec, probe 1).

- [ ] **Step 8: Commit.** Add the decisions doc and the three source files, with the message `feat(core): ROM crc32_le stub`.

### Task 2: The fake credential in the Rust helper, and the Sync rungs (Gate 3's frames)

**Files:**
- Modify: `emulator-core/tests/common/provision.rs`
- Modify: `emulator-core/tests/boot_progress.rs` (new code after `sync_now_without_a_credential_is_not_ready`, about line 2558)

**Interfaces:**
- Consumes: Task 1's stub; `boot_progress.rs`'s `first_run_screen`, `provisioned_from`'s body pattern, `walk_to_shake_page_from`, `walk_to_registered_from`, `walk_to_launcher_from`, `press_a_on_cell_from`, `run_until_stable_frame`, `console_command`, `assert_radio_clean`, `assert_no_panic_text`, `HACKER_MAC`, `SYNC_HOME_HASH`, `A_SLOT`.
- Produces:
  - `provision::FAKE_UPLOAD_TOKEN: &str`;
  - `provision::set_upload_token(rt: &mut FirmwareRuntime, token: &str, deadline: u64) -> Result<String, String>`;
  - in `boot_progress.rs`: `fn credential_launcher() -> FirmwareRuntime` (a `OnceLock` checkpoint) and `fn sync_open_with_credential() -> FirmwareRuntime`.

- [ ] **Step 1: Add `FAKE_UPLOAD_TOKEN` and `set_upload_token`** to `provision.rs`.
  - `FAKE_UPLOAD_TOKEN`'s doc says it is obviously fake, committed with the organizers' permission (2026-11-10), and mirrored in `provisioner.ts`.
  - `set_upload_token` runs only at an idle prompt (the caller has just seen one). It types `badge_token set` and the line end, then waits for `BADGE_TOKEN READY`. A `usage:` or `BADGE_TOKEN FAIL` line is an `Err`.
  - It then sends the 64 token bytes in one `serial_input` and waits for `BADGE_TOKEN OK` (`BADGE_TOKEN FAIL` is an `Err`).
  - Last, it waits for the prompt after that line, the way `provision` waits for the prompt after `PROV OK`. It returns the console text from the command on.

- [ ] **Step 2: Write the credential checkpoint and its rung** in `boot_progress.rs`.
  - `credential_launcher()`: from `first_run_screen()`, set `HACKER_MAC`, run `provision` and assert `PROV OK id=test-`, then `set_upload_token(FAKE_UPLOAD_TOKEN)`. Then onboarding page 1 must be stable on hacker's hash, as `provisioned_from` asserts. Then walk with the three `walk_*_from` helpers, which assert every frame.
  - Assert `CREDENTIAL_LAUNCHER_STEPS` and `CREDENTIAL_LAUNCHER_US`, plus `radio_init_count() == 0`. Pin them at first as `0` and read the real values from the failure. The spec's probe measured 342,894,540 and 94,521,622.
  - Test `launcher_with_a_credential_keeps_every_walk_frame` calls it.
  - Its doc states that the onboarding, registered and launcher frames are the same hashes as `launcher_with_mac`'s.

- [ ] **Step 3: Write the Sync rungs.**
  - `sync_open_with_credential()` is a `OnceLock` checkpoint. From `credential_launcher()`, A on cell 3 settles on `SYNC_HOME_HASH`, with the same console order and `assert_radio_clean` as `sync_open()`, and `soft_reset_count() == 1`.
  - Test `sync_with_a_credential_finds_no_station`, from `sync_open_with_credential()`:
    - Before A, `console_command(&mut rt, "badge_token get")` contains `BADGE_TOKEN ` and not `MISSING`. This shows the credential survives Sync's reboot. Do not print or assert the token text in a failure message beyond what `assert!` shows.
    - After `press(A_SLOT)`, within `SYNC_PACKING_STEPS`: the console shows `sync: snapshot 1026 bytes, crc ec2f90df,` and `complete=1`. Pin the exact line prefix as measured, re-measured by the implementer.
    - The firmware's "Asking the station..." state: `uitree` (through `console_command`) contains `text="Asking the station..."` and `text="B: cancel"`.
    - Then `run_until_stable_frame` with a stable window above the border's pulse period (the probe used 30,000,000 steps), capped at `SYNC_NO_STATION_STEPS`, gives `SYNC_NO_STATION_HASH`. The probes measured `0xa465_e4c0_e16e_ede6` at +415,049,485 steps.
    - `uitree` then contains `text="No sync station in range"`, `text="Move closer to the sync desk"` and `text="A: try again   -   HOME: home"`, and does not contain "Badge not ready".
    - `assert_radio_clean(&rt, before)` and `assert_no_panic_text` pass, with `radio_init_count() == 1` and `soft_reset_count() == 1`.
  - Budgets follow the suite's convention (measured, then +25 %, rounded up to 1,000,000). Each constant's doc gives its measurement.
  - The packing frame: pin it by hash if it is stable for `AFTER_PRESS_STABLE_FOR`. Otherwise use the console line alone. Record the choice as a ruling.
  - Typing `uitree` mid-wait shifts timing. Measure all pins with the rung exactly as committed.

- [ ] **Step 4: Run the new rungs and pin the measured values.**
  Run: `cargo test -p emulator-core --release --test boot_progress credential -- --nocapture` and then `… sync_with_a_credential`.
  Expected: PASS after pinning. Record each measured value and the suite's new time in the decisions doc's "Measurements".

- [ ] **Step 5: Save the frames for Gate 3.** Write a scratch (uncommitted) ignored test, or use `examples/boot-probe.rs`'s PNG writer, to save the packing frame, one "Asking the station..." frame and the no-station frame. Save them as `local/m12-frames/sync-{packing,asking,no-station}.png`. Delete the scratch test.

- [ ] **Step 6: Run the full check list.** The release `boot_progress` count is 88 + the new rungs; it must stay under 90 s.

- [ ] **Step 7: Commit** `provision.rs`, `boot_progress.rs` and the decisions doc, with the message `test: Sync with a fake upload credential reaches No sync station in range`.

### Task 3: The browser provisioner sets the token

**Files:**
- Modify: `frontend/src/runtime/provisioner.ts`
- Modify: `frontend/src/bench/redraw-bench.ts:249`
- Test: `frontend/test/provisioner.test.ts`, `frontend/test/test-identities.test.ts`, `frontend/test/cpu-wasm.test.ts` (unchanged, but must pass)

**Interfaces:**
- Consumes: the firmware strings in Global Constraints; Task 2's `FAKE_UPLOAD_TOKEN` text in `provision.rs`.
- Produces:
  - `export const FAKE_UPLOAD_TOKEN: string`;
  - `export const BADGE_TOKEN_READY = "BADGE_TOKEN READY"`, `BADGE_TOKEN_OK = "BADGE_TOKEN OK"`, `BADGE_TOKEN_FAIL = "BADGE_TOKEN FAIL"`;
  - `createProvisioner(handle, identityJson, bootConsoleStart = () => 0, options: ProvisionOptions = {})`, with `export interface ProvisionOptions { uploadToken?: string | null }`. Leaving out `uploadToken` means `FAKE_UPLOAD_TOKEN`; `null` skips the token.

- [ ] **Step 1: Probe re-provisioning** with a scratch ignored test in `boot_progress.rs`, removed afterwards.
  - From a clone of `credential_launcher()`, `provision` role `organizer`. Does `prov apply` reboot (`soft_reset_count()`)? Where is the prompt after `PROV OK`, relative to `boot_console_start` (the runtime's marker)? Does `set_upload_token` then succeed?
  - Also check `badge_token get` for the new identity before setting it. The spec expects `MISSING`, since the old credential's `badge_id` differs.
  - Record the findings in the decisions doc's "Traces".

- [ ] **Step 2: Write the failing tests** in `provisioner.test.ts`, using its fake handle. The fake handle must answer `badge_token set` the way the firmware does.
  - `provisioner_sets_the_token_after_prov_ok`: after `PROV OK …` and a prompt, the input queue receives `badge_token set\n`. After `BADGE_TOKEN READY`, it receives exactly the 64 token bytes. The result is `{ kind: "ok", line: "PROV OK …" }` only after `BADGE_TOKEN OK` and a prompt.
  - `provisioner_fails_on_badge_token_fail`: the result is `{ kind: "failed", reason: "BADGE_TOKEN FAIL" }`.
  - `provisioner_fails_on_badge_token_usage`: `usage: badge_token <set|get>` fails the provision with that line.
  - `provisioner_skips_the_token_when_asked`: with `{ uploadToken: null }`, `ok` comes right after `PROV OK`, as today, and `badge_token` is never typed.
  - The Step 1 case: `provisioner_sets_the_token_after_a_prov_ok_reboot` if `prov apply` reboots, else `provisioner_sets_the_token_after_prov_ok_without_a_reboot`. It covers a stale prompt from before `PROV OK` (or from the old boot) that must not start the token phase.
  - `provisioner_never_types_a_token_into_a_prompt_before_prov_ok`: a `badge> ` before `PROV OK` in the text after `prov apply` must not count.
  - In `test-identities.test.ts`, `fake_upload_token_matches_the_rust_helper`: read `emulator-core/tests/common/provision.rs` as text, extract the `FAKE_UPLOAD_TOKEN` literal, and compare. Also check `/^[0-9a-f]{64}$/` and that no identity JSON contains the token.

- [ ] **Step 3: Run the tests and see them fail.**
  Run: `bun test test/provisioner.test.ts test/test-identities.test.ts`
  Expected: FAIL (no token phase; `FAKE_UPLOAD_TOKEN` is not exported).

- [ ] **Step 4: Implement the token phases.**
  - The new phases go after `applyReply`: `tokenPrompt` → `tokenReady` → `tokenReply` → `done`.
  - The `PROV OK` line is kept as the eventual `ok` line.
  - The prompt the token waits for is one printed after the `PROV OK` line, in the current boot (`bootConsoleStart()`). Use whichever offset is later.
  - Update the module doc: the flow, the skip option, and why the benchmark skips.
  - `redraw-bench.ts` passes `{ uploadToken: null }`, with a comment: its rows stay like-for-like with Milestones 7, 9 and 11.

- [ ] **Step 5: Run the tests and see them pass.**
  Run: `bun run build:wasm && bun test`
  Expected: all pass, including `cpu-wasm.test.ts`, whose frame pins hold with the token (spec, probe 3). If a twin frame pin fails, stop: that is a gate, never a re-pin.

- [ ] **Step 6: Run the full check list**, then commit `provisioner.ts`, `redraw-bench.ts`, the two test files and the decisions doc, with the message `feat: the browser provisioner sets a fake upload credential`.

- [ ] **Step 7 (orchestrator, not the implementer): Check it in the browser, then Gate 3.**
  - Run `bun run dev`. In Chrome (chrome-devtools-mcp), choose firmware mode, Provision hacker, walk onboarding to the launcher, open Sync, press A, and wait about 25 s.
  - Screenshot "No sync station in range" to `local/m12-browser/sync-no-station.png`.
  - Re-provision as organizer from the launcher, and check that Sync reaches the same screen.
  - Stop the dev server.
  - **Gate 3:** send the human partner the three PNGs from Task 2 Step 5, the browser screenshot, the suite times, the re-provision finding, and anything else open, in one message.

### Task 4: Type-check every test file

**Files:**
- Modify: `frontend/tsconfig.json` (`"include": ["src", "api", "test"]`)
- Modify: the test files with errors: `firmware-worker.test.ts` (29), `firmware-runtime.test.ts` (19), `shell.test.ts` (3), and one each in `manifest.test.ts`, `provisioner.test.ts`, `radio-lock.test.ts`, `redraw-bench.test.ts`, `signal-client.test.ts` and `signal-handler.test.ts`, plus `vite.config.ts` (1). These counts are from before Task 3, so re-count.

**Interfaces:** none produced.

- [ ] **Step 1: Widen the include** and run `bun run typecheck`. Expected: about 58 errors. Record the per-file count in the ledger.
- [ ] **Step 2: Fix each error** without loosening anything:
  - no change to `strict`;
  - no file-level `any`, `@ts-nocheck` or `@ts-ignore`;
  - a narrow, commented `as` cast is acceptable for a test fake.
  - `redraw-bench.test.ts`'s fake handle gains `softResetCount` and `rebootUs`.
  - A `lib` bump to `ES2022` (for `.at()`) is allowed only if `bun run build`'s output files keep their byte sizes. Otherwise rewrite the call. Record which as a ruling.
- [ ] **Step 3: Run `bun run typecheck`.** Expected: 0 errors. Then run the full check list; `bun test`'s count is unchanged from Task 3's.
- [ ] **Step 4: Commit** `tsconfig.json` and the test files, with the message `chore: type-check every frontend test file`.

### Task 5: The three signaling fixes (Gate 4)

**Files:**
- Modify: `frontend/src/radio/signal-client.ts` (`call` at about line 92, `resolveDeps` at about line 82, the host's `connected` at about lines 230–265)
- Modify: `frontend/api/signal.ts:115` (`REDIS_OPTIONS`)
- Test: `frontend/test/signal-client.test.ts`, `frontend/test/signal-handler.test.ts`

**Interfaces:**
- Produces:
  - `export const SIGNAL_FETCH_TIMEOUT_MS = 10_000`;
  - `SignalDeps.fetchTimeoutMs?: number`, defaulting to `SIGNAL_FETCH_TIMEOUT_MS` (tests inject a small value);
  - `REDIS_OPTIONS.retry === false`.

- [ ] **Step 1: Write the failing tests.**
  - `call_aborts_a_hung_fetch`: a fake `fetch` that settles only when its `init.signal` aborts. With `fetchTimeoutMs: 20`, joining a room rejects with "Could not reach the signaling server; check the connection.", and the fetch's signal was aborted.
  - `host_cancel_still_aborts_an_in_flight_fetch`: the host's cancel aborts a hung poll. The request's signal combines the timeout with the cancel.
  - `host_gives_up_and_sends_close`: no joiner by the deadline (fake `now` and `sleep`). `connected` rejects with "No one joined room …", and exactly one `close` POST with the room's token is sent.
  - `host_sends_close_on_a_server_error`: a poll answers 500, `connected` rejects, and one `close` is sent.
  - `host_failure_then_cancel_sends_close_once` and `host_cancel_then_failure_sends_close_once`: both orders send exactly one `close`.
  - `host_success_sends_no_close`: today's behaviour holds.
  - In `signal-handler.test.ts`, `redis_options_disable_retries`: `REDIS_OPTIONS.retry` is `false`.

- [ ] **Step 2: Run the tests and see them fail.**
  Run: `bun test test/signal-client.test.ts test/signal-handler.test.ts`
  Expected: the new tests fail.

- [ ] **Step 3: Implement the three fixes.**
  - `call` takes an optional `signal`. It aborts on that signal or after `d.fetchTimeoutMs`, whichever comes first, and maps any abort or throw to the existing `SignalError`. If `AbortSignal.any` or `AbortSignal.timeout` are missing from the types, use a hand-made `AbortController` and timer. Clear the timer once the fetch settles.
  - The host's polls pass `cancel.signal`.
  - A `closeSent` flag, shared by `cancel()` and `connected`'s catch, ensures exactly one `close`.
  - In `REDIS_OPTIONS`, add `retry: false`, with a comment: a retried `EVALSHA` after a lost reply would count one limiter hit twice; a store error stays the bare 500.

- [ ] **Step 4: Run the tests and see them pass**, then run the full check list.

- [ ] **Step 5: Commit** `signal-client.ts`, `api/signal.ts`, the two test files and the decisions doc, with the message `fix(signal): fetch timeout, no Upstash retries, close when the host gives up`.

- [ ] **Step 6 (orchestrator): Gate 4.** Ask the human partner for a Vercel preview deploy of the branch, then for two checks there: a pairing, and a host left to time out. On the second, the room is gone: a join with its code gets "Room closed or expired." at once.

### Task 6: Share's trace, and Sync's 20 s (recorded, no code)

**Files:**
- Modify: `docs/milestone-12-decisions.md` ("Traces")

**Interfaces:** none.

- [ ] **Step 1: Fetch the public source** into the session scratchpad if it is not already there: `https://codeload.github.com/hackthenorth/badge-firmware/tar.gz/refs/heads/main`, unpacked into its own empty directory. Read `main/apps/share_xfer.{h,cpp}`, `main/apps/app_share.cpp`, `main/apps/app_share_proto.h` and `main/hal/hal_radio.cpp`.
- [ ] **Step 2: Check them against v392.**
  - Look for Share's strings in `firmware.bin`.
  - Disassemble the call sites, with `riscv64-unknown-elf-objdump` if present, otherwise a `local/.venv` tool such as `capstone` (any install goes into the venv; name it).
  - Find which HCI commands Share's path sends. The scratch boot-probe and the model's command log may help.
  - The answer: does v392's Share use connections (`LE Create Connection`, GATT), or advertisements only?
- [ ] **Step 3: Run a scratch two-badge probe.**
  - Use Connect's two-runtime pattern in an ignored test, never committed: hacker and organizer launchers with MACs, both open Share, with one sending and one listening.
  - Record the last screen on each side (from `uitree`) and the first thing that is missing.
- [ ] **Step 4: Settle Sync's 20 s against the source's 15 s.**
  - From v392's disassembly of `bbx_xfer_tick`'s OPEN branch, find the constants compared against `s_ticks` and `s_open_retries`.
  - If they explain 20 s, record that. If they say 15 s, the emulated tick is slow: stop at a gate, since a fix would move pins.
- [ ] **Step 5: Write "Traces".**
  - For Share: transport, frame types and sizes against the model's 251 B out and 229 B in, how far two badges get, what is missing, and a size for Milestone 13.
  - For Sync: the timeout's constants.
  - Commit the decisions doc alone, with the message `docs: Milestone 12 traces (Share, Sync's timeout)`.

### Task 7: Docs, the final review, and the finish (Gate 5)

**Files:**
- Modify: `CLAUDE.md`, `docs/firmware-emulator-notes.md`, `docs/milestone-12-decisions.md`

- [ ] **Step 1: Update `CLAUDE.md`.**
  - **Current state:** Sync ends on "No sync station in range" with the provisioned fake credential.
  - **Test counts and times.**
  - **The architecture map:** `rom.rs`'s table gains `crc32_le`, and `runtime/provisioner.ts` sets the token, with the skip option for the benchmark.
  - **"Local data":** the only committed identity data is the eleven fake identities and the fake upload token `FAKE_UPLOAD_TOKEN`, committed with the organizers' permission (2026-11-10).
- [ ] **Step 2: Update `docs/firmware-emulator-notes.md`.** Add a Milestone 12 section (the stub, the credential, Sync's path and its timings). Update "Known limitations": Sync stops at no station, and there is no fake station.
- [ ] **Step 3: Finish the decisions doc.**
  - Fill "Final suites" (counts and times for all five).
  - Under "Dependencies added", write "none" or each one, with version and size cost.
  - Fill "Milestone 13 backlog": every item of `docs/milestone-11-decisions.md`'s "Milestone 12 backlog" that this milestone did not close, carried word for word or condensed. Add the fake sync station (the protocol is advertisement-only, so it is possible) and Share's transfer, sized by Task 6.
- [ ] **Step 4: Run the full check list,** then commit the three docs with the message `docs: Milestone 12 notes, CLAUDE.md and decisions`.
- [ ] **Step 5 (orchestrator): the whole-branch review.**
  - An opus reviewer checks `BASE..HEAD` against the spec and this plan, and a sonnet reviewer checks the docs.
  - Fix rounds get scoped re-reviews.
  - Then finish with superpowers:finishing-a-development-branch. **Gate 5:** merge, push and deploy are the human partner's call. List every `Ruling:` line under "Rulings I made", each with its cost if wrong.

---

## Orchestrator notes

- **Execution method:** chosen by the human partner at Gate 2. The convention since Milestone 6 is subagent-driven: opus implementers and reviewers, sonnet scoped re-reviews of fix rounds and docs reviews, and an opus whole-branch review.
- **Ledger:** `local/m12-sdd-ledger.md` in the worktree (it exists). Write one line per dispatch, verdict, fix round, ruling, measurement, dependency and human answer. Keep `local/m12-sdd-resume-prompt.md` current whenever a subagent reports.
- **Order:** 1 → 2 → 3 (Gate 3 at Step 7) → 4 → 5 (Gate 4 at Step 6) → 6 → 7 (Gate 5). Run the tasks one at a time, since each one's check list runs the whole suite. Task 4 comes after Task 3, so the new provisioner tests are type-checked, and before Task 5, so the signaling tests are.
- **Gates (stop and ask; ask everything open in one message):** Task 3 Step 7, Task 5 Step 6, Task 6 Step 4 (if the tick is slow), and Task 7 Step 5. Also stop for:
  - a Milestone 11 pin moving;
  - a WASM-twin frame failing;
  - any hardware-compared frame moving;
  - the release `boot_progress` over 90 s;
  - any override of a recorded decision;
  - any deploy, push or merge;
  - anything needing the badge.
- **Implementer brief:** this plan's Global Constraints and Review Focus, the full task text, the Interfaces it consumes, the spec path, the relevant ledger lines, and the worktree path `/Users/powersagitar/Developer/powersagitar/hack-the-badge/.claude/worktrees/milestone-12`.
  - For anything under `local/`, give paths only.
  - Tell every subagent to use `local/.venv/bin/python -I` and `/usr/bin/grep`.
- **Browser work:** Chrome through chrome-devtools-mcp is the orchestrator's alone. Implementers stop at "full check list green" and report.
- **Reviewer checks (on top of the skill's):**
  - no chip constant in `cpu/` or `clock.rs` (the CRC polynomial is generic; the address is in `rom.rs`);
  - `emulator-wasm` untouched;
  - no Milestone 11 rung, constant or helper behaviour changed;
  - the token only as `FAKE_UPLOAD_TOKEN`, never in identity JSON or user-visible text;
  - `iceServers: []`, and no server URL other than `/api/signal`;
  - no `local/` data, PNGs or scratch probes committed (`git show --stat`).
- **Finish:** copy everything new under the worktree's `local/` to the repo-root `local/` before removing the worktree.

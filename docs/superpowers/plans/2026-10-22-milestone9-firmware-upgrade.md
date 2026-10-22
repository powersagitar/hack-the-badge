# Milestone 9 — v392 Firmware Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The emulator runs the official v392 badge firmware as `firmware.bin`, every rung re-pinned with its reason, and the five apps that reboot into themselves (Scanner, Sync, Share, Sponsors, Workshop Quest) open through a modeled software reset.

**Architecture:** Task 1 renames "factory" to "firmware" with v335 still inside (no pin moves). Task 2 swaps in v392 and re-pins (no `emulator-core/src` change). Task 3 adds a soft reset: a chip-agnostic ROM-stub effect requests it, and `FirmwareRuntime` re-runs the shortcut boot while carrying over flash, RTC retention, the framebuffer, the console log, host inputs and host time. Task 4 pins every launcher app. Tasks 5 and 6 are the human's browser check and the docs.

**Tech Stack:** Rust (`emulator-core`, `emulator-wasm`), wasm-pack, Bun/TypeScript (`frontend/`).

**Spec:** `docs/superpowers/specs/2026-10-22-milestone9-firmware-upgrade-design.md`

## Global Constraints

- The official image is `local/flashes/firmware.bin` (2,651,840 bytes, `v0.1.2-392-gd3089c4`). It is committed **unpadded** as `frontend/public/firmware/firmware.bin`. `factory.bin` is removed. `local/flashes/` itself is never committed.
- "factory" is renamed to "firmware" only where it names the file or the image loaded from it. The flash partition keeps its firmware-defined name `factory`: `flash.rs`'s partition label, `ESP_PARTITION_SUBTYPE_APP_FACTORY`, and docs about the partition. Historical docs (`docs/milestone-*-decisions.md`, the notes' history sections, earlier specs and plans) keep their wording. `frontend/src/badge/ui.ts`'s widget `factory` helper is unrelated and stays.
- **Re-pins happen only in Task 2.** Each changed value is recorded in `docs/milestone-9-decisions.md` § "Re-pins (v335 → v392)" with the rung, the old value, the new value and the reason. After Task 2's commit, the equivalence rule of Milestone 7 applies again: every step count, console needle, emulated time and frame hash stays byte-identical through Tasks 3 to 6, and a pin that moves is a bug. `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged` guard this. The exception is a pin that Task 3 changes on purpose, for an app that now reboots instead of crashing. Each such case is recorded as a ruling with its reason.
- Rung docs say which frames were compared with the physical badge **on v335**. A v392 frame is never described as compared with hardware; v392 is checked by the human partner's eye (Task 5).
- `emulator-core/src/clock.rs` and `emulator-core/src/cpu/` stay chip-agnostic. ESP32-C3 values (ROM addresses, reset-reason codes, register offsets) live in `rom.rs`, `mem/soc.rs` and `peripherals/`.
- `FirmwareBus` keeps ordered, concrete named fields with no `dyn`. The word fast path keeps its order. `emulator-core` has no wasm-bindgen dependency. `emulator-wasm` stays one-line passthroughs.
- Unmapped data access never panics; an unmapped instruction fetch always traps. Browser input never panics the emulator.
- Register layouts come from ESP-IDF **v5.5.3** headers and ROM addresses from its linker scripts. Cite them in the module doc and never guess. Fetch them into the session scratchpad (`gh api "repos/espressif/esp-idf/contents/<path>?ref=v5.5.3" --jq .content | base64 -d`), never into the repo.
- Milestones 3 to 8 decisions stand. Milestone 9 rulings go in `docs/milestone-9-decisions.md` with their task's commit.
- Data handling:
  - never commit, quote or print anything from `local/full_flash_dump.bin`, `local/boot_log.txt` or `local/m6-badge/`; give paths only;
  - the dump stays v335, so code reading `BADGE_FULL_DUMP` must not assume it matches `firmware.bin`;
  - committed tests depend only on `firmware.bin` and the committed fake identities.
- Tools:
  - use `/usr/bin/grep`, since the shell's `grep` may silently return nothing;
  - use `local/.venv/bin/python` for any Python;
  - use Bun, never npm or node.
- The full check list after any change under `emulator-core/` or `emulator-wasm/`, before each commit:
  ```
  cargo fmt --all && cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck && bun run build
  ```
  All green. Clippy stays at its 3 known warnings. The release `boot_progress` suite stays under 90 s.
- Commits end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Mi88bZyQaC9Zsf56wSHJYT
  ```
  Pushing and merging are the human partner's call.

## Review Focus

1. **A button held across a reboot stays held, and nothing replays.** The emulated GPIO and the 74HC165 show the held state from the first step after the reset. A press released before the reset is not seen again. Pinned by Task 3's `held_button_survives_a_soft_reset`.
2. **Host-visible time never goes backwards across a reset.** `emulated_time_us()` keeps counting while the new SYSTIMER starts at 0, and a `run_until` whose target spans a reset still stops at the target. Otherwise the browser's wall-clock pacing would run away. Pinned by Task 3's `emulated_time_is_monotonic_across_a_soft_reset`.
3. **The console log survives the reset.** The browser provisioner and every rung read `console_output()`, and losing the pre-reset text would hide `reboot into <app>`. Pinned by Task 3's `console_output_survives_a_soft_reset`.
4. **Opening an app reboots exactly once, and leaving it reboots back once.** There is no reboot storm, and the identity and `reboot.cfg` survive. Pinned by Task 3's `scanner_opens_after_one_reboot` and `scanner_home_reboots_to_the_launcher`, which assert `soft_reset_count()`.
5. **The host's own reset button is still a cold power-on after soft resets.** It brings back blank flash and `POWERON_RESET`, with a reset count of 0. Pinned by Task 3's `cold_reset_after_a_soft_reset_is_a_power_on`.

---

## File Structure

- `frontend/public/firmware/firmware.bin`: new (v335 in Task 1, v392 from Task 2). `factory.bin` is deleted.
- `emulator-core/src/boot.rs`: rename `boot_from_factory_image*` to `boot_from_firmware_image*` (Task 1).
- `emulator-core/src/cpu/rom_stubs.rs` and `cpu/mod.rs`: `RomStubEffect::RequestReset(u32)` and `Cpu::take_reset_request` (Task 3).
- `emulator-core/src/rom.rs`:
  - the `software_reset` and `software_reset_cpu` stubs;
  - the reset-reason constants;
  - `esp32c3_rom_stubs_with_reset_reason` (Task 3).
- `emulator-core/src/peripherals/rtc_cntl.rs`: `STORE0..7` as retained R/W registers (Task 3).
- `emulator-core/src/runtime.rs`: the soft reset and `soft_reset_count`. Host time becomes an offset plus the clock (Task 3).
- `emulator-core/tests/boot_progress.rs`: re-pins (Task 2), reboot rungs (Task 3), all-apps rungs (Task 4).
- `emulator-core/tests/boot_integration.rs`, `rom_stub_boot.rs`, `examples/boot-probe.rs`: rename (Task 1), re-pin (Task 2).
- `emulator-wasm/src/lib.rs`: doc and error text (Task 1).
- Frontend loaders: `frontend/src/main.ts`, `frontend/src/cpu/bridge.ts`, `frontend/src/runtime/firmware-worker*.ts`, `frontend/src/bench/bench-worker.ts`, `frontend/bench/bun-bench.ts`, `frontend/test/cpu-wasm.test.ts` (Task 1).
- `frontend/src/bench/walk.ts`: the shared walk constants, re-pinned in Task 2.
- `docs/milestone-9-decisions.md`: new (Task 1), grown by every task.
- `docs/firmware-emulator-notes.md`, `CLAUDE.md`: Task 1 (rename) and Task 6.

---

### Task 1: Rename "factory" to "firmware" (v335 still inside; no pin moves)

**Files:**
- Rename: `frontend/public/firmware/factory.bin` → `frontend/public/firmware/firmware.bin` (`git mv`; same bytes)
- Modify: every file in the File Structure marked "Task 1"
- Create: `docs/milestone-9-decisions.md`

**Interfaces:**
- Produces:
  - `emulator_core::boot::boot_from_firmware_image(image: &[u8]) -> Result<(Cpu, FirmwareBus), ImageParseError>`;
  - `emulator_core::boot::boot_from_firmware_image_with_rom_stubs(image: &[u8]) -> Result<(Cpu, FirmwareBus), ImageParseError>`;
  - the tests' helper `fn firmware() -> Vec<u8>` in `boot_progress.rs`; `firmware_bin_path()` and `read_firmware_bin()` in the other two test files;
  - the frontend URL `"/firmware/firmware.bin"`.

- [ ] **Step 1: `git mv` the file, then rename every file reference.**

  Rename every `factory.bin` path, `factory()`, `factory_bin_path()`, `read_factory_bin()`, `boot_from_factory_image*`, the test names containing `factory_image` (for example `boots_real_factory_image_…` becomes `boots_real_firmware_image_…`), and the doc comments that mean the file or image. Leave alone everything the Global Constraints exclude.
- [ ] **Step 2: Verify nothing is left over.**

  Run: `git grep -n -i factory -- ':!docs/milestone-*' ':!docs/superpowers' ':!docs/firmware-emulator-notes.md'`

  Expected: only partition uses (`flash.rs` label and subtype, `rom.rs:3667`'s "nvs, phy_init, factory, storage") and `frontend/src/badge/ui.ts`.

  In `docs/firmware-emulator-notes.md` and `CLAUDE.md`, update only the current-state text and the architecture map (`firmware/  firmware.bin (official v0.1.2-335 at this commit; public)`). History stays.
- [ ] **Step 3: Create `docs/milestone-9-decisions.md`.**

  It gets sections "Why", "Measurements", "Re-pins (v335 → v392)" (empty table: rung | old | new | reason), "Rulings" and "Milestone 10 backlog". Copy the spec's measured facts into "Measurements".
- [ ] **Step 4: Run the full check list.**

  Expected: all green with **the same counts as at `6a27f5f`**: release `boot_progress` 64 pass 1 ignored, workspace 644, bun 182. No hash or step number edited: `git diff 6a27f5f -- emulator-core/tests frontend/test frontend/src/bench/walk.ts | /usr/bin/grep '^[-+].*0x'` prints nothing.
- [ ] **Step 5: Commit** `refactor: rename factory.bin to firmware.bin`.

### Task 2: Swap in v392 and re-pin (no `emulator-core/src` change)

**Files:**
- Replace: `frontend/public/firmware/firmware.bin` with `local/flashes/firmware.bin` (bytes as-is, unpadded)
- Modify:
  - `emulator-core/tests/boot_progress.rs`, `boot_integration.rs`, `rom_stub_boot.rs`;
  - `frontend/src/bench/walk.ts`, `frontend/test/cpu-wasm.test.ts`;
  - `docs/milestone-9-decisions.md`.

**Interfaces:**
- Consumes: Task 1's names.
- Produces: the re-pinned base commit. Every later task's equivalence is measured against it, so record its hash in the ledger as `REPIN_BASE`.

The failures to expect, measured with v392 swapped in at `6a27f5f`:
- `boot_integration`: `boots_real_…_image_without_panicking_and_reaches_step_budget`, because the size assert is `2_752_512` and is now `2_651_840`.
- `rom_stub_boot`: `boot_stubs_reach_load_partitions_md5init_after_drawing_the_splash`.
- `boot_progress`: 35 rungs. Three groups:
  - the splash/first-run checkpoint, whose skip list holds the v335 splash, and every walk behind it;
  - the v335 code-address rungs: `first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line`, `boot_escapes_the_pre_task_9_spi_update_poll_into_spi_clock_setup`, `boot_no_longer_faults_at_the_pre_task_d10_…`, `…_d12_bswapsi2_call_site`, `load_partitions_accepts_the_synthesized_table_through_the_flash_mmu`;
  - the two equivalence rungs.
- The trial walk (`local/m9-probe.patch`, `m9_unpinned_walk`) found these frames unchanged from v335: the first-run screen, onboarding page 2, pages 4 to 11 and the shaken page, and hacker's registered screen. The splash, page 1, page 3's button frames and the launcher changed. On page 3, AUX1's press left the frame unchanged, so AUX1 looks like the button v392 dropped ("Press all 8 buttons").

- [ ] **Step 1: Copy the image** (`cp local/flashes/firmware.bin frontend/public/firmware/firmware.bin`) and confirm the SHA-256 is `b2fc91dbcb0159d79c3b15e987301979f466163a819c7a90aff0e806ed943380`.
- [ ] **Step 2: Confirm the dropped button.**

  Read page 3's frame after each press, with `boot-probe --dump-frame` or the patch's walk: which button's tile is gone, and whether START still finishes the page. Remove that entry from `TO_SHAKE_PAGE` in `boot_progress.rs` and in `walk.ts` (the two tables are kept identical), and update the "all nine" comment.
- [ ] **Step 3: Re-pin each data rung from a measured value.**

  This covers frame hashes, step budgets and caps, and console needles. For each: run it, read the new value, check the frame by eye with `--dump-frame` under `local/m9-frames/`, then pin it. Recompute the budgets by each constant's documented formula (for example "stable step × 2, rounded up") and keep that formula in its doc. Add a row to "Re-pins" for every changed value. Mark the rung docs' hardware-comparison sentences as "compared on v335 (2026-10-0x); v392 checked by eye in Milestone 9".
- [ ] **Step 4: Re-derive each code-address rung for v392.**

  Find the same function in v392 by the evidence its doc cites (call shape, the ROM call it makes, nearby strings). Re-pin its address and step, citing the evidence in the rung doc. If the guarded code no longer exists in v392, retire the rung, and add a ruling with the evidence (`Ruling R-T2-<k>: …`).
- [ ] **Step 5: Re-pin the two equivalence rungs last,** from the fully re-pinned walk.
- [ ] **Step 6: Run the full check list.**

  Expected: all green. `git diff 6a27f5f -- emulator-core/src emulator-wasm/src` shows only Task 1's rename (no logic). The release suite time is recorded in the decisions doc.
- [ ] **Step 7: Commit** `feat: emulate the official v392 firmware; re-pin the rungs`.
- [ ] **Step 8: GATE.** Stop and give the human partner the "Re-pins" table and the changed frames, as paths under `local/m9-frames/`. Continue only after their answer.

### Task 3: Software reset (Scanner opens after its reboot)

**Files:**
- Modify: `emulator-core/src/cpu/rom_stubs.rs`, `emulator-core/src/cpu/mod.rs`, `emulator-core/src/rom.rs`, `emulator-core/src/peripherals/rtc_cntl.rs`, `emulator-core/src/runtime.rs`, `emulator-core/tests/boot_progress.rs`
- Test: unit tests in `runtime.rs`, `rtc_cntl.rs`, `rom.rs`; rungs in `boot_progress.rs`

**Interfaces:**
- Consumes: `boot_from_firmware_image_with_rom_stubs` (Task 1); `REPIN_BASE` (Task 2).
- Produces:
  - `RomStubEffect::RequestReset(u32)`: a stub with this effect records the reason and does not return;
  - `Cpu::take_reset_request(&mut self) -> Option<u32>`;
  - `rom::SOFTWARE_RESET: u32 = 0x4000_0090` and `rom::SOFTWARE_RESET_CPU: u32 = 0x4000_0094` (`esp32c3.rom.ld`);
  - `rom::RESET_REASON_CORE_SW: u32` and `rom::RESET_REASON_CPU0_SW: u32`, both from `components/soc/esp32c3/include/soc/reset_reasons.h` v5.5.3, cited;
  - `rom::esp32c3_rom_stubs_with_reset_reason(reason: u32) -> RomStubTable`, where `esp32c3_rom_stubs()` returns exactly `esp32c3_rom_stubs_with_reset_reason(POWERON_RESET)`;
  - `FirmwareRuntime::soft_reset_count(&self) -> u32`.

What a soft reset keeps, on top of a fresh `boot_from_firmware_image_with_rom_stubs` of the same image:
- **The emulated flash, whole.** It holds the identity and `reboot.cfg`.
- **`RTC_CNTL_STORE0..7`:** plain R/W, retained. Offsets come from `soc/rtc_cntl_reg.h`. ESP-IDF keeps the reset hint in `RTC_RESET_CAUSE_REG`, which is `STORE6`, per `esp_system/port/soc/esp32c3/reset_reason.c`; cite it.
- **RTC fast memory** (`soc::RTC_RANGE`), before the image's RTC segment is reloaded over it.
- **The framebuffer pixels:** the panel keeps its picture until the firmware redraws.
- **The console's output text,** with its queued host input dropped as `reset()` does.
- **Held buttons and the host-set acceleration.**
- **`total_steps`,** and **host time.** `emulated_time_us()` = the time accumulated before the reset plus the new clock's time.

After the reset, `rtc_get_reset_reason` returns the reset's reason through `esp32c3_rom_stubs_with_reset_reason`. `reset()` (the host's cold reset) is unchanged: blank flash, `POWERON_RESET`, the count back to 0, host time back to 0.

- [ ] **Step 1: Write the failing unit tests in `runtime.rs`** (synthetic image, as the existing runtime tests use):
  - `soft_reset_keeps_flash_store_regs_and_counts_steps`: write a flash byte, `STORE6`, and an RTC-RAM word; execute a `RequestReset(RESET_REASON_CPU0_SW)` stub. Assert the three values survive, `soft_reset_count() == 1`, `total_steps()` did not drop, and the next `rtc_get_reset_reason` call returns `RESET_REASON_CPU0_SW`.
  - `emulated_time_is_monotonic_across_a_soft_reset`: sample `emulated_time_us()` around the reset (never decreasing). `run_until(t_before + 50_000, cap)` across the reset ends with `emulated_time_us() >= t_before + 50_000`.
  - `held_button_survives_a_soft_reset`: hold slot 1 and reset. `raw_button(1)` is true and the GPIO/HC165 read shows it pressed.
  - `console_output_survives_a_soft_reset`: the text before the reset is still a prefix of `console_output()`.
  - `cold_reset_after_a_soft_reset_is_a_power_on`: after a soft reset, `reset()`. The flash byte is back to `0xFF`, `soft_reset_count() == 0`, `emulated_time_us() == 0`, and `rtc_get_reset_reason` returns `POWERON_RESET`.
- [ ] **Step 2: Run** `cargo test -p emulator-core --lib -- soft_reset emulated_time_is_monotonic held_button_survives console_output_survives cold_reset_after`.

  Expected: FAIL to compile (`RequestReset`, `soft_reset_count` missing).
- [ ] **Step 3: Implement.**
  - The effect and `take_reset_request` live in `cpu/` with no chip constant.
  - The two stubs and the reason-parameterized table live in `rom.rs`.
  - `STORE0..7` live in `rtc_cntl.rs`. `STORE4` keeps its current behaviour.
  - The runtime's run loop checks `take_reset_request()` once per step, after the step, and performs the reset there. That keeps `run`'s budget accounting exact.
  - The model and its citations go in `rom.rs`'s module doc ("software reset") and `runtime.rs`'s doc.
- [ ] **Step 4: Run the unit tests.** Expected: PASS.
- [ ] **Step 5: Write the Scanner rungs.**

  Scanner is launcher row 0, column 2: RIGHT ×2 from the launcher checkpoint, then A held `PRESS_HOLD_US`.
  - `scanner_opens_after_one_reboot`:
    - the console shows `app_reboot: reboot into scanner`, then a later `app_reg: launched Scanner`;
    - `soft_reset_count() == 1`;
    - `assert_no_panic_text`;
    - the first stable frame is pinned (measured, then checked by eye).
  - `scanner_home_reboots_to_the_launcher`: from the open Scanner, HOME. Assert the firmware's exit path, measured as either a second reboot (`soft_reset_count() == 2`) or a direct return, then `launched Launcher` again and the launcher frame. Record which path it took in the rung doc.
- [ ] **Step 6: Run the release suite.**

  Expected: the two new rungs pass. Every pin from `REPIN_BASE` is unchanged: `git diff REPIN_BASE -- emulator-core/tests frontend/test frontend/src/bench/walk.ts` shows only additions. Both equivalence rungs pass.
- [ ] **Step 7: Run the full check list and commit** `feat: software reset; Scanner opens after its reboot`.

### Task 4: Every launcher app, pinned

**Files:**
- Modify: `emulator-core/tests/boot_progress.rs`, `docs/milestone-9-decisions.md`

**Interfaces:**
- Consumes:
  - Task 3's `soft_reset_count()`;
  - the launcher checkpoint `launcher_for(OWN_ROLE)`;
  - the probe's navigation (`local/m9-probe.patch`): DOWN × row, RIGHT × column, then A.

- [ ] **Step 1: Add one rung per app, generated by a macro so they run in parallel.**

  Each rung asserts the app's `app_reg: launched <name>`, `assert_no_panic_text`, the expected `soft_reset_count()`, and the first stable frame, pinned after measuring and an eye check. Table (cell = row × 4 + column; "reboots" from the v392 probe):

  | Cell | App (`launched …`) | Reboots |
  |---|---|---|
  | 0 | My Badge | no |
  | 2 | Scanner (Task 3) | yes |
  | 3 | Sync | yes |
  | 4 | Share | yes |
  | 5 | Solana Scavenger Hunt | no |
  | 6 | Sponsors | yes |
  | 7 | Workshop Quest | yes |
  | 8 | Mood LEDs | no |
  | 9 | Socials | no |
  | 10 | Schedule | no |
  | 11 | Info & Rules | no |
  | 12 | Museum | no |
  | 13 | Progress | no |
  | 14 | Snake (existing rungs) | no |
  | 15 | Dice (existing rungs) | no |

  Skip cells whose existing rungs already pin the open frame (Dice, Snake), and reuse those rungs' constants instead.

  **Sync:** if after its reboot it stops at the Bluetooth ROM call (`MEPC    : 0x400008ec` in the console), the rung asserts exactly that instead of `launched Sync`. Its doc says Milestone 10 lifts it.

  **Connect (cell 1)** gets `connect_stops_at_the_bluetooth_controller`, asserting `hal_radio: init_once` and `MEPC    : 0x400008ec`.

  The exact app names come from the probe's console. Leave the table in the rung doc.
- [ ] **Step 2: Run the release suite.**

  Expected: all pass, the time under 90 s and recorded, and no existing pin moved: `git diff REPIN_BASE -- emulator-core/tests` shows only additions, plus Task 3's additions.
- [ ] **Step 3: Re-measure the benchmarks on v392.**

  Run `cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture` and `bun run bench:wasm`. Add a "Measurements" row in the decisions doc next to Milestone 8's numbers (HOME 0.614x, DOWN 0.977x, onboarding 1.224x on WASM). There is no speed target.
- [ ] **Step 4: Run the full check list and commit** `test: every launcher app opens on v392`.

### Task 5: Human gate — the browser eyeball check

- [ ] **Step 1: Smoke-test in Chrome first.**

  Run `bun run build:wasm && bun run dev`. In Chrome (the chrome-devtools MCP, orchestrator only):
  - provision hacker, walk onboarding, reach the launcher;
  - open Scanner and confirm the reboot shows as an ordinary screen change, with no stall and a sane speed readout;
  - capture screenshots under `local/m9-browser/`.

  Stop the dev server after.
- [ ] **Step 2: GATE.** Ask the human partner to eyeball v392 in their browser: boot, onboarding, the launcher, and the apps that open, including the five rebooting ones. One message with everything open. Record their verdict in the decisions doc. A "doesn't make sense" finding goes back to the task that owns it.

### Task 6: Docs and the final review

**Files:**
- Modify: `docs/firmware-emulator-notes.md`, `CLAUDE.md`, `docs/milestone-9-decisions.md`

- [ ] **Step 1: Update the notes.**
  - Current state: v392, the apps table, the soft reset.
  - "Known limitations":
    - Bluetooth (Connect; Sync after its reboot);
    - the RTC timer restarting at the reset;
    - real ROM/bootloader reboot time not modeled.
  - "Ground truth": the badge runs v335.
  - A "Milestone 9" history section.
- [ ] **Step 2: Update CLAUDE.md.**
  - The current state and the firmware file and version.
  - The suite counts and times.
  - The architecture map: `runtime.rs` soft reset, `rom.rs` reset stubs, `rtc_cntl` STORE registers.
- [ ] **Step 3: Finish the decisions doc.**
  - Every `Ruling:` line.
  - The final suite numbers.
  - The Milestone 10 backlog, led by `docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`. Carry over Milestone 8's backlog items that are still open.
- [ ] **Step 4: Run the full check list and commit** `docs: Milestone 9 notes, CLAUDE.md and decisions`.
- [ ] **Step 5: Whole-branch review** (opus), then superpowers:finishing-a-development-branch.

## Orchestrator notes

- **Execution method:** chosen by the human partner. The Milestone 6 and 7 convention: subagent-driven with opus implementers and reviewers, sonnet scoped re-reviews of fix rounds, and an opus whole-branch review.
- **Ledger:** `local/m9-sdd-ledger.md` in the worktree. One line per dispatch, verdict, fix round, ruling (`Ruling R-T<n>-<k>: <decision> — cost if wrong: <...>`), measurement and human answer. Start it with a preflight scan of the Interfaces blocks, and record `REPIN_BASE` after Task 2.
- **Order:** 1 → 2 (gate) → 3 → 4 → 5 (gate) → 6, strictly sequential.
- **Gates (stop and ask; ask everything open in one message):**
  - Task 2 Step 8;
  - Task 5;
  - a release `boot_progress` over 90 s;
  - any override of a recorded decision;
  - any `emulator-core/src` change that v392 turns out to need in Task 2 (report it, do not improvise);
  - a pin moving after `REPIN_BASE` for any reason other than a recorded Task 3 ruling.
- **Implementer brief:**
  - this plan's Global Constraints and Review Focus, the full task text, the Interfaces it consumes, the spec path, the ledger lines that matter, and the worktree path;
  - paths only for anything under `local/`;
  - remind them about `/usr/bin/grep` and the scratchpad-only rule for ESP-IDF sources.
- **Reviewer checks (on top of the skill's):**
  - every re-pin has a row with its reason, and no v392 frame is described as hardware-compared;
  - no `factory` left outside the excluded uses;
  - `cpu/` and `clock.rs` free of ESP32-C3 constants;
  - `emulator-wasm` logic-free;
  - after Task 2, `git diff REPIN_BASE -- emulator-core/tests frontend/test frontend/src/bench/walk.ts` has no changed pin;
  - no `local/` data, PNGs or the probe patch committed (`git show --stat`).
- **Finish:** copy everything new under the worktree's `local/` to the repo-root `local/` before removing the worktree.

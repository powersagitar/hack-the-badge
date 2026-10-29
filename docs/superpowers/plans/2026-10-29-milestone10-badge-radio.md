# Milestone 10 — The Badge Radio Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect and Sync open on v392 through a fake BLE controller at the VHCI seam. Two emulated badges pair to "SYNCED" in a native test, and in two browsers on two devices linked through WebRTC signaled by a Vercel function. Real-badge hearing is built only if the firmware shows it would matter.

**Architecture:** Three changes in `emulator-core`:
- a chip-agnostic app-image hook mechanism in `cpu/`, sharing one out-of-line "host attention" slow path with the software reset;
- an opt-in eFuse factory MAC;
- a pure HCI controller model in `radio/`, wired to v392's bt.c VHCI functions by the hooks.

Events reach NimBLE through guest calls injected in task context. `FirmwareRuntime` exposes the badge's current advertisement and accepts incoming ones; `emulator-wasm` passes them through. In the browser:
- the Worker carries them;
- a main-thread radio hub fans them out over WebRTC data channels;
- a `navigator.locks` lock keeps one badge per browser.

A Vercel function (Upstash Redis REST, no new dependency) swaps offers and answers, with copy-paste as the fallback.

**Tech Stack:** Rust (`emulator-core`, `emulator-wasm`), wasm-pack, Bun/TypeScript (`frontend/`), WebRTC, Web Locks, Vercel Functions, Upstash Redis REST.

**Spec:** `docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md` (approved 2026-10-29; read "Measured on v392" before any radio task).

## Global Constraints

- **No Milestone 9 pin moves.** The radio initializes only when a radio app opens. Every other rung, `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged` stay byte-identical. A move is a bug in the change, never a re-pin. The only pins that change on purpose:
  - `connect_stops_at_the_bluetooth_controller` and `sync_reboots_then_stops_at_the_bluetooth_controller` are replaced in Task 6, with the reason recorded;
  - `run_to_the_bluetooth_fault` goes with them.
- **Pure performance work** (Task 1, the hook check in Task 3) changes no emulated behaviour: every step count, needle, emulated time and frame hash stays identical.
- Hardware-compared frames: registered screen, onboarding, the launcher and the launcher after DOWN, Dice open/roll/animation, Snake open/after-turn/game over. If one moves, stop and ask.
- `emulator-core/src/cpu/` and `clock.rs` hold no ESP32-C3 or v392 constant. The hook addresses live in `emulator-core/src/radio/v392.rs`, each cited with the spec's evidence.
- `FirmwareBus` keeps ordered, concrete named fields with no `dyn`; the word fast path keeps its order. `emulator-core` has no wasm-bindgen dependency. `emulator-wasm` stays one-line passthroughs, so validation and encoding live in `emulator-core`.
- Unmapped data access never panics; an unmapped instruction fetch always traps. Browser, peer and network input never panic the emulator. A malformed peer packet never crashes a tab.
- Register layouts come from ESP-IDF **v5.5.3** headers, ROM addresses from its linker scripts, and app-image addresses from cited evidence. Never guess. Fetch sources into the session scratchpad, never into the repo, with one plain `curl -sfL -o <scratchpad>/<file> https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>` per file (the worktree's guard refuses loops and variables in shell commands).
- Servers: only the Vercel signaling function and its Upstash Redis store. No relay, no radio traffic through any server, no STUN/TURN (`iceServers: []`), no other third-party service. No new npm or Cargo dependency without asking the human partner first.
- Milestones 3 to 9 decisions stand. Milestone 10 rulings go in `docs/milestone-10-decisions.md` with their task's commit, as `Ruling R-T<n>-<k>: <decision> — cost if wrong: <...>`.
- Data handling:
  - never commit, quote or print anything from `local/full_flash_dump.bin`, `local/boot_log.txt` or `local/m6-badge/`; give paths only;
  - committed tests depend only on `firmware.bin` and the committed fake identities in `frontend/public/firmware/test-identities/`;
  - the fake MACs in tests are obviously fake, locally administered values: `02:00:00:00:10:01` (hacker) and `02:00:00:00:10:02` (organizer).
- Tools:
  - use `/usr/bin/grep`;
  - use `/Users/powersagitar/Developer/powersagitar/hack-the-badge/local/.venv/bin/python -I` for Python;
  - use Bun, never npm or node;
  - scratch probes (copies of `boot_progress.rs` with an ignored probe test) are deleted before committing.
- The full check list after any change under `emulator-core/` or `emulator-wasm/`, before each commit:
  ```
  cargo fmt --all && cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck && bun run build
  ```
  All green. Clippy stays at its 3 known warnings. The release `boot_progress` suite stays under 90 s, with its time recorded.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` only. Pushing, merging and deploying are the human partner's call.

## Review Focus

1. **Connect opened, left with HOME, and opened again** (the radio is `init_once` and stays up): the second open must not re-run controller init, double-register the VHCI callback or replay stale events, and it reaches the home screen again. Pinned by Task 6's `connect_reopens_after_home`.
2. **A soft reset while the radio is up** (Sync's reboot into itself, or the panic reboot): the controller model resets with the guest (no events queued from before the reset), keeps its MAC, and the radio initializes again cleanly. Pinned by Task 6's `radio_state_resets_with_a_soft_reset` and by `sync_opens_after_its_reboot`.
3. **A peer advertisement that arrives when nobody is scanning** (a linked peer before Connect opens, or Sync not looking): it is dropped, never queued for later, and the queue is bounded. Pinned by Task 5's `advertisement_without_scanning_is_dropped` and `event_queue_is_bounded`.
4. **Garbage from the network:** truncated, oversized or random bytes from a peer or the signaling function are rejected without a panic or an unhandled promise rejection. Pinned by Task 8's `deliver_rejects_malformed_bytes` (core), Task 9's `hub_drops_malformed_peer_messages`, and Task 10's `signal_rejects_bad_requests`.
5. **Two tabs, or a hidden tab:** a tab that takes the badge over stops the other tab's emulator before starting its own. While a tab is hidden (R-T8-10's pause), incoming advertisements do not pile up. Pinned by Task 9's `takeover_stops_the_previous_holder` and `paused_hub_keeps_only_the_latest_advertisement_per_peer`.

---

## File Structure

- `emulator-core/src/runtime.rs`: `run_impl`'s per-step reset check moves to an out-of-line slow path (Task 1). Hook dispatch (Task 3), the radio (Task 6), the MAC (Task 4) and the radio host API (Task 8) are added.
- `emulator-core/src/cpu/mod.rs`: one `attention` flag that `step` sets for a reset request or a hook hit (Tasks 1, 3).
- `emulator-core/src/cpu/app_hooks.rs`: new, the chip-agnostic app-image hook table (Task 3).
- `emulator-core/src/peripherals/efuse.rs`: new, the factory-MAC eFuse words, or a ROM stub in `rom.rs`, whichever Task 4 measures. Its field goes in `mem/bus.rs` (Task 4).
- `emulator-core/src/radio/mod.rs`: new, `Radio` (hook handling, event injection, soft-reset state) (Task 6).
- `emulator-core/src/radio/hci.rs`: new, the pure HCI controller model (Task 5).
- `emulator-core/src/radio/v392.rs`: new, the hook addresses and their evidence (Task 6).
- `emulator-core/src/rom.rs`: the injection trampoline code blob, if Task 6 uses one.
- `emulator-core/tests/boot_progress.rs`:
  - R-T6-1 test gaps (Task 2);
  - the radio rungs that replace the fault rungs (Task 6);
  - the two-badge rung (Task 8).
- `emulator-wasm/src/lib.rs`: passthroughs (Tasks 2, 8).
- `frontend/src/runtime/firmware-runtime.ts`, `firmware-worker.ts`, `firmware-worker-protocol.ts`, `firmware-worker-client.ts`: the radio messages, the MAC and Bump (Task 9).
- `frontend/src/radio/hub.ts`, `frontend/src/radio/lock.ts`, `frontend/src/radio/peer-link.ts`, `frontend/src/radio/signal-client.ts`: new (Tasks 9, 10).
- `frontend/src/ui/shell.ts`, `frontend/src/main.ts`: the Bump button, the take-over notice, the link panel (Tasks 9, 10).
- `frontend/api/signal.ts`: new, the Vercel entry. `frontend/src/radio/signal-handler.ts`: new, the pure handler with a store interface. `frontend/vite.config.ts`: a dev middleware for `/api/signal` (Task 10).
- `frontend/test/*.test.ts`: one test file per new module.
- `docs/milestone-10-decisions.md`: new (Task 1), grown by every task. `docs/firmware-emulator-notes.md` and `CLAUDE.md`: Task 12.

---

### Task 1: The reset check off the hot loop (ride-along; no behaviour change)

**Files:**
- Modify: `emulator-core/src/runtime.rs` (`run_impl`, lines 314-354), `emulator-core/src/cpu/mod.rs`
- Create: `docs/milestone-10-decisions.md`

**Interfaces:**
- Produces:
  - `Cpu::needs_attention(&self) -> bool`: one `bool` that `step` sets when anything needs the host (today a reset request; Task 3 adds hook hits);
  - `FirmwareRuntime::attend(&mut self)`: `#[cold] #[inline(never)]`, which takes the reset request and soft-resets. `run_impl` calls it only when `needs_attention()` is true.
  - Record `BASE` (this task's parent commit, `9095d5d` or later) in the ledger. Every later equivalence check diffs against it.

- [ ] **Step 1: Create `docs/milestone-10-decisions.md`** with sections "Why", "Measurements" (copy the spec's "Measured on v392"), "Rulings", "Replaced rungs" (rung | why) and "Milestone 11 backlog".
- [ ] **Step 2: Baseline A/B, before any change.** Run `cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture` and `bun run bench:wasm`, alternating between a worktree checkout of `b25a68c` and `BASE`, three pairs each, in one session. Record the table: HOME, DOWN, onboarding, steps/s. This is also the WASM A/B Milestone 9 lacked.
- [ ] **Step 3: Implement the flag and the cold path.** `take_reset_request` stays as is. `run_impl`'s per-step work becomes the flag test only.
- [ ] **Step 4: Run the full check list.** Expected: all green, and `git diff BASE -- emulator-core/tests frontend/test frontend/src/bench/walk.ts` is empty.
- [ ] **Step 5: Re-run the A/B** (this commit against `b25a68c`, three pairs, native and Bun). Record the numbers. The target is native within noise of `b25a68c`. If it falls short, record what was recovered as a ruling; do not change behaviour to get more.
- [ ] **Step 6: Commit** `perf: take the soft-reset check off the hot loop`.

### Task 2: `soft_reset_count` in WASM and R-T6-1's test gaps (ride-along)

**Files:**
- Modify: `emulator-wasm/src/lib.rs`, `emulator-core/src/runtime.rs` (tests), `emulator-core/tests/boot_progress.rs`, `frontend/test/cpu-wasm.test.ts`

**Interfaces:**
- Produces: `FirmwareEmulator::soft_reset_count(&self) -> u32` (wasm).

- [ ] **Step 1: Write the failing tests.**
  - `runtime.rs`: `released_button_does_not_replay_after_a_soft_reset`: press slot 1, run, release, run, soft-reset. After the reset, `raw_button(1)` is false, and the GPIO/HC165 read shows it released from the first step.
  - `boot_progress.rs`: `assert_launcher_opens` asserts `app_reboot: reboot into <slug>` in the console since A whenever `soft_resets == 1`. The slugs come from the console: `scanner`, `share`, `sponsors`, `workshop_quest` (confirm each by running).
  - `cpu-wasm.test.ts`: `soft_reset_count` is 0 after `new` and after `reset()`.
- [ ] **Step 2: Run them.** Expected: the wasm test fails to compile; the other two pass, or fail only on a wrong slug (fix the slug, not the code).
- [ ] **Step 3: Add the passthrough.** `bun run build:wasm`.
- [ ] **Step 4: Run the full check list.** Expected: all green, and no pin changed against `BASE` (`git diff BASE -- emulator-core/tests` shows only additions).
- [ ] **Step 5: Commit** `test: close R-T6-1's gaps; soft_reset_count in WASM`.

### Task 3: App-image hooks in `cpu/` (chip-agnostic)

**Files:**
- Create: `emulator-core/src/cpu/app_hooks.rs`
- Modify: `emulator-core/src/cpu/mod.rs`, `emulator-core/src/runtime.rs`

**Interfaces:**
- Consumes: Task 1's `needs_attention` and `attend`.
- Produces:
  - `pub struct AppHookTable` with `insert(&mut self, addr: u32, id: u16) -> &mut Self` and `lookup(&self, addr: u32) -> Option<u16>`;
  - `Cpu::set_app_hooks(&mut self, table: AppHookTable)`;
  - `Cpu::take_hook_hit(&mut self) -> Option<HookHit>`, where `pub struct HookHit { pub id: u16, pub pc: u32 }`;
  - `Cpu::hook_return(&mut self, a0: Option<u32>)`: writes `a0` if given and sets `pc = ra`;
  - `Cpu::hook_pass_through(&mut self)`: the next step at the hooked pc fetches and executes normally, once.

  A hit consumes the step without fetching (like a ROM stub), sets the attention flag and leaves `pc` on the hooked address. The handler (Task 6) must call exactly one of `hook_return`, `hook_pass_through` or a redirect (writing `pc`). `run_impl`'s cold path dispatches hits to a handler that Task 6 supplies. Until then, an unhandled hit passes through.

- [ ] **Step 1: Write the failing unit tests** in `app_hooks.rs` and `cpu/mod.rs`, on a synthetic program (as the existing CPU tests use):
  - `empty_table_never_hits`;
  - `hook_hit_consumes_the_step_and_keeps_pc`;
  - `hook_return_sets_a0_and_returns_to_ra`;
  - `pass_through_executes_the_hooked_instruction_once`: the instruction runs once, and the next arrival at that pc hits again;
  - `interrupt_is_taken_before_a_hook`: same ordering as a ROM stub;
  - `hooks_and_rom_stubs_coexist`.
- [ ] **Step 2: Run** `cargo test -p emulator-core --lib app_hooks`. Expected: FAIL to compile.
- [ ] **Step 3: Implement.**

  The hook check must not add a map lookup to every step. Keep the fast path to at most a range compare plus a branch. The hooks Task 6 installs span `0x42080c64..=0x42081218` plus one idle-hook address elsewhere, so a single min/max range is not enough: use two ranges, or a flag in the decode cache keyed by pc. It sits after the interrupt check and the ROM-stub check, before the fetch. No address or chip constant goes in `cpu/`.
- [ ] **Step 4: Run the unit tests.** Expected: PASS.
- [ ] **Step 5: Measure.** Run the full check list, then `bench_redraws` native and `bun run bench:wasm`, A/B against Task 1's commit, three pairs. Expected: all green, no pin change against `BASE`, and the slowdown within noise. Record it.
- [ ] **Step 6: Commit** `feat: chip-agnostic app-image hooks`.

### Task 4: The eFuse factory MAC (opt-in)

**Files:**
- Create (if registers): `emulator-core/src/peripherals/efuse.rs`. Modify: `emulator-core/src/mem/bus.rs` (a new named field, in address order), `emulator-core/src/mem/soc.rs`, `emulator-core/src/runtime.rs`. Or modify (if a ROM call): `emulator-core/src/rom.rs`.
- Test: unit tests in the new module; a scratch probe for the measurement.

**Interfaces:**
- Produces:
  - `FirmwareRuntime::set_factory_mac(&mut self, mac: &[u8]) -> bool`: false, and no change, unless `mac.len() == 6`;
  - `FirmwareRuntime::factory_mac(&self) -> Option<[u8; 6]>`.

  With none set, the read path is byte-identical to today. A cold `reset()` keeps the MAC (it is the chip's, like a real badge's). A soft reset keeps it.

- [ ] **Step 1: Measure the read path.** Use a scratch probe (deleted after). From hacker's launcher, open Connect with the controller still unmodeled. Trace every load in `0x6000_8800..0x6000_8a00` (`DR_REG_EFUSE_BASE`, `soc/reg_base.h`) and every ROM-stub call, from boot to the fault. Answer three things: which registers or ROM call carry the factory MAC; whether anything reads them before `hal_radio: init_once`; and what MAC `esp_read_mac` returns today. Record all three in the decisions doc ("Measurements").
- [ ] **Step 2: Decide the checkpoint question** from Step 1, and record it as a ruling. If nothing reads the MAC before the radio, rungs set it on a clone of `launcher_for(role)`. Otherwise it is set at power-on, and the radio rungs walk from their own boot.
- [ ] **Step 3: Write the failing tests:**
  - `unset_mac_reads_as_before`: the registers or stub return today's values;
  - `set_mac_is_served_in_efuse_layout`: the word and byte order come from `efuse_reg.h` v5.5.3 / `esp_efuse_table.csv`, cited;
  - `set_factory_mac_rejects_a_wrong_length`;
  - `mac_survives_cold_and_soft_reset`.
- [ ] **Step 4: Implement.** Cite the register offsets and field layout in the module doc.
- [ ] **Step 5: Verify against the firmware.** The scratch probe with `set_factory_mac(02:00:00:00:10:01)` shows `esp_read_mac` returning it, in the same `esp_read_mac` result buffer read at `0x4201035a`.
- [ ] **Step 6: Run the full check list.** Expected: all green, no pin changed against `BASE`.
- [ ] **Step 7: Commit** `feat: opt-in eFuse factory MAC`.

### Task 5: The HCI controller model (pure)

**Files:**
- Create: `emulator-core/src/radio/hci.rs`, `emulator-core/src/radio/mod.rs` (the module declaration only, for now)
- Test: unit tests in `hci.rs`

**Interfaces:**
- Produces:
  - `pub struct Advertisement { pub addr: [u8; 6], pub addr_type: u8, pub data: Vec<u8> }`, with `data.len() <= 229`, the single-report limit the firmware enforces;
  - `pub struct HciController`, with:
    - `new(public_addr: [u8; 6]) -> Self`;
    - `host_to_controller(&mut self, h4: &[u8]) -> bool`: false for a malformed or unknown packet type, never a panic;
    - `pop_event(&mut self) -> Option<Vec<u8>>`: H4-framed (`0x04`, event code, length, params);
    - `advertisement(&self) -> Option<Advertisement>`: `Some` only while extended advertising is enabled with data set;
    - `deliver(&mut self, adv: &Advertisement, rssi: i8) -> bool`: true if it became an LE Extended Advertising Report, false if not scanning or rejected;
    - `is_scanning(&self) -> bool`.
  - `pub const MAX_QUEUED_EVENTS: usize = 32`.

  Behaviour comes from the Bluetooth Core Specification v5.x Vol 4 Part E, cited per command in the module doc:
  - every command answers with Command Complete (or Command Status, where the spec says) in order;
  - unknown commands answer `Unknown HCI Command` (0x01), so the host's own error path runs;
  - the commands covered are those NimBLE sends to reach host sync and to run extended advertising and scanning: Reset, Read Local Version / Supported Commands / Features / BD_ADDR / Buffer Size (LE and classic), Set Event Mask (and Page 2, LE), LE Set Random Address, the LE Set Extended Advertising Parameters / Data / Scan Response / Enable, LE Set Extended Scan Parameters / Enable, LE Read Maximum Advertising Data Length, LE Read Number of Supported Advertising Sets, and any vendor command the firmware sends, which is answered success only if Task 6's trace shows it;
  - the exact list is whatever Task 6's command trace shows. Add commands to this task's tests as Task 6 finds them.
  - reports respect the scan's filter-duplicates setting.

- [ ] **Step 1: Write the failing tests:**
  - `reset_answers_command_complete_success`;
  - `unknown_opcode_answers_unknown_command`;
  - `ext_adv_enable_exposes_the_advertisement`;
  - `ext_adv_disable_clears_it`;
  - `scan_enable_turns_delivery_into_ext_adv_reports` (check the report's fields against Vol 4 Part E 7.7.65.13);
  - `advertisement_without_scanning_is_dropped`;
  - `event_queue_is_bounded` (`MAX_QUEUED_EVENTS`; the oldest advertising report is dropped first, never a command reply);
  - `oversized_or_truncated_packets_are_rejected`;
  - `random_bytes_never_panic` (a seeded loop over 10,000 random packets).
- [ ] **Step 2: Run** `cargo test -p emulator-core --lib radio::hci`. Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Run the full check list and commit** `feat: HCI controller model`.

### Task 6: Connect and Sync open (Part 1)

**Files:**
- Create: `emulator-core/src/radio/v392.rs`
- Modify: `emulator-core/src/radio/mod.rs`, `emulator-core/src/runtime.rs`, possibly `emulator-core/src/rom.rs` (trampoline blob), `emulator-core/tests/boot_progress.rs`, `docs/milestone-10-decisions.md`

**Interfaces:**
- Consumes: Task 3's hooks, Task 4's `set_factory_mac`, Task 5's `HciController`.
- Produces:
  - `pub struct Radio` in `radio/mod.rs`, owned by `FirmwareRuntime`, with `on_hook(&mut self, hit: HookHit, cpu: &mut Cpu, bus: &mut FirmwareBus)` called from `attend`, `soft_reset(&mut self)`, and `init_count(&self) -> u32` (successful `esp_bt_controller_init` hooks since power-on). `FirmwareRuntime::radio_init_count(&self) -> u32` passes it through for tests;
  - `v392.rs`: one `pub const` per hooked address, named after the bt.c function (`ESP_BT_CONTROLLER_INIT: u32 = 0x4208_0ce4`, `ESP_BT_CONTROLLER_ENABLE: u32 = 0x4208_0fc8`, `ESP_VHCI_HOST_CHECK_SEND_AVAILABLE: u32 = 0x4208_0c64`, `ESP_VHCI_HOST_SEND_PACKET: u32 = 0x4208_0c84`, `ESP_VHCI_HOST_REGISTER_CALLBACK: u32 = 0x4208_0cbe`, `BTDM_CONTROLLER_STATUS: u32 = 0x3fca_089c`, plus the idle hook point Step 2 identifies), each with the spec's evidence in its doc;
  - the radio rungs below.

The hook semantics (bt.c v5.5.3, which the module doc cites):
- `init`: status must be IDLE (0), else return `ESP_ERR_INVALID_STATE` (0x103) as bt.c does. Otherwise write INITED (1) and return 0.
- `enable`: status must be INITED, else return 0x103. Otherwise write ENABLED (2) and return 0.
- `register_callback(cb)`: store the two function pointers read from `cb` and return 0 (`ESP_OK`).
- `check_send_available`: return 1.
- `send_packet(data, len)`: hand the H4 bytes to `HciController::host_to_controller`, then deliver its replies.

Delivering an event means calling the stored `notify_host_recv(data, len)` in task context, never in an interrupt, with the interrupted code's registers preserved and the packet in guest memory the firmware does not use.

- [ ] **Step 1: Trace before you build.** Use a scratch probe (deleted after) with the hooks returning success and logging, and no delivery yet. Record the H4 commands NimBLE sends up to the host-sync wait. Add any missing ones to Task 5's model, with tests, as a fix to Task 5's code in this task's commit.
- [ ] **Step 2: Choose the injection points, with evidence.** Command replies go inside the `send_packet` hook. Asynchronous events go through an idle-task hook point; identify its v392 address with evidence (FreeRTOS `prvIdleTask` / `esp_vApplicationIdleHook` in ESP-IDF v5.5.3 and the call graph to the WFI). If a guest-executed trampoline is used, its code is a `RomCodeBlob` built with `encode.rs`, at an address the firmware never uses, cited. Record the choice as a ruling, with the stack and context argument.
- [ ] **Step 3: Write the rungs that fail today**, replacing the two fault rungs (delete them and `run_to_the_bluetooth_fault`; add a "Replaced rungs" row each). Every radio rung sets MAC `02:00:00:00:10:01` per Task 4's checkpoint ruling.
  - `connect_opens_with_the_radio_up`: A on cell 1. The console since A has `launched Connect` and `BLE radio ready`, with no `host sync timeout`, no `ble_hs_id_set_rnd`, no `NO RADIO` and no panic text. `soft_reset_count() == 0`. The first stable frame is pinned (measured, then checked by eye).
  - `sync_opens_after_its_reboot`: A on cell 3. Assert `app_reboot: reboot into sync`, then `launched Sync` and `BLE radio ready`, with no panic. `soft_reset_count() == 1`. The first stable frame is pinned.
  - `sync_finds_no_station`: from the open Sync, A ("A: sync now"). The firmware reaches "No sync station in range" (assert by `uitree` over the console, or a pinned frame, whichever is available), with no panic.
  - `connect_reopens_after_home`: Connect, HOME to the launcher (`LAUNCHER_HASH`), then Connect again. `radio_init_count() == 1` throughout, the home frame appears again, and nothing panics.
  - `radio_state_resets_with_a_soft_reset` (unit test in `radio/mod.rs`): queued events and controller state clear, the MAC stays.
  - `no_radio_without_a_radio_app`: the equivalence rungs pass unchanged, and the radio's hooks never hit on `hacker_walk`.
- [ ] **Step 4: Implement** `Radio`, `v392.rs` and the wiring. Install the hook table only in the firmware boot path (`boot_from_firmware_image_with_rom_stubs`'s caller in `runtime.rs`). The table is inert until the firmware calls those addresses.
- [ ] **Step 5: Run the release suite.** Expected: all pass, the replaced rungs are gone, and `git diff BASE -- emulator-core/tests` changes no existing pin outside the replaced rungs. Record the suite time.
- [ ] **Step 6: Run the full check list and commit** `feat: Connect and Sync open through a fake BLE controller`.
- [ ] **Step 7: GATE (spec, "After Part 1, before Part 2").**
  - Save Connect's and Sync's home frames under `local/m10-frames/` for the human partner's eyeball check.
  - Read Connect's pairing exchange in the firmware (the advertising payload's layout, what a bump or both-hold-A sends, what "SYNCED" waits for, and the `rssi_min` and `threshold_mg` defaults), with addresses and strings as evidence, into the decisions doc.
  - Stop and ask in one message: the frames' verdict. If "SYNCED" needs connections, GATT or scan requests/responses, or a check the fake identities cannot pass (a signature against an organizer key), give the evidence; the human partner decides scope.

### Task 7: (reserved for the gate's outcome)

If the gate brings in more radio work (for example scan responses), the plan gets a task here, written then and reviewed like the others. Otherwise skip.

### Task 8: Two badges pair in one test (Part 2)

**Files:**
- Modify: `emulator-core/src/runtime.rs`, `emulator-core/src/radio/mod.rs`, `emulator-wasm/src/lib.rs`, `emulator-core/tests/boot_progress.rs`, `docs/milestone-10-decisions.md`

**Interfaces:**
- Consumes: Task 6's `Radio`; the gate's reading (the `rssi_min` default).
- Produces (the radio host API; the bytes encoding lives in core so wasm stays logic-free):
  - `FirmwareRuntime::radio_advertisement(&self) -> Option<Advertisement>`;
  - `FirmwareRuntime::radio_advertisement_bytes(&self) -> Vec<u8>`: empty when not advertising, else `addr(6) ‖ addr_type(1) ‖ data`;
  - `FirmwareRuntime::deliver_advertisement(&mut self, adv: &Advertisement, rssi: i8) -> bool`;
  - `FirmwareRuntime::deliver_advertisement_bytes(&mut self, bytes: &[u8], rssi: i8) -> bool`: false for anything malformed (length < 7, data > 229, addr_type > 3);
  - `pub const LINKED_PEER_RSSI: i8`: a fixed strong value above the firmware's `rssi_min`, cited from the gate's reading;
  - wasm: `radio_advertisement() -> Vec<u8>`, `deliver_advertisement(bytes: &[u8], rssi: i8) -> bool`, `set_factory_mac(mac: &[u8]) -> bool`.
  - `boot_progress.rs`: `fn bump(rt: &mut FirmwareRuntime)`, the acceleration pulse (peak and duration chosen from `threshold_mg`, and distinct from `shake`). Record its shape as a ruling. Task 9 mirrors it in TypeScript.

- [ ] **Step 1: Write the failing tests.**
  - `deliver_rejects_malformed_bytes` (runtime unit test): short, oversized, bad addr type and random bytes all return false with no panic.
  - `two_badges_pair_to_synced` (rung): hacker and organizer launchers, MACs `…10:01` and `…10:02`, both open Connect. Lockstep: advance both by the same emulated slice (1 ms), then swap each one's `radio_advertisement` into the other with `LINKED_PEER_RSSI`. Then `bump` both at the same emulated time, or both hold A if the gate's reading says the bump needs more. Assert both reach "SYNCED" (frame or `uitree`), then "Contacts: 1" on each home screen, with no panic. Pin the SYNCED frames after an eye check.
- [ ] **Step 2: Run them.** Expected: FAIL to compile.
- [ ] **Step 3: Implement**, then `bun run build:wasm`.
- [ ] **Step 4: Run the release suite.** Expected: all pass, no pin changed against `BASE`, and the suite time recorded (two runtimes in lockstep are the slowest rung; stop and ask if the suite passes 90 s).
- [ ] **Step 5: Run the full check list and commit** `feat: two emulated badges pair to SYNCED`.

### Task 9: The browser radio: Worker, hub, one badge per browser, Bump (Part 3)

**Files:**
- Create: `frontend/src/radio/hub.ts`, `frontend/src/radio/lock.ts`, `frontend/test/radio-hub.test.ts`, `frontend/test/radio-lock.test.ts`
- Modify: `frontend/src/runtime/firmware-worker-protocol.ts`, `firmware-worker.ts`, `firmware-worker-client.ts`, `firmware-runtime.ts`, `frontend/src/ui/shell.ts`, `frontend/src/main.ts`, `frontend/test/firmware-runtime.test.ts`, `frontend/test/firmware-worker.test.ts`, `frontend/test/shell.test.ts`

**Interfaces:**
- Consumes: Task 8's wasm API and `bump` shape.
- Produces:
  - protocol `ToWorker`: `{ kind: "setMac"; mac: Uint8Array }`, `{ kind: "advIn"; bytes: Uint8Array; rssi: number }`, `{ kind: "bump" }`;
  - protocol `FromWorker`: `{ kind: "advOut"; bytes: Uint8Array }`, sent when the badge's advertisement changes and at most every 100 ms while it advertises;
  - `firmware-runtime.ts`: `bumpAcceleration(elapsedUs: number): [number, number, number] | null`, mirroring Task 8's `bump`, and `bump(): void`;
  - `hub.ts`: `class RadioHub { addLink(link: RadioLink): void; removeLink(id: string): void; fromLocal(bytes: Uint8Array): void; bump(): void; setPaused(paused: boolean): void }`, where `interface RadioLink { id: string; send(msg: HubMessage): void; onMessage(cb: (msg: unknown) => void): void }` and `type HubMessage = { t: "adv"; b: string } | { t: "bump" }` (`b` is base64). The hub validates every incoming message's shape and size before it reaches the Worker, and delivers adverts at `LINKED_PEER_RSSI`. That is a TypeScript constant in `hub.ts` with the same value as Task 8's Rust one, and a test asserts the value matches the one written in Task 8's ruling;
  - `lock.ts`: `acquireBadge(onLost: () => void): Promise<{ release(): void } | null>` and `takeOver(): Promise<void>`, over `navigator.locks` with the name `"htn-badge"`;
  - the per-device MAC: `loadOrCreateMac(storage: Storage | null): Uint8Array`, a random locally administered unicast MAC (`(b0 & 0xfc) | 0x02`), kept under `localStorage["htn-badge-mac"]`. Every read and write is in try/catch; with no storage it is random per load.

- [ ] **Step 1: Write the failing tests:**
  - `bumpAcceleration` matches Task 8's shape at its sample points;
  - `hub_forwards_local_adverts_to_every_link`;
  - `hub_does_not_forward_between_links` (radio does not hop);
  - `hub_drops_malformed_peer_messages` (wrong `t`, bad base64, more than 236 bytes, non-object, with no throw);
  - `bump_hits_local_and_every_link`;
  - `paused_hub_keeps_only_the_latest_advertisement_per_peer`;
  - `takeover_stops_the_previous_holder` (a fake `LockManager`): the old holder's `onLost` runs and its runtime stops before the new one starts;
  - `loadOrCreateMac` is stable across calls with storage, locally administered, and works with storage throwing;
  - shell: a Bump button next to Shake; a "Badge running in another tab — Run here" notice with a take-over button.
- [ ] **Step 2: Run** `bun test`. Expected: the new tests fail.
- [ ] **Step 3: Implement.** The Worker sets the MAC once before `start`. R-T8-10's hidden-page pause also pauses the hub (`setPaused`).
- [ ] **Step 4: Run the full check list.** Expected: all green.
- [ ] **Step 5: Smoke-test in Chrome** (orchestrator only, chrome-devtools MCP). Run `bun run dev`. One tab: provision, reach the launcher, open Connect; the home screen draws. A second tab shows the take-over notice; taking over stops the first. Bump with no link: Connect reacts as alone ("No badge found…" or whatever the firmware shows). Screenshots go under `local/m10-browser/`. Stop the dev server.
- [ ] **Step 6: Commit** `feat: browser radio hub, one badge per browser, Bump`.

### Task 10: WebRTC links, signaled through Vercel (Part 4)

- [ ] **Step 0: GATE (spec, "Before Part 4").** Ask the human partner to connect Upstash Redis to the Vercel project through the Marketplace, and to tell you the env var names it created (`KV_REST_API_URL` / `KV_REST_API_TOKEN` or `UPSTASH_REDIS_REST_URL` / `_TOKEN`). They also need a second device on the same network for Task 11. Continue after their answer.

**Files:**
- Create:
  - `frontend/src/radio/signal-handler.ts` (pure);
  - `frontend/api/signal.ts` (the Vercel entry: Upstash store plus the handler);
  - `frontend/src/radio/signal-client.ts`, `frontend/src/radio/peer-link.ts`;
  - `frontend/test/signal-handler.test.ts`, `frontend/test/peer-link.test.ts`.
- Modify: `frontend/vite.config.ts` (a dev-only middleware serving `/api/signal` with an in-memory store), `frontend/src/ui/shell.ts`, `frontend/src/main.ts`, `frontend/tsconfig.json` (include `api/` if needed), `frontend/test/shell.test.ts`.

**Interfaces:**
- Consumes: Task 9's `RadioLink` and `RadioHub`.
- Produces:
  - `interface SignalStore { set(key: string, value: string, ttlSeconds: number, onlyIfAbsent: boolean): Promise<boolean>; get(key: string): Promise<string | null> }`, with `MemoryStore` and `UpstashStore` (Upstash's REST API through `fetch`: `SET key value EX ttl [NX]`, `GET key`; no npm dependency);
  - `handleSignal(req: Request, store: SignalStore): Promise<Response>`, with this protocol (JSON):
    - `POST {op:"create", offer}` returns `{room}`;
    - `GET ?room=R&want=offer` returns `{offer}`, or 404;
    - `POST {op:"answer", room, answer}` returns 204, or 409 if already answered;
    - `GET ?room=R&want=answer` returns `{answer}`, or 204 while pending.

    Room codes are 8 characters of Crockford base32 from `crypto.getRandomValues`. Keys expire after 300 s. An SDP is at most 16 KiB. Bad JSON, an unknown op, a bad room format or an oversized body give 400. Nothing is logged;
  - `frontend/api/signal.ts`: the Web-handler entry Vercel's Functions runtime calls for `/api/signal`, which reads the env vars from the gate and calls `handleSignal`;
  - `peer-link.ts`: `createOffer(): Promise<{ link: PeerLink; offer: string }>`, `acceptOffer(offer: string): Promise<{ link: PeerLink; answer: string }>` and `PeerLink.complete(answer: string)`. `PeerLink` implements `RadioLink`. It uses `new RTCPeerConnection({ iceServers: [] })` with one ordered data channel `"radio"`, waits for `iceGatheringState === "complete"` before returning the SDP (no trickle), and sends no traffic until open;
  - the copy-paste fallback: the same SDP strings, compressed (`CompressionStream("deflate-raw")`) and base64url-encoded, behind a "manual" toggle in the link panel;
  - `signal-client.ts`: `hostRoom()` and `joinRoom(code)`. They poll every 1 s for up to 120 s and give a clear message on expiry or error.

- [ ] **Step 1: Write the failing tests:**
  - `signal-handler.test.ts` with `MemoryStore`: the create → offer → answer → answer round trip; `signal_rejects_bad_requests` (each 400 case, an oversized SDP, 409 on a second answer); expiry (an injected clock in `MemoryStore`); room codes unique over 10,000 creates;
  - `peer-link.test.ts`: the manual codes round-trip (`encode(decode(x)) === x`) and reject garbage without throwing; `PeerLink.onMessage` passes data-channel messages through. The real `RTCPeerConnection` is not available under `bun test`, so inject a fake.
  - shell: the link panel has "Host link" (shows a room code), "Join link" (an input), the link list with remove, and the "manual" toggle.
- [ ] **Step 2: Run** `bun test`. Expected: the new tests fail.
- [ ] **Step 3: Implement.** Check the Vercel Functions entry format against Vercel's current docs for a Vite (non-Next) project, cited in the file's comment.
- [ ] **Step 4: Run the full check list.** Expected: all green.
- [ ] **Step 5: Smoke-test in Chrome**, two windows of one profile on `bun run dev` (the dev middleware). The lock means only one runs a badge, so the second window uses a separate Chrome profile. Host in one, join in the other, and the link list shows it connected. Then the manual toggle: the same through pasted codes. Stop the dev server.
- [ ] **Step 6: Commit** `feat: WebRTC radio links signaled through Vercel, with copy-paste fallback`.

### Task 11: Part 5 — hearing real badges (conditional)

- [ ] **Step 1: Read the firmware.** From Task 6's gate reading, does one-way hearing of a real badge change anything visible in Connect or Sync? Name the code paths: what Connect does with a report and no answer, and when Sync shows "station nearby". Write the evidence into the decisions doc.
- [ ] **Step 2: GATE (Part 5 go/no-go).** Ask the human partner with the evidence. If nothing visible results, Part 5 is dropped: add a ruling and a backlog line, then skip to Task 12.
- [ ] **Step 3 (if go): GATE (before the spike).** Arrange the human partner's badge and Chrome with `chrome://flags/#enable-experimental-web-platform-features`.
- [ ] **Step 4 (if go): the spike.** A throwaway page calls `navigator.bluetooth.requestLEScan({ acceptAllAdvertisements: true })`. Check whether the badge's extended advertisements arrive, and with which fields (raw data, or only parsed manufacturer and service data). Record the result. If the raw payload is not recoverable, stop and ask.
- [ ] **Step 5 (if the spike passes): build the bridge.** Add `frontend/src/radio/web-bluetooth.ts`, which turns scan results into an `Advertisement` with the real RSSI and becomes one more `RadioLink`-like source for the hub (receive only). Add a Chrome-only "Hear real badges" toggle, and tests with a fake scan event. Then run the full check list and commit `feat: hear real badges through Web Bluetooth`.

### Task 12: Browser gate, docs and the final review

- [ ] **Step 1: GATE (success criterion 5).** Deploy is the human partner's call. Either they deploy a preview, or the check runs on `bun run dev` with the dev middleware over the LAN (`--host`). Ask them to check, in one message:
  - Connect's and Sync's home screens;
  - two devices linked through the signaling function, Connect open on both, Bump, and "SYNCED" on both;
  - the manual fallback once.

  Record the verdict.
- [ ] **Step 2: Update the notes.**
  - Current state: 16 of 16 apps open, and the radio model.
  - "Known limitations": no real controller; one-way limits; LAN-only links; hook addresses tied to v392; the lines that no longer print (`BT controller compile version`, `Feature Config`).
  - A "Milestone 10" history section.
- [ ] **Step 3: Update CLAUDE.md.**
  - Current state.
  - Suite counts and times.
  - The architecture map: `cpu/app_hooks.rs`, `radio/`, `peripherals/efuse.rs`, `frontend/src/radio/`, `frontend/api/signal.ts`.
- [ ] **Step 4: Finish the decisions doc.**
  - Every `Ruling:` line, each with what it costs if wrong.
  - The final suite numbers and benchmarks.
  - The Milestone 11 backlog: Milestone 9's still-open items minus the ones closed here, plus Share's radio transfer and whatever this milestone found.
- [ ] **Step 5: Run the full check list and commit** `docs: Milestone 10 notes, CLAUDE.md and decisions`.
- [ ] **Step 6: Whole-branch review** (opus), then superpowers:finishing-a-development-branch. List every `Ruling:` line under "Rulings I made", each with what it costs if wrong.

## Orchestrator notes

- **Execution method:** chosen by the human partner. The Milestone 6, 7 and 9 convention: subagent-driven with opus implementers and reviewers, sonnet scoped re-reviews of fix rounds, and an opus whole-branch review.
- **Ledger:** `local/m10-sdd-ledger.md` in the worktree. One line per dispatch, verdict, fix round, ruling, measurement and human answer. Start it with a preflight scan of the Interfaces blocks, and record `BASE` at Task 1. Keep `local/m10-sdd-resume-prompt.md` current whenever a subagent reports.
- **Order:** 1 → 2 → 3 → 4 → 5 → 6 (gate) → [7] → 8 → 9 → 10 (gate at Step 0) → 11 (gates) → 12 (gate). Tasks 1-2 and Task 5 touch disjoint code, but run them sequentially anyway: every task's check list runs the whole suite.
- **Gates (stop and ask; ask everything open in one message):**
  - Task 6 Step 7;
  - Task 10 Step 0;
  - Task 11 Steps 2 and 3;
  - Task 12 Step 1;
  - a release `boot_progress` over 90 s;
  - any override of a recorded decision;
  - any hardware-compared frame or Milestone 9 pin that moves (other than the two replaced rungs);
  - any new dependency.
- **Implementer brief:**
  - this plan's Global Constraints and Review Focus, the full task text, the Interfaces it consumes, the spec path (especially "Measured on v392"), the ledger lines that matter, and the worktree path;
  - paths only for anything under `local/`.
- **Browser work:** Chrome through the chrome-devtools MCP is the orchestrator's alone (subagents lack it). Implementers stop at "full check list green" and report; the orchestrator runs the smoke tests.
- **Reviewer checks (on top of the skill's):**
  - no ESP32-C3 or v392 constant in `cpu/` or `clock.rs`;
  - `emulator-wasm` logic-free;
  - no pin changed against `BASE` except the replaced rungs, each with a "Replaced rungs" row;
  - every hook address cited with evidence;
  - `iceServers: []` everywhere, and no server URL other than `/api/signal`;
  - no `local/` data, PNGs or scratch probes committed (`git show --stat`).
- **Finish:** copy everything new under the worktree's `local/` to the repo-root `local/` before removing the worktree.

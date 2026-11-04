# Milestone 11: The Backlog Sweep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear the Milestone 11 backlog. The work has four strands:
- make the emulated reboot match the badge's (time, the RTC, what a CPU reset keeps);
- explain two untraced step gaps;
- fix every open core and browser review item;
- replace hand-written code where a library does better.

Each new dependency is named to the human partner.

**Architecture:** Most core work is in `FirmwareRuntime::soft_reset` (`emulator-core/src/runtime.rs`) and the peripherals it carries. `cpu/` and `clock.rs` stay chip-agnostic. The browser work spans four areas:
- the Worker's pacing and status;
- the radio hub;
- the signaling function, moved to `@upstash/redis` and `@upstash/ratelimit`, with `valibot` validating input;
- the shell.

Every swap of hand-written code for a library preserves behaviour.

**Tech Stack:** Rust (`emulator-core`, `emulator-wasm`), wasm-pack, Bun/TypeScript (`frontend/`), Vercel Functions, Upstash Redis. New: `md-5`, `proptest` (dev), `object` (dev), `@upstash/redis`, `@upstash/ratelimit`, `valibot`, `happy-dom` (dev).

**Spec:** `docs/superpowers/specs/2026-11-03-milestone11-backlog-sweep-design.md` (approved 2026-11-04). Read "Measured (2026-11-03)" before Tasks 1–3, and Part 7's tables before any dependency swap.

## Global Constraints

- **Pins.** Only Tasks 2 and 3 may move pins, and only rungs that run after a soft reset. Every move is recorded in `docs/milestone-11-decisions.md` under "Moved pins" (rung | constant | before | after | cause). Every other task leaves every pin byte-identical against the commit before it. These never move:
  - `execution_digest_is_unchanged`;
  - `hacker_walk_totals_are_unchanged`;
  - any rung with `soft_reset_count() == 0`;
  - the hardware-compared frames: the registered screen, onboarding, the launcher and the launcher after DOWN, Dice open/roll/animation, and Snake open/after-turn/game over.

  A move outside these rules is a bug in the change, never a re-pin.
- **`REBOOT_US = 466_000`** (spec, "Measured"). Both reset kinds take it. It moves host time (`emulated_time_us`) and the RTC timer only. In the new boot, the CPU's cycle counter and SYSTIMER start at 0. A cold `reset()` still starts at 0 µs.
- **Reset behaviour.** Every peripheral kept or reset across a CPU reset is cited from the ESP32-C3 Technical Reference Manual or ESP-IDF v5.5.3 source. Where both are silent, the peripheral is reset (as today) and the gap is recorded.
- **The core's structural rules:**
  - `emulator-core/src/cpu/` and `clock.rs` hold no ESP32-C3 or v392 constant;
  - `FirmwareBus` keeps ordered, concrete named fields with no `dyn`, and the word fast path keeps its order;
  - `emulator-core` has no wasm-bindgen dependency;
  - `emulator-wasm` stays one-line passthroughs, and `framebuffer()` returns an owned `Vec`.
- **Input safety.** Unmapped data access never panics, and an unmapped instruction fetch always traps. Browser, peer and network input never panic the emulator, and a malformed peer packet never crashes a tab. A `valibot` failure is a 400 or a drop, never a throw past the boundary.
- **Sources.** Register layouts come from ESP-IDF **v5.5.3** headers, ROM addresses from its linker scripts, and app-image addresses from cited disassembly. Never guess. Fetch sources into the session scratchpad, one plain `curl -sfL -o <scratchpad>/<file> <url>` per file, never into the repo.
- **Servers.** Only `/api/signal` (Vercel) and its Upstash store (`KV_REST_API_URL` / `KV_REST_API_TOKEN`). No relay, no STUN/TURN (`iceServers: []`), no other third-party service. The dev server and `bun test` run with no Upstash credentials.
- **Dependencies are allowed** (the human partner, 2026-11-03). Each one added is reported in the implementer's report (package, exact version, why) and recorded in the decisions doc's "Dependencies added". The orchestrator names it to the human partner in its next message. Pin exact versions in `Cargo.toml` and `package.json`.
- **Rulings** go in `docs/milestone-11-decisions.md` with their task's commit, as `Ruling R-T<n>-<k>: <decision> — cost if wrong: <...>`. The Milestones 3 to 10 decisions stand, except R-T10-1 (bump ungated) and R-T10-3's "imports nothing", which the spec revisits by the human partner's choice.
- **Data handling:**
  - never commit, quote or print anything from `local/full_flash_dump.bin`, `local/boot_log.txt`, `local/m6-badge/` or `local/m11-badge/`; give paths only;
  - committed tests use only `firmware.bin` and the committed fake identities;
  - the fake MACs are `02:00:00:00:10:01` (hacker) and `02:00:00:00:10:02` (organizer).
- **Tools:**
  - use `/usr/bin/grep`;
  - use `/Users/powersagitar/Developer/powersagitar/hack-the-badge/local/.venv/bin/python -I` for any Python, never the system `python3`;
  - use Bun, never npm or node;
  - scratch probes (copies of `boot_progress.rs` with an ignored probe test) are deleted before committing.
- **The full check list** before each commit that touches code, all green; clippy stays at its 3 known warnings, and the release `boot_progress` suite stays under 90 s, with its time recorded:
  ```
  cargo fmt --all && cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck && bun run build
  ```
- **Commits** end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` only. Pushing, merging and deploying are the human partner's call.

## Review Focus

1. **A provision typed across a reboot.** Serial input is queued, then the firmware soft-resets (CPU reset) before reading it all. The rest arrives in the new boot's console, in order, exactly once, and the provisioner does not match the old boot's `badge> `. Pinned by Task 3's `queued_serial_input_survives_a_cpu_reset`, and Task 13's `provisioner_ignores_a_prompt_from_before_the_provision`.
2. **`run_until` with a target inside the reboot's jump.** It returns at the reset, with time past the target, and never goes backwards. The next `run_until` from there runs normally. Pinned by Task 2's `run_until_target_inside_the_reboot_jump`.
3. **A tap held when the firmware reboots** (A on Scanner is held while it reboots). The hold lasts its full `PRESS_HOLD_US` in wall terms. The release is not lost, and the button is not stuck down in the new boot. Pinned by Task 8's `tap_held_across_a_reboot_keeps_its_hold`.
4. **A signaling store that fails mid-pairing** (quota spent, network error) on any call:
  - the limiter's;
  - `create`;
  - `poll`;
  - `answer`;
  - `close`.

  The server answers a bare 500. The client shows "Signaling unavailable" and stops polling: no unhandled rejection, no endless loop. Pinned by Task 9's `store_failure_in_each_op_is_a_bare_500`, and Task 10's `client_stops_on_500`.
5. **Two quick soft resets in a row** (a reboot into an app that reboots again). Each one adds `REBOOT_US` once, the RTC offset accumulates, and the diagnostics markers count 2. Pinned by Task 2's `two_soft_resets_accumulate`.

---

## File Structure

- `emulator-core/src/runtime.rs`: `soft_reset` takes the reset kind. It also adds `REBOOT_US`, carries the RTC offset, rewrites `STORE4`, carries the diagnostics, and keeps the CPU-reset peripherals (Tasks 2, 3). It gets the test hook id fix (Task 5) and the proptest serial fuzz (Task 6).
- `emulator-core/src/rom.rs`: `REBOOT_US`, cited (Task 2). The reset stubs carry their kind (Task 3).
- `emulator-core/src/cpu/rom_stubs.rs`: `RomStubEffect::RequestReset` gains the kind (Task 3). `cpu/` takes no chip constant: the kind is a generic enum.
- `emulator-core/src/peripherals/rtc_cntl.rs`: the RTC offset (Task 2).
- `emulator-core/src/mem/bus.rs`, `cpu/mod.rs`: carrying the diagnostics across a reset (Task 2).
- `emulator-core/src/peripherals/*.rs`: whatever Task 1's table says a CPU reset keeps (Task 3).
- `emulator-core/src/radio/{mod.rs,hci.rs}`, `peripherals/efuse.rs`, `cpu/app_hooks.rs`: the review items (Tasks 5, 6).
- `emulator-core/src/md5.rs`: rounds through `md-5` (Task 7). `emulator-core/tests/riscv_integration.rs`: an ELF loader through `object` (Task 7).
- `emulator-core/tests/boot_progress.rs`: the moved pins (Tasks 2, 3), the header fix and the release-gap rung (Task 5).
- `emulator-wasm/src/lib.rs`: `reboot_us()` (Task 8).
- `frontend/src/runtime/firmware-runtime.ts`, `firmware-worker.ts`, `firmware-worker-protocol.ts`: the reboot's pause, and status on a reset (Task 8); advertisement coalescing and `macLocked` (Task 11); the tap queue (Task 13).
- `frontend/src/radio/signal-handler.ts`, `frontend/api/signal.ts`, `frontend/vite.config.ts`: Upstash clients, the limiter, `valibot`, `close`, the IPv6 /64 key, and the dev middleware (Task 9).
- `frontend/src/radio/signal-client.ts`, `peer-link.ts`: `close`, the 429 and 500 messages, and `waitOpen` on the event (Task 10).
- `frontend/src/radio/hub.ts`: `valibot` for peer messages, and the bump gate (Task 11).
- `frontend/src/main.ts`, `frontend/src/ui/shell.ts`, `frontend/test/shell.test.ts`: the lock release, Provision without the badge, and `happy-dom` (Task 12).
- `frontend/src/runtime/provisioner.ts`: the prompt scan (Task 13).
- `docs/milestone-11-decisions.md`: created in Task 1 and grown by every task. `docs/firmware-emulator-notes.md` and `CLAUDE.md`: Task 14.

---

### Task 1: The decisions doc, `BASE`, and the CPU-reset table (Gate 1)

**Files:**
- Create: `docs/milestone-11-decisions.md`

**Interfaces:**
- Produces:
  - `BASE` (this task's commit), recorded in the ledger;
  - the table "What a CPU reset keeps" (peripheral | kept or reset | citation | reproduced how), which Task 3 implements row by row.

- [ ] **Step 1: Create the decisions doc.** Its sections:
  - "Why";
  - "Measurements": copy the spec's "Measured (2026-11-03)";
  - "What a CPU reset keeps";
  - "Moved pins";
  - "Traces";
  - "Dependencies added";
  - "Closed in writing": every spec item closed without work, each citing who decided it (the spec's "Decided by the human partner");
  - "Rulings";
  - "Milestone 12 backlog".
- [ ] **Step 2: Baselines.** Record `bun run build`'s output sizes (every `dist/assets` file, in bytes) and `bun test`'s count, for Part 7's before-and-after.
- [ ] **Step 3: Fetch the evidence** into the scratchpad:
  - ESP-IDF v5.5.3's `components/esp_system/port/soc/esp32c3/system_internal.c` and `components/esp_system/port/cpu_start.c`;
  - its `components/bootloader_support/src/esp32c3/bootloader_esp32c3.c` and `bootloader_support/src/bootloader_clock_init.c`;
  - its `components/soc/esp32c3/register/soc/system_reg.h`;
  - the ESP32-C3 TRM PDF (`https://www.espressif.com/sites/default/files/documentation/esp32-c3_technical_reference_manual_en.pdf`), read through its "Reset and Clock" chapter.
- [ ] **Step 4: Write the table.** One row per modeled peripheral:
  - SYSTIMER, INTC, SYSTEM, APB_CTRL, GPIO and the 74HC165;
  - SPI2 and the ST7789, GDMA, TIMG, RTC_CNTL, USB-Serial-JTAG;
  - I2C0 and the SC7A20H, RMT, eFuse, the flash MMU, SPI1 and the flash.

  Each row has three parts:
  - the TRM's reset scope for a CPU reset (`RTC_SW_CPU_RST`) and for a core reset (`RTC_SW_SYS_RST`);
  - what `esp_restart_noos` and the ROM or 2nd-stage bootloader reset or rewrite in it (guest code that runs on the badge but not in the shortcut boot);
  - the emulator's choice: "kept", "reset", or "kept, with these registers rewritten as the bootloader does".

  Mark "silent → reset" where nothing says.
- [ ] **Step 5: Commit** `docs: Milestone 11 decisions doc and the CPU-reset table`.
- [ ] **Step 6: GATE 1.** The orchestrator shows the human partner the table and its citations, and waits.

### Task 2: The reboot takes time; the RTC keeps counting; `STORE4`; diagnostics carry

**Files:**
- Modify: `emulator-core/src/runtime.rs` (`soft_reset`, `reset`, `run_impl`, module doc "Software reset"), `emulator-core/src/rom.rs`, `emulator-core/src/peripherals/rtc_cntl.rs`, `emulator-core/src/mem/bus.rs`, `emulator-core/src/cpu/mod.rs`, `emulator-core/tests/boot_progress.rs`

**Interfaces:**
- Produces:
  - `pub const REBOOT_US: u64 = 466_000;` in `rom.rs`. Its doc cites the spec's measurement and `local/m11-badge/reboot_log_{1,2}.txt` by path.
  - `RtcCntl::rtc_ticks_at(&self, elapsed_ticks: u64) -> u64` (offset included). `RtcCntl::set_rtc_offset_ticks(&mut self, ticks: u64)` and `RtcCntl::rtc_offset_ticks(&self) -> u64`.
  - `FirmwareBus::unmapped_log` and `Cpu::rom_stub_index_drops` carry across a reset. The log gains a reset marker: `FirmwareBus::unmapped_log_resets(&self) -> u32`, or a marker entry; the builder picks, recorded as a ruling.

- [ ] **Step 1: Write the failing tests** (`runtime.rs` tests, on `synthetic_image()`):
  - `soft_reset_adds_reboot_time`: `emulated_time_us()` after the reset step equals before, plus that step's own time, plus `REBOOT_US`. `total_steps` grows by exactly the steps run.
  - `rtc_timer_continues_across_a_soft_reset`: latch the RTC timer (the `TIME_UPDATE` write and the `TIME_LOW0`/`HIGH0` reads, as `rtc_cntl_ll_get_rtc_time` does) before the reset and right after it. After ≥ before + `REBOOT_US × 136_000 / 1_000_000`, within one tick.
  - `cold_reset_clears_the_rtc_offset`: after `reset()`, the first latch is under 1,000 ticks.
  - `store4_is_the_bootloaders_after_a_soft_reset`: the guest writes `0xdead_beef` to `STORE4`, then resets, and `STORE4` reads `crate::rom::RTC_XTAL_FREQ_REG`'s seeded value. `STORE0..3` and `STORE5..7` keep the guest's values.
  - `diagnostics_carry_across_a_soft_reset`: an unmapped read before the reset is still in `unmapped_log()` after it, and the reset marker counts 1.
  - `run_until_target_inside_the_reboot_jump` (Review Focus 2).
  - `two_soft_resets_accumulate` (Review Focus 5).
- [ ] **Step 2: Run them.** `cargo test -p emulator-core --lib`. Expected: the seven fail.
- [ ] **Step 3: Implement.**
  - `soft_reset` adds `REBOOT_US` to `time_base_us` after the existing `elapsed_us` carry.
  - It sets the new `RtcCntl`'s offset to the old timer's value at the reset, plus `REBOOT_US` in RC_SLOW ticks (`REBOOT_US * RC_SLOW_HZ / 1_000_000`).
  - It rewrites `STORE4` after `set_store_regs`, with the value `boot.rs` seeds.
  - It moves the old diagnostics in.
  - `bus.rs` passes the RTC offset through its existing `rtc_cntl.write_byte(.., elapsed_ticks)` call, unchanged in shape (the offset lives in `RtcCntl`).
  - `run_impl` needs no change for the jump: `stop_at_ticks` is already re-derived after `attend` returns true. Check that `ticks_at_host_us` saturates when the target lies before the new clock's start.
- [ ] **Step 4: Run them.** Expected: they pass.
- [ ] **Step 5: Measure the moved pins.** Run the release `boot_progress`. For each failing rung, record in "Moved pins":
  - its constant;
  - its value before and after;
  - the cause ("after a soft reset: +466 ms host time" or "the RTC keeps counting").

  Only rungs that run after a soft reset may fail. A failure anywhere else is a bug: fix the change. A frame hash that moves is shown at Gate 2 with a PNG under `local/` (boot-probe's `--dump-frame`, or a scratch dump). Do not re-pin a frame hash before Gate 2.
- [ ] **Step 6: Re-pin the non-frame constants** that moved for the recorded cause (step bands, emulated times). Update the module doc's "Software reset" paragraph and the `rtc_cntl.rs` doc.
- [ ] **Step 7: Run the full check list.** Commit `feat: the reboot takes 466 ms and the RTC keeps counting`.

### Task 3: A CPU reset keeps what a CPU reset keeps

**Files:**
- Modify: `emulator-core/src/cpu/rom_stubs.rs` (`RomStubEffect::RequestReset`), `emulator-core/src/cpu/mod.rs` (`take_reset_request`), `emulator-core/src/rom.rs` (entry 26 doc, the two stubs), `emulator-core/src/runtime.rs` (`soft_reset`, `attend`), the peripheral modules that Task 1's table marks "kept", `emulator-core/tests/boot_progress.rs`

**Interfaces:**
- Consumes: Task 1's table; Task 2's `soft_reset`.
- Produces:
  - `pub enum ResetKind { Cpu, Core }` in `cpu/rom_stubs.rs`. Generic names, with no chip constant.
  - `RomStubEffect::RequestReset { reason: u32, kind: ResetKind }`. `Cpu::take_reset_request(&mut self) -> Option<(u32, ResetKind)>`.
  - `software_reset_cpu` gives `ResetKind::Cpu`, and `software_reset` gives `ResetKind::Core`.
  - `FirmwareRuntime::soft_reset(&mut self, reason: u32, kind: ResetKind)`.

- [ ] **Step 1: Write the failing tests** (`runtime.rs` tests):
  - `queued_serial_input_survives_a_cpu_reset` (Review Focus 1). Queue 100 bytes with `serial_input`, CPU-reset before the guest reads them, and `serial_pending()` is still 100.
  - `core_reset_rebuilds_every_peripheral`. Through `software_reset`, `serial_pending()` is 0, as today, and so on for one register per peripheral the table marks "kept".
  - One test per other "kept" row: `<peripheral>_survives_a_cpu_reset` writes a register, CPU-resets, and reads it back.
  - One per "rewritten" row: the register reads the bootloader's value.
- [ ] **Step 2: Run them.** Expected: they fail.
- [ ] **Step 3: Implement.** Thread the kind through. In `soft_reset`, for `ResetKind::Cpu`, move each "kept" peripheral from the old bus into the new one by its concrete named field, then apply the "rewritten" rows. `ResetKind::Core` stays today's path.
- [ ] **Step 4: Run them.** Expected: they pass.
- [ ] **Step 5: Measure the moved pins**, as Task 2 Step 5 does, with the cause "the CPU reset keeps <peripheral>". If a reboot app no longer reaches its open frame, or panics, stop and report with the console tail. That is a gate, not something to fix by un-keeping a peripheral without evidence.
- [ ] **Step 6: Re-pin the non-frame constants.** Update `rom.rs` entry 26's doc ("Both reset stubs are the same full re-boot" goes).
- [ ] **Step 7: Run the full check list.** Commit `feat: a CPU reset keeps what the badge's keeps`.
- [ ] **Step 8: GATE 2.** The orchestrator shows the human partner the "Moved pins" table from Tasks 2 and 3, and any moved frame as a PNG for an eyeball check. Frame hashes are re-pinned only after the human partner's OK, in a follow-up commit `test: re-pin <rung> after the reset-fidelity fixes`.

### Task 4: Two traces (Part 2)

**Files:**
- Modify: `docs/milestone-11-decisions.md` ("Traces"); code only if a trace finds an emulator fault.
- Test: a rung in `boot_progress.rs` if a fix lands; `frontend/test/provisioner.test.ts` for the pacing pin.

- [ ] **Step 1: Trace `PROV FAIL`'s 3.0M busy steps** on v392. From a provisioned checkpoint, type a `prov apply` that fails, as `tests/common/provision.rs` does with a bad identity. Then run boot-probe-style hot PCs over the 3.0M steps until the next `badge> `. Name the functions behind the top PCs from a disassembly of `firmware.bin`'s segments (`llvm-objdump`, scratchpad only). Record the result:
  - if the firmware's own work causes it (NVS, LittleFS, a retry loop with a delay), say so with addresses;
  - if an emulator effect causes it (a spin on an unmodeled register, or unmapped reads in the log), fix it with a rung pinning the new step count, and report: a fix that is more than a small register model is GATE 4.
- [ ] **Step 2: Trace the 740,001-step gap.**
  1. Build the frontend provisioner's input timeline: `frontend/src/runtime/provisioner.ts` driven by `firmware-runtime.ts`'s scheduling in a `bun test` scratch harness over the WASM handle, logging (emulated µs, bytes) per chunk and each press.
  2. Build the same timeline for `tests/common/provision.rs` in a scratch Rust probe.
  3. Diff them, and name the first difference.
  4. If one host is wrong against the firmware's console behaviour, fix it. Otherwise add `provisioner_pacing_matches_the_rust_host` in `frontend/test/provisioner.test.ts`, pinning the frontend's chunk schedule (chunk size 64, and its gaps) against the Rust host's documented one.
- [ ] **Step 3: Run the full check list** if code changed. Commit `docs: Milestone 11 traces` (with `fix:` if a fix landed).

### Task 5: Core review items

**Files:**
- Modify: `emulator-core/src/runtime.rs` (tests: `hooked_count_loop`), `emulator-core/src/radio/mod.rs`, `emulator-core/src/cpu/app_hooks.rs`, `emulator-core/src/peripherals/efuse.rs`, `emulator-core/src/mem/soc.rs` (tests), `emulator-core/tests/boot_progress.rs`, `frontend/test/cpu-wasm.test.ts`, `frontend/tsconfig.json`

- [ ] **Step 1: Write the tests:**
  - `runtime.rs`: `hooked_count_loop` uses hook id `0x7f00`, which the radio does not know. The runtime gets `#[cfg(test)] fn set_test_hook_dispatch(&mut self, f: fn(HookHit, &mut Cpu))`, through which the tests answer their own hits without the radio's `debug_assert`. The radio's real path is unchanged.
  - `radio/mod.rs`: `report_popped_in_send_packet_follows_the_reply`, covering R-T6-2. A command whose reply and a pending report are both due: the guest sees the reply first, and the report goes out through the idle-hook path.
  - `radio/mod.rs`: `host_rcv_pkt_does_not_block_the_idle_task`: the idle hook returns within a bounded step count while reports are queued.
  - `radio/mod.rs`: `deliver_rejects_malformed_bytes` builds its runtime with the guest scanning (extended scan enabled through the HCI model). A well-formed advertisement then arrives (positive control) and each malformed one does not.
  - `app_hooks.rs`: `set_app_hooks_drops_a_pending_pass_through`, `a_clone_carries_a_pending_hit` and `a_clone_carries_a_pass_through`. `hook_step`'s doc gains "a WFI-parked core returns before the hook check".
  - `soc.rs`: `efuse_range_is_disjoint_from_every_other_range`. `efuse.rs`'s doc cites the MAC builder `0x42010344` (`docs/milestone-10-decisions.md`, "hal_radio's init").
  - `cpu-wasm.test.ts`: `soft_reset_count` is 1 after a soft reset. Reach it with `serial_input("reboot\r\n")` from the console checkpoint the file already builds. Add `test/cpu-wasm.test.ts` to `tsconfig.json`'s `include`, and fix any type errors it surfaces.
  - `boot_progress.rs`:
    - the radio section header says the bump runs one blocking 100 ms slice (R-T8-3);
    - a new rung, `two_downs_a_release_gap_apart_move_twice`. From hacker's launcher, press DOWN for `PRESS_HOLD_US` (100 ms), release for `RELEASE_GAP_US` (40 ms; a test constant mirroring `firmware-runtime.ts`), press DOWN again, and run 1.5 s. The frame is the launcher with the cursor two cells down. Pin its hash; the launcher-after-two-DOWNs frame is not hardware-compared.
- [ ] **Step 2: Run them.** Expected: the new tests pass where they only add coverage, and fail where they need the change.
- [ ] **Step 3: Implement** whatever failed.
- [ ] **Step 4: Run the full check list.** No existing pin moves. Commit `test: Milestone 10's core review items and the release-gap rung`.

### Task 6: The HCI model's rules, and `proptest`

**Files:**
- Modify: `emulator-core/Cargo.toml` (dev-dependency `proptest`, exact version), `emulator-core/src/radio/hci.rs`, `emulator-core/src/runtime.rs` (the serial fuzz test)

- [ ] **Step 1: Fetch the Core spec's command descriptions** for LE Set Extended Advertising Enable and LE Set Extended Scan Enable (Bluetooth Core v5.4, Vol 4, Part E, §7.8.56 and §7.8.65) into the scratchpad. Cite the section numbers in `hci.rs`.
- [ ] **Step 2: Write the failing tests:**
  - `ext_adv_enable_rejects_duplicate_handles`: status `0x12`.
  - `ext_scan_enable_rejects_filter_duplicates_2_with_period_0`: status `0x12`.
- [ ] **Step 3: Replace both fuzz tests with `proptest`.** Remove `XorShift` (`hci.rs`) and the xorshift32 in `runtime.rs`.
  - The HCI one generates whole H4 packets, with weighted strategies that reach the report path: scanning is enabled, then advertisements are delivered with every address byte drawn.
  - The serial one generates byte vectors of length 0 to 299.

  Both keep their old invariants (no panic, a bounded queue). `ProptestConfig` sets a fixed `cases` and `failure_persistence: None`, so runs stay deterministic in CI-free use. Record the case count, and the test time native and debug.
- [ ] **Step 4: Implement the two rules.** Run `cargo test -p emulator-core`. Expected: all pass.
- [ ] **Step 5: Run the full check list.** No radio pin moves. If one does, NimBLE sends one of those forms: stop and report. Commit `feat: HCI parameter checks; proptest for the fuzz tests`.

### Task 7: `md-5` and `object`

**Files:**
- Modify: `emulator-core/Cargo.toml` (`md-5`, exact version, `default-features = false`; dev-dependency `object`, exact version, the `read` features only), `emulator-core/src/md5.rs`, `emulator-core/tests/riscv_integration.rs`

- [ ] **Step 1: The MD5 rounds through `md-5`.** `Md5Context`'s block processing calls `md5::compress(&mut self.buf, &[block])`, which the RustCrypto crate exposes. Everything else stays: the Plumb context layout, `bits`, `in`, and the byte-exact writes. Delete the hand-written round functions and their constants. The existing tests (RFC 1321's test suite, the ROM-stub tests, `PARTITION_TABLE_MD5`) must pass unchanged. If `md-5`'s published version does not expose `compress`, revert the change and record it under "Keep".
- [ ] **Step 2: Load the fixtures' ELF with `object`.** Copy each `PT_LOAD` segment to its physical address. Remove `find_objcopy`, the objcopy invocation, and the `llvm-tools-preview` line in the module doc. The tests' expectations stay unchanged.
- [ ] **Step 3: Check that it builds for wasm.** `bun run build:wasm` succeeds with `md-5` in `emulator-core`. Record the WASM size before and after.
- [ ] **Step 4: Run the full check list.** Every pin is unchanged. Commit `refactor: md-5 for the MD5 rounds; object for the RISC-V fixtures`.

### Task 8: The reboot's pause in the browser; status on a reset

**Files:**
- Modify: `emulator-wasm/src/lib.rs`, `frontend/src/cpu/bridge.ts` (its types), `frontend/src/runtime/firmware-runtime.ts`, `frontend/src/runtime/firmware-worker.ts`, `frontend/src/runtime/firmware-worker-protocol.ts`
- Test: `frontend/test/firmware-runtime.test.ts`, `frontend/test/firmware-worker.test.ts`

**Interfaces:**
- Consumes: Task 2's `REBOOT_US`; the existing `soft_reset_count()` passthrough.
- Produces:
  - `FirmwareEmulator::reboot_us() -> u64` (wasm, a one-line passthrough of `emulator_core::rom::REBOOT_US`, read by TS as a `bigint` or a `number`, matching the file's convention);
  - `StepResult` (the return of `stepFirmwareFrame`) gains `rebootUs: number`;
  - the `status` message gains `softResets: number`.

- [ ] **Step 1: Write the failing tests:**
  - `firmware-runtime.test.ts`: `reboot_is_paced_as_wall_time`. In a fake emulator, `soft_reset_count` steps from 0 to 1 within a frame and emulated time jumps by `REBOOT_US`. The next frames advance no emulated time until `REBOOT_US / 1000` ms of wall time have passed (no `runUntil` call moves time). The speed reading for the jump frame leaves the jump out.
  - `tap_held_across_a_reboot_keeps_its_hold` (Review Focus 3). A press at time t, then a reboot jump during the hold: the release fires once `PRESS_HOLD_US` of non-jump time has passed.
  - `firmware-worker.test.ts`: `status_is_posted_at_once_on_a_soft_reset`. A reset inside the throttle window posts a `status` with `softResets: 1` immediately.
- [ ] **Step 2: Run them.** Expected: they fail.
- [ ] **Step 3: Implement.**
  - `firmware-runtime.ts` compares `soft_reset_count()` before and after each slice. On an increase it adds `n × reboot_us()` to a `rebootDebtUs` that `frameTargetUs`'s wall delta pays down before any new emulated time is run.
  - The tap queue's `downSince` and `upSince` are shifted forward by the jump, so holds and gaps count only non-jump time.
  - The speed reading subtracts the jump.
  - The Worker posts `status` outside the throttle when `softResets` changes.
- [ ] **Step 4: Run the full check list** (`bun run build:wasm` first). Commit `feat: the reboot's pause shows in the browser`.

### Task 9: The signaling function: Upstash clients, the limiter, `valibot`, `close`

**Files:**
- Modify: `frontend/package.json` (`@upstash/redis`, `@upstash/ratelimit`, `valibot`, exact versions), `frontend/src/radio/signal-handler.ts`, `frontend/api/signal.ts`, `frontend/vite.config.ts`
- Test: `frontend/test/signal-handler.test.ts`

**Interfaces:**
- Produces:
  - `interface RoomStore { set(key, value, ttlSeconds, onlyIfAbsent): Promise<boolean>; get(key): Promise<string | null>; del(keys: string[]): Promise<void> }`, with two implementations: `MemoryStore` (dev, tests) and `redisRoomStore(redis: Redis)`.
  - `interface Limiter { limit(key: string): Promise<{ success: boolean; reset: number }> }`, with `upstashLimiter(redis, tokens, windowSeconds, prefix)` over `Ratelimit.fixedWindow`, and `memoryLimiter(tokens, windowSeconds, now)`.
  - `handleSignal(req, deps: { rooms: RoomStore; limiters: { client; roomPolls; clientWrites; global } })`. The four caps and windows stay exactly as today (`RATE_*` constants).
  - The new op: `POST {op:"close", room, token}` gives 204, or 404 for an unknown room or token. `create` now answers `{room, token}`, where `token` is 22 base64url chars from `crypto.getRandomValues`, stored with the offer (key `htn-signal:<room>:token`, same TTL).
  - `clientKey(req): string`: the first `x-forwarded-for` entry. For IPv6 it is the address's /64 prefix (the first four hextets, expanded); for IPv4 the whole address.

- [ ] **Step 1: Write the failing tests:**
  - `close_deletes_the_room` (then a poll is 404).
  - `close_with_a_wrong_token_is_404_and_keeps_the_room`.
  - `create_returns_a_token`.
  - `ipv6_clients_share_a_64_prefix_key` (two addresses in one /64 count against one key; one in another /64 does not).
  - `store_failure_in_each_op_is_a_bare_500` (Review Focus 4: a store or limiter that throws on each of create, poll, answer and close gives 500 with an empty body).
  - `valibot_rejections_are_400`: every existing bad-request case, plus a `close` without a token.

  Every existing test keeps passing against `MemoryStore` and `memoryLimiter`. 429 keeps `Retry-After` from the limiter's `reset`.
- [ ] **Step 2: Run them.** Expected: they fail.
- [ ] **Step 3: Implement.**
  - `api/signal.ts` builds `Redis.fromEnv()`. A missing env var is still a bare 500, caught before building it.
  - `vite.config.ts` imports `./src/radio/signal-handler.ts` with its extension, and its middleware stops reading at `MAX_BODY_BYTES` (`req.destroy()` after the 400).
  - Request bodies are parsed with `valibot` schemas (`v.safeParse`); a failure is 400.
  - Nothing is logged, ever (unchanged).
- [ ] **Step 4: Measure the command cost.** With a scratch fake `fetch` under `@upstash/redis`, count the REST commands for one request of each kind. Record per request and per typical pairing, and the free month's exhaustion time at the global cap, in "Measurements". Cite Upstash's docs on how a script call (`EVALSHA`) is billed.
- [ ] **Step 5: Run the full check list.** Record the bundle and function sizes. Commit `feat: signaling on @upstash/ratelimit, valibot and a close op`.

### Task 10: The signaling client and the peer link

**Files:**
- Modify: `frontend/src/radio/signal-client.ts`, `frontend/src/radio/peer-link.ts`, `frontend/src/main.ts` (the cancel wiring), `frontend/src/ui/shell.ts` (message text)
- Test: `frontend/test/signal-client.test.ts`, `frontend/test/peer-link.test.ts`

**Interfaces:**
- Consumes: Task 9's `close` op and `{room, token}`.
- Produces:
  - `PeerLink.opened: Promise<void>`, which resolves on the data channel's `open` and rejects on `close` before open;
  - `hostRoom` keeps its token, and its cancel sends `close` (best effort, errors swallowed);
  - `SignalError` messages:
    - `"Signaling server busy, try again in N min."` (N = ceil(Retry-After / 60), at least 1);
    - `"Signaling unavailable."` (5xx);
    - `"Room closed or expired."` (404 on join).

- [ ] **Step 1: Write the failing tests:**
  - `429_maps_to_busy_with_minutes`, for Retry-After 1, 60 and 600.
  - `client_stops_on_500` (Review Focus 4): no further `fetch` after the 500.
  - `cancel_sends_close_with_the_token`.
  - `join_of_a_closed_room_says_closed`.
  - `waitOpen_resolves_on_the_open_event`: a link whose channel opens resolves without a sleep elapsing (the fake `sleep` is never called).
- [ ] **Step 2: Run them.** Expected: they fail.
- [ ] **Step 3: Implement.** `waitOpen` races `link.opened` against the deadline and cancellation, with no polling loop.
- [ ] **Step 4: Run the full check list.** Commit `feat: signaling client messages, close on cancel, open on the event`.

### Task 11: The hub and the Worker's radio

**Files:**
- Modify: `frontend/src/radio/hub.ts`, `frontend/src/runtime/firmware-worker.ts`, `frontend/src/main.ts`
- Test: `frontend/test/radio-hub.test.ts`, `frontend/test/firmware-worker.test.ts`

**Interfaces:**
- Produces:
  - `Hub.setRunning(running: boolean)`;
  - `hub.bump()` sends nothing and returns `false` while not running;
  - the peer-message parser is a `valibot` schema (`{t:"adv", b: base64 ≤ MAX_BASE64_CHARS}` | `{t:"bump"}`), with the same drops as today.

- [ ] **Step 1: Write the failing tests:**
  - `bump_while_stopped_sends_nothing`;
  - `bump_while_hidden_sends_nothing`;
  - `changed_advertisements_are_coalesced_to_10_hz`: 20 changes in 100 ms post at most 2, the last one being the latest;
  - `mac_locked_only_when_set_factory_mac_succeeds`;
  - `hub_drops_malformed_peer_messages`, which keeps passing through the schema.
- [ ] **Step 2: Run them.** Expected: the new ones fail.
- [ ] **Step 3: Implement.**
  - `main.ts` calls `hub.setRunning` from the runtime's start, stop and visibility path.
  - The Worker keeps the latest changed advertisement and posts it on the `ADV_REPOST_MS` tick instead of at once.
  - Record R-T10-1's revision as a ruling, citing the spec.
- [ ] **Step 4: Run the full check list.** Commit `fix: gate the bump on the running badge; coalesce advertisements`.

### Task 12: The shell: the lock, Provision without the badge, `happy-dom`

**Files:**
- Modify: `frontend/package.json` (dev `happy-dom`, exact version), `frontend/src/main.ts`, `frontend/src/ui/shell.ts`
- Test: `frontend/test/shell.test.ts`

- [ ] **Step 1: Move `shell.test.ts` onto `happy-dom`.** In `beforeEach`, set `globalThis.document` to a fresh `new Window().document` with the page's skeleton (`index.html`'s body ids). Restore the previous value in `afterEach`. Delete `FakeElement` and `fakeDoc`. Do not use the global registrator. "Shows a room code" now drives the shell's real update path and reads the DOM.
- [ ] **Step 2: Write the failing tests** (`shell.test.ts`, with the wiring factored from `main.ts` into a testable `createBadgeController(deps)` in a new `frontend/src/ui/badge-controller.ts` if `main.ts` cannot be driven directly):
  - `switching_to_lua_releases_the_badge`;
  - `a_failed_firmware_start_releases_the_badge`;
  - `provision_without_the_badge_boots_no_worker_and_shows_the_notice`;
  - `notice_clears_the_canvas`.
- [ ] **Step 3: Run them.** Expected: they fail.
- [ ] **Step 4: Implement.**
  - `setMode("lua")` and `startFirmware`'s failure path call `badgeHold.release()` and null it.
  - Provision checks `claimBadge()` before `ensureFirmwareRuntime()`.
  - The notice clears the canvas with a fill in the page's background colour.
- [ ] **Step 5: Run the full check list.** Commit `fix: release the badge on Lua mode or a failed start; happy-dom for the shell tests`.

### Task 13: The tap queue and the provisioner

**Files:**
- Modify: `frontend/src/runtime/firmware-runtime.ts`, `frontend/src/runtime/provisioner.ts`
- Test: `frontend/test/firmware-runtime.test.ts`, `frontend/test/provisioner.test.ts`

- [ ] **Step 1: Write the failing tests:**
  - `pad_release_does_not_release_a_key_held_button`: a per-source hold count.
    - `injectButton` gains an optional `source: "key" | "pad"`, defaulting to `"pad"`.
    - `shell.ts` passes `"key"` from `mountKeyboard`.
    - The button is released when every source has let go.
  - `queued_tap_survives_stop_and_plays_on_start`: the tap and its later release are honoured.
  - `provisioner_ignores_a_prompt_from_before_the_provision` (Review Focus 1). The console already holds `badge> ` when `provision()` starts, and the provisioner waits for a new one. It records the console length at start, and scans only after it.
- [ ] **Step 2: Run them.** Expected: they fail.
- [ ] **Step 3: Implement.** `RELEASE_GAP_US` stays 40,000 (R-M8-4).
- [ ] **Step 4: Run the full check list.** Commit `fix: tap sources, taps across stop, and the provisioner's prompt scan`.

### Task 14: Gate 3, the docs, and the final review

- [ ] **Step 1: The orchestrator's smoke tests in Chrome** (chrome-devtools MCP), on `bun run dev`:
  - open Scanner and see the pause;
  - force a 429: send 31 `create` writes within 60 s from the page's console, against the dev middleware's in-memory limiter (the per-client write cap is 30 per 60 s);
  - host a room, cancel it, join with its code in a second tab: "Room closed or expired.";
  - switch to Lua mode and check a second tab can take the badge.

  Stop the dev server afterwards.
- [ ] **Step 2: GATE 3.** Ask the human partner, in one message, to check the same four things on their machine. Also ask them to re-deploy a Vercel preview, since R-T10-3's "imports nothing" ended and the function now bundles `@upstash/*` and `valibot`. They check:
  - a 400 on a bad request;
  - a pairing;
  - the 429 message.

  Record the verdicts.
- [ ] **Step 3: Update the notes.** In `docs/firmware-emulator-notes.md`:
  - "Timing model": `REBOOT_US`, the RTC offset, and the browser's pause;
  - "Known limitations": the CPU-reset table's silent rows, and the cold boot at 0 µs;
  - a "Milestone 11" section.
- [ ] **Step 4: Update `CLAUDE.md`.**
  - "Current state" (the reboot's time).
  - The test counts and times.
  - Clippy's warning count.
  - The architecture map:
    - `runtime.rs` (reset kinds, what a reset keeps);
    - `rtc_cntl.rs` (the offset);
    - `signal-handler.ts` (Upstash clients, `close`, `valibot`);
    - `md5.rs` (`md-5`).
  - Drop the `llvm-tools-preview` requirement if Task 7 removed it.
- [ ] **Step 5: Finish the decisions doc.**
  - Every `Ruling:` line, with its cost if wrong.
  - "Dependencies added": each with its version, why, and the size cost.
  - The final suites.
  - The Milestone 12 backlog:
    - Share's radio transfer;
    - Sync's empty-room path;
    - a cold boot's 466 ms;
    - a joiner who answered before a cancel waits out `waitOpen`;
    - the CPU-reset table's silent rows;
    - anything else this milestone leaves open.
- [ ] **Step 6: Run the full check list.** Commit `docs: Milestone 11 notes, CLAUDE.md and decisions`.
- [ ] **Step 7: Whole-branch review** (opus), then superpowers:finishing-a-development-branch. List every `Ruling:` line under "Rulings I made", each with its cost if wrong.

## Orchestrator notes

- **Execution method:** chosen by the human partner. The Milestones 6, 7, 9 and 10 convention: subagent-driven with opus implementers and reviewers, sonnet scoped re-reviews of fix rounds and docs reviews, and an opus whole-branch review.
- **Ledger:** `local/m11-sdd-ledger.md` in the worktree (it exists). One line per dispatch, verdict, fix round, ruling, measurement, dependency added and human answer. Record `BASE` at Task 1. Keep `local/m11-sdd-resume-prompt.md` current whenever a subagent reports.
- **Order:** 1 (Gate 1) → 2 → 3 (Gate 2) → 4 (Gate 4 if needed) → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12 → 13 → 14 (Gate 3). Run them sequentially: every task's check list runs the whole suite.
- **Gates (stop and ask; ask everything open in one message):**
  - Task 1 Step 6;
  - Task 3 Step 8;
  - Task 4 (a fix bigger than a small register model);
  - Task 14 Step 2.

  Plus:
  - a pin moving outside Tasks 2 and 3, or for an unrecorded cause;
  - any hardware-compared frame moving;
  - a release `boot_progress` over 90 s;
  - any override of a recorded decision;
  - any deploy, push or merge;
  - anything needing the badge.
- **Dependencies:** not a gate. After each task that adds one, the orchestrator's next message to the human partner names it (package, version, why).
- **Implementer brief:** this plan's Global Constraints and Review Focus, the full task text, the Interfaces it consumes, the spec path, the ledger lines that matter, and the worktree path. For anything under `local/`, paths only. Tell every subagent to use `local/.venv/bin/python -I` and never the system `python3`.
- **Browser work:** Chrome through the chrome-devtools MCP is the orchestrator's alone. Implementers stop at "full check list green" and report.
- **Reviewer checks (on top of the skill's):**
  - no chip constant in `cpu/` or `clock.rs` (`ResetKind` is generic);
  - `emulator-wasm` logic-free;
  - every moved pin has a "Moved pins" row, and only Tasks 2 and 3 move any;
  - every kept peripheral cited;
  - `iceServers: []` everywhere, and no server URL other than `/api/signal`;
  - exact dependency versions;
  - no `local/` data, PNGs or scratch probes committed (`git show --stat`).
- **Finish:** copy everything new under the worktree's `local/` to the repo-root `local/` before removing the worktree.

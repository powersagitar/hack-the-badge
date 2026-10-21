# Milestone 7 — Core Throughput and a Web Worker Host — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make busy screens (launcher HOME and DOWN, onboarding page turns) play at the physical badge's speed in Chrome and Safari, with input that never waits behind a frame, without changing anything the firmware can observe.

**Architecture:** A committed benchmark (native `#[ignore]` rung plus one TypeScript bench run under Bun, in Chrome and in Safari) and two new equivalence rungs come first and pin the baseline at `6cd0a6e`. Then the core gets faster in measured steps: `opt-level = 3`, a cheaper interrupt path, a word-sized bus fast path, a cheap ROM-stub precheck and a decoded-instruction cache. Each step keeps every pin byte-identical. Last, the browser host moves the emulator into a dedicated Web Worker that posts frames to the main thread.

**Tech Stack:** Rust 2021 (`emulator-core`, `emulator-wasm` via wasm-bindgen/wasm-pack), Bun + TypeScript + Vite frontend, samply 0.13.1 for profiles (installed in the session scratchpad, not the repo), `safaridriver` (enabled by the human partner) and the Claude-in-Chrome extension for browser runs.

**Spec:** `docs/superpowers/specs/2026-10-21-milestone7-core-throughput-design.md`. Read it first, including "Measured baseline". Also read `docs/milestone-6-decisions.md` (its Rulings, and its Milestone 7 backlog, which this milestone carries forward unchanged except for the throughput item), and `docs/firmware-emulator-notes.md` "Timing model", "Known limitations" and "Data-handling note".

## Handoff state (as of 2026-10-21)

- Worktree `.claude/worktrees/milestone-7` on branch `milestone-7`, from `main` @ `6cd0a6e` (pushed; `origin/main` = `6cd0a6e`). The spec is committed there (`5e64ce2`); this plan is the next commit. The repo-root `local/` is copied in; `bun install` and `bun run build:wasm` are done.
- Orchestrator ledger: `local/m7-sdd-ledger.md` in the worktree (gitignored), copied to the repo-root `local/` at the finish together with anything else new under the worktree's `local/`.
- Throwaway measurements from brainstorming: the bench in the scratch copy and the samply profiles live in the session scratchpad only; their numbers are in the spec. samply: `<scratchpad>/samply/bin/samply` (re-install into the scratchpad with `cargo install samply --root <scratchpad>/samply --locked` if it is gone). Build profiles with `CARGO_PROFILE_RELEASE_DEBUG=true`.
- The shell's `grep` may silently return nothing; use `/usr/bin/grep`. Python only via `local/.venv/bin/python`. JS/TS only via Bun.

## Global Constraints

- **Equivalence (spec criterion 1):** every step-exact rung, step count, step-budget outcome, console needle, emulated time and frame hash in `cargo test --workspace`, the release `boot_progress` suite and `bun test` stays byte-identical to `6cd0a6e`. A pin that moves is a bug in the change, never a re-pin. Timing may change only in the browser host (Task 8), never in `emulator-core`.
- `emulator-core/src/clock.rs` and `emulator-core/src/cpu/` stay chip-agnostic: no ESP32-C3 constant in either.
- `FirmwareBus` keeps ordered, concrete named fields; no `dyn` dispatch. A word fast path may only run the XIP-aperture check and then the RAM-region check, in that order (as `read_byte` does today), and must fall back to the byte path for everything else. Reordering or bypassing any other check is an override: stop and ask.
- `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays logic-free (one-line passthroughs, numeric casts only).
- Unmapped data access never panics (reads 0, writes drop, logged exactly as today); an unmapped instruction fetch always traps. Browser input never panics the emulator.
- `docs/milestone-3-decisions.md` through `docs/milestone-6-decisions.md` stand; overriding one needs the human partner's OK. Milestone 7 rulings go in `docs/milestone-7-decisions.md` (Task 1 creates it), each with its task's commit.
- Data handling: never commit, quote or print anything from `local/full_flash_dump.bin`, `local/boot_log.txt` or the badge videos under `local/m6-badge/`; give paths only. Committed tests and benchmarks depend only on `factory.bin` and the committed fake identities.
- External sources (ESP-IDF v5.5.3 and the like) are fetched into the session scratchpad, never into the repo.
- After any change under `emulator-core/` or `emulator-wasm/`, before commit, the full check list: `cargo fmt --all && cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck`. All green, no new clippy warnings (Task 1 records the count). Frontend-only tasks run `bun test && bun run typecheck && bun run build`.
- Every optimization task (Tasks 3 to 7) ends with a benchmark run (`cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture` and `bun run bench:wasm`) whose numbers go into `docs/milestone-7-decisions.md`'s "Measurements" table in the same commit.
- Pushing and merging are the human partner's call. Commits end with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`

## Review Focus

1. **Code that changes after it was decoded must run as the new code.** A store over a cached instruction, a flash erase/program under an XIP page, or an MMU remap must never execute a stale decode. Pinned by Task 7's `decode_cache_sees_a_store_over_a_cached_instruction` and `decode_cache_sees_a_remapped_xip_page`.
2. **A word access that straddles a region edge, an MMU page edge, or is unaligned must behave exactly as four (or two) byte accesses**, including the unmapped log and the 0 read. Pinned by Task 5's `word_access_across_a_region_edge_matches_the_byte_path` and `word_fetch_across_an_mmu_page_edge_matches_the_byte_path`.
3. **Changing an interrupt line's enable, priority or the threshold mid-run must take effect on the next step**, as the per-step poll does today. Pinned by Task 4's `line_gate_follows_enable_priority_and_threshold_writes`.
4. **Reset, provisioning or a mode switch while a press is held must not strand the button** once input crosses a Worker boundary (a release posted before the Worker has started, a `stop()` with a deferred release). Pinned by Task 8's `worker client flushes a deferred release on stop` and `messages before the worker is ready are queued in order`.
5. **A backgrounded tab must not come back to a catch-up burst, and a slow machine must not freeze the Worker or the page.** The wall delta stays clamped to `MAX_FRAME_DELTA_MS` and each slice is capped. Pinned by Task 8's `worker slice clamps the wall delta` and `worker slice is capped at WORKER_STEPS_PER_SLICE`.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `emulator-core/tests/boot_progress.rs` | Modify (Task 1) | `bench_redraws` (ignored), `execution_digest_is_unchanged`, `hacker_walk_totals_are_unchanged` |
| `docs/milestone-7-decisions.md` | Create (Task 1), grow per task | Rulings, measurements table, profiles, Milestone 8 backlog |
| `frontend/src/bench/walk.ts` | Create (Task 2) | Bridge-level walk helpers shared by the WASM twin test and the bench |
| `frontend/src/bench/redraw-bench.ts` | Create (Task 2) | The host-independent benchmark over a `FirmwareEmulatorHandle` |
| `frontend/bench/bun-bench.ts` | Create (Task 2) | Bun CLI entry (`bun run bench:wasm`) |
| `frontend/bench.html`, `frontend/src/bench/bench-page.ts`, `frontend/src/bench/bench-worker.ts` | Create (Task 2) | Browser bench page; runs the bench unpaced in a Worker |
| `frontend/bench/safari-bench.ts` | Create (Task 2) | Drives Safari through `safaridriver` (WebDriver HTTP) |
| `frontend/test/cpu-wasm.test.ts` | Modify (Task 2) | Uses `src/bench/walk.ts`; assertions unchanged |
| `frontend/test/redraw-bench.test.ts` | Create (Task 2) | Bench logic over a fake handle |
| `frontend/package.json`, root `package.json` | Modify (Task 2) | `bench:wasm`, `bench:safari` scripts |
| `Cargo.toml` | Modify (Task 3) | Release `opt-level = 3` |
| `emulator-core/src/peripherals/intc.rs` | Modify (Task 4) | Set-bit routing; precomputed line gate |
| `emulator-core/src/peripherals/usb_serial_jtag.rs` | Modify (Task 4) | Register store as fields/array |
| `emulator-core/src/mem/bus.rs` | Modify (Tasks 5, 7) | Word fast paths; per-step trims the profile names |
| `emulator-core/src/cpu/rom_stubs.rs` | Modify (Task 6) | Address-range precheck in `RomStubTable::lookup` |
| `emulator-core/src/cpu/decode_cache.rs` | Create (Task 7) | Decoded-instruction cache |
| `emulator-core/src/cpu/mod.rs` | Modify (Task 7) | `Cpu::step` uses the cache |
| `frontend/src/runtime/firmware-runtime.ts` | Modify (Task 8) | Pure core: present callback instead of a canvas context; slice pacing |
| `frontend/src/runtime/firmware-worker-protocol.ts` | Create (Task 8) | Message types, both directions |
| `frontend/src/runtime/firmware-worker.ts` | Create (Task 8) | Worker entry: owns the emulator and the paced loop |
| `frontend/src/runtime/firmware-worker-client.ts` | Create (Task 8) | Main-thread `FirmwareRuntime` over a Worker |
| `frontend/src/main.ts` | Modify (Task 8) | Builds the Worker client instead of the in-thread runtime |
| `frontend/test/firmware-runtime.test.ts`, `frontend/test/firmware-worker.test.ts` | Modify / Create (Task 8) | Core pacing tests; message-layer tests |
| `docs/firmware-emulator-notes.md`, `CLAUDE.md` | Modify (Task 10) | Throughput figures, Worker architecture, suite times |

---

### Task 1: Native benchmark, equivalence rungs and the decisions doc (no behaviour change)

**Files:**
- Modify: `emulator-core/tests/boot_progress.rs`
- Create: `docs/milestone-7-decisions.md`

**Interfaces:**
- Produces: the ignored test `bench_redraws`; rungs `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged`; constants `HOME_REDRAW`, `DOWN_REDRAW`, `ONBOARDING_REDRAW` of type `Redraw { emulated_us: u64, steps: u64, end_hash: u64 }`; the decisions doc's "Measurements" table (columns: commit, task, host, profile, walk M steps/s, HOME x, DOWN x, onboarding x, release suite s).

- [ ] **Step 1: Baseline the check list** at `6cd0a6e` (the full Global Constraints list). Record test counts, the release suite time and the clippy warning count in the ledger.

- [ ] **Step 2: Add the equivalence rungs (they pass at baseline by construction; they are pins for later tasks)**

```rust
/// Milestone 7 equivalence proof: FNV-1a over every step's `pc` for the
/// first 20M steps of blank-flash boot, plus all 32 registers every
/// 1,000,000 steps. Any change to what the core executes moves it.
#[test]
fn execution_digest_is_unchanged() {
    assert_eq!(execution_digest(20_000_000), EXECUTION_DIGEST);
}

/// Milestone 7 equivalence proof at the end of the longest chain: hacker's
/// launcher checkpoint (`launcher_for`).
#[test]
fn hacker_walk_totals_are_unchanged() {
    let rt = launcher_for(OWN_ROLE);
    assert_eq!(rt.total_steps(), HACKER_LAUNCHER_STEPS);
    assert_eq!(rt.emulated_time_us(), HACKER_LAUNCHER_US);
    assert_eq!(fnv1a_bytes(rt.console_output().as_bytes()), HACKER_LAUNCHER_CONSOLE_HASH);
}
```

`execution_digest(steps: u64) -> u64` runs `rt.run(1)` per step (reading `rt.pc()` and, every 1,000,000 steps, `rt.cpu().regs.read(r)` for r in 0..32). `HACKER_LAUNCHER_STEPS` is 372,597,011 (measured while brainstorming; confirm). Measure `EXECUTION_DIGEST`, `HACKER_LAUNCHER_US` and `HACKER_LAUNCHER_CONSOLE_HASH` at `6cd0a6e` and pin them. Each constant's doc says it was pinned at `6cd0a6e` and must never be re-pinned in Milestone 7.

- [ ] **Step 3: Add `bench_redraws` (`#[ignore]`)**

It times the walk to hacker's launcher (`launcher_for`), then measures three redraws from cloned checkpoints with the brainstorming bench's method. Press the slot. Sample the framebuffer every 1,000 µs of emulated time (`run_until`), releasing after `PRESS_HOLD_US` + 1,000 µs. Record the last frame change within the window: emulated µs, steps and busy ticks since the press. Then re-run the same schedule unsampled from fresh clones five times, timing wall clock (median and min).

| Constant | From | Slot | Window | Pinned (measured at `6cd0a6e`) |
|---|---|---|---|---|
| `HOME_REDRAW` | `registered(OWN_ROLE)` | `TO_LAUNCHER_SLOT` | 3,000,000 µs | 254,000 µs, 19,106,041 steps, `LAUNCHER_HASH` |
| `DOWN_REDRAW` | `launcher_for(OWN_ROLE)` | `NAV_SLOT` | 1,500,000 µs | 115,000 µs, 6,858,235 steps, `LAUNCHER_AFTER_NAV_HASH` |
| `ONBOARDING_REDRAW` | `provisioned(OWN_ROLE)` | 1 | 3,000,000 µs | 67,000 µs, 3,018,254 steps, `TO_SHAKE_PAGE[0].1` |

It asserts each measured `(emulated_us, steps)` equals the constant and that the frame at the end equals `end_hash`, then prints one line per item:
`BENCH <name> emulated_ms=<> steps=<> busy_pct=<> wall_ms_median=<> wall_ms_min=<> factor=<> msteps_per_s=<>`, plus `BENCH walk steps=<> wall_s=<> msteps_per_s=<>`.
If `ONBOARDING_REDRAW`'s end frame is not page 2's pinned hash at 67 ms (the sweep may still be finishing), pin the hash it does show, and record a ruling.

- [ ] **Step 4: Run it at baseline**

Run: `cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture`
Expected: PASS. Factors near the spec's (HOME about 0.19x, DOWN about 0.23x, onboarding about 0.30x at `opt-level = "s"`).

- [ ] **Step 5: Create `docs/milestone-7-decisions.md`**

Sections, modelled on `docs/milestone-6-decisions.md`: intro (spec and plan paths), "Rulings", "Measurements" (the table above, first row = this baseline), "Profiles", "Milestone 8 backlog" (stub, filled in Task 10). "Profiles" opens with the procedure every optimization task follows. Build with `CARGO_PROFILE_RELEASE_DEBUG=true cargo test -p emulator-core --release --test boot_progress --no-run`, then run `samply record --save-only --unstable-presymbolicate -o <scratchpad>/prof.json.gz -- <test binary> bench_redraws --ignored --nocapture`. Aggregate with a scratchpad script (not committed) and record the top-10 self-time symbols as shares of the emulation thread (the test harness's waiting thread excluded). Its first entry is the baseline profile.

- [ ] **Step 6: Full check list, then commit** (`test: Milestone 7 benchmark and equivalence rungs`).

---

### Task 2: The WASM benchmark (Bun, Chrome, Safari)

**Files:**
- Create: `frontend/src/bench/walk.ts`, `frontend/src/bench/redraw-bench.ts`, `frontend/bench/bun-bench.ts`, `frontend/bench.html`, `frontend/src/bench/bench-page.ts`, `frontend/src/bench/bench-worker.ts`, `frontend/bench/safari-bench.ts`, `frontend/test/redraw-bench.test.ts`
- Modify: `frontend/test/cpu-wasm.test.ts`, `frontend/package.json`, root `package.json`

**Interfaces:**
- Consumes: `FirmwareEmulatorHandle` (`src/cpu/bridge.ts`), `createProvisioner` (`src/runtime/provisioner.ts`), the Task 1 redraw constants (copied as TS constants with a comment naming their Rust source).
- Produces:
  - `walk.ts`: `fnv1a(fb: Uint16Array): bigint`; `runUntilStable(handle, maxSteps: number, stableFor: number, skip: bigint[]): bigint`; `runForUs(handle, us: number): void`; `press(handle, slot: number): void`; `pressAndSettle(handle, slot: number, previous: bigint): bigint`; the walk constants now in `cpu-wasm.test.ts`. These helpers throw `Error` where the test used `expect`.
  - `redraw-bench.ts`: `runRedrawBench(handle: FirmwareEmulatorHandle, identityJson: string, now: () => number, log: (line: string) => void): BenchResult`, where `BenchResult = { walk: { steps: number; wallMs: number }; redraws: Array<{ name: "home" | "down" | "onboarding"; emulatedMs: number; steps: number; wallMs: number; factor: number; mStepsPerS: number }> }`.
  - The bench page writes `JSON.stringify(result)` into `<pre id="result" data-done="true">` (or `data-error`).

- [ ] **Step 1: Move the walk helpers** from `cpu-wasm.test.ts` into `src/bench/walk.ts`. The test imports them; every assertion and pinned hash in the test stays as it is. Run `bun test test/cpu-wasm.test.ts`: PASS.

- [ ] **Step 2: Write `test/redraw-bench.test.ts`** against a fake handle (as `test/firmware-runtime.test.ts` fakes one). Tests: `measures a redraw from the press to the first sample showing the pinned end hash`, and `throws when a redraw ends on the wrong hash`. Run: FAIL.

- [ ] **Step 3: Implement `runRedrawBench`.** The WASM handle cannot clone, so the bench runs the rungs' own walk on one handle, **without changing its input schedule** (a different schedule could move later frames): boot, provision hacker, onboarding (with the shake) to the registered screen, HOME to the launcher, then DOWN (as `launcher_responds_to_navigation`). Three presses are timed in place:
  - onboarding page 1 → 2: the first `pressAndSettle` after provisioning;
  - HOME: the press to the launcher;
  - DOWN: the extra press.

  For each, record wall time and emulated time from the press to the first 250,000-step sample whose hash is the redraw's end hash. The factor is emulated over wall. This in-walk method includes hashing overhead; the decisions doc says so next to its numbers. Task 1's native numbers use the unsampled method. *Ruling for the ledger:* both methods are reported, and only like is compared with like.

- [ ] **Step 4: Run the unit tests**: PASS.

- [ ] **Step 5: Bun CLI.** `frontend/bench/bun-bench.ts` loads `src/cpu/wasm-pkg/emulator_wasm_bg.wasm`, `public/firmware/factory.bin` and the hacker fixture, runs `runRedrawBench` three times, and prints each run's lines plus a median summary. Script `"bench:wasm": "bun bench/bun-bench.ts"` in `frontend/package.json`, with a root proxy. Not part of `bun test`.

- [ ] **Step 6: Browser page.** `frontend/bench.html` loads `src/bench/bench-page.ts`, which starts `new Worker(new URL("./bench-worker.ts", import.meta.url), { type: "module" })`. The worker fetches the WASM, the image and the fixture, runs `runRedrawBench` unpaced and posts the result; the page writes the result block. `bun run build` must not emit `bench.html`. Vite builds only `index.html` by default; verify that `dist/` has no `bench`.

- [ ] **Step 7: Safari driver.** `frontend/bench/safari-bench.ts` starts `safaridriver -p 4444` (or uses one already running), opens a WebDriver session (`POST /session`, `{"capabilities":{"alwaysMatch":{"browserName":"safari"}}}`), navigates to `http://localhost:5173/bench.html`, polls `POST /session/{id}/execute/sync` for `#result`'s `data-done`/`data-error` (timeout 15 min), prints the JSON, then deletes the session. Script `"bench:safari"`. It expects `bun run dev` to be running.

- [ ] **Step 8: Baseline in every host.** Run `bun run bench:wasm`; `bun run dev` plus `bun run bench:safari`; Chrome: open `http://localhost:5173/bench.html` through the Claude-in-Chrome extension and read `#result`. Add one Measurements row per host. If Chrome or Safari cannot be driven, record that and ask the human partner to run the page (gate).

- [ ] **Step 9: Commit** (`feat: WASM redraw benchmark for Bun, Chrome and Safari`).

---

### Task 3: Release `opt-level = 3`

**Files:** Modify: `Cargo.toml`

- [ ] **Step 1:** Change `[profile.release] opt-level = "s"` to `opt-level = 3`, with a comment citing the spec's measurement (1.65x native, 1.30x WASM, +10.7 KB WASM; LTO measured at under 2 %, not adopted).
- [ ] **Step 2:** Full check list. Equivalence: all green with no pin moved. Record the release suite time (expected well under 42 s).
- [ ] **Step 3:** Benchmarks (native and `bench:wasm`); Measurements row; WASM size in the row's note.
- [ ] **Step 4: Commit** (`perf: release opt-level 3`).

---

### Task 4: Interrupt path — set-bit routing, a precomputed line gate, USB register fields

**Files:** Modify: `emulator-core/src/peripherals/intc.rs`, `emulator-core/src/peripherals/usb_serial_jtag.rs`

**Interfaces:**
- Produces: `InterruptController::poll(&self, pending_sources: u64) -> u32` (same signature and results); private `line_gate: u32` with `fn recompute_line_gate(&mut self)`.

*Mechanism (ruling for the ledger, within spec Part 2 item 2):* only the interrupt controller's own register-derived state is precomputed: the mask of lines that are enabled, have priority != 0 and have priority >= threshold. It is recomputed after every write that reaches `InterruptController::write_byte`, including ROM-stub writes, which go through the bus. Peripheral pending state is **not** cached; `pending_sources` is still read every step. So the spec's audit and cross-check for cached peripheral state is not needed. A `debug_assert!` in `poll` compares `line_gate` with a fresh recomputation on every call, so `cargo test --workspace` (debug) runs the cross-check over the whole suite. Before relying on the write hook, `/usr/bin/grep` for every assignment to `cpu_int_enable`, `cpu_int_thresh` and `cpu_int_pri`, and list them in the ruling.

- [ ] **Step 1: Failing tests** in `intc.rs`:
  - `routed_lines_visits_only_pending_sources`: two sources routed to lines 3 and 9, `pending` with only the second set, gives `1 << 9`; source 63 routed gives its line.
  - `line_gate_follows_enable_priority_and_threshold_writes`: enable line 5 with priority 3 and threshold 1 asserts. Then each in turn must stop line 5 asserting on the very next `poll`: writing threshold 4, writing priority 0, clearing the enable bit. Writing threshold 3 makes it assert again (`>=`).
- [ ] **Step 2:** Run: they FAIL (the second fails only if written against the new private API; otherwise it passes at baseline as a regression pin, which is fine).
- [ ] **Step 3:** Implement: the `trailing_zeros` loop in `routed_lines` (as in the spec's prototype), and `poll` = `routed_lines(pending) & line_gate`.
- [ ] **Step 4:** In `usb_serial_jtag.rs`, replace `regs: HashMap<u32, u32>` with fixed storage (an array indexed by word offset over the modeled range, or named fields). Every existing test in the file stays as it is.
- [ ] **Step 5:** Full check list; equivalence rungs green; benchmarks; Measurements and Profiles rows.
- [ ] **Step 6: Commit** (`perf: interrupt routing over pending bits, precomputed line gate`).

---

### Task 5: Word-sized bus fast path

**Files:** Modify: `emulator-core/src/mem/bus.rs`

**Interfaces:**
- Produces: `FirmwareBus`'s `Bus::read16/read32/write16/write32/fetch16` with fast paths; behaviour identical to the byte path.

Fast path rules:
- **Reads:** if the whole access lies in the cache aperture (`in_cache_aperture`) and inside one 64 KiB MMU page, translate once; a `None` translation falls back to the byte path, which logs as today. Else, if the whole access lies in one RAM region, slice it. Anything else uses the byte path.
- **Writes:** RAM regions only (XIP writes keep the byte path).
- **`fetch16`:** the IBUS half of the aperture only (DBUS is never fetchable, as `is_mapped` says), then RAM, then the existing `is_mapped` path for ROM code blobs and the rest.
- A last-hit RAM region index is allowed; it must be re-checked with `contains` before use.

- [ ] **Step 1: Failing/regression tests** in `bus.rs`'s tests (each compares the word path against a second bus driven byte by byte through `read8`/`write8`, including `unmapped_log()` contents):
  - `word_access_across_a_region_edge_matches_the_byte_path` (a word starting 2 bytes before the end of a RAM region, read and written);
  - `word_fetch_across_an_mmu_page_edge_matches_the_byte_path` (a `fetch16` at offset 0xFFFF of a mapped page whose next page is unmapped returns `None`);
  - `unaligned_word_read_in_ram_matches_the_byte_path`;
  - `dbus_is_still_not_fetchable`.
- [ ] **Step 2:** Run: these pass at baseline (they are regression pins); commit them first if the reviewer prefers. Then implement.
- [ ] **Step 3:** Full check list; equivalence; benchmarks; Measurements and Profiles rows.
- [ ] **Step 4: Commit** (`perf: word-sized bus fast path for XIP and RAM`).

---

### Task 6: ROM-stub precheck and the profile-directed per-step pass

**Files:** Modify: `emulator-core/src/cpu/rom_stubs.rs`, and whatever the profile names among `emulator-core/src/mem/bus.rs`, `emulator-core/src/boot.rs`, `emulator-core/src/runtime.rs`.

- [ ] **Step 1:** `RomStubTable` keeps `min_addr`/`max_addr`, updated on insert; `lookup` returns `None` outside the range before the map lookup. Test `lookup_outside_the_registered_range_is_none` (and inside a gap still uses the map). This is chip-agnostic: no address constant in `cpu/`.
- [ ] **Step 2:** Profile `bench_redraws`. For each symbol at 5 % or more of emulation self time outside decode (Task 7's job), make the cheapest change that keeps behaviour identical. Candidates the brainstorming profile showed: `tick_peripherals`'s per-step `sync_cpu_clock` (decode the SYSTEM clock registers on write, not every step), and `line_priorities()` building an array per multi-line step. Each change gets a ledger ruling naming the profile share it addressed.
- [ ] **Step 3:** Full check list; equivalence; benchmarks; Measurements and Profiles rows.
- [ ] **Step 4: Commit** (`perf: ROM-stub range precheck and per-step trims`).

---

### Task 7: Decoded-instruction cache

**Files:** Create: `emulator-core/src/cpu/decode_cache.rs`. Modify: `emulator-core/src/cpu/mod.rs` (`fetch_and_decode`, `Cpu` field, `Clone` stays derived).

**Interfaces:**
- Produces: `pub(crate) struct DecodeCache`, with `fn new() -> Self`, `fn lookup(&self, pc: u32, raw: u32) -> Option<(Instruction, u8)>` and `fn insert(&mut self, pc: u32, raw: u32, instr: Instruction, len: u8)`.

*Mechanism (ruling for the ledger, within spec Part 3):* a direct-mapped table (`1 << 14` entries, index `(pc >> 1) & mask`), each entry `{ pc, raw, instr, len }`. **Validated by the raw instruction bits on every hit**: `Cpu::step` still fetches through `Bus::fetch16`, exactly as today, so fault behaviour is untouched. It then looks up `(pc, raw)` and decodes only on a miss.

Because a hit requires the bytes fetched now to equal the bytes decoded then, no invalidation is needed. Stores, ROM-stub memory writes, flash erase/program and MMU remaps are all correct by construction, and the spec's "Bus tells the core about invalidations" hook is unnecessary (record that as the ruling). Cost: the fetch is not skipped; Task 5 made it cheap. If after this task fetch is still at 10 % or more of emulation time, an invalidation-based variant becomes a Milestone 8 backlog item, not part of this milestone. For a 16-bit instruction, `raw` is the halfword zero-extended; a 32-bit instruction's `raw` is the full word (both `fetch16`s, as today). The cache is a plain field, so a `FirmwareRuntime` clone copies it (a full snapshot).

- [ ] **Step 1: Failing tests** in `cpu/mod.rs`'s tests (with `TestBus`):
  - `decode_cache_sees_a_store_over_a_cached_instruction`: a loop executes `addi x5, x5, 1` twice, then a store overwrites it with `addi x5, x5, 7`; the next iteration adds 7.
  - `decode_cache_sees_a_remapped_xip_page`: in `bus.rs` tests, with a real `FirmwareBus`, run an instruction from an XIP page, rewrite that page's MMU entry to point at different flash bytes, step again, and assert the new instruction ran.
  - `decode_cache_keeps_unmapped_fetch_faulting`: a cached pc whose memory becomes unmapped (a `TestBus` shrink, or an MMU entry set invalid) traps with `INSTRUCTION_ACCESS_FAULT`.
  - `compressed_and_full_instructions_share_the_cache`: a 16-bit and a 32-bit instruction at adjacent pcs both decode correctly on their second execution.
- [ ] **Step 2:** Run: FAIL (the cache does not exist yet; write them against `DecodeCache`).
- [ ] **Step 3:** Implement. `Instruction` is already `Copy` (`decode.rs`).
- [ ] **Step 4:** Full check list; equivalence; benchmarks; Measurements and Profiles rows. If HOME is still under 1.0x in Bun, write its measured factor and the remaining profile into the doc for Task 10.
- [ ] **Step 5: Commit** (`perf: decoded-instruction cache validated by raw bits`).

---

### Task 8: Web Worker host

**Files:** Modify: `frontend/src/runtime/firmware-runtime.ts`, `frontend/src/main.ts`, `frontend/test/firmware-runtime.test.ts`. Create: `frontend/src/runtime/firmware-worker-protocol.ts`, `frontend/src/runtime/firmware-worker.ts`, `frontend/src/runtime/firmware-worker-client.ts`, `frontend/test/firmware-worker.test.ts`.

**Interfaces:**
- `firmware-runtime.ts`:
  - `createFirmwareRuntime(handle, present: (fb: Uint16Array, width: number, height: number) => void, options)` replaces the `ctx` parameter; `stepFirmwareFrame` takes `present` likewise.
  - `FirmwareRuntimeOptions` gains `now?: () => number` (wall ms).
  - New export `WORKER_STEPS_PER_SLICE` (default for the Worker; see below). `DEFAULT_STEPS_PER_FRAME` stays for the in-thread fallback used by tests.
- `firmware-worker-protocol.ts`:
  - `type ToWorker = { kind: "init"; wasm: ArrayBuffer; image: ArrayBuffer } | { kind: "start" } | { kind: "stop" } | { kind: "button"; button: ButtonName; pressed: boolean } | { kind: "provision"; identityJson: string } | { kind: "shake" } | { kind: "reset" } | { kind: "frameAck" } | { kind: "dispose" }`
  - `type FromWorker = { kind: "ready" } | { kind: "frame"; pixels: ArrayBuffer; width: number; height: number } | { kind: "status"; speed: number; provision: ProvisionState | null } | { kind: "error"; message: string }`
- `firmware-worker.ts`: `handleMessage(state: WorkerState, msg: ToWorker, post: (m: FromWorker, transfer?: Transferable[]) => void): void`, pure enough to test without a real Worker, plus the `self.onmessage` wiring.
- `firmware-worker-client.ts`: `createWorkerFirmwareRuntime(worker: WorkerLike, ctx: CanvasRenderingContext2D, wasm: ArrayBuffer, image: ArrayBuffer): FirmwareRuntime`, where `WorkerLike = { postMessage(m: ToWorker, transfer?: Transferable[]): void; onmessage: ((e: { data: FromWorker }) => void) | null; terminate(): void }`. It is the same `FirmwareRuntime` interface; `speed()` and `provisionState()` return the last `status`.

Rules:
- The Worker's loop runs slices on `setTimeout(…, 0)` (`requestAnimationFrame` is not available in every browser's dedicated workers). Each slice targets the wall delta since the previous slice, clamped to `MAX_FRAME_DELTA_MS` (50 ms), capped at `WORKER_STEPS_PER_SLICE` steps, with debt dropped (Milestone 6's rule).
- `WORKER_STEPS_PER_SLICE` = 400,000: about 5 ms of work at 80M steps/s. Real time needs well under that per slice, so the cap binds only when the core is slower than the badge. The builder may tune it with a measured reason (ledger ruling).
- Frames are posted at most one in flight: post, then wait for `frameAck`, which the client sends after painting on its next `requestAnimationFrame`. A slice with no ack outstanding posts the current framebuffer. `status` goes out at most every 250 ms.
- Buttons are applied at the start of the next slice. The press-hold deferral (`PRESS_HOLD_US`) and the shake schedule stay in the core runtime inside the Worker, unchanged. Messages sent before `ready` are queued in order by the client.
- `main.ts`: `ensureFirmwareRuntime` fetches the image and the WASM bytes (the bridge's existing URL logic), creates the Worker with `new Worker(new URL("./runtime/firmware-worker.ts", import.meta.url), { type: "module" })` and builds the client. The bridge stays lazily imported, now by the Worker. The status line's speed reading is unchanged.

- [ ] **Step 1: Move the existing tests** to the `present` signature (`firmware-runtime.test.ts`); they keep their assertions. Run: FAIL until Step 3.
- [ ] **Step 2: New tests** in `test/firmware-worker.test.ts`: `worker slice clamps the wall delta`, `worker slice is capped at WORKER_STEPS_PER_SLICE`, `at most one frame is in flight until frameAck`, `messages before the worker is ready are queued in order`, `worker client flushes a deferred release on stop` (press then `stop()` within `PRESS_HOLD_US`; the Worker applies the release), `a button message for an unknown name is ignored without throwing`, `reset abandons a provision and a shake` (as today). Run: FAIL.
- [ ] **Step 3: Implement.** Run: `bun test && bun run typecheck && bun run build`, all PASS.
- [ ] **Step 4: Manual smoke** with `bun run dev` in Chrome (Claude-in-Chrome): firmware mode boots, provisioning and the shake work, the launcher responds, Snake plays. Record INP for a launcher DOWN from DevTools (or the `PerformanceObserver` `event` entries) and check for long tasks (> 50 ms) on the main thread during a launcher redraw. Ledger plus Measurements note.
- [ ] **Step 5: Commit** (`feat: run the firmware emulator in a Web Worker`).

---

### Task 9: Human gate — the browser check

Stop and ask the human partner, in one message:
- the Measurements table so far (every host), and which redraws are under 1.0x and why;
- the INP and long-task observations from Task 8;
- to judge, side by side with the badge, in Safari and Chrome on the M5 Pro: launcher HOME and DOWN, onboarding page turns, and Snake still at about 1.0x;
- any open ruling that needs their OK.

Nothing later runs before the answer. Record the verdicts in the ledger and the decisions doc.

---

### Task 10: Docs, suite time, final review

**Files:** Modify: `docs/milestone-7-decisions.md`, `docs/firmware-emulator-notes.md`, `CLAUDE.md`.

- [ ] **Step 1:** Decisions doc: final Measurements and Profiles. Milestone 8 backlog: the block cache first if any redraw is under 1.0x, with its measured factor; then every Milestone 7 backlog item from `docs/milestone-6-decisions.md` except the two throughput items, carried word for word, under the same headings.
- [ ] **Step 2:** Notes: "Timing model" (no change to emulated timing; the Worker host's slice pacing), "Known limitations" (current throughput per host). `CLAUDE.md`: current state (the throughput figures), the release profile, the Worker in the architecture block (`firmware-worker*.ts`), `decode_cache.rs`, the bench commands, and the suite times (`bun test`, the release suite, `cargo test --workspace`).
- [ ] **Step 3:** Full check list; record the suite times. The release suite must be at or under its Milestone 6 time (about 42 s); over 90 s is a gate.
- [ ] **Step 4: Commit** (`docs: Milestone 7 throughput, Worker host, current state`).
- [ ] **Step 5:** Whole-branch review on the most capable model, then `superpowers:finishing-a-development-branch`. The human partner decides merge and push. List every `Ruling:` line from the ledger under "Rulings I made".

---

## Orchestrator notes

- **Execution method:** to be chosen by the human partner. Milestone 6 used `superpowers:subagent-driven-development`: a fresh opus implementer per task; a fresh opus reviewer per task (spec compliance, then code quality); sonnet for scoped re-reviews of fix rounds; an opus whole-branch review after Task 10.
- **Ledger:** `local/m7-sdd-ledger.md` in the worktree. One line per dispatch, review verdict, fix round, ruling (`Ruling R-T<n>-<k>: <decision> — cost if wrong: <...>`), measurement (steps/s, factors, suite times) and human answer. Start it with a preflight scan of the Interfaces blocks (producer vs consumer per task pair). Every ruling that changes the design also goes into `docs/milestone-7-decisions.md` in the task's commit.
- **Order:** 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 (gate) → 10. Strictly sequential: each optimization is measured against the previous one, and the equivalence rungs from Task 1 guard every later task.
- **Gates (stop and ask; nothing later runs before the answer):**
  - any hardware-compared frame hash moving: the registered screen, onboarding, the launcher and the launcher after DOWN, Dice open and roll (including the animation), Snake open, after-turn and game over (under the equivalence rule any moved pin is already a bug; these also stop the run);
  - a release `boot_progress` suite over 90 s;
  - any override of a recorded decision;
  - Task 2 Step 8 only if a browser cannot be driven;
  - Task 9 always.

  Ask everything open at a gate in one message.
- **Implementer brief:** this plan's Global Constraints and Review Focus, the full task text, the Interfaces of the tasks it consumes, the spec path, the relevant ledger lines (the latest Measurements row, rulings) and the worktree path. Give paths only for anything under `local/`. Tell implementers about `/usr/bin/grep`, the scratchpad-only rule for external sources, and that a moved pin is a bug to fix, not a number to update.
- **Implementer report:** commits; the full check list's results (test counts, suite times, clippy count); the benchmark lines; confirmation that no pin moved (`git diff 6cd0a6e -- emulator-core/tests frontend/test` shows only additions and the Task 2 helper move); any deviation from the task text; what is left.
- **Reviewer checks (in addition to the skill's):**
  - no pinned number or hash edited (diff the test files against `6cd0a6e`);
  - `cpu/` and `clock.rs` free of ESP32-C3 constants;
  - no `dyn` in `FirmwareBus`, and the fast paths keep the XIP-then-RAM order with a byte-path fallback;
  - unmapped logging unchanged;
  - `emulator-wasm` still logic-free;
  - the bench page absent from `dist/`;
  - the Measurements row and the profile present in the commit;
  - no scratch files, PNGs or `local/` data committed (`git show --stat`).
- **Model choice:** opus implementers and reviewers, sonnet scoped re-reviews, opus whole-branch review (Milestone 6's convention). Tasks 5, 7 and 8 are the riskiest; give their reviewers the Review Focus line they own.
- **Finish:** copy the worktree's `local/m7-sdd-ledger.md` and anything else new under its `local/` to the repo-root `local/` before removing the worktree.

# Milestone 7: design decisions, measurements, and the Milestone 8 backlog

Milestone 7 makes the real-firmware emulator fast enough for the badge's
busy screens (the launcher's HOME and DOWN redraws, onboarding's page
turns) to play at the physical badge's speed, and moves the emulator
into a Web Worker so input never waits behind a frame. It is pure
performance work: nothing the firmware can observe may change (spec
criterion 1). The design spec is
`docs/superpowers/specs/2026-10-21-milestone7-core-throughput-design.md`
and the plan is
`docs/superpowers/plans/2026-10-21-milestone7-core-throughput.md`;
`docs/firmware-emulator-notes.md` holds the forensic findings, the timing
model and the stall-by-stall history. This file records the rulings, the
benchmark results and profiles, and what is deferred. Where the spec or
plan disagree with this file or the code, this file and the code win.

The equivalence pins are the ones Task 1 added to
`emulator-core/tests/boot_progress.rs` (`execution_digest_is_unchanged`,
`hacker_walk_totals_are_unchanged`, and the three `Redraw` constants that
`bench_redraws` checks), plus every pin that was already there. All were
measured at `6cd0a6e` and are never re-pinned in this milestone: a pin
that moves is a bug in the change.

## Rulings

Every ruling of the milestone, with its reason and what it would cost if
wrong. The four preflight rulings came from the orchestrator's scan of
the plan before Task 1 and keep the id of the task they govern;
R-T*n*-*k* rulings are made in Task *n*.

### Preflight

- **R-T2-1, native and WASM bench numbers are compared like with like
  only.** Task 1's native bench times the unsampled schedule to the
  redraw's last frame change; Task 2's WASM bench cannot clone, so it
  times the rungs' own walk in place, to the first 250,000-step sample
  that shows the end hash, hashing included. Both are reported, and only
  like is compared with like; the doc says so next to the numbers. If
  wrong: browser factors read slightly pessimistic (hashing overhead),
  possibly an extra optimization judged needed.
- **R-T4-1, Task 4 caches the line gate, not peripheral pending
  state.** Task 4 precomputes only the interrupt controller's
  register-derived line gate (recomputed on every intc write, with a
  `debug_assert!` cross-check in `poll`) and does not cache peripheral
  pending state. This meets spec Part 2 item 2's bar ("exactly the lines
  the per-step poll delivers, at the same step") without the audit,
  which applies only to cached peripheral state. If wrong: less speedup
  than a full asserted-mask cache; that becomes a Milestone 8 item.
- **R-T7-1, the decode cache validates by raw bits, with no Bus hook.**
  Task 7's decode cache is validated by the raw instruction bits on every
  hit (the fetch still goes through `Bus::fetch16`), so no invalidation
  hook is added to the `Bus` trait. This meets spec Part 3's binding
  outcome (no stale decode after a store, a flash erase or program, or an
  MMU remap; an unmapped fetch still traps) by construction. If wrong:
  fetch cost stays per step; if fetch is still 10 % or more of the
  profile after Task 7, an invalidation-based variant is a Milestone 8
  item.
- **R-T7-2, `decode_cache_sees_a_remapped_xip_page` lives in
  `mem/bus.rs`'s tests.** Task 7's file list names only `cpu/mod.rs`, but
  its test text puts the test in `mem/bus.rs`; the file list is
  incomplete and the test text governs. If wrong: only a test location.

### Task 1

- **R-T1-1, the baseline pins are measured, and match the plan.** Task 1
  ran at `8165dd6`, whose code is identical to `6cd0a6e` (the two commits
  since add only the spec and the plan), so its measurements are the
  `6cd0a6e` pins. Every number the plan quoted from brainstorming was
  confirmed: `HACKER_LAUNCHER_STEPS` 372,597,011, and the three redraws
  (HOME 254,000 µs and 19,106,041 steps, DOWN 115,000 µs and 6,858,235,
  onboarding 67,000 µs and 3,018,254). Onboarding's last change in the
  3 s window is to page 2's pinned frame (`TO_SHAKE_PAGE[0].1`), so the
  brief's fallback (pin whatever frame it shows) was not needed. Newly
  measured: `EXECUTION_DIGEST` `0x30e1_c4af_04a0_f326`,
  `HACKER_LAUNCHER_US` 98,461,748, `HACKER_LAUNCHER_CONSOLE_HASH`
  `0xd042_f54c_287b_c3ef`. If wrong: none; a later task that moves one of
  these has a bug.
- **R-T1-2, the timed pass runs the sampled pass's schedule exactly,
  and stops at the redraw's end.** The sampled pass releases the button
  once `PRESS_HOLD_US` has run, before the slice that ends at
  `PRESS_HOLD_US` + 1 ms, so the press is held exactly 100 ms, as the
  rungs' `press` holds it. The timed (unsampled) pass now does the same,
  and runs only to the redraw's end time: when that end lies inside the
  hold (onboarding's 67 ms), it never reaches the release. The
  brainstorming bench always ran to `PRESS_HOLD_US` + 1 ms before
  releasing, so its onboarding timing covered 101 ms of emulated time
  (3,085,579 steps instead of 3,018,254; the extra 34 ms are mostly idle)
  while its factor divided 67 ms by that wall time, and its HOME and DOWN
  presses were held 1 ms longer than measured. The spec's factors move by
  at most 2 % (onboarding 0.30x then, 0.31x now). If wrong: the bench
  would time a slightly different schedule than it reports.
- **R-T1-3, the timed pass asserts the sampled pass's steps.** Slicing a
  run with `run_until` is invisible to the guest (`boot::idle_until`'s
  doc): measured from onboarding page 1 through 3 s with 1 ms, 7 ms and
  100 ms slices and unsliced, every step count, idle tick count and frame
  matched at 30, 67, 100, 200 and 3,000 ms. So each timed run must end
  on the redraw's frame after exactly the redraw's steps, and the bench
  asserts both; `msteps_per_s` divides those steps by the median wall
  time. If wrong: a change that made slicing observable would fail the
  bench rather than be reported as a speed-up, which is the intent.

## Measurements

The native benchmark is
`cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture`.
It walks to hacker's launcher (timed: the `walk` line), then measures
three redraws from cloned checkpoints: HOME from the registered screen,
DOWN on the launcher, A from onboarding page 1 to page 2. Each redraw is
first sampled every 1 ms of emulated time to find its last frame change
(the pinned `Redraw`), then re-run five times unsampled from fresh
clones; the factor is the redraw's emulated time over the median wall
time of those runs (1.0x is the badge's speed). The walk's steps/s
includes the walk's own frame sampling (`run_until_stable_frame` hashes
the framebuffer every 250,000 steps) and the provisioning host, so it
reads a little below the redraws' steps/s.

| Commit | Task | Host | Profile | Walk M steps/s | HOME x | DOWN x | Onboarding x | Release suite s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `6cd0a6e` (measured at `8165dd6` plus Task 1's tests) | 1 | native, Apple M5 Pro (18 cores), rustc 1.98.1 | release `opt-level = "s"` | 13.43 | 0.184 | 0.232 | 0.308 | 41.5 |

The baseline's `BENCH` lines:

```
BENCH walk steps=372597011 wall_s=27.75 msteps_per_s=13.43
BENCH home emulated_ms=254.0 steps=19106041 busy_pct=94.0 wall_ms_median=1384.2 wall_ms_min=1382.4 factor=0.184 msteps_per_s=13.80
BENCH down emulated_ms=115.0 steps=6858235 busy_pct=74.6 wall_ms_median=495.8 wall_ms_min=494.9 factor=0.232 msteps_per_s=13.83
BENCH onboarding emulated_ms=67.0 steps=3018254 busy_pct=56.4 wall_ms_median=217.2 wall_ms_min=217.0 factor=0.308 msteps_per_s=13.90
```

### Task 1 baseline check list (`6cd0a6e` code)

All green. Wall times on the development machine (Apple M5 Pro, 18
cores). Before Task 1's tests, then with them:

| Check | Before Task 1's tests | With them | Time (before / with) |
| --- | --- | --- | --- |
| `cargo fmt --all --check` | clean | clean | |
| `cargo test --workspace` (debug) | 534 lib + 93 integration tests pass (62 boot rungs) | 534 lib + 95 integration pass (64 boot rungs, 1 ignored) | 30.2 s / 28.7 s wall including the build check; `boot_progress` 28.2 s / 28.1 s |
| `cargo test -p emulator-core --release --test boot_progress` | 62 pass | 64 pass, 1 ignored | 41.3 s / 41.5 s |
| `cargo clippy --workspace --all-targets` | 3 warnings (2 doc-list indentation in `boot.rs`, 1 no-effect operation in a `console.rs` test), the same three as at Milestone 6's base | the same 3 | |
| `bun run build:wasm`, `bun run typecheck` | ok | ok | |
| `bun test` | 147 pass, 0 fail | 147 pass, 0 fail | 36.5 s / 36.4 s |

The two new rungs do not lengthen the suites: `hacker_walk_totals_are_unchanged`
continues from the shared launcher checkpoint, and
`execution_digest_is_unchanged` (20M single steps) runs in parallel with
the longest chain.

## Profiles

Procedure (every optimization task follows it):

1. Build the test binary with symbols:
   `CARGO_PROFILE_RELEASE_DEBUG=true cargo test -p emulator-core --release --test boot_progress --no-run`
   (it prints the binary's path under `target/release/deps/`).
2. Record the benchmark with samply 0.13.1 (installed in the session
   scratchpad, `cargo install samply --root <scratchpad>/samply --locked`;
   never in the repo):
   `<scratchpad>/samply/bin/samply record --save-only --unstable-presymbolicate -o <scratchpad>/prof.json.gz -- <test binary> bench_redraws --ignored --nocapture`.
3. Aggregate with a scratchpad script (not committed): resolve each
   sample's leaf frame through the `.syms.json` sidecar and count self
   time per symbol over the thread named `bench_redraws` only. samply
   samples both threads equally often; libtest's main thread spends all
   of its samples waiting (`semaphore_timedwait_trap`) and is excluded.
4. Record the top 10 self-time symbols below, as shares of the
   `bench_redraws` thread. The thread includes the benchmark's own work:
   the walk's and the sampled passes' framebuffer hashing (most of the
   `bench_redraws` closure's self time) and the provisioning host.

### Task 1 baseline (`6cd0a6e` code, release `opt-level = "s"`)

45,222 samples on the `bench_redraws` thread.

| Share | Symbol |
| --- | --- |
| 50.0 % | `InterruptController::poll` (with the 64-source `routed_lines` scan inlined into it) |
| 5.6 % | `FirmwareBus::read_byte` |
| 5.5 % | `cpu::decode::decode_16` |
| 5.4 % | `RandomState::hash_one::<&u32>` (hashing for the `HashMap<u32, _>` lookups: `RomStubTable::lookup` on every fetch, `UsbSerialJtag`'s register store) |
| 4.6 % | `FirmwareBus::pending_sources` |
| 3.7 % | `cpu::decode::decode_32` |
| 3.3 % | `cpu::fetch_and_decode` |
| 3.1 % | `cpu::execute::execute` |
| 2.8 % | `FirmwareBus::is_mapped` |
| 2.3 % | the `bench_redraws` closure (the benchmark's own framebuffer hashing) |

Below the top 10: `tick_peripherals` 2.1 %, `write_byte` 1.5 %,
`RomStubTable::lookup` 1.2 %, `UsbSerialJtag::int_st` 1.1 %, `fetch16`
1.0 %, `System::cpu_hz` 1.0 %, `Cpu::step` 1.0 %. At `opt-level = "s"`
the interrupt poll dominates even more than in the spec's
`opt-level = 3` brainstorming profile (40 % `step_with_interrupts` self
time), because less of the rest is inlined and fast.

## Milestone 8 backlog

Filled in by Task 10: what this milestone leaves (a block cache first if
HOME is still short of 1.0x), plus everything carried from
`docs/milestone-6-decisions.md`'s Milestone 7 backlog except the
throughput item.

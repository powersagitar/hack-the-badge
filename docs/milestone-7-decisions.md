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

### Task 2

- **R-T2-2, the WASM bench's comparable factor is the pin factor.** The
  in-walk method (R-T2-1) stops at the first 250,000-step sample showing
  the end hash, and a sample's emulated time can lie far past the
  redraw's end: idle steps fast-forward to the next alarm, and
  onboarding's end (67 ms) lies inside the 100 ms press hold, which has no
  sample. Measured under Bun, the samples land 180,000 steps (95.8 ms)
  after HOME's pinned end, 50,000 steps (0.6 ms) after DOWN's and 317,355
  steps (173 ms) after onboarding's, so emulated over wall (`factor`)
  reads 0.857x for onboarding, against 0.308x native. The bench therefore
  also reports `pin_factor`: the pinned redraw's emulated time over the
  wall time its pinned steps take at the in-walk steps/s, which is the
  native factor's formula (native: pinned emulated time over the wall
  time of exactly the pinned steps). It keeps R-T2-1's hashing overhead
  (pessimistic, as intended), and is the number in the HOME, DOWN and
  onboarding columns for WASM rows; `factor` is printed beside it. If
  wrong: if busy steps cost more per step than the idle tail the in-walk
  steps include, `pin_factor` reads slightly optimistic; the in-walk
  steps exceed the pinned steps by at most 10.5 % (onboarding; HOME 0.9 %,
  DOWN 0.7 %), so the error is small.
- **R-T2-3, the unit tests test `timeRedraw`, not `runRedrawBench`.**
  A fake handle cannot produce frames with the pinned FNV hashes (that
  needs FNV-1a preimages), so `runRedrawBench`'s walk cannot run against
  one. `redraw-bench.ts` exports `timeRedraw(name, endHash, handle, now,
  act)`, the timing core `runRedrawBench` calls for each redraw with the
  pinned end hash; `test/redraw-bench.test.ts` drives it with a fake whose
  end hash is computed. `runUntilStable` and `pressAndSettle` take an
  optional `onSample` hook (it only observes) so the timing sees each
  sample. The walk itself is checked by running the bench (it throws on
  any hash the twin test pins). If wrong: only test coverage.
- **R-T2-4, the WASM walk is the twin test's, 371,857,010 steps to the
  launcher.** The bench walks with `test/cpu-wasm.test.ts`'s schedule
  (the frontend provisioner, polled every 50,000 steps), then
  `launcher_responds_to_navigation`'s DOWN. Its walk reaches the
  launcher 740,001 steps before the native walk (372,597,011), which
  provisions with the Rust helper (the two provisioning hosts' schedules
  differ; the exact cause was not traced); every pinned hash matches, and the
  three redraws start from the same frames. The `walk` line covers boot
  to the launcher, including the onboarding and HOME timings, not DOWN.
  If wrong: the walk's steps/s compares two slightly different
  schedules, under 0.2 % apart.
- **R-T2-5, `bench:safari` loads the page three times by default.** The
  brief says the driver prints the JSON; it does, once per load, plus the
  same median summary `bench:wasm` prints (`--runs N` changes the
  count), so a Safari row is a median of three like the Bun row. If
  wrong: none; `--runs 1` is the brief's behaviour.
- **R-T2-6, which browser runs make the baselines.** The Chrome baseline
  is the single run taken alone (the bench page does one run per load).
  The Safari baseline is the median of 3 runs. Run 1 overlapped a Chrome
  run and is kept, because runs 2 and 3 ran alone and agree within 1 %.
  If wrong: the browser baselines are off by a few %, below the effect
  sizes Tasks 3 to 7 measure.

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
| `6cd0a6e` core (Task 2's bench, measured at `b9a7336` plus Task 2) | 2 | WASM, Bun 1.3.14 (JavaScriptCore), Apple M5 Pro | release `opt-level = "s"` (wasm-pack) | 11.00 | 0.151 | 0.199 | 0.264 | n/a |
| `6cd0a6e` core (Task 2's bench, measured at `0468972`) | 2 | WASM, Safari 27.0.1 (bench page, Worker), Apple M5 Pro | release `opt-level = "s"` (wasm-pack) | 11.07 | 0.152 | 0.201 | 0.264 | — |
| `6cd0a6e` core (Task 2's bench, measured at `0468972`) | 2 | WASM, Chrome 155 (bench page, Worker), Apple M5 Pro | release `opt-level = "s"` (wasm-pack) | 12.36 | 0.172 | 0.228 | 0.301 | — |
| Task 3 (measured at `0fff5d4` plus Task 3's `Cargo.toml`) | 3 | native, Apple M5 Pro (18 cores), rustc 1.98.1 | release `opt-level = 3` | 20.78 | 0.302 | 0.385 | 0.510 | 27.0 |
| Task 3 (measured at `0fff5d4` plus Task 3's `Cargo.toml`) | 3 | WASM, Bun 1.3.14 (JavaScriptCore), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 13.75 | 0.191 | 0.257 | 0.340 | n/a |

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

### Task 2: the WASM benchmark (`6cd0a6e` core)

`bun run bench:wasm` (Bun, three runs, median); `bun run bench:safari`
with `bun run dev` running (Safari, through `safaridriver`); the page
`http://localhost:5173/bench.html` (Chrome, or any browser). Each run
walks once (R-T2-4) and times the three redraws in place, in a dedicated
Worker in a browser. **In-walk method (R-T2-1):** each redraw is timed
from the press to the first 250,000-step sample showing its end hash, so
its wall time includes the sampling's framebuffer copies and hashing,
which the native bench's unsampled passes do not; compare WASM rows with
WASM rows only. The table's factors for WASM rows are the pin factors
(R-T2-2); `factor` (emulated time to that sample over wall time) is
printed beside them and is not comparable with native. The walk's steps/s
includes the walk's hashing and the provisioning host, like the native
walk line.

Bun (JavaScriptCore), Apple M5 Pro:

```
BENCH host="Bun 1.3.14 (JavaScriptCore), Apple M5 Pro, darwin arm64" commit=b9a7336+dirty profile=release opt-level="s"
BENCH median runs=3 walk msteps_per_s=11.00
BENCH median runs=3 home emulated_ms=349.8 steps=19286041 wall_ms=1697.1 factor=0.206 msteps_per_s=11.36 pin_factor=0.151
BENCH median runs=3 down emulated_ms=115.6 steps=6908235 wall_ms=581.9 factor=0.199 msteps_per_s=11.87 pin_factor=0.199
BENCH median runs=3 onboarding emulated_ms=240.0 steps=3335609 wall_ms=280.2 factor=0.857 msteps_per_s=11.90 pin_factor=0.264
```

(`+dirty`: Task 2's own uncommitted files; the core and the WASM build
are `b9a7336`'s, whose `emulator-core` is `6cd0a6e`'s plus Task 1's
tests. The three runs agreed within 0.4 % on every wall time.)

Browsers (Step 8's gate, closed): the orchestrator measured both at
`0468972` (in-walk method, Apple M5 Pro). Chrome was driven through
chrome-devtools-mcp, because the Claude-in-Chrome extension was not
available. Safari was driven through `safaridriver` (`bun run
bench:safari`) after the human partner allowed automation; the
implementer's earlier attempts had failed with "session not created"
before that. Which runs make each baseline is ruling R-T2-6. In every
host the steps per redraw were identical (home 19,286,041, down
6,908,235, onboarding 3,335,609), and so was the walk (371,857,010).

- Safari 27.0.1, `bun run bench:safari`, median of 3: walk 11.07
  Msteps/s; pin factor HOME 0.152x, DOWN 0.201x, onboarding 0.264x;
  literal factor 0.208, 0.201, 0.854. Run 1 overlapped a Chrome run;
  runs 2 and 3 ran alone and agree within 1 %.
- Chrome 155, `bench.html`, one run with nothing else running: walk
  12.36 Msteps/s; pin factor HOME 0.172x, DOWN 0.228x, onboarding 0.301x;
  literal factor 0.234, 0.227, 0.976. An earlier run that overlapped
  Safari read 12.15 Msteps/s and pin factors 0.167, 0.219, 0.302, within
  4 %.

### Task 3: release `opt-level = 3`

`Cargo.toml`'s `[profile.release]` goes from `opt-level = "s"` to
`opt-level = 3` (the debug profile already built `emulator-core` at 3,
so the debug suite does not change). Every pin held. Native `BENCH`
lines:

```
BENCH walk steps=372597011 wall_s=17.93 msteps_per_s=20.78
BENCH home emulated_ms=254.0 steps=19106041 busy_pct=94.0 wall_ms_median=839.8 wall_ms_min=837.2 factor=0.302 msteps_per_s=22.75
BENCH down emulated_ms=115.0 steps=6858235 busy_pct=74.6 wall_ms_median=298.6 wall_ms_min=297.1 factor=0.385 msteps_per_s=22.97
BENCH onboarding emulated_ms=67.0 steps=3018254 busy_pct=56.4 wall_ms_median=131.3 wall_ms_min=131.2 factor=0.510 msteps_per_s=23.00
```

Bun (`bun run bench:wasm`, median of 3; the runs agreed within 3 % on
the walk and 1.5 % on every redraw):

```
BENCH host="Bun 1.3.14 (JavaScriptCore), Apple M5 Pro, darwin arm64" commit=0fff5d4+dirty profile=release opt-level=3
BENCH median runs=3 walk msteps_per_s=13.75
BENCH median runs=3 home emulated_ms=349.8 steps=19286041 wall_ms=1342.7 factor=0.261 msteps_per_s=14.36 pin_factor=0.191
BENCH median runs=3 down emulated_ms=115.6 steps=6908235 wall_ms=450.3 factor=0.257 msteps_per_s=15.34 pin_factor=0.257
BENCH median runs=3 onboarding emulated_ms=240.0 steps=3335609 wall_ms=218.0 factor=1.101 msteps_per_s=15.30 pin_factor=0.340
```

(`+dirty`: Task 3's uncommitted `Cargo.toml`.) Against the baselines:
native redraws 1.65x the steps/s (HOME 13.80 to 22.75 M), the walk
1.55x; Bun redraws 1.27x to 1.29x (pin factors 0.151, 0.199, 0.264 to
0.191, 0.257, 0.340), the walk 1.25x. This matches the spec's
brainstorming measurement (1.65x native, 1.30x WASM).

WASM size (`frontend/src/cpu/wasm-pkg/emulator_wasm_bg.wasm`): 94,419
bytes at `"s"`, 105,110 bytes at 3 (+10,691 bytes, +11.3 %), the spec's
figures exactly.

Check list, all green: `cargo fmt --all` clean; `cargo test --workspace`
534 lib + 95 integration pass (64 boot rungs, 1 ignored), 29.1 s wall,
`boot_progress` 28.4 s; release `boot_progress` 64 pass, 1 ignored, 27.0 s
(41.5 s at `"s"`); clippy the same 3 warnings; `bun run build:wasm` and
`bun run typecheck` ok; `bun test` 149 pass, 0 fail, 29.4 s.

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

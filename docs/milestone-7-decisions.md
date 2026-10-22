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

### Task 4

- **R-T4-2, the intc write hook sees every change to the line gate's
  inputs.** The gate derives from `cpu_int_enable`, `cpu_int_pri[0..32]`
  and `cpu_int_thresh` (all private to `InterruptController`).
  `/usr/bin/grep` over `emulator-core/src` and `emulator-wasm/src` finds
  exactly these assignments: the `Default` initializer (all 0, with
  `line_gate: 0`, consistent: nothing enabled) and three `set_byte` calls
  in `InterruptController::write_byte` (`CPU_INT_ENABLE_REG`,
  `CPU_INT_THRESH_REG`, `CPU_INT_PRI_<n>_REG`). No `&mut` to them escapes
  the module. Every write reaches `write_byte` through
  `FirmwareBus::write_byte`'s `INTERRUPT_CORE0_RANGE` branch: guest
  stores, and the six ROM-stub effects (`intr_matrix_set`,
  `esprv_intc_int_*`), which `Cpu::apply_bus_register_op` performs as
  `Bus::read32`/`write32` on the bus. A reset builds a fresh controller
  (`InterruptController::new`), and `Clone` copies `line_gate` with the
  registers. `write_byte` recomputes the gate after every byte it
  accepts, so a byte-split word write is consistent after each byte and
  the next step's `poll` sees the last one. The `debug_assert_eq!` in
  `poll` ran on every step of the debug `cargo test --workspace` (536 lib
  + 95 integration tests, the 64 boot rungs included) without firing. If
  wrong: a stale gate would deliver the wrong lines; debug builds panic
  on the first such step, release builds would diverge from a pin.
- **R-T4-3, the gate is recomputed on every intc write, not only on
  enable, priority and threshold writes.** (Superseded by R-T6-5.) MAP, TYPE and CLEAR writes
  recompute it too (a 32-line loop; intc writes happen during interrupt
  setup and in the FreeRTOS port's threshold changes, far rarer than
  steps, and the Task 4 profile shows `InterruptController::write_byte`
  at 0.1 %).
  It keeps the hook one line with no per-register case to get wrong. If
  wrong: a negligible cost per intc write.
- **R-T4-4, routing ORs `1 << (MAP[src] & 0x1F)` and clears bit 0.** The
  set-bit loop has no branch on the route; a line-0 route ("not routed")
  sets bit 0, cleared once at the end, which is what skipping it did.
  `eip_status` shares `routed_lines`, so `CPU_INT_EIP_STATUS_REG` reads
  as before. If wrong: line 0 could appear asserted; the existing
  `unrouted_source_line_zero_never_fires` pins it.
- **R-T4-5, the USB-Serial-JTAG store is one boxed array over the whole
  4 KiB window.** `regs: Box<[u32; 1024]>` indexed by `offset / 4`
  (`REG_WORDS`), rather than named fields: the module doc promises plain
  storage for every unnamed offset, which named fields cannot keep. Boxed
  so `FirmwareBus` stays small; a checkpoint clone copies 4 KiB more than
  an empty map. An offset past the window (the bus never sends one) reads
  0 and drops the write rather than panicking. The file's existing tests
  are unchanged. If wrong: 4 KiB per runtime and per clone.

### Task 5

- **R-T5-1, the RAM fast path looks addresses up in a resolved span map,
  not in the first region containing the access's first byte.**
  `crate::boot` backs the whole DRAM, IRAM and RTC apertures with scratch
  RAM appended after the image's segments, so regions overlap by design
  and the byte path's first-match rule decides each byte separately: a
  word across a segment's edge reads segment bytes on one side and
  scratch bytes on the other. `FirmwareBus::rebuild_ram_spans` cuts the
  address space at every region edge and every cache-aperture edge, gives
  each piece outside the apertures to the first region containing it,
  merges neighbours of the same region and caps the map at 2^32 (so a
  word at `0xffff_fffe` never wraps into a span). An access wholly inside
  one span reads or writes exactly the bytes the byte path would, in the
  same region, with nothing to log; anything else takes the byte path.
  It is still only the XIP-aperture check, then the RAM-region check, in
  that order (common.md's rule); no other tier is reordered or bypassed.
  The map is rebuilt by the only two places `ram_regions` changes,
  `from_segments` and `add_scratch_ram`. If wrong: a word whose bytes
  resolve to different regions would read or write the wrong one;
  `word_access_across_a_region_edge_matches_the_byte_path` pins both
  edges of a segment inside a later scratch region.
- **R-T5-2, the last-hit hint is a span index, re-checked against the
  whole access.** The brief allows a last-hit RAM region index re-checked
  with `contains`; with spans, the equivalent check is
  `span.start <= addr && addr + len <= span.end`, then a
  `partition_point` binary search on a miss. If wrong: only speed (the
  hint is never trusted unchecked).
- **R-T5-3, writes run the aperture check before the RAM spans** even
  though the spans already exclude both apertures, to keep the
  XIP-then-RAM order literally; an aperture write takes the byte path,
  which drops it as before. If wrong: one range compare per write.
- **R-T5-4, `fetch16`'s fast path is IBUS within one page, else (outside
  IBUS) one RAM span, else the original `is_mapped` path.** An IBUS hit
  through a valid entry is exactly `is_mapped` true for both bytes plus
  `read16`'s XIP read; an invalid entry, a fetch at page offset `0xffff`,
  DBUS, ROM code blobs and unmapped space all fall through unchanged, so
  DBUS stays unfetchable and an unmapped fetch still returns `None`.
  If wrong: a fetch that should trap would run;
  `word_fetch_across_an_mmu_page_edge_matches_the_byte_path` and
  `dbus_is_still_not_fetchable` pin it.
- **R-T5-5, the regression pins go in a commit of their own, before the
  code** (`c54e8c8`), and pass at `5236bd1`. Beyond the brief's cases
  they cover words 1, 2 and 3 bytes before a region's end, both edges of
  a segment inside a later scratch region, the top of the address space,
  unaligned XIP words, and the DBUS aperture's end. If wrong: none; they
  only add coverage.

### Task 6

Shares below are of the `bench_redraws` thread in the Task 5 profile
re-taken at `f23288a` before any Task 6 change (11,051 samples; table
under Profiles).

- **R-T6-1, the ROM-stub table answers `None` outside its registered
  address range before hashing** (addressed `RomStubTable::lookup`,
  16.2 %). `RomStubTable` keeps the lowest and highest registered
  address, widened on every `insert`; `lookup` compares against both,
  then looks the map up. An empty table has an empty range
  (`min_addr = u32::MAX > max_addr = 0`), which replaces the old
  `is_empty()` short-circuit. Identical: no registered address lies
  outside the range, and inside it the map decides as before (a gap
  still misses). The range is learned from the inserted addresses, so
  `cpu/` gains no chip constant. On the badge every fetch from flash
  (IBUS, `0x42...`) or IRAM (`0x4037...`) lies above the highest ROM stub
  and costs two compares. Tests
  `lookup_outside_the_registered_range_is_none` (both ends, a gap, a
  widening insert, a re-insert) and
  `a_single_entry_table_matches_only_its_address`. If wrong: a stub
  outside the learned range would not run; the boot rungs reach every
  stub the firmware calls.
- **R-T6-2, `System` decodes the CPU clock when a clock register is
  written, not every step** (addressed `FirmwareBus::tick_peripherals`,
  11.5 %, whose per-step `sync_cpu_clock` decoded `SYSCLK_CONF` and
  `CPU_PER_CONF`). `System::write_byte` is the only writer of the two
  registers; after each byte it stores the decode in `selected_hz`, and
  `cpu_hz()` returns it (debug builds assert it equals a fresh decode on
  every call, so every step of the debug suite checks it). Milestone 6's
  clock rulings hold unchanged: the bus still samples `cpu_hz()` once per
  step after the instruction (and at the start of an idle fast-forward)
  and calls `Clock::set_cpu_hz` with it; after a byte-split word write
  the stored value is the decode of the whole word, because the sample
  comes after the last byte; an invalid encoding is still `None`, which
  keeps the previous frequency. If wrong: the clock would run at a stale
  rate; the debug assertion fires on the first such step, and
  `boot_reports_the_badges_80_mhz_cpu_clock` and every time pin would move.
- **R-T6-3, `Clock::step` returns 0 before dividing when the remainder
  is still below one tick** (also `tick_peripherals`). At 80 MHz against
  the 16 MHz SYSTIMER four steps in five complete no tick; for them the
  division gives 0 and the modulo leaves the remainder as it is, which the
  early return does without dividing. Pure arithmetic identity, still
  chip-agnostic. If wrong: none possible beyond the identity.
- **R-T6-4, the interrupt poll routes only sources that can reach a
  gated line** (addressed `FirmwareBus::asserted_lines`, 11.5 %; its
  per-source routing loop ran on most busy steps, because some source is
  pending while its line is masked). `InterruptController` keeps
  `gated_sources`, the sources whose MAP route is a non-zero line in
  `line_gate`; `poll` is `routed_lines(pending & gated_sources) &
  line_gate`, 0 without the loop when that is empty. Identical: a pending
  source outside the mask routes to line 0 (cleared) or to a line the
  gate masks, so it cannot change the result. The mask is derived from
  the intc's registers only (MAP and the gate), like `line_gate` (R-T4-1),
  never from peripheral state, so spec Part 2's audit does not apply; no
  pending state is cached. Debug builds check the result against the
  unfiltered `routed_lines(pending) & line_gate` on every poll, and the
  mask against a scan of all 64 MAP registers whenever it is rebuilt;
  Task 4's `debug_assert_eq!` on `line_gate` in `poll` is unchanged. Test
  `gated_sources_follow_map_and_gate_writes` (reroutes to a masked line,
  to line 0 and back; a gate opening under a routed source; two sources on
  one line). If wrong: a deliverable line would be dropped; debug builds
  panic at that step, release builds would move a pin.
- **R-T6-5, the line gate is rebuilt only when one of its inputs
  changes, and the gated sources from per-line source sets; this
  replaces R-T4-3.** With R-T6-4's mask, rebuilding on every intc byte
  write (R-T4-3) added a 64-source scan to each one, and
  `InterruptController::write_byte` rose from 0.3 % to 2.1 % (the FreeRTOS
  port writes `CPU_INT_THRESH` in every critical section, as four byte
  writes). Now `write_byte` rebuilds only when an enable, priority or
  threshold word, or a MAP register's 5-bit route, actually changed
  value (both are pure functions of those registers, so an unchanged
  value leaves them correct; TYPE and CLEAR never feed them), and
  `line_sources[line]` (kept in step with each MAP write; every source
  starts on line 0) makes `gated_sources` the union over the gate's
  lines. `write_byte` is back to 0.2 %. Review Focus 3 holds: a changed
  enable, priority or threshold rebuilds before the write returns, so the
  next step's poll sees it (`line_gate_follows_enable_priority_and_threshold_writes`
  passes unchanged). If wrong: a stale gate or mask; R-T6-4's debug checks
  and Task 4's catch it.
- **R-T6-6, `tick_peripherals` and `asserted_lines` are inlined into the
  step loop; the timed peripherals' advance is out of line.** The two
  were separate calls on every step (together 23.0 % with
  `step_with_interrupts`'s own 1.8 %); `#[inline]` on both, with
  `SysTimer::advance_by` and the USB host's advance moved into
  `advance_timed_peripherals` (`#[inline(never)]`, reached on a step that
  completes a tick), keeps the order of every operation and only removes
  call overhead. If wrong: none observable; only code size (WASM 106,475
  to 110,286 bytes for the whole task).
- **R-T6-7, what Task 6 left alone.** `line_priorities()` builds its
  array only when several lines are asserted at once, inside
  `step_with_interrupts` (1.8 % in total), below the 5 % bar; unchanged.
  An `#[inline]` `fetch16` with its fallback out of line measured no gain
  (the trait method stayed a call) and was reverted; fetch and decode are
  Task 7's (R-T7-1 still fetches every step). `Cpu::step` and `execute`
  are instruction execution. What remains of the interrupt sample is the
  per-step loads of every peripheral's pending state, which only caching
  that state would remove (spec Part 2's audit); not done here. If wrong:
  speed left on the table, a Milestone 8 item if HOME needs it.

### Task 7

- **R-T7-3, the decode cache is tagged by the raw bits alone; this
  departs from the brief's entry shape.** The brief's entry is
  `{ pc, raw, instr, len }` and a hit matched both `pc` and `raw`. The
  decoders take nothing but the bits (`decode_16(u16)`,
  `decode_32(u32)`: no `pc`, no machine state), so an entry another
  address left in the slot is the true decode whenever its bits match
  the bits fetched now. The table is still indexed by `pc`
  (`(pc >> 1) & mask`), `lookup(pc, raw)` and `insert(pc, raw, instr,
  len)` keep the brief's signatures, and the entry is `{ raw, instr, len }`
  (16 bytes instead of 20). Measured in one session, back to back:
  HOME 1.047x and 1.050x with the `pc` tag, 1.110x and 1.125x without it
  (more hits on shared slots, one compare and four bytes less per entry).
  Review Focus 1 holds unchanged: a hit still requires the bits fetched
  this step. If wrong: only if a future decoder reads anything but the
  bits; the module doc says the `pc` tag must come back then, and the
  debug check of R-T7-5 fails on the first such hit.
- **R-T7-4, the table stays at `1 << 14` entries, 262,144 bytes per
  `Cpu` and so per `FirmwareRuntime` clone.** Measured with the `pc` tag
  (HOME factor): `1 << 10` 1.001x, `1 << 12` 1.033x, `1 << 14` 1.047x,
  `1 << 16` 1.063x. The clone-heavy release suite got faster, not slower
  (9.7 s to 8.0 s wall; the table is one 256 KiB `memcpy` per clone, and
  the emulator state a clone already copies is larger), so clone cost is
  not material. If wrong: `1 << 16` would give about 1.5 % more for
  1 MiB per clone and in WASM linear memory; a Milestone 8 option, not
  needed for the bar.
- **R-T7-5, debug builds check every cache hit against a fresh
  decode** (`debug_assert_eq!` in `fetch_and_decode`), so every step of
  the debug suite proves the hit equals what the decoder says. Release
  builds skip it. Cost: the debug workspace run went from 14.0 s to
  14.9 s wall. If wrong: none in release.
- **R-T7-6, no invalidation-based variant goes on the Milestone 8
  backlog.** R-T7-1 sets the bar at fetch 10 % or more after this task;
  `FirmwareBus::fetch16` is 9.0 % of the Task 7 profile (8.8 % before).
  The ROM-stub check still runs before the fetch, and the cache is
  consulted only after a successful fetch, so a stubbed `pc` never
  reaches the cache and an unmapped fetch traps before any lookup. If
  wrong: fetch stays a per-step cost of about 9 %; a block cache that
  skips fetch (Task 10's backlog lead if HOME needs it) would remove it
  together with the dispatch cost below.
- **R-T7-7, Safari's post-core row was taken after Task 8, when the
  human partner attended** (orchestrator). The Safari bench needs the
  human at the machine, so Task 7's row was deferred; it was measured at
  `ea7a035`, whose core is `abd4c3a` (Task 8 changed only the frontend,
  and the bench page drives the bridge directly, not the Worker), and is
  the Measurements table's Safari row after Task 7. If wrong: none; the
  core under test is the same.

### Task 8

- **R-T8-1, a slice the step cap stopped short yields through a
  `MessageChannel`, any other slice through `setTimeout(…, 0)`.** The
  brief schedules every slice on `setTimeout(…, 0)`. HTML clamps a timer
  nested five or more deep to 4 ms, and the Worker's loop reschedules
  itself from inside its own timer, so every slice after the fifth waits
  at least 4 ms. When the core is behind the badge (every busy slice),
  that wait is pure loss: at Chrome's 40.5M steps/s (Task 7) a
  400,000-step slice takes about 9.9 ms, so the core would run 9.9 of
  every 13.9 ms, and a busy screen would play at about 0.71 of the speed
  the core can reach (HOME 0.601x would show as about 0.43x). A
  `MessageChannel` message is not a timer and is not clamped, and it
  still yields to the event loop, so button and `frameAck` messages are
  handled between slices exactly as with a timer. A slice that reached
  its target keeps `setTimeout` (about 250 slices/s once clamped), so an
  idle firmware (Snake between moves, a static screen) does not spin a
  CPU core. The choice is `WorkerHost.schedule(cb, immediate)` in
  `runtime/firmware-worker.ts`, with `immediate` = the last slice was
  capped (`FirmwareRuntimeOptions.onFrame` reports it); pinned by
  `worker slice is capped at WORKER_STEPS_PER_SLICE` and `an uncapped
  slice yields through the timer`. If wrong: if a browser ran the
  channel's messages ahead of the parent's, input would lag on busy
  screens (the smoke test's launcher DOWN checks it); the fix is
  `setTimeout` for every slice, at the throughput cost above.
- **R-T8-2, `WORKER_STEPS_PER_SLICE` stays at the brief's 400,000.** It
  was not tuned: the implementer has no browser, and Bun's
  JavaScriptCore is not the browser's engine. With R-T8-1 the slice
  length no longer trades against throughput (a capped slice is followed
  at once by the next), so its only costs are how long a button or
  `frameAck` message waits behind a running slice (at most one slice,
  about 9.9 ms in Chrome and 9.8 ms in Bun at the Task 7 rates) and the
  per-slice overhead (one `framebuffer()` copy of 153,600 bytes and the
  wall-clock reads). A press is held at least `PRESS_HOLD_US` (100 ms
  emulated) anyway. If wrong: a measured long-slice input lag would argue
  for 200,000; the smoke test's INP and the status line's speed show it.
- **R-T8-3, the Worker applies a message when it receives it, which is
  between slices.** A slice runs to completion on the Worker's one
  thread, so a message handled when it arrives takes effect at the start
  of the next slice, as the brief requires, with no queue in the Worker.
  Messages other than `init` that reach the Worker before the emulator
  has booted are ignored: the client queues everything it sends before
  `ready` and posts it in order on `ready` (pinned by `messages before
  the worker is ready are queued in order`), and `main.ts` hands out the
  runtime only after `ready`. If wrong: none while the client is the
  Worker's only sender.
- **R-T8-4, three additions to the brief's interfaces.**
  `FirmwareRuntimeOptions` gains `onFrame(result, wallDeltaMs)` beside
  `now` (the Worker reads each slice's `capped` for R-T8-1 and posts its
  status from it); `createWorkerFirmwareRuntime` takes an optional fifth
  `options` argument (an injectable `requestAnimationFrame`, for tests);
  and it returns a `WorkerFirmwareRuntime`, the `FirmwareRuntime` plus a
  `ready` promise. `main.ts` awaits `ready` (and the Worker's `error`
  event), so a failed boot (a missing WASM, a bad image) falls back to Lua
  mode exactly as before the Worker. If wrong: only interface shape.
- **R-T8-5, the client acknowledges a frame it does not paint.** A frame
  that arrives while the client is stopped, and one waiting for its
  animation frame at `stop()`, is dropped and acknowledged at once.
  Otherwise the Worker's one frame in flight would never be acknowledged
  after a mode switch and the firmware would never paint again, and a
  late firmware frame would land on the canvas after a switch to Lua
  mode. Only the latest frame is painted. If wrong: none.
- **R-T8-6, button names are checked with `hasOwnProperty`.**
  `FirmwareRuntime.injectButton` checked `RAW_SLOT_BY_BUTTON[button] ===
  undefined`, which a name like `toString` passes (it finds
  `Object.prototype.toString`). Input now crosses a message boundary, so
  both the Worker and the runtime check `isButtonName` (own property of
  `RAW_SLOT_BY_BUTTON`), and the Worker treats `pressed` as `=== true`;
  pinned by `a button message for an unknown name is ignored without
  throwing`. A valid name behaves exactly as before. If wrong: none.
- **R-T8-7, the WASM bytes are fetched on the main thread from the
  glue's own URL and compiled synchronously in the Worker.** `main.ts`
  fetches `new URL("./cpu/wasm-pkg/emulator_wasm_bg.wasm",
  import.meta.url)`, the `.wasm` beside the generated glue, which is
  where the glue's default `init()` looks; Vite emits one hashed asset for
  both references (the build's `dist/assets/` holds one `.wasm`). The
  Worker initializes with `initCpuWasmSync` (Chrome forbids synchronous
  compilation of a module over 4 KB only on the main thread) and imports
  the bridge lazily, so the bridge and the glue are a separate chunk the
  Worker chunk loads. Vite's `worker.format` is `"es"`, since the default
  `"iife"` cannot code-split. If wrong: a failed fetch or compile shows
  as a fallback to Lua mode with the error in the console.
- **R-T8-8, an error in a slice stops the loop and is posted.** A WASM
  trap (a Rust panic) inside `runUntil` is caught in the slice, posted as
  `error` (the client logs it to the console) and the loop stops; nothing
  reschedules a failed emulator. Boot failures are posted the same way.
  If wrong: none; before the Worker the same panic threw out of the
  `requestAnimationFrame` callback and the loop died silently.
- **R-T8-9, the status line can show a pre-reset state for one status
  interval.** `provision()` and `reset()` update the client's cached
  provisioning state (and `reset()` the speed) at once, as the in-thread
  runtime did, but a `status` the Worker posted before it handled the
  message can arrive after and show the old state until the next one, at
  most `STATUS_INTERVAL_MS` (250 ms) later. If wrong: a status line that
  is stale for a quarter second after a reset.
- **R-T8-10, a started runtime pauses while the page is hidden**
  (review finding, fix round 1). A dedicated Worker is not paused like
  main-thread `requestAnimationFrame`, so after the move the firmware
  kept running in a background tab: Snake played on unseen and died, and
  a busy screen kept a core busy, while only the painting stopped. The
  client now listens to `document.visibilitychange` (an injectable
  `VisibilitySource`): on hidden it posts `stop` (the Worker's
  `FirmwareRuntime.stop()` flushes any deferred release) and drops any
  frame waiting to be painted; on visible it posts `start`, whose first
  slice counts no hidden time. Only a runtime the user started is paused
  and resumed: a runtime stopped by a mode switch stays stopped, and
  `start()` while hidden waits for visible. This restores Milestone 6's
  behaviour. The Worker also clears `lastCapped` on `start`, so a restart
  yields through the timer (reviewer Minor 2). Pinned by `hidden tab
  pauses the worker and resumes on visible`, `a runtime the user stopped
  is not started by a visible tab`, `starting while hidden waits for
  visible, and frames while paused are acknowledged` and `a restart after
  a capped slice yields through the timer`. If wrong: none; the
  alternative is firmware that runs unseen.

### Task 9a: Dev server

- **R-T9a-1, the dependency pre-bundle gets the production build's
  `process.env` define.** `bun run dev` died at load with `Uncaught
  ReferenceError: process is not defined` (pre-existing; reproduced at
  `6cd0a6e`). Root cause: fengari's `src/luaconf.js` line 3 reads
  `process.env.FENGARICONF` at module top level with no `typeof process`
  guard. Every other `process` use in fengari (`lauxlib`, `lbaselib`,
  `lualib`, `linit`, `liolib`, `loslib`, `ldblib`, `loadlib`) sits in the
  Node branch of a `typeof process` guard, and its Node-only deps `tmp`
  and `readline-sync` are required only inside those branches. Vite 8.3.0's production build
  replaces `process.env` with `{}` (`vite:define`, non-lib builds), so
  `conf` is `{}` there; the dev optimizer (`prepareRolldownOptimizerRun`)
  defines only `process.env.NODE_ENV` plus
  `optimizeDeps.rolldownOptions.transform.define`, and dev serves
  `node_modules` deps only through that pre-bundle. The fix is
  `optimizeDeps.rolldownOptions.transform.define = { "process.env": "{}" }`
  in `frontend/vite.config.ts`: the same rule the production build applies,
  so fengari's config resolves to `{}` in both and the Lua sandbox is
  unchanged. Rejected: a top-level `define` (it does not reach the
  optimizer, and in dev Vite's `env.mjs` assigns each define onto
  `globalThis` at runtime, so `globalThis.process = { env: {} }` would
  make `typeof process` true and switch on fengari's Node paths, `io` and
  `process.stdout` among them); a narrower `process.env.FENGARICONF`
  define (same value today, but diverges from production for any other
  dep's `process.env`); a `process` shim (also flips the `typeof` guards);
  excluding fengari from the optimizer (it is CommonJS and must be
  pre-bundled in dev); patching `node_modules`. Pinned by `pre-bundled
  fengari evaluates without a process global` (`test/vite-prebundle.test.ts`),
  which runs the real optimizer with this config into a temporary cache
  directory and evaluates the output in a `node:vm` context with no
  `process`; it fails without the define. If wrong: a dep that needs a
  real `process.env` in dev would see `{}`, as it already does in the
  production build.

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
| Task 4 (measured at `7e7b1f0` plus Task 4's code) | 4 | native, Apple M5 Pro (18 cores), rustc 1.98.1 | release `opt-level = 3` | 40.10 | 0.611 | 0.796 | 1.003 | 15.4 |
| Task 4 (measured at `7e7b1f0` plus Task 4's code) | 4 | WASM, Bun 1.3.14 (JavaScriptCore), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 25.80 | 0.377 | 0.550 | 0.707 | n/a |
| Task 5 (measured at `c54e8c8` plus Task 5's code) | 5 | native, Apple M5 Pro (18 cores), rustc 1.98.1 | release `opt-level = 3` | 52.25 | 0.788 | 1.026 | 1.289 | 10.6 |
| Task 5 (measured at `c54e8c8` plus Task 5's code) | 5 | WASM, Bun 1.3.14 (JavaScriptCore), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 32.39 | 0.476 | 0.709 | 0.886 | n/a |
| Task 6 (measured at `f23288a` plus Task 6's code) | 6 | native, Apple M5 Pro (18 cores), rustc 1.98.1 | release `opt-level = 3` | 66.74 | 1.020 | 1.313 | 1.649 | 9.7 |
| Task 6 (measured at `f23288a` plus Task 6's code) | 6 | WASM, Bun 1.3.14 (JavaScriptCore), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 38.01 | 0.557 | 0.858 | 1.089 | n/a |
| Task 7 (measured at `fb968ad` plus Task 7's code) | 7 | native, Apple M5 Pro (18 cores), rustc 1.98.1 | release `opt-level = 3` | 69.43 | 1.077 | 1.400 | 1.756 | 8.0 |
| Task 7 (measured at `fb968ad` plus Task 7's code) | 7 | WASM, Bun 1.3.14 (JavaScriptCore), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 40.77 | 0.609 | 0.972 | 1.211 | n/a |
| Task 7 (measured at `abd4c3a`, one run alone) | 7 | WASM, Chrome 155 (bench page, Worker), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 40.51 | 0.601 | 0.897 | 1.113 | — |
| Task 7 core (measured at `ea7a035`, core = `abd4c3a`, median of 3; R-T7-7) | 7 | WASM, Safari 27.0.1 (bench page, Worker), Apple M5 Pro | release `opt-level = 3` (wasm-pack) | 41.78 | 0.627 | 0.990 | 1.234 | — |

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

### Task 4: interrupt routing over pending bits, precomputed line gate

`InterruptController::routed_lines` visits only the set bits of
`pending_sources` (a `trailing_zeros` loop, the spec's prototype);
`poll` is `routed_lines(pending) & line_gate`, where `line_gate` (lines
enabled, priority != 0, priority >= threshold) is recomputed on every
intc write and checked against a fresh computation by a
`debug_assert_eq!` on every `poll` (R-T4-1 to R-T4-4).
`UsbSerialJtag`'s `HashMap<u32, u32>` register store is a fixed array
(R-T4-5), so the per-step `pending_sources` reads `INT_ENA` without
hashing. Every pin held. Native `BENCH` lines:

```
BENCH walk steps=372597011 wall_s=9.29 msteps_per_s=40.10
BENCH home emulated_ms=254.0 steps=19106041 busy_pct=94.0 wall_ms_median=415.7 wall_ms_min=414.8 factor=0.611 msteps_per_s=45.96
BENCH down emulated_ms=115.0 steps=6858235 busy_pct=74.6 wall_ms_median=144.4 wall_ms_min=144.4 factor=0.796 msteps_per_s=47.48
BENCH onboarding emulated_ms=67.0 steps=3018254 busy_pct=56.4 wall_ms_median=66.8 wall_ms_min=66.7 factor=1.003 msteps_per_s=45.18
```

A second native run read within 4 % (walk 39.42 Msteps/s, factors
0.607, 0.774, 0.995).

Bun (`bun run bench:wasm`, median of 3; the runs agreed within 3 % on
the walk and 4 % on every redraw):

```
BENCH host="Bun 1.3.14 (JavaScriptCore), Apple M5 Pro, darwin arm64" commit=7e7b1f0+dirty profile=release opt-level=3
BENCH median runs=3 walk msteps_per_s=25.80
BENCH median runs=3 home emulated_ms=349.8 steps=19286041 wall_ms=679.7 factor=0.515 msteps_per_s=28.38 pin_factor=0.377
BENCH median runs=3 down emulated_ms=115.6 steps=6908235 wall_ms=210.5 factor=0.549 msteps_per_s=32.81 pin_factor=0.550
BENCH median runs=3 onboarding emulated_ms=240.0 steps=3335609 wall_ms=104.8 factor=2.291 msteps_per_s=31.84 pin_factor=0.707
```

(`+dirty`: Task 4's uncommitted code.) Against Task 3: native redraws
2.0x to 2.1x the steps/s (HOME 22.75 to 45.96 M), the walk 1.93x; Bun
redraws 1.98x to 2.14x (pin factors 0.191, 0.257, 0.340 to 0.377,
0.550, 0.707), the walk 1.88x. Native onboarding reaches 1.0x.

WASM size: 105,110 bytes at Task 3, 100,002 bytes now (-5,108 bytes;
the USB store's `HashMap` code is gone).

Check list, all green: `cargo fmt --all` clean; `cargo test --workspace`
536 lib (Task 4's two new intc tests) + 95 integration pass (64 boot
rungs, 1 ignored), 18.3 s wall, `boot_progress` 15.5 s with the gate
cross-check on every step; release `boot_progress` 64 pass, 1 ignored,
15.4 s wall (14.1 s in the harness); clippy the same 3 warnings;
`bun run build:wasm` and `bun run typecheck` ok; `bun test` 149 pass,
0 fail, 15.9 s.

### Task 5: word-sized bus fast path

`FirmwareBus`'s `read16`/`read32` try the XIP aperture (an access inside
one 64 KiB MMU page, translated once; an invalid entry falls back to the
byte path, which logs as before), then one resolved RAM span (R-T5-1);
`write16`/`write32` try RAM spans only; `fetch16` tries IBUS within one
page, then a RAM span, then the original `is_mapped` path (R-T5-4).
Everything else is the byte path, unchanged. Every pin held. Native
`BENCH` lines:

```
BENCH walk steps=372597011 wall_s=7.13 msteps_per_s=52.25
BENCH home emulated_ms=254.0 steps=19106041 busy_pct=94.0 wall_ms_median=322.3 wall_ms_min=321.3 factor=0.788 msteps_per_s=59.29
BENCH down emulated_ms=115.0 steps=6858235 busy_pct=74.6 wall_ms_median=112.1 wall_ms_min=112.1 factor=1.026 msteps_per_s=61.17
BENCH onboarding emulated_ms=67.0 steps=3018254 busy_pct=56.4 wall_ms_median=52.0 wall_ms_min=51.8 factor=1.289 msteps_per_s=58.08
```

A second native run read within 2 % (walk 53.09 Msteps/s, factors
0.795, 1.031, 1.311). The profiled run (the same code built with debug
info, under samply) read faster still: walk 55.86, factors 0.856, 1.110,
1.388; the table keeps the plain build.

Bun (`bun run bench:wasm`, median of 3; three invocations agreed within
1 % on the walk and 1.5 % on every pin factor; this is the third):

```
BENCH host="Bun 1.3.14 (JavaScriptCore), Apple M5 Pro, darwin arm64" commit=c54e8c8+dirty profile=release opt-level=3
BENCH median runs=3 walk msteps_per_s=32.39
BENCH median runs=3 home emulated_ms=349.8 steps=19286041 wall_ms=539.0 factor=0.649 msteps_per_s=35.78 pin_factor=0.476
BENCH median runs=3 down emulated_ms=115.6 steps=6908235 wall_ms=163.4 factor=0.708 msteps_per_s=42.28 pin_factor=0.709
BENCH median runs=3 onboarding emulated_ms=240.0 steps=3335609 wall_ms=83.5 factor=2.873 msteps_per_s=39.93 pin_factor=0.886
```

(`+dirty`: Task 5's uncommitted code.) Against Task 4: native redraws
1.29x the steps/s (HOME 45.96 to 59.29 M), the walk 1.30x; Bun redraws
1.25x to 1.29x (pin factors 0.377, 0.550, 0.707 to 0.476, 0.709, 0.886),
the walk 1.26x. Native DOWN and onboarding now beat the badge; HOME is at
0.79x.

WASM size: 100,002 bytes at Task 4, 106,475 bytes now (+6,473 bytes;
the fast paths are inlined into the four word accessors and `fetch16`).

Check list, all green: `cargo fmt --all` clean; `cargo test --workspace`
540 lib (Task 5's four bus tests) + 95 integration pass (64 boot rungs,
1 ignored), 13.0 s wall, `boot_progress` 12.3 s; release `boot_progress`
64 pass, 1 ignored, 10.6 s wall; clippy the same 3 warnings;
`bun run build:wasm` and `bun run typecheck` ok; `bun test` 149 pass,
0 fail, 13.0 s.

### Task 6: ROM-stub range precheck and per-step trims

`RomStubTable::lookup` answers outside its registered range with two
compares (R-T6-1); `System` decodes the CPU clock on a clock-register
write (R-T6-2); `Clock::step` skips the division on a step that
completes no tick (R-T6-3); the interrupt poll routes only sources that
can reach a gated line (R-T6-4), with the gate and that mask rebuilt
only when an input changes (R-T6-5); `tick_peripherals` and
`asserted_lines` are inlined into the step loop (R-T6-6). Every pin
held. Native `BENCH` lines:

```
BENCH walk steps=372597011 wall_s=5.58 msteps_per_s=66.74
BENCH home emulated_ms=254.0 steps=19106041 busy_pct=94.0 wall_ms_median=249.1 wall_ms_min=248.4 factor=1.020 msteps_per_s=76.70
BENCH down emulated_ms=115.0 steps=6858235 busy_pct=74.6 wall_ms_median=87.6 wall_ms_min=87.3 factor=1.313 msteps_per_s=78.32
BENCH onboarding emulated_ms=67.0 steps=3018254 busy_pct=56.4 wall_ms_median=40.6 wall_ms_min=40.5 factor=1.649 msteps_per_s=74.29
```

A second native run read within 2 % (walk 67.03 Msteps/s, factors
1.028, 1.337, 1.664); the profiled build read walk 67.26, factors 1.038,
1.342, 1.671. The table keeps the first plain run.

Bun (`bun run bench:wasm`, median of 3; a second invocation agreed
within 2 % on the walk and every pin factor: 38.59, 0.563, 0.867, 1.103):

```
BENCH host="Bun 1.3.14 (JavaScriptCore), Apple M5 Pro, darwin arm64" commit=f23288a+dirty profile=release opt-level=3
BENCH median runs=3 walk msteps_per_s=38.01
BENCH median runs=3 home emulated_ms=349.8 steps=19286041 wall_ms=460.3 factor=0.760 msteps_per_s=41.90 pin_factor=0.557
BENCH median runs=3 down emulated_ms=115.6 steps=6908235 wall_ms=135.1 factor=0.856 msteps_per_s=51.15 pin_factor=0.858
BENCH median runs=3 onboarding emulated_ms=240.0 steps=3335609 wall_ms=68.0 factor=3.529 msteps_per_s=49.04 pin_factor=1.089
```

(`+dirty`: Task 6's uncommitted code.) Against Task 5: native redraws
1.28x to 1.29x the steps/s (HOME 59.29 to 76.70 M), the walk 1.28x; Bun
pin factors 1.17x to 1.23x (0.476, 0.709, 0.886 to 0.557, 0.858, 1.089),
the walk 1.17x. Native HOME now reaches the badge's speed (1.02x); under
Bun, onboarding does (1.09x) and HOME is at 0.56x.

WASM size: 106,475 bytes at Task 5, 110,286 bytes now (+3,811 bytes;
mostly R-T6-6's inlining).

Check list, all green: `cargo fmt --all` clean; `cargo test --workspace`
543 lib (Task 6's three new tests: two in `cpu::rom_stubs`, one in
`peripherals::intc`) + 95 integration pass (64 boot rungs, 1 ignored),
14.0 s wall, `boot_progress` 10.9 s with the debug cross-checks on every
step; release `boot_progress` 64 pass, 1 ignored, 9.7 s wall (8.4 s in
the harness); clippy the same 3 warnings; `bun run build:wasm` and
`bun run typecheck` ok; `bun test` 149 pass, 0 fail, 11.4 s.

### Task 7: decoded-instruction cache

`Cpu` holds a `DecodeCache` (`cpu/decode_cache.rs`): a direct-mapped
table of `1 << 14` entries indexed by `(pc >> 1) & mask`, validated by
the raw instruction bits on every hit (R-T7-1) and tagged by them alone
(R-T7-3). `Cpu::step` is unchanged up to the fetch: interrupt entry,
then the ROM-stub check, then both `fetch16`s exactly as before;
`fetch_and_decode` then looks the bits up and decodes only on a miss.
`raw` is the halfword zero-extended for a 16-bit instruction and the
whole word for a 32-bit one. Every pin held. Native `BENCH` lines:

```
BENCH walk steps=372597011 wall_s=5.37 msteps_per_s=69.43
BENCH home emulated_ms=254.0 steps=19106041 busy_pct=94.0 wall_ms_median=235.9 wall_ms_min=235.2 factor=1.077 msteps_per_s=80.98
BENCH down emulated_ms=115.0 steps=6858235 busy_pct=74.6 wall_ms_median=82.2 wall_ms_min=81.7 factor=1.400 msteps_per_s=83.48
BENCH onboarding emulated_ms=67.0 steps=3018254 busy_pct=56.4 wall_ms_median=38.1 wall_ms_min=38.0 factor=1.756 msteps_per_s=79.12
```

Native runs of the same code varied more than in earlier tasks (HOME
1.069x to 1.104x over five runs, the profiled build included), so the
cache was also measured against itself switched off (the lookup and
insert compiled out, everything else equal), alternating, in one
session: off 0.999x and 0.981x HOME, on 1.103x and 1.102x; DOWN 1.288x
and 1.261x against 1.440x and 1.439x; onboarding 1.613x and 1.591x
against 1.801x and 1.792x; the walk 65.6 and 65.1 against 70.8 and 70.7
Msteps/s. Like for like, the cache is worth 1.10x to 1.14x natively. The
table keeps the first plain run.

Bun (`bun run bench:wasm`, median of 3; a second invocation agreed
within 2 % on the walk and every pin factor: 40.74, 0.603, 0.953, 1.186):

```
BENCH host="Bun 1.3.14 (JavaScriptCore), Apple M5 Pro, darwin arm64" commit=fb968ad+dirty profile=release opt-level=3
BENCH median runs=3 walk msteps_per_s=40.77
BENCH median runs=3 home emulated_ms=349.8 steps=19286041 wall_ms=421.1 factor=0.831 msteps_per_s=45.80 pin_factor=0.609
BENCH median runs=3 down emulated_ms=115.6 steps=6908235 wall_ms=119.2 factor=0.970 msteps_per_s=57.94 pin_factor=0.972
BENCH median runs=3 onboarding emulated_ms=240.0 steps=3335609 wall_ms=61.1 factor=3.927 msteps_per_s=54.57 pin_factor=1.211
```

(`+dirty`: Task 7's uncommitted code.) Against Task 6: Bun pin factors
1.09x to 1.13x (0.557, 0.858, 1.089 to 0.609, 0.972, 1.211), the walk
1.07x. **HOME under Bun is still short of the badge's speed, at a pin
factor of 0.609** (DOWN 0.972, onboarding 1.211); for Task 10, the
remaining native profile is the Task 7 table under Profiles: instruction
execution and its dispatch (`execute` 24.7 %, `Cpu::step` 15.3 %), the
per-step interrupt sample and clock in `step_with_interrupts` (20.5 %),
and `fetch16` (9.0 %). Only a cache that skips the per-step fetch and
dispatch (a block cache) or the per-step peripheral sample (spec Part 2's
audit) attacks the first two.

Cache size: 262,144 bytes per `Cpu` (16,384 entries of 16 bytes), so per
`FirmwareRuntime` clone and per runtime in WASM linear memory. The
clone-heavy release suite went from 9.7 s to 8.0 s wall (R-T7-4).

WASM size: 110,286 bytes at Task 6, 110,768 bytes now (+482 bytes).

Check list, all green: `cargo fmt --all` clean; `cargo test --workspace`
549 lib (Task 7's six new tests: three in `cpu`, two in
`cpu::decode_cache`, one in `mem::bus`) + 95 integration pass (64 boot
rungs, 1 ignored), 14.9 s wall, `boot_progress` 11.6 s; release
`boot_progress` 64 pass, 1 ignored, 8.0 s wall (7.9 s in the harness);
clippy the same 3 warnings; `bun run build:wasm` and `bun run typecheck`
ok; `bun test` 149 pass, 0 fail, 10.8 s.

### Task 8: Web Worker host

The firmware emulator runs in a dedicated module Worker
(`frontend/src/runtime/firmware-worker.ts`); the main thread fetches the
WASM bytes and `factory.bin`, forwards input and paints the frames the
Worker posts (`firmware-worker-client.ts`). Slices: wall delta clamped to
`MAX_FRAME_DELTA_MS` (50 ms), at most `WORKER_STEPS_PER_SLICE` (400,000)
steps, debt dropped; a capped slice yields through a `MessageChannel`,
any other through `setTimeout(…, 0)` (R-T8-1); one frame in flight;
`status` at most every 250 ms. No `emulator-core` or `emulator-wasm`
change, so no benchmark row: the core's numbers are Task 7's.

Check list (frontend only), all green: `bun test` 167 pass, 0 fail,
10.1 s (Task 8 adds 18 in `test/firmware-worker.test.ts`); `bun run
typecheck` ok; `bun run build` ok, `dist/` holds `index.html`, `apps/`,
`firmware/` and `assets/` (the main chunk, `firmware-worker-*.js`,
`bridge-*.js`, one `emulator_wasm_bg-*.wasm`, the CSS) and no bench page.

Fix round 1 (`34625ff`, R-T8-10): the client pauses a started runtime
while the page is hidden. `bun test` 171 pass, 0 fail, 10.2 s;
`bun run typecheck` and `bun run build` ok, no bench page in `dist/`.

Browser smoke test (orchestrator, Apple M5 Pro, Chrome 155, `ea7a035`).
The dev server (`bun run dev`) page dies with `Uncaught ReferenceError:
process is not defined`: fengari's `luaconf` reads
`process.env.FENGARICONF`, and Vite 8.3.0's dependency pre-bundle does
not shim it. This is pre-existing (reproduced at `abd4c3a` and at
`6cd0a6e`) and not fixed here; the human partner decides at Task 9. The
smoke test therefore ran on the production build (`bun run build && bun
run preview`):

- Real Firmware boots in the Worker to the first-run screen. Provision
  (hacker) reaches PROV OK in about 1 s; the full onboarding walk works,
  including Shake; Finished setup leads to the registered screen, HOME to
  the launcher, and DOWN moves the selection.
- Snake opens and runs at a steady 1.0x to game over. Switching to Lua
  with DOWN held and back resumes painting, and A restarts Snake.
- Status-line minimum speed (250 ms smoothed, so coarse): onboarding page
  turns 0.7x to 1.0x, HOME 0.7x, launcher DOWN 0.7x to 0.8x.
- A `PerformanceObserver` over the whole session saw 0 long tasks
  (> 50 ms) on the main thread and 0 key event-timing entries of 16 ms
  or more: the INP for a launcher DOWN is under 16 ms (synthetic key
  events), against 64 ms before the Worker (spec criterion 4).

The browser bench rows after Task 7 (Chrome 155 and Safari 27.0.1) are in
the table above; the bench page runs the bridge directly in its own
Worker, not the app's Worker host, so they measure the core.

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

### Task 3 (`7e7b1f0`, release `opt-level = 3`), before Task 4

28,981 samples on the `bench_redraws` thread (this run: walk 20.40
Msteps/s, HOME 0.291x).

| Share | Symbol |
| --- | --- |
| 38.8 % | `FirmwareBus::asserted_lines` (with `InterruptController::poll` and the 64-source `routed_lines` scan inlined into it) |
| 9.6 % | `FirmwareBus::read_byte` |
| 7.5 % | `cpu::decode::decode_16` |
| 5.7 % | `UsbSerialJtag::pending_sources` (the `HashMap` lookup of `INT_ENA`) |
| 5.6 % | `Cpu::step` |
| 5.6 % | `RomStubTable::lookup` |
| 4.9 % | `FirmwareBus::tick_peripherals` |
| 4.7 % | `cpu::decode::decode_32` |
| 4.1 % | `FirmwareBus::fetch16` |
| 3.8 % | `cpu::execute::execute` |

Below the top 10: the `bench_redraws` closure 3.6 %, `write_byte` 2.6 %,
`SysTimer::advance_by` 1.0 %, `step_with_interrupts` 0.7 %.

### Task 4 (interrupt routing and line gate, release `opt-level = 3`)

14,894 samples on the `bench_redraws` thread (this run: walk 40.16
Msteps/s, HOME 0.634x).

| Share | Symbol |
| --- | --- |
| 17.0 % | `FirmwareBus::read_byte` |
| 12.5 % | `Cpu::step` |
| 11.3 % | `RomStubTable::lookup` |
| 9.6 % | `FirmwareBus::tick_peripherals` |
| 9.1 % | `FirmwareBus::asserted_lines` (now with `UsbSerialJtag::pending_sources` inlined) |
| 9.1 % | `cpu::decode::decode_16` |
| 7.3 % | `FirmwareBus::fetch16` |
| 7.0 % | the `bench_redraws` closure (the benchmark's own framebuffer hashing) |
| 5.5 % | `cpu::decode::decode_32` |
| 4.6 % | `FirmwareBus::write_byte` |

Below the top 10: `SysTimer::advance_by` 2.3 %, `step_with_interrupts`
1.3 %, `FirmwareRuntime::run_impl` 1.0 %. The interrupt path (poll plus
the USB source) went from 44.5 % to 9.1 %; the profile is now the flat
one the spec predicted, led by byte-wise bus access (Task 5) and the
ROM-stub lookup and decode on every fetch (Tasks 6 and 7).

### Task 5 (word-sized bus fast path, release `opt-level = 3`)

11,025 samples on the `bench_redraws` thread (this run: walk 55.86
Msteps/s, HOME 0.856x).

| Share | Symbol |
| --- | --- |
| 15.7 % | `RomStubTable::lookup` |
| 13.4 % | `Cpu::step` |
| 12.3 % | `FirmwareBus::tick_peripherals` |
| 11.3 % | `cpu::decode::decode_16` |
| 11.1 % | `FirmwareBus::asserted_lines` |
| 9.3 % | the `bench_redraws` closure (the benchmark's own framebuffer hashing) |
| 7.4 % | `FirmwareBus::fetch16` |
| 5.8 % | `cpu::decode::decode_32` |
| 2.4 % | `SysTimer::advance_by` |
| 1.8 % | `step_with_interrupts` |

Below the top 10: `write_byte` 1.6 %, `read32` 1.6 %, `read_byte` 1.3 %,
`run_until_stable_frame` 1.3 %, `FirmwareRuntime::run_impl` 1.1 %,
`write32` 1.1 %. Byte-wise bus access (`read_byte` plus `write_byte`)
went from 21.6 % to 2.9 % of a shorter profile (14,894 to 11,025
samples); `fetch16` now reads through the fast path and costs about
what it did as a share, about a quarter less in samples. The ROM-stub
lookup on every fetch (Task 6) and decode (Task 7) lead what is left.

### Task 6 (ROM-stub precheck and per-step trims, release `opt-level = 3`)

Before: Task 5's code re-profiled at `f23288a` (11,051 samples; this run:
walk 55.69 Msteps/s, HOME 0.858x), within a point of the Task 5 table
above:

| Share | Symbol |
| --- | --- |
| 16.2 % | `RomStubTable::lookup` |
| 13.8 % | `Cpu::step` |
| 11.5 % | `FirmwareBus::tick_peripherals` |
| 11.5 % | `FirmwareBus::asserted_lines` |
| 11.2 % | `cpu::decode::decode_16` |
| 9.2 % | the `bench_redraws` closure (the benchmark's own framebuffer hashing) |
| 6.9 % | `FirmwareBus::fetch16` |
| 5.9 % | `cpu::decode::decode_32` |
| 2.4 % | `SysTimer::advance_by` |
| 1.8 % | `FirmwareBus::read32` |

Below the top 10: `step_with_interrupts` 1.8 %, `write_byte` 1.6 %,
`InterruptController::write_byte` 0.3 %.

After: 9,329 samples (this run: walk 67.26 Msteps/s, HOME 1.038x).

| Share | Symbol |
| --- | --- |
| 23.6 % | `step_with_interrupts` (now with `tick_peripherals` and `asserted_lines` inlined into it) |
| 13.0 % | `cpu::decode::decode_16` |
| 12.8 % | `Cpu::step` (with `RomStubTable::lookup`'s range check inlined) |
| 11.0 % | the `bench_redraws` closure (the benchmark's own framebuffer hashing) |
| 9.1 % | `cpu::execute::execute` (no longer inlined into `Cpu::step`) |
| 8.8 % | `FirmwareBus::fetch16` |
| 6.6 % | `cpu::decode::decode_32` |
| 2.9 % | `SysTimer::advance_by` |
| 1.9 % | `FirmwareBus::read32` |
| 1.9 % | `FirmwareBus::write_byte` |

Below the top 10: `read_byte` 1.5 %, `run_until_stable_frame` 1.4 %,
`FirmwareRuntime::run_impl` 1.4 %, `write32` 1.3 %,
`advance_timed_peripherals` 0.8 %, `InterruptController::write_byte`
0.2 %. In samples: the ROM-stub lookup (1,790) is gone into `Cpu::step`'s
two compares; the interrupt sample, the clock tick and the step glue went
from 2,741 samples (24.8 %) to 2,202 (23.6 % of a shorter profile).
Fetch and decode (`decode_16`, `decode_32`, `fetch16`: 28.4 %) lead
what is left, which is Task 7's decode cache.

### Task 7 (decoded-instruction cache, release `opt-level = 3`)

Before: the Task 6 "After" table above (9,329 samples). After: 9,087
samples (this run: walk 69.38 Msteps/s, HOME 1.069x).

| Share | Symbol |
| --- | --- |
| 24.7 % | `cpu::execute::execute` |
| 20.5 % | `step_with_interrupts` (with `tick_peripherals` and `asserted_lines` inlined) |
| 15.3 % | `Cpu::step` (with `fetch_and_decode` and the cache lookup inlined) |
| 11.7 % | the `bench_redraws` closure (the benchmark's own framebuffer hashing) |
| 9.0 % | `FirmwareBus::fetch16` |
| 2.7 % | `FirmwareBus::read32` |
| 2.7 % | `SysTimer::advance_by` |
| 1.8 % | `FirmwareBus::write_byte` |
| 1.7 % | `FirmwareBus::write32` |
| 1.7 % | `FirmwareBus::read_byte` |

Below the top 10: `run_until_stable_frame` 1.5 %, `idle_until` 1.5 %,
`UsbSerialJtag::advance` 1.2 %, `FirmwareRuntime::run_until` 1.2 %,
`decode_16` 0.6 % and `decode_32` 0.4 % (misses only). In samples,
decode went from 1,834 (`decode_16` 1,216, `decode_32` 618) to 87, and
`Cpu::step` rose from 1,191 to 1,386 (the lookup), but `execute` rose
from 852 to 2,247: much of what the profile charged to decode was the
host's mispredicted branches on the instruction's kind, which now land
on `execute`'s `match` over the cached `Instruction` instead. The net
gain is the 1.10x to 1.14x measured above, not the 20 % decode share.
Fetch (`fetch16`, 9.0 %) is under R-T7-1's 10 % bar (R-T7-6).

## Milestone 8 backlog

Filled in by Task 10: what this milestone leaves (a block cache first if
HOME is still short of 1.0x), plus everything carried from
`docs/milestone-6-decisions.md`'s Milestone 7 backlog except the
throughput item.

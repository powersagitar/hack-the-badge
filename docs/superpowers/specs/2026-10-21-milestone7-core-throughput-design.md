# Milestone 7 — Core throughput and a Web Worker host

Status: scope agreed in conversation with the human partner (2026-10-21);
written spec awaiting the human partner's approval. Implementation plan:
`docs/superpowers/plans/2026-10-21-milestone7-core-throughput.md` (to be
written after approval).

## Goal

Make the badge's busy screens play at the physical badge's speed in the
browser, and make input never wait behind a frame.

At the end of Milestone 6 (main @ `6cd0a6e`) the real-firmware emulator is
correct but slow: the core runs about 14M steps/s natively and 12M in the
browser, against the badge's 80M cycles/s (CPI 1 holds, so one step is one
cycle). Idle-heavy screens (Snake, the first-run screen) still run at about
1.0x because the WFI fast-forward skips idle time, but busy work runs in
slow motion. On the launcher and in onboarding a full-screen redraw
visibly sweeps top to bottom (LVGL paints it in 12 bands; measured), and a
press does nothing until the frame catches up. The human partner judged
both "terrible" at Milestone 6's browser gate (INP 64 ms, 120 to 60 fps
dips on each launcher move) and backlogged it as this milestone.

The human partner's bar: **match the badge side by side**, unless the
browser or the machine is shown to be the limit. Reference setup: Safari
and Chrome on the human partner's MacBook Pro (Apple M5 Pro, 64 GB).

## Measured baseline (2026-10-21, M5 Pro, `6cd0a6e`)

Measured with a throwaway benchmark (scratch copy of the repo; a
committed version is Part 0). "Real-time factor" is emulated time over
wall time for the same input schedule; 1.0x is the badge's speed.

### What real time requires

Each redraw is measured from the press (held `PRESS_HOLD_US` = 100 ms of
emulated time, as in the rungs) to its last frame change, sampled every
1 ms of emulated time (role hacker):

| Redraw | Emulated duration | Steps | Busy (by ticks) | Steps/s for 1.0x |
|---|---|---|---|---|
| HOME: registered screen → launcher | 254 ms | 19.1M | 94 % | **75M** |
| DOWN on the launcher | 115 ms | 6.86M | 75 % | **60M** |
| A: onboarding page 1 → 2 | 67 ms | 3.02M | 56 % | **45M** |

### Where the core is

| Build | Native steps/s | WASM steps/s (Bun / JavaScriptCore) | DOWN real-time factor (native) |
|---|---|---|---|
| release `opt-level = "s"` (today) | 13.9M | 12.1M | 0.23x |
| `opt-level = 3` | 23.2M | 15.7M | 0.39x |
| `opt-level = 3` + fat LTO, 1 codegen unit | 23.6M | not measured | 0.40x |
| `opt-level = 3` + interrupt-routing fix (below) | 39.4M | not measured | 0.66x |

- Native numbers: the walk to hacker's launcher (372.6M steps) took
  27.7 s at "s", 17.6 s at 3, 10.8 s with the routing fix; per-redraw
  steps/s matched within 3 %.
- WASM: boot steps 4.4M..17.4M under Bun 1.x (JavaScriptCore, Safari's
  engine), best of 3. WASM runs at about 70 % of native. Chrome (V8) was
  not measured yet; Part 0 measures it.
- WASM size: 94,419 bytes at "s", 105,110 at 3.
- The release `boot_progress` suite took about 42 s at "s" (Milestone 6);
  the bench walk suggests about 2.5x less with both changes.

### Where the time goes (samply, native, `opt-level = 3`)

Shares of emulation time (the test harness's waiting thread excluded):

1. **Before the routing fix:** `step_with_interrupts` self time was 40 %,
   about 32 % in one loop: `InterruptController::routed_lines` scans all
   64 source `MAP` registers on every step whenever any source is pending
   (some source almost always is, routed or not). Prototype fix: visit
   only the set bits of `pending_sources` (`trailing_zeros` loop). Every
   pinned hash on the walk to the launcher still matched; 1.65x overall.
2. **After it, the profile is flat:**
   - bus data access one byte at a time (`FirmwareBus::read_byte` 14 %,
     `write_byte` 4 %; word accesses split into byte calls through the
     ordered region list);
   - per-step interrupt polling, about 23 %: `asserted_lines` 14 % plus
     `UsbSerialJtag::pending_sources` 9 %, which reads `INT_ST`/`INT_ENA`
     out of a `HashMap<u32, u32>` register store;
   - instruction decode 12 % (`decode_16` 8 %, `decode_32` 5 %), plus
     `fetch16` 6 %;
   - the ROM-stub check on every fetch (`RomStubTable::lookup`) 10 %;
   - `tick_peripherals` 7 %, `Cpu::step` and `execute` about 13 %.

My estimate (not a commitment): removing the per-step costs plus a decoded
instruction cache brings native to about 70M to 80M and WASM to about 50M
to 55M steps/s: real time for onboarding, probably for DOWN, short for
HOME. A block cache could close the rest; the human partner ruled it out of
this milestone (below).

## Success criteria

1. **Equivalence (the hard rule).** Pure performance work does not change
   emulated behaviour. Every step-exact rung, step count, step budget
   outcome, console needle, emulated time and frame hash in
   `cargo test --workspace`, the release `boot_progress` suite and
   `bun test` (including `test/cpu-wasm.test.ts`) stays byte-identical to
   `6cd0a6e`. A pin that moves is a bug in the change, never a re-pin.
   Timing may change only in the browser host (Part 4), never in
   `emulator-core`.
2. **The benchmark exists and is repeatable** (Part 0): one command per
   host (native, Bun, Chrome, Safari) prints steps/s and the real-time
   factor for the three redraws, checks each redraw's end frame against
   its pinned hash, and its baseline at `6cd0a6e` plus the numbers after
   each optimization task are recorded in
   `docs/milestone-7-decisions.md`.
3. **Throughput.** The target is 1.0x for all three redraws in Chrome and
   in Safari on the reference machine. The milestone is done when the
   planned optimizations (Parts 1 to 3) have landed under criterion 1 and
   their numbers are recorded; any redraw still under 1.0x is written up
   with its measured factor and its reason (a shown browser or machine
   limit, or the block-cache backlog item), not chased further.
4. **Input never waits on a frame** (Part 4): the emulator runs in a Web
   Worker; the main thread does no emulation. On the reference machine in
   Chrome, launcher moves show no main-thread long task (> 50 ms) from the
   emulator, and the INP that DevTools reports for a launcher move is
   recorded (it was 64 ms).
5. **The human partner judges the feel** at the browser gate: the launcher
   (HOME, DOWN) and onboarding, side by side with the badge, in Safari and
   Chrome. Snake must still play at about 1.0x (Milestone 6's gate).
6. The release `boot_progress` suite stays at or under its Milestone 6 time
   (about 42 s); its new time is recorded.

## Out of scope (backlog for later milestones)

Decided by the human partner: Milestone 7 is throughput and the Worker only.

- A block cache, trace cache or any JIT-style translation (named as the
  next milestone's throughput item if HOME is short of 1.0x).
- Every other `docs/milestone-6-decisions.md` backlog item: other built-in
  apps, RMT/LED decode, a console pane, NFC, `DeviceMotionEvent`, the RNG
  seed, the small cleanups, the step-counted `PROVISION_TIMEOUT_STEPS`,
  the unexplained Snake game-over delay, timing busy work against the
  badge, and the peripheral and test minors. They are carried to
  `docs/milestone-7-decisions.md`'s Milestone 8 backlog unchanged.

## Design

### Part 0 — A repeatable benchmark (first, before any optimization)

- **Native:** an `#[ignore]`d test in `emulator-core/tests/boot_progress.rs`
  (run with `--ignored --nocapture`), reusing its checkpoints and walk
  helpers. It times the walk to hacker's launcher and replays the three
  redraws above (press schedule as in the table; five timed repeats from a
  cloned checkpoint, median and min), and asserts each redraw ends on its
  pinned frame (`LAUNCHER_HASH`, `LAUNCHER_AFTER_NAV_HASH`, onboarding
  page 2's pinned hash). *My call:* a test, not an example, because the
  checkpoints and pins live in that file.
- **WASM:** one TypeScript benchmark module over the `src/cpu/bridge.ts`
  handle that boots `factory.bin`, provisions the fake hacker identity with
  the existing provisioner, walks onboarding to the registered screen and
  the launcher with the same inputs as the rungs, then times the three
  redraws (the WASM handle cannot clone, so each repeat re-walks or the
  redraws are timed once per walk in sequence; builder's choice, stated
  in the output). It checks the same end hashes. It runs:
  - from the command line under Bun (JavaScriptCore), as a script, not
    part of `bun test`;
  - in a browser, as a separate bench page served by the dev server (not
    linked from the app UI and not in the production build), printing a
    machine-readable result block that an automated driver can read.
    Chrome is driven through the Claude-in-Chrome extension; Safari
    through `safaridriver` (the human partner has enabled it).
- **What the browser number means** (*my call*): the bench page runs the
  emulator **unpaced inside a dedicated Worker** (the same thread setup as
  the app after Part 4, without frame posting or pacing), so its
  real-time factor shows headroom above 1.0x and how far short a redraw
  falls. Criterion 3's target is measured against this number. It needs
  only a minimal Worker wrapper, so Part 0 comes first and does not wait
  for Part 4. The app's own paced speed reading (which cannot exceed 1.0x)
  and the human's eye at the browser gate check the end-to-end result.
- Output per run: host, build profile, commit, walk steps/s, and per redraw
  emulated ms, steps, wall ms, steps/s and real-time factor.

### Part 1 — Build profile

Release `opt-level = 3` (measured 1.65x native, 1.30x WASM; +10.7 KB
WASM). LTO and `codegen-units = 1` gave under 2 % and are not adopted
unless a later measurement says otherwise (*my call*). The Milestone 5
note that the WASM build "would likely speed up the same way" is
answered by the table above.

### Part 2 — Per-step costs in the core

Each change ships with the full check list and a benchmark run; each
keeps criterion 1.

1. **Interrupt routing over set bits only** (`intc.rs` `routed_lines`;
   prototype above).
2. **Cheaper interrupt polling.** The per-step `asserted_lines` should not
   rebuild the pending mask from every peripheral and re-scan every line
   when nothing changed. The builder chooses the mechanism (for example
   caching the asserted-line mask and recomputing only after a peripheral
   register access, a timer tick or a host input that can change it), but
   it must deliver exactly the lines the per-step poll delivers today, at
   the same step. The bar is what the CPU sees on every step, not the
   mechanism. Because a peripheral can change its pending state without
   a register access (a SYSTIMER comparator on a tick, a USB packet
   arriving, a GDMA/SPI transfer completing), the change must come with
   (*my call*): an audit of every `pending_sources` contributor, listed
   in the decisions doc, and a cross-check mode, on in debug builds and
   tests, that compares the cached mask with the full poll on every step
   and panics on a mismatch, run over the whole `cargo test --workspace`
   suite. If that audit is large, the builder may schedule this item
   after items 3 to 5 and Part 3.
3. **USB-Serial-JTAG register store:** replace the `HashMap<u32, u32>` with
   plain fields or an array, at least for what the per-step path reads.
4. **Word-sized bus access fast path:** aligned 16- and 32-bit loads,
   stores and fetches to RAM and XIP flash resolve the region once instead
   of four ordered-region lookups per word. Peripheral registers keep
   their byte-split semantics wherever the model depends on them (the
   SYSTEM clock registers' after-the-step sampling, Milestone 6 Review
   Focus 3, is one such place). The ordered, concrete, named-field
   dispatch stays; no `dyn`. This is not an override: `read_byte`
   already checks the XIP cache aperture first and the RAM regions
   second, so a word path that runs those same two checks once per word,
   in that order, and falls back to today's byte path for everything else
   (including a word that straddles a region edge) keeps the existing
   order. A design that would reorder or bypass any other check is an
   override and goes to the human partner.
5. **ROM-stub check:** keep the interception exactly as today, but make
   the per-fetch test cheap (for example an address-range precheck before
   the table lookup, or folding it into the decode cache).
6. `tick_peripherals` and the rest of the step loop, as the profile after
   each change directs.

### Part 3 — Decoded-instruction cache

A cache of decoded instructions keyed by fetch address, in `cpu/`
(chip-agnostic). Requirements:

- **Exact invalidation.** Any write that can change the bytes a cached
  fetch would read invalidates the affected entries before the next
  fetch: stores to executable RAM (IRAM), the emulated flash erase and
  program paths, and any flash MMU table change that remaps an XIP page.
  An unmapped instruction fetch still traps exactly as today (never served
  from the cache). The cache is part of the `FirmwareRuntime` clone (a
  clone stays a full snapshot) or is rebuilt cold after a clone; the
  builder chooses, and either way behaviour is identical.
- The bus tells the core about invalidations through the existing
  generic `Bus` trait (a chip-agnostic hook), not through ESP32-C3
  constants in `cpu/`.
- Compressed (16-bit) and 32-bit instructions, including a 32-bit
  instruction that straddles two fetch units, behave exactly as today.
- A unit test proves self-modifying code (a store over a cached
  instruction) executes the new instruction.

### Part 4 — Web Worker host

The emulator moves off the main thread.

- A dedicated Worker owns the WASM `FirmwareEmulator`, the provisioner, the
  press-hold deferral (`PRESS_HOLD_US`), the shake schedule and the pacing
  loop (today's `stepFirmwareFrame` logic in
  `frontend/src/runtime/firmware-runtime.ts`).
- The main thread keeps the shell, the canvas and the input bindings, and
  talks to the Worker by messages: buttons, provision, shake, reset,
  start/stop; the Worker posts frames (transferable buffers, at most one in
  flight), the speed reading and the provisioning state.
- The `FirmwareRuntime` interface that `main.ts`/`shell.ts` use stays the
  same shape (synchronous getters become the last value the Worker posted),
  so the Lua mode and the mode toggle are untouched; the lazy dynamic
  import of the WASM bridge stays lazy.
- Pacing stays Milestone 6's rule: emulated time follows wall-clock time,
  each slice clamped (`MAX_FRAME_DELTA_MS`), debt dropped, a speed reading
  shown. The per-slice step cap (`DEFAULT_STEPS_PER_FRAME`, 150,000 per
  animation frame today, which alone caps the browser at about 18M
  steps/s at 120 fps) is re-derived for the Worker so it never binds below
  real time on the reference machine. These are host parameters; changing
  them is allowed by criterion 1.
- A press reaches the emulator at the start of the Worker's next slice;
  a slice never runs long enough to delay input or the next frame
  noticeably (*builder's choice of slice length*, recorded with its
  measured effect).
- The existing `bun test` coverage of the runtime (fake handle, pacing,
  capped frames, press hold, shake, provisioning) keeps passing, moved to
  wherever the logic now lives; the Worker message layer gets its own
  tests.

### Part 5 — Docs

`docs/milestone-7-decisions.md` (created in Task 1): rulings, every
benchmark result, and the Milestone 8 backlog (block cache first if HOME
is short, then everything carried). `docs/firmware-emulator-notes.md`
("Timing model", "Known limitations") and `CLAUDE.md` updated at the end
(the throughput figures, the Worker in the architecture, suite times).

## Constraints (carried from Milestone 6)

- `clock.rs` and `cpu/` stay chip-agnostic (no ESP32-C3 constant).
- `FirmwareBus` keeps ordered, concrete named fields, no `dyn` dispatch.
  If profiling shows this blocks throughput, that is an override of a
  recorded decision: ask the human partner.
- `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays
  logic-free (one-line passthroughs).
- Unmapped data access never panics; an unmapped instruction fetch always
  traps. Browser input never panics the emulator.
- The Milestone 3 to 6 decisions docs stand; overriding one needs the
  human partner's OK.
- Data handling as before: nothing from `local/full_flash_dump.bin`,
  `local/boot_log.txt` or the badge videos is committed, quoted or
  printed; committed tests and benchmarks depend only on `factory.bin`
  and the committed fake identities.
- After any change under `emulator-core/` or `emulator-wasm/`, run
  `bun run build:wasm` before `bun test`/`bun run dev`.

## Gates (stop and ask the human partner)

- Any hardware-compared frame hash moving: the registered screen,
  onboarding, the launcher and the launcher after DOWN, Dice open and roll
  (including the animation), Snake open, after-turn and game over. Under
  criterion 1 any moved pin is a bug anyway; these additionally stop the
  run.
- A release `boot_progress` suite over 90 s.
- Any override of a recorded decision.
- The browser gate (criterion 5), after the Worker lands.

## Decided by the human partner (2026-10-21)

- Scope: core throughput and the Web Worker only; all other backlog items
  wait for later milestones.
- Bar: match the badge side by side unless the browser or the hardware is
  the limit; reference setup Safari and Chrome on the M5 Pro MacBook Pro.
- If the planned work leaves HOME short of 1.0x, the milestone is done and
  the block cache goes to the next milestone.
- A repeatable benchmark that Claude can run itself (native, Bun, Chrome,
  Safari); `safaridriver` is enabled.
- Work in the worktree `.claude/worktrees/milestone-7` on branch
  `milestone-7` from main @ `6cd0a6e` (pushed).

## Left to the builder

Task order inside Parts 2 and 3 (driven by the profile after each step);
the interrupt-poll caching mechanism; the decode-cache layout, size and
clone policy; the bus fast-path shape; the Worker's slice length and step
cap; the benchmark's file names and output format.

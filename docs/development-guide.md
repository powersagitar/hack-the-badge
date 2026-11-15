# Development guide

How to build, test, debug and change the emulator, and how the project
was run. `CLAUDE.md` has the rules and the file map; this file is the
practice behind them.

## Setup

- **Bun** for everything JS/TS (never npm, yarn, pnpm or node). The root
  `package.json` forwards to `frontend/`, so commands work from either.
- **Rust** through `rustup`, with the `wasm32-unknown-unknown` target
  (the browser build) and `riscv32imc-unknown-none-elf` (for
  `tests/riscv_integration.rs`), plus `wasm-pack`.
- **Python**, only for the photo scripts and esptool, from
  `local/.venv/bin/python` (local-only), run with `-I`.

```sh
bun run install:frontend
bun run build:wasm        # regenerates frontend/src/cpu/wasm-pkg/ (gitignored)
bun run dev               # http://localhost:5173 (also /bench.html, /api/signal)
bun run dev -- --host     # serve the LAN, for two-device links
```

**Re-run `bun run build:wasm` after any change under `emulator-core/` or
`emulator-wasm/`**, before `bun run dev`, `bun run build` or `bun test`.
Nothing else regenerates it, and a stale package is the usual cause of
"my Rust change does nothing in the browser".

## The suites

| Command | What it guards | Time |
| ------- | -------------- | ---- |
| `cargo test -p emulator-core` | Unit tests of the core; the fast iteration loop | seconds |
| `cargo test -p emulator-core --release --test boot_progress` | The real-firmware **boot ratchet**: 97 rungs plus 1 ignored benchmark | about 20 s |
| `cargo test --workspace` | Everything in debug (782 tests, 1 ignored), with per-step cross-checks | about 30 s |
| `cargo clippy --workspace --all-targets` | Must stay warning-free | |
| `bun test` | 448 frontend tests in 27 files, including the WASM twins | about 26 s |
| `bun run typecheck` | `tsc --noEmit`, test files included | |
| `bun run build` | The production build | |

Counts are as of the end of Milestone 14.

**The boot ratchet** (`emulator-core/tests/boot_progress.rs`). Each rung
boots `firmware.bin` and asserts one thing: a console line, a point
reached with no fault, or a framebuffer hash. Rungs share cloned
`FirmwareRuntime` checkpoints held in `OnceLock`s, so a walk to the
launcher is paid once per role. Helpers in `tests/common/` provision,
walk onboarding, press buttons and wait for stable frames.

**The WASM twins.** `frontend/test/cpu-wasm.test.ts` drives the real
WASM build the way the Worker does (provision, walk to the launcher),
and `test/physics-shake.test.ts` shakes Setup's page 6 through the
lanyard. They catch a core that works natively but not in the browser.

**The equivalence rungs.** `execution_digest_is_unchanged` and
`hacker_walk_totals_are_unchanged` pin a digest of the whole execution
and the totals of the hacker walk. They are what makes performance work
safe.

## Pins and re-pins

Rungs pin exact values: step counts, emulated times, frame hashes,
console needles. The rules:

- **Performance work must not move any pin.** If one moves, the change
  is wrong.
- **A behaviour change may move pins**, and each moved pin is recorded
  in that milestone's decisions doc with the reason, old and new value
  (Milestone 9's "Re-pins (v335 → v392)" and Milestone 14's "Re-pins
  (Task 1)" are the models). A frame that changed is looked at, by the
  human partner when it is user-visible.
- When boot moves, update the rung whose budget it affects, and put the
  history in the notes, not in the test's doc comment.
- Frames compared with the physical badge carry that fact in the rung's
  doc. Only v335 frames were ever compared with hardware.

To see a frame, use `boot-probe --dump-frame` (below).

## Debugging a stuck boot

Almost all of Milestones 2 to 4, and every new app path since, went the
same way. The stall-by-stall log in the notes has dozens of worked
examples.

1. **Run the probe.**
   ```sh
   cargo run -p emulator-core --release --example boot-probe -- \
     --steps 20000000 --window 1000000 --dump-frame frame.png
   ```
   It prints the console, the hot PCs per window, unmapped accesses and
   framebuffer diversity. A relative `--dump-frame` path lands in
   `local/`.
2. **Classify the stall.**
   - *An instruction access fault at a `0x4000_xxxx` address* is a
     mask-ROM function with no stub. Find its name in ESP-IDF v5.5.3's
     `esp32c3.rom*.ld` linker scripts.
   - *A hot loop polling one peripheral address* is a register whose
     model is missing or wrong. Find the register in ESP-IDF v5.5.3's
     `soc/esp32c3/include/soc/*_reg.h` and read what the driver waits
     for.
   - *Unmapped data accesses* are logged, not fatal; a new one near the
     stall is the likely culprit.
   - *A panic in the console* (`Guru Meditation Error`) gives `MEPC` and
     `RA`; disassemble around them.
3. **Disassemble.** `firmware.bin` has no ELF or symbols: split out its
   code segments (IROM at `0x4200_0020`, IRAM at `0x4038_0000`), wrap
   each in an ELF at its load address with `llvm-objcopy -I binary`, and
   run `llvm-objdump -d --triple=riscv32 --mattr=+m,+c` (the local
   helpers in `local/m10-tools/` do this and find strings and xrefs).
   For what a ROM function really does, read the chip's ROM ELF
   (`local/rom-elfs/esp32c3_rev3_rom.elf`, from Espressif's
   `esp-rom-elfs` release).
4. **Fix with a cited model**, never a guess: a ROM stub's effect from
   the ROM ELF or newlib/libgcc source; a register's layout from the
   ESP-IDF header, cited in the module doc.
5. **Add a rung** that the fix makes pass and that failed before.

## Common changes

**A ROM stub.** Add an entry to `rom.rs`'s table at the address from
the linker script, with the right `RomStubEffect`: `Return` only if the
caller truly ignores the result, a real computation when the caller uses
it (a fabricated value silently corrupts the caller). For a ROM routine
that calls firmware code back, write it as guest RV32 code with
`cpu/encode.rs`'s const encoders. The nineteen libgcc helpers still
missing are listed in `milestone-14-decisions.md`, "Self-test
accelerometer reboot", addendum; `soft_float_ops_match_libgcc_soft_fp`
is the test to extend.

**A peripheral or register.** Add it to `peripherals/` and as a named
field of `FirmwareBus`, in address order, with its range in `mem/soc.rs`.
Model only what the firmware reads or waits for. Decide what a CPU reset
keeps (`milestone-11-decisions.md`, "What a CPU reset keeps") and what
the Worker or WASM shim must expose (passthroughs only in
`emulator-wasm`).

**An app-image hook.** Only for what cannot be a peripheral, like the
radio's VHCI seam. Put the address in `radio/v392.rs` (or a sibling for
another image) with its disassembly evidence; the hook table in
`cpu/app_hooks.rs` learns its ranges from it.

**A page feature.** The Worker owns the runtime; add a message to
`firmware-worker-protocol.ts`, handle it between slices in
`firmware-worker.ts`, mirror it in `firmware-worker-client.ts`, and
surface it through `badge-controller.ts`. Anything that types into the
console uses `console-io.ts`'s helpers and absolute byte offsets.

**A dependency.** Allowed where a library fits better than hand-written
code. Pin the exact version, record it (package, version, why, size) in
the milestone's "Dependencies added", and tell the human partner.
`emulator-core` never takes wasm-bindgen.

## Benchmarks

Never part of a test run. Results go in the current milestone's
decisions doc. A factor of 1.0x is the badge's speed; compare WASM rows
only with WASM rows.

```sh
cargo test -p emulator-core --release --test boot_progress bench_redraws -- --ignored --nocapture
bun run bench:wasm
bun run bench:safari   # needs bun run dev and Safari's remote automation
# or open /bench.html in any browser under bun run dev
```

## Deploying

The site is a Vercel project rooted at `frontend/`
(`frontend/vercel.json`). Its build script,
`frontend/scripts/vercel-build.sh`, installs Rust, wasm-pack 0.15.0 and
Bun when the image lacks them, then builds the WASM and the site. The
signaling function `frontend/api/signal.ts` needs `KV_REST_API_URL` and
`KV_REST_API_TOKEN` (an Upstash Redis database); without them
`/api/signal` returns 500 and only links are lost. Rate limits are sized
for a toy project with few concurrent users. `bun run preview` serves a
local production build on port 4173.

## How the project was run

Each milestone followed the same loop, with a human partner deciding
scope and checking results and an AI agent doing the work:

1. **Kickoff.** The human partner names the gap they feel using the
   emulator next to the badge. The previous decisions doc's backlog is
   the menu.
2. **Spec** (`docs/superpowers/specs/`): goals, non-goals, measured
   facts, the human partner's decisions, success criteria.
3. **Plan** (`docs/superpowers/plans/`): numbered tasks, each with its
   tests first.
4. **Subagent-driven execution.** One implementer per task, each task
   reviewed for spec compliance and code quality, with fix rounds; a
   ledger in `local/m*-sdd-ledger.md` tracked state across sessions.
   Milestone 14 used a worktree per task, merged one at a time.
5. **Gates.** At set points the human partner checked the result in a
   browser or against the physical badge. Their answers are quoted in
   the decisions doc.
6. **Final review** of the whole branch, a fix wave, the doc sweep
   (`CLAUDE.md`, the notes' "Current state", the decisions doc and the
   next backlog), final suites, and a merge to `main`.

Rulings made along the way are numbered within each milestone (R1,
R-T6-2, R-M5-2 …) and keep their reasons, so later milestones can tell a
deliberate choice from an accident.

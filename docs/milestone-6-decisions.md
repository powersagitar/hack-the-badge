# Milestone 6: design decisions, measurements, and the Milestone 7 backlog

Milestone 6 gives the real-firmware emulator a calibrated clock (emulated
time follows the firmware's own 40/80 MHz CPU frequency switches) and
makes Snake playable. The design spec is
`docs/superpowers/specs/2026-10-09-milestone6-playable-snake-design.md`
and the plan is
`docs/superpowers/plans/2026-10-09-milestone6-playable-snake.md`;
`docs/firmware-emulator-notes.md` holds the forensic findings and the
stall-by-stall history. This file records the rulings, the measurements
and what is deferred. Where the spec or plan disagree with this file or
the code, this file and the code win.

## Rulings

(None yet.)

## Measurements

### Task 1 baseline (branch base `ed71dd8`, before any Milestone 6 code)

All green. Wall times on the development machine (18 cores):

| Check | Result | Time |
| --- | --- | --- |
| `cargo test --workspace` (debug) | 505 lib + 86 integration tests pass (55 boot rungs) | about 39 s wall including the build; `boot_progress` 27.3 s |
| `cargo test -p emulator-core --release --test boot_progress` | 55 pass | 41.0 s |
| `cargo clippy --workspace --all-targets` | 3 warnings (2 doc-list indentation in `boot.rs`, 1 no-effect operation in a `console.rs` test) | |
| `bun run build:wasm` | ok | |
| `bun test` | 136 pass, 0 fail | 38.4 s |
| `bun run typecheck` | ok | |

## Milestone 7 backlog

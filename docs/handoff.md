# Handoff: the project at its end

Written 2026-11-15, when the project wrapped up after Milestone 14.
`main` is at `9baa6c2` and matches `origin/main`. No branch or worktree
is open.

## What it is

A browser emulator for the Hack the North 2026 hacker badge (ESP32-C3,
ST7789 screen, six WS2812 LEDs, D-pad and buttons through a 74HC165,
SC7A20H accelerometer, BLE). It runs the official firmware
`v0.1.2-392-gd3089c4` unmodified on an ESP32-C3 emulator written from
scratch in Rust and compiled to WebAssembly. The page is a photo of the
board on a lanyard: press its buttons, swing it, flip its power switch.

Size at the end: about 34,000 lines of Rust in the crates' `src/`, 6,900
in their integration tests, and 8,800 of TypeScript and CSS in the
frontend; 287 commits.

## What works

- **Boot** to the splash and My Badge's first-run screen, from blank
  synthetic flash.
- **Provisioning** a fake identity through the firmware's own USB
  console (eleven roles), then the 11-page onboarding walk, "Shake it!"
  included, to the launcher. Or **Skip setup**, which lands on the
  registered screen.
- **All 16 launcher apps open**, each pinned by a rung:

  | App | State |
  | --- | ----- |
  | My Badge | Opens; first-run and registered screens compared with hardware (v335) |
  | Snake | Playable at about 1.0x; speed and frames compared with hardware (v335) |
  | Dice | Rolls (fixed-seed RNG, so not the badge's rolls); compared with hardware (v335) |
  | Connect | Two emulated badges pair ("SYNCED"), in a rung and across two browsers |
  | Sync | Ends on "No sync station in range" (no station exists) |
  | Share | Opens after a reboot; its transfer works only in a two-badge probe |
  | Scanner, Sponsors, Workshop Quest | Open after a reboot; NFC is not modeled, so they log an RC522 error and still draw |
  | Mood LEDs | Opens; the LED colours are decoded and shown |
  | Solana Scavenger Hunt, Socials, Schedule, Info & Rules, Museum, Progress | Open and draw their first frame |

  Only My Badge, Snake and Dice (and boot and onboarding) were compared
  with a physical badge, on v335. v392's other frames were checked by the
  human partner's eye.
- **Reboots** take the badge's 466 ms and keep what a CPU reset keeps.
- **LEDs**: the six colours come from a WS2812 decoder on RMT.
- **Motion**: the lanyard's physics, or a phone's DeviceMotion, feed the
  accelerometer; shakes and bumps come from motion.
- **The power switch**: a cold boot that keeps the flash.
- **Two-browser links** over WebRTC (same network), with signaling on
  Vercel. Verified on real devices (2026-11-03, again 2026-11-11 on a
  Vercel preview, and 2026-11-14 with the redesigned page).
- **CONSOLE.LOG**: the USB serial output, byte-exact, with `[page]` lines.

## Verified and unverified

Verified on real hardware or real devices:

- Boot, onboarding, the registered screen, the launcher, Dice and Snake's
  frames and Snake's 170 ms move period, all against a v335 badge.
- The 466 ms reboot (measured on a v335 badge, scaled to v392).
- iOS's inverted DeviceMotion axes (R21), through the self-test's signed
  axes on an iPhone.
- Pairing across two browsers, three times.

Not verified, and worth checking first if a physical badge is at hand:

- `LED_CHAIN_TO_SPOT` (`[0, 1, 3, 5, 4, 2]`), which chain index sits
  where on the board. Check it against Setup's LED chase.
- `BOARD_AXES` (right +x, up +y, out +z). Check with Level or Accel.
- Android's DeviceMotion signs (`platformSignFix` is 1 there, untested).
- Sync's 20 s timeout against a stopwatch on a v392 badge (the source
  suggests about 15 s).
- Anything v392 draws differently from v335, beyond the eye check.

## Known limitations

The full list, item by item, is `firmware-emulator-notes.md`, "Known
limitations", "Current state (end of Milestone 14)". The ones a visitor
can notice:

1. **No real Bluetooth.** A browser cannot advertise BLE, so an emulated
   badge hears only other emulated badges, Sync never finds a station,
   and two badges with the same identity never pair (Connect drops its
   own badge ID).
2. **Speed.** The WASM core does about 41M steps/s against the badge's
   80M cycles/s. Most apps play at full speed; in Bun's bench a launcher
   move plays at about 0.85x and the HOME redraw about 0.63x. The human
   partner found the prod build's speed normal.
3. **Share** is not reachable in the browser (no way to install a Lua
   app to share).
4. **No NFC**, one I2C device, and RMT modeled only as far as the LEDs.
5. **A cold power-on takes no time** (the badge takes about 466 ms);
   adding it would move every pin for no visible gain.
6. **Determinism**: the RNG has a fixed seed, so Dice and Snake's food
   are the same every run.
7. **Page physics**: a jolt along gravity alone cannot fire a phone
   bump; on narrow stages a moderate shake can hit the walls and send a
   bump.

## Open work, ranked

The complete backlog, every item with its source, is
`milestone-14-decisions.md`, "Milestone 15 backlog". It was never
started. A reasonable order:

1. **Check the unverified board facts** above on a physical badge:
   `LED_CHAIN_TO_SPOT`, `BOARD_AXES`, Android's signs. Each is a
   one-constant fix once known.
2. **Stub the remaining ROM libgcc helpers.** Nineteen `firmware.bin`
   calls go to ROM functions `rom.rs` lacks, `__divsf3` (93 call sites)
   first. The self-test's accelerometer page rebooted on a missing one
   (`__fixsfsi`, now stubbed) in Milestone 14. Any app path that reaches one faults today. The list is
   in `milestone-14-decisions.md`, "Self-test accelerometer reboot",
   addendum.
3. **Share between two browsers**: a two-badge Share rung, a control
   that installs a Lua app over the console, and a two-browser transfer
   check. The core and controller model need no change.
4. **A fake sync station**, so Sync can complete. Possible on today's
   advertisement-only model; the human partner chose not to build it in
   Milestone 12.
5. **Phone-motion fixes**: detect jolts on unclamped readings, so a jolt
   along gravity fires a bump.
6. **Small page faults** from the Milestone 14 reviews: CONSOLE.LOG's
   invisible held prompt and its screen-reader live region, the drag
   ending on a second finger's pointerup, `select.tsx`'s dead animation
   classes, the `typescript` and `vite` carets in `frontend/package.json`.
7. **Speed**, only if it becomes a complaint again: the block cache was
   shelved in Milestone 8 (R-M8-1), and Milestone 7's profiles show where
   time goes.

## Rules that must survive

These are in `CLAUDE.md`, with reasons in the decisions docs. The ones
that are easiest to break by accident:

- **Never commit** anything from `local/`: the full flash dump, the real
  boot log, badge recordings. Never quote the boot log's identity lines.
  A full dump holds personal data and other attendees' contacts (notes,
  "Data-handling note").
- The only committed identity data is the eleven fake fixtures and
  `FAKE_UPLOAD_TOKEN`, committed with the organizers' permission. Never
  a real attendee's identity or credential.
- `firmware.bin` is published with the organizers' approval. The badge
  photo is cut from their public scan.
- **Performance work must not change emulated behaviour.** A pin that
  moves under a speed change is a bug in the change.
  `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged`
  guard it.
- Unmapped data access never panics; an unmapped instruction fetch
  always traps. Peer and network input never panics the emulator.
- The core stays chip-agnostic (`cpu/`, `clock.rs`); app-image addresses
  live only in `radio/v392.rs`, each cited.
- Recorded decisions stand unless the human partner agrees to change
  one.

## Picking it up again

1. Read this file, then `CLAUDE.md`, then the notes' "Timing model" and
   "Known limitations".
2. Set up and run the suites (`development-guide.md`, "Setup"). All of
   them passed at the end of Milestone 14: `bun test` 448 tests in 27
   files, the release boot ratchet 97 rungs plus 1 ignored benchmark,
   `cargo test --workspace` 782 tests plus 1 ignored, and clippy clean.
3. Pick from "Open work" above or the Milestone 15 backlog. Start a
   `docs/milestone-15-decisions.md` and keep the convention: rulings with
   reasons, measurements, re-pins, the human partner's answers, the
   dependencies added, and the next backlog.

### Moving to a newer firmware

Milestone 9 is the precedent (v335 → v392): try the new image in a
probe, open every launcher app, re-pin the moved frames and steps with a
reason each, and get the human partner's eye on the changed frames. The
radio's hook addresses in `radio/v392.rs` are specific to v392's image
and must be found again in the new one's disassembly (Milestone 10's
decisions show how each was found), as must the libgcc call list.

## Local-only material

`local/` (gitignored, never committed) holds what the committed docs
could not: the full flash dump and the real serial boot log (personal
data), the esptool venv (`local/.venv/bin/python`), the ROM ELFs
(`local/rom-elfs/`), badge photos and recordings, frame dumps, the design
files (`local/m14-design/`) and the badge photo's source scan
(`local/m14-source/`), and each milestone's kickoff prompts and
subagent-driven-development ledgers (`m*-sdd-ledger.md`). It exists only
on the human partner's machine; a fresh clone works without it, since
every committed test depends only on `firmware.bin` and committed data.

# Milestone 9 — Upgrade to the official v392 firmware

Status: scope agreed in conversation with the human partner (2026-10-22);
spec approved by the human partner (2026-10-22). Implementation plan:
`docs/superpowers/plans/2026-10-22-milestone9-firmware-upgrade.md`.
The Bluetooth work that started this
conversation is Milestone 10:
`docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`.

## Why

The human partner's gap, from using the emulator next to the badge: "not
all built in apps open … it just freezes the whole emulator." Not speed:
there is no open speed complaint (Milestone 8 decisions), and the block
cache stays shelved (R-M8-1). While scoping the fix, the human partner
supplied the official flashes and decided to **upgrade the emulator to
the official v392 and re-pin the tests** first, then do Bluetooth as its
own milestone (decided: "split into 2 milestones. first one for upgrade
the other for bluetooth").

## Measured (2026-10-22, `fdbd50b`, native release)

### Today's firmware (v335): 14 of 16 apps open

A throwaway probe (uncommitted; `m9_probe_all_apps` and
`m9_unpinned_walk` appended to the worktree's
`emulator-core/tests/boot_progress.rs`; logs `local/m9-probe.txt`,
`local/m9-v392-probe.txt`, `local/m9-v392-suite.txt`; frames
`local/m9-apps/`) opened each of the 16 launcher cells from role
hacker's launcher (DOWN x row, RIGHT x column, each followed by 300 ms;
A held 100 ms) and ran 5 s of emulated time.

- 14 open and draw a plausible first screen. Scanner logs
  `hal_nfc: rc522_start: ESP_ERR_INVALID_STATE` and still draws (NFC is
  not modeled).
- Connect and Sync freeze: after `hal_radio: init_once` they fetch
  mask-ROM `0x4000_08ec`, `btdm_controller_rom_data_init`
  (`esp32c3.rom.bt_funcs.ld`, ESP-IDF v5.5.3), which has no stub; a Guru
  Meditation follows, then "Panic handler entered multiple times"
  forever. (Milestone 10.)

### The official flashes

The human partner added `local/flashes/` (`bootloader.bin` at `0x0`,
`partition-table.bin` at `0x8000`, `firmware.bin` at `0x10000`). The
official firmware is public (the human partner, 2026-10-22).

- The bootloader and partition table are byte-identical to the badge's
  dump. The app is newer: `v0.1.2-392-gd3089c4` (compiled Sep 17 2026)
  against today's `factory.bin`, `v0.1.2-335-gbf0bee6` (Sep 9), which is
  the badge dump's `factory` partition (all 672 4-KiB pages equal at
  `0x10000`; the whole `0x2a0000` partition, padded with `0xFF`). Same
  ESP-IDF, v5.5.3. No ELF or symbols.
- The human partner's badge stays on v335.

### v392 in the emulator (trial; the swap was reverted)

- **It boots and reaches the launcher with no fault.** An unpinned walk
  (the rungs' inputs: boot, provision hacker, onboarding with the shake,
  START, HOME; fixed 24M-step settles) reaches the launcher at
  716,479,117 steps. The bare `firmware.bin` and a copy padded to
  `0x2a0000` give identical results (same steps, launcher hash
  `0x94d7_a7a8_5fa9_3ea3`): the emulated flash starts erased, so padding
  adds nothing.
- **Unchanged frames** (same hash as the v335 pins): the first-run
  screen, onboarding page 2, pages 4 to 11 and the shaken page, and
  hacker's registered screen. **Changed:** the splash, onboarding page 1
  and the button-test page 3 ("Press all 8 buttons", was 9), the
  launcher (same grid and order, a subtle pixel change).
- Release `boot_progress` with v392: 35 of 64 fail, every one at a pin
  (splash hash, v335 code-address rungs, the digest) or behind the
  first-run checkpoint, whose splash skip list holds the old splash.
- **Six apps freeze on v392** (`local/m9-v392-probe.txt`):
  - Connect: the same Bluetooth ROM fault (Milestone 10).
  - Scanner, Sync, Share, Sponsors, Workshop Quest: v392 enters these
    by rebooting (`app_reg: reboot-enter <app>`, `app_reboot: reboot into
    <app>`, a persisted `/littlefs/config/reboot.cfg`), and the reboot
    calls mask-ROM `software_reset_cpu` (`0x4000_0094`,
    `esp32c3.rom.ld`), which has no stub.
  - The other ten open.
- Other v392 changes seen in strings: Share sends apps over the radio;
  Sync adds "sync via hacker dashboard"; Connect drops "Nobody bumped
  back" and "Confirm the bump"; reset-reason logging; the LCD repaints in
  30-row stripes.

## Goals

1. **The emulator runs the official v392**, committed unpadded as
   `frontend/public/firmware/firmware.bin`, with "factory" renamed to
   "firmware" (Part 1).
2. **Every rung is re-pinned or re-derived, each with its reason**, and
   the suites pass (Part 1).
3. **The five apps that reboot into themselves open:** Scanner, Sync,
   Share, Sponsors, Workshop Quest (Part 2). Sync's radio stays for
   Milestone 10: its reboot must work, and it may then freeze in the
   Bluetooth ROM call like Connect, which is expected here.
4. The other ten apps still open on v392.

## Non-goals

- Bluetooth: Connect, and whatever Sync does after its reboot (Milestone
  10).
- NFC, LEDs, apps' deeper screens, speed (benchmarks are re-measured,
  not targeted).
- Lua sandbox mode: unchanged.
- Re-checking v392 side by side with a v392 badge: the badge stays on
  v335.

## Anti-goals (a failure even if it works)

- A re-pin without its reason, or a v335 hardware comparison presented as
  a v392 one.
- After Part 1's re-pin, any pin moving in Part 2: the reset happens only
  when an app reboots, so every boot, onboarding, launcher, Dice and
  Snake rung, `execution_digest_is_unchanged` and
  `hacker_walk_totals_are_unchanged` stay byte-identical. A move is a bug.
- A reset that loses the flash (the identity, `reboot.cfg`), or a reboot
  loop.
- Guessed register layouts or ROM addresses (CLAUDE.md Rules).

## Design

### Part 1 — Upgrade to v392, rename, re-pin

- **Commit the bare official image as
  `frontend/public/firmware/firmware.bin` and remove `factory.bin`**
  (decided).
- **Rename "factory" to "firmware"** (decided) wherever it names the file
  or the image loaded from it: the path in the Rust tests, examples, the
  WASM shim and the frontend (loader, Worker, bench, tests), helpers such
  as the tests' `factory()`, and current docs (CLAUDE.md, the notes'
  current-state sections). The flash *partition* keeps its
  firmware-defined name `factory` (the partition table says so); code and
  docs about the partition are not renamed. Historical docs (milestone
  decisions, the notes' history) keep their wording.
- The full dump stays local and stays v335; code that reads
  `BADGE_FULL_DUMP` must not assume the two apps match.
- **Every re-pin carries its reason** (the upgrade), with old and new
  values, in the rung doc or the decisions doc.
  - Rungs that pin a v335 code address (the "no longer faults at the
    pre-task call site" rungs, the hot-PC escapes) are re-derived for
    v392 from the same evidence, or retired with a recorded reason if the
    code they guarded no longer exists.
  - Step budgets are margins, not pins: they move with measured values.
  - Onboarding's input table changes if page 3 needs eight buttons, not
    nine. The trial walk suggests AUX1 left: its press was the only one
    that left page 3's frame unchanged. The builder confirms it.
  - The equivalence rungs are re-pinned once, at the end of Part 1, and
    guard Part 2.
- **Hardware comparisons go stale.** Every frame compared with the badge
  was v335. Rung docs say which frames were compared on v335 and that
  v392 frames are checked by the human partner's eye for plausibility
  (decided: "I'll eyeball and see if things make sense").
- The WASM twin (`frontend/test/cpu-wasm.test.ts`) and the browser
  provisioner follow the same walk on v392.
- The redraw benchmarks are re-measured on v392; numbers go in the
  decisions doc.

### Part 2 — Software reset (five apps)

`software_reset_cpu` (and `software_reset`, its neighbour in
`esp32c3.rom.ld`) get a model. The runtime re-runs the shortcut boot from
the image while **keeping the emulated flash** (littlefs holds
`reboot.cfg` and the identity), and keeps whatever the firmware reads to
learn why it reset: the reset reason and any hint it stores in RTC
registers that survive a reset (`esp_reset_reason_set_hint` is in the
v392 strings). Register layouts come from the ESP-IDF v5.5.3 headers.
Emulated time, steps and host inputs (held buttons, acceleration)
continue across the reset.

Success: each of the five apps opens after its reboot with no panic
text, and leaving it returns to the launcher the way the firmware does it
(another reboot, if so). Their first frames are pinned and join the
human partner's eyeball check. The browser shows the reboot like any
other screen change, with no special handling unless the human partner
asks.

### Part 3 — Docs

Notes (current state, "Known limitations", a Milestone 9 section),
CLAUDE.md (the firmware file, its version, the architecture map),
`docs/milestone-9-decisions.md` (rulings, measurements, re-pins, the
Milestone 10 backlog led by the Bluetooth spec).

## Success criteria

1. Release `boot_progress`, `cargo test --workspace`, `bun test`,
   `bun run typecheck` and `bun run build` pass on v392; clippy has no
   new warnings; release `boot_progress` stays under 90 s.
2. Every changed pin is listed with its reason; every v335 code-address
   rung is re-derived or retired with a reason.
3. A probe over all 16 launcher apps: 15 open with no panic text (Connect
   still faults in the Bluetooth ROM call; Sync may too, after its reboot
   works). New rungs pin the five rebooting apps' first frames.
4. **Human gate:** the human partner eyeballs v392 in the browser
   (boot, onboarding, launcher, the apps that open) and finds it makes
   sense.

## Gates (stop and ask the human partner)

- After Part 1's re-pin, before Part 2: the list of changed pins and
  frames.
- Release `boot_progress` over 90 s; any override of a recorded decision.
- The browser eyeball check at the end.

## Constraints (carried; CLAUDE.md Rules)

- `cpu/` and `clock.rs` chip-agnostic; `FirmwareBus` ordered concrete
  fields, no `dyn`; the word fast path order unchanged.
- `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays
  one-line passthroughs.
- Unmapped data access never panics; an unmapped instruction fetch always
  traps. Browser input never panics the emulator.
- Register layouts from ESP-IDF v5.5.3 headers, ROM addresses from its
  linker scripts; never guessed.
- Milestones 3 to 8 decisions stand.
- Local data: never quote the full dump, the boot log or recordings.
  Tests depend on the committed `firmware.bin` and committed fake
  identities only. `local/flashes/` stays uncommitted.

## Decided by the human partner (2026-10-22)

- Upgrade the emulated firmware to the official v392 and re-pin the
  tests; commit it unpadded as `firmware.bin`; rename "factory" to
  "firmware". The official firmware is public.
- v392 frames are judged by the human partner's eye; the badge stays on
  v335.
- Two milestones: the upgrade first (this one), Bluetooth second.

## My calls (the human partner can override)

- The five rebooting apps (software reset) belong to this milestone: the
  upgrade breaks them, so the upgrade fixes them.
- The partition keeps the name `factory`; only the file and image are
  renamed.
- After a reset, emulated time and held inputs carry on.

## Left to the builder

- How the reset re-runs boot inside `FirmwareRuntime` (what is rebuilt,
  what is kept), within the constraints above.
- Which v335 code-address rungs are re-derived and which are retired.
- Test structure for the five apps.

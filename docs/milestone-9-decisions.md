# Milestone 9: decisions and the Milestone 10 backlog

Milestone 9 upgrades the emulated firmware from the badge's dumped v335
to the official v392, renames the committed image from `factory.bin` to
`firmware.bin`, re-pins the tests, and adds the software reset that five
v392 apps use to open. Spec:
`docs/superpowers/specs/2026-10-22-milestone9-firmware-upgrade-design.md`;
plan: `docs/superpowers/plans/2026-10-22-milestone9-firmware-upgrade.md`.

## Why

The human partner's gap, from using the emulator next to the badge: "not
all built in apps open … it just freezes the whole emulator." Not speed:
there is no open speed complaint (Milestone 8 decisions), and the block
cache stays shelved (R-M8-1). While scoping the fix, the human partner
supplied the official flashes and decided to upgrade the emulator to the
official v392 and re-pin the tests first, then do Bluetooth as its own
milestone, Milestone 10.

## Measurements

Copied from the spec's "Measured" section (2026-10-22, `fdbd50b`, native
release). The probe was throwaway and uncommitted; its logs and frames
live under `local/` (`local/m9-probe.txt`, `local/m9-v392-probe.txt`,
`local/m9-v392-suite.txt`, `local/m9-apps/`).

### v335 (the dumped firmware): 14 of 16 apps open

The probe opened each of the 16 launcher cells from role hacker's
launcher (DOWN x row, RIGHT x column, each followed by 300 ms; A held
100 ms) and ran 5 s of emulated time.

- 14 open and draw a plausible first screen. Scanner logs
  `hal_nfc: rc522_start: ESP_ERR_INVALID_STATE` and still draws (NFC is
  not modeled).
- Connect and Sync freeze: after `hal_radio: init_once` they fetch
  mask-ROM `0x4000_08ec`, `btdm_controller_rom_data_init`
  (`esp32c3.rom.bt_funcs.ld`, ESP-IDF v5.5.3), which has no stub; a Guru
  Meditation follows, then "Panic handler entered multiple times"
  forever.

### The official flashes

`local/flashes/` (never committed) holds `bootloader.bin` at `0x0`,
`partition-table.bin` at `0x8000` and `firmware.bin` at `0x10000`. The
official firmware is public (the human partner, 2026-10-22).

- The bootloader and partition table are byte-identical to the badge's
  dump. The app is newer: `v0.1.2-392-gd3089c4` (compiled Sep 17 2026,
  2,651,840 bytes) against the old `factory.bin`, `v0.1.2-335-gbf0bee6`
  (Sep 9), which is the badge dump's `factory` partition (all 672 4-KiB
  pages equal at `0x10000`; the whole `0x2a0000` partition, padded with
  `0xFF`). Same ESP-IDF, v5.5.3. No ELF or symbols.
- The human partner's badge stays on v335.

### v392 in the emulator (trial; the swap was reverted)

- It boots and reaches the launcher with no fault. An unpinned walk (the
  rungs' inputs: boot, provision hacker, onboarding with the shake,
  START, HOME; fixed 24M-step settles) reaches the launcher at
  716,479,117 steps. The bare `firmware.bin` and a copy padded to
  `0x2a0000` give identical results (same steps, launcher hash
  `0x94d7_a7a8_5fa9_3ea3`): the emulated flash starts erased, so padding
  adds nothing.
- Unchanged frames (same hash as the v335 pins): the first-run screen,
  onboarding page 2, pages 4 to 11 and the shaken page, and hacker's
  registered screen. Changed: the splash, onboarding page 1 and the
  button-test page 3 ("Press all 8 buttons", was 9), the launcher (same
  grid and order, a subtle pixel change).
- Release `boot_progress` with v392: 35 of 64 fail, every one at a pin
  (splash hash, v335 code-address rungs, the digest) or behind the
  first-run checkpoint, whose splash skip list holds the old splash.
- Six apps freeze on v392:
  - Connect: the same Bluetooth ROM fault (Milestone 10).
  - Scanner, Sync, Share, Sponsors, Workshop Quest: v392 enters these by
    rebooting (`app_reg: reboot-enter <app>`, `app_reboot: reboot into
    <app>`, a persisted `/littlefs/config/reboot.cfg`), and the reboot
    calls mask-ROM `software_reset_cpu` (`0x4000_0094`,
    `esp32c3.rom.ld`), which has no stub.
  - The other ten open.
- Other v392 changes seen in strings: Share sends apps over the radio;
  Sync adds "sync via hacker dashboard"; Connect drops "Nobody bumped
  back" and "Confirm the bump"; reset-reason logging; the LCD repaints in
  30-row stripes.

## Re-pins (v335 → v392)

Every pin that Task 2 changes gets a row. Frames are described as
compared with the physical badge only for v335; v392 frames are checked
by the human partner's eye.

| rung | old | new | reason |
|---|---|---|---|

## Rulings

- Ruling R-T1-1: Task 1 renames every "factory" that means the committed
  file or the image loaded from it, in every file, including source files
  the plan's File Structure did not list (`mem/{bus,image,mod,soc}.rs`,
  `peripherals/{flash,mmu}.rs`, `cpu/rom_stubs.rs`, `rom.rs`,
  `runtime.rs`). The flash partition's firmware-defined name `factory`
  (`flash.rs`'s label and `ESP_PARTITION_SUBTYPE_APP_FACTORY`, `rom.rs`'s
  "nvs, phy_init, factory, storage"), docs about the partition,
  "factory reset" (the badge's own reset), `frontend/src/badge/ui.ts`'s
  unrelated widget `factory` helper and historical docs keep their
  wording — cost if wrong: a stale doc comment or a renamed partition
  label; the label is a pinned partition-table byte (its MD5 entry is
  checked by a unit test), so renaming it would have moved emulated
  behaviour, while a missed comment costs only a later doc fix.

## Milestone 10 backlog

Led by the Bluetooth spec,
`docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`
(Connect and Sync: the Bluetooth controller ROM calls, then the radio
links). Filled in at the end of this milestone.

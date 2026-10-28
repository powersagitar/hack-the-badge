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

### Every launcher app on v392 (Task 4)

Measured natively in release from role hacker's launcher (DOWN × row,
RIGHT × column, each settled; A held 100 ms), then pinned in
`boot_progress.rs` (`LAUNCHER_SELECTED` and the `launcher_opens!`
rungs). Every soft-reset count agrees with the probe above: Scanner,
Sync, Share, Sponsors and Workshop Quest reboot once; the others open
directly. Steps are from A's release to the first stable frame.

| Cell | App | Soft resets | First stable | Frame |
|---|---|---|---|---|
| 0 | My Badge | 0 | 9.0M | hacker's registered screen (unchanged) |
| 1 | Connect | 0 | — | stops at `0x4000_08ec` (dump at 0.75M) |
| 2 | Scanner | 1 | 14.5M | `0x4958_3da5_06ff_ab49` (Task 3) |
| 3 | Sync | 1 | — | stops at `0x4000_08ec` (dump at 6.75M) |
| 4 | Share | 1 | 9.0M | `0xfc51_aedc_c0ab_617d` |
| 5 | Solana Scavenger Hunt | 0 | 5.25M | `0x7ad3_a516_f050_78cf` |
| 6 | Sponsors | 1 | 16.0M | `0xb082_0063_bef2_19eb` |
| 7 | Workshop Quest | 1 | 21.75M | `0x894d_5dae_e0d9_8e2a` |
| 8 | Mood LEDs | 0 | 8.0M | `0x0c6c_07b5_b963_4775` |
| 9 | Socials | 0 | 4.0M | `0xeedf_ef97_7fd2_ea52` |
| 10 | Schedule | 0 | 8.25M | `0x59e7_950e_07fb_f1e9` |
| 11 | Info & Rules | 0 | 8.0M | `0x3472_9924_2cdb_54aa` |
| 12 | Museum | 0 | 8.25M | `0xd7f7_b7e2_9354_c399` |
| 13 | Progress | 0 | 5.75M | `0x2651_4b50_3ed5_3c44` |
| 14 | Snake | 0 | — | existing rungs, unchanged |
| 15 | Dice | 0 | 2.75M | `DICE_OPEN_HASH`, unchanged |

Scanner, Sponsors and Workshop Quest log `hal_nfc: rc522_start:
ESP_ERR_INVALID_STATE` (NFC is not modeled) and still draw. No app
prints panic text, and none faults except Connect and Sync. The frames
are in `local/m9-apps/<cell>-<slug>.png` (not committed).

### Benchmarks on v392 (Task 4)

Re-measured with every Task 4 rung in place (`3a87c3b` + tests; no
change under `emulator-core/src` since Task 3). Bun is `bun run
bench:wasm`, the median of 3 runs, its `pin_factor` column; native is
one `bench_redraws` run. Milestone 8's v335 numbers are beside them for
reference; they measure different firmware, so the columns are not a
speed comparison. There is no speed target.

| Redraw | Bun (WASM), v392 | Bun (WASM), M8 v335 | Native, v392 |
|---|---|---|---|
| HOME (enter the launcher) | 0.623x | 0.614x | 0.947x |
| DOWN (move inside it) | 0.857x | 0.977x | 1.006x |
| Onboarding | 1.110x | 1.224x | 1.430x |

Walk throughput: 39.96M steps/s in Bun's WASM (Milestone 8: 41.2M),
62.99M natively. Task 2's single native run on this machine gave home
1.13, down 1.20, onboarding 1.70; native runs vary that much between
sessions.

## Re-pins (v335 → v392)

Every pin that Task 2 changes gets a row. Frames are described as
compared with the physical badge only for v335; v392 frames are checked
by the human partner's eye.

All values were measured natively in release mode, with v392 in place
and no change under `emulator-core/src` or `emulator-wasm/src`. Frames
dumped for the eye check are in `local/m9-frames/` (not committed). A
row's "both" means `boot_progress.rs` and its copy in
`frontend/src/bench/walk.ts` (or `redraw-bench.ts`).

| rung | old | new | reason |
|---|---|---|---|
| `boot_integration` `boots_real_…_reaches_step_budget`: image size | 2,752,512 | 2,651,840 | v392 is committed unpadded |
| `SPLASH_HASH` (both; `boots_to_first_real_frame`, the first-run skip list, the WASM twin) | `0x5599_c270_ab04_29fa` | `0xfb30_5408_a9e5_0e98` | v392's splash: "Hack the North × SOLANA", footer "FW v0.1.2-392-gd3089c4"; stable from step 4,250,000 (v335: 5,500,000 sample) |
| `boots_to_first_real_frame` `MAX` | 8,900,000 | 8,500,000 | formula: stable step (4,250,000) × 2 |
| `FIRST_RUN_MAX_STEPS` (both, and the WASM twin's literal) | 17,250,000 | 16,500,000 | formula: first sampled at 14,500,000 (v335: 15,250,000) + 2,000,000 |
| `SELF_TEST_MAX_STEPS` | 31,250,000 | 29,750,000 | formula: first sampled at 20,683,077 (hold 5,183,077) + 8M + 1M, rounded up to 250k |
| `boot_starts_the_console` budget | 21,000,000 | 19,000,000 | formula: first printed at 15,200,000 (v335: 16,050,000) + 25 %, rounded up to 1M |
| `CONSOLE_DEADLINE` | 23,000,000 | 26,000,000 | formula: `console_rejects_an_empty_identity` ends at 20.30M (its `prov apply` answers at 19.30M; v335: 18.20M) + 25 %, rounded up to 1M |
| `RING_TEST_DEADLINE` | 22,000,000 | 21,000,000 | formula: `put` finished at 16.43M (v335: 17.18M) + 25 %, rounded up to 1M |
| `PROVISION_DEADLINE` (both) | 36,000,000 | 36,000,000 (doc only) | slowest `PROV OK` 28.74M (v335: 28.39M); + 25 % still rounds to 36M |
| `ROLE_ROWS` page 1, all 11 roles (hacker also `OWN_ROLE_PAGE1_HASH`) | hacker `0xd364_7661_7b1a_89b5` (others in the v335 file) | hacker `0xbf9c_cb7e_80b4_4b1a`; organizer `0x3c31_f95a_a12e_406f`, sponsor `0xc5ba_681b_7473_21bc`, judge `0x192b_1f45_de51_5d49`, mentor `0x4929_00a1_98c0_3e4d`, volunteer `0xd370_a34f_7bc7_d738`, media `0xa0b0_7c3c_ff8f_289d`, staff `0xe912_c748_aab7_152c`, general `0x0c04_ea57_eeb8_f8ec`, workshop_lead `0x96a7_9955_fffc_187c`, visitor `0x2770_ab89_5a47_e576` | v392's page 1 ("Welcome to Hack the North", name, "This is your badge…", "Any button to start") differs from v335's; registered screens are unchanged for all 11 roles |
| `TO_SHAKE_PAGE` (both): page 3 and its button frames | 11 page-3 entries, incl. `(8, 0xc6fb_a44d_8132_2795)` Aux1; page 3 `0x9621_b8aa_8b04_0d38`, A `0xa283…`, B `0x8522…`, UP `0xb9e7…`, LEFT `0x362f…`, RIGHT `0x16eb…`, DOWN `0x8a7a…`, HOME `0x824a…`, START `0x3a72…` | 10 entries, Aux1 removed; page 3 `0x38a9_d040_707c_c4e8`, A `0x8a68_96e1_b5c9_5d6b`, B `0x8b0b_8358_e2b5_d6e8`, UP `0xf38f_281c_4b05_dabd`, LEFT `0xb185_0f5c_8931_2eb3`, RIGHT `0x5266_68ba_6df4_b25e`, DOWN `0x3d7b_e2d1_7265_7413`, HOME `0x1fd7_15c4_6f18_7c4c`, START `0xc2eb_c5fd_5353_2d8c` | v392's page 3 shows 8 tiles ("Press all 8 buttons once"); slot 8 changes nothing (ruling R-T2-1). Page 2, pages 4 to 6 unchanged |
| `LAUNCHER_HASH` (both) | `0xb694_a61f_ebfd_62ff` | `0x94d7_a7a8_5fa9_3ea3` | v392's launcher: same 4×4 grid and order, a few pixels differ; equal for all 11 roles |
| `LAUNCHER_RESPONSE_STEPS` (both) | 26,000,000 | 25,000,000 | formula: release to stable launcher 11.5M (v335: 12.25M) + 8M, + 25 %, rounded up to 1M |
| `first_trap_is_the_from_cpu_0_yield_interrupt_on_its_routed_line` | yield written step 528,312, taken 528,313 | 527,878 / 527,879 | v392's `sw` to `0x600c_0028` at `0x4038_9662`; `mcause` `0x8000_0004`. Spin address `0x4200_0cd2` unchanged (same bytes in v392) |
| `boot_no_longer_faults_at_the_pre_task_d10_…` | zero traps through 528,311 | 527,877 | the step before the v392 yield request |
| `PRE_TASK_9_SPI_UPDATE_POLL` (`boot_escapes_the_pre_task_9_spi_update_poll_…`, `rom_stub_boot` phase 3) | `0x420f_d6fc..=0x420f_d702`, first reached step 584,140 | `0x4210_010a..=0x4210_0110`, step 585,551 | v335's 52 bytes around the poll occur once in v392; each address hit once in 583,000..596,000 |
| `SPI_LL_SET_COMMAND_BSWAP_RA` (`…_d12_bswapsi2_call_site`, `rom_stub_boot` phases 4-5) | `0x4039_45fa`, entered step 594,724 | `0x4039_463a`, entered after step 594,523 | v335's 40 bytes from the return address occur once in v392; every boot `__bswapsi2` call returns there |
| `load_partitions_accepts_…`: `LOAD_PARTITIONS_EPILOGUE` | `0x420f_a878` | `0x420f_d282` | the same function shape (error paths 0x105/0x103 into `s4`, same cleanup `jal`, identical epilogue bytes); its first `MD5Init` returns to `0x420f_d0f4` |
| `load_partitions_accepts_…`: run before single-stepping | 4,000,000 | 1,500,000 | v392 calls `MD5Init` first on step 1,855,884 (v335: 4,450,328), so a 4M run would pass it |
| `rom_stub_boot` `boot_stubs_reach_load_partitions_md5init_after_drawing_the_splash` → `…_after_the_first_flush` | phases at 528,312 / 584,140 / 594,723-4 / wfi `0x4038_b8bc` 598,317-9, tick jump 145,940 / `MD5Init` 4,450,328, ≥ 16 tick periods, ≥ 2,000 colours, `a0` `0x3fcb_fd30`, RA `0x420f_a6ea` | 527,878 / 585,551 / 594,523-4 / wfi `0x4038_b8b8` 598,117-9, jump 145,893 / `MD5Init` 1,855,884, ≥ 13 periods (13.5), frame a uniform `0xf7be` fill, `a0` `0x3fcb_9080`, RA `0x420f_d0f4` | v392 runs `load_partitions()` before the splash is drawn (ruling R-T2-2); wfi found by v335's 16 bytes around it, unique in v392 |
| `rom_stub_boot`'s first-trap comment (`rom_stubbed_boot_gets_past_the_mask_rom_wall`) | 528,313 | 527,879 | doc only |
| `EXECUTION_DIGEST` | `0x30e1_c4af_04a0_f326` | `0x0ae0_2ec4_374c_d285` | v392 executes different code; re-pinned last, from the re-pinned suite |
| `HACKER_LAUNCHER_STEPS` | 372,597,011 | 342,600,776 | the re-pinned hacker walk (Aux1 press gone, faster responses) |
| `HACKER_LAUNCHER_US` | 98,461,748 | 94,521,615 | same |
| `HACKER_LAUNCHER_CONSOLE_HASH` | `0xd042_f54c_287b_c3ef` | `0xabcd_cc17_7a35_80fe` | v392's console text (new lines such as `boot: reset reason=1`) |
| `HOME_REDRAW` (both) | 254,000 µs, 19,106,041 steps | 244,000 µs, 18,292,336 steps | `bench_redraws` on v392; end hash is the new `LAUNCHER_HASH` |
| `DOWN_REDRAW` (both; `redraw-bench.test.ts` pin factor) | 115,000 µs, 6,858,235 steps | 109,000 µs, 7,942,619 steps | `bench_redraws` on v392 |
| `ONBOARDING_REDRAW` (both) | 67,000 µs, 3,018,254 steps | 52,000 µs, 2,547,134 steps | `bench_redraws` on v392 |

Unchanged on v392 (same hash or value as v335): `FIRST_RUN_HASH`,
`SELF_TEST_BUTTONS_HASH`, page 2, pages 4 to 11 and the shaken page,
every role's registered screen, `LAUNCHER_AFTER_NAV_HASH`, `TO_DICE`,
Dice's open and rolled frames, every Snake frame and move count, the
`qsort` return (step 408,988 to `0x4200_2a18`), and the console needles.
Approximate step numbers in rung docs ("~") were updated to v392's
measurements (for example `esp_littlefs` formatting ~4,508,000 →
~1,908,000; `launched My Badge` ~10.99M → ~10.54M).

Release `boot_progress` with v392: 64 passed, 1 ignored, 7.5 s wall
(`/usr/bin/time`), far under the 90 s limit. Native `bench_redraws` on
v392: home factor 1.13, down 1.20, onboarding 1.70 (one run; not
comparable with Milestone 7's v335 rows).

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
- Ruling R-T2-1: v392 dropped the Aux1 button (raw slot 8) from
  onboarding page 3, so the walk no longer presses it; `TO_SHAKE_PAGE`
  loses its Aux1 entry in `boot_progress.rs` and `walk.ts` alike, and its
  comment now reads "all eight". Evidence: page 3 shows eight tiles and
  "Press all 8 buttons once"; after DOWN the frame is `0x3d7b…7413`, and a
  100 ms press of slot 8 leaves that exact frame for the whole 24M-step
  settle; HOME and START then light their tiles, START's frame shows "A:
  next", and A reaches page 4 with v335's hash (frames
  `local/m9-frames/step2-aux1-kept-TO_SHAKE_PAGE-hacker-*.png`) — cost if
  wrong: if slot 8 still did something the walk skips one input the
  badge accepts; the rung would still pass, and only the Aux1 path would
  be untested.
- Ruling R-T2-2: `rom_stub_boot`'s
  `boot_stubs_reach_load_partitions_md5init_after_drawing_the_splash` is
  renamed `boot_stubs_reach_load_partitions_md5init_after_the_first_flush`
  and its phase 9 frame check changes from "at least 2,000 colours (the
  splash)" to "a uniform `0xf7be` fill". v392 calls `load_partitions()`'s
  `MD5Init` at step 1,855,884, 13.5 tick periods in, while the frame is
  LVGL's first flush; the splash is stable only from step 4,250,000. The
  tick-period floor drops from 16 to 13. Every other phase keeps its
  check, re-pinned. `docs/firmware-emulator-notes.md` still uses the old
  name and v335's phase steps (Task 6 updates the notes) — cost if
  wrong: if the rung should still prove the splash is drawn before
  `load_partitions()`, it no longer does; `boot_progress.rs`'s
  `boot_no_longer_faults_at_the_pre_task_d13_md5init_call_site` still
  checks the splash is on screen at step 5,560,000.
- Ruling R-T2-3: no code-address rung is retired. Each v335 address was
  found again in v392 by its bytes (the SPI poll, the `__bswapsi2` return
  address, the idle `wfi`, the scheduler-start `j .`, which did not move)
  or by its function shape (`load_partitions()`'s epilogue), and the rung
  docs cite that evidence. `first_trap_…` keeps v335's step 571,933 as
  its no-fault horizon, labelled as v335's `gpio_matrix_out` fault step —
  cost if wrong: a re-derived address in the wrong function would make a
  rung pass vacuously; each rung also asserts the hit counts and return
  values it did on v335, which a wrong address would fail.
- Ruling R-T2-4: Task 2's commit carries only the `Co-Authored-By`
  trailer. The session's latest attribution instruction lists that line
  alone, and Global Constraints also list a `Claude-Session` line — cost
  if wrong: one missing trailer line, which can be amended before the
  push.
- Ruling R-T3-1: the software reset carries the whole ST7789 model
  (`bus.spi.st7789`), not just its pixels. The panel is a separate chip
  that the MCU's reset does not touch, every command byte clears its
  transient parameter state, and the field is already public, so
  `spi.rs` needs no setter — cost if wrong: a stale CASET/RASET window
  could survive until the firmware's first window command; the new boot
  sends its own before drawing, and the Scanner frame is pinned.
- Ruling R-T3-2: `RtcCntl::handles` stays `STORE4` alone. `STORE0..7`
  are plain storage already; the reset reads and restores them through
  `store_regs`/`set_store_regs`. Listing them in `handles` would only
  drop their accesses from the unmapped-access log, which boot-probe and
  the rungs read — cost if wrong: STORE accesses keep appearing in the
  diagnostic log.
- Ruling R-T3-3: `STORE4` is carried over like the other seven, after
  the fresh boot seeded it, so the app's last value wins over the seed.
  On the badge the bootloader would rewrite it with the XTAL frequency
  (`0x0028_0028`, the value the shortcut boot seeds) — cost if wrong: a
  firmware that wrote something else to `STORE4` would keep that value
  across a reboot instead of the bootloader's.
- Ruling R-T3-4: Scanner's HOME is a direct return, measured: after HOME
  the console shows `heap after exit Scanner` and `launched Launcher`
  with no `reboot into`, and the soft-reset count stays 1. The rung keeps
  the plan's name, `scanner_home_reboots_to_the_launcher`, and asserts
  the direct return; its doc records the path — cost if wrong: a name
  that reads as a second reboot; the assertions say what happens.
- Ruling R-T4-1: Sponsors' pinned open frame is its first stable
  frame, the floor map before any booth is drawn ("--/64"). The booth
  icons then appear in 42 redraws, the last about 60.5M steps after A's
  release, with the counter at "0/64 0%"; they are not pinned. The brief
  pins each app's first stable frame, and the loaded map would add about
  45M steps to the rung — cost if wrong: a change that breaks only the
  booth drawing would not fail the rung.
- Ruling R-T4-2: Connect's and Sync's rungs stop at the register dump
  (`MEPC    : 0x400008ec`), as the brief asks. What follows was measured
  and is not pinned: the panic handler prints `Rebooting...`, the
  runtime soft-resets (Task 3), and the badge boots with `boot: reset
  reason=4` into My Badge, once, with no loop. On v335 without the
  software reset this was "Panic handler entered multiple times"
  forever — cost if wrong: a regression in the post-panic reboot would
  go unnoticed until Milestone 10 replaces the fault.
- Ruling R-T4-3: the launcher's 16 selection frames are one table,
  `LAUNCHER_SELECTED`, which reuses `LAUNCHER_HASH`,
  `LAUNCHER_AFTER_NAV_HASH`, `TO_SCANNER` and `TO_DICE` for the nine
  cells they already pin. Each rung walks from the hacker launcher
  checkpoint and asserts every selection on its way, rather than sharing
  per-row checkpoints; the release suite stays at about 8 s. My Badge's
  open frame is `ROLE_ROWS`' hacker registered screen, the same frame —
  cost if wrong: the walks repeat some navigation work.

## Milestone 10 backlog

Led by the Bluetooth spec,
`docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`
(Connect and Sync: the Bluetooth controller ROM calls, then the radio
links). Filled in at the end of this milestone.

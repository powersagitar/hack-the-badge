# Milestone 9: decisions and the Milestone 10 backlog

Milestone 9 upgrades the emulated firmware from the badge's dumped v335
to the official v392, renames the committed image from `factory.bin` to
`firmware.bin`, re-pins the tests, and adds the software reset that v392
uses to enter five apps (four of them now open; Sync then stops at the
Bluetooth ROM). Spec:
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
directly. Steps are from A's release to the first stable frame, except
Sponsors, whose pin is its loaded frame (the human partner's ruling at
the Task 5 gate, below R-T4-1).

| Cell | App | Soft resets | First stable | Frame |
|---|---|---|---|---|
| 0 | My Badge | 0 | 9.0M | hacker's registered screen (unchanged) |
| 1 | Connect | 0 | — | stops at `0x4000_08ec` (dump at 0.75M) |
| 2 | Scanner | 1 | 14.5M | `0x4958_3da5_06ff_ab49` (Task 3) |
| 3 | Sync | 1 | — | stops at `0x4000_08ec` (dump at 6.75M) |
| 4 | Share | 1 | 9.0M | `0xfc51_aedc_c0ab_617d` |
| 5 | Solana Scavenger Hunt | 0 | 5.25M | `0x7ad3_a516_f050_78cf` |
| 6 | Sponsors | 1 | 60.5M (loaded) | `0xcdc7_9972_60aa_f1c8` |
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

Release `boot_progress` with every Task 4 rung: 79 passed, 1 ignored,
11.7 s wall (`/usr/bin/time`, real 11.74 s; Task 3's 66 rungs took about
8 s), under the 90 s limit.

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

### The browser gate (Task 5, 2026-10-28)

**Smoke test** (Chrome, `bun run build:wasm && bun run dev`; screenshots
under `local/m9-browser/`, not committed): provisioning hacker, the
11-page onboarding app, the launcher, and Scanner opened and left again
through HOME all work. The speed readout holds at 1.0x, except about
0.8x for about 0.5 s while Scanner reboots; the reboot shows as an
ordinary screen change, with no stall. The console is clean apart from
a 404 for the favicon.

**The human partner's verdicts:**

1. The v392 frames are right: "side by side comes clean". This is the
   human's eye on v392 next to the badge; the rung docs still describe
   only v335 frames as compared with hardware.
2. "pin the loaded frame instead now": Sponsors' rung pins its loaded
   frame, superseding R-T4-1 (see that ruling).
3. The lower speed on v392 (for example WASM DOWN 0.977x → 0.857x, in
   the table above) is accepted: "speed is indeed slower and noticeable
   yet acceptable". There is still no speed target.

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
| `SPLASH_HASH` (both; `boots_to_first_real_frame`, the first-run skip list, the WASM twin) | `0x5599_c270_ab04_29fa` | `0xfb30_5408_a9e5_0e98` | v392's splash: "Hack the North × SOLANA", footer "FW v0.1.2-392-gd3089c4"; stable from step 4,250,000 (v335: from about 4.43M; its rung sampled it at 5,500,000) |
| `boots_to_first_real_frame` `MAX` | 8,900,000 | 8,500,000 | formula: stable step (4,250,000) × 2 |
| `FIRST_RUN_MAX_STEPS` (both, and the WASM twin's literal) | 17,250,000 | 16,500,000 | formula: first sampled at 14,500,000 (v335: 15,250,000) + 2,000,000 |
| `SELF_TEST_MAX_STEPS` | 31,250,000 | 29,750,000 | formula: first sampled at 20,683,077 (hold 5,183,077) + 8M + 1M, rounded up to 250k |
| `boot_starts_the_console` budget | 21,000,000 | 19,000,000 | formula: first printed at 15,200,000 (v335: 16,050,000) + 25 %, rounded up to 1M |
| `CONSOLE_DEADLINE` | 23,000,000 | 26,000,000 | formula: `console_rejects_an_empty_identity` ends at 20.30M (v335: 18.20M) + 25 %, rounded up to 1M. Measured in Task 6 (scratch probe, not committed): `PROV FAIL` prints at 16.30M (v335: 17.00M), but the next prompt follows at 19.30M, 3.0M busy steps (about 40 ms emulated) after the result line, against 0.2M (about 6 ms) on v335; what v392 does in that gap was not traced |
| `RING_TEST_DEADLINE` | 22,000,000 | 21,000,000 | formula: `put` finished at 16.43M (v335: 17.18M) + 25 %, rounded up to 1M |
| `PROVISION_DEADLINE` (both) | 36,000,000 | 36,000,000 (doc only) | slowest `PROV OK` 28.74M (v335: 28.39M); + 25 % still rounds to 36M |
| `ROLE_ROWS` page 1, all 11 roles (hacker also `OWN_ROLE_PAGE1_HASH`) | hacker `0xd364_7661_7b1a_89b5`, organizer `0x5dba_66e0_b019_1c24`, sponsor `0x997d_dc94_a332_cb23`, judge `0xa681_3acc_1906_5c12`, mentor `0x2619_ad73_bf0d_7e86`, volunteer `0x05f0_cede_9341_fc3f`, media `0x49ee_edbe_2a64_a1d6`, staff `0xc270_f7ca_eb37_28d3`, general `0xa6a3_0959_d158_ab93`, workshop_lead `0xf723_ec7a_d14b_c6e3`, visitor `0x6e32_6aa6_bcd6_c3f1` | hacker `0xbf9c_cb7e_80b4_4b1a`; organizer `0x3c31_f95a_a12e_406f`, sponsor `0xc5ba_681b_7473_21bc`, judge `0x192b_1f45_de51_5d49`, mentor `0x4929_00a1_98c0_3e4d`, volunteer `0xd370_a34f_7bc7_d738`, media `0xa0b0_7c3c_ff8f_289d`, staff `0xe912_c748_aab7_152c`, general `0x0c04_ea57_eeb8_f8ec`, workshop_lead `0x96a7_9955_fffc_187c`, visitor `0x2770_ab89_5a47_e576` | v392's page 1 ("Welcome to Hack the North", name, "This is your badge…", "Any button to start") differs from v335's; registered screens are unchanged for all 11 roles |
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
  - The human partner at the Task 2 gate (2026-10-22): "repins and
    frames fine exclude claude session". The re-pins and the changed
    frames are approved, every Milestone 9 commit carries only the
    `Co-Authored-By` line, and Task 2's commit, `b25a68c`, is the base
    the equivalence rule guards from.
- Ruling R-T2-5: the doc-only minors that the Task 1 and Task 2 reviews
  deferred are swept in Task 6, with the docs — cost if wrong: small doc
  fixes land later than they could.
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
- Ruling R-T3-5: the rung keeps the plan's name through Task 3, and Task
  6 may rename it to `scanner_home_returns_to_the_launcher` as part of
  its doc sweep — cost if wrong: a misleading test name until then.
  (Done in Task 6, R-T6-2.)
- Ruling R-T4-1 (superseded at the Task 5 gate, see below): Sponsors'
  pinned open frame is its first stable frame, the floor map before any
  booth is drawn ("--/64"). The booth icons then appear in 42 redraws,
  the last about 60.5M steps after A's release, with the counter at
  "0/64 0%"; they are not pinned. The brief pins each app's first stable
  frame, and the loaded map would add about 45M steps to the rung — cost
  if wrong: a change that breaks only the booth drawing would not fail
  the rung.
  - Superseded by the human partner at the Task 5 browser gate
    (2026-10-28): "pin the loaded frame instead now". The rung now pins
    `SPONSORS_LOADED_HASH` = `0xcdc7_9972_60aa_f1c8` ("0/64 0%", the map
    covered in "?" booth icons, the first one outlined) in place of
    `0xb082_0063_bef2_19eb`. Measured: the map is stable 16.0M steps
    after A's release; the icons come in four batches after pauses of
    8.25M, 8.5M, 8.75M and 9.5M steps (42 redraws); the last redraw is
    at 60.5M and the frame is unchanged through 150M. The rung waits for
    a frame that holds `SPONSORS_LOADED_STABLE_FOR` = 20M steps (over
    twice the longest pause; the other rungs' 8M would stop on the bare
    map) within 101M (60.5M + 20M = 80.5M, + 25 %, rounded up). It is a
    test pin chosen by the human, not a re-pin: no emulated behaviour
    changed and no other pin moved. The rung takes about 7 s alone; the
    release suite stays at 11.7 s, since the rungs run in parallel.
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
  per-row checkpoints; the release suite goes from about 8 s to 11.7 s,
  under the 90 s limit. My Badge's
  open frame is `ROLE_ROWS`' hacker registered screen, the same frame —
  cost if wrong: the walks repeat some navigation work.

- Ruling R-T6-1: Task 6 sweeps every doc-only minor that the task
  reviews deferred: the notes' current sections that still stated v335
  facts as current (Task 1), the Task 2 re-pin rows and rung comments
  (the `MD5Init` step, the `ROLE_ROWS` old hashes, the splash and
  `CONSOLE_DEADLINE` comparisons, the unlabelled colour count, the notes'
  old `rom_stub_boot` rung name, a long line in `redraw-bench.ts`), the
  Task 3 doc comments (`soft_reset` drops the unmapped-access log and the
  ROM-stub index drops; reset requests are collected only by the run
  loop; `rom.rs` entry 26 and its stub count) and the Task 4 rung docs
  (`LAUNCHER_SELECTED`'s reuse of `LAUNCHER_AFTER_NAV_HASH`, who made the
  binding eye check, the Sponsors rung outside `launcher_opens!`). The
  behavioural minors (a released-button test, the `app_reboot` line
  assertion, `run_to_the_bluetooth_fault`'s per-chunk fault check) go to
  the Milestone 10 backlog, not this task — cost if wrong: two small test
  gaps carried a milestone longer.
- Ruling R-T6-2: Task 6 renames `scanner_home_reboots_to_the_launcher`
  to `scanner_home_returns_to_the_launcher` (R-T3-5's option; its doc and
  R-T3-4 record a direct return). Current docs use the new name; the plan
  and the rulings above keep the old one where they record what happened
  — cost if wrong: a rung name differs from the plan's text.

## Task 6: the doc sweep, measured

Two of R-T6-1's minors needed a measurement; both are comments only.

- `MD5Init`'s first call is step 1,855,884 (1-based, as v335's
  4,450,328 was): `rom_stub_boot`'s phase 10 runs 1,855,883 steps and
  then asserts `pc` is `MD5Init`. `boot_progress.rs`'s comment said
  1,855,883 and now says 1,855,884.
- v392's splash also has 2,340 distinct RGB565 values at step 5,550,000
  (`boot-probe --steps 5550000`), the same count as v335's, so
  `boot_draws_frames_through_gdma_before_the_md5init_stall`'s doc now
  says the count holds for both.
- `CONSOLE_DEADLINE`: see its re-pin row; the rung doc now gives both
  versions' steps.
- `rom.rs`'s stub count, from `esp32c3_rom_stubs().len()`: 125. Milestone
  6 Task 2 turned the two CPU-frequency stubs into guest code (123), and
  Task 3 added the two reset stubs.

## Final suite (Task 6)

Measured at the Task 6 commit, native on the development machine:

| Check | Result |
|---|---|
| `cargo test -p emulator-core --release --test boot_progress` | 79 passed, 1 ignored; real 11.66 s and 12.65 s in two warm runs (`/usr/bin/time -p`; 11.63 s and 11.77 s in the harness), under the 90 s limit |
| `cargo test --workspace` | 669 passed, 1 ignored (559 unit tests, the 79 rungs in debug and 31 in the other test binaries); real 17.0 s warm |
| `cargo clippy --workspace --all-targets` | the 3 known warnings (two in `boot.rs`, one in a `console.rs` test) |
| `bun run build:wasm` | builds |
| `bun test` | 182 passed, 0 failed, 15 files; 9.8 s to 10.1 s |
| `bun run typecheck` | clean |
| `bun run build` | builds |

`execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged`
pass, and no pin moved after `b25a68c` except Sponsors' (the human
partner's ruling at the Task 5 gate).

## Milestone 10 backlog

### Lead: the badge radio

`docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`:
Connect, and Sync after its reboot, stop at mask-ROM `0x4000_08ec`
(`btdm_controller_rom_data_init`, `esp32c3.rom.bt_funcs.ld`). The spec
replaces the closed controller with a fake BLE controller in
`emulator-core`, then pairs two emulated badges in one test, then the
browser's radio hub and Bump button and a WebRTC link. Its measurements
are re-taken on v392 at kickoff, and the human partner approves it
before a plan.

### Found in Milestone 9

- Tests deferred by R-T6-1:
  - No test presses a button, releases it, then soft-resets: the
    "nothing replays" half of Review Focus 1
    (`held_button_survives_a_soft_reset` covers only the held half).
  - `assert_launcher_opens` does not assert the `app_reboot: reboot into
    <slug>` line its docs name for Share, Sponsors and Workshop Quest;
    only `soft_reset_count()` shows the reboot.
  - `run_to_the_bluetooth_fault` reads only each chunk's
    `last_instruction_fault`, so an earlier fault at another address in
    the same 250,000-step chunk would go unseen.
- After Connect's and Sync's Bluetooth fault, the panic handler's reboot
  into My Badge (`boot: reset reason=4`, once, no loop) is measured but
  not pinned (R-T4-2). Milestone 10 replaces the fault.
- `soft_reset_count()` is not exposed through `emulator-wasm`, so the
  browser cannot show or test it.
- The RTC timer restarts at 0 at a software reset; on the badge the RTC
  domain keeps counting. Nothing observed depends on it.
- A reboot takes no emulated time: the ROM and the 2nd-stage bootloader
  are not run (the shortcut boot), so rebooting apps open sooner than on
  the badge. The badge's reboot time is not measured.
- Both reset stubs are the same full re-boot through the shortcut boot;
  only the reported reason differs (`rom.rs` entry 26). The difference
  between a CPU-only and a digital-core reset is not modeled; nothing
  observed depends on it.
- `STORE4` keeps the app's value across a reset, where the bootloader
  would rewrite it with the XTAL frequency (R-T3-3).
- A soft reset starts the unmapped-access log and the ROM-stub index
  drop count afresh (documented in `runtime.rs`); carry them over if a
  diagnostic ever needs pre-reset entries.
- On v392, `prov apply`'s `PROV FAIL` is followed by 3.0M busy steps
  (about 40 ms emulated) before the next prompt, against 0.2M on v335;
  not traced.
- No v392 frame is compared with hardware: the badge runs v335. A
  comparison needs the badge reflashed to v392, which is the human
  partner's call. The full dump also stays v335.
- Chrome, Safari and the dev bench page were not re-benchmarked on v392
  (only Bun's WASM and native, in "Benchmarks on v392").
- v392's early log timestamps have not been compared with any badge's.

### Carried from Milestone 8's backlog (still open)

`docs/milestone-8-decisions.md`'s Milestone 9 backlog, word for word
under its headings, except where a Milestone 9 fact is added. Only one
item moved: every built-in app now has an open rung ("Apps and UI").

#### Shelved lead: a block cache (HOME is short of the badge)

Unchanged from Milestone 8's lead (read it in
`docs/milestone-7-decisions.md`, "Lead: a block cache"), but shelved by
R-M8-1: after the tap fix the human felt no sluggishness. Bring it back
only if the human finds entering the launcher (HOME, about 0.6x in the
browsers) or another screen slow. Measure the basic-block length
distribution on the hot path before designing one. (Milestone 9: on
v392 HOME is 0.623x and DOWN 0.857x in Bun's WASM; the human partner
called the drop noticeable and accepted it at the Task 5 gate.)

#### Found in Milestone 8

- One button fed by two sources (a key in `KEY_TO_BUTTON` and the pad, or
  two keys mapped to one button) shares one tap queue: holding the key
  and clicking the pad ignores the click's press, but its mouseup
  releases the button while the key is still down. Partly so before
  Milestone 8; a per-source hold count would fix it.
- A queued tap that has not been pressed yet (waiting out the gap) while
  the user still holds the key is dropped by `stop()`, and its later
  release is ignored. Not a regression; noted by the re-review.
- The tap queue adds a small lag to fast bursts (each tap takes at least
  140 ms of emulated time). The human called it tiny; `RELEASE_GAP_US`
  could shrink toward the 20 ms minimum if it ever matters.
- The release-gap probe is not a committed rung. A rung "two DOWNs with a
  `RELEASE_GAP_US` gap move twice" in `boot_progress` would pin the
  measurement against firmware or timing changes.

#### Found in Milestone 7

- The decode cache at `1 << 16` entries read about 1.5 % faster on HOME
  than `1 << 14`, measured with the `pc` tag, for 1 MiB per runtime and
  per clone (R-T7-4); not re-measured untagged.
- `WORKER_STEPS_PER_SLICE` (400,000) was never tuned in a browser
  (R-T8-2); a measured long-slice input lag would argue for 200,000.
- The frontend and Rust provisioning hosts reach the launcher 740,001
  steps apart (R-T2-4; measured on v335, not re-measured on v392); the
  cause was not traced.
- The status line can show a pre-reset state for up to one
  `STATUS_INTERVAL_MS` (250 ms) after a reset or provision (R-T8-9).

#### From the final review

- `stepFirmwareFrame` (firmware-runtime.ts:239, called from
  firmware-worker.ts) copies the 153,600-byte framebuffer out of WASM on
  every slice (line 249), even when a frame is already in flight; fetch
  it only when a frame will be posted.
- A Worker error after ready only logs to the console
  (firmware-worker-client.ts:157-159): the client keeps `running`, the
  canvas freezes and the status line gives no hint; surface it there.
- In firmware-runtime.ts, the `now` parameter of `serviceTaps` and
  `pollTaps` (emulated us, lines 387 and 405) shadows the `now` option
  (wall clock, line 327); rename it. (Milestone 8 removed
  `injectButton`'s local `now`, the original instance, and the tap queue
  added these two.)
- A button held (not a deferred release) across a mode switch or a hidden
  tab stays held, unchanged from Milestone 6; a UI release-on-blur would
  fix it. On a mode switch, `main.ts` routes the key's release to the Lua
  runtime, so the firmware's button stays down until the next press and
  release (Milestone 8 review).

#### Speed and timing

- **Snake's game-over draw after an eat** comes 50 to 134 ms after the
  tick that hits the wall, against about 22 ms with no eat (Task 8).
  Unexplained; possibly a high-score save, unverified.
- CPI 1 is calibrated only against a timer-paced app (Snake's move
  period). Busy work (a redraw, the Dice roll animation, which played "a
  bit faster" than the badge under the pre-Milestone-6 clock and was not
  re-checked) has not been timed against the badge.
- Zero-latency RMT, I2C0 and SPIMEM1 (carried).
- The cycle counter counts one per working step; PCER/PCMR do not gate
  it (carried).
- `PROVISION_TIMEOUT_STEPS` (60M) in `frontend/src/runtime/firmware-runtime.ts`
  is still counted in steps. Under wall-clock pacing an idle firmware
  advances only about 0.5M steps/s in the browser, so a hung provision
  takes minutes to time out. Add an emulated-time or wall-time bound.

#### Apps and UI

- Every built-in app now has an open rung on v392 (Milestone 9 Task 4),
  but no app's deeper screens are tested, and only Dice and Snake were
  compared with the badge (on v335).
- RNG: a browser-supplied seed for `APB_CTRL_RND_DATA_REG` (today fixed).
- A console pane in the frontend (the firmware's REPL is reachable
  through `serialInput`/`consoleOutput`); `serialInput` copies the whole
  JS array before the 1 MiB cap is applied, so cap or chunk it on the JS
  side when the pane lands.
- Motion input beyond the Shake button (`DeviceMotionEvent` on a phone).
- Milestone 5 Task 7 review minors still open: a fetch error message
  lingers and can be masked by a stale provision state (clear it on a
  successful `provision()` and on reset, `main.ts`); two fast clicks
  start concurrent fetches and the last to resolve wins, not the last
  click. (The third, flushing deferred releases in `stop()`, is done.)
- Small Task 7 cleanups: `speed()` returns 0 as its "no reading yet"
  sentinel; `main.ts` writes the status separator as a literal middle dot
  where the file otherwise uses escapes; `pollShake` in
  `firmware-runtime.ts` clears `lastShakeAccel` twice in a row.
- Only role hacker's frames were compared with hardware; onboarding page 8
  ("Bump to connect") is animated and not pinned; the six navigation
  frames between the launcher and Dice and Snake's after-eat frame are
  pinned but not compared.

#### Peripherals

- RMT transmissions are consumed but not decoded, so the LEDs (including
  `prov apply`'s green flashes and role colour) are never shown.
- NFC is not modeled (the self-test waits for a card forever; on v392
  Scanner, Sponsors and Workshop Quest log `hal_nfc: rc522_start:
  ESP_ERR_INVALID_STATE` and draw anyway).
- The busy phase after START on the first-run screen (about 6M steps
  under the old clock; hot PCs `0x420b_ead4`, `0x420b_b6a4`) is not
  traced.
- SC7A20H: data ready is always set (no ODR time base); 1 mg/digit is
  the typical sensitivity where the datasheet's table says 1024 counts/g;
  the face-up rest orientation is an assumption.
- I2C0: an `OP_WRITE` command with an empty TX FIFO leaves the address
  phase armed (malformed command lists only; untested).
- USB-Serial-JTAG: where the `put` task spends its CPU time per 64-byte
  packet is not identified (a per-byte VFS read is the suspect); if it
  is an emulator gap, `PUT_CHUNK_GAP_STEPS` could shrink.
- `R_M5_3`'s closure follows direct calls only (indirect VFS, littlefs
  and cJSON pointers were not followed).
- ROM stubs: `strtol`/`atoi` do not set `errno`; the 8 MiB
  `MAX_STUB_MEMORY_BYTES` cap of the string stubs has no test.
- Unmapped accesses at the idle point: ASSIST_DEBUG (`0x600c_e0xx`) and
  RTC_CNTL `0x6000_80bc` (SYSTEM's clock registers are modeled since
  Task 2). Harmless so far.
- Early log timestamps have not been compared with the badge's.

#### Build and tests

- The release suite's wall time is one chain (hacker's walk to Dice,
  about 487M steps on v335); only a faster emulator shortens it. On v392
  the suite takes about 11.7 s, and Sponsors' loaded-frame rung about 7 s
  alone.
- Test minors left from Milestone 6's reviews: `snake_eats` could assert
  growth of exactly one cell (`after * 3 == at_open * 4`) and that its
  150 ms frame is `SNAKE_OPEN_HASH`, and its `SNAKE_DOWN_SLOT` = 4
  duplicates `NAV_SLOT` without a cross-reference;
  `run_until_a_past_target_runs_nothing` uses target 0 (always past),
  where a mid-run target would test it properly; the
  `DEFAULT_STEPS_PER_FRAME` test restates the constant;
  `snake_moves_on_its_own`'s exact gap pin is as fine as `MOVE_SAMPLE_US`
  (2 ms).

#### Carried from Milestones 3 and 4 (untouched)

- ST7789 `MADCTL` (`0x20`, then `0x60`) is unmodeled; orientation is
  human-confirmed for every pinned frame so far, Snake's included.
- GPIO0 (the display D/C pin) is routed as SPI2 `FSPIWP`/`FSPIHD`; routing
  is stored only.
- eFuse block unmodeled: boot logs chip revision v0.0, the badge is v0.4.
- Flagged ROM-stub guesses: `rom_i2c_readReg*` returning 0,
  `Cache_Get_*` returning 0 (notes, history item 5).
- Edge-type interrupts are simplified: `CPU_INT_TYPE_REG` and
  `CPU_INT_CLEAR_REG` are plain storage.
- GDMA: DMA spans that cross into an adjacent RAM region are rejected;
  the RX in-link walk, CPU FIFO push/pop, the `OUT_DSCR*` pre-fetch
  registers and transfer timing are not modeled; a trailing zero-length
  `suc_eof` descriptor is never visited; `RESTART` with nothing to
  restart leaves the channel active.
- `boot_until_console_contains` checks in 250,000-step chunks, so rung
  budgets are looser than they read; stable-frame steps are quantized
  the same way.
- Small duplications: the `memcpy`/`memset` and `memcmp`/`strncmp` stub
  arms, and the `header_with_speed_size` test helper in `image.rs` and
  `rom.rs`.

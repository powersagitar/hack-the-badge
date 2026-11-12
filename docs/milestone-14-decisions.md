# Milestone 14: decisions

Milestone 14 redesigns the badge page. Spec:
`docs/superpowers/specs/2026-11-12-milestone14-badge-page-redesign-design.md`;
plan: `docs/superpowers/plans/2026-11-12-milestone14-badge-page-redesign.md`.

## Human decisions (2026-11-12)

- **Friendly identities override Milestone 5's naming.** Milestone 5
  recorded that the committed fixtures are named "Test <Role>"
  (`docs/milestone-5-decisions.md`). The human partner overrode that
  decision on 2026-11-12 in favour of the spec's friendly names
  (historical figures only, no living person; "Friendly identities").
  Everything else Milestone 5 required still holds: `example.com`
  addresses, `test-` badge IDs, the same attendee and claim IDs, empty
  contact fields, and no data from the flash dump or the boot log.
  `frontend/test/test-identities.test.ts` now checks each role's
  `display_name` and `account_email` exactly against the spec's table,
  in place of the old `"Test "` prefix check.
- **Task 1's re-pins approved.** The human partner approved the table
  below after viewing the renamed frames (all 11 roles' onboarding page
  1 and registered screen, and hacker's pages 2 and 7). No name is
  truncated; "Katherine Johnson", the longest, renders whole.

## Re-pins (Task 1)

Every new value was measured with the renamed fixtures, never derived.
The reason for every row is **fixture rename (spec: Friendly
identities)**. Each moved pin is on a chain that types the identity into
the console. The frame pins show the name. The other pins move because
the identity bytes differ, so the provisioning, and everything after it,
runs a different number of steps. No emulator code changed.
`execution_digest_is_unchanged` (20M steps of a blank-flash boot, with
no provisioning) did not move, and neither did any rung that never
provisions.

All constants are in `emulator-core/tests/boot_progress.rs`.

### Frames that show the name

| constant | old | new |
|---|---|---|
| `ROLE_ROWS` hacker page 1 | `0xbf9c_cb7e_80b4_4b1a` | `0x1c45_937d_99c0_d2ec` |
| `ROLE_ROWS` hacker registered | `0xbd6d_7613_08a7_bf24` | `0x83ca_0950_da3c_55da` |
| `ROLE_ROWS` organizer page 1 | `0x3c31_f95a_a12e_406f` | `0xbc11_2320_8e0c_d357` |
| `ROLE_ROWS` organizer registered | `0x1974_c49f_a562_08a3` | `0xaf2c_3ce0_0305_d503` |
| `ROLE_ROWS` sponsor page 1 | `0xc5ba_681b_7473_21bc` | `0x48c4_61eb_3bb4_f4d0` |
| `ROLE_ROWS` sponsor registered | `0x0bc2_dee2_1a99_c4c0` | `0xa70c_5cc0_123b_93d4` |
| `ROLE_ROWS` judge page 1 | `0x192b_1f45_de51_5d49` | `0x4d1c_e946_d7eb_3e87` |
| `ROLE_ROWS` judge registered | `0x54f8_e3d0_65a3_04c1` | `0x68a9_5319_23e9_6293` |
| `ROLE_ROWS` mentor page 1 | `0x4929_00a1_98c0_3e4d` | `0xae1a_f160_422b_d853` |
| `ROLE_ROWS` mentor registered | `0x03a9_7611_bf45_9332` | `0xa043_1930_243e_4ff0` |
| `ROLE_ROWS` volunteer page 1 | `0xd370_a34f_7bc7_d738` | `0x017b_6081_6992_ce44` |
| `ROLE_ROWS` volunteer registered | `0xfd95_a0e7_844f_4019` | `0xf09f_89b2_0ac6_9c99` |
| `ROLE_ROWS` media page 1 | `0xa0b0_7c3c_ff8f_289d` | `0x652d_f154_e30b_f6c7` |
| `ROLE_ROWS` media registered | `0x5092_210f_e8d8_f1d2` | `0x04f3_99ce_2b61_f4fc` |
| `ROLE_ROWS` staff page 1 | `0xe912_c748_aab7_152c` | `0x65db_0aa2_2612_d47c` |
| `ROLE_ROWS` staff registered | `0xc68e_18b7_16ef_40ea` | `0x5dc5_e488_daae_02b2` |
| `ROLE_ROWS` general page 1 | `0x0c04_ea57_eeb8_f8ec` | `0xa713_09fb_3fff_a163` |
| `ROLE_ROWS` general registered | `0x4574_d9ed_d13c_330a` | `0xc566_be81_c7bf_1829` |
| `ROLE_ROWS` workshop_lead page 1 | `0x96a7_9955_fffc_187c` | `0x7601_ff30_62bc_d767` |
| `ROLE_ROWS` workshop_lead registered | `0xf8e2_3246_19a6_4d76` | `0x2cf2_489d_7b3e_31e1` |
| `ROLE_ROWS` visitor page 1 | `0x2770_ab89_5a47_e576` | `0x26c7_39db_80ec_f6be` |
| `ROLE_ROWS` visitor registered | `0x647a_71d4_5ab8_cf23` | `0xea69_0e22_0d10_1b23` |
| `TO_SHAKE_PAGE[0]` (hacker page 2, "This is you") | `0xe788_a781_9328_1b08` | `0xfea7_9ef1_3a7f_bbea` |
| `ONBOARDING_PAGE7_HASH` (hacker page 7, "Badge Connect", its "Badge name:" row) | `0x110e_8782_bc4e_5ff0` | `0x7f0c_e585_b026_c6c0` |

None of hacker's other 15 walk frames moved, and neither did any
launcher or app frame. The `ROLE_ROWS` doc comment now says that
hacker's row was compared with the physical badge for layout on v335,
and that the renamed frames were checked by eye in Milestone 14.

### Chain totals

| constant | old | new |
|---|---|---|
| `HACKER_LAUNCHER_STEPS` (`hacker_walk_totals_are_unchanged`) | `342_600_776` | `342_657_333` |
| `HACKER_LAUNCHER_US` (`hacker_walk_totals_are_unchanged`) | `94_521_615` | `94_501_734` |
| `HACKER_LAUNCHER_CONSOLE_HASH` (`hacker_walk_totals_are_unchanged`; the console echoes the typed identity) | `0xabcd_cc17_7a35_80fe` | `0xd9e7_08f4_1965_a9b8` |
| `RADIO_LAUNCHER_STEPS` | `342_600_681` | `342_657_243` |
| `RADIO_LAUNCHER_US` | `94_521_615` | `94_501_734` |
| `CREDENTIAL_LAUNCHER_STEPS` | `342_894_540` | `342_985_252` |
| `CREDENTIAL_LAUNCHER_US` | `94_521_622` | `94_521_739` |
| `ORGANIZER_LAUNCHER_STEPS` | `342_889_379` | `342_921_401` |
| `ORGANIZER_LAUNCHER_US` | `94_541_620` | `94_601_811` |
| `SKIP_REGISTERED_AT_STEPS` | `84_392_000` | `84_742_000` |
| `SKIP_REGISTERED_AT_US` | `8_855_536` | `8_893_591` |

`RADIO_LAUNCHER_STEPS` is now 90 steps fewer than
`HACKER_LAUNCHER_STEPS`; ruling R-T4-1 had measured 95.

### Console needle

| constant | old | new |
|---|---|---|
| `SYNC_SNAPSHOT_LINE` | `"sync: snapshot 1026 bytes, crc ec2f90df,"` | `"sync: snapshot 1028 bytes, crc f7cd2dea,"` |

Sync's snapshot packs the identity: the name and the email are each one
byte longer than before. The doc comment now quotes the new full line,
`sync: snapshot 1028 bytes, crc f7cd2dea, 37ms, complete=1`, and keeps
the old values.

### Redraw pins (bench only)

The ignored `bench_redraws` checks these; they all run on hacker's
provisioned chain. They are copied into `frontend/src/bench/redraw-bench.ts`,
and `frontend/test/redraw-bench.test.ts` pins DOWN's.

| constant | old | new |
|---|---|---|
| `HOME_REDRAW.steps` | `18_292_336` | `18_318_598` |
| `DOWN_REDRAW.emulated_us` | `109_000` | `129_000` |
| `DOWN_REDRAW.steps` | `7_942_619` | `7_978_324` |
| `ONBOARDING_REDRAW.steps` | `2_547_134` | `2_577_629` |

DOWN's last frame change moved by 20 ms on an unchanged launcher frame
(`LAUNCHER_AFTER_NAV_HASH`). The chain reaches the launcher at a
different emulated time, so DOWN's redraw ends at a different point;
this is not a frame that shows the name. The end hashes are unchanged.

### Frontend

- `frontend/src/bench/walk.ts`: `OWN_ROLE_PAGE1_HASH`,
  `OWN_ROLE_REGISTERED_HASH`, `TO_SHAKE_PAGE[0]` and
  `AFTER_SHAKE_PAGES[0]` take the same new values as above. The WASM
  twins in `test/cpu-wasm.test.ts` use them.
- `frontend/test/test-identities.test.ts`: the exact name/email table
  (see "Human decisions").
- `frontend/test/provisioner.test.ts`: the fake console lines say
  "Ada Lovelace" and "Grace Hopper".
- No frontend test pinned a fixture byte count.

## LEDs (Task 2)

A passive WS2812 decoder, `emulator-core/src/peripherals/ws2812.rs`, a
field of the RMT model (`rmt.ws2812`). `Rmt::transmit` hands it every
word it consumes and ends its frame where it raises `TX_END` or stops
(an overrun, or `TX_STOP` on a running channel). Accessors:
`FirmwareRuntime::led_colors()` (RGB per LED, chain order) and
`led_frame_count()`, with one-line passthroughs in `emulator-wasm`
(`led_colors()` is a flattened, owned `Vec<u8>`).

### Evidence (Step 1, v392)

The evidence comes from a temporary `eprintln!` of each transmission's
words in `Rmt::transmit`, run over the boot rung, hacker's walk to Mood
LEDs and the organizer, sponsor and general walks. It was removed before
the commit.

- **Channel.** Every LED transmission is on **RMT TX channel 0** (the
  only channel the firmware starts). `CONF0` = `0x0052_0850`:
  `DIV_CNT` 8, `MEM_SIZE` 2, wrap on, idle output enabled. `SYS_CONF` =
  `0x0504_0001`: `SCLK_SEL` 1 (APB, 80 MHz), `SCLK_DIV_NUM` 0, so one
  tick is 0.1 µs. The data goes out ping-pong, as the existing RMT test
  `ping_pong_transmission_pauses_at_each_threshold_until_the_isr_acks`
  describes.
- **The encoder's symbols.** Every frame is 145 words, then the zero
  end-marker:
  - a 1 bit is `0x0003_8009`: high 9 ticks, then low 3 (0.9 µs, then
    0.3 µs);
  - a 0 bit is `0x0009_8003`: high 3, then low 9 (0.3 µs, then 0.9 µs);
  - 144 bit words, MSB first, G, R, B per LED;
  - then the reset code `0x0578_0578`, low for 1400 + 1400 ticks
    (280 µs);
  - then `rmt_tx_mark_eof`'s `0`.

  This is the shape of the `led_strip` component's RMT encoder: a bytes
  encoder for the GRB bytes, then a copy of the reset symbol.
- **Chain length: 6 LEDs** (144 bits = 6 × 24). The gate passes. The
  firmware's own strings agree: "Six WS2812B-2020 RGB LEDs ring the board
  on one GPIO3 daisy chain", "Your badge has six LEDs.", and "All 6 LEDs
  cycle R / G / B / W".

### Ruling R1: bits by relative timing

The decoder reads a bit as 1 when `duration0 > duration1`, and as 0
otherwise. It does not compare the high time with a threshold in the
RMT clock and divider, as the spec first said; the controller ruled for
relative timing. The two give the same answer for any encoder that keeps
to the WS2812B datasheet:

- T0H 0.4 µs < T0L 0.85 µs;
- T1H 0.8 µs > T1L 0.45 µs (each ±150 ns).

The firmware's observed symbols confirm it: a 1 is 9 high > 3 low, and
a 0 is 3 high < 9 low. A symbol is a bit only when `level0` = 1 and both
durations are non-zero. Any other symbol, the reset code (`level0` 0)
or an end-marker, ends the frame. The symbol layout is
`rmt_symbol_word_t`, from ESP-IDF v5.5.3
`components/hal/include/hal/rmt_types.h`, the citation `rmt.rs`
already uses. No ESP-IDF tree was available offline, so the citation
was reused, not re-read.

### Ruling R2: the chain survives a software reset

The LEDs are external chips, like the panel. A soft reset of either kind
carries `rmt.ws2812`, so the colours and the frame count go on.
`FirmwareRuntime::reset()`, the cold power-on, clears them. This is
observation only.

### What the firmware sends (measured, RGB)

| When | Frames (cumulative) | Colours |
|---|---|---|
| Boot to the first-run screen | 3 | two dark, then all six `42 42 42` (dim white) |
| `prov apply` (hacker) | +12 → 15 | three flashes of all six at `00 06 00`, each followed by two dark frames, then the role colour `06 1F 06` on all six, held on onboarding page 1 |
| Setup (hacker) | → 101 at the shake page | role colour `07 25 08` on all six; a fill 0→5 at `03 11 03`; `07 25 08`, `0C 42 0E`; then a single LED at `0C 42 0E` chasing 0→5, six laps and two more steps; then indices 3 and 4 together, blinking 7 times (before and on page 6) |
| After the shake, to the registered screen | 102 → 116 | all six at `0C 42 0E`, then dark; **the registered screen is dark** |
| Mood LEDs open | 227 | all six breathing green (default mood): `00 02 01` at the stable open frame, peaking at `00 21 0E` 1.7M steps later; one breath, dark pause included, is about 7.8M steps |

These are role colours: the same walk on other roles sends the same
patterns in another colour. After `prov apply`, organizer shows
`01 07 13` (blue), sponsor `30 05 05` (red) and general `3B 2D 06`
(amber). At full brightness they are `04 17 42`, `42 06 06` and
`42 32 07`. All walks have 116 frames at the registered screen.

### Rungs (boot_progress)

- `leds_flash_during_prov_apply`: from the first-run screen (3 frames,
  `42 42 42` × 6), the rung types `prov apply` and samples every 1,000
  steps up to the prompt after PROV OK. It checks 12 more frames, 3
  flashes of `00 06 00` × 6, and the role colour `06 1F 06` × 6.
- `leds_go_dark_on_the_registered_screen`: 116 frames, all six off.
  **Deviation from the brief's
  `leds_show_the_role_colour_when_registered`:** at hacker's registered
  checkpoint the LEDs are dark, measured on every walk. The role colour
  is shown after `prov apply` and at the end of Setup, but not on the
  registered screen. The rung is named for what it pins, and the role
  colour is pinned in the `prov apply` rung.
- `mood_leds_light_the_chain`: from the Mood LEDs open checkpoint, 227
  frames and `00 02 01` × 6, then `00 21 0E` × 6 after 1,700,000 more
  steps.
- `launcher_opens_mood_leds` now takes its arguments from named
  constants (`MOOD_LEDS_CELL`, `MOOD_LEDS_RESPONSE_STEPS`,
  `MOOD_LEDS_OPEN_HASH`), with the same values as before, so the new
  rung shares its `OPENED` checkpoint.

Nothing moved. Every pre-existing pin is byte-identical: the diff
removes no value, and `execution_digest_is_unchanged` and
`hacker_walk_totals_are_unchanged` pass at their Task 1 values.

### Chain index → board position (for Task 11's `LED_CHAIN_TO_SPOT`)

**Inconclusive.** What the evidence shows:

- The firmware says the six LEDs "ring the board" on one daisy chain.
- Setup's fill (0→5) and chase (0→5, then 0 again) step through the
  chain indices in order. On a ring, that means the chain follows the
  ring, so neighbouring indices are neighbouring LEDs.
- The shake page lights indices 3 and 4 together. Under that reading
  they are a neighbouring pair.

What it does not show is where index 0 sits or which way the chain
runs. Every pattern seen is uniform or index-sequential. The
`out_top_left`-style strings in the firmware are LVGL alignment names
for the Lua API, not LED positions. The board's traces in the photo, or
the human partner watching the physical badge's Setup chase (where it
starts and which way it turns), would settle it.

## Dependencies added

None in Task 1. None in Task 2.

Removed: `fengari` (^0.1.5) and `fengari-interop` (^0.1.4), removed in
Task 4 with the Lua sandbox.

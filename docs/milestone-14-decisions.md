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
- **The badge hangs (Task 9's axis gate).** v392 reads single
  accelerometer axes in several apps ("Accelerometer axes (Task 9)"
  below), so the hanging rest changes what they show. The human partner
  answered "Yes, hang it": the hanging physics goes ahead, and Level,
  Tilt Maze, Accel, Diag, Hopper, 6-7 Counter and the Lua `badge.sensor`
  bindings behaving as they would on a badge hanging from a lanyard is
  accepted as faithful.
- **BOARD_AXES is assumed, verified later.** The human partner answered
  "Assume, verify later": `BOARD_AXES` is right = +x, up = +y,
  out = +z, and it is **UNVERIFIED**. The human partner checks the signs
  on the physical badge (for example with Level or Accel) in Task 14's
  browser check, and `BOARD_AXES` is fixed then.

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
relative timing.

**The justification is the firmware's observed encoder**, not the
datasheet in general. A 1 is 9 ticks high, then 3 low (0.9 µs, then
0.3 µs). A 0 is 3 high, then 9 low (0.3 µs, then 0.9 µs). The longer
half always names the bit, so for these symbols relative timing gives
the same answer as an absolute high-time threshold.

The part is the WS2812B-2020: the firmware's string names it, and its
280 µs reset fits the 2020/V5 sheets. Those sheets give these ranges,
**recalled and not re-read, so unverified**:

- T0H 220–380 ns;
- T1H 580 ns–1 µs;
- T0L and T1L about 580 ns–1 µs;
- RES at least 280 µs.

Under those ranges T1H > T1L is not guaranteed. An encoder with other
timing, such as a long T1L, could need the absolute threshold instead.
The badge's encoder is fixed, and its symbols are pinned through the
rungs. A symbol is a bit only when `level0` = 1 and both
durations are non-zero. Any other symbol, the reset code (`level0` 0)
or an end-marker, ends the frame. The symbol layout is
`rmt_symbol_word_t`, from ESP-IDF v5.5.3
`components/hal/include/hal/rmt_types.h`, the citation `rmt.rs`
already uses. No ESP-IDF tree was available offline, so the citation
was reused, not re-read.

### Ruling R2: the chain survives a software reset

The LEDs are external chips, like the panel. A soft reset of either kind
carries `rmt.ws2812`, so the colours and the frame count go on. A frame
the reset cut off mid-way is ended first: its whole LEDs are published,
and its partial bits are dropped.
`FirmwareRuntime::reset()`, the cold power-on, clears them. This is
observation only.

### What the firmware sends (measured, RGB)

| When | Frames (cumulative) | Colours |
|---|---|---|
| Boot to the first-run screen | 3 | two dark, then all six `42 42 42` (dim white) |
| `prov apply` (hacker) | +12 → 15 | frames 4–5 dark (sent before `PROV OK` prints); then three times a flash of all six at `00 06 00` followed by two dark frames (6–14); then the role colour `06 1F 06` on all six (15), held on onboarding page 1. 2 + 3 × 3 + 1 = 12 |
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
  steps up to the prompt after PROV OK. It checks 12 more frames (2
  dark, 3 × a flash and 2 dark, then the role colour), 3 flashes of
  `00 06 00` × 6, and the role colour `06 1F 06` × 6.
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

All pinned exactly in `frontend/package.json`.

| package | exact version | why |
|---|---|---|
| `react` | 19.3.1 | The page's UI (Milestone 14 replaces `ui/shell.ts`'s DOM building with components). |
| `react-dom` | 19.3.1 | Renders the React page into `#root`. |
| `matter-js` | 0.20.0 | The lanyard and badge physics (the design's swing). |
| `radix-ui` | 1.8.1 | Primitives under shadcn's Button (`Slot`), Select and ScrollArea; added by the shadcn CLI. |
| `class-variance-authority` | 0.7.1 | shadcn's Button variants (`cva`). |
| `clsx` | 2.1.1 | shadcn's `cn` (`src/lib/utils.ts`). |
| `tailwind-merge` | 3.7.0 | shadcn's `cn` (`src/lib/utils.ts`). |
| `lucide-react` | 1.72.0 | The Select's chevron and check icons. |
| `tailwindcss` (dev) | 4.3.5 | The styling and theme tokens. |
| `@tailwindcss/vite` (dev) | 4.3.5 | Tailwind v4's Vite plugin. |
| `@vitejs/plugin-react` (dev) | 6.3.0 | JSX/React in Vite 8. |
| `@types/react` (dev) | 19.3.0 | React's types. |
| `@types/react-dom` (dev) | 19.3.0 | react-dom's types. |
| `@types/matter-js` (dev) | 0.20.2 | matter.js's types. |
| `@testing-library/react` (dev) | 16.3.3 | Component tests over happy-dom. |
| `@testing-library/dom` (dev) | 10.4.2 | `@testing-library/react`'s required peer. |

The shadcn components came from the CLI, `bunx --bun shadcn@4.23.0 add
button select input scroll-area` (not a dependency). Its current
`new-york-v4` registry imports `cn` from shadcn's new npm package `cn`
(0.5.2, published 2026-11-02) and lists it as a dependency. We kept the
classic `src/lib/utils.ts` (`clsx` + `tailwind-merge`) instead, as the
plan asks, rewrote the four components' import to `@/lib/utils`, and did
not add `cn`. The CLI also resolves the `@` alias only with a tsconfig
`baseUrl`, which TypeScript 7 removed (TS5102), so `baseUrl` was present
only while the CLI ran; `paths` alone serves `tsc`, Bun and Vite (whose
`@` alias is in `vite.config.ts`).

Removed: `fengari` (^0.1.5) and `fengari-interop` (^0.1.4), removed in
Task 4 with the Lua sandbox. Tasks 1 and 2 added none.

## Build size (Task 7)

`bun run build`, `frontend/dist/`:

- Before (main.ts page): JS 27.40 kB (gzip 10.31 kB) + bridge 6.24 kB +
  worker 10.81 kB, CSS 2.71 kB, wasm 143.64 kB; `dist/` 3380 KiB.
- After (the empty React shell): JS 222.71 kB (gzip 69.81 kB), CSS
  22.03 kB (gzip 5.12 kB); `dist/` 3428 KiB.

Until Task 12 wires the controller, the page does not reference the
firmware Worker, so the worker, bridge and wasm chunks are absent from
the build; they return then.

## CONSOLE.LOG (Task 8)

- **Line cap: 2,000 entries** (`DEFAULT_MAX_LINES` in
  `frontend/src/ui/console-log.ts`), the builder's call within the spec's
  "thousands". The newest are kept. A held partial line longer than
  4,096 characters (`MAX_LINE_CHARS`) is flushed as a line, so a flood
  with no newline stays bounded too.
- **Flood handling.** The model notifies once per append call, however
  many lines it adds, and none for a call that only extends the held
  partial line. `getSnapshot()` rebuilds one frozen copy per change, at
  most, and React batches a task's notifications into one render. Rows
  are memoised on their (immutable) entry, so a render creates only the
  new rows.
- **Partial lines are held**, as the brief says: a prompt with no newline
  (`hack> `) shows once more output follows it. A reboot flushes the old
  boot's held tail as its own line before the `— badge rebooted —`
  marker.
- **Text is passed through.** Only `\n` splits and `\r` is dropped; U+FFFD,
  lone surrogates and escape bytes (ANSI colour codes included) are
  shown as they came.
- **Mounting** is left to Task 12 (controller ruling R5): the panel takes
  a `log: ConsoleLog` prop, and the log and its `onConsole` wiring belong
  to the controller.

## Power cycle (Task 3)

`FirmwareRuntime::power_cycle()` (WASM `power_cycle()`) is the page's
power switch: `reset()`'s cold power-on, sharing its body, except that
the emulated flash is kept, as on the physical badge, so the identity,
NVS and `reboot.cfg` survive. Everything else is lost as in `reset()`:
RAM, RTC memory and the RTC_CNTL `STORE` registers, every peripheral,
the panel, the console, queued serial input and the diagnostics.
Emulated time, the step count and the soft-reset count restart at 0, and
`rtc_get_reset_reason` reports `POWERON_RESET`. **The LEDs are cleared**,
as `reset()` clears them (Ruling R2's cold case): `led_colors()` is empty
and `led_frame_count()` is 0 until the new boot's first frame. Held
buttons, the host-set acceleration and the factory MAC are host input
and are re-applied, as `reset()` does. `reset()`'s behaviour is
unchanged; `power_cycle` is a new entry point, and no pre-existing pin
moved.

Observed (v392, role hacker): from the registered checkpoint, a power
cycle boots through the splash straight to My Badge's registered screen,
the same hash as the walk's (`0x83ca_0950_da3c_55da`), with no reboot and
no Setup. First stable at 38,500,000 steps, confirmed at 46,500,000 steps
and 1,554,210 µs. Rung: `power_cycle_boots_to_the_registered_screen`
(budget 59,000,000 steps, the confirmed step + 25%, rounded up).

## Accelerometer axes (Task 9)

Traced in v392's disassembly (`firmware.bin`'s IROM segment at
`0x4200_0020`, llvm-objdump), from the direct reader `0x4200_ae2e`
(Milestone 10's bump trace) and the sample cache.

- **The API.** The direct reader `0x4200_ae2e` (it takes the mutex,
  then `0x4200_a9f2` reads OUT_X..OUT_Z as floats in mg). The cache task
  `0x4200_ae9e` stores a sample every `vTaskDelay(2)` through
  `0x4200_a970` into `0x3fca_32f4`. Its getter `0x4200_aed4` has three
  callers, `0x4205_bc46`, `0x4205_c3f4` and `0x4205_c566`, in the Lua
  `badge.sensor` bindings, which hand x, y and z to Lua apps.
- **The direct reader's ten callers.** Each feature is named from the
  strings next to it:

| call site | feature | how it uses the axes |
|---|---|---|
| `0x4201_3db4` | "6-7 COUNTER" ("SHAKE UP / DOWN") | x, y and z each feed a chart series (`0x420c_b578`), printed as `X %+.0f Y %+.0f Z %+.0f mg`; then `0x4205_7f5c(x, y, z, t)` (an up/down detector, not traced) |
| `0x4201_4636` | Accel (`Accel`/`ACC`/`accel`) | prints X, Y and Z separately |
| `0x4201_d25a` | Connect's bump | the change between two readings: does not depend on how the badge is turned (Milestone 10) |
| `0x4202_1d48` | Diag | display |
| `0x4202_81c0` | Hopper ("HARDWARE PHYSICS", "TILT + SHAKE") | tilt and shake, per axis |
| `0x4203_0234` | Level ("Pitch %s  Roll %s") | x and y separately: the bubble from x and y clamped to ±500 (`0x3c15_5328`/`2c`); "level" (green) only when \|x\| < 100 and \|y\| < 100 (`0x3c15_5330`); pitch and roll in degrees (÷1000, ×57.2958; `0x3c15_5334`/`38`) |
| `0x4203_1cee` | Tilt Maze | ball velocity += (−x, +y) × 0.00012 per tick (`0x3c15_5348`), damped by 0.985 and clamped to ±4 (`0x3c15_51fc`, `0x3c15_518c`/`98`) |
| `0x4204_5392` | the hardware self-test | display |
| `0x4205_0ae4` | the bump-pairing module (`BPI1`) | not traced |
| `0x4205_a654` | near Workshop Quest / `sensor.shake` | not traced |

- **Ruling.** "Shake it!" (a magnitude EMA, Milestone 5) and Connect's
  bump (the change between two readings) do not depend on how the badge
  is turned, but the apps above read single axes. Level and Tilt Maze
  treat face-up (x = y = 0) as flat. With the hanging rest (0, +1000, 0),
  Level's bubble sits at one edge (pitch about 90°) and Tilt Maze's ball
  keeps rolling along one axis. None of these apps is among the 16
  launcher cells that `boot_progress.rs` pins. How they are reached was
  not traced. The human partner accepted this as faithful ("Human
  decisions" above).
- **BOARD_AXES** = right +x, up +y, out +z (`frontend/src/motion/axes.ts`),
  **UNVERIFIED**. The photo (`local/m14-design/front.webp`) cannot
  settle it. The likeliest accelerometer is the small LGA part just
  below the ESP32-C3 module, about (41.4 %, 62.1 %) of the photo, which
  is `CHIP_SPOT` in `lanyard.ts`. Its pin-1 mark is not legible. Task
  14's browser check on the physical badge fixes the signs.
- **Nothing in the firmware treats "not face-up" as a special case**
  beyond these apps' own tilt use. No other code reads the sensor.

## Lanyard physics and the sensor (Task 9)

`frontend/src/motion/`: `source.ts` (the `MotionSource` seam),
`axes.ts`, `sensor.ts` (pure) and `lanyard.ts` (matter.js, headless).
Review fix round 1 changed the impact detector, the grip clamps,
relayout, the start-up settle and the gain; this section describes the
result.

- **World.**
  - **Rope.** 10 point-mass segments (0.04 each, frictionAir 0.03, no
    collisions) from the anchor at (W/2, −10) to the badge's ring. The
    ring is pinned `clip + bh/2` above the badge's centre (the design's
    `dPC`).
  - **Badge and walls.** The badge is a rigid body of mass 1,
    frictionAir 0.022, restitution 0.35. The side walls are static
    colliders at the stage edges. As the design's `detect()` clamps C,
    the badge's bounds are also clamped inside the walls after every
    substep.
  - **Time.** Gravity is 2400 px/s². Every substep is a fixed 1/120 s,
    at most 12 per `step()`; any further debt is dropped.
  - **Layout.** The clamps are the design's `layout()`, as the brief
    gives them.
- **Start-up settle.** A (re)build runs 240 silent substeps (2 s), then
  zeroes every velocity. The rope stretches as it takes the badge's
  weight. Without the settle, the first 0.6 s bounced about ±180 mg at
  a gain of 0.1. That is enough to trip Connect's 400 mg detector at
  higher gains, on page load or on any relayout.
- **Relayout.** As the design does, a badge height that changes by more
  than 0.5 px rebuilds the world at rest, which ends a drag. The same
  height keeps the swing: the rope, the badge and any grip move with
  the anchor, the walls are re-made, and the rope links take the new
  length.
- **Damping.** With the badge's frictionAir at 0.014 (the design's
  0.993 per step), a drag-and-release was still 36 mg from rest after
  5 s, against the test's limit of 30. At 0.022 it is about 12 mg.
- **Drag.** `grab` pins the badge at the grabbed point with a
  stiffness-0.6 constraint. The grip's target is the pointer, clamped
  three ways:
  - to the stage ±40 px (as the design does);
  - to within the rope's reach of the anchor, plus 40 px;
  - so that the badge's centre stays at most 40 px past where it would
    touch a wall.

  The 40 px overreach lets a fling arrive at the wall at speed. The
  wall collider and the bounds clamp stop it, not the grip.
  - Round 1 clamped exactly at the wall and the reach. That braked a
    fling to a stop at the wall: on 1000 px the centre never passed
    754, with the wall at 823. Fast flings at 820, 600, 430 and 360 px
    then missed their impact.
  - With no clamp at all, a slow push dragged the badge through the
    wall, and on release the rope whipped it above the anchor, flipped
    over (y = −467, angle −2.36).
  - Speeds are capped at 7200 px/s (the design's 60 px per step) and
    the spin at 40 rad/s. A NaN or runaway body rebuilds the world at
    rest.
- **Sensor.** The chip spot's acceleration comes from second
  differences of its position across substeps, so the rotational terms
  are included. Gravity is a true 1 g through the badge's tilt. Each
  axis is clamped to ±2000 mg and rounded, and a non-finite input reads
  as rest.
  - The reading is the average of the substeps' accelerations within
    one `step()` call. After a stall, up to the 12-substep cap, a short
    spike such as a wall hit is diluted across that call's substeps.
- **Impacts.** One geometric detector on the badge's bounds, so the
  leading corner's rotation counts. It works from the badge's actual
  displacement, which includes the grip's position corrections.
  - **The rule.** A contact episode starts when a bounds edge reaches
    its wall (within 1 px) after at least 50 ms away from it. It is an
    impact if that edge approached faster than `IMPACT_PX_S` =
    **840 px/s**, over the substep before contact or the substep of
    contact. 840 px/s is the design's `detect()` threshold of 7 px per
    1/120 s step, unchanged. At most one impact fires per 800 ms of
    simulated time. A listener that throws is caught and logged, and
    never aborts `step()`.
  - **Why not matter's collision events.** Round 1 used
    `collisionStart`, which reads matter's velocity at contact. That
    missed flings where the grip moved the badge by position
    correction: a 3-frame fling on 600 px jumped the badge 39 px into
    the wall within one substep while its velocity stayed small.
    Adding `collisionActive` fired repeatedly while a held badge rested
    against the wall. matter's velocity there is pre-resolution, a
    steady ~1400 px/s, plus a 7 rad/s angle flicker as the grip and the
    wall fight.
  - **Measured (the reviewer's probe).** Grabbed at the centre, the
    pointer was dragged past the wall over 50, 100, 150, 250 and
    400 ms, either released at 60% of the way or held. On 1000×780,
    820×780, 600×780, 390×640, 430×800 and 360×600 every case fires
    exactly one impact.
    - A push over 1000 ms or 1500 ms fires none on any stage while it
      is held.
    - On the narrow stages (600 px and the phones) the released badge
      then swings into the opposite wall fast enough to count once.
      That is a real hit.
    - ±20 px and ±40 px drags at 1 Hz fire none on any stage.
    - ±60 px at 2 Hz fires 1–2 on the stages narrower than 820 px,
      where the badge has 50–120 px of room.
  - `impulse("knock")` is a 30,000 px/s² push toward the nearer wall,
    for at most 400 ms, ending at contact. It fires exactly one, on both
    layouts.
- **Peer bump.** `peerBump()` pushes the badge at 30,000 px/s² for
  100 ms, away from the nearer wall. That reads about 2000 mg (clamped),
  far above Connect's 400 mg, for longer than its 40–70 ms sample gap.
  For 1000 ms afterwards no outgoing impact fires, so a peer's bump is
  never echoed back. A shake started during a peer bump replaces only an
  earlier shake's pushes, never the bump's.
- **Shake key.** `impulse("shake")` is six alternating 80 ms pushes at
  40,000 px/s².
  - At gain 0.1 it passed "Shake it!" on the WASM twin (a scratch
    probe, not a committed test).
  - At gain 0.2 the detector replica (below) passes it at 100% of
    sampling phases on 1000×780 and 96% on 390×640.
- **The firmware's sample rate.** With the acceleration changed every
  1 ms on page 6, the frame changed every **~80 ms** (12.5 Hz; gaps 76
  to 81 ms over 1 s). The page's tick runs the detector and then prints
  `G-force`, so the detector samples at about that rate. That is
  inferred, not traced. Connect samples every 40–70 ms (Milestone 10).
  The lanyard emits one sample per display frame, 60 Hz, which is
  4–5× faster than either detector needs. Delivery (Task 12) should
  keep about that rate.

### GAIN_MG_PER_PX_S2 = 0.2

Measured with `frontend/test/physics-shake.test.ts`'s method:
- The WASM twin is walked to Setup page 6.
- The lanyard runs on a 1000×780 desktop stage, with the badge grabbed
  at its centre.
- Every 1/60 s the lanyard's reading goes to `setAcceleration`, and the
  emulator runs to that emulated time.
- Each drag is 0.5 s at rest, 2 s of drag and 1 s after release.

The verdict is A: once the page is shaken, A opens page 7. The page
prints the live `G-force` and `Peak`, so the frame changes with any
motion, and "leaves page 6's hash" cannot tell the two cases apart.

The table is the final physics, after fix round 1. "Phase pass" comes
from the detector replica described below the table.

| gain | gentle (±20 px, 1 Hz) peak \|a\| | gentle: page | vigorous (±120 px, 3 Hz) peak \|a\| | vigorous: page | vigorous phase pass | gentle phase pass |
|---|---|---|---|---|---|---|
| 0.02 | 1018 | stayed | 2728 | stayed | – | – |
| 0.03 | 1026 | stayed | 2828 | passed | 0.42 | 0 |
| 0.04 | 1035 | stayed | 2828 | passed | 0.42 | 0 |
| 0.05 | 1044 | stayed | 2828 | passed | 0.50 | 0 |
| 0.08 | 1071 | stayed | 2828 | **stayed** | 0.18 | 0 |
| 0.1 | 1088 | stayed | 2828 | **stayed** | 0.20 | 0 |
| 0.12 | 1106 | stayed | 2828 | passed | 0.81 | 0 |
| 0.15 | 1132 | stayed | 2828 | passed | 0.97 | 0 |
| **0.2** | **1177** | **stayed** | **2828** | **passed** | **1.00** | **0** |
| 0.25 | 1222 | stayed | 2828 | passed | – | – |
| 0.3 | 1267 | stayed | 2828 | passed | 1.00 | 0 |

Peak |a| is the largest magnitude in mg. 2828 is both in-plane axes at
the ±2000 clamp.

- **The headroom check.** The face-normal axis reads 0, so a shake
  needs both in-plane axes high at once (the ~1304 mg trigger above a
  1000 mg baseline). The vigorous drag reaches 2828 mg, both axes
  clamped, at every gain from 0.03 up.
- **Sampling phase decides the pass, not the gain alone.**
  - The firmware's detector samples about every 80 ms.
  - Its EMA b then has to sit far enough from a sampled m. At 3 Hz a
    sample lands on a dip or a peak only at some phases.
  - So the firmware outcome is not monotonic in the gain: 0.08 and 0.1
    stay, while 0.03–0.05 and 0.12 pass.
- **The replica.** A replica of the detector (`b = 0.92 b + 0.08 m`,
  trigger `|m − b| > 1200`) holds each frame's reading. It samples at
  70, 80 and 90 ms periods, every phase in 2 ms steps (135 runs). The
  fraction of runs that pass matches the firmware: about 20% at 0.08
  and 0.1, where the twin stayed; about 45% at 0.03–0.05, where the
  twin happened to pass.
- **Why 0.2.** It is the lowest gain measured where the vigorous drag
  passes at every sampled phase and period. The gentle drag passes at
  none, even at 0.3.
  - **Peaks.** The gentle drag peaks at 1177 mg; the vigorous one reads
    2–3 g (2828 mg at the clamp), as the spec's tuning rule asks.
  - **Connect's bump.** The largest change between readings 50–67 ms
    apart is 299 mg for the ±20 px, 1 Hz drag, below Connect's 400 mg.
    A ±40 px, 1 Hz drag reads 548 mg, so it can trip Connect's local
    detector. The spec accepts that: Connect's own detector may fire on
    midair motion, and only wall impacts are sent to peers.
- **Superseded picks.** Round 1 picked 0.1, which passed the twin then
  only by phase. Fix round 1's physics changes moved it to "stayed",
  which exposed the sensitivity.
- `physics-shake.test.ts` takes about 7–10 s: one walk to page 6 serves
  both tests, gentle first. The default `bun test` grew from about 16 s
  to about 23 s.

## Phone motion (Task 10)

`frontend/src/motion/device-motion.ts`, the second `MotionSource`.

- **Mapping.** `accelerationIncludingGravity` (m/s²) / 9.80665 × 1000 ×
  `signFix`, phone x/y/z as the board's right/up/out through
  `BOARD_AXES`, clamped by sensor.ts's `clampRound` (now exported, so
  both sources share one clamp).
- **`platformSignFix`** returns 1 everywhere until Task 14's real-device
  check on iOS and Android fills its table.
- **"Unsupported".** `motionAvailable()` is false when
  `DeviceMotionEvent` is missing or `isSecureContext` is false (Chrome
  hides the API outside a secure context; iOS keeps it but never
  grants it over plain HTTP). `requestMotionPermission()` returns
  "unsupported" then; "granted" where there is no
  `requestPermission` (Android, desktop); otherwise iOS's answer, with a
  rejection (e.g. no user gesture) read as "denied". A desktop browser
  may report available and never send an event; it then never drives
  the badge.
- **Ignored events.** A null `accelerationIncludingGravity`, or any
  null or non-finite component, produces no sample.
- **Peer pulse.** +1000 mg on chip x (index 0) for readings in the
  100 ms after `peerBump()`; peerBump itself emits nothing, the next
  phone events carry it (phones send at about 60 Hz).
- **Impacts.** The Euclidean jump between consecutive *raw* readings
  (the pulse excluded, so a peer's bump is never echoed back) above
  1500 mg fires `onImpact`, at most once per 800 ms. `stop()` forgets
  the previous reading, so a restart never counts as a jump.
- **Clock.** `createDeviceMotionSource` takes an optional third
  argument, `now` (ms, default `performance.now`), beyond the brief's
  signature, so the tests drive the pulse and the cooldown by hand.
- **onMotion** gets the same samples as the sink, only while started.

## Badge component (Task 11)

`frontend/src/ui/Badge.tsx`, `Stage.tsx`, `badge-geometry.ts` and
`keyboard.ts`; the photo is `frontend/public/badge/front.webp` (sha256
`f1e63701…89bd4`).

- **`LED_CHAIN_TO_SPOT = [0, 1, 3, 5, 4, 2]` is a best guess (Ruling
  R8).** Task 2's evidence ("Chain index → board position" above) shows
  the chain follows the ring but not where it starts or which way it
  turns. The guess walks clockwise from the top-left spot: TL, TR,
  mid-right, bottom-right, bottom-left, mid-left. It fits the shake page
  lighting chain 3 and 4 as a neighbouring pair (bottom-right and
  bottom-left). **The human partner confirms it on the physical badge in
  Task 14** (where Setup's chase starts and which way it turns).
- **Desktop stage width cap: 1016 px** (`STAGE_MAX_W`, an inline `maxWidth` on the desktop layout). The design's stage at 1440 px is about 1016 px (border
  box), and the page's 1360 px cap keeps it there at wider windows, so
  the cap changes nothing the design draws. It guards the lanyard: past
  about that width the rope can't swing the badge into a wall (Task 9),
  so drags would never bump.
- **The LED glow.** Opacity is `max(r,g,b)/255` (off is 0), as the
  brief says, and `--rgb` is the colour scaled to full brightness, so
  the brightness is counted once, not twice. The firmware's colours peak
  at about `0x42`, so the glows read at about 26% opacity; Task 14's
  look decides whether that needs a curve.
- **Layout and hit sizes** follow `matchMedia("(min-width: 768px)")`
  (Tailwind's `md`, the same breakpoint as the shell), not
  `pointer: coarse`. A layout change builds a new lanyard (its size
  clamps differ); a resize relays it out.
- **Keyboard.** `keyAction(code, target?)` takes the event's target as
  an optional second argument, which the brief's one-argument signature
  leaves out but its "ignores typing in inputs" needs. Fields (input,
  select, textarea, editable content) keep their keys. Enter and Space
  on a focused button or link stay that control's activation, so a
  keyboard-only visitor can still click a board button (a tap) or
  "Reset swing". Keys with Ctrl, Meta or Alt are ignored, a held key
  repeats nothing, and a window blur releases every key-held button.
- **AUX1** toggles through the `pad` source both from the pointer and
  from Space, so the runtime's per-source hold matches the drawn state.
- **The power hit area** keeps the design's `.hit` translate (it is
  centred on 34.1%, 1.5%), as the design draws it.
- **"Reset swing"** is `lanyard.reset()` (Task 9's rebuild at rest).
- **The rope** is drawn as the design's `tick`: quadratic segments
  through the rope's nodes, ending 12 px above the ring, toward the
  anchor.

## Controller and tools (Task 12)

`frontend/src/ui/badge-controller.ts` (the store), `ToolsPanel.tsx`,
`frontend/src/radio/links.ts`; `main.ts`, `ui/shell.ts`, `ui/shell.css`
and `test/shell.test.ts` are deleted, and their still-relevant coverage
moved to `test/badge-controller.test.ts`, `test/tools-panel.test.tsx`,
`test/links.test.ts` and `test/app-shell.test.tsx`.

- **The store.** `createBadgeController(deps)` keeps the old controller's
  logic (badge lock, lazy Worker boot that is not cached on failure, the
  hub's running gate, the skip gate) and exposes one immutable
  `BadgeSnapshot` through `subscribe`/`getSnapshot`. `main.tsx` polls
  `refreshStatus()` every 250 ms, as the old `main.ts` did, and every
  action refreshes too.
- **Beyond the brief's interface:** `client` (a stable `BadgeClient` for
  the stage, forwarding to the firmware once it boots, LED listeners
  included), `setCanvas` (the stage's `onCanvas`; the Worker client
  blits through a forwarder, so the Worker boots before the canvas
  mounts; its `ctx` type is now `Pick<…, "putImageData">`),
  `cancelHost`, `refreshStatus`, and the snapshot field `hostingRoom`.
  `createLinks` takes a third `options` argument: `note` (the console's
  `appendPage`) and `pageEvents` (where `pagehide` comes from).
- **Page lines.** The texts are the old status line's. Each stream is
  logged when its own text changes: `rebooting…` on a reboot's start, the
  provision's (`waiting for the console…`, `provisioning…`, its PROV OK
  line, `failed: …`) and the skip's (`skipping setup…`, `skip failed: …`;
  `done` adds nothing). The old "whichever was started last" rule
  (`lastStarted`) is gone: in a log both streams coexist, and an old skip
  failure the Worker keeps reporting is not repeated. The speed is not a
  line; it is the header's SPEED INDEX. `rebooting…` can land just after
  the console's reboot marker, since the marker comes with the serial
  chunk and the line with the next status poll.
- **Boot failure (Task 4 minor).** A failed boot from a provision now
  says `could not start the badge` (`START_FAILED_NOTE`); only a failed
  identity fetch says `could not load <role>`, and keeps the badge.
- **Hidden page (Review Focus 1).** On hidden, the controller sets the
  hanging rest `[0, 1000, 0]` through `BOARD_AXES` (`HANGING_REST_MG`) and
  its sink drops samples until visible. The client holds that reading (no
  animation frame runs while hidden) and posts it just before `start`
  (Task 6). A badge booted on a hidden page gets the rest before its
  first start.
- **Phone motion.** `usePhoneMotion(true)` calls the permission request
  before any `await` (iOS's gesture). Denied or unsupported keeps
  `"physics"` with a `[page]` line saying why. The stage's `sink` is the
  controller's while `motion` is `"physics"` and `null` otherwise, which
  stops the lanyard's readings; the phone source gets the same gated sink.
  Lanyard-follows-phone, drag/keys off and bumps are Task 13's (R3), so
  `onLanyard` is not wired yet. Desktop Chrome on localhost reports
  DeviceMotion available, so the button shows there (Task 10's note).
- **BADGE LINK.** `summary()` is `Hosting <room>`, `Linked to N badge(s)`
  (joined by " · " when both apply), or the signal client's last message.
  **Cancelling a hosted room:** while a room waits, Host link becomes
  "Cancel host" (the design's `.ghost.on`), which closes the room on the
  server and notes `Stopped hosting room <room>.`; the design has no other
  slot for it. An open link that closes notes `A badge link closed.`; a
  link that never opened notes its failure's reason instead. `pagehide`
  closes every link and cancels a waiting room.
- **Unload.** `main.tsx` disposes the controller on `pagehide` (not
  `beforeunload`, which mobile browsers skip), freeing the badge lock; a
  page restored from the back/forward cache reloads.
- **Escape and the ROLE select (Task 11 minor).** A real conflict: with
  the Radix Select open, focus is on a `role="option"` div, so Escape,
  the arrows and Enter also pressed HOME/UP/DOWN/START. `keyAction` now
  ignores targets with role `combobox`, `listbox` or `option`; Radix still
  closes on Escape (`tools-panel.test.tsx` opens the real Select and
  checks both).
- **CONSOLE.LOG rows** got `shrink-0`: mounted in the aside's fixed-height
  column, wrapped rows shrank to one line and overlapped (seen in the
  browser).
- **Tab notice** colours are theme tokens (`--notice`, `--notice-border`,
  `--notice-foreground`), from the design's banner.
- **Build:** JS 483.81 kB (gzip 155.95 kB), CSS 27.54 kB; the worker,
  bridge and wasm chunks are back.
- **Fix round 1 (review).**
  - Phone motion: a sequence number and a pending flag. A second "on"
    click while the permission prompt is open asks nothing, and an "off"
    (or dispose) while it is open wins over a late grant. The request
    itself is still made synchronously inside the click.
  - `dispose()` sets the runtime to null, closes the hub's running gate
    (`setRunning(false)`), and stops a boot that is still in flight from
    ever starting. `Links.dispose()` removes its `pagehide` listener and
    closes the links. `main.tsx` clears its status poll on `pagehide`.
  - A store subscriber that throws is logged and does not stop the
    others or `refreshStatus`.
  - `main.tsx` guards construction. If the hub, the controller or the
    first render throws, the page shows a plain "Failed to start: …"
    (the old `main.ts` fallback).
  - **Focus after choosing a role: kept on the ROLE trigger** (the
    reviewer's choice 6). Radix returns focus to the trigger when the
    list closes, and that is the expected keyboard behaviour: a
    keyboard-only visitor can Tab on to PROVISION from there. Blurring
    would drop that visitor's place to `<body>`. While the trigger has
    focus, the badge keys stay with it (ruling above): its arrows reopen
    the list. A mouse visitor's next click (PROVISION, the board) moves
    focus off it, and the keys work again.
  - The tab notice's screen clear is covered again (the ported
    `notice_clears_the_canvas`). The screen is cleared when the badge is
    lost with the canvas already mounted, and when the canvas mounts
    under the notice. A frame blits into whichever canvas is current.

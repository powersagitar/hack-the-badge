# Milestone 5: design decisions, remarks, and the Milestone 6 backlog

Milestone 5 takes the real-firmware emulator from Milestone 4's end state
(blank flash settles on My Badge's first-run screen) to the app launcher,
by provisioning the emulated badge through the firmware's own USB console
(`put` + `prov apply`). The design spec is
`docs/superpowers/specs/2026-10-07-milestone5-provisioned-launcher-design.md`
and the plan is
`docs/superpowers/plans/2026-10-07-milestone5-provisioned-launcher.md`;
`docs/firmware-emulator-notes.md` holds the forensic findings and the
stall-by-stall history. This file records what those do not make obvious:
the rulings, the facts traced from `factory.bin` that later tasks build
on, the design decisions taken along the way (with what breaks if each one
is wrong) and what is deferred. Where the spec or plan disagree with this
file or the code, this file and the code win.

## Rulings

- **R-M5-1, format source.** Every committed fact about the identity
  format (field names, types, maximum lengths, the role table, the
  validation rules) is derived from tracing `factory.bin` and cites the
  firmware address it came from. A structure-only summary of the human
  partner's registered-badge dump (local, parsed with their permission)
  may be used only locally, to cross-check the trace and to notice fields
  the trace missed; it is never the source of a committed value or byte.
  This replaces `milestone-4-decisions.md`'s "its format to be read from
  the firmware, never from the physical badge's dump" (approved by the
  human partner, 2026-10-07). If wrong: a committed format fact would
  have no firmware citation to check it against, and personal data could
  leak into the repo through a "format" detail.
- **R-M5-2, permission gate.** Fake identities exist only under `local/`
  (gitignored) until the human partner relays the Hack the North
  organizers' permission to commit them. One gate task (Task 6) then
  commits the fixtures and every test that needs them. Pre-gate tasks
  commit only code, unit tests and rungs that use no identity. If the
  permission has not arrived when the pre-gate work is done, the
  milestone pauses at the gate; what to ship without it is the human
  partner's decision then. If wrong: identity-shaped data the organizers
  did not approve would be published, or (the other way) approved
  fixtures would sit unused.
- **R-M5-3, technical stop conditions.** If the trace finds an offline
  signature, HMAC or other cryptographic check of the identity or the
  `badge_upload/credential` token, or a binding of the identity to the
  chip's MAC address or eFuse, the work stops and asks the human partner.
  No workaround (no forged signature, no patched check, no PC
  interception). Task 1 found none (`R_M5_3` below). If wrong: the
  emulator would end up defeating a security check the firmware's authors
  rely on.
- **R-M5-4, fixtures.** One fixture per role in the traced role table
  (`ROLE_TABLE` below). Each is written by hand from the traced schema,
  never derived from the dump, and is obviously fake: `display_name`
  "Test <Role>", addresses at `example.com` (RFC 2606), a `badge_id`
  beginning `test-`, empty optional `net_*` fields, numeric fields set to
  small obviously synthetic values, every string within the traced
  maximum. Committed at the gate as
  `frontend/public/firmware/test-identities/<role>.json`, the single copy
  both the Rust tests and the frontend read. If wrong: a fixture could be
  mistaken for, or collide with, a real attendee's record.
- **R-M5-5, flash stays synthetic.** `EmulatedFlash` still starts blank
  (partition table + `factory.bin`). The provisioned state only ever comes
  from what the firmware itself writes in response to console input; the
  emulator never writes littlefs or NVS contents itself. If wrong: the
  emulator would carry a second, unverified writer of the firmware's
  on-flash formats, and a format mismatch would make the firmware
  silently reformat.

## Trace facts (Task 1)

Traced by disassembling `factory.bin`'s segments (IROM `0x4200_0020`,
IRAM `0x4038_0000`, DROM `0x3C13_0020`) and single-stepping scratch runs
of the blank-flash boot. ROM names come from Espressif's ESP32-C3 ROM ELF
(`__call_*` jump-table entries); ESP-IDF behavior from v5.5.3 sources.
Tasks 3, 4, 6 and 7 read these instead of re-deriving them.

- **`PROVISIONED_FLAG_WRITER`.** The identity record is a 0x1B4-byte
  struct at `0x3fca_92b0`; the provisioned flag is its byte +2
  (`0x3fca_92b2`, read by `0x4200_ee06`; `0x4200_ee14` returns the
  struct's address). The only code that sets it is `hal_identity`'s
  parser `0x4200_e64e(dst, text)`: it fills a stack copy, sets byte +2 to
  1 at `0x4200_e6fe`..`0x4200_e700` only when every field check passes,
  then copies the whole record to `dst` (`0x4200_e704`..`0x4200_e728`).
  Its one caller that targets `0x3fca_92b0` is the loader `0x4200_e8a6`,
  which zeroes the struct (`0x4200_e8bc`), reads `/littlefs/identity.json`
  (`0x4200_e234`) and parses it; on any failure it zeroes the struct again
  (`0x4200_e9b8`). Either way it bumps an identity generation counter at
  `0x3fcb_cabc` (atomic add, `0x4039_7574`). The loader runs at boot
  (`app_main` → `0x4200_ede6` at `0x4200_a512`) and from `prov apply`
  (`0x4200_be22`). The flag is cleared by `prov erase confirm`
  (`0x4200_ee1e`, memset at `0x4200_ee52`), and the record is replaced
  wholesale by a re-sync routine (`0x4200_e9da`, copy at
  `0x4200_ed64`..`0x4200_ed8e`) that runs only when the flag is already
  set and the new record's `badge_id`, `claim_id`, `attendee_id` and role
  equal the current ones. Input is `identity.json` only: NVS
  `badge_upload/credential` is not read on this path (it is touched only
  by the `badge_token` get/set commands `0x4200_eeb8`/`0x4200_eff2` and
  erased by `0x4200_ee1e`).
- **`VALIDATION`** (parser `0x4200_e64e`, helpers below).
  - File size: `stat` size must be 1..4096 bytes (`size - 1 < 0x1000` at
    `0x4200_e252`..`0x4200_e256`), else `identity.json size %ld out of
    range`. The text must parse as JSON (cJSON, `0x4209_8aac`), else
    `identity.json is not valid JSON`.
  - String fields (`0x4200_e2fe(obj, key, buf, bufsize)`): absent or JSON
    `null` is accepted as empty; a non-string value fails (`%s: not a
    string`); a string of `bufsize` bytes or more fails (`%s: %u bytes,
    max %u`, max = `bufsize - 1`). Maximum bytes per field (the `a3`
    immediates at each call): `badge_id` 63 (`0x4200_e690`),
    `display_name` 39 (`0x4200_e76c`), `account_email` 63 (`0x4200_e784`),
    `claim_id` 23 (`0x4200_e79e`), `net_email` 63 (`0x4200_e7b6`),
    `net_phone` 19 (`0x4200_e7d0`), `net_linkedin` 39 (`0x4200_e7e8`),
    `net_discord` 39 (`0x4200_e802`), `net_instagram` 31 (`0x4200_e81c`),
    `net_x` 31 (`0x4200_e836`). The first failing field stops the string
    checks.
  - Required non-empty: `badge_id` (`badge_id is missing or empty`,
    `0x4200_e6e6`) and `display_name` (`display_name is missing or
    empty`, `0x4200_e6ee`). Every other field is optional.
  - Numeric fields (`0x4200_e3cc(obj, key, default)`): a JSON number is
    converted with ROM `__fixunsdfsi` (unsigned 32-bit, no range check); a
    string is parsed with ROM `strtoul(s, NULL, 10)`; anything else gives
    the default. `version`: default 1, stored as 16 bits (`0x4200_e68c`)
    and never compared anywhere on this path. `attendee_id` and
    `provisioned_unix`: default 0, stored as 32 bits, no range check.
  - Role (`0x4200_e41a`): a string is matched case-insensitively (ROM
    `strcasecmp`) against, per index, the label table (`0x3c15_4548`)
    and the slug table (`0x3c15_451c`); no match logs `unknown role '%s',
    defaulting to hacker` and gives index 0. A number is converted with
    `__fixdfsi` and accepted if 0..10, else 0. Absent or another type
    gives 0 silently.
  - `role_color` (optional, `0x4200_e54a`): defaults to the role's entry
    in the colour table at `0x3c15_44f0`; overridden by a string of six
    hex digits (optional leading `#`) or a 3-element array of numbers
    (each clamped to 0..255).
  - Nothing else is checked: no signature, checksum, MAC or eFuse
    comparison (`R_M5_3`).
- **`ROLE_TABLE`** (slugs at `0x3c15_451c`, labels at `0x3c15_4548`,
  11 entries, index order): `hacker` (Hacker), `organizer` (Organizer),
  `sponsor` (Sponsor), `judge` (Judge), `mentor` (Mentor), `volunteer`
  (Volunteer), `media` (Media), `staff` (Staff), `general` (General),
  `workshop_lead` (Workshop Lead), `visitor` (Visitor). Index 0 (`hacker`)
  is the fallback. `0x4200_e4f4` maps an index to its label
  (`Attendee` if out of range).
- **`CONSOLE_START`.** `hal_console_start()` is `0x4200_db28`, called once
  and unconditionally from the tail of `app_main` (`0x4200_a73c`), after
  the first app is launched (`0x4203_8e0a`; My Badge when unprovisioned,
  `0x4200_a724`) and after `0x4205_70b4` (takes the LVGL lock and arms an
  LVGL timer). It calls `esp_console_new_repl_usb_serial_jtag`
  (`0x4206_1d2c`; driver TX and RX ring buffers 256 bytes each,
  `0x4206_1d96`), registers the commands and `help`, calls
  `esp_console_start_repl` (`0x4206_02ee`) and logs `hal_console:
  console started` (`0x4200_dc70`). In the emulator (blank flash, release
  build, single-stepped) `app_main` is **not** blocked: the launch call
  returns at step 12,289,374, `hal_console_start` is entered at
  12,290,658, the REPL constructor returns `ESP_OK` at 16,124,512 (it
  includes linenoise's 500 ms terminal probe), `esp_console_start_repl`
  returns `ESP_OK` at 16,349,927, `hal_console_start` returns at
  16,552,475 and `main_task` reaches `Returned from app_main()`
  (`0x4212_88e2`) at 16,552,541. The lines are missing because, once the
  driver is installed, stdout goes through `usb_serial_jtag_write_bytes`
  (`0x4200_8774`) into the driver's TX ring buffer (`xRingbufferSend`,
  `0x4038_de9e`), and only the driver's ISR moves ring-buffer bytes into
  the EP1 FIFO; the write enables `SERIAL_IN_EMPTY` in `INT_ENA`
  (`0x4200_87d0`..`0x4200_87de`) but `ETS_USB_SERIAL_JTAG_INTR_SOURCE` is
  not modeled, so the ISR never runs. By step 16.58M, 256 one-byte writes
  had been accepted and 178 refused (ring buffer full); refused writes
  return at once (the VFS gives up blocking after one 50 ms attempt), so
  nothing waits forever. The spec's cause (the missing interrupt source)
  stands, but nothing is blocked; Task 2's interrupt source is expected
  to release the buffered output, so there is no Task D-M5-1 yet.
- **`REPL_LINE_END`.** The REPL runs in linenoise's dumb mode: the probe
  (`0x4206_1432`, called from `esp_console_setup_prompt` `0x4206_01ca`)
  writes `ESC[5n`, polls stdin non-blocking for 500 ms in 10 ms steps
  (`0x1f4` at `0x4206_1488`) and, with no answer, sets dumb mode
  (`0x4206_09dc`) and an uncoloured prompt `badge> `. During those 500 ms
  every received byte before an `ESC` is read and discarded, so input
  must wait for the prompt. The REPL constructor sets the VFS RX line
  ending to CR (`0x4206_1d5e`..`0x4206_1d60`): the VFS read turns `\r`
  into `\n` (compare at `0x4200_8320`, store at `0x4200_8308`) and
  passes `\n` unchanged.
  `linenoiseDumb` (`0x4206_08e4`) ends the line at `\n`
  (`0x4206_0990`..`0x4206_0992`), drops other control bytes below 0x20,
  handles backspace (0x08/0x7F) and echoes printable bytes. So either
  `\r` or `\n` ends a command; `\r\n` ends it and then submits an empty
  line, which the REPL ignores and answers with a fresh prompt. Use a
  single `\n`.
- **`PUT_READ`.** `put` (`0x4200_d0a2`) needs exactly two arguments
  (`usage: put <path> <size>`). The path is used as given if it starts
  with `/`, else prefixed with `/littlefs/` (`0x4200_cb36`); it must be
  shorter than 160 bytes (`path too long`). The size is `strtol(.., 10)`,
  fully numeric and non-negative (`bad size`). It creates parent
  directories (`0x4200_e008`), opens the file `wb`, prints `READY`
  (`0x4200_d18c`) and flushes stdout, then reads the payload with raw
  `read()` on `fileno(stdin)` (`0x4211_a000`, not `fread`), in chunks of
  at most 256 bytes, writing each chunk with `fwrite`, until `<size>`
  bytes have arrived; `EINTR`/`EAGAIN` retry, a zero or failed read ends
  the loop. It then prints `OK <size>` (`0x4200_d29a`), `write error`, or
  `short read: %ld bytes missing`. stdin is blocking, so a short payload
  leaves `put` waiting forever. Bytes sent before `put` starts reading are
  not lost by the REPL: linenoise reads one byte at a time and the VFS
  never reads past the requested count, so they wait in the driver's RX
  ring buffer. That buffer is 256 bytes, and the driver's ISR drops a
  received packet that does not fit (v5.5.3 `usb_serial_jtag.c`,
  `xRingbufferSendFromISR` result unchecked). The payload must therefore
  be sent only after `READY` (or kept within 256 bytes of unread input);
  identity files are larger than 256 bytes. The VFS's CR-to-LF rule also
  applies to the payload, so it must not contain `\r` (compact one-line
  JSON has none). The provisioning path is `/littlefs/identity.json`
  (`prov`'s usage line, `0x4200_c04e`).
- **`APPLY_EFFECTS`.** The `prov` handler is `0x4200_bd56`; `apply`
  (`0x4200_be1c`): takes the LVGL lock with timeout 0, which
  `lvgl_port_lock` (`0x420a_06da`) treats as wait-forever; loads the
  identity (`0x4200_e8a6`); unlocks (`0x420a_0740`). On load failure or
  flag clear it prints `PROV FAIL invalid or missing
  /littlefs/identity.json` (the `%s` is always the path; the failing
  field appears only in `hal_identity`'s `E` log line) and sets the six
  LEDs dim red (`0x4200_bc04(0, ..)`). On success it calls `0x4203_28ca`,
  which writes `onboard_build=-1` to the `system` config
  (`/littlefs/config/system.cfg` via `hal_config`, `0x4200_b8fa`): that
  is `tutorial=reset`. It then prints `PROV OK id=<badge_id> ` followed on
  the same line by `mac=..` (`0x4200_bd04`, the BLE address: the radio's
  cached one, or one derived from the base MAC through `esp_read_mac`,
  `0x4200_f55c`), then `name=<display_name> role=<label>
  attendee=<attendee_id> tutorial=reset`, and flashes the LEDs green three
  times before setting them to the role colour (`0x4200_bc04(1, ..)`).
  It does not switch apps, redraw or restart (`esp_restart` is not on the
  path). The visible change comes from My Badge's per-tick hook
  `0x4201_4254` (vtable slot `0x3c15_48b0`, next to the button handler
  `0x4201_4130`): when the identity generation (`0x4200_f15e`) changes it
  rebuilds its screen (`0x4201_40d8`), and while the badge is provisioned
  and `system.onboard_build` is negative (`0x4203_2894`) it switches
  (`0x4203_8b8e`) to the onboarding app at `0x3fc9_e240` ("Setup",
  `0x4203_27d4`; id `onboarding`, `0x4203_27e8`). `app_main` makes the
  same choice at boot (`0x4200_a748`). `prov erase confirm` is the only
  `prov` path that can print `PROV FAIL display busy`, and with the
  wait-forever lock it cannot in practice.
- **`R_M5_3`: negative.** No offline signature, HMAC, hash or
  MAC/eFuse binding exists on the provisioning path. Checked: the
  static direct-call closure of the loader `0x4200_e8a6` (240 functions:
  stdio/VFS/littlefs, heap, FreeRTOS, cJSON, logging) contains no
  mbedTLS/SHA/HMAC/DS/ECDSA/ed25519 code, no ROM crypto or eFuse routine
  and no MAC read; the parser `0x4200_e64e` only copies and length-checks
  fields. In the `prov apply` handler the only MAC access is the
  `mac=` print (`0x4200_bca6` → `0x4201_0100` → `0x4200_f55c` →
  `esp_read_mac` `0x420f_e046`); the value is printed, never stored in or
  compared with the identity. (That print's fallback, a NimBLE
  advertising restart `0x4200_fbc0` whose PHY init compares stored RF
  calibration with the MAC, `cal_mac` at `0x4210_2b10`, runs only if the
  address is all zero, which `0x4200_f55c` rules out by setting the top
  two bits; it is RF calibration, not identity.) None of `apply`'s
  callees reaches `esp_restart` or ROM `software_reset_cpu`. The `badge_upload/credential` NVS blob is
  only read and written by the `badge_token` command
  (`0x4200_eeb8`/`0x4200_eff2`), which compares its copies of `badge_id`
  and `claim_id` with the identity by `memcmp` and checks the token is 64
  lowercase hex digits; there is no signature check there either, and it
  is not on the provisioning path. The one ed25519 routine in the image
  (`crypto_sign_seed_keypair`, referenced at `0x4205_5126`, tag `solana`)
  is an app's wallet generator, unreachable from these roots.
- **`ACCEL`** (Task D-M5-2; `hal_accel`, register helpers `0x4200_a76e`
  read / `0x4200_a86e` write, init `0x4200_a8da`).
  - Address: the init adds an I2C device with `dev_addr_length` 0 (7-bit),
    `device_address` `0x19` (`0x4200_a8ec`) and `scl_speed_hz` 400,000
    (`0x4200_a8f2`..`0x4200_a8f6`) to the bus from `0x4200_e22a`
    (`i2c_master_bus_add_device`, `0x4212_3ce6`; failure logs `add
    device`). Every access then takes a mutex (`0x4038_f31a`, created at
    `0x4200_a918`).
  - Register access: a read is the driver's transmit-receive
    (`0x4212_4120`: one sub-address byte written, `len` bytes read,
    timeout -1); for `len` > 1 the sub-address gets bit 7 set
    (`ori a5, a5, -0x80` at `0x4200_a784`), the SC7A20H auto-increment
    bit. A write is a two-byte transmit (`0x4212_4090`: register, value).
  - Probe: `WHO_AM_I` (`0x0F`, 1 byte, `0x4200_a98e`) must read `0x11`
    (`0x4200_a99a`); otherwise `unexpected WHO_AM_I 0x%02X` and
    `ESP_ERR_NOT_FOUND` (`0x105`). A transfer error gives the
    `accelerometer setup failed: %s` line (`0x4200_aace`), which is what an
    empty bus produced (`ESP_ERR_INVALID_STATE`). Success logs `SC7A20H
    detected (0x%02X)` (`0x4200_a9ec`).
  - Setup writes, in order: `CTRL_REG1` (`0x20`) = `0x57` (`0x4200_a9fe`:
    ODR 0101 = 100 Hz, `LPen` 0, X/Y/Z enabled), then `CTRL_REG4` (`0x23`)
    = `0x80` (`0x4200_aa92`: `BDU` 1, `BLE` 0, `FS` 00 = +/-2 g, no
    self-test), then `vTaskDelay(2)` (`0x4039_12f2` is `vTaskDelay`, per
    its assert string). Nothing else is written: no `CTRL_REG3`/`CTRL_REG6`
    interrupt routing, no FIFO, no `CTRL_REG0` (`HR`). A separate setter
    (`0x4200_aae8`, used by apps) rewrites `CTRL_REG1` to `0x87` (ODR 1000,
    800 Hz) or back to `0x57` and checks the readback (`ODR readback
    0x%02X, expected 0x%02X`, `ESP_ERR_INVALID_RESPONSE`, `0x108`).
  - Sampling (`0x4200_a7aa`): polls `DRDY_STATUS_REG` (`0x27`) bit 3
    (`ZYXDA`, `andi 0x8` at `0x4200_a7d4`) up to 100 times with
    `vTaskDelay(0)` between tries (else `ESP_ERR_TIMEOUT`, `0x107`), then
    reads six bytes from `0x28 | 0x80`. Each axis is
    `(int16_t)(H << 8 | L) >> 4` (`0x4200_a800`..`0x4200_a810`: 12-bit
    left-justified, little-endian), converted to `float` with ROM
    `__floatsisf`. The count is used as mg directly (the onboarding page
    prints it with `%d mg`), which matches the datasheet's 1 mg/digit at
    +/-2 g. The SC7A20H's interrupt pins are not used: no GPIO interrupt,
    only status polling.
  - Consumers: a cache task `accel_cache` (`0x4200_ac76`, created at
    `0x4200_aa56`) reads a sample every `vTaskDelay(2)` into
    `0x3fca_9294` (the getter `0x4200_aca0` copies it out; its callers
    `0x4205_9252`/`0x4205_9a00`/`0x4205_9b72` are in the Lua
    `badge.sensor.accel` bindings); other code (the onboarding detector,
    several apps) calls the direct reader `0x4200_ac08`.
  - Shake ("Shake it!", onboarding page 6). The page's tick `0x4203_2c2a`
    calls the detector `0x4205_7292`, then shows `G-force %d mg` (the
    magnitude) and `Peak %d mg` (its running maximum). The detector reads a
    fresh sample (`0x4200_ac08` into `0x3fca_bffc`, at `0x4205_71d4`),
    computes the magnitude `m = sqrtf(x^2 + y^2 + z^2)` (`0x4210_fdbe`),
    seeds a baseline with the first `m` and then updates it as an EMA,
    `b = 0.92 b + 0.08 m` (constants at `0x3c15_4d38` and `0x3c15_4aac`),
    and reports a shake when `|m - b| > 1200.0` (`0x3c15_4cd4`, compared
    with ROM `__gtsf2` after `__subsf3` and a sign-bit mask); a detection
    starts a cooldown of 8 calls (`0x3fcb_ccd0`). Since `b` already
    includes `m`, the trigger is `|m - b_old| > 1200 / 0.92`, about
    1304 mg away from the resting magnitude. On the first detection the
    page sets its done flag (`+0x11`) and its footer becomes `A / START:
    next   B: back` (`0x4203_2cc2`); A or START then advances. At rest
    (1000 mg), a shake therefore needs a magnitude above ~2304 mg, which
    at +/-2 g full scale (2047 per axis) takes at least two axes near full
    scale.

## USB-Serial-JTAG receive (Task 2)

- **Pacing (amended by R-T4-1 / R-D1-1, Task D-M5-1):** the host delivers
  one 64-byte OUT packet when the FIFO is empty and `PACKET_STEPS` = 8,210
  CPU steps have passed since the last. A packet occupies ~51.3 us of
  full-speed bus (616 bits at 12 Mbit/s); the ESP32-C3 runs at 160 MHz
  (ESP-IDF v5.5.3 `components/esp_system/port/soc/esp32c3/Kconfig.cpu`:
  `default ESP_DEFAULT_CPU_FREQ_MHZ_160`; `.../esp32c3/clk.c` applies
  `CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ`), and the emulator has no cycle model
  (one instruction per step), so 51.3 us x 160 = 8,208, taken as 8,210. The
  earlier 821 counted SYSTIMER ticks (16 MHz) and let packets arrive ~10x
  too fast relative to the CPU. `PACKET_TICKS = PACKET_STEPS *
  TICKS_PER_STEP` keeps the idle fast-forward (which works in ticks)
  consistent: with `TICKS_PER_STEP` = 1 it skips exactly the steps it
  replaces. If wrong: a future cycle model rescales this one constant.
- **Host chunk pacing (R-D1-1):** 8,210 is the faithful bus rate, not
  enough on its own: the bytes are dropped inside the firmware's driver
  (the ISR pushes into a full 256-byte ring) because the `put` task spends
  ~20-24k steps per 64-byte packet (measured: a 600-byte `put` fails at
  20,000 steps/packet and passes at 24,000; 317 bytes passes at 8,210, 350
  fails). A fast real host can overflow the ring the same way, so hosts
  pace: `tests/common/provision.rs::put_file` sends the payload in 64-byte
  chunks `PUT_CHUNK_GAP_STEPS` = 48,000 steps apart (2x the threshold).
  This is host behavior, not a bus property; the bus constant was not
  inflated. Context for where the cost goes (hot PCs while one packet is
  drained, 40,000 steps): ~20% in one tight IRAM loop at
  `0x4038_f930..0x4038_f9f0`, the rest spread over IRAM `0x4038_cf00..e1ff`
  and flash `0x420f_29xx`, `0x4200_83xx..87xx`; not identified further (a
  per-byte VFS read through the ring buffer is the suspect). If wrong: if
  the cost is an emulator gap, a later task can lower the gap.
- **Host queue cap:** 1 MiB (`HOST_QUEUE_CAPACITY`); excess is refused and
  reported by `serial_input`'s return value. If wrong: a client sending
  more is refused where a real host would block.
- `INT_RAW` R/WTC semantics and the forced SOF/SERIAL_IN_EMPTY bits are
  unchanged; `INT_ST`/`INT_ENA`/`INT_CLR` are real, and source 26 now
  exists (resolves M4's "No USB-Serial-JTAG interrupt source yet"). No USB
  bus reset, JTAG or EP2.
- **R-T2-1 (press hold):** `first_run_screen_responds_to_start` now holds
  START for `PRESS_HOLD_STEPS` = 1,600,000 steps (100 ms of emulated time),
  not 100,000. Why: with the console draining, the firmware's button
  polling falls outside a 6.25 ms window; the old hold registered, likely
  only because the undrained console left the firmware idle (unmeasured). The expected
  `SELF_TEST_BUTTONS_HASH` is unchanged. If wrong: the test no longer
  models a realistic press, and a slow poll could still miss it.
  `SELF_TEST_MAX_STEPS` followed: the self-test frame is first sampled at
  22,850,000 with the 1.6M hold, so the cap is 22.85M + 8M + 1M rounded up
  to 250,000 = 32,000,000 (was 31,500,000 from the 100k-hold figure).
- **R-T3-1 (ROM calls on the console path):** a ROM call that faults gets a
  real stub in the task that hits it (Global Constraints). Task 3 added
  `strlcpy`, `strtol` and `strrchr` (notes, "Milestone 5 Task 3"). If wrong:
  `strtol` ignoring `errno` would matter to a caller that checks it.
- **`put` payload timing (replaces `PUT_SETTLE_STEPS`):** the helper waits
  for `READY` in the console output before sending the payload (R-T1-3);
  there is no fixed sleep. `READY` is matched as a substring, so it works
  whether or not the firmware terminates the line (the capture shows
  `READY\r\n`). The payload (up to ~320 bytes) exceeds the driver's
  256-byte RX ring buffer, whose ISR drops what does not fit; `put` reads
  with raw `read()` only after printing `READY`, and the emulated host
  paces 64-byte packets, so the ring is drained as it fills. The deadline
  is the only step bound. If wrong: a firmware that prints `READY` before
  it can read would lose payload bytes.

## SC7A20H accelerometer on I2C0 (Task D-M5-2)

Ruling R-T4-3 (binding): model the SC7A20H as an I2C device behind
I2C0's address phase, plus a host motion input, so a rung (and later the
browser) can shake the badge; onboarding is not bypassed (R-T4-2). This
supersedes in part `milestone-4-decisions.md` "I2C0 with no device
attached (Task D-M4-1)", whose "If wrong" anticipated it; the rest of that
section (zero-latency transactions, the STOP after a NACK, what is not
modeled) stands. Sources: the Silan *SC7A20H 说明书* v0.7 and ESP-IDF
v5.5.3 `i2c_master.c`, cited in `peripherals/sc7a20h.rs` and
`peripherals/i2c.rs`.

- **Device model.** `peripherals/sc7a20h.rs`, a concrete field of the I2C0
  controller (`i2c0.accel`; no `dyn`). It ACKs address `0x19` only (SDO
  floating/high, the address the firmware uses); every other address still
  NACKs. A write transfer's first byte is the sub-address (bit 7 =
  auto-increment), later bytes write registers; reads return registers,
  auto-incrementing when bit 7 was set; a repeated START or STOP ends the
  transfer. Only the `rw` registers of the datasheet's register list store
  writes. Transactions stay zero-latency (no clock stretching). If wrong:
  firmware that relies on a register the model computes differently (FIFO,
  interrupts, high-pass filter, self-test, `BDU` latching, the `0x61..0x66`
  copies, the click-coefficient reset values) reads storage or 0; none is
  reached by this firmware's `hal_accel`.
- **Output encoding.** `OUT_*` = host mg / sensitivity (1, 2, 4, 8 mg/digit
  for `FS` 00..11, datasheet section 6), clamped to the 12-bit range
  -2048..2047, left-justified by 4; `BLE` swaps the bytes. The firmware
  confirms the 12-bit left-justified format (`>> 4`) and uses the count as
  mg. The datasheet's own conversion table says 1024 counts = 1.0 g (not
  1000); the model follows the typical sensitivity, so a host value in mg
  reads back as that many mg on the badge's screen. If wrong (the part is
  really 1024 counts/g): every reading is 2.4% low, well inside the
  datasheet's sensitivity tolerance, and the shake threshold still trips.
- **Data ready, no time base.** `DRDY_STATUS_REG` reads `0x0F` (`ZYXDA` and
  the per-axis bits, no overruns) whenever `ODR` != 0, and 0 in
  power-down. The real part sets `ZYXDA` once per ODR period (10 ms at the
  configured 100 Hz); every reader in this firmware waits at least
  `vTaskDelay(2)` between samples or polls through the page's redraw tick,
  so a sample is always due. If wrong (a reader polls faster than the ODR
  and counts fresh samples): it sees a new sample on every read, where the
  real part would make it wait.
- **Stationary default: (0, 0, +1000) mg**, the badge lying face up, display
  toward the viewer (+Z out of the screen). That is what the onboarding
  page shows at rest (`G-force 1000 mg`). If wrong (the badge's Z axis
  points the other way, or the sensor is mounted rotated): the rest reading
  is (0, 0, -1000) or another axis; magnitudes, and so the shake detector
  and the `G-force` line, are unchanged, but an app that reads orientation
  (tilt, which side is up) would see the badge flipped.
- **Host input.** `FirmwareRuntime::set_acceleration(x_mg, y_mg, z_mg)`
  takes any `i32`, clamps to +/-16 g (the widest full scale) and never
  panics; the output registers clamp again to the configured full scale.
  `FirmwareRuntime::acceleration()` reads it back. The value survives
  `reset()`, like held buttons (the badge is still held the same way).
- **Shake pattern** (tests and the local walk): alternate (+2000, +2000,
  +1000) and (-2000, -2000, +1000) mg, about 1,000,000 steps (62.5 ms) per
  half-period, for six half-periods, then return to rest. Each half has a
  magnitude of 3000 mg, 2000 mg from the 1000 mg baseline, well above the
  traced ~1304 mg trigger (`ACCEL`). One axis alone cannot shake the badge
  at +/-2 g full scale (2047 - 1000 < 1304), so the pattern uses two. If
  wrong (the trigger was mis-traced higher): the page stays on "Shake to
  continue"; the walk records the step at which the footer changes.

## Permission gate, identity rungs and the WASM twin (Task 6)

- **Gates.** On 2026-10-08 the human partner said "commit the
  identities": the R-M5-2 go-ahead. The 11 fixtures went from `local/`
  to `frontend/public/firmware/test-identities/` unchanged (the R-T1-1
  versions). On the same date the human partner confirmed the frames
  against the physical badge (Task 5), for role hacker only: onboarding
  pages, the registered screen, the launcher and the launcher after DOWN.
  No other role's frames were compared on hardware; the rung docs say
  which frames were compared.
- **R-T6-1 (amends the plan's Task 6 code).** The plan assumed `prov
  apply` lands on My Badge's registered screen. It does not: provisioning
  starts the onboarding app ("Setup", 11 pages, a shake on page 6), and
  only finishing Setup (START on page 11) shows the registered screen;
  HOME from there opens the launcher. The committed rungs are Task 4's
  measured walk (`local/m5-held-rungs.rs`), fitted to the plan's
  structure. Presses are held `PRESS_HOLD_STEPS` (1.6M); the shake is
  `set_acceleration` (D-M5-2's pattern). Each response cap is relative to
  the press (the walk is about 340M steps long), not a total-step budget.
  If wrong: a firmware change to onboarding moves every hash after page 1.
- **Rungs.** `provisions_<role>_through_the_console` (11, one per
  `ROLE_TABLE` entry) pin onboarding page 1 and the registered screen for
  every role (both show the name, so all differ); only hacker pins the
  pages in between. `onboarding_shake_page_advances_after_a_shake`
  (D-M5-2's held rung), `boots_to_launcher` (the finish line),
  `launcher_is_role_independent` (workshop_lead; the launcher hash was
  equal for 11 of 11 roles in Task 4, so one sample stands in for the
  rest) and `launcher_responds_to_navigation` (DOWN, slot 4). Page 8
  ("Bump to connect") is animated (about 14 frames, none held 8M steps),
  so it is not pinned: the walk runs 9M steps there and presses A. If
  wrong (a role's launcher differs after a firmware change): only
  workshop_lead and hacker would catch it.
- **Constants' origins.** `PROVISION_DEADLINE` 38M: the slowest role's
  `PROV OK` (30.19M) + 25%. `LAUNCHER_RESPONSE_STEPS` 33M: HOME's release
  to a confirmed stable launcher (26.3M) + 25%.
  `ONBOARDING_RESPONSE_MARGIN` 16M: page 7's ~9M transition plus margin.
  `ANIMATED_PAGE_STEPS` 9M: the walk's settle. No constant is 0.
- **Suite time.** With these rungs `cargo test -p emulator-core --release
  --test boot_progress` takes about 36 s wall (about 485 CPU-s; 54
  tests). Each identity rung is 280M to 345M steps. Task 8's checkpoint
  fixture is the planned fix; most of the time is in the walk after
  provisioning, which a first-run checkpoint does not save.
- **Fixture loader.** `common::provision::identity_fixture(role)` reads
  the committed file; `$BADGE_TEST_IDENTITIES` (relative to the repo root)
  overrides the directory, for trying a local variant.
- **Frontend provisioner** (`frontend/src/runtime/provisioner.ts`). A
  polled state machine mirroring `provision.rs`: wait for `badge> `, type
  `put /littlefs/identity.json <size>\n`, wait for `READY`, send 64-byte
  chunks at least `PUT_CHUNK_GAP_STEPS` (48,000) steps apart by
  `handle.totalSteps()`, at most one per poll, wait for `OK <size>` and
  the next prompt, type `prov apply\n`, and report the complete `PROV OK`
  or `PROV FAIL` line (or a `put` error line). It replaces the plan's
  fixed `PUT_SETTLE_STEPS` (R-T1-3, R-D1-1). `put` blocks until every
  byte arrives, so a slower poller (the browser polls once per 500,000-step
  frame) only takes longer. It does not walk onboarding; that is the
  user's job in the browser. If wrong (`put` drains slower than one
  packet per 48,000 steps): the driver drops payload bytes, `put` blocks,
  and the provisioner stays in `typing`; it has no deadline of its own (the
  caller's, as in the twin).
- **WASM surface.** `serialInput`, `serialPending` and `setAcceleration`
  (one-line passthroughs) in `emulator-wasm`; `bridge.ts`'s handle gains
  those plus `consoleOutput`.
- **WASM twin** (`frontend/test/cpu-wasm.test.ts`): boots hacker,
  provisions it with the frontend provisioner (so the provisioner is
  proven against the real firmware), walks the same onboarding sequence
  (presses plus the shake through `setAcceleration`) and reaches
  `LAUNCHER_HASH`, every intermediate hash checked; constants copied from
  `boot_progress.rs`. About 35 s under `bun test`. The test's FNV-1a now
  computes in 32-bit halves; BigInt per byte was too slow for ~1,400
  samples (same hashes, checked by the existing splash and first-run
  twins).


## Milestone 6 backlog

- **Accelerometer in the browser (M6).** Task 6 added the one-line
  `setAcceleration` passthrough (`emulator-wasm`, `bridge.ts`); the
  frontend still needs a motion input (Task 7 or M6: a shake button, or
  `DeviceMotionEvent` on a phone) so onboarding page 6 can be passed in
  the browser.

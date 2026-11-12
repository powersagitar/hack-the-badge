# Milestone 12: decisions and the Milestone 13 backlog

Milestone 12 takes Sync from "Badge not ready to sync" to "No sync station
in range", and carries three ride-alongs: three signaling faults,
type-checking the test files, and a trace of Share's radio transfer.
Spec: `docs/superpowers/specs/2026-11-10-milestone12-sync-no-station-design.md`;
plan: `docs/superpowers/plans/2026-11-11-milestone12-sync-no-station.md`.

`BASE` (the branch point for this milestone's work, `milestone-12`):
`534a3145cb7635378f8461a76e59e5b6211a1e25` (`534a314`, "docs: Milestone
12 plan").

## Why

Sync is one of the launcher's 16 apps, and today it ends at "Badge not
ready to sync": the fake identities have no upload credential
(Milestone 10's R-T6-5). The human partner wants Sync to go as far as the
firmware goes before it meets something the browser cannot supply, and
chose to stop at "No sync station in range / Move closer to the sync
desk": no sync station exists in the browser, and none is emulated (the
human partner's call, 2026-11-10). The organizers' OK covers a fake
upload credential, used with the committed fake identities.

## Measurements

### Copied from the spec ("Measured (2026-11-10)")

Scratch probes at the end of `boot_progress.rs`, with a scratch
`crc32_le` HLE effect (bitwise, the semantics in `crate::rom` entry 28),
run release, then reverted. Token used: the value later named
`FAKE_UPLOAD_TOKEN`.

1. **Without the stub, setting the token faults.** From `sync_open()`
   (the open Sync, no credential): `badge_token set` → `BADGE_TOKEN
   READY`; after the 64 bytes, an instruction access fault at
   `0x40000628`. NVS checksums every entry it writes, so the stub is
   needed for the credential itself, before Sync's own CRC. Nothing
   reached `0x40000628` before this: a fetch there faults, and no
   existing rung faults, so adding the stub cannot move a Milestone 11
   pin.
2. **With the stub, from `sync_open()`:** `BADGE_TOKEN OK`; A → first
   frame change at +5.9M steps ("Packing your badge..."); console
   `sync: snapshot 1026 bytes, crc ec2f90df, 37ms, complete=1`, then
   NimBLE's `extended advertise` and `extended discovery`; "Asking the
   station..." pulses for about 20.0 s of emulated time (NimBLE stops
   advertising and discovery at the firmware's `I (21141)`); the final
   frame `0xa465_e4c0_e16e_ede6` lands at +20.16 s, about +415M steps.
   `uitree`: "No sync station in range", "Move closer to the sync desk",
   "A: try again   -   HOME: home". No fault.
3. **With the token typed right after `PROV OK`** (from the first-run
   screen, `HACKER_MAC` set, then `provision`): `BADGE_TOKEN READY` 150k
   steps after `PROV OK`'s prompt, `BADGE_TOKEN OK` at 300k. The walk
   helpers (`walk_to_shake_page_from`, `walk_to_registered_from`,
   `walk_to_launcher_from`), which assert every hacker frame, all pass:
   the launcher is reached at 342,894,540 steps and 94,521,622 µs
   (`RADIO_LAUNCHER`: 342,600,681 and 94,521,615). Sync opens to
   `SYNC_HOME_HASH`, and A ends on the same `0xa465_e4c0_e16e_ede6`
   (stable from +415,049,485 steps; +21.1 s emulated when the probe
   stopped).

Open question carried from the spec, for the Share trace: the public
source's open timeout is 3 attempts × 5 s (15 s), but v392 took about
20 s.

### ROM `crc32_le` (Task 1)

Fetched from ESP-IDF v5.5.3 into the session scratchpad (not committed):

- `components/esp_rom/esp32c3/ld/esp32c3.rom.ld:415`:
  `crc32_le = 0x40000628;` (`:416`..`:421`: `crc32_be`, `crc16_le`,
  `crc16_be`, `crc8_le`, `crc8_be`, `esp_crc8`, all unstubbed).
- `components/esp_rom/esp32c3/ld/esp32c3.rom.api.ld:4`:
  `PROVIDE ( esp_rom_crc32_le = crc32_le );`, the name NVS calls.
- `components/esp_rom/include/esp_rom_crc.h`: the CRC32 polynomial
  `0x04c11db7` (`:29`); a `~` added "at the beginning and the end of the
  functions" (`:32`); the seed is "result of last calculation or 0 for
  the first time" (`:65`); the declaration (`:70`).

The stub is `crate::rom::CRC32_LE` (`0x4000_0628`) with
`RomStubEffect::Crc32Le`: a bitwise reflected CRC-32 (`0xEDB8_8320`),
starting from `!a0` and returning the inverse, `len` capped at
`MAX_STUB_MEMORY_BYTES`. `esp32c3_rom_stubs().len()` goes from 127 to
128.

### Sync with the fake credential (Task 2)

Measured in release with the rungs exactly as committed
(`launcher_with_a_credential_keeps_every_walk_frame`,
`sync_with_a_credential_finds_no_station`), plus one scratch probe for the
packing frame's timing and the frames (deleted before the commit):

- `credential_launcher()` (hacker, `HACKER_MAC`, `provision`, then
  `set_upload_token(FAKE_UPLOAD_TOKEN)`, then the walk with every hacker
  frame asserted): 342,894,540 steps and 94,521,622 µs, the spec's probe
  values exactly (`CREDENTIAL_LAUNCHER_STEPS`, `CREDENTIAL_LAUNCHER_US`).
  `RADIO_LAUNCHER` is 342,600,681 and 94,521,615; the token adds 293,859
  steps and 7 µs. Every walk frame is `launcher_with_mac`'s.
- Sync opens from it to `SYNC_HOME_HASH` (405,422,627 steps), and
  `badge_token get` there reports the credential (it survived Sync's
  reboot into itself).
- A's press to the console line
  `sync: snapshot 1026 bytes, crc ec2f90df, 37ms, complete=1`:
  4,394,418 steps (1 ms slices; 82 ms into the 100 ms hold; the release is
  at +4,914,783). `SYNC_PACKING_STEPS` = 6,000,000 (+25 %, rounded up).
- "Packing your badge..." (`0xb882_9410_38c1_75a2`) is finished at
  +1,188,996 and holds until about 500,000 steps after the release, about
  4.2M steps: under `AFTER_PRESS_STABLE_FOR` (8M). It is pinned by the
  console line (R-T2-1).
- "Asking the station..." pulses between `0xa8ea_6aac_fa65_1423` and
  `0x6c55_722c_2b58_aa1b`, each held 6.0M to 7.0M steps.
  `SYNC_NO_STATION_STABLE_FOR` = 30,000,000.
- `SYNC_NO_STATION_HASH` `0xa465_e4c0_e16e_ede6` (the spec's hash), first
  sampled 414,064,783 steps after A's press, with the rung's mid-wait
  `uitree`. `SYNC_NO_STATION_STEPS` = (414,064,783 + 30,000,000) × 1.25,
  rounded up: 556,000,000.
- Frames for the human partner's eye (Gate 3), gitignored:
  `local/m12-frames/sync-packing.png`, `sync-asking.png`,
  `sync-no-station.png` (in the milestone-12 worktree's `local/`).
- Suites at Task 2: release `boot_progress` 90 pass, 1 ignored, 18.6 s
  (was 88 and about 15 s); `cargo test --workspace` 757 pass, 1 ignored,
  29.4 s; `bun test` 319 pass, 12.4 s; clippy and `bun run build` 0
  warnings.

### The browser provisioner's token (Task 3)

`createProvisioner` types `badge_token set` and `FAKE_UPLOAD_TOKEN` after
`PROV OK` and its prompt by default (`uploadToken: null` skips it; the
WASM redraw benchmark does). The WASM twin (`test/cpu-wasm.test.ts`)
takes the default, and every frame pin it holds is unchanged. Suites at
Task 3: release `boot_progress` 90 pass, 1 ignored, 18.5 s;
`cargo test --workspace` 757 pass, 1 ignored, about 28 s wall (its
debug `boot_progress` 27.0 s); `bun test` 330 pass, 12.4 s; clippy
and `bun run build` 0 warnings.

### Type-checking the test files (Task 4)

`frontend/tsconfig.json`'s `include` widened from
`["src", "api", "test/cpu-wasm.test.ts"]` to `["src", "api", "test"]`.
`bun run typecheck` then reported 58 errors (at `b0040ec`, after Task 3):
`firmware-worker.test.ts` 29, `firmware-runtime.test.ts` 19,
`shell.test.ts` 3, and one each in `manifest.test.ts`,
`provisioner.test.ts`, `radio-lock.test.ts`, `redraw-bench.test.ts`,
`signal-client.test.ts`, `signal-handler.test.ts` and `vite.config.ts`
(which `signal-handler.test.ts` imports). 50 were `.at()` (ES2022) and
one `.replaceAll()` (ES2021), all cleared by the `lib` bump (R-T4-1).
The other seven were fixed in place, with `strict` unchanged and no
`any`, `@ts-nocheck` or `@ts-ignore` (R-T4-2).

`bun run build`'s output before and after the `lib` bump (and after the
whole task): every file in `frontend/dist/` has the same byte size and
the same SHA-1 (`index.html` 984, `assets/bridge-Ctgh-ak7.js` 5,919,
`assets/firmware-worker-BYvDG9ar.js` 8,946,
`assets/emulator_wasm_bg-DjWHtz-t.wasm` 142,270,
`assets/index-XRjmGPO9.css` 2,962, `assets/index-BNNDwprf.js` 261,491;
the hashed names are unchanged too). Suites at Task 4: release
`boot_progress` 90 pass, 1 ignored, 18.9 s; `cargo test --workspace` 757
pass, 1 ignored, 29.0 s wall; `bun test` 330 pass, 12.6 s;
`bun run typecheck` 0 errors; clippy and `bun run build` 0 warnings.

### The three signaling fixes (Task 5)

- **A fetch timeout.** Every request the browser makes to `/api/signal`
  aborts after `SIGNAL_FETCH_TIMEOUT_MS` (10 s; `SignalDeps.fetchTimeoutMs`
  injects a smaller one in tests) and maps to "Could not reach the
  signaling server; check the connection.". So a joiner, which has no
  cancel, ends a hung request within its 120 s budget plus one
  timeout. The host's answer polls also abort on its cancel (R-T5-1).
- **No Upstash retries.** `REDIS_OPTIONS.retry` is `{ retries: 0 }`: one
  REST request per command, so a lost `EVALSHA` reply is never re-sent
  and counted twice. A store error stays the bare 500 (R-T5-2).
- **`close` when the host gives up.** `connected`'s failure (no joiner
  in 120 s, a 404/429/5xx poll, an unusable answer, no direct path, a
  fetch timeout) now sends `close` with the room's token, as `cancel()`
  already did. A `closeSent` flag shared by the two sends it exactly once,
  in either order. A host that pairs sends none. `client_stops_on_500`
  no longer counts that `close` as a request after the failure (R-T5-3).

Suites at Task 5: release `boot_progress` 90 pass, 1 ignored, 18.8 s;
`cargo test --workspace` 757 pass, 1 ignored, 28.3 s wall; `bun test` 339
pass, 13.0 s; `bun run typecheck` 0 errors; clippy and `bun run build` 0
warnings.

## Traces

### Re-provisioning a registered badge (Task 3)

A scratch `#[ignore]` probe at the end of `boot_progress.rs` (release,
deleted before the commit), from a clone of `credential_launcher()`
(hacker, `HACKER_MAC`, `FAKE_UPLOAD_TOKEN` set, on the launcher at
342,894,540 steps). It ran `put_file` with organizer's fixture, typed
`prov apply`, and stepped in 50,000-step slices, watching
`soft_reset_count()` and the console after each slice as the browser's
runtime does when it records `bootConsoleStart`.

1. **`prov apply` on a registered badge does not reboot.**
   `soft_reset_count()` stays 0 through `prov apply` and for 20M steps
   after it; nothing moves the boot console start. The console after the
   command, in order: the echo `prov apply`, `hal_identity`'s
   `identity: Test Organizer (Organizer) id=test-fake-badge-0002` log
   line, `PROV OK id=test-fake-badge-0002 mac=…`,
   `name=Test Organizer role=Organizer attendee=2 tutorial=reset`, then
   `badge> `. `PROV OK` printed about 3.5M steps after the launcher
   checkpoint (put included), and its prompt 1.35M steps after
   `PROV OK`, 117 console bytes after the `PROV OK` line's start.
2. **The old credential does not carry over.** `badge_token get` right
   after organizer's `PROV OK` prints `BADGE_TOKEN MISSING` (and the
   REPL's `Command returned non-zero error code: 0x1`), as the spec
   expected: the stored credential belonged to hacker's `badge_id`.
3. **`set_upload_token` then succeeds:** `badge_token set`,
   `BADGE_TOKEN READY`, `BADGE_TOKEN OK`, a prompt; `badge_token get`
   then prints the token. No fault.
4. **Twice:** the same run after first re-provisioning hacker from the
   launcher (`PROV OK id=test-fake-badge-0001`, again no reboot) and
   setting the token again (`BADGE_TOKEN OK`) gives the same four
   findings for the organizer that follows.

So on v392 every provision, first-run or re-provision, prints `PROV OK`
and its prompt in the boot that took `prov apply`. The stale prompts
already in the console when the token phase starts are the launcher's
idle one and `put`'s, and the old `BADGE_TOKEN READY`/`OK` lines of an
earlier provision; the provisioner's token phase looks only past the
`PROV OK` line (and past `bootConsoleStart()`), and its `READY`/`OK`
matching only past its own `badge_token set`.

### Share's radio transfer (Task 6)

Sources. The public source (`hackthenorth/badge-firmware`, branch
`main`, later than v392), fetched into the session scratchpad and read,
never built: `main/apps/share_xfer.{h,cpp}`, `main/apps/app_share.cpp`,
`main/apps/app_share_proto.h`, `main/hal/hal_radio.cpp`. v392's
disassembly (`llvm-objdump` over `firmware.bin`'s segments, the
Milestone 10 tools) is the authority; the source only names things.

**Transport: advertisements only.** No connection, no GATT, no ACL.

- The source's protocol ("APP1") rides hal_radio's manufacturer-data
  channel (company `0xFFFF`), like Connect's `BMP2` cards. v392 has the
  same app: "Share Custom Apps" (`0x3c13c1d8`), " Send an app",
  " Receive an app", "No custom apps yet" (`0x3c13c6a0`), "WAIT FOR
  ACCEPTANCE" (`0x3c13c220`), the
  `share_xfer` log tag and its "offering", "peer accepted" and
  "accepted" lines.
- v392's NimBLE logs only three GAP procedures: "stop extended
  advertising", "extended advertise" and "extended discovery"
  (`0x3c145da8`, `0x3c145dec`, `0x3c145e34`). In NimBLE's source (as
  known, not fetched) the connect procedures log "GAP procedure
  initiated: connect" or "extended connect", and the GATT client logs
  "GATT procedure initiated"; no such string is in the image, so those
  functions are not linked.
- Share's engine (`0x42055c00`..`0x42058400`) reaches the radio only
  through hal_radio: `hal_radio_send` (`0x42010c54`), pause
  (`0x42010cc2`, its "ext_adv_stop" warning at `0x42010d1a`), resume
  (`0x42010d36`, `0x42010da2`), the advertising-interval retune
  (`0x42010dbe`, "retune" at `0x42010e8a`), the scan-duty setter
  (`0x42010ec8`), enable (`0x420109c6`, "BLE radio ready" at
  `0x42010b0a`) and two more hal_radio entries (`0x42010f76`,
  `0x42011330`). It makes no direct NimBLE call.
- The probe's HCI trace (below) agrees: every command from both badges is
  one the model already answers, each with Success, and none is LE
  Create Connection (`0x200D`), LE Extended Create Connection (`0x2043`)
  or an ACL packet.

**Frames** (v392, every one checked against the source's
`app_share_proto.h`):

- `set_adv_payload` (`0x42010712`) refuses a payload over 225 bytes
  (`li a5, 0xe1; bltu a5, a1` at `0x4201072e`) and wraps it in one AD
  structure, `len ff ff ff` (`li a5, -0x1` and three `sb` from
  `0x42010742`): at most 229 bytes of advertising data.
- Share's send wrapper (`0x42056356`) takes 1 to 225 bytes
  (`addi a4, a1, -0x1; li a5, 0xe0; bgeu`) and calls `hal_radio_send`.
- Its header is 12 bytes: `APP1` (`li a5, 0x41`, `0x50`, `0x50`,
  `0x31`), version 1, the type, `req_id`, `seq`, `total`, `status`, a
  zero byte. The magic check is `0x42055cda`. The types: OFFER 1
  (`0x42056380`), ACCEPT 2 (`0x4205641a`), DATA 3 (`0x42056fea`), STAT 4
  (`0x420564a4`), DONE 5 (`0x42056558`). Bodies are capped at 213 (`li
  a1, 0xd5` at `0x420563fc`).
- Measured in the probe (the AD length is the payload + 4):

  | Frame | Payload | AD | LE Set Extended Advertising Data parameters |
  |---|---|---|---|
  | OFFER (smoke_test, 2,445 B, 12 frames) | 47 | 51 | 55 |
  | ACCEPT ("Test Organizer") | 27 | 31 | 35 |
  | DATA, full | 225 | 229 | 233 |
  | DATA, last (102 B) | 114 | 118 | 122 |
  | STAT | 46 | 50 | 54 |
  | DONE | 13 | 17 | 21 |

  Against the model: the largest command is 233 parameter bytes (229
  bytes of data), under the 251 data bytes one LE Set Extended
  Advertising Data command may carry (Bluetooth Core Specification v5.x,
  Vol 4, Part E, §7.8.54) and the 255 parameter
  bytes of any command. The largest AD is 229, exactly
  `MAX_ADV_DATA_LEN`, and its report is 26 + 229 = 255 parameter bytes,
  exactly one event's maximum.
  So a full DATA frame fits both directions with no byte to spare, and
  needs no fragmentation.

**The two-badge probe.** A scratch `#[ignore]` test at the end of
`boot_progress.rs` (release; deleted, never committed), with a temporary
env-gated `eprintln!` of each command in `Radio::on_hook` (reverted;
ruling R-T6-3):

1. Hacker from `radio_launcher()` (`HACKER_MAC`): `put` of a
   `manifest.cfg` and the committed `smoke-test/main.lua` to
   `/littlefs/apps/smoke_test/`, no `reload`. Share then reboots into
   itself and the new boot loads the app (`script_app: loaded lua app:
   Smoke Test (smoke_test)`). The committed `smoke-test/manifest.cfg` is
   refused by v392's loader (ruling R-T6-2).
2. Organizer from `organizer_launcher()` (`ORGANIZER_MAC`). Both open
   Share (cell 4) to `0xfc51_aedc_c0ab_617d`, the pinned home.
3. Then lockstep in 1 ms slices (`lockstep_to`, `LINKED_PEER_RSSI`),
   presses held 100 ms inside the lockstep. Organizer: DOWN, A
   ("WAITING...", "Ask them to send,\nthen accept here"). Hacker: A, A, A ("Send an
   app" list with " Smoke Test (hack-the-badge)", then "Send this app?",
   then "Offering app" / "WAIT FOR ACCEPTANCE").
4. The organizer's next `uitree` (250 ms on) already shows "Incoming
   app" / "Smoke Test" / "2 KB  -  v0.1.0" / "A: accept   B: decline".
   A on the organizer starts it: "SENDING" on the hacker, "Receiving" /
   "Hold still   -   B: cancel" on the organizer.
5. The consoles (each badge's own boot clock): hacker `offering
   smoke_test: bytes=2445 frames=12 req=4903` (I 6781), `peer accepted
   req=4903; starting sweep` (I 7521); organizer `accepted smoke_test:
   bytes=2445 frames=12 req=4903` (I 8071), `first DATA seq=0/12
   bytes=213` (I 8131), `script_app: loaded lua app: Smoke Test
   (smoke_test)` (I 9491). From acceptance to the install: 1.42 s.
6. The last screens, held to the probe's end (30 s of rounds):
   - hacker: "Done", "Sent 'Smoke Test'", "They still need to install
     it", "A: share more   HOME: home";
   - organizer: "Done", "Added 'Smoke Test'", "Find it in your
     launcher", "A: share more   HOME: home".

   No fault, no panic text. Reports queued: 1,217 to the hacker, 2,738 to
   the organizer.
7. The HCI commands, per badge: the startup set Milestone 10 traced,
   then `0x2036` (hacker 2, organizer 3; a 30 ms interval while
   transferring, 200 to 220 ms otherwise), `0x2041` (2 and 3; a 100 ms
   window in a 100 ms interval while transferring, 30 ms in 150 ms
   otherwise), `0x2042` (3 and 5), `0x2039` (4 each) and `0x2037`.
   The hacker sent `0x2037` 25 times: the placeholder, OFFER × 4, DATA
   × 13 (11 full, the 102-byte last one twice) and DONE × 7. The
   organizer sent it 213 times: the placeholder, ACCEPT, and STAT × 211
   (each bitmap update). All answered Success; no Unknown HCI Command.

**What is missing: nothing in the emulator.** Two emulated badges move an
app with Share, end to end, on today's controller model. In the browser:

- **No way to install a custom app** in firmware mode. With none, Share's
  sender stops at "No custom apps yet" / "Install one with the
  console:\nput + reload". The browser types into the console only for
  provisioning (`provisioner.ts`).
- **The browser's advertisement sampling is untested for Share.** The
  Worker posts the advertisement at most every `ADV_REPOST_MS` (10 Hz),
  and a DATA frame is held for 3 ticks (60 to 90 ms; the source's
  `kDwellTicks`). A frame between two posts is never sent to the peer.
  STAT repair re-sends it, inside the source's 10 s no-progress window
  (`kStallWindowTicks` 500) and 60 s sweep cap (`kSweepCapTicks` 3000).
  Share's engine holds the matching constants (`li a4, 0x1f4` at
  `0x42057304` and three other sites; `li a4, 0x5dc`, the source's
  1500-tick offer latch, at `0x42056bfc`), but which compare is which
  was not traced.

**A size for Milestone 13: small.** No core or model change. Three
pieces:

- a two-badge Share rung pinning the probe's end;
- a browser control that installs a Lua app over the console, as the
  provisioner types;
- a two-browser check of a transfer over WebRTC at 10 Hz.

The connection model left out in Milestone 10 (R-T5-8; Task 7 skipped)
is not needed.

### Sync's timeout: 20 s from the source's 750 ticks (Task 6)

**v392's constants are the source's** (`bbx_xfer.cpp`: 250 ticks of
20 ms per OPEN attempt, 3 attempts, "15 s" by its comments):

- `bbx_xfer_tick` is `0x420493a0` (one caller, Sync's `on_tick`,
  `0x4204457e`). Its state byte is `0x3fcb5e91` (`lbu a5, -0x16f(a5)` at
  `0x42049412`), and OPEN is state 1 (`0x4204942e`).
- OPEN branch: `s_ticks` (`0x3fcb5e50`) is incremented at `0x42049444`,
  then `li a4, 0xf9; bge a4, a5` (`0x4204944e`): under 250 it goes to
  `0x420497f8`, which re-sends OPEN at tick 20 (`li a4, 0x14`, then
  `send_open(0)` at `0x42049800`).
- At 250, `s_open_retries` (`0x3fcb5e4c`) is incremented (`0x4204945a`),
  then `li a4, 0x2; blt a4, a5` (`0x42049464`): a third timeout calls
  `finish(8, 2)` (`0x420497ee`), the Failed state with reason 2. Earlier
  ones re-enter OPEN (`0x42048e92`, which zeroes `s_ticks` and sends
  OPEN).
- Reason 2 is "No sync station in range" / "Move closer to the sync
  desk" (`0x4204401e`..`0x42044030`), drawn by `0x42043b02`.
- So the timeout is 750 ticks, and 15 s only if every tick is 20 ms.

**The tick is 20 or 30 ms, by the firmware's own scheduling.**

- The app registry's tick is an LVGL timer of 20 ms:
  `lv_timer_create(0x4203a3a0, 20, NULL)` (`li a1, 0x14` at
  `0x4203a8ba`, `lv_timer_create` `0x420c021e`, between
  `lvgl_port_lock` and `lvgl_port_unlock`).
- The LVGL task (`0x420a318c` on) runs `lv_timer_handler` (`0x420c0332`),
  then makes a `vTaskDelay(1)`-shaped call at `0x420a3244` (to
  `0x40391332` with `a0` = 1), then sleeps whole FreeRTOS ticks: the ms
  `lv_timer_handler` returns × 100 / 1000 (`li a5, 0x64; mul` at
  `0x420a325c`, then `mulhu` by `0x10624dd3` and `srli 6`), at least 1
  (`li a4, 0x1` at `0x420a31ea`), in an event-group wait (the call at
  `0x420a3292`, bits `0xff`). `CONFIG_FREERTOS_HZ` is 100
  (`docs/firmware-emulator-notes.md`).
- So each pass sleeps to the next 10 ms tick boundary, then waits
  ⌊ms/10⌋ more ticks (at least 1), and the task wakes only on 10 ms
  boundaries. A timer that comes due just after one waits for the next,
  and the 20 ms timer runs every 20 or every 30 ms: a 30 ms tick is a
  pass whose own work crosses a boundary.

**Measured** (a scratch traced probe, release, deleted). Entry times of
`0x4203a3a0` (the registry tick), `0x420493a0`, `0x42048e92`,
`0x42048c82` and `0x42043b02`, in 500-step traced chunks (about 6 us),
from `sync_open_with_credential()` and A:

| Window | Registry tick intervals | Total |
|---|---|---|
| Launcher, idle, 1.5 s | 74, every one 20 ms | — |
| Snake, 1.5 s | 60 × 20 ms, 8 × 30 ms, 1 × 50 ms | — |
| Sync, the first OPEN to `finish` | 249 × 20 ms, 500 × 30 ms | 19,984,116 us |
| Sync, "No sync station" to the home's return | 200 × 20 ms, 201 × 30 ms | 10,066,523 us |

- `bbx_xfer_tick` ran 750 times in OPEN: the 3 × 250 above. 249 × 20 +
  500 × 30 = 19.98 s; NimBLE's stop is at the firmware's `I (21121)`
  against OPEN's start at `I (1131)`.
- The emulated clock is not slow: on the idle launcher every tick is
  20.000 ms. A tick stretches to 30 ms when its work, or a redraw
  before it, pushes the next due time past a 10 ms boundary (read from
  the wait's arithmetic, not traced tick by tick). "Asking the
  station..." redraws its pulsing border, and two ticks in three are
  30 ms.
- The physical badge shows the same quantization. The source's Snake
  steps every 8 ticks ("160 ms at the 50 Hz app tick",
  `kStepIntervalTicks`), and the badge (v335) was filmed at 170 ms ± 3
  ms (Milestone 6, `BADGE_MOVE_INTERVAL_US`), that is 7 × 20 + 30. The
  emulator's Snake on v392 gives the same 170 ms
  (`SNAKE_MOVE_INTERVAL_US`). v335's and v392's Snake constants were not
  read here.

So 750 ticks explain the 20 s. The emulated clock is not slow (the idle
launcher ticks at exactly 20 ms); the stretch is the firmware's
quantization of its own work time. The emulator charges 1 cycle per
step (`emulator-core/src/mem/bus.rs:524`, `Clock::new(XTAL_HZ,
SYSTIMER_HZ, 1)`), so emulated CPU work is no longer than the badge's;
peripheral waits (SPI and DMA flushes) modelled slower than the
hardware would add ticks. No fix, and no pin moves (ruling R-T6-1).
The gate's 15 s case (an emulated tick slower than the firmware asks
for) is not what was found.

**The auto-return.** "No sync station in range" holds, then Sync goes back
to its home ("Find a sync station", `SYNC_HOME_HASH`) without input:

- The hold is `error_ticks_ = 400` (`li a5, 0x190; sw a5, 0x3c(s0)` at
  `0x42043b4e`, in `0x42043b02`), the source's `kErrorHoldTicks` ("~8
  s").
- `on_tick` (`0x4204455c`) counts it down (`0x42044574`..`0x4204457c`,
  then `bbx_xfer_tick`), and the render (`0x4204431c`) keeps the error
  screen while it is above 0 (`lw a5, 0x3c(a0); bgtz` at `0x4204432a`).
- Measured: `0x42043b02` at +20.070 s after A, the stable no-station
  frame at +20.117 s, `SYNC_HOME_HASH` back at +30.137 s: 10.07 s of
  emulated time (400 ticks, the same 20/30 ms mix).
- The browser (Task 3, the orchestrator's wall-clock stopwatch; the
  page displayed about 1.0x) showed it for both roles: hacker, then the
  same badge re-provisioned as organizer. "No sync station in range"
  came about 22.4 s (hacker) and 22.2 s (organizer) after A. The
  auto-return, measured on the hacker run, came after about 12 s of
  hold. The emulated values are 20.1 s and 10.1 s, so against the wall
  clock the emulator ran at about 0.85x to 0.9x there, below the
  displayed speed.

## Gates and the human partner's answers

- **Gate 1** (2026-11-11): the spec approved. **Gate 2** (2026-11-11):
  the plan approved, run subagent-driven.
- **Gate 3** (2026-11-11), Sync's frames (`local/m12-frames/`, and the
  orchestrator's Chrome check of Task 3): the human partner said the
  frames look right. The partner did not rule on whether
  re-provisioning's tutorial reset (below) is acceptable; it is
  recorded as known firmware behaviour, with no code change.
- **Gate 4** (2026-11-11), the Vercel preview of `6df703a`: a host left
  to time out (the room gone) and a WebRTC link both worked. Pairing
  first appeared to fail: Connect showed "No badge found, try again" on
  both badges (Safari on a Mac and an iPad, one network; the same was
  seen on main's production deploy). Both badges had been provisioned
  as the same hacker identity, and Connect's receive branch
  (`0x4201ee96`, Milestone 10 Task 11) drops a card carrying its own
  badge ID. The human partner called it a false alarm: with distinct
  identities pairing works. Gate 4 passed. Known limitation: two badges
  with the same identity never pair (the notes' "Known limitations",
  item 1). No code change.

**Re-provisioning, recorded at Gate 3.** `prov apply` on a registered
badge does not reboot (Task 3's trace, above). `badge_token get` reads
`BADGE_TOKEN MISSING` for the new identity until the token is set
again; the provisioner now always sets it. The firmware prints
`tutorial=reset`: the launcher stays on screen until the next button
press, which drops into onboarding (the orchestrator's Chrome check:
re-provisioned as organizer from Sync's screen, onboarding walked
again, the same no-station frame at about 22.2 s).

## Dependencies added

None. `git diff 35a6c37..HEAD` touches no `package.json`, `Cargo.toml`,
`Cargo.lock` or `bun.lock`. Task 1's CRC is the hand-written bitwise
loop the spec chose over a `crc32fast` dependency.

## Rulings

Ruling R-T1-1: `crc32_le_stub_wild_buffer_does_not_panic` asserts the exact result (the CRC of `MAX_STUB_MEMORY_BYTES` zero bytes from seed 0, computed in the test by a reference that repeats the stub's bitwise loop, so it checks the cap and the zero reads but not the CRC itself; Milestone 13 Task 8 made it a table-driven reference checked against CRC-32's published `0xCBF43926`), not only "some value", so it also pins the cap and the unmapped reads as zeros — cost if wrong: the test costs about 0.1 s in debug; if the cap ever changes, the test follows it through the shared constant.

Ruling R-T1-2: the stub's citations add ESP-IDF v5.5.3's `esp32c3.rom.api.ld:4` (a third fetched file) to the two the brief named, because `esp_rom_crc.h` documents `esp_rom_crc32_le`, and that alias is what ties the header's semantics to the ROM's `crc32_le` — cost if wrong: one extra citation line.

Ruling R-T2-1: the packing frame ("Packing your badge...", `0xb882_9410_38c1_75a2`) holds about 4.2M steps, under `AFTER_PRESS_STABLE_FOR`, so the rung pins the console line `sync: snapshot 1026 bytes, crc ec2f90df,` (with `complete=1`; the `37ms` run time left out) instead of a hash, and the frame is kept only as a PNG for the eye check — cost if wrong: a change that redraws the packing frame wrongly but still prints the line would pass; the stable no-station hash and the `uitree` texts still bound it.

Ruling R-T2-2: `SYNC_PACKING_STEPS` and `SYNC_NO_STATION_STEPS` count from A's press, not its release, because the snapshot line is printed inside the 100 ms hold (the release-relative measurement would be 0) — cost if wrong: none to behaviour; a reader comparing with `SYNC_RESPONSE_STEPS` (from the release) must note the different origin, which each constant's doc states.

Ruling R-T2-3: `sync_open`'s body moved, unchanged, into `open_sync_from(rt)`, which `sync_open_with_credential` shares, rather than copying the walk and its console-order assertions — cost if wrong: none to behaviour (every Milestone 11 pin and `sync_open`'s rungs pass unchanged); one more helper name.

Ruling R-T2-4: `credential_launcher` also asserts `radio().hook_hits() == 0`, as `launcher_with_mac` does, besides the brief's `radio_init_count() == 0` — cost if wrong: one extra assertion.

Ruling R-T3-1: the probe found no reboot on `prov apply` (first-run or re-provision), so the pinned re-provision test is `provisioner_sets_the_token_after_prov_ok_without_a_reboot`; the token phase still applies the boot-console-start rule (the later of the `PROV OK` line's end and `bootConsoleStart()`), pinned by a second test, `provisioner_waits_for_the_current_boots_prompt_after_prov_ok` — cost if wrong: one rule and one test for a case v392 does not show; if a later firmware reboots on `prov apply`, the prompt rule already holds.

Ruling R-T3-2: beyond the brief's tests, `provisioner.test.ts` adds `provisioner_fails_on_badge_token_fail_before_ready` (a FAIL instead of READY never sends the token), `provisioner_sends_a_given_upload_token` (the option's non-default value) and a constants test; the old test "waits for OK and the prompt, then types prov apply and reports PROV OK" now ends in `typing` at the `PROV OK` line (the default sets the token; the skip test keeps the old `ok`-at-`PROV OK` path) — cost if wrong: three small tests; the renamed test no longer pins `ok` on its own.

Ruling R-T3-3: `redraw-bench.ts` passes `() => 0` for `bootConsoleStart` (the default's value; the bench never reboots while provisioning) to reach the options argument — cost if wrong: none to behaviour.

Ruling R-T4-1: `lib` goes from `ES2020` to `ES2022` (the `target` stays `ES2020`) rather than rewriting 50 `.at()` calls and one `.replaceAll()` in the tests, because `bun run build`'s output is byte-identical with it (same files, sizes and SHA-1s) — cost if wrong: `src/` may now type-check an ES2021/ES2022 library call (`.at()`, `.replaceAll()`, `Object.hasOwn`, …) that `target` does not down-level; Vite's default build target (widely available browsers) already ships those, and a browser without them would fail at that call.

Ruling R-T4-2: the seven non-`lib` errors are fixed by typing, not loosening: `manifest.test.ts` reads the absent key with `Reflect.get` (no cast); `signal-client.test.ts` and `signal-handler.test.ts` name their case arrays `Record<string, string>[]`; `redraw-bench.test.ts`'s fake gains `softResetCount: () => 0` and `rebootUs: () => 466_000` (`REBOOT_US`, as `firmware-runtime.test.ts`'s fake); `vite.config.ts`'s `readCapped` is typed `Promise<Buffer<ArrayBuffer> | null>`, what `Buffer.concat` already returns (type-only; the build is unchanged); and `shell.test.ts` takes one commented `as unknown as MouseEvent` cast in a `mouse()` helper, since happy-dom's `MouseEvent` typings lack only `isTrusted`, `returnValue` and `srcElement` — cost if wrong: the cast would hide a real happy-dom/DOM event mismatch in that one keyboard-and-pad test; its assertions on the Worker's messages still run.

Ruling R-T5-1: `call` combines the caller's signal and the timeout through a hand-made `AbortController` and `setTimeout`, cleared (and the listener removed) once the fetch settles, not `AbortSignal.any`/`AbortSignal.timeout`, though both are in the DOM lib types: `AbortSignal.any` is missing before Safari 17.4 and Chrome 116, and there `call` would throw on every request, while `AbortSignal.timeout`'s timer cannot be cleared — cost if wrong: about eight lines where two would do; the timer still covers only the fetch up to its headers, not the small JSON body read after it.

Ruling R-T5-2: `REDIS_OPTIONS.retry` is `{ retries: 0 }`, not the brief's `false`, because @upstash/redis 1.41.0 reads `retry: false` as `attempts: 1` and its request loop runs `i <= attempts`, so `false` still re-sends a request after a network error, with no backoff (`node_modules/@upstash/redis/chunk-*.mjs`, `HttpClient`'s constructor and `request`); `redis_options_disable_retries` pins one fetch per command with `{ retries: 0 }` and two with `false`, so a library upgrade that changes either shows up — cost if wrong: none to behaviour; the option's value differs from the plan's interface line (`REDIS_OPTIONS.retry === false`).

Ruling R-T5-3: `client_stops_on_500`'s count of requests after the failing one leaves out the host's `close`, which a failed host now sends by design; every other request still counts, and `host_sends_close_on_a_server_error` pins the one `close` — cost if wrong: a stray second `close` would not show in that test, but the four `host_*_close*` tests count closes exactly.

Ruling R-T6-1: Sync's 20 s is recorded as the firmware's own timing, with no change: v392 times out after 750 registry ticks (the source's 3 × 250), and its 20 ms LVGL timer runs every 20 or 30 ms because the LVGL task sleeps in whole 10 ms FreeRTOS ticks (`0x420a325c`). The emulated clock gives exactly 20 ms on the idle launcher, and Snake's 7 × 20 + 30 = 170 ms matches the filmed badge — cost if wrong: if the physical badge's "Asking the station..." ticks are mostly 20 ms (its tick work ending before a 10 ms boundary where the emulator's does not), it gives up about 5 s sooner than the emulator; `SYNC_NO_STATION_STEPS` would then be generous, not wrong. A stopwatch on a v392 badge, from A to "No sync station in range", settles it (about 20 s against about 15 s).

Ruling R-T6-2: Share's probe used a hand-written `manifest.cfg` (`heap_kb=48`, `wake_lock=0`, `home_button=1`, `confirm_home=0`), because v392's loader refuses the committed `frontend/public/apps/smoke-test/manifest.cfg`: `script_app: skipping /littlefs/apps/smoke_test: heap_kb must be 48 or 96`, then, with that fixed, `wake_lock must be 0 or 1` (it says `true`/`false`). The committed file is left alone: Lua mode reads it, and changing it is Milestone 13's call — cost if wrong: none to behaviour; a Milestone 13 install control that sends the committed bundle as it is will be refused by the badge.

Ruling R-T6-3: the HCI command trace came from a temporary `eprintln!` in `Radio::on_hook`'s send path, gated by an environment variable and reverted before the commit, because the controller model keeps no command log — cost if wrong: none; re-adding the line reproduces the trace.

## Final suites

At `89899dc` (Task 6's last commit; the commit after it is these docs
only), from the full check list, run once in order, each step timed
with `/usr/bin/time -p` (wall):

| Check | Result |
|---|---|
| `cargo fmt --all` | no changes, 0.17 s |
| `cargo test --workspace` | 757 passed, 0 failed, 1 ignored, 30.06 s: `emulator-core` lib 636; debug `boot_progress` 90 + 1 ignored in 26.81 s; the seven other integration binaries 31 |
| `cargo test -p emulator-core --release --test boot_progress` | 90 passed, 1 ignored, 18.60 s (20.20 s with the build; the 90 s bound holds) |
| `cargo clippy --workspace --all-targets` | 0 warnings, 0.61 s |
| `bun run build:wasm` | OK, 1.44 s |
| `bun test` | 339 pass, 0 fail, 18,575 `expect()` calls, 20 files, 13.20 s |
| `bun run typecheck` | 0 errors, 0.24 s |
| `bun run build` | OK, 0 warnings, 0.22 s |

Against Milestone 11's baseline at `35a6c37`: release `boot_progress`
88 → 90 (Task 2's two rungs; about 15 s → 18.6 s); `cargo test
--workspace` 751 → 757 (Task 1's four `crc32_le` tests, Task 2's two
rungs); `bun test` 319 → 339 (Task 3's 11, Task 5's 9).

`bun run build`'s output (bytes, `ls -l`): `index.html` 984,
`bridge-Ctgh-ak7.js` 5,919, `firmware-worker-BYvDG9ar.js` 8,946,
`emulator_wasm_bg-DjWHtz-t.wasm` 142,270 (Vite's gzip 62.64 kB),
`index-XRjmGPO9.css` 2,962, `index-CCVnrp6Y.js` 261,789 (gzip
86.61 kB). Against Task 4's record: only `index-*.js` changed,
261,491 → 261,789 (+298 B, Task 5's signaling client).

## Milestone 13 backlog

### New in Milestone 12

- **Share between two browsers** (Task 6: the core and the controller
  model need no change). Three pieces: a two-badge Share rung pinning
  the probe's end; a browser control that installs a Lua app over the
  console, as the provisioner types; a two-browser check of a transfer
  over WebRTC, where the Worker posts the advertisement at most at
  10 Hz and a DATA frame dwells only 60 to 90 ms (STAT repair should
  cover the frames it misses; untested).
- **v392's loader refuses the committed smoke-test manifest**
  (R-T6-2): `heap_kb must be 48 or 96`, then `wake_lock must be 0 or
  1`. An install control that sends `frontend/public/apps/smoke-test/`
  as it is will be refused; Lua mode reads the same file.
- **A fake sync station.** The protocol is advertisement-only (v392's
  NimBLE links no connect or GATT procedure, Task 6's trace; Sync's
  OPEN goes out through `send_open`), so a station is possible on
  today's model. The human partner chose
  not to build one this milestone (2026-11-10).
- **A stopwatch check of Sync's 20 s on a v392 badge** (R-T6-1): from A
  to "No sync station in range", about 20 s (the emulator's 750 ticks of
  20 or 30 ms) against about 15 s (every tick 20 ms).
- **The fetch timeout ends at the headers** (Task 5's deferred minor):
  `call` clears its timer once `fetch` settles, so a reply whose body
  stalls hangs `res.json()`, and with it the joiner's offer read,
  weakening Review Focus 4's bound. Fix: read the body before clearing
  the timer (R-T5-1 notes the same gap).
- **Re-provisioning's tutorial reset** ("Gates", above): firmware
  behaviour, recorded, not ruled on by the human partner.
- **Milestone 12's other deferred review minors** (the ledger):
  - Task 1: the wild-buffer test's "independent" reference repeats the
    stub's loop, and R-T1-1's wording overstates it; the
    `esp_rom_crc.h` line ranges differ between `rom_stubs.rs`
    (`:62..:70`) and `rom.rs` (`:23..:70`); a `--` for `—` in the
    `crc32_le` constructor's doc.
  - Task 2: the `complete=1` check snapshots the console at the prefix
    match (it would fail falsely, never pass falsely, if the line moved
    past the release); `PROVISION_DEADLINE` is reused for the token
    phase and its doc does not mention the token's ~0.29M steps; a
    redundant `assert!(text.contains(TOKEN_OK))` after
    `set_upload_token`'s `Ok`; `set_upload_token`'s needle checks follow
    list order, not console order, within one chunk.
  - Task 3: Milestone 11's "reports PROV OK" provisioner test now ends
    in `typing` (R-T3-2); a caller's `uploadToken` is not validated (a
    value not 64 bytes long waits to the timeout); a `token as string`
    cast in `provisioner.ts`; one overlong line in this doc.
  - Task 4: `lib` ES2022 ahead of `target` ES2020 (R-T4-1's cost); the
    commented `as unknown as MouseEvent` cast in `shell.test.ts`.
  - Task 5: no dedicated `close` test for `waitOpen`'s deadline or a bad
    answer (the same catch covers them); a timed-out `close` is dropped
    silently (best effort).

### Carried from Milestone 11's backlog (still open)

Word for word in `docs/milestone-11-decisions.md`, "Milestone 12
backlog", minus what this milestone closed: Share's transfer (traced,
above), Sync's empty-room path (Tasks 1 to 3), the `tsconfig.json`
`include` (Task 4) and the three signaling items (Task 5).

- **A cold power-on's 466 ms.** The shortcut boot starts the app at
  0 µs; the badge takes about 466 ms. Adding it would move every pin for
  no visible gain (Milestone 11's spec, "Non-goals").
- **A joiner who answered before a cancel waits out `waitOpen`.** A
  joiner who arrives after the cancel gets the 404 at once. Covering the
  earlier one needs a new poll, which costs commands (Milestone 11's
  spec, Part 4.6).
- **The reset model:** the CPU-reset table's silent rows (INTC, R-T1-2;
  the external chips); the core reset narrower than the TRM; the SYSTEM
  clock-gate and reset bits not modeled; DRAM `.noinit` not kept; the
  RTC and timer-group watchdogs inert; three reboot apps' 465,999 µs
  deltas, untraced; v335's 0.2M steps after `PROV FAIL`, not
  established.
- **The provisioner:** a console at its 256 KiB cap plus a soft reset
  makes the boot-scoped prompt scan never match (R-T13-3); a provision
  past the first prompt when the badge reboots loses bytes in the new
  REPL's terminal probe, then times out cleanly.
- **The radio model:** `host_rcv_pkt` blocking on the real firmware is
  not established (Milestone 10's R-T6-2, half open); LE Set Extended
  Scan Enable's other parameter rules (Core v5.4, 7.8.65).
- **The browser:** Provision in a tab whose badge was since freed; a
  Provision and a `startFirmware` racing on a claim; no feedback for a
  refused Bump; an advertisement change just before a Worker stop never
  posted; the host tab's stale "Room …: enter this code" line after
  Remove; `inject` feeding a stopped runtime behind the "another tab"
  notice; the speed readout at about 1x through the reboot's pause
  (R-T8-2); a shake or bump across a reboot losing its tail (R-T8-3);
  `status.softResets` unused by the Worker client.
- **Carried from Milestone 8's backlog**, as `milestone-11-decisions.md`
  lists them: Milestone 7's final-review items; speed and timing (Snake's
  late game-over after an eat, CPI 1, zero-latency RMT, I2C0 and
  SPIMEM1, PCER/PCMR, `PROVISION_TIMEOUT_STEPS` in steps); apps and UI
  (no app's deeper screens tested beyond Connect's pairing and Sync's
  "not ready" and "no station", a browser RNG seed, a console pane,
  `DeviceMotionEvent`, Milestone 5 Task 7's minors, Task 7's small
  cleanups, only role hacker's frames compared with hardware);
  peripherals (RMT, NFC, the busy phase after START, SC7A20H, I2C0's
  empty-FIFO `OP_WRITE`, the `put` task's CPU time, `R_M5_3`'s closure,
  `strtol`/`atoi` and `errno`, the 8 MiB stub cap, the idle unmapped
  accesses); build and tests (the release suite's one chain, Milestone
  6's test minors); and Milestones 3 and 4 (`MADCTL`, GPIO0 routing, the
  eFuse block beyond the factory MAC, flagged ROM-stub guesses, edge
  interrupts, GDMA's corners, the 250,000-step chunks, small
  duplications).

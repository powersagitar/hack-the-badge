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
run release, then reverted. Token used: `0123456789abcdef` four times.

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

## Traces

(Filled by later tasks.)

## Dependencies added

None so far. Task 1's CRC is the hand-written bitwise loop the spec
chose over a direct `crc32fast` dependency.

## Rulings

Ruling R-T1-1: `crc32_le_stub_wild_buffer_does_not_panic` asserts the exact result (the CRC of `MAX_STUB_MEMORY_BYTES` zero bytes from seed 0, computed by an independent bitwise loop in the test), not only "some value", so it also pins the cap and the unmapped reads as zeros — cost if wrong: the test costs about 0.1 s in debug; if the cap ever changes, the test follows it through the shared constant.

Ruling R-T1-2: the stub's citations add ESP-IDF v5.5.3's `esp32c3.rom.api.ld:4` (a third fetched file) to the two the brief named, because `esp_rom_crc.h` documents `esp_rom_crc32_le`, and that alias is what ties the header's semantics to the ROM's `crc32_le` — cost if wrong: one extra citation line.

## Final suites

(Filled at the finish.)

## Milestone 13 backlog

(Filled at the finish.)

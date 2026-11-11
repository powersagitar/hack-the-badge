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
debug `boot_progress` 27.0 s); `bun test` 330 pass, 12.4 s; clippy and `bun run build` 0
warnings.

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

## Dependencies added

None so far. Task 1's CRC is the hand-written bitwise loop the spec
chose over a direct `crc32fast` dependency.

## Rulings

Ruling R-T1-1: `crc32_le_stub_wild_buffer_does_not_panic` asserts the exact result (the CRC of `MAX_STUB_MEMORY_BYTES` zero bytes from seed 0, computed by an independent bitwise loop in the test), not only "some value", so it also pins the cap and the unmapped reads as zeros — cost if wrong: the test costs about 0.1 s in debug; if the cap ever changes, the test follows it through the shared constant.

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

## Final suites

(Filled at the finish.)

## Milestone 13 backlog

(Filled at the finish.)

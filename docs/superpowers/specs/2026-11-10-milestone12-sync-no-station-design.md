# Milestone 12: Sync to "No sync station in range"

Status: scope agreed in conversation with the human partner (2026-11-10).
main @ `35a6c37` (the Milestone 11 merge), and `origin/main` is the same:
the human partner had main pushed on 2026-11-10, before this spec.
Branch `milestone-12`, worktree `.claude/worktrees/milestone-12`.
**Approved by the human partner on 2026-11-11.**

## Why

Sync is one of the launcher's 16 apps, and today it ends at "Badge not
ready to sync": the fake identities have no upload credential
(Milestone 10's R-T6-5). The human partner wants Sync to go as far as the
firmware goes before it meets something the browser cannot supply, and
chose to stop at **"No sync station in range"**: the real limit is that no
sync station exists (a mains-powered ESP32-C3 box that bridges the badge's
advertisements to HackerAPI over Wi-Fi), and none is emulated.

Three small items ride along: three signaling faults, type-checking the
test files, and a trace of Share's radio transfer that records how big
Milestone 13's version would be, with no implementation.

## Decided by the human partner (2026-11-10)

- **Lead: Sync's empty-room path**, ending on "No sync station in range /
  Move closer to the sync desk".
- **No fake sync station.** The badge's sync protocol is connectionless
  and an emulated station is possible (below, "Facts read"), but the human
  partner chose to stop before it.
- **The organizers' OK covers a fake upload credential**, used with the
  committed fake identities ("Can use fake identities", 2026-11-10).
- **Every offered ride-along is in:**
  1. the signaling faults: a fetch timeout in `signal-client.ts`, Upstash
     retries off, and `close` when the host gives up;
  2. every test file type-checked (`tsconfig.json`'s `include`);
  3. a trace of Share's radio transfer, recorded, not implemented.
- **main was pushed** (`7f3960d..35a6c37`, 2026-11-10). A push or deploy
  of Milestone 12 is a separate question at the finish.

## Facts read (2026-11-10)

### The firmware's source

The organizers published the firmware's source at
https://github.com/hackthenorth/badge-firmware (one squashed commit,
`23c66f0`, "Open source firmware release", 2026-10-14). It is **later than
v392**: v392's commit `d3089c4` is not in it, and it has more apps than
v392's 16. So it is a guide, and every fact this milestone pins comes from
v392's binary or a run of it. Fetched into the session scratchpad only,
never into the repo. Files read: `main/apps/app_sync.cpp`,
`main/apps/bbx_proto.h`, `main/apps/bbx_xfer.cpp`,
`main/hal/hal_identity.cpp`, `main/hal/hal_console.cpp`,
`main/apps/share_xfer.h`, `tools/sync_station/README.md`.

What they say (public source; the v392 checks follow):

- **The credential** (`hal_identity.cpp`, `hal_identity_get_upload_token`
  and `_set_`): NVS namespace `badge_upload`, key `credential`, a blob of
  the token (64 lowercase hex characters and a NUL), the identity's
  `badge_id` and its `claim_id`. It is valid only for the identity it was
  set against. Setting it needs a provisioned identity whose `claim_id` is
  all digits and does not start with `0`.
- **`badge_token set`** (`hal_console.cpp`): prints `BADGE_TOKEN READY`,
  reads exactly 64 bytes from stdin within 10 s, prints `BADGE_TOKEN OK`
  or `BADGE_TOKEN FAIL`. The token never enters `identity.json`, REPL
  history or snapshot output.
- **Sync's A** (`app_sync.cpp`, `start_sync`): no token → "Badge not
  ready to sync" (today's rung). With one: "Packing your badge..." (a
  snapshot of the badge's files staged to LittleFS), a CRC32 of it with
  `esp_rom_crc32_le` (seed 0), then `bbx_xfer_start`: "Asking the
  station..." with a pulsing border, and after the OPEN retries run out,
  `XferFail::NoStation` → "No sync station in range", "Move closer to the
  sync desk", footer "A: try again   -   HOME: home".
- **The station protocol is connectionless** (`bbx_proto.h`, "BBX1"):
  every frame (SYNC_BEACON, XFER_OPEN, GRANT, DATA, STAT, DONE,
  PROFILE_DATA, ACK) is manufacturer data (company `0xFFFF`) in an
  extended advertisement, at most 225 bytes. This is recorded for a later
  milestone; this one emulates no station.
- **Share's transfer engine** (`share_xfer.h`) is also advertisement-only:
  OFFER, ACCEPT, DATA sweep, STAT bitmap repair, DONE, over the same
  `hal_radio` channel. Nothing under `main/hal/` or `main/apps/` calls GAP
  connect, GATT or L2CAP. Whether v392 matches is Part 4's question.

### v392's binary

`frontend/public/firmware/firmware.bin` contains, once each:
`BADGE_TOKEN READY`, `HTNSYNC1` (the snapshot's transport header),
`Find a sync station`, `Station is sending it up`, `No sync station in
range`, `Profile update failed`. The 4-byte magic `BBX1` does not appear
as a string (the source builds it byte by byte).

### ROM `crc32_le`

ESP-IDF v5.5.3, `components/esp_rom/esp32c3/ld/esp32c3.rom.ld:415`:
`crc32_le = 0x40000628;`. The neighbours (`:416`..`:421`) are `crc32_be`,
`crc16_le`, `crc16_be`, `crc8_le`, `crc8_be`, `esp_crc8`; none is reached
(below). Semantics, from ESP-IDF's `esp_rom_crc.h` (the builder fetches
and cites it): `crc32_le(crc, buf, len)` is the zlib-compatible CRC-32,
reflected polynomial `0xEDB88320`, with the running value inverted on the
way in and out, so a seed of 0 gives zlib's `crc32(0, buf, len)` and a
result can be passed back in as the next seed.

## Measured (2026-11-10, scratch probes, not committed)

Two scratch tests at the end of `boot_progress.rs`, with a scratch
`crc32_le` HLE effect (bitwise, the semantics above), run release, then
reverted. The diff is in the session scratchpad
(`m12-sync-probe.diff`). Token used: `0123456789abcdef` four times.

1. **Without the stub, setting the token faults.** From `sync_open()`
   (the open Sync, no credential): `badge_token set` → `BADGE_TOKEN
   READY`; after the 64 bytes, an instruction access fault at
   `0x40000628`. NVS checksums every entry it writes, so the stub is
   needed for the credential itself, before Sync's own CRC.
   **Nothing reached `0x40000628` before this**: a fetch there faults, and
   no existing rung faults, so adding the stub cannot move a Milestone 11
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
   steps after `PROV OK`'s prompt, `BADGE_TOKEN OK` at 300k. The existing
   walk helpers (`walk_to_shake_page_from`, `walk_to_registered_from`,
   `walk_to_launcher_from`), which assert every hacker frame, all pass:
   the launcher is reached at 342,894,540 steps and 94,521,622 µs
   (`RADIO_LAUNCHER`: 342,600,681 and 94,521,615). Sync opens to
   `SYNC_HOME_HASH`, and A ends on the same `0xa465_e4c0_e16e_ede6`
   (stable from +415,049,485 steps; +21.1 s emulated when the probe
   stopped).

**An open question for Part 4's trace:** the public source's timeout is
3 OPEN attempts × 5 s (`kMaxOpenRetries = 3`, `kOpenTimeoutTicks = 250`
at 50 Hz), which would be 15 s, but v392 took about 20 s. Either v392's
constants differ, or the emulated tick runs slow. Snake's timing was
compared with the badge, so the first is likelier; the trace settles it
from v392's disassembly.

## Goals

1. **The `crc32_le` ROM stub**, at `0x4000_0628`, a real implementation.
2. **A fake upload credential** set by both provisioners (the browser's
   and the Rust test helper's opt-in variant) through the firmware's own
   `badge_token set`.
3. **Rungs** pinning Sync's path with a credential: "Packing your
   badge...", "Asking the station...", and "No sync station in range".
4. **In the browser**, Provision then Sync's A reaches "No sync station in
   range".
5. **The ride-alongs:** the three signaling fixes, every test file
   type-checked with 0 errors, and Share's trace recorded.
6. **Docs:** `docs/milestone-12-decisions.md`, CLAUDE.md, and the notes.

## Non-goals

- A fake sync station, and anything past "No sync station in range":
  uploading, "Synced", the profile downlink.
- Implementing Share's transfer.
- The rest of the Milestone 12 backlog (the reset model, the browser
  items, the provisioner's 256 KiB edge, the radio model's open rules,
  the Milestone 8 carry-overs, and the joiner who waits out `waitOpen`):
  carried to Milestone 13 as written.
- Other roles' Sync frames. Role hacker's is pinned; other roles reach the
  same screen (their names are not on it), and one rung may assert that
  by `uitree` if it is cheap, the builder's call.

## Anti-goals (a failure even if it works)

- **Any Milestone 11 pin moving.** The Rust provisioning helper keeps
  today's behaviour by default; only new rungs set the token. The two
  equivalence rungs stay byte-identical. The WASM twin
  (`test/cpu-wasm.test.ts`) pins frame hashes, not step counts, and probe
  3 shows the token leaves every frame of the walk unchanged, so its pins
  hold while the browser provisioner sets the token; it is not a moved
  pin.
- A real attendee's credential, or anything from `local/`, in the repo.
- A token in `identity.json`, the console history or the page's
  visible text. (Typing it into the emulated console is how the badge
  takes it; the console pane does not exist.)
- A crc stub that only returns a plausible number. Sync's CRC goes into
  OPEN on the air, and NVS checks the CRC it wrote when it reads it back;
  a wrong one would surface later as a missing credential.
- A release `boot_progress` over 90 s.

## Success criteria

- `cargo test -p emulator-core --release --test boot_progress`: today's
  88 rungs unchanged, plus the new Sync rungs, all passing, under 90 s.
- `cargo test --workspace`, clippy (0 warnings), `bun test`,
  `bun run typecheck` (now covering `test/`), `bun run build` (0
  warnings): all pass.
- A unit test for the stub against known CRC-32 vectors (at least
  `"123456789"` → `0xCBF43926` with seed 0, the empty buffer, and
  chaining two calls equals one call).
- In Chrome (chrome-devtools-mcp, by the orchestrator): Provision, walk
  to the launcher, open Sync, A, and "No sync station in range" after
  about 20 s. A screenshot saved under `local/`.
- The human partner checks the new frames by eye (Gate 3).

## Design

### Part 1: The ROM stub (core)

- `crate::rom`: `CRC32_LE = 0x4000_0628`, cited to `esp32c3.rom.ld:415`,
  with a module-doc entry in the existing style.
- The effect: a chip-agnostic `RomStubEffect::Crc32Le` in
  `cpu/rom_stubs.rs` and `cpu/mod.rs` (like `Memcmp`: `a0` = crc, `a1` =
  buf, `a2` = len, read through the bus, capped at
  `MAX_STUB_MEMORY_BYTES`, result in `a0`). The CRC algorithm is generic,
  not chip data, so it may live in `cpu/`.
- Library or hand-written: `crc32fast` is already in `Cargo.lock` (via
  `flate2`/`png`, dev-only), and `crc32fast::Hasher::new_with_initial`
  takes exactly ROM's chaining value. The builder may make it a direct
  dependency or write the 8-line bitwise loop; either way tell the human
  partner (rule "New dependencies"). My call: the bitwise loop, since it
  reads bytes one at a time through the bus anyway, and the snapshot is
  about 1 KB.
- The ROM-libc guard test (`no_rom_libc_function_carries_a_generic_zero_return_stub`)
  gains `CRC32_LE`, with its effect.

### Part 2: The fake credential (both provisioners)

- **The token:** one constant, the same for every role, obviously fake.
  My call: `0123456789abcdef` repeated four times (the probes' token).
  It lives as a constant in `frontend/src/runtime/provisioner.ts` and in
  `emulator-core/tests/common/provision.rs`, kept equal like the other
  mirrored constants, with a test in `test/test-identities.test.ts` (or
  next to it) that checks the two copies match and that the token is
  64 lowercase hex characters. It is not added to the identity JSON
  files, and the eleven files stay byte-identical.
- **The browser's provisioner:** after `PROV OK` and its prompt, types
  `badge_token set`, waits for `BADGE_TOKEN READY`, sends the 64 bytes in
  one chunk (64 bytes, one USB packet, under the 256-byte RX ring), and
  waits for `BADGE_TOKEN OK` and the next prompt. `BADGE_TOKEN FAIL` or
  `usage:` fails the provision with that line. The `ok` state keeps
  `PROV OK …` as its line (the shell shows it), and arrives after the
  token. Its existing tests gain the token phases.
- **Re-provisioning** (my call): Provision can be pressed at any time
  (from the launcher, for another role, twice), and a token is valid
  only for the identity it was set against, so **every** Provision sets
  it again. Probe 3 covered only the first-run screen. The builder first
  probes `prov apply` on a registered badge (does it reboot, and where
  does its next prompt appear?), and the token phases wait for a prompt
  printed after the `PROV OK` line in the current boot, by the same
  boot-console-start rule as the first prompt (R-T13-3), with a test for
  each case found. If a case cannot be made to work, it is recorded as a
  known limitation and raised at Gate 3.
- **The benchmark keeps today's run** (my call): `createProvisioner`
  gains an option to skip the token, defaulting to setting it, and
  `src/bench/redraw-bench.ts` passes it, so WASM bench rows stay
  like-for-like with Milestones 7, 9 and 11. The WASM twin
  (`test/cpu-wasm.test.ts`) uses the default, so it covers the token
  path the browser takes.
- **The Rust helper:** `provision::provision` is unchanged.
  A new `provision::set_upload_token(rt, token, deadline)` does the token
  steps, and new rungs call it after `provision`.
- **Data rule:** CLAUDE.md's "Local data" says the only committed identity
  data is the eleven fake identities. It gains the fake upload token,
  "committed with the organizers' permission (2026-11-10)".

### Part 3: The rungs

All new; none edits a Milestone 11 rung.

- **A checkpoint:** role hacker's launcher with `HACKER_MAC` and the token
  set at provisioning (probe 3), asserting every frame on the walk as
  `launcher_with_mac` does. Its steps and time are pinned (the probe's
  342,894,540 and 94,521,622 µs, re-measured by the builder).
- **Sync with a credential:** from that checkpoint, Sync opens to
  `SYNC_HOME_HASH`; A gives "Packing your badge...", then "Asking the
  station...", then the stable "No sync station in range" frame
  (`0xa465_e4c0_e16e_ede6` in the probes), each pinned by hash (the
  packing frame may be too brief to be stable; then it is pinned by a
  `uitree` or console needle instead, the builder's call), plus the
  console's `sync: snapshot` line (bytes and `complete=1`; the crc value
  pinned too) and `uitree`'s three texts. No fault, no panic text, the
  radio initialized once, no Unknown HCI Command.
- **The credential survives Sync's reboot**: implied by the rung
  (Sync reboots into itself before A); stated in its doc.
- **The no-credential rung stays** (`sync_now_without_a_credential_is_not_ready`)
  and still passes, unchanged: it is now the path for a badge provisioned
  without the token.
- **Budget:** the new walk is about 343M steps and Sync's wait about 415M;
  in release that is roughly 10 s more on today's 15 s. The builder
  measures the suite's wall time and records it; over 90 s is a gate.

### Part 4: Share's trace (recorded, no implementation)

Answers, in `docs/milestone-12-decisions.md`, "Traces":

1. Does v392's Share use LE connections, or only advertisements as the
   public source does? Evidence from v392's binary (strings, the HCI
   commands its code path sends, a disassembly of the call sites), not
   only from the source.
2. Its frame types and sizes, against what the HCI model already carries
   (extended advertising data up to 251 bytes out, reports up to 229
   in).
3. How far two emulated badges get today: a scratch two-badge probe
   (Connect's two-runtime pattern), never committed, recording the last
   screen on each side.
4. A size for Milestone 13: what is missing (a peripheral, a file
   picker's data, a model rule), and what it would take.
5. The 15 s against 20 s question from "Measured".

### Part 5: Signaling

1. **A fetch timeout** in `signal-client.ts`'s `call`: each request
   aborts after a bounded time (builder picks, about 10 s; the server
   poll is answered at once, so this is only for a hung server), and the
   abort maps to the existing "Could not reach the signaling server"
   error. A joiner, who has no cancel, can then never wait past its 120 s
   budget by more than one timeout.
2. **Upstash retries off:** `retry: false` in `api/signal.ts`'s Redis
   options (`@upstash/redis` 1.41.0's `RetryConfig = false | {…}`), so a
   lost reply's retried `EVALSHA` cannot count one limiter hit twice. A
   store error stays the bare 500 it is today.
3. **`close` when the host gives up:** every way the host's `connected`
   ends in failure (no joiner by the deadline, `waitOpen`'s deadline, a
   bad answer, a server error) sends `close` with the token once, as
   cancel does; cancel still sends it once and never twice. A success
   sends none (today's behaviour).

Each with a unit test. Item 2 changes the deployed function, so Gate 4 is
a Vercel preview check by the human partner.

### Part 6: Type-checked tests

`tsconfig.json`'s `include` becomes `["src", "api", "test"]` (and
`vite.config.ts` stays type-checked, as it is today through a test's
import). Today that gives 58 errors: `firmware-worker.test.ts` 29,
`firmware-runtime.test.ts` 19, `shell.test.ts` 3, and one each in
`manifest`, `provisioner`, `radio-lock`, `redraw-bench`, `signal-client`,
`signal-handler` and `vite.config.ts`. Fix them in the tests (or the
config), never by loosening `strict`, `any`-casting a whole file, or
`@ts-nocheck`. One is real drift: `redraw-bench.test.ts`'s fake handle
lacks `softResetCount` and `rebootUs`. A `lib` bump (for `.at()`) is
allowed if the build target is unaffected; the builder says which.

### Part 7: Docs

- `docs/milestone-12-decisions.md`: rulings, measurements, the Share
  trace, dependencies added (if any, with versions and size cost), final
  suites, and the Milestone 13 backlog: everything open in Milestone 11's
  backlog that this milestone does not close, plus the fake sync station
  and Share's transfer as candidates.
- CLAUDE.md: current state (Sync ends on "No sync station in range"),
  test counts, `rom.rs`'s table, the provisioner's token, the data rule.
- `docs/firmware-emulator-notes.md`: a Milestone 12 section, and
  "Known limitations" updated for Sync.

## Constraints (carried; CLAUDE.md Rules)

- `cpu/` and `clock.rs` hold no ESP32-C3 or v392 constant: the address is
  in `rom.rs`, the algorithm in `cpu/`.
- ROM addresses from ESP-IDF v5.5.3's linker scripts, cited.
- No new server, service or network path; `iceServers: []`.
- Unmapped data access never panics; unmapped fetch traps.
- `emulator-wasm` stays logic-free; nothing new crosses it.
- Committed tests use only `firmware.bin`, the fake identities and the
  fake token.
- Python only via `local/.venv/bin/python -I`; Bun for JS.

## Gates (stop and ask the human partner)

1. **This spec's approval.**
2. **The plan's approval and the execution method.**
3. **After Parts 1–3 land:** the new Sync frames saved as PNGs under
   `local/m12-frames/`, for the human partner's eye, with the suite times.
4. **After Part 5:** a Vercel preview deploy, checked by the human partner
   (a pairing, and a host left to time out leaves no room behind).
5. **The finish:** merge, push and deploy are the human partner's call.

Also stop for anything CLAUDE.md's hard rules name: a hardware-compared
frame hash moving, a Milestone 11 pin moving, the release suite over
90 s, an override of a recorded decision, or anything needing the
physical badge.

## My calls (the human partner can override)

- The token value, and that it is one constant for all roles.
- The browser provisioner always sets the token (no opt-out in the UI);
  the Rust helper sets it only when a rung asks.
- The bitwise CRC loop over a `crc32fast` dependency.
- About 10 s for the fetch timeout.
- Every Provision sets the token, re-provisions included.
- The redraw benchmark skips the token (an option on
  `createProvisioner`), so its rows stay comparable.

## Left to the builder

- Exact rung names, budgets (+25 %, as the suite's convention), and
  whether the packing frame is pinned by hash or by `uitree`.
- Where the token-match test lives.
- The fetch-timeout mechanism (`AbortSignal.timeout`, or a combined
  signal with the host's cancel).
- How the trace gets its evidence (boot-probe, disassembly tools in
  `local/.venv`).

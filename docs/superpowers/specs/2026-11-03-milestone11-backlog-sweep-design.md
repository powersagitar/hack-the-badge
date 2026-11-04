# Milestone 11: the backlog sweep

Status: scope agreed in conversation with the human partner (2026-11-03).
main @ `7f3960d` (the Milestone 10 merge `e614117` plus the human's
`.gitignore` commit), and `origin/main` is the same: Milestone 10 is
pushed. **Approved by the human partner on 2026-11-04.**

## Why

The human partner chose to clear the whole Milestone 11 backlog
(`docs/milestone-10-decisions.md`, "Milestone 11 backlog", with its
Milestone 9 and Milestone 8 carry-overs). They set aside the large
features and dismissed the items that need work nobody will do. That
leaves two kinds of item:

- **Small faults**, mostly found in reviews, in the core, the browser
  and the signaling function.
- **Three places where the emulated reboot differs from the badge:**
  - the RTC timer restarts;
  - a reboot takes no time;
  - a CPU-only reset is treated as a full system reset.

  The human partner wants these fixed, and accepts moved pins if the
  spec names each one and says why.

Every item ends as one of three things:
- a fix with a test;
- a trace whose result is recorded;
- a written closing that cites who decided it.

## Decided by the human partner (2026-11-03)

- **Lead: the backlog sweep**, with every ride-along offered:
  - deploy hardening;
  - the hook-id-1 test fix;
  - the release-gap rung.
- **Clear the whole backlog**, not only Milestone 10's leftovers.
- **Saved for Milestone 12:**
  - Sync's empty-room path (the fake upload credential and the
    `crc32_le` stub);
  - Share's radio transfer.
- **Speed needs no work: it is normal in the prod build.** Closed
  without work:
  - the block cache, and with it the cheaper hook check;
  - mobile Safari's 0.0x–0.2x, which was a dev-build effect;
  - re-measuring desktop Chrome and Safari on v392;
  - the untagged decode-cache size;
  - tuning `WORKER_STEPS_PER_SLICE`.
- **The Vercel preview deploy works.** Checked by the human partner, which
  closes the unverified Vercel build (R-T10-3).
- **Upstash is on the free plan** (measured below).
- **Reflashing the badge to v392 is dismissed.** This closes two items:
  - no v392 frame is compared with hardware;
  - v392's early log timestamps are compared with no badge.

  The badge stays on v335.
- **The reset-fidelity fixes are in**, and pins may move if the spec
  names them. The human partner provides the v335 badge when it is needed.

## Measured (2026-11-03)

### A software reboot on the physical badge (v335)

Two captures, through the badge's own console command `reboot` (its help
text is "Restart the badge"), over `/dev/cu.usbmodem101` with pyserial
from `local/.venv`. The logs are in
`local/m11-badge/reboot_log_{1,2}.txt` (local only). Only the timestamps,
tags and segment sizes were read from them.

- **The reset reason** is `rst:0xc (RTC_SW_CPU_RST)`: `esp_restart()`
  ends in ROM `software_reset_cpu`, a CPU-only reset, as `crate::rom`
  entry 26 says.
- **The USB-Serial-JTAG link survived the reset.** The host port never
  disconnected, and the ROM banner and the bootloader's log arrived on
  the same open port. ESP-IDF's `esp_restart_noos` keeps the BBPLL on
  "so that USB Serial JTAG can log at 1st stage bootloader".
- **The timeline is identical in both runs, to the millisecond:**

  | Stage | Timestamp (ms, the bootloader's log clock) |
  |---|---|
  | 2nd-stage bootloader's first line | 24 |
  | `esp_image` segment 0 (map, 1,277,576 B) | 26 → 230 |
  | segment 1 (load, 33,128 B) | 230 → 236 |
  | segment 2 (map, 1,219,404 B) | 236 → 431 |
  | segment 3 (load, 27,260 B) | 431 → 436 |
  | segment 4 (load, 105,324 B) | 436 → 456 |
  | segment 5 (load, 32 B), then the image hash | 456 → 467 |
  | the app's `cpu_start` (first line) | 468 |

- **The cold boot looks the same.** The real cold boot log
  (`local/boot_log.txt`, `USB_UART_CHIP_RESET`) has the same timeline,
  within 1 ms per line (`cpu_start` at 468).
- **Checking a segment costs a steady amount per byte:** 0.160 µs per
  byte for the two mapped segments (204 ms and 195 ms), and about
  0.18–0.19 µs per byte for the loaded ones.
- **Scaled to v392's segments** (from `firmware.bin`'s header), the delta
  against v335 is:

  | Segments | Size change | Time |
  |---|---|---|
  | Mapped | +13,420 B | +2.1 ms |
  | Loaded | −24,416 B | −4.6 ms |

  So v392's reset-to-app time is about 466 ms, against v335's measured
  468.

### Upstash's free plan (upstash.com/pricing/redis, fetched 2026-11-03)

| Limit | Value |
|---|---|
| Commands | 500K per month |
| Rate | 10,000 commands per second |
| Data | 250 MB (the comparison table says 256 MB) |
| Bandwidth | 10 GB per month |
| Request size | 10 MB at most |

Today's limiter (`frontend/src/radio/signal-handler.ts`) costs about 7
commands per valid request:
- each request counts against three windows;
- each count is an `INCR` and an `EXPIRE … NX` in one pipeline;
- then the room's own `GET` or `SET`.

What that buys:
- **A typical pairing** (one create, about 10–30 polls, one answer) is
  about 70–210 commands, so the free month covers a few thousand
  pairings.
- **At the global cap** (5,000 requests per 10 min) the month's quota is
  spent in about 2.4 hours.

When the quota is gone, Upstash errors, and every request gets the bare
500 (the store-failure path). The 10K per second rate cannot be reached
under the caps.

### Facts read for the reset fixes

- **What `esp_restart_noos` resets.** Read from ESP-IDF v5.5.3,
  `components/esp_system/port/soc/esp32c3/system_internal.c`, before the
  CPU reset:
  - it disables the TG0 and TG1 watchdogs;
  - it arms the RTC watchdog for 1 s;
  - it disables the ICache;
  - it rewrites `GPIO_FUNC0..5_IN_SEL_CFG` to `0x30`;
  - `esp_system_reset_modules_on_exit()` pulses three reset groups:

    | Register | What it resets |
    |---|---|
    | `SYSTEM_CORE_RST_EN` | the WiFi, BT and SDIO blocks |
    | `SYSTEM_PERIP_RST_EN0` | `TIMERS`, `SPI01`, `UART`, `SYSTIMER` |
    | `SYSTEM_PERIP_RST_EN1` | `DMA` and the crypto blocks |

  - `rtc_clk_cpu_set_to_default_config()` puts the CPU back on XTAL.

  Nothing else is reset by software. In particular these keep their
  state: SPI2, I2C0, RMT, GPIO (except the six input selects), the
  interrupt matrix, USB-Serial-JTAG and APB_CTRL.
- **What the emulator does today.** `FirmwareRuntime::soft_reset`
  (`emulator-core/src/runtime.rs`) rebuilds every peripheral, whichever
  stub asked. It keeps the flash, `STORE0..7`, RTC fast memory, the
  panel, the console text and the host input, and drops queued serial
  input.
- **No pin depends on a reset before the launcher.** The walk to hacker's
  launcher crosses no soft reset (Connect's rungs assert
  `soft_reset_count() == 0` from the launcher). So
  `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged`
  cannot move from Part 1. The hardware-compared frames cannot either,
  since none of them follow a reboot.

## Goals

1. **The emulated reboot matches the badge's** where it shows (Part 1):
   - the RTC timer keeps counting across it;
   - it takes the measured time;
   - a CPU reset keeps what a CPU reset keeps;
   - `STORE4` is what the bootloader leaves.
2. **Two untraced step gaps are explained** (Part 2), and fixed if the
   emulator is at fault.
3. **Every open core and browser review item is fixed or closed** (Parts
   3 and 4).
4. **The signaling function is hardened** for its free plan and its users
   (Part 5).
5. **Hand-written code that a library does better is replaced** (Part 7),
   each new dependency named to the human partner.
6. **The docs carry every closing**, and the Milestone 12 backlog starts
   with Share and Sync (Part 6).

## Non-goals

- Share's radio transfer, and Sync's empty-room path (Milestone 12, by
  the human partner).
- Any speed work (the human partner: normal in the prod build).
- Hearing real badges (dropped, R-T11-2; it stays dropped).
- Reflashing the badge, or comparing any v392 frame or timestamp with
  hardware.
- **A cold power-on's boot time.** The shortcut boot still starts the
  app at 0 µs. Adding the 466 ms to a cold boot would move every pin for
  no visible gain. Recorded for the Milestone 12 backlog. My call.
- Lua sandbox mode: unchanged.

## Anti-goals (a failure even if it works)

- **A pin that moves and is not named below**, or one that moves for a
  reason other than the one given. Either is a bug in the change.
- **Any hardware-compared frame hash moving:**
  - the registered screen;
  - onboarding;
  - the launcher, before and after DOWN;
  - Dice: open, roll and animation;
  - Snake: open, after a turn and game over.
- **`execution_digest_is_unchanged` or `hacker_walk_totals_are_unchanged`
  moving.**
- **A guessed behaviour.** Every peripheral kept or reset across a CPU
  reset is cited from ESP-IDF v5.5.3 or the ESP32-C3 Technical Reference
  Manual. Where the two are silent, the peripheral is reset, as today,
  and the gap is recorded.
- Browser, peer or network input that can panic the emulator; a malformed
  peer packet that can crash a tab.
- A new server, relay, STUN/TURN or third-party service.
- A new dependency the human partner is not told about. They allowed new
  dependencies on 2026-11-03, on condition that each one is named.
- A swap that moves a pin. MD5 is MD5, and validation is validation:
  every replacement in Part 7 is behaviour-preserving.

## Success criteria

1. **The Part 1 rungs pass.** After a soft reset:
   - the RTC timer reads at least its pre-reset value plus `REBOOT_US`;
   - `emulated_time_us()` grows by `REBOOT_US` across the reset, with no
     steps run;
   - `STORE4` holds the bootloader's XTAL value;
   - queued USB serial input survives a CPU reset;
   - a `software_reset` (system) reset still rebuilds every digital
     peripheral.
2. **Every moved pin is listed** in `docs/milestone-11-decisions.md`, with
   its value before and after and its cause, and all of them are
   Part 1's. Each app that opens by rebooting still opens:
   - Scanner;
   - Sync;
   - Share;
   - Sponsors;
   - Workshop Quest.

   Each still shows the frame the human partner checked on v392. A moved
   frame hash goes to the gate, not to a re-pin.
3. **Each traced gap is recorded:** its cause, and a fix with a rung or a
   recorded "firmware behaviour, not ours".
4. **Each core and browser review item has a test or a recorded closing.**
5. **The full check list passes**, and release `boot_progress` stays
   under 90 s.
6. **The human partner's browser checks (Gate 3).** On the dev server:
   - a 429 message;
   - a cancelled room telling the joiner "room closed";
   - switching to Lua mode freeing the badge for another tab.

## Design

### Part 1: Reset fidelity (core)

All of it lives in `runtime.rs`'s `soft_reset`, plus the peripherals it
carries. `cpu/` and `clock.rs` stay chip-agnostic.

1. **The reboot's duration.**
   - **The constant.** A named constant in chip or app data, not in
     `clock.rs`: `REBOOT_US`, the measured time from the reset to the
     app's first instruction. **466,000 µs** (v335's 468 ms scaled to
     v392's segments, above). Its doc cites this spec's measurement and
     the two capture files by path.
   - **Host time.** A soft reset adds it to `time_base_us`, so
     `emulated_time_us()` jumps by `REBOOT_US` at the reset step.
     `run_until` across the jump stops at its target. If the target lies
     inside the jump, the run returns at the reset with time past the
     target, as an idle overshoot would. That keeps `run_until`'s
     contract of never going backwards.
   - **Both kinds of reset take it.** The 2nd-stage bootloader and its
     image check run after either one, and they are most of the 466 ms.
     Only the ROM's first 24 ms could differ by reset kind, and that was
     not measured. My call; recorded.
   - **The CPU's cycle counter** starts at 0 in the new boot, as today
     (the bootloader's cycles are not modeled). My call: modeling it
     would also mean the cold boot, which is a non-goal.
2. **The RTC timer keeps counting.** The RTC timer (`rtc_cntl.rs`)
   derives from the SYSTIMER tick count, which restarts with the new bus.
   - It gains an offset in RTC slow-clock ticks that `soft_reset` sets to
     two parts added together:
     - the old timer's value at the reset;
     - `REBOOT_US` converted at `RC_SLOW_HZ`.
   - A cold `reset()` clears the offset.
   - Cite TRM's reset chapter: the RTC domain survives a CPU reset and a
     core reset. Find the section and cite it; do not guess.
3. **A CPU reset keeps what a CPU reset keeps.** `software_reset_cpu`
   (`RESET_REASON_CPU0_SW`) and `software_reset` (`RESET_REASON_CORE_SW`)
   become different resets.
   - **A core (system) reset** stays what `soft_reset` does today.
   - **A CPU reset** carries each modeled peripheral that neither the
     reset itself nor `esp_restart_noos` resets. The list comes from the
     ESP32-C3 TRM's reset chapter (which modules a CPU reset clears) and
     `esp_restart_noos` (above).
   - **The builder's first task is that table**, one row per modeled
     peripheral, with kept or reset and the citation:
     - SYSTIMER, INTC, SYSTEM, APB_CTRL, GPIO and the 74HC165;
     - SPI2 and the ST7789, GDMA, TIMG, RTC_CNTL, USB-Serial-JTAG;
     - I2C0 and the SC7A20H, RMT, eFuse, the flash MMU, SPI1 and the
       flash.

     Guest-visible resets that `esp_restart_noos` performs by writing
     registers already run as guest code. The table records them, and
     the model honours them where the peripheral models the reset bits
     (`SYSTEM_PERIP_RST_EN0/1`). Where it does not, the builder resets
     that peripheral at the reset step, and records it.
   - **Expected to be kept** (to be confirmed by the table): the
     USB-Serial-JTAG, its queued host input included, as the badge's
     open port shows. Which resolves the provisioner's stale-prompt item
     (Part 4) one way or the other.
   - **The ROM and 2nd-stage bootloader also run on the badge** and
     touch peripherals (clocks, the flash MMU, the cache, the
     watchdogs). Bootloader effects on a modeled peripheral are cited
     from ESP-IDF v5.5.3's bootloader sources, and reproduced at the
     reset step where they leave state the app reads. The shortcut boot
     already does this for a cold boot (`STORE4`, the MMU).
4. **`STORE4` gets the bootloader's value.** After the carried
   `STORE0..7`, `soft_reset` writes `STORE4` as the shortcut boot seeds
   it (`crate::rom::RTC_XTAL_FREQ_REG`). This closes Milestone 9's
   R-T3-3 item.
5. **The diagnostics carry across a reset.** The bus's unmapped-access
   log, and the core's ROM-stub index drop count and last drop, carry
   over. A reset no longer empties them, and the log notes the reset (a
   marker entry or a per-boot counter; left to the builder). The module
   doc's "Everything else starts fresh … two diagnostics" paragraph
   changes to match.

**The pins that may move, all after a soft reset.** The builder measures
each before and after, and records it.
- The emulated times and step bands of every rung that opens an app by
  rebooting:
  - Scanner;
  - Sync;
  - Share;
  - Sponsors;
  - Workshop Quest;
  - the Scanner HOME response;
  - the Sponsors HOME response.
- `SCANNER_OPEN_HASH`, `SPONSORS_LOADED_HASH` and `SYNC_HOME_HASH` and
  the rest, only if what they show depends on RTC time or kept state.
  The gate shows any such move with the frame, and the human partner
  re-checks it by eye.

**The rest must not move:** every rung before the launcher, the walk,
the digest, and every rung with `soft_reset_count() == 0`.

**Gate 1 (after Part 1's table, before code):** the human partner sees
the kept-or-reset table and its citations. **Gate 2 (after Part 1's
code):** the moved-pin list with before and after, and any moved frame.

### Part 2: Two traces

Both use scratch probes, never committed; boot-probe and copies of
`boot_progress.rs` are fine.

1. **v392's `PROV FAIL` takes 3.0M busy steps before the next prompt**,
   against 0.2M on v335 (Milestone 9). Trace what runs: hot PCs and the
   functions behind them, from the disassembly. If it is the firmware's
   own work (an NVS or LittleFS write, a retry), record it with the
   evidence. If it is an emulator effect (a spin on an unmodeled
   register, or an unmapped access), fix it, with a rung.
2. **The frontend and Rust provisioning hosts reach the launcher 740,001
   steps apart** (Milestone 7). Diff the two hosts' input timelines:
   - chunk pacing;
   - the first-prompt wait;
   - the press schedule.

   Name the difference. If one host is wrong, fix it. If both are valid
   (different but legal pacing), record that and add a test that pins
   the frontend's pacing against `tests/common/provision.rs`.

### Part 3: Core review items

Each gets a test or a doc fix:

1. **The test hook id 1** (`runtime.rs`, `hooked_count_loop`) collides
   with the radio's `HOOK_INIT`.
   - Give the tests an id the radio does not know.
   - Add a test-only way past the radio's `debug_assert` (a
     `#[cfg(test)]` dispatch path, or a test-only empty `Radio`).
   - The radio's real path is unchanged.
2. **R-T6-2's ordering risk gets a test.** The test covers a report
   popped in `send_packet` ahead of a command reply, and checks that
   `host_rcv_pkt` cannot block the idle task.
3. **`deliver_rejects_malformed_bytes` feeds a scanning guest**, so a
   rejection is told apart from not scanning.
4. **`boot_progress.rs`'s radio section header** is corrected: the bump
   runs one blocking 100 ms slice (R-T8-3).
5. **The app-hook tests and doc:**
   - `set_app_hooks` dropping a pass-through;
   - a clone carrying a pending hit;
   - a clone carrying a pass-through;
   - `hook_step`'s doc says a WFI-parked core returns before the hook
     check.
6. **The eFuse module:**
   - an `EFUSE_RANGE` disjointness test;
   - `efuse.rs` cites the MAC's builder call site, `0x42010344`.
7. **The HCI model:**
   - **The fuzz test** reaches the report path and varies every address
     byte.
   - **LE Set Extended Advertising Enable** rejects duplicate handles
     with 0x12 (Invalid HCI Command Parameters).
   - **LE Set Extended Scan Enable** rejects Filter_Duplicates 2 with
     Period 0 (0x12).

   Both rules are cited from the Core spec's command descriptions
   (Vol 4, Part E), by section number. If NimBLE on v392 sends either
   form, no radio pin may move; stop if one does.
8. **The WASM `soft_reset_count`** is tested after a reset, not only at
   0, and `cpu-wasm.test.ts` joins `tsconfig` (type-checked).
9. **The release-gap rung** (ride-along). Two DOWNs separated by
   `RELEASE_GAP_US` (40 ms) move the launcher's cursor twice, pinned in
   `boot_progress`.

### Part 4: Browser items

1. **The badge lock is released** on a switch to Lua mode and when the
   firmware runtime fails to start. Another tab still needs "Run here" to
   start. My call: no automatic start in waiting tabs.
2. **Provision in a tab without the badge boots no Worker.** It shows the
   badge notice, and the canvas behind the notice is cleared (not the Lua
   sandbox's last frame).
3. **Advertisements from the Worker:**
   - changed advertisements are coalesced to R-PF-3's 10 Hz bound, the
     latest kept;
   - `macLocked` is set only when `setFactoryMac` returns `true`.
4. **`hub.bump()` is gated on the running state.** No `{t:"bump"}` goes
   to links while the local runtime is stopped or hidden. This revisits
   R-T10-1, which said to revisit if confusing; the human partner put
   the item in scope. Called out at Gate 3.
5. **A joiner's link joins the hub on the channel's `open` event**, not a
   1 s poll (`waitOpen`), so a bump right after it opens is not lost.
6. **A cancelled host room closes.**
   - **A new `close` op** (POST, the host's room, a write under the
     per-client write cap) deletes the room's keys. It is the only
     addition to `/api/signal`'s surface.
   - **The host's cancel sends it**, best effort.
   - **A joiner who arrives after the cancel** gets 404 at once, and the
     shell says "room closed or expired", not a 120 s wait. A joiner who
     had already answered before the cancel still waits out `waitOpen`.
     Covering that would need a new poll, which costs commands. My call;
     recorded for the Milestone 12 backlog.
   - Knowing a room code is today's only gate on joining. So the host
     proves it owns a room with a random token returned by `create`
     (stored with the offer). My call: otherwise any joiner could close
     any room.
7. **Small config fixes.** `vite.config.ts` imports
   `./src/radio/signal-handler.ts` with its extension. The dev
   middleware stops reading a body past `MAX_BODY_BYTES`.
8. **`shell.test.ts`'s "shows a room code"** asserts through the shell's
   real update path, not text it set itself.
9. **The tap queue (Milestone 8):**
   - **One button fed by two sources** (a key and the pad) gets a
     per-source hold count, so the pad's mouseup no longer releases a
     key-held button.
   - **A queued, not yet pressed tap survives `stop()`** and plays when
     the runtime starts again; its later release is honoured. My call:
     the alternative, dropping it, is today's behaviour.
   - **`RELEASE_GAP_US` stays 40 ms** (R-M8-4 stands; the human partner
     called the lag tiny). Closed in writing.
10. **The provisioner's first-prompt scan** matches only console text
    written after the provision started. That stops it taking a stale
    `badge> ` from before a reboot. Part 1 may keep queued input across
    a CPU reset, so a provision typed across a reboot still reaches the
    new console.
11. **The status line can show a pre-reset state for up to 250 ms.** The
    Worker posts a status at once when a soft reset happens, outside the
    throttle; it needs one new field in the status message.

12. **The reboot shows in the browser** (the human partner, 2026-11-04).
    The Worker's pacing measures from the current emulated time, so a
    soft reset's `REBOOT_US` jump would otherwise be absorbed at once.
    Instead:
    - the runtime reports the jump;
    - the pacing carries it as wall-clock debt, so the screen holds for
      about 466 ms, as the badge's does;
    - the speed readout leaves the jump out;
    - a tap hold or release gap in progress is measured in wall time
      across the jump, so it is not cut short.

    Checked at Gate 3, by opening Scanner.

### Part 5: Signaling

1. **A 429 gets a message.** `signal-client.ts` maps it to "Busy, try
   again in N min", using `Retry-After` and rounding up. The same goes
   for a 429 during host or join polling.
2. **IPv6 clients are keyed by their /64.** The per-client rate keys use
   the /64 prefix for an IPv6 address, and the whole address for IPv4.
   It stays the first `x-forwarded-for` entry, which is sound on Vercel.
3. **The limiter moves to `@upstash/ratelimit`, over `@upstash/redis`**
   (Part 7). This answers the human partner's question of 2026-11-03.
   - **Each window becomes one fixed-window limiter**, with the same caps
     and windows (R-T10-8 unchanged). One script call each, so
     about 4 commands per request against today's 7. The builder
     confirms how Upstash bills a script call, and records the real
     count.
   - **`UpstashStore` becomes `@upstash/redis`'s `Redis`** (`fromEnv`
     reads the same `KV_REST_API_URL` and `KV_REST_API_TOKEN`).
   - **Dev and tests need no Upstash.** `MemoryStore`, and an in-memory
     limiter behind the same small interface, keep the dev server and
     `bun test` working without credentials.
   - **R-T10-3's "signal-handler imports nothing" ends.** Vercel bundles
     the function's dependencies. Re-checked on the human partner's next
     preview deploy (Gate 3).
   - **The global cap** stays at 5,000 per 10 min (toy scale; no
     escalation). The free month's exhaustion time is recomputed and
     recorded.
4. **When the store fails** (including a spent quota), the client shows
   "Signaling unavailable" instead of the bare HTTP status. The server
   still answers a bare 500 (R-T10-3 unchanged).

### Part 7: Hand-written code that a library does better (the human partner, 2026-11-03)

The human partner asked for a whole-repo sweep. Each adoption is named
to them, with its version, when it lands, and recorded in the decisions
doc. The survey (2026-11-03) covered the Rust crates (`emulator-core`
and `emulator-wasm`; today the only dependencies are `wasm-bindgen`,
plus `png` as a dev-dependency) and the frontend (`fengari`,
`fengari-interop`, `vite`, `typescript`).

**Adopt:**

| Where | Today | Library | Why it fits |
|---|---|---|---|
| `signal-handler.ts`: the limiter | Four `INCR`/`EXPIRE NX` pipelines | `@upstash/ratelimit` | Tested fixed windows, one script call each; fewer commands on the free plan (Part 5) |
| `signal-handler.ts`: `UpstashStore` | REST over `fetch` | `@upstash/redis` | The official client, which the limiter needs anyway |
| Untrusted input: signaling request bodies (`signal-handler.ts`), peer messages (`hub.ts`'s parser) | Hand-written type guards | `valibot` | Schemas at the network and peer boundary; small and tree-shaken. A rejection still never throws past the boundary, and is still a 400 or a drop |
| `md5.rs`: the 64 rounds | Hand-written compression function | `md-5`'s `compress` (RustCrypto) | Keeps `md5.rs`'s guest-memory Plumb context and its byte-exact writes; only the block compression is the crate's. Pure Rust, builds for wasm32 |
| HCI fuzz test, `runtime.rs`'s serial fuzz test | `XorShift`, hand-picked seeds | `proptest` (dev) | Generated inputs with shrinking. Fixes Part 3.7's coverage item (the report path, every address byte) |
| `riscv_integration.rs`: the fixture loader | Shells out to `rust-objcopy`/`llvm-objcopy` | `object` (dev), reading the ELF's loadable segments | Drops the `llvm-tools-preview` requirement |
| `shell.test.ts` and its `FakeElement` | A hand-written fake DOM | `happy-dom` (dev; a `Window` per test file, not the global registrator, which would change globals other tests detect) | A real DOM for the shell's tests. Part 4.8's "asserts through the real update path" needs one |

**Keep, recorded with the reason:**

| Where | Why it stays |
|---|---|
| `lifecycle.ts`'s `manifest.cfg` parser | It mirrors the badge firmware's own format (`key=value`, `#` and `;` comments), not INI, whose quoting and sections the badge does not have |
| `cpu/decode.rs`, `encode.rs`, the decode cache | The core's hot path, pinned byte-for-byte; no crate decodes RV32IMC into this core's form without a speed and equivalence risk |
| `hci.rs`'s splitmix64 (LE Rand) | Emulated behaviour: a crate's generator would change the values NimBLE sees |
| The tests' FNV-1a | Ten lines, and the pins are its exact values |
| `peer-link.ts`'s base64url | `btoa`/`atob` with two replaces; `Uint8Array.toBase64` is not in every target browser yet |
| The deflate-raw codes | Already the platform's `CompressionStream` |
| `bench/safari-bench.ts`'s WebDriver calls | About 20 lines over `fetch`, for a dev-only script; a WebDriver client would be far larger |
| The Worker protocol and its client | A frame stream with acks and throttling, not RPC; `comlink` would not fit |
| `boot-probe`'s argument parsing | Three flags with strict errors; a parser crate saves almost nothing |
| `fake-rtc.ts`, the fake Web Locks, the fake 2D context | Test doubles for platform APIs that no library fakes better |

**The rule for each swap:**
- every suite passes unchanged and no pin moves;
- `emulator-core` keeps no wasm-bindgen dependency;
- `bun run build`'s bundle size is recorded before and after, and
  `valibot`'s cost is noted.

**If a swap turns out worse** (a bigger bundle than the gain warrants, a
behaviour difference, a crate that won't build for wasm32), it is
reverted and recorded under "Keep".

### Part 6: Docs

- **`docs/milestone-11-decisions.md`:**
  - the rulings;
  - the measurements above;
  - Part 1's kept-or-reset table and moved-pin list;
  - Part 2's findings;
  - every written closing, each citing who decided it;
  - the Milestone 12 backlog. It starts with Share's transfer and
    Sync's empty-room path, then the cold-boot time, then anything this
    milestone leaves open.
- **`docs/firmware-emulator-notes.md`:** "Timing model" (the reboot's
  time) and "Known limitations" (the reset), plus a Milestone 11 section.
- **`CLAUDE.md`:**
  - "Current state";
  - the test counts;
  - the runtime and RTC_CNTL lines in the architecture map;
  - the signaling API's `close` op.
- **The human partner's notes from the backlog that need only an
  acknowledgement** are recorded as acknowledged. One is R-T9-2's
  Shake-timing change from Milestone 6 behaviour.

## Constraints (carried; CLAUDE.md Rules)

- **The core.** `cpu/` and `clock.rs` stay chip-agnostic. `FirmwareBus`
  keeps concrete named fields and its fast-path order. `emulator-wasm`
  stays one-line passthroughs. Unmapped data access never panics, and
  an unmapped fetch traps.
- **Citations.** Register layouts come from ESP-IDF v5.5.3 headers, ROM
  addresses from its linker scripts, and app-image addresses from cited
  disassembly. Reset behaviour comes from the TRM or ESP-IDF source,
  cited.
- **Local data.**
  - Nothing from `local/` is committed or quoted.
  - The reboot captures are cited by path only, and their identity lines
    are never read out.
  - Committed tests use only `firmware.bin` and the committed fake
    identities and MACs.
- **Servers.** Only `/api/signal` on Upstash, with `iceServers: []`. No
  new dependency without asking.

## Gates (stop and ask the human partner)

1. **Part 1's table**, before any reset code.
2. **Part 1's moved pins and any moved frame**, before Part 2.
3. **The browser checks**, after Parts 4 and 5:
   - the reboot's pause, opening Scanner;
   - the 429 message;
   - room closed;
   - the lock freed on a switch to Lua mode;
   - the bump gate.
4. **A Part 2 trace that points at a change bigger than a small fix**,
   for example a new peripheral model.

Plus the standing ones:
- any pin that moves and is not named in Part 1;
- any hardware-compared frame moving;
- release `boot_progress` over 90 s;
- any override of a recorded decision;
- any deploy, push or merge;
- anything needing the badge.

## Questions for the human partner (answered at approval)

1. **The rate limiter.** *Answered (2026-11-03): dependencies are
   allowed, so (c), `@upstash/ratelimit` (Part 5.3).* Today's limiter
   costs about 7 commands per
   request. That is about 70–210 per pairing, and a few thousand pairings
   a month on the free plan. At the global cap the month is spent in
   about 2.4 h. The options:
   - (a) keep it, and record the closing;
   - (b) one hand-written Lua script (`EVAL`) counting all three windows,
     about 2 commands per request, with no dependency;
   - (c) `@upstash/ratelimit`, a new dependency, which ends R-T10-3's
     "imports nothing";
   - also, with any of them, lower the global cap so a month cannot be
     spent in hours.
2. **Whether the reboot's 466 ms shows in the browser.** *Answered
   (2026-11-04): it shows, as on the badge (Part 4.12).* The browser's
   pacing measures from the current emulated time, so today the jump
   would be absorbed at once:
   - the screen would not pause;
   - the speed readout would spike;
   - a tap hold or release gap in progress would be cut short.

   Either it shows as on the badge (the Worker carries the jump as
   wall-clock debt and the speed readout ignores it, which is new browser
   work checked at Gate 3), or it stays core-only and the browser
   behaviour is recorded.

## My calls (the human partner can override)

- **`REBOOT_US` = 466,000 µs**, scaled from v335's measured 468 ms rather
  than taken raw.
- **The reboot's time goes to host time and the RTC only.** The CPU's
  cycle counter and SYSTIMER start at 0, as on the badge (SYSTIMER is
  reset by `esp_restart_noos`; the cycle counter by the CPU reset).
- **A cold power-on keeps 0 µs boot time** (non-goal; Milestone 12
  backlog).
- **A peripheral where the TRM and ESP-IDF are both silent is reset**, as
  today, and the gap is recorded.
- **Waiting tabs do not auto-start** when the badge is released.
- **A queued tap survives `stop()`.**
- **`close` needs the host's token from `create`.**
- **The diagnostics carry across a reset**, with a reset marker.

## Left to the builder

- The RTC offset's representation, and where `REBOOT_US` lives
  (`rom.rs`, `mem/soc.rs`, or a boot-time constant near the shortcut
  boot).
- The reset marker's shape in the unmapped log.
- The test structure, beyond the rungs in the success criteria.
- The exact wording of the shell's new messages.

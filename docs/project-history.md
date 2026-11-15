# Project history

Fourteen milestones, 2026-09-20 to 2026-11-14, and the README on
2026-11-15. The git history has 287 commits. Each milestone from 3 on
was a branch (or a worktree per task, in Milestone 14) merged into
`main`; each ends with a decisions doc that carries the next
milestone's backlog.

The driver of nearly every milestone was a gap the human partner felt
using the emulator next to a physical badge. The "Why" line of each
entry below says what it was.

| # | Dates | Ends at | Outcome |
| - | ----- | ------- | ------- |
| 1 | 09-20 | `8493950` | A Lua sandbox with a `badge.*` API |
| 2 | 09-20 – 09-22 | `8bd7e03` | An RV32IMC core and ESP32-C3 basics; the real firmware runs 2M+ instructions |
| 3 | 09-22 – 10-06 | `c0e98d2` | The first real frame: the boot splash |
| 4 | 10-06 – 10-07 | `1139723` | My Badge's first-run screen |
| 5 | 10-07 – 10-09 | `3af5bba` | Provisioning through the console; onboarding; the launcher; Dice |
| 6 | 10-09 – 10-20 | `6cd0a6e` | A calibrated clock; Snake playable; wall-clock pacing |
| 7 | 10-21 – 10-22 | `208a69d` | Core throughput (3.3–3.8x in WASM, 5.2x native); the Web Worker host |
| 8 | 10-22 | `ea9c361` | The browser's tap queue (no dropped double taps) |
| 9 | 10-22 – 10-28 | `0d56ba5` | Official firmware v392; the software reset |
| 10 | 10-29 – 11-03 | `e614117` | The fake BLE controller; two badges pair over WebRTC |
| 11 | 11-03 – 11-10 | `35a6c37` | Backlog sweep; a reboot takes 466 ms; the CPU-reset table |
| 12 | 11-10 – 11-11 | `065ad00` | Sync to "No sync station in range" |
| 13 | 11-11 – 11-12 | `13511dc` | Skip setup |
| 14 | 11-12 – 11-14 | `957b335` | The page redesign: board photo, lanyard, LEDs, power switch, CONSOLE.LOG |
| — | 11-15 | `9baa6c2` | README with screenshots and demos |

All dates are 2026. `main` is pushed and matches `origin/main`.

## Milestone 1: a Lua sandbox (2026-09-20)

The first plan was to extract the badge's built-in apps as Lua
(`main.lua` plus `manifest.cfg`, like sideloaded apps) and run them in a
browser Lua VM with a re-implemented `badge.*` API on a canvas. Built
and tested, then made moot by Milestone 2's forensics. It survived as
the page's "Lua mode" until Milestone 14 removed it.

## Milestone 2: emulate the processor instead (2026-09-20 to 09-22)

A read-only esptool dump of a real badge showed that the built-in apps
are native RISC-V code statically linked into one ESP-IDF image, with no
Lua bytecode anywhere (notes, "Why real firmware is emulated instead of
extracted"). So the project became an ESP32-C3 emulator: a generic
RV32IMC core, the memory map, a "shortcut boot" straight from the app
image, the interrupt matrix, SYSTIMER, GPIO with the 74HC165 button
register, SPI2 with ST7789 framebuffer reconstruction, and the first
mask-ROM high-level stubs. A whole-branch review fixed interrupt gating
and RVC decoding. A full flash dump that briefly landed on the branch
held personal data and was purged from history before merging (notes,
"Data-handling note"). The web app moved into `frontend/`.

## Milestone 3: first pixels (2026-09-22 to 10-06)

From a stall inside `rtc_clk_cal()` to the firmware's first real ST7789
frame, one stall at a time: TIMG's RTC calibration, the RTC timer, ROM
`memcpy`/`itoa`/`strcat`/`ets_printf`, the eFuse SPI config chain, XIP
flash-cache pages, a guest-executed ROM `qsort`, ROM layout data,
libgcc helpers and newlib init hooks. The `boot_progress` ratchet and
the `boot-probe` diagnostic date from here. The stall log is in the
notes, "History: the stall-by-stall log".

## Milestone 4: the first-run screen (2026-10-06 to 10-07)

The flash MMU, `load_partitions()`, console output after the scheduler
starts, SPIMEM1 flash writes (littlefs formats `storage`), I2C0 with no
device, and RMT as a zero-latency TX engine let the first app launch.
The finish line was My Badge's "Not registered yet" screen, compared
with a factory-reset badge.

## Milestone 5: provisioned to the launcher (2026-10-07 to 10-09)

USB-Serial-JTAG receive and its interrupt, so a host can type into the
firmware's own console: `put /littlefs/identity.json` and `prov apply`.
Eleven obviously fake identities, one per role, were committed with
the organizers' permission. The SC7A20H accelerometer let onboarding's
"Shake it!" page pass. The cycle counter and RNG made `esp_random()`
deterministic. The browser gained a Provision control. Dice opened as a
stretch. The human partner compared the frames with the badge.

## Milestone 6: Snake (2026-10-09 to 10-20)

Why: Snake ran at the wrong speed. A drift-free clock converts steps to
16 MHz SYSTIMER ticks at whatever CPU frequency the firmware selects
(SYSTEM's clock registers and the ROM frequency pair became stateful).
The RTC timer counts SYSTIMER time. Snake moves every 170 ms, exactly
as the badge does (stopwatch-checked against hardware, 2026-10-20). The
browser paces the emulator to wall-clock time.

## Milestone 7: core throughput (2026-10-21 to 10-22)

Why: busy redraws (the launcher's HOME) were slow. Pure performance
work under the equivalence rule (no emulated behaviour may change):
release `opt-level = 3`, interrupt routing over pending bits, a word
fast path on the bus, a ROM-stub range precheck, and a decoded-
instruction cache. The emulator moved into a Web Worker. Benchmarks and
profiles are in its decisions doc. A block cache was shelved.

## Milestone 8: double taps (2026-10-22)

Why: "if i double tap the second tap is dropped." A per-button tap
queue in the browser (press hold, release gap). No core change. Approved
in chat; no spec or plan.

## Milestone 9: v392 and the software reset (2026-10-22 to 10-28)

Why: "not all built in apps open … it just freezes the whole emulator."
The emulated image moved from the badge's dumped v335 to the official
v392 (`firmware.bin`, published with the organizers' approval) and the
tests were re-pinned. v392 enters five apps by rebooting, so the
emulator gained a software reset. Fourteen of the 16 launcher apps
opened; Connect and Sync stopped at the Bluetooth ROM.

## Milestone 10: the badge radio (2026-10-29 to 11-03)

A fake LE-only HCI controller replaces the mask-ROM BLE controller at
bt.c's VHCI seam, through app-image hooks at v392's addresses and a
guest trampoline in free ROM space. Each badge gets its own eFuse
factory MAC. Browsers link over WebRTC data channels with no ICE
servers, signalled through `/api/signal` (Upstash Redis on Vercel).
All 16 apps open, and two badges pair in Connect: "SYNCED" on both, in
a rung and on two real devices (2026-11-03).

## Milestone 11: the backlog sweep (2026-11-03 to 11-10)

The human partner chose to clear the backlog. A reboot now takes the
badge's measured 466 ms, the RTC timer counts through it, and a CPU
reset keeps what the TRM and ESP-IDF say the badge keeps (the cited
table). Fuzz tests on the peer-input paths, valibot schemas, the
`@upstash/ratelimit` limiter, signaling cost work, and the review
faults. Items nobody would do were closed in writing.

## Milestone 12: Sync to "No sync station in range" (2026-11-10 to 11-11)

A ROM `crc32_le` stub lets NVS store the upload credential, and both
provisioning hosts set the fake `FAKE_UPLOAD_TOKEN`. Sync then packs the
badge, asks for a station for about 20 s, gives up and returns home.
Share's radio transfer was traced working between two emulated badges
(in a probe; not reachable in the browser). Three signaling faults were
fixed and the test files type-checked.

## Milestone 13: Skip setup (2026-11-11 to 11-12)

A button, enabled only while the onboarding app is on screen, types
`config system onboard_build 2147483647` and `reboot`, landing on the
registered screen. The console gained absolute byte offsets, and the
provisioner fails at once if the badge reboots mid-provision. Nine
backlog ride-alongs.

## Milestone 14: the page redesign (2026-11-12 to 11-14)

The human partner's design. The page is the real firmware only: a photo
of the board (rebuilt from the organizers' public orthographic scan,
with per-corner radii) hanging on a matter.js lanyard, in React with
shadcn/ui on Tailwind v4. The swing's proper acceleration feeds the
SC7A20H, so shakes and bumps come from motion (or a phone's
DeviceMotion). A passive WS2812 decoder gives the LEDs' real colours. A
power switch (`power_cycle()`) keeps the flash. CONSOLE.LOG streams the
USB serial output. The fixtures were renamed to historical figures and
re-pinned. Fixes after the browser check: iOS's inverted axes (R21), a
fling that flipped the badge over its anchor, the self-test's
accelerometer page (a missing libgcc ROM helper), and a final review
wave. The Lua sandbox, button pad, Shake/Bump buttons, manual pairing
and links list were removed.

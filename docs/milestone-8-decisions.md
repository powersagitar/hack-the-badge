# Milestone 8: decisions and the Milestone 9 backlog

Milestone 8 was planned as throughput round two (a block cache and/or
cached peripheral pending state, aiming at HOME 1.0x in the browsers).
In brainstorming the human partner described the gap they actually feel:
the launcher is "a tiny little sluggish when pressing
up/down/left/right too many times", and "if i double tap the second tap
is dropped". The milestone became one browser-side fix for the dropped
tap. There is no spec or plan: the human approved the design in chat as
a small change. `docs/firmware-emulator-notes.md`'s "Milestone 8: double
taps" holds the measurement history. Merged to local `main` as
`ea9c361` (fix commits `f1e1641`, `a5ad9b7`, `f85f95b`).

Outcome: in the human's side-by-side check against the badge, "all taps
registered and no sluggishness anymore except the tiny ones caused by
the tap queue". No `emulator-core` or `emulator-wasm` file changed, so
every pin holds by construction (release `boot_progress` 64 pass, 1
ignored; `bun test` 182 pass).

## Measurements (at `96fe58c`, before the fix)

Re-run because the human did not see the launcher at 0.6x. The
benchmark's HOME is the registered screen to the launcher (entering
it); moving inside the launcher is DOWN.

| Redraw | Bun (WASM) | Native |
|---|---|---|
| HOME (enter the launcher) | 0.614x | 1.103x |
| DOWN (move inside it) | 0.977x | 1.436x |
| Onboarding | 1.224x | 1.768x |

Walk throughput: 41.2M steps/s in Bun's WASM, 70.5M natively. Bun's
factors are its `pin_factor` column, the one comparable with the
Milestone 7 rows. Navigation was already at about the badge's speed, so
the sluggishness was input, not core speed.

Release-gap probe (native, scratch tests from the launcher checkpoint,
not committed): two 100 ms DOWN presses with no release between them
move the selection once (the bug reproduced in the firmware, not only
the browser); with a gap of 15 ms or less, once; 20 ms or more, twice,
at all ten scan phases tried (start offsets 0 to 9.9 ms in 1.1 ms steps)
for gaps of 20, 25, 30 and 40 ms.

## Rulings

- **R-M8-1, rescoped to the dropped tap.** The human's complaint was a
  dropped second tap, and the launcher's DOWN already ran at 0.97x. The
  block cache and cached pending state were shelved, not dropped (see
  the backlog's lead). If wrong: entering the launcher (HOME) still
  plays at about 0.6x in the browsers.
- **R-M8-2, sized as a small change.** No spec, plan or ledger; one
  implementer (the orchestrator, test-first), a subagent review (opus)
  and a scoped re-review (sonnet), then the human's browser check.
  The human approved the in-chat design ("lgtm") and asked for subagent
  review.
- **R-M8-3, a per-button tap queue.** A second press inside the first
  press's deferred release used to cancel the release, so the firmware
  saw one long press. Now each tap is held at least `PRESS_HOLD_US`
  (100 ms, unchanged), the button stays released at least
  `RELEASE_GAP_US` before the next queued press, a press while the last
  tap is still held is ignored (key repeat), and taps on different
  buttons are independent. If wrong: a burst plays back slightly later
  than it was tapped (the "tiny" lag the human noticed).
- **R-M8-4, `RELEASE_GAP_US` = 40 ms**, twice the 20 ms measured
  minimum. A double tap takes 240 ms of emulated time. If wrong: a
  firmware state that scans more slowly than the launcher could still
  merge taps; a shorter gap would shave the queue's lag.
- **R-M8-5, `MAX_QUEUED_TAPS` = 8**, the held tap included; later
  presses are dropped. It bounds the replay after a burst to about
  1.1 s. If wrong: mashing faster than about 7 taps/s for over a second
  loses taps.
- **R-M8-6, `stop()` and `reset()` flush the queue** but keep a button
  the user is still holding down in the emulator. `stop()` keeps the
  last release time (emulated time does not move while stopped, so a
  tap right after a hidden-tab or mode-switch resume still waits the
  gap; review finding); `reset()` clears it and treats a held tap as
  held long enough (emulated time restarts at 0).
- **R-M8-7, the test that pinned the merge was replaced.** "a re-press
  before the deferred release cancels it" encoded the bug; it is now "a
  re-press before the deferred release is a second tap, after
  `RELEASE_GAP_US`". A frontend unit test, not an emulator pin.

## Milestone 9 backlog

The Milestone 8 backlog (`docs/milestone-7-decisions.md`), carried word
for word under the same headings except where marked, plus what
Milestone 8 found. The human named no lead: ask.

### Shelved lead: a block cache (HOME is short of the badge)

Unchanged from Milestone 8's lead (read it in
`docs/milestone-7-decisions.md`, "Lead: a block cache"), but shelved by
R-M8-1: after the tap fix the human felt no sluggishness. Bring it back
only if the human finds entering the launcher (HOME, about 0.6x in the
browsers) or another screen slow. Measure the basic-block length
distribution on the hot path before designing one.

### Found in Milestone 8

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

### Found in Milestone 7

- The decode cache at `1 << 16` entries read about 1.5 % faster on HOME
  than `1 << 14`, measured with the `pc` tag, for 1 MiB per runtime and
  per clone (R-T7-4); not re-measured untagged.
- `WORKER_STEPS_PER_SLICE` (400,000) was never tuned in a browser
  (R-T8-2); a measured long-slice input lag would argue for 200,000.
- The frontend and Rust provisioning hosts reach the launcher 740,001
  steps apart (R-T2-4); the cause was not traced.
- The status line can show a pre-reset state for up to one
  `STATUS_INTERVAL_MS` (250 ms) after a reset or provision (R-T8-9).

### From the final review

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

### Speed and timing

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

### Apps and UI

- Other built-in apps are untested; only Dice and Snake are checked.
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

### Peripherals

- RMT transmissions are consumed but not decoded, so the LEDs (including
  `prov apply`'s green flashes and role colour) are never shown.
- NFC is not modeled (the self-test waits for a card forever).
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

### Build and tests

- The release suite's wall time is one chain (hacker's walk to Dice,
  about 487M steps); only a faster emulator shortens it.
- Test minors left from Milestone 6's reviews: `snake_eats` could assert
  growth of exactly one cell (`after * 3 == at_open * 4`) and that its
  150 ms frame is `SNAKE_OPEN_HASH`, and its `SNAKE_DOWN_SLOT` = 4
  duplicates `NAV_SLOT` without a cross-reference;
  `run_until_a_past_target_runs_nothing` uses target 0 (always past),
  where a mid-run target would test it properly; the
  `DEFAULT_STEPS_PER_FRAME` test restates the constant;
  `snake_moves_on_its_own`'s exact gap pin is as fine as `MOVE_SAMPLE_US`
  (2 ms).

### Carried from Milestones 3 and 4 (untouched)

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

Resolved from the Milestone 8 backlog in Milestone 8: none of its listed
items; the dropped double tap was found and fixed (R-M8-3).

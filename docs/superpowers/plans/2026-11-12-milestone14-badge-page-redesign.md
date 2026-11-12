# Milestone 14: Badge Page Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the page with the design canvas's hanging badge, which is real-firmware only. A matter.js swing (or the phone's own motion) drives the emulated accelerometer, the power switch power-cycles the badge, LEDs show the decoded WS2812 output, and CONSOLE.LOG streams real serial output.

**Architecture:**
- **Emulator.** It gains a passive WS2812 decoder on the RMT transmitter, a flash-keeping `power_cycle()`, and the renamed fixtures.
- **The Worker** gains `accel`, `powerCycle`, `leds` and `console` messages.
- **The page** is rebuilt in React, Tailwind and shadcn/ui. Non-visual state lives in a store the components read with `useSyncExternalStore`. The physics loop and per-frame painting write to refs, never through React renders.
- **Motion** comes through a `MotionSource` interface: the matter.js lanyard and DeviceMotion are its two implementations.

**Tech Stack:**
- Rust (`emulator-core`, `emulator-wasm`).
- TypeScript with Bun.
- React 19, Tailwind CSS v4 (`@tailwindcss/vite`) and shadcn/ui (Button, Select, Input, ScrollArea).
- matter-js.
- Google Fonts: Space Grotesk and JetBrains Mono.

**Spec:** `docs/superpowers/specs/2026-11-12-milestone14-badge-page-redesign-design.md`. Read it first. Section names below refer to it.

## Global Constraints

- Everything in `CLAUDE.md` holds. Every task's requirements include it.
- **Tooling:**
  - Bun only; never npm, yarn, pnpm or node.
  - Python only via `local/.venv/bin/python`.
- **Dependencies:**
  - Pin every new one exactly (no `^`).
  - Record each one (package, exact version, why) in `docs/milestone-14-decisions.md`'s "Dependencies added" as you add it.
  - `emulator-core` takes no wasm-bindgen.
  - `emulator-wasm` stays one-line passthroughs. A `Vec` return is owned.
- **Core rules:**
  - `emulator-core/src/cpu/` and `clock.rs` get no chip or board constants.
  - WS2812 lives in `peripherals/`.
  - Register layouts are cited from ESP-IDF v5.5.3.
  - Unmapped data access never panics. Peer, browser and network input never panic or crash a tab.
- **Pins:**
  - The LED decoder is observation only. Nothing it adds may move a step count, emulated time, console byte or frame hash.
  - The only pins allowed to move are the ones that show or type the identity fixtures (Task 1). Each moved pin is recorded old → new with its reason.
- **Fixed design values:**
  - Lanyard `#3b1f52`.
  - Simulation gravity 2400 px/s², fixed step 1/120 s.
  - Sensor clamp ±2000 mg per axis. Rest magnitude 1000 mg.
  - Firmware thresholds: shake ≈1,304 mg magnitude change; Connect's bump >400 mg between samples taken 40–70 ms apart.
- **Data:**
  - Fixtures stay obviously fake, on `@example.com`.
  - `FAKE_UPLOAD_TOKEN` stays only where it is.
  - Nothing from `local/` is committed.
- **Radio:** links use `iceServers: []`, and `/api/signal` is the only server endpoint.
- **Not built:** no light theme, no console input box, no manual pairing, no links list, no Shake or Bump buttons, no Lua.
- **Git:** work on a branch in a worktree (superpowers:using-git-worktrees). Never commit to `main` directly. End commits with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **The page is hidden mid-drag, or the pointer is released outside the window.** The drag ends, the sensor goes back to the resting reading, and the badge isn't left reading a stale 2 g. Tests: Task 9's `drag ends on pointercancel and lostpointercapture`, and Task 12's `hidden page sends the rest reading`.
2. **Power is pressed again during a reboot, or during a provision or skip.** A second press is ignored while booting, and a running provision fails with the existing "the badge rebooted during provisioning". Test: Task 7's `powerCycle while rebooting is ignored`.
3. **A flood of serial output, or bytes that aren't valid UTF-8.** CONSOLE.LOG stays bounded and responsive, and never throws. Test: Task 8's `caps lines and tolerates partial and invalid sequences`.
4. **A peer's bump arrives while phone motion is on, or while the page is hidden.** With phone motion on, it becomes a pulse on the phone readings. While hidden, it is dropped (as today). It is never a crash. Tests: Task 13's `peer bump with phone motion pulses readings` and `peer bump while paused is dropped`.
5. **The window is resized mid-swing, or the phone rotates.** The badge re-lays out without the physics exploding: no NaN, and the sensor stays within ±2000 mg. Test: Task 9's `relayout mid-swing stays finite and clamped`.

---

## Phase A: Emulator

### Task 1: Friendly identities and their re-pins

**Files:**
- Modify: `frontend/public/firmware/test-identities/*.json` (all 11)
- Modify: `emulator-core/tests/boot_progress.rs` (`ROLE_ROWS`, the doc comment at ~:2115, and any equivalence pins the typed identity bytes move)
- Modify: `frontend/test/*.ts` (every assertion that pins these fixtures)
- Create: `docs/milestone-14-decisions.md`

**Interfaces:**
- Produces: the new fixtures (Task 7's power-cycle rung and Task 2's LED rungs pin against them).

- [ ] **Step 1: Rewrite the fixtures.** Change only `display_name` and `account_email`, per the spec's table:

  | role | display_name | account_email |
  |---|---|---|
  | hacker | Ada Lovelace | ada.lovelace@example.com |
  | organizer | Grace Hopper | grace.hopper@example.com |
  | sponsor | Andrew Carnegie | andrew.carnegie@example.com |
  | judge | Alan Turing | alan.turing@example.com |
  | mentor | Charles Babbage | charles.babbage@example.com |
  | volunteer | Annie Easley | annie.easley@example.com |
  | media | Hedy Lamarr | hedy.lamarr@example.com |
  | staff | Katherine Johnson | katherine.johnson@example.com |
  | general | Jane Doe | jane.doe@example.com |
  | workshop_lead | Dennis Ritchie | dennis.ritchie@example.com |
  | visitor | Nikola Tesla | nikola.tesla@example.com |

  Keep each file's key order and single-line form.
- [ ] **Step 2: Run the ratchet and collect what moved.**
  - Run: `cargo test -p emulator-core --release --test boot_progress 2>&1 | tee local/m14-repin.log`
  - Expected: failures only in rungs that pass through provisioning: `ROLE_ROWS` page-1 and registered hashes, `hacker_walk_totals_are_unchanged`, `execution_digest_is_unchanged` if its chain provisions, and launcher-chain step and time pins.
  - Any failure on an unprovisioned path is a bug. Stop.
- [ ] **Step 3: Check that no name is truncated.** For each role, dump page 1 and the registered screen with `cargo run -p emulator-core --release --example boot-probe` (or a scratch test calling `--dump-frame`) to `local/m14-frames/<role>-{page1,registered}.png`. Look at all 22 frames: every name must render whole. "Katherine Johnson" is the longest. If one is cut off, stop and report to the human partner.
- [ ] **Step 4: Re-pin.**
  - Update each failing constant to its new value.
  - In `docs/milestone-14-decisions.md`, add a "Re-pins (Task 1)" table: constant, old, new, and the reason "fixture rename (spec: Friendly identities)".
  - Update the `ROLE_ROWS` doc comment: the hacker row was compared with the physical badge for layout on v335, and the renamed frames were checked by eye in Milestone 14.
  - Replace "Test Hacker" at ~:2115.
- [ ] **Step 5: Fix the frontend tests** that pin a name, an email or a fixture byte count (`bun test` to find them; `test/provisioner.test.ts:319` prints "Test Hacker" in a fake console line, so make it "Ada Lovelace"). Run `bun test` and expect PASS.
- [ ] **Step 6: Run the whole suite.**
  - Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
  - Expected: PASS, with no warnings.
- [ ] **Step 7: Commit** with the message `test: friendly identity fixtures, re-pinned`. Include the decisions doc.

### Task 2: WS2812 decoding on the RMT transmitter

**Files:**
- Create: `emulator-core/src/peripherals/ws2812.rs`
- Modify: `emulator-core/src/peripherals/rmt.rs` (feed each sent word to the decoder; add a `ws2812` field)
- Modify: `emulator-core/src/peripherals/mod.rs`
- Modify: `emulator-core/src/runtime.rs` (`led_colors`, `led_frame_count`)
- Modify: `emulator-wasm/src/lib.rs`
- Test: unit tests in `ws2812.rs`; rungs in `emulator-core/tests/boot_progress.rs`

**Interfaces:**
- Produces (Rust):
  - `pub struct Ws2812` (`Clone`, `Default`) with `pub fn symbol(&mut self, word: u32)`, `pub fn end_of_transmission(&mut self)`, `pub fn colors(&self) -> &[[u8; 3]]` (RGB, chain order, empty until the first frame) and `pub fn frame_count(&self) -> u32`.
  - `FirmwareRuntime::led_colors(&self) -> Vec<[u8; 3]>` and `FirmwareRuntime::led_frame_count(&self) -> u32`.
- Produces (WASM): `led_colors(&self) -> Vec<u8>` (flattened RGB) and `led_frame_count(&self) -> u32`.

**The decoder** (cite `rmt_types.h`'s `rmt_symbol_word_t`, the WS2812B datasheet's T0H/T1H, and the firmware's encoder once you have found it in Step 1):
- **Bits.** A symbol with `level0 = 1` and both durations non-zero is a bit. It is 1 when `duration0 > duration1` and 0 otherwise. This is relative timing, so it needs no clock divider.
- **Byte order.** Bits are MSB-first. Every 24 bits is one LED in G, R, B order, stored as RGB.
- **Frame end.** Any other symbol (an end-marker, or a long low reset), or `end_of_transmission`, ends the frame. If at least one LED was assembled, publish it: replace `colors` and increment `frame_count`. Partial trailing bits are dropped.
- **Hook.** `Rmt::transmit` calls `self.ws2812.symbol(word)` for every word it consumes, and `end_of_transmission()` where it raises `TX_END` or stops. Nothing else in `Rmt` changes.

- [ ] **Step 1: Find the evidence and record it** in the decisions doc's "LEDs" section:
  - Which RMT channel the firmware uses for `LED_DIN` (GPIO3).
  - Its encoder's symbols. Run boot-probe through `prov apply` with a temporary `eprintln!` of the RMT words.
  - The chain length.
  - **Gate:** if the chain is not 6 LEDs, stop and ask the human partner.
- [ ] **Step 2: Write the failing unit tests** in `ws2812.rs`:
  - `decodes_grb_bytes_msb_first`: 24 symbols encoding G=0x10, R=0x20, B=0x30, then an end-marker, give `colors() == [[0x20,0x10,0x30]]` and `frame_count() == 1`.
  - `two_leds_in_chain_order`: 48 bits give two entries in order.
  - `partial_bits_are_dropped`: 30 bits then the end give 1 LED.
  - `no_bits_no_frame`: the end alone leaves `frame_count() == 0`.
  - `relative_timing_only`: the same bits at durations (8,4)/(4,8) and (64,32)/(32,64) decode identically.
- [ ] **Step 3:** Run `cargo test -p emulator-core ws2812` and expect FAIL (the module is missing).
- [ ] **Step 4: Implement** `Ws2812`, the RMT hook and the runtime and WASM accessors.
- [ ] **Step 5:** Run `cargo test -p emulator-core` and expect PASS.
- [ ] **Step 6: Add rungs and pin from measurement.**
  - `leds_flash_during_prov_apply`: from hacker's provisioning checkpoint, `led_frame_count()` rose and some frame was non-black.
  - `leds_show_the_role_colour_when_registered`: at hacker's registered checkpoint, `led_colors()` equals the measured colours.
  - `mood_leds_light_the_chain`: open Mood LEDs from the launcher, using the existing app-open helper, and pin `led_colors()`.
- [ ] **Step 7: Prove nothing moved.**
  - Run: `cargo test -p emulator-core --release --test boot_progress`
  - Expected: PASS with every pre-existing pin unchanged, `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged` included (at their Task 1 values).
  - Then run `cargo clippy --workspace --all-targets` (clean) and `bun run build:wasm`.
- [ ] **Step 8:** Update the notes: the "Known limitations" item 6 ("No LED output"), the GPIO table row for `LED_DIN`, and the "Current state" in `CLAUDE.md`. Commit with the message `feat(core): decode WS2812 frames from RMT`.

### Task 3: Power cycle that keeps the flash

**Files:**
- Modify: `emulator-core/src/runtime.rs` (add `power_cycle`; unit test next to the reset tests at ~:1768)
- Modify: `emulator-wasm/src/lib.rs`
- Test: rung in `emulator-core/tests/boot_progress.rs`

**Interfaces:**
- Produces:
  - `FirmwareRuntime::power_cycle(&mut self) -> Result<(), ImageParseError>`, the same as `reset()` except that it keeps `bus.flash_chip`.
  - It also keeps held host input and the factory MAC (as `reset()` does), restarts at 0 µs, and sets `soft_reset_count` to 0.
  - WASM: `power_cycle(&mut self) -> Result<(), JsError>`.

- [ ] **Step 1: Write the failing unit test** `power_cycle_keeps_flash`: program `FLASH_PROBE` to `0x5A`, call `power_cycle()`, and assert `flash_chip.read(FLASH_PROBE) == 0x5A`, `emulated_time_us() == 0` and `total_steps() == 0`.
- [ ] **Step 2:** Run `cargo test -p emulator-core power_cycle` and expect FAIL.
- [ ] **Step 3: Implement** `power_cycle` by sharing `reset()`'s body through a private helper that takes whether to keep the flash. Document it in the module doc's reset section: a cold power-on keeps the flash, as the physical badge does, and RTC memory and STORE registers are lost.
- [ ] **Step 4: Add the rung** `power_cycle_boots_to_the_registered_screen`. From hacker's `REGISTERED` checkpoint, call `power_cycle()`, run to the first stable frame, and assert it equals hacker's registered hash from `ROLE_ROWS`. If the firmware shows a different first screen after a cold boot, pin what it shows and record the observation.
- [ ] **Step 5:** Run `cargo test --workspace` and expect PASS. Run `bun run build:wasm`.
- [ ] **Step 6: Commit** with the message `feat(core): power_cycle keeps the flash`.

## Phase B: Frontend plumbing

### Task 4: Remove the Lua sandbox

**Files:**
- Delete:
  - `frontend/src/lua/`
  - `frontend/src/badge/` (all but what Step 1 moves)
  - `frontend/src/runtime/lifecycle.ts`
  - `frontend/public/apps/`
  - `frontend/test/{vm,lifecycle,ui,storage,manifest,canvas}.test.ts`
  - `frontend/src/render/canvas.ts`, if only Lua uses it
- Create: `frontend/src/runtime/buttons.ts`
- Modify: `frontend/src/main.ts`, `frontend/src/ui/badge-controller.ts`, `frontend/src/ui/shell.ts`, `frontend/src/runtime/*.ts` (imports), `frontend/package.json` (remove `fengari` and `fengari-interop`), `frontend/test/vite-prebundle.test.ts`, `frontend/test/shell.test.ts`

**Interfaces:**
- Produces: `frontend/src/runtime/buttons.ts` exports `BUTTON_NAMES` (same tuple, same order), `type ButtonName`, `type ButtonSource = "key" | "pad"`, and `type Rgb = [number, number, number]`. Every former importer of `badge/input` and `badge/led` imports from here.
- `BadgeController` loses `mode()` and `setMode()`. It starts the firmware on creation (lazy Worker boot as today).

- [ ] **Step 1:** Create `runtime/buttons.ts` and re-point all imports. Run `bun run typecheck` and expect PASS.
- [ ] **Step 2:** Delete the Lua modules, apps and tests. Remove the mode toggle (`EmulatorMode`, `MODE_OPTIONS`, `mountModeToggle`, `#mode-toggle`) and every Lua branch in `main.ts` and `badge-controller.ts`. Remove the Fengari deps with `bun remove fengari fengari-interop`.
- [ ] **Step 3:** Run `bun run typecheck && bun test && bun run build` and expect PASS. Then run `bun run dev`: the page boots straight into the firmware with the old shell.
- [ ] **Step 4: Commit** with the message `refactor: remove the Lua sandbox`.

### Task 5: Remove manual pairing, the Shake and Bump buttons, and the links list

**Files:**
- Modify: `frontend/src/radio/peer-link.ts` (delete `ManualCodeKind`, `encodeManualCode`, `decodeManualCode`, `deflate`/`inflate`, the base64url helpers and the doc paragraph)
- Modify: `frontend/src/ui/shell.ts` and `frontend/src/ui/badge-controller.ts` (`mountLinks`' manual handlers, the Shake and Bump buttons, `BUMP_REFUSED_NOTE`, `BUMP_NOTE_MS`)
- Modify: `frontend/test/peer-link.test.ts` and `frontend/test/shell.test.ts`

- [ ] **Step 1:** Delete the code and its tests. Run `bun run typecheck && bun test` and expect PASS.
- [ ] **Step 2:** Run `grep -rn "Manual\|manual code\|deflate-raw" frontend/src frontend/test` and expect no hits.
- [ ] **Step 3: Commit** with the message `refactor: remove manual pairing and the shake/bump buttons`.

### Task 6: Worker protocol for acceleration, power, LEDs and console

**Files:**
- Modify: `frontend/src/runtime/firmware-worker-protocol.ts`, `frontend/src/runtime/firmware-worker.ts`, `frontend/src/runtime/firmware-worker-client.ts` and `frontend/src/runtime/firmware-runtime.ts`
- Test: `frontend/test/firmware-worker.test.ts` and `frontend/test/firmware-runtime.test.ts`

**Interfaces:**
- **`ToWorker`:**
  - adds `{ kind: "accel"; mg: [number, number, number] }` and `{ kind: "powerCycle" }`;
  - removes `shake` and `bump`.
- **`FromWorker`:**
  - adds `{ kind: "leds"; rgb: Uint8Array }`, posted when `led_frame_count()` changed since the last post, at most every 50 ms;
  - adds `{ kind: "console"; text: string; at: number; boot: number }`: `text` is `consoleSince(at)` up to `consoleWritten()`, and `boot` is the soft-reset count plus power cycles. It is posted with each `status`, and at once on a reset.
- **`FirmwareRuntime` (TS):**
  - adds `setAcceleration(mg: [number, number, number]): void`, which cancels any canned motion playing and applies the reading;
  - adds `powerCycle(): void`, ignored while `rebooting()` or within 500 ms of the last one; it fails a running provision or skip as a reboot does;
  - adds `ledColors(): Uint8Array`, `ledFrameCount(): number` and `consoleWritten(): number`;
  - keeps `shake()` and `bump()`, because `cpu-wasm.test.ts` uses them.
- **Worker client:**
  - adds `setAcceleration(mg)`: the latest wins, and it is posted at most once per animation frame;
  - adds `powerCycle()`, `onLeds(cb: (rgb: Uint8Array) => void)` and `onConsole(cb: (chunk: { text: string; boot: number }) => void)`;
  - removes `shake()` and `bump()` from `WorkerFirmwareRuntime`.
- **Validation:** the Worker checks `accel` values are finite numbers, clamps each to ±2000 and truncates to integers. A malformed message is ignored.

- [ ] **Step 1: Write the failing tests:**
  - `accel applies the latest reading between slices`: two `accel` messages before a slice give one `setAcceleration` with the second value on the fake handle.
  - `accel rejects non-finite and clamps`: `[NaN, 3000, -9000]` is ignored, and a valid `[100, 3000, -9000]` gives `[100, 2000, -2000]`.
  - `powerCycle while rebooting is ignored` (Review Focus 2): a second `powerCycle` within 500 ms calls `power_cycle` once.
  - `powerCycle fails a running provision`.
  - `leds posts only on change`.
  - `console streams deltas with the boot counter`: two statuses carry contiguous text, and after a power cycle `boot` increments and `at` restarts at 0.
- [ ] **Step 2:** Run `bun test test/firmware-worker.test.ts test/firmware-runtime.test.ts` and expect FAIL.
- [ ] **Step 3: Implement.** The WASM handle's `power_cycle`, `led_colors` and `led_frame_count` come from Tasks 2 and 3, and the fake handle in the tests gains them.
- [ ] **Step 4:** Run `bun test && bun run typecheck` and expect PASS.
- [ ] **Step 5: Commit** with the message `feat(worker): accel, powerCycle, leds and console messages`.

## Phase C: The page

### Task 7: UI stack, theme and app shell

**Files:**
- Modify: `frontend/package.json`, `frontend/vite.config.ts` (`@vitejs/plugin-react`, `@tailwindcss/vite`, the `@` alias to `src`), `frontend/tsconfig.json` (`"jsx": "react-jsx"`, `paths: {"@/*": ["./src/*"]}`) and `frontend/index.html` (a single `<div id="root">`, the Google Fonts `<link>` exactly as in the design's `<helmet>`, and `<title>Hack the Badge Emulator</title>`)
- Create: `frontend/components.json` (shadcn), `frontend/src/ui/theme.css`, `frontend/src/lib/utils.ts` (shadcn's `cn`), `frontend/src/components/ui/{button,select,input,scroll-area}.tsx` (via the shadcn CLI, `bunx --bun shadcn@<pinned> add …`), `frontend/src/ui/App.tsx` and `frontend/src/main.tsx` (which replaces `main.ts`)
- Test: `frontend/test/app-shell.test.tsx` (happy-dom plus `@testing-library/react`)

**Theme tokens** (in `theme.css` as shadcn CSS variables, dark only):

| token | value |
|---|---|
| background | `#050505` |
| card / panel | `#0a0a0a` |
| border | `#1c1c1c` |
| foreground | `#ececec` |
| muted-foreground | `#8c8c8c` |
| accent text | `#c9a7dc` |
| accent strong | `#b07ccb` |
| primary (active fill) | `#6b4a7d` |
| field background | `#0d0d0d` |
| field border | `#2a2a2a` |
| focus ring | `#b07ccb` (2 px ring `rgba(176,124,203,.35)`) |
| radius | 6 px controls, 12 px desktop panels, 14 px mobile panels |

Fonts: `font-sans` is Space Grotesk and `font-mono` is JetBrains Mono. Restyle the shadcn Button into the design's `.ghost` (mono 600 12 px, uppercase, `.06em`, `min-h-10`), and Input and Select into `.field`.

- [ ] **Step 1:** Add the deps at exact versions: `react`, `react-dom`, `@types/react`, `@types/react-dom`, `@vitejs/plugin-react`, `tailwindcss`, `@tailwindcss/vite`, `matter-js`, `@types/matter-js`, `@testing-library/react`, plus whatever the shadcn CLI adds (Radix, `class-variance-authority`, `clsx`, `tailwind-merge`, `lucide-react`). Pin every one exactly and record it in the decisions doc.
- [ ] **Step 2: Write the failing test** `App renders header and footer`: render `<App controller={fakeController} />` and expect the texts "HACK THE NORTH 2026", "Badge Emulator", "Source on GitHub" and "GPL-3.0". There is no "Lua Sandbox" text and no "Real Firmware" toggle.
- [ ] **Step 3:** Run `bun test test/app-shell.test.tsx` and expect FAIL.
- [ ] **Step 4: Implement** `App.tsx`: the header, an empty stage slot, an empty aside slot, the footer (the design's markup and links verbatim), and the responsive layout.
  - **Desktop** (`md` and up) is a wrapping row: the stage at `flex: 999 1 560px`, 780 px tall; the aside at `flex: 1 1 320px`. Max width 1360 px, padding `28px clamp(16px,3vw,40px) 40px`.
  - **Mobile** is one column with 16 px gutters and a 640 px stage.

  `main.tsx` creates the controller and renders `<App>`.
- [ ] **Step 5:** Run `bun test && bun run typecheck && bun run build` and expect PASS. Note the build size before and after in the decisions doc.
- [ ] **Step 6: Commit** with the message `feat(ui): React, Tailwind and shadcn shell`.

### Task 8: Console log model and the CONSOLE.LOG panel

**Files:**
- Create: `frontend/src/ui/console-log.ts` and `frontend/src/ui/ConsolePanel.tsx`
- Test: `frontend/test/console-log.test.ts`

**Interfaces:**
- `type ConsoleSource = "serial" | "page" | "event"` and `interface ConsoleEntry { id: number; source: ConsoleSource; text: string }`.
- `createConsoleLog(maxLines = 2000)` returns:
  - `appendSerial(text, boot)`: splits on `\n`, strips `\r`, holds a trailing partial line until more text arrives, and pushes a `page` marker "— badge rebooted —" when `boot` changes;
  - `appendPage(text)` and `entries(): readonly ConsoleEntry[]`;
  - `subscribe(cb): () => void` and `getSnapshot()` (for `useSyncExternalStore`).
- `ConsolePanel`:
  - the design's section with the "CONSOLE.LOG" header (desktop: `max-height: 360px`; mobile: `height: 180px`);
  - newest at the bottom, sticking to the tail unless the visitor scrolled up more than 24 px;
  - `page` and `event` entries in muted text with a `[page]` / `[event]` prefix;
  - a `footer` slot left empty for a future input row.

- [ ] **Step 1: Write the failing tests:**
  - `splits lines and holds the partial tail`.
  - `marks a reboot when boot changes`.
  - `caps lines and tolerates partial and invalid sequences` (Review Focus 3): appending 10,000 lines keeps 2,000 (the newest), and text containing `�` passes through unchanged.
  - `page entries interleave in arrival order`.
- [ ] **Step 2:** Run `bun test test/console-log.test.ts` and expect FAIL.
- [ ] **Step 3: Implement** both files, and wire the client's `onConsole` to `appendSerial` in the controller (Task 12 finishes the wiring).
- [ ] **Step 4:** Run `bun test` and expect PASS.
- [ ] **Step 5: Commit** with the message `feat(ui): CONSOLE.LOG with serial and page sources`.

### Task 9: Lanyard physics and the sensor mapping

**Files:**
- Create:
  - `frontend/src/motion/source.ts`
  - `frontend/src/motion/sensor.ts` (pure)
  - `frontend/src/motion/lanyard.ts` (matter.js)
  - `frontend/src/motion/axes.ts`
- Test: `frontend/test/sensor.test.ts`, `frontend/test/lanyard.test.ts` and `frontend/test/physics-shake.test.ts`

**Interfaces:**
- `source.ts`:

  ```ts
  export type Mg = [number, number, number];
  export interface MotionSource {
    start(sink: (mg: Mg) => void): void;
    stop(): void;
    /** A linked peer bumped us: show it and let the sensor feel it. */
    peerBump(): void;
    /** A wall impact (or jolt) worth telling linked peers about. */
    onImpact(cb: () => void): () => void;
  }
  ```

- `sensor.ts`:
  - `export const REST_MG = 1000; export const CLAMP_MG = 2000;`
  - `export function sensorMg(angleRad: number, accelPx: { x: number; y: number }, gainMgPerPxS2: number, axes: AxisMap): Mg`.
  - It returns gravity (1000 mg pointing down the board's tilt) plus the motion (screen-frame px/s², rotated into the board frame, times the gain).
  - It maps the board's in-plane axes and face normal to the chip's x, y and z through `axes`, clamps each axis to ±2000, and rounds.
  - The face-normal axis gets 0 from motion. The badge is in the screen plane.
- `axes.ts`: `export interface AxisMap { right: [axis: 0|1|2, sign: 1|-1]; up: [0|1|2, 1|-1]; out: [0|1|2, 1|-1] }` and `export const BOARD_AXES: AxisMap` (Step 1 sets it).
- `lanyard.ts`: `export function createLanyard(opts: { width: number; height: number; layout: "desktop" | "mobile" }): Lanyard & MotionSource`. `Lanyard` has:
  - `step(dtMs: number)`, which runs fixed 1/120 s substeps with gravity 2400 px/s²;
  - `relayout(width, height)`;
  - `pose(): { rope: {x:number;y:number}[]; badge: { x: number; y: number; angle: number; w: number; h: number } }`;
  - `grab(p)`, `drag(p)` and `release()`, plus `impulse(kind: "shake" | "knock")`.

  The badge body's acceleration comes from velocity differences across substeps, at the chip's spot (the rotational terms included).
- **Layout clamps from the design's `layout()`:**
  - Desktop: `bh = max(280, min(560, H-230, (W-48)*1513/974))`, rope length `max(60, H-96-bh-clip+10)`.
  - Mobile: `max(260, min(560, H-190, (W-40)*1513/974))`, rope length `max(50, H-84-bh-clip+10)`.
  - Both: `clip = round(bw*0.2)`, about 10 rope segments, the anchor at `(W/2, -10)`, and the side walls at the stage edges.
- **Impact detection:** on wall contact, measure the normal velocity at impact. Fire `onImpact` above a threshold, rate-limited to one per 800 ms (the design's `detect()`).
- **Gain:** `GAIN_MG_PER_PX_S2`, tuned in Step 6.

- [ ] **Step 1: Settle the axis evidence.**
  - Search the firmware traces in `docs/milestone-5-decisions.md` (ACCEL), `docs/milestone-10-decisions.md` and the v392 disassembly for any per-axis use, such as a tilt or orientation feature.
  - If both detectors (shake magnitude, bump vector delta) are rotation-invariant and nothing reads a single axis, record the ruling "the mapping does not change firmware behaviour". Set `BOARD_AXES` to right = +x, up = +y, out = +z, cited from the chip's pin-1 orientation in the photo if it is legible.
  - If something does read a single axis, stop and bring the evidence to the human partner.
- [ ] **Step 2: Write the failing tests** for `sensor.ts`:
  - `rest hanging reads 1000 mg along up`: `sensorMg(0, {x:0,y:0}, g, BOARD_AXES)` has magnitude 1000 ± 1 on the up axis and 0 on the others.
  - `tilted 90° moves gravity to right`.
  - `linear motion adds gain-scaled`: `accelPx = {x: 1000, y: 0}` at angle 0 adds `1000*g` on the right axis (with the sign pinned).
  - `clamps each axis to ±2000`.
- [ ] **Step 3: Write the failing tests** for `lanyard.ts`, headless with no DOM:
  - `settles to rest`: after 5 s of steps with no input, the sensor is within 30 mg of rest.
  - `drag ends on pointercancel and lostpointercapture`: `release()` after `grab` and `drag` returns to free swinging, and after 5 s the reading is within 30 mg of rest (Review Focus 1).
  - `relayout mid-swing stays finite and clamped` (Review Focus 5): swing hard, then `relayout(390, 640)`. Every reading over the next 2 s is finite and within ±2000.
  - `knock fires onImpact once`: `impulse("knock")` gives exactly one `onImpact` in 800 ms, and some pair of readings 40–70 ms apart differs by more than 400 mg in vector magnitude.
- [ ] **Step 4:** Run `bun test test/sensor.test.ts test/lanyard.test.ts` and expect FAIL.
- [ ] **Step 5: Implement** `sensor.ts`, `axes.ts` and `lanyard.ts`.
- [ ] **Step 6: Tune against the real firmware.** In `test/physics-shake.test.ts`, use the WASM twin and the helpers `cpu-wasm.test.ts` uses to reach Setup page 6 ("Shake it!"):
  - Drive the lanyard with a scripted "vigorous drag": grab the badge and move the pointer ±120 px horizontally at 3 Hz for 2 s.
  - Each 1/60 s, feed `sensorMg` into `handle.setAcceleration` and run the emulator to the matching emulated time.
  - Test `a vigorous drag passes Shake it!`: the frame leaves page 6's hash.
  - Test `a gentle drag does not`: ±20 px at 1 Hz stays on page 6.

  Choose `GAIN_MG_PER_PX_S2` so both pass. Record the value, the readings' peak magnitude and the rationale in the decisions doc. **Gate:** if no gain gives both results within the ±2000 mg clamp, stop and bring the numbers to the human partner. Do not change the firmware or the sensor.
- [ ] **Step 7:** Run `bun test && bun run typecheck` and expect PASS.
- [ ] **Step 8: Commit** with the message `feat(motion): matter.js lanyard drives the accelerometer`.

### Task 10: Phone motion source

**Files:**
- Create: `frontend/src/motion/device-motion.ts`
- Test: `frontend/test/device-motion.test.ts`

**Interfaces:**
- `export function deviceMotionToMg(a: { x: number; y: number; z: number }, signFix: 1 | -1, axes: AxisMap): Mg`. It takes `accelerationIncludingGravity` in m/s², divides by 9.80665, multiplies by 1000 and by `signFix`, maps phone axes to board axes (portrait, screen toward the visitor: phone x is right, y is up and z is out), and clamps.
- `export function platformSignFix(ua: string): 1 | -1`: the sign that makes an upright phone read +1000 on up. Its table is filled from the real-device check in Task 14. It starts at 1 for all platforms, with a comment that Task 14 verifies it.
- `export function motionAvailable(): boolean` and `export async function requestMotionPermission(): Promise<"granted" | "denied" | "unsupported">`. On iOS this calls `DeviceMotionEvent.requestPermission()` and must be called from a user gesture.
- `export function createDeviceMotionSource(win: Pick<Window, "addEventListener" | "removeEventListener">, signFix: 1 | -1): MotionSource & { onMotion(cb: (mg: Mg) => void): () => void }`.
  - `onMotion` lets the lanyard follow the phone.
  - `peerBump()` adds +1000 mg on x for 100 ms on top of the readings.
  - `onImpact` fires on a jump of more than 1500 mg between consecutive events, rate-limited to 800 ms.

- [ ] **Step 1: Write the failing tests** with a fake window that dispatches `devicemotion` events:
  - `upright phone reads 1000 mg up`.
  - `flat phone reads 1000 mg out`.
  - `peer bump with phone motion pulses readings` (Review Focus 4): readings during the 100 ms have x increased by 1000, clamped.
  - `jolt fires onImpact once per 800 ms`.
  - `null accelerationIncludingGravity is ignored`.
- [ ] **Step 2:** Run `bun test test/device-motion.test.ts` and expect FAIL.
- [ ] **Step 3: Implement** it.
- [ ] **Step 4:** Run `bun test` and expect PASS.
- [ ] **Step 5: Commit** with the message `feat(motion): DeviceMotion source`.

### Task 11: The badge component

**Files:**
- Create:
  - `frontend/public/badge/front.webp`, copied from `local/m14-design/front.webp` (the design's two artboards are beside it); check its sha256 starts with `f1e63701` and ends with `89bd4`
  - `frontend/src/ui/badge-geometry.ts`
  - `frontend/src/ui/Badge.tsx`
  - `frontend/src/ui/Stage.tsx`
  - `frontend/src/ui/keyboard.ts`
- Modify: `frontend/src/render/framebuffer.ts`, only if it needs to draw into a supplied canvas
- Test: `frontend/test/badge-geometry.test.ts` and `frontend/test/keyboard.test.ts`

**Interfaces:**
- `badge-geometry.ts` holds the design's values, in percent of the 974×1513 photo:

  ```ts
  export const PHOTO = { w: 974, h: 1513, src: "/badge/front.webp" };
  export const SCREEN = { left: 15.39, top: 14.76, width: 45.45, height: 21.93 };
  export const POWER = { left: 34.1, top: 1.5, width: 10.5, height: 4 };
  export const HOLES = [[19.59, 3.08], [80.18, 2.83]] as const;
  export const LED_SPOTS = [[10.3, 5.58], [85.13, 5.45], [17.08, 52.44], [79.53, 51.85], [17.27, 91.19], [77.86, 91.19]] as const;
  export const BUTTON_SPOTS: Record<ButtonName, { x: number; y: number; s: { desktop: number; mobile: number } }>;
  // UP 24.44,62.06 · LEFT 17.57,67.11 · RIGHT 31.41,67.11 · DOWN 24.74,72.36: s 8.25/9.6
  // A 84.34,63.44 · B 69.12,68.2 · HOME 44.18,81.28 · START 59.2,81.28: s 10.31/12.5
  // AUX1 4.41,37.28: s 6.09/7.5 (side switch: aspect 1/2, radius 6px)
  /** LED_SPOTS index for each chain index (Task 2's order); Task 14 confirms against the physical badge. */
  export const LED_CHAIN_TO_SPOT: readonly number[];
  ```

- `keyboard.ts`: `export function keyAction(code: string): { button: ButtonName } | { toggle: "AUX1" } | { impulse: "shake" | "knock" } | null`.
  - ArrowUp, ArrowDown, ArrowLeft, ArrowRight → UP, DOWN, LEFT, RIGHT.
  - KeyZ → A, KeyX → B, Enter → START, Escape → HOME.
  - Space → toggle AUX1. KeyS → shake. KeyK → knock.
  - It ignores events whose target is an input or a select.
- `Badge.tsx` renders the design's markup (`Main.dc.html` lines 77–98):
  - the photo;
  - the pixelated screen `<canvas width=320 height=240>` at `SCREEN`;
  - the LED glow `div`s (the design's `.led` CSS, `--rgb` and opacity set per frame through refs from the client's `onLeds`; opacity is `max(r,g,b)/255`, and off is 0);
  - the hit `<button>`s with the design's `.hit`, `.is-down` and `.side` styles, `aria-label`s and `aria-pressed` for AUX1;
  - the power `<button aria-label="Power switch: reboot the badge">`, which calls `client.powerCycle()` and shows `.is-down` while rebooting;
  - the lanyard SVG (rope, stitch, shadow, rings, clasp).

  Pointer handlers follow the design: a button press calls `injectButton(name, "pressed"/"released", "pad")`, AUX1 toggles, and a keyboard click (`detail === 0`) taps. Pointer-down on the badge body (not on a button) calls `lanyard.grab`. `pointermove`, `pointerup`, `pointercancel` and `lostpointercapture` on the window call `drag` and `release`.
- `Stage.tsx`:
  - the dotted stage (the design's `.stage`) with the badge;
  - "Reset swing", which calls `lanyard.relayout` with a reset;
  - the hint: desktop "DRAG THE BADGE TO SWING IT · PRESS THE BUTTONS ON THE BOARD", mobile "DRAG TO SWING · TAP THE BUTTONS";
  - one rAF loop that steps the lanyard, writes the transforms, SVG paths and LED styles, and sends `sensorMg` to the active sink.
- **Layout and hit sizes:** the layout prop comes from `matchMedia("(min-width: 768px)")`, and the bigger mobile hit sizes follow the layout.

- [ ] **Step 1: Write the failing tests:**
  - `every button has a spot inside the photo`: all nine names are present, with 0 < x, y < 100.
  - `mobile hit sizes are larger`.
  - `LED_CHAIN_TO_SPOT is a permutation of 0..5`.
  - `keyAction maps the documented keys`: one assertion per key above.
  - `keyAction ignores typing in inputs`.
- [ ] **Step 2:** Run `bun test test/badge-geometry.test.ts test/keyboard.test.ts` and expect FAIL.
- [ ] **Step 3: Implement** them. Set `LED_CHAIN_TO_SPOT` from Task 2's evidence; if the evidence is inconclusive, use `[0,1,2,3,4,5]` and leave a decisions-doc note for Task 14.
- [ ] **Step 4:** Run `bun test && bun run typecheck` and expect PASS.
- [ ] **Step 5: Check by eye.** With `bun run dev`, compare the desktop and mobile layouts (Chrome DevTools at 1440×900 and 390×844) against the design's two artboards. Buttons sit on the board's buttons, the screen sits in the LCD, and the swing looks like the design's.
- [ ] **Step 6: Commit** with the message `feat(ui): the hanging badge`.

### Task 12: Controller store, FIRMWARE TOOLS and BADGE LINK

**Files:**
- Modify: `frontend/src/ui/badge-controller.ts`, to become a non-visual store
- Create: `frontend/src/ui/ToolsPanel.tsx` and `frontend/src/radio/links.ts`
- Delete: `frontend/src/ui/shell.ts`, `frontend/src/ui/shell.css`, `frontend/test/shell.test.ts` and `frontend/src/main.ts`
- Test: `frontend/test/badge-controller.test.ts` (adapt the existing coverage) and `frontend/test/links.test.ts`

**Interfaces:**
- `createBadgeController(deps)` returns `BadgeController`:
  - `subscribe(cb)` and `getSnapshot(): BadgeSnapshot`, where `BadgeSnapshot = { speed: number; rebooting: boolean; provision: ProvisionState | null; skip: SkipState | null; setupOnScreen: boolean; tabNotice: boolean; linkStatus: string; motion: "physics" | "phone"; phoneMotionAvailable: boolean }`;
  - `firmware()`, `console: ConsoleLog` (Task 8), `provision(role)`, `skipSetup()`, `powerCycle()`, `runHere()`, `host()`, `join(code)`, `usePhoneMotion(on: boolean): Promise<void>`, `motionSink(): (mg: Mg) => void` and `dispose()`.
  - Status changes that today's status line shows become `console.appendPage(...)` lines, using the same texts `badge-controller.ts` already builds.
- **Visibility:** on a hidden page, the controller sends `[0, 1000, 0]` mapped through `BOARD_AXES` (the hanging rest) and stops forwarding samples until visible.
- `links.ts`: `createLinks(hub, signal)` returns `{ host(): Promise<string>; join(code: string): Promise<void>; summary(): string; subscribe(cb): () => void }`.
  - `summary()` is "" when there are no links, "Hosting ABC123" while a hosted room waits, "Linked to N badge(s)", or the signal client's last message.
  - A closed or failed link drops out, and its reason goes to `console.appendPage`.
  - Every link closes on `pagehide`.
- **`ToolsPanel.tsx`**, the design's FIRMWARE TOOLS section:
  - the header shows "SPEED INDEX" and `speed.toFixed(1) + "x"`;
  - the ROLE `Select` lists the roles from `test-identities.ts`, labelled with `roleLabel`;
  - PROVISION is disabled while a provision or skip runs;
  - Skip setup is enabled only when `setupOnScreen`;
  - "Use phone motion" is a `Button`, shown only when `phoneMotionAvailable`, and toggles between physics and phone;
  - BADGE LINK has the room-code `Input` (uppercased, maxLength 16), Host link, Join link (disabled with an empty code) and the `linkStatus`.
- **Tab notice:** the design's banner, "Badge running in another tab" with "Run here", shown when `tabNotice`.

- [ ] **Step 1: Write the failing tests:**
  - `hidden page sends the rest reading` (Review Focus 1).
  - `provision and skip states become page console lines`.
  - `summary counts open links`.
  - `closed link drops out with a console reason`.
  - `skip enabled only when setup on screen`, which renders `ToolsPanel` with snapshots.
  - `phone motion button hidden when unavailable`.
  - `denied phone motion keeps physics and explains`: when `requestMotionPermission` gives `"denied"`, `motion` stays `"physics"` and a `[page]` line says phone motion was denied.
- [ ] **Step 2:** Run `bun test test/badge-controller.test.ts test/links.test.ts` and expect FAIL.
- [ ] **Step 3: Implement** them. Port the existing logic (badge lock, lazy Worker boot, provisioning, the skip gate, the hub's running gate), and delete the old shell, its CSS, `main.ts` and the old tests.
- [ ] **Step 4:** Run `bun test && bun run typecheck && bun run build` and expect PASS.
- [ ] **Step 5: Commit** with the message `feat(ui): firmware tools, badge link and controller store`.

### Task 13: Bumps through motion

**Files:**
- Modify: `frontend/src/radio/hub.ts` and `frontend/src/ui/badge-controller.ts`
- Test: `frontend/test/radio-hub.test.ts`

**Interfaces:**
- `RadioHub.bump()` becomes `broadcastBump(): boolean`. It keeps the gate (running and not paused) and sends `{t:"bump"}` to every link, but no longer bumps the local badge. It returns `false` when gated.
- `RadioHubTarget.bumpLocal()` is implemented by the controller as `activeMotionSource.peerBump()`: an impulse on the lanyard, or a pulse in phone mode.
- The controller subscribes each source's `onImpact` to `hub.broadcastBump()`.
- In phone mode the lanyard follows `deviceMotion.onMotion` by applying the reading's in-plane part as a force at the rope anchor. Its own `sensorMg` is not sent, and `grab` and the S/K keys are ignored.

- [ ] **Step 1: Write the failing tests:**
  - `broadcastBump sends to links without a local bump`.
  - `broadcastBump is gated while stopped or paused`.
  - `peer bump calls bumpLocal` (unchanged path).
  - `peer bump while paused is dropped` (Review Focus 4).
  - In `badge-controller.test.ts`: `peer bump with phone motion pulses readings`, which goes through the controller to the phone source's `peerBump`, and `an impact broadcasts a bump`.
- [ ] **Step 2:** Run `bun test test/radio-hub.test.ts test/badge-controller.test.ts` and expect FAIL.
- [ ] **Step 3: Implement** it.
- [ ] **Step 4:** Run `bun test && bun run typecheck` and expect PASS.
- [ ] **Step 5: Commit** with the message `feat(radio): bumps come from motion`.

### Task 14: Docs, final suites and the human partner's checks

**Files:**
- Modify: `CLAUDE.md`, `docs/firmware-emulator-notes.md` and `docs/milestone-14-decisions.md`
- Possibly modify: `frontend/src/ui/badge-geometry.ts` (`LED_CHAIN_TO_SPOT`) and `frontend/src/motion/device-motion.ts` (`platformSignFix`)

- [ ] **Step 1: Update `CLAUDE.md`:**
  - The intro becomes one mode, the real firmware; drop the "Lua sandbox" paragraphs.
  - Add the LEDs, the power cycle, the motion sources and the UI stack to "Current state" and the architecture tree. Remove the `lua/`, `badge/` and `shell.ts` entries, and add `motion/`, `ui/*.tsx`, `components/ui/` and `radio/links.ts`.
  - Update the test counts from the final runs.
  - Update the notes ("Known limitations", the GPIO table).
- [ ] **Step 2: Write the decisions doc's remaining sections:**
  - "Human decisions (2026-11-12)", from the spec;
  - "Rulings" (the axis mapping, the gain, the LED order, the impact threshold);
  - "Dependencies added";
  - "Milestone 15 backlog": copy Milestone 14's backlog from `docs/milestone-13-decisions.md` (from "## Milestone 14 backlog" to the end of that section), dropping the items this milestone deleted (Task 6's refused-Bump note and Remove's room line, `manifest.test.ts`), plus anything new from these tasks.
- [ ] **Step 3: Run the final suites** and record their counts in the decisions doc:
  - `cargo test --workspace`
  - `cargo test -p emulator-core --release --test boot_progress`
  - `cargo clippy --workspace --all-targets`
  - `bun run build:wasm && bun test && bun run typecheck && bun run build`

  Expected: all PASS, with clippy clean.
- [ ] **Step 4: Gate: the human partner's browser check.** Ask them to run `bun run build && bun run preview` (and the deployed build or an HTTPS dev server for the phone) and check:
  1. desktop and phone layouts against each artboard;
  2. Provision → shake to pass "Shake it!" (drag on desktop; phone motion on the phone, on iOS and on Android if available);
  3. Skip setup works;
  4. Mood LEDs: **the LED positions match the physical badge's chain** (fix `LED_CHAIN_TO_SPOT` if not);
  5. power switch: the badge reboots and stays registered;
  6. two linked browsers reach "SYNCED" in Connect by swinging into a wall;
  7. CONSOLE.LOG streams serial output with `[page]` lines;
  8. on the phone, an upright phone reads about 1 g "up". The Shake it! page shows G-force, so check it: fix `platformSignFix` per platform if it doesn't.

  Record their answers in the decisions doc, and apply the fixes with tests updated.
- [ ] **Step 5: Commit** with the message `docs: Milestone 14 decisions, notes and CLAUDE.md`.

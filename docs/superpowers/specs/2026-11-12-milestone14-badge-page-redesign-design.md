# Milestone 14: the badge page redesign

Date: 2026-11-12. Status: draft for the human partner's approval.

## Intent

The page becomes the hanging badge from the human partner's design
canvas ("Hack the Badge — UI redesign",
https://claude.ai/code/artifact/d5fbbc3e-1c84-446a-96bd-f6df1f9e89ba,
two artboards: `Main.dc.html` for desktop and `Mobile.dc.html` for
mobile). The badge is a photo of the real board on a lanyard. You swing
it by dragging, and you press the buttons on the board itself. The
emulator feels that swing through its accelerometer, and its LEDs light
with the colours the firmware actually sends. The page is the real
firmware only. The Lua sandbox goes away.

Visitors land straight on this page, with no intro or landing screen
(the human partner's answer).

## Goals

1. **Match the design** on desktop and on mobile, as two separate
   layouts (the canvas has one artboard each), built with this
   codebase's new UI stack (below).
2. **Real firmware only.** Remove the Lua sandbox and the mode toggle.
3. **Badge physics → accelerometer.** The lanyard swing is a physics
   simulation, and the badge's proper acceleration feeds the emulated
   SC7A20H continuously. "Shake it!" and Connect's bump are triggered by
   motion the visitor makes, not by canned pulses.
4. **The power switch on top reboots the badge.**
5. **Real LEDs.** The emulator decodes the WS2812 data the firmware
   sends over RMT. The design's six glows show those colours at the
   positions of the board's real LEDs.
6. **Friendly test identities.** Each role's fake identity gets a
   human-friendly name, re-pinned in the tests.
7. **CONSOLE.LOG shows the badge's real USB serial output**, with the
   page's own status lines (provisioning, skip, rebooting, link) merged
   in.
8. **Use libraries rather than hand-written code** where one fits.
9. **A phone's own motion sensor can drive the badge** (DeviceMotion),
   added to scope by the human partner on 2026-11-12. See "Phone
   motion".

## Non-goals

These may come later, so the code leaves a seam for each (see
"Flexibility").

- Typing into the console.
- A log of the visitor's actions mixed into CONSOLE.LOG.

Not planned at all:

- A 3D badge. The human partner approved matter.js (2D) over three.js.
- The manual copy-paste pairing mode. It is removed, code included.
- The open-links list with its Remove buttons, and the Shake and Bump
  buttons. All removed.
- The on-page button pad and the LED HUD. The board replaces both.
- Persisting the badge across page reloads.

## Anti-goals (a failure even if it works)

- **Emulated behaviour moves for any reason other than the identity
  rename.** LED decoding only observes RMT. It must not change a step
  count, an emulated time, a console byte or a frame hash. The only pins
  allowed to move are the frames and needles that show the renamed
  identities (goal 6). `execution_digest_is_unchanged` and
  `hacker_walk_totals_are_unchanged` stay as they are, unless a pin
  there depends on the identity bytes. If one does, the re-pin is
  recorded with the reason.
- **Fake effects.** LEDs that light on button presses, or a rainbow, as
  in the design's demo `ledMode`, when the firmware didn't light them.
  Accelerometer readings that aren't the physics.
- **Losing accessibility.** The board's buttons stay real `<button>`s,
  keyboard-reachable and labelled, as the design draws them.
- **Slowing the emulator.** Physics and rendering run on the main thread
  at display rate. The Worker's emulation speed must not drop measurably
  because of them.
- **A real person's identity or credential** in the committed fixtures
  (CLAUDE.md's data rules).

## Constraints

All of `CLAUDE.md` holds. In particular: Bun only; dependencies pinned
to exact versions and reported to the human partner; a chip-agnostic
`cpu/` and `clock.rs`; chip and board data in `peripherals/`;
`emulator-wasm` stays one-line passthroughs; register layouts cited from
ESP-IDF v5.5.3 headers; unmapped data access never panics; peer input
never crashes a tab; `iceServers: []`; and the `local/` data rules.
Decisions in `docs/milestone-*-decisions.md` stand unless the human
partner overrides one.

## Decided by the human partner

- No landing or intro; straight to the page.
- The swing feeds the accelerometer continuously, and shakes and bumps
  fall out of the physics.
- LEDs show the firmware's real colours.
- The identity rename re-pins the tests. There is no separate
  test-only copy.
- CONSOLE.LOG is the real serial output, and the page's status lines
  merge into it. The design should leave room for a later "what you
  did" stream.
- Manual pairing, the links list, Remove, and the Shake and Bump
  buttons are removed.
- UI stack: **shadcn/ui** (which brings React and Tailwind). Physics:
  **matter.js**, not three.js.
- Console typing is out of scope, but the code should make it easy to
  add.
- Phone motion is in scope (added after the first draft). While it is
  on, the phone is the badge, and the drawn badge follows it.
- The swing keeps the design's look. The sensor gets a true 1 g of
  gravity from the badge's tilt, plus the motion scaled by a tuned
  constant.
- Keyboard impulse keys for shaking and bumping.

## The design

### Look and layout

Match both artboards' markup, spacing, type and colour. The design's
values:

- Background `#050505`, panels `#0a0a0a` with `#1c1c1c` borders, text
  `#ececec`, muted `#8c8c8c`, accent `#c9a7dc` and `#b07ccb`, and a
  `#6b4a7d` active fill.
- Space Grotesk for the page and JetBrains Mono for labels, panels and
  the console.
- The stage has a dotted grid (22 px).

Express these as Tailwind theme tokens or shadcn CSS variables, not
inline colours scattered around. Dark only: the design has no light
theme.

**Desktop** (`Main.dc.html`):

- The header: "HACK THE NORTH 2026" over "Badge Emulator." The mode
  toggle that sat on the header's right is gone.
- Below it, a wrapping row: the stage (`flex 999 1 560px`, 780 px tall,
  rounded) and an aside (`flex 1 1 320px`) holding the FIRMWARE TOOLS
  section and the CONSOLE.LOG section.
- The footer, with the copyright, the GitHub link and the GPL-3.0 link,
  as drawn.

**Mobile** (`Mobile.dc.html`):

- One column with 16 px gutters: the header, a 640 px stage, then the
  tools, the console (180 px tall) and the stacked footer.
- Bigger hit targets on the board: `s` 9.6 for the D-pad, 12.5 for
  A/B/HOME/START and 7.5 for AUX1, against desktop's 8.25 / 10.31 /
  6.09.
- The hint reads "DRAG TO SWING · TAP THE BUTTONS" (desktop: "DRAG THE
  BADGE TO SWING IT · PRESS THE BUTTONS ON THE BOARD").
- Smaller badge-size clamps (in `layout()`: `max(260, min(560, H − 190,
  (W − 40)·ratio))` and a rope length of `max(50, H − 84 − …)`).

*Builder's call:* the breakpoint that switches between the two layouts
(something near Tailwind's `md`), and whether the bigger hit targets
follow the layout or `pointer: coarse`.

The "Badge running in another tab · Run here" banner is the design's
`tabNotice` block, wired to the existing badge lock.

### The badge

- **Photo.** The board photo from the canvas (974×1513 WebP, asset
  `a6ea902a16af216c78fc7a7af68af228` of the design artifact; the
  `.dc.html` files only reference it). Already downloaded to
  `/private/tmp/claude-501/-Users-powersagitar-Developer-powersagitar-hack-the-badge/345f9927-2ac3-4758-923b-04028d585119/scratchpad/artifact-files/d5fbbc3e-1c84-446a-96bd-f6df1f9e89ba/a6ea902a16af216c78fc7a7af68af228.webp`
  (sha256 `f1e63701…89bd4`). If the scratchpad is gone, re-read the
  asset from the design artifact. Commit it
  under `frontend/public/` (*my call:* `frontend/public/badge/front.webp`).
  The human partner supplied it for this page.
- **Screen.** The firmware's 320×240 framebuffer, drawn pixelated into
  the photo's LCD rectangle at the design's percentages (left 15.39%,
  top 14.76%, width 45.45%, height 21.93%), reusing
  `render/framebuffer.ts`.
- **Buttons.** Hit targets at the design's `BTNS` percentages, with its
  hover, focus-visible and pressed styles. Pointer down/up drive the
  existing press/release path through the per-button tap queue, as the
  pad does today. A keyboard click (`detail === 0`) is a tap.
- **AUX1** is the side slide switch, so it toggles: on holds the slot,
  off releases it. *My call:* the Space key toggles it too.
- **Keyboard shortcuts stay**: arrows, Z/X → A/B, Enter → START, Escape
  → HOME, Space → AUX1.
- **Lanyard.** The rope, its stitch and shadow, the split ring and clasp
  SVG, as drawn. Lanyard colour `#3b1f52`. The design's `lanyard`,
  `gravity`, `ledMode` and `showHitAreas` props were canvas tweaks: fix
  them at their defaults. `ledMode` goes away (LEDs are real now).
- **"Reset swing"** re-centres the badge.

### Physics (matter.js)

- A rope of jointed segments from a fixed anchor above the stage to the
  badge's ring, and the badge as one rigid body. The stage's side walls
  are colliders. Pointer drag on the badge body (not on its buttons)
  pulls it, like the design's `onBadgeDown` / `onMove` / `onUp`.
- Fixed timestep (about 120 Hz, as the design's two `step(1/120)` per
  frame), decoupled from frame rate.
- **The sensor reading: the design's look wins** (the human partner,
  2026-11-12). The simulation keeps the design's slow swing (gravity
  about 2400 px/s²). It does not run at real-world scale, which would
  swing about 4x faster. The sensor reading is the sum of two parts,
  expressed in the board's frame in mg:
  - **Gravity:** a true 1 g, directed by the badge's current tilt. It
    is not the simulation's px/s² value. The rest magnitude stays
    1000 mg, which the firmware's shake detector uses as its baseline.
  - **Motion:** the body's simulated acceleration at the
    accelerometer's spot (linear plus the rotational terms; the spot
    comes from the chip's position in the photo), times one tuned
    constant (mg per px/s²).

  The constant is tuned so that a vigorous drag reads about 2–3 g and
  clears the firmware's thresholds. Measured from the firmware
  (`docs/milestone-5-decisions.md` ACCEL, `docs/milestone-10-decisions.md`):
  - a shake is a magnitude change of about 1,304 mg;
  - Connect's bump is more than 400 mg between consecutive samples,
    which are 40–70 ms apart.

  The board's physical size is therefore not needed. The board hangs
  vertically, so the face-normal axis reads about 0 g at rest. Clamp
  each axis to the firmware's range (`CTRL_REG4` = `0x80`, FS = ±2 g).
  Because the face-normal axis is near 0, a shake needs both in-plane
  axes near their limits at once. The builder checks that this headroom
  is real before tuning anything else.
- **Axis mapping is evidence-based.** Which SC7A20H axis is the board's
  "up" and which way the face normal points must be established, not
  guessed. Sources: the chip's position and pin-1 orientation in the
  photo, the firmware's own use of the axes (its shake detection, any
  tilt use; traced as Milestone 5 traced `hal_accel`), and the human
  partner's physical badge if needed. Record the evidence in the
  decisions doc. Today's face-up rest of (0, 0, 1000) changes to the
  hanging rest. The builder checks that nothing in the firmware treats
  "not face-up" specially.
- **Delivery.** The main thread sends acceleration samples to the
  Worker (a new protocol message, coalesced to the latest sample or a
  short batch). The Worker applies the latest one between slices
  through `setAcceleration`. Emulated time is paced to wall time, so a
  sample lands about when it happened. *Builder's call:* the send rate
  (it needs to be fast enough that the firmware's shake detector sees
  the reversals; measure what it samples at).
- **Tuning.** A vigorous drag must pass "Shake it!", and a hard swing
  into a stage wall must register as Connect's bump. Tune the physics
  (masses, damping, wall restitution), never the firmware or the
  sensor. If the firmware's thresholds can't be met by plausible
  motion, stop and bring numbers to the human partner.
- **Keyboard impulses** (the human partner, 2026-11-12). Two keys give
  a keyboard-only visitor a way to shake and bump, as real physics:
  - S shakes: a short train of alternating sideways impulses.
  - K knocks the badge into a wall.

  They are not drawn on the page. They are documented with the other
  shortcuts and in the board's accessible description. *Builder's
  call:* the exact keys, if S or K clash.
- **Bumps between linked badges.** Today `RadioHub.bump()` bumps the
  local badge and sends `{t:"bump"}` to peers, and a peer's bump calls
  `bumpLocal()`. Now:
  - *My call:* only a wall impact sends `{t:"bump"}` to peers: a drag,
    the K key, or a phone jolt above the threshold. Shaking in midair
    does not, even though Connect's own detector may fire locally. A
    wall hit stands in for the other badge.
  - A wall impact above a threshold (the design's `detect()` shape)
    sends `{t:"bump"}` through the hub, keeping its gate (badge
    running, page visible). The local badge already felt the impact
    through physics.
  - Tune the threshold so that a deliberate swing into the wall counts
    and normal dragging doesn't.
  - A peer's bump applies an impulse to the local badge body, so it
    visibly jolts and its sensor feels it, instead of the canned
    profile. The impulse must reliably read more than 400 mg within one
    of Connect's samples. Connect's card window is about 1 s, and
    network delay eats into it.
  - When phone motion is on, a peer's bump instead adds a short pulse
    to the phone's readings.
  - The wire message stays the same.
- The canned `shake()` / `bump()` motion schedules stay in the core
  tests and the WASM twin (the rungs use them). The page no longer
  calls them.

### Phone motion

- **The source.** The DeviceMotion API's `accelerationIncludingGravity`
  is the phone's proper acceleration, the same quantity a real SC7A20H
  measures. Use it as a second implementation of the motion-source
  interface (see "Flexibility").
- **Mapping.** Map it into the board's axes as if the phone were the
  badge held in portrait, screen toward the visitor, and convert it to
  mg.
  - Sign conventions differ between browsers (older iOS Safari reports
    the opposite sign). The builder verifies each platform on real
    devices and normalises, rather than trusting the spec text.
  - A phone lying flat reads about 1 g on the face-normal axis. That
    is the reading the emulator's old face-up rest assumed.
- **Turning it on.** A control in FIRMWARE TOOLS, shown only where
  DeviceMotion exists. iOS requires `DeviceMotionEvent.requestPermission()`
  from a user gesture and a secure context.
  - A denied or missing permission leaves the physics source in charge,
    with a console line explaining why.
  - `bun run dev -- --host` over plain HTTP won't get motion events on
    iOS. The human partner's phone check needs HTTPS (the deployed
    build, or a dev HTTPS setup; *builder's call* which).
- **While it's on, the phone is the badge** (the human partner,
  2026-11-12):
  - Its readings feed the sensor.
  - Dragging is off.
  - The drawn badge follows the phone: the phone's motion is applied
    to the physics as forces on the lanyard's anchor, so it sways
    visibly. Its simulated reading is not sent.
  - The keyboard impulse keys are off too (*my call*).
- **Bumps.** A sharp phone jolt above the same threshold as a wall
  impact sends `{t:"bump"}` to linked peers, under the same gate.
- **Seam.** Exactly one motion source drives the sensor at a time. The
  Worker and its protocol don't know which one it is.

### Power switch

- The slide switch at the top of the board (the design's `power` hit
  area: left 34.1%, top 1.5%, 10.5% × 4%) reboots the badge.
- *My call:* it behaves like a real power cycle, a cold power-on that
  **keeps the flash** (the provisioned identity, settings and app
  state), as the physical badge does. The emulator's `reset()` rebuilds
  the synthetic flash today, so this needs a new runtime entry point.
  It keeps the flash, the factory MAC and the held host input, starts
  at 0 µs like `reset()`, and gets a rung: a provisioned badge
  power-cycles and boots to its registered screen.
- While it boots, the button shows the pressed style and the console
  logs "rebooting…". A provision or skip in flight fails as on any
  reboot ("the badge rebooted during provisioning").

### LEDs

- **Core.** `peripherals/rmt.rs` already accepts transmissions and
  completes them at once. Add a WS2812 decoder (board data, so in
  `peripherals/`, e.g. `ws2812.rs`). It reads each transmission's RMT
  symbols (level and duration pairs) and classifies bits by high time
  against the RMT clock and divider the firmware set. It assembles GRB
  bytes into a colour per LED in chain order, and keeps the latest
  frame plus a change counter.
- Cite the RMT symbol layout from ESP-IDF v5.5.3 (`rmt_struct.h`,
  `rmt_types.h`), the timing from the led_strip encoder the firmware
  uses, and the LED's datasheet.
- **Pure observation:** no effect on the bus, timing or interrupts.
- **Runtime/WASM.** `FirmwareRuntime::led_colors()` and a change count,
  with a one-line passthrough in `emulator-wasm`. The Worker posts
  colours when they change.
- **Chain order → positions.** The design's six positions (`LEDS`: top
  left and right, mid left and right, bottom left and right) match the
  six LEDs in the photo. Which chain index sits where must come from
  evidence: the firmware's LED patterns (a role colour, a chase, Mood
  LEDs), the board's traces in the photo, or the human partner
  observing the physical badge. The decisions doc records the
  evidence. If the chain has a count other than 6, stop and ask.
- **Look.** The design's `.led` radial glow (screen blend, `--rgb`),
  with opacity from the colour's brightness. Off is fully transparent.
- **Tests:** decoder unit tests on synthetic symbol streams, and rungs
  that pin real firmware LED output (for example `prov apply`'s green
  flashes and role colour, and an app such as Mood LEDs).

### FIRMWARE TOOLS panel

As drawn:

- The SPEED INDEX in the section header (the Worker's existing speed
  reading).
- PROVISIONING: the ROLE select (shadcn `Select`), then PROVISION and
  Skip setup side by side. Skip setup is enabled only while Setup is on
  screen (the existing `setupOnScreen`). *My call:* it is shown
  disabled, not hidden, as the design does. PROVISION is disabled
  while a skip or a provision runs.
- BADGE LINK: the room-code input, Host link and Join link, and a
  one-line link status on the right. *My call:*
  - Several links can still be open at once.
  - The status line summarises them ("Hosting K3X9QF", "Linked to 2
    badges", or the signal client's messages such as "busy N min").
  - Hosting or joining adds a link, and leaving the page drops them.
  - With no list there's no per-link Remove. A failed or closed link
    just drops out of the count, and the reason goes to the console.

### CONSOLE.LOG

- **Serial.** The badge's USB serial output, decoded as today's
  `consoleSince` does (byte-exact, not ASCII). The Worker sends the new
  bytes since the last offset with its status. The panel appends them,
  newest at the bottom, and follows the tail unless the visitor has
  scrolled up.
  - A reboot is shown as a marker line, not a cleared panel.
  - The panel keeps a bounded number of lines (*builder's call*,
    thousands).
  - *Note:* the design's mock renders newest-first. The human partner
    accepted "newest at the bottom" for real serial output.
- **Page lines.** The status texts that today's status line shows (the
  provision and skip states, `rebooting…`, link status changes,
  failures) become console entries from a separate source, styled
  apart from serial output (for example a muted `[page]` prefix).
- **Model.** The console is a list of entries `{ source, text }`, where
  `source` is `"serial"` or `"page"`. A later `"event"` source (what the
  visitor did) only needs a producer and a style.

### Friendly identities

Rewrite the eleven fixtures in
`frontend/public/firmware/test-identities/`, changing `display_name`
and `account_email` and nothing else (badge IDs, attendee IDs and
claim IDs stay). The upload token stays where it is. They remain
obviously fake. *My call:* historical figures only, no living person.

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

The sponsor is Andrew Carnegie, history's best-known patron. The human
partner asked for a historical figure, any one, on 2026-11-12.

- Check each name fits the firmware's name fields and screens (no
  truncation the old names didn't have).
- Re-pin every rung, `frontend/test` case and WASM-twin assertion that
  shows a name or an email. Record each old → new hash in the decisions
  doc, with the reason.
- The physical-badge comparisons stay valid for layout. The renamed
  frames get the human partner's eye.

### Removing Lua

- Delete `frontend/src/lua/`, the Lua-only `frontend/src/badge/*`
  modules, `runtime/lifecycle.ts`, `frontend/public/apps/`, the
  Fengari dependency and their tests.
- Delete the mode toggle and everything routing between two runtimes
  in `main.ts` and `badge-controller.ts`.
- Anything the firmware path still uses (for example `BUTTON_NAMES` /
  `ButtonName` in `badge/input.ts`) moves somewhere firmware-owned.
- Delete the manual pairing code (`peer-link.ts`'s copy-paste codes and
  their deflate-raw / base64url helpers, and the UI).
- Update `CLAUDE.md` (the "two modes" intro, the layout, the
  architecture tree, the test count) and the notes.

### UI stack

- React with shadcn/ui components (Button, Select, Input; a ScrollArea
  for the console if it fits) on Tailwind, themed to the design's
  tokens.
- matter.js for the physics.
- Fonts: Space Grotesk and JetBrains Mono from Google Fonts, through
  the design's own `<link>` (the human partner is fine with the
  requests).
- Report every package and exact version to the human partner, per
  `CLAUDE.md`.
- `ui/shell.ts`'s DOM building is replaced by components.
  `badge-controller.ts`'s logic (badge lock, Worker boot, provisioning,
  the skip gate, the bump gate) survives as non-visual state that the
  components read (*builder's call* how: a store, hooks, context).
  Keep it unit-testable without a DOM where it is today.
- **The physics loop and the LED/screen painting do not go through
  React renders.** They write to refs and DOM attributes per frame, as
  the design does.

### Flexibility (the seams)

- **Motion source.** The Worker takes acceleration samples from a
  "motion source" interface. The matter.js badge and the phone's
  DeviceMotion are its two implementations, and one is active at a
  time.
- **Console input.** The serial input path the provisioner and the
  skipper already use stays a general "send bytes to the console" API
  on the runtime and worker client. The console panel's layout leaves
  room for an input row.
- **Console sources.** As above: the entry `source` field.

## Testing and acceptance

- `cargo test -p emulator-core` and the `boot_progress` ratchet pass.
  New rungs cover LED output and the power cycle. Only the
  identity-bearing pins move.
- `cargo clippy --workspace --all-targets` is clean.
- `bun test` and `bun run typecheck` pass. New unit tests cover the
  physics → mg mapping (rest pose, a known linear acceleration, a known
  rotation), the console entry model, and the worker protocol's new
  messages.
- `bun run build` passes. The build size grows with React; note it.
- **Browser check by the human partner**, on desktop and a phone:
  - layout matches each artboard;
  - provision, then pass "Shake it!" by shaking;
  - Skip setup works;
  - open Mood LEDs and see the LEDs;
  - power-cycle, keeping registration;
  - two linked browsers bump to "SYNCED" by swinging into a wall;
  - the console streams serial output;
  - on the phone, with phone motion on, shaking the phone passes
    "Shake it!" (iOS and Android if both are available).

## Paperwork

- `docs/milestone-14-decisions.md`: the rulings, the axis-mapping and
  LED-order evidence, the re-pins, the dependencies added, and the
  Milestone 15 backlog.
- *My call:* the Milestone 14 backlog in
  `docs/milestone-13-decisions.md` carries over to Milestone 15
  unchanged, except items this work deletes (for example the Bump and
  Remove minors).
- Update `docs/firmware-emulator-notes.md`'s "Known limitations" (LEDs
  are now modeled).

## Left to the builder

- Task order and structure.
- Component boundaries.
- The state management between the controller logic and React.
- The layout breakpoint.
- Physics constants within the tuning rule above.
- The sample send rate.
- The console line cap.
- Where the photo and fonts live.
- The exact wording of `[page]` lines.

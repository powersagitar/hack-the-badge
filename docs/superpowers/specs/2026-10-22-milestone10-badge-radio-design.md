# Milestone 10 — The badge radio: Connect and Sync, emulated badges bump

Status: scope agreed in conversation with the human partner (2026-10-22)
and split out of Milestone 9, which upgrades the emulator to the official
v392 firmware first
(`docs/superpowers/specs/2026-10-22-milestone9-firmware-upgrade-design.md`).
This spec waits for Milestone 9 to finish; its measurements are re-taken
on v392 at kickoff, then the human partner approves it and a plan
follows.

## Why

The human partner's gap: "not all built in apps open … it just freezes
the whole emulator." On v392, after Milestone 9's software reset, the
apps left are the Bluetooth ones: Connect (and Sync, if it reaches the
radio after its reboot). Both stop at mask-ROM `0x4000_08ec`,
`btdm_controller_rom_data_init` (`esp32c3.rom.bt_funcs.ld`, ESP-IDF
v5.5.3), which has no stub: a Guru Meditation, then "Panic handler
entered multiple times" forever (measured on v335 and on v392; see the
Milestone 9 spec).

## What the firmware's radio does (from the v335 strings; re-read on v392, which changed some of Connect's text)

- **Stack:** ESP-IDF's NimBLE host over Espressif's BLE controller
  (`nimble_port_init`, "waiting for host sync", "host sync timeout",
  `BLE_INIT` "Feature Config …"). On the ESP32-C3 the controller is
  ESP-IDF's closed `libbtdm_app` blob in the app image calling a large
  controller in mask ROM (`esp32c3.rom.bt_funcs.ld`), which this project
  does not have and cannot run.
- **Connectionless only:** a random static address
  (`ble_hs_id_set_rnd`), extended advertising
  (`ble_gap_ext_adv_configure`/`_set_data`/`_start`) and extended
  discovery (`ble_gap_ext_disc`). Payloads must fit one HCI advertising
  report (229 bytes); fragmented reports are dropped with a warning. No
  GATT, no connections were seen.
- **Connect:** home "CONNECT BADGES", "Bump badges or Press A",
  "Contacts: %u", "B/Start Contacts  Home Apps". Pairing by a bump
  (accelerometer; settings `threshold_mg`, `require_bump`) or by "Both
  press or hold A" / "Hold A for up to 2 seconds", gated on proximity
  (`rssi_min`, "Hold badges within 10 cm"). Then "PAIRING...", "SYNCED"
  / "Already connected" / "Conversation Starters unlocked!", or "No badge
  found, try again". "NO RADIO" also exists.
- **Sync:** "Ready to sync", "Find a sync station", "A: sync now",
  "looking for a station...", "station nearby %d dBm", "No sync station
  in range", then a multi-frame upload protocol to a desk ("filling
  gaps", "repair"s).
- The USB console has a `radio [probe]` diagnostic and `uitree` (LVGL
  tree dump), useful for tests.

## What the browser can do (the human partner's constraints)

- No browser can **advertise** BLE. Web Bluetooth can only scan
  (`requestLEScan`, Chrome behind the experimental-web-platform flag;
  Safari has no Web Bluetooth). So a real badge or desk can never hear
  the emulator.
- **No servers whatsoever** (decided): no relay, no signaling server.
  Cross-browser and cross-device links use WebRTC with a manual
  copy-paste of connection codes (accepted by the human partner).

## What the browser can do (the human partner's constraints)

- No browser can **advertise** BLE. Web Bluetooth can only scan
  (`requestLEScan`, Chrome behind the experimental-web-platform flag;
  Safari has no Web Bluetooth). So a real badge or desk can never hear
  the emulator.
- **No servers whatsoever** (decided): no relay, no signaling server.
  Links between devices use WebRTC with a manual copy-paste of
  connection codes (accepted by the human partner).

## Goals

1. **Connect and Sync open to their home screens** and never freeze
   (Part 1).
2. **Emulated badges on different devices hear each other**, through a
   WebRTC link set up by copy-paste. **One active emulator per device,
   like a real badge** (decided): a second firmware-mode tab in the same
   browser does not run a second badge (Part 3).
3. **A bump between two emulated badges pairs them in Connect**, ending in
   "SYNCED" on both, as two physical badges do when bumped (or when both
   hold A).
4. **Conditional: hearing real badges** through Chrome's Web Bluetooth
   scan. Kept only if the firmware shows that one-way hearing has a
   visible effect (Part 5); dropped otherwise, with the evidence
   recorded.

## Non-goals

- A real badge (or desk) hearing an emulated badge: browsers cannot
  advertise.
- A pretend sync desk: its protocol would be guessed with no desk to
  check against. Sync only needs to reach its home screen and fail
  politely ("No sync station in range").
- Share's v392 app transfer over the radio, unless it comes for free
  (Milestone 11 backlog otherwise).
- Connections/GATT, NFC, LEDs, speed. Lua sandbox mode: unchanged.

## Anti-goals (a failure even if it works)

- Any pin from Milestone 9 moving: the radio initializes only when a
  radio app opens, so every other rung,
  `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged`
  stay byte-identical. A move is a bug in the change.
- Any server, relay, STUN/TURN or third-party service, including public
  STUN.
- Browser or network input that can panic the emulator, or a malformed
  peer packet that can crash a tab.
- Guessed register layouts, ROM addresses or app-image addresses.

## Success criteria

1. Rungs: Connect and Sync open (`launched Connect`, `launched Sync`),
   no panic text, the firmware's radio-ready log (`BLE radio ready`),
   and their home screens' frames pinned after the human partner's
   eyeball check.
2. A native two-runtime rung: two `FirmwareRuntime`s with different
   committed fake identities exchange advertisements through the fake
   controller, bump (or both hold A), and both reach "SYNCED"; the
   contact count goes to 1.
3. Sync with no peer reaches "No sync station in range" (or the
   firmware's own empty-room result) without a panic.
4. Every existing suite and pin unchanged; release `boot_progress` under
   90 s.
5. **Human gates:** Connect's and Sync's home screens; a bump between
   emulated badges on two devices, linked by copy-paste WebRTC, ending in
   "SYNCED" on both.

## Design

### Part 1 — A fake BLE controller in emulator-core (home screens)

The ROM controller cannot be run, so it is replaced as a whole by a
high-level model at the **HCI boundary**: the firmware's NimBLE host
sends standard HCI commands and receives HCI events, and the model
answers them. HCI is a published standard (Bluetooth Core Specification,
Vol 4 Part E), so the model's behaviour is specified, not guessed.

- **The seam is the builder's to find, with evidence.** The likely seam
  is ESP-IDF's VHCI API in the app image (`esp_bt_controller_init`,
  `esp_bt_controller_enable`, `esp_vhci_host_register_callback`,
  `esp_vhci_host_check_send_available`, `esp_vhci_host_send_packet`, in
  `components/bt/controller/esp32c3/bt.c`, v5.5.3), intercepted at its
  app-image addresses; or the ROM entry points the blob calls, if that is
  smaller. The firmware has no symbols, so app-image addresses must be
  identified from evidence (call graph from the faulting return address (`0x4207dd0a` on v335; re-measure on v392), string cross-references such as the `BLE_INIT` log, the
  ESP-IDF v5.5.3 source) and the evidence cited in the module doc, the
  same standard as ROM addresses. A new hook mechanism for app-image
  addresses must stay chip-agnostic in `cpu/` (the addresses live in chip
  data, like `rom.rs`).
- **The model answers what NimBLE needs to reach host sync** (reset, read
  local version/features/buffer sizes, set event masks, set random
  address, the LE extended advertising and scanning commands), with
  success events in HCI order.
- **Radio I/O is host input, like buttons:** advertising data the
  firmware sets is exposed to the host (`FirmwareRuntime`, passed through
  the logic-free WASM shim), and the host can deliver an advertisement
  (address, RSSI, data) that becomes an LE Extended Advertising Report
  when scanning is on. With no host input the badge hears nothing:
  emulated time and behaviour stay deterministic for a given input
  schedule.
- Malformed host input is rejected or clamped, never a panic.
- **Each badge needs its own radio identity.** The RNG has a fixed seed
  (Milestone 5, D-M5-3) and every runtime boots the same image, so two
  runtimes would likely pick the same random static address and could
  ignore each other as "self". The model gets an opt-in per-runtime radio
  seed (or address override) as host input; with none set, behaviour and
  every pin are unchanged, so D-M5-3 stands. Tests set two fixed values;
  the browser sets a random value per tab.

### Part 2 — Two badges in one test (pairing)

A native rung runs two runtimes in lockstep (advance each by the same
emulated time, swap advertisements between them), using two committed
fake identities, and drives a bump (Part 3's acceleration pulse on both
at once) or both holding A to "SYNCED". The RSSI of a linked peer is a
fixed strong value that passes `rssi_min` (the builder reads the
firmware's threshold and records it).

### Part 3 — The browser radio hub and the Bump button

- The Worker exposes the runtime's outgoing advertisements and accepts
  incoming ones; a main-thread **radio hub** fans them between the local
  badge and every link.
- **One active emulator per device** (decided 2026-10-22, replacing an
  earlier same-browser `BroadcastChannel` link, which is dropped): a
  per-origin lock (for example `navigator.locks`) lets only one tab run
  the firmware. A second tab shows that the badge is running elsewhere,
  with a control to take it over here (the other tab then stops). My
  call: take-over rather than refusal. The lock is per browser; two
  different browsers on one machine cannot see each other's lock, which
  is accepted, not worked around. R-T8-10 (pause while hidden) stands.
- **Bump button** next to Shake: a short sharp acceleration pulse on this
  badge and, through the hub, on every linked emulated badge at the same
  moment (a real bump hits both). Its shape (peak, duration) is the
  builder's, chosen from the firmware's `threshold_mg` so it reliably
  registers and is distinguishable from Shake.
- Each emulator runs at its own pace; an advertisement is delivered at
  the receiving badge's current emulated time on arrival. No cross-tab
  clock sync.

### Part 4 — WebRTC copy-paste link

- Tab A makes an offer code; the user pastes it into tab B, which makes
  an answer code; the user pastes that back into A. Two pastes per pair.
  A tab can hold several links. Links do not forward to each other
  (radio does not hop): a badge hears only badges it is directly linked
  to.
- **No ICE servers** (`iceServers: []`): host and mDNS candidates only,
  so links work between devices on the same local network, not across
  the internet. Codes wait for ICE
  gathering to finish, so one paste each way suffices.
- Peer data is validated (size, shape) before it reaches the Worker.

### Part 5 — Conditional: hearing real badges (Chrome)

1. **First, from the firmware's code:** does one-way hearing of a real
   badge change anything visible in Connect or Sync? Expected: Connect's
   pairing is two-way ("Both press or hold A", "PAIRING...") and fails
   without an answer; Sync shows "station nearby %d dBm" only for a desk.
   If nothing visible results, Part 5 is dropped and the evidence goes in
   the decisions doc.
2. If kept: a spike with the human partner's badge, Chrome with the flag,
   checks whether the badge's extended advertisements reach the page at
   all (macOS/Chrome may hide them or hand over only parsed fields), and
   only then is the bridge built (scan results → incoming
   advertisements, with the real RSSI).

### Part 6 — Docs

Notes ("Known limitations", a Milestone 10 section),
`docs/milestone-10-decisions.md` with rulings, measurements and the
Milestone 11 backlog, and CLAUDE.md's architecture map.

## Constraints (carried; CLAUDE.md Rules)

- `cpu/` and `clock.rs` chip-agnostic; `FirmwareBus` ordered concrete
  fields, no `dyn`; the word fast path order unchanged.
- `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays
  one-line passthroughs.
- Unmapped data access never panics; an unmapped instruction fetch always
  traps. Browser and peer input never panic the emulator.
- Register layouts from ESP-IDF v5.5.3 headers, ROM addresses from its
  linker scripts, app-image addresses from cited evidence; never guessed.
- Milestones 3 to 8 decisions stand.
- Local data: never quote the full dump, the boot log or recordings.
  Tests depend on the committed `firmware.bin` and committed fake identities only.

## Gates (stop and ask the human partner)

- **After Part 1, before Part 2:** the builder reads Connect's pairing
  exchange in the firmware. If "SYNCED" needs more than connectionless
  advertising and scanning (connections, GATT, scan requests/responses),
  or checks data the committed fake identities cannot pass (a signature
  against an organizer key), stop with the evidence; the human partner
  decides whether that work is in scope or the success criteria change.
- Before Part 4 and before Part 5's spike (the human partner's badge and
  a second device may be needed).
- Part 5's go/no-go after its firmware reading.
- Release `boot_progress` over 90 s; any override of a recorded decision.
- The browser checks in success criterion 5.

## Decided by the human partner (2026-10-22)

- Connect and Sync to their home screens; emulated badges hear each
  other; hear real badges if possible.
- No servers whatsoever; WebRTC with copy-paste accepted.
- One active emulator per device, like a real badge; the peer comes from
  another device. The same-browser link is dropped. R-T8-10 stands.
- Being heard by a real badge and a pretend sync desk are out.
- Real-badge hearing stays on condition that one-way hearing does
  something visible.
- Success is judged by the home screens, and a bump between two emulated
  badges ending in "SYNCED".
- Bluetooth is its own milestone, after the v392 upgrade.

## My calls (the human partner can override)

- The HCI-level model, rather than any attempt at the ROM controller.
- A second tab can take the badge over; it is not simply refused.
- Links do not forward; WebRTC without ICE servers (LAN only).
- A linked peer's RSSI is a fixed strong value.
- Bump hits every linked emulated badge at once.
- An opt-in per-runtime radio seed, default unchanged (D-M5-3 stands).
- A pairing that turns out to need more than connectionless radio, or a
  signature fake identities can't pass, is a stop-and-ask gate, not in
  scope by default.

## Left to the builder

- The exact seam and its evidence; the HCI command set the model covers.
- The host API shape for advertisements; the Bump pulse; code encoding
  (compression) for WebRTC; the hub's UI placement.
- Test structure, beyond the rungs in the success criteria.

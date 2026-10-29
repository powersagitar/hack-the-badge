# Milestone 10 — The badge radio: Connect and Sync, emulated badges bump

Status: scope agreed in conversation with the human partner (2026-10-22)
and split out of Milestone 9, which upgrades the emulator to the official
v392 firmware first
(`docs/superpowers/specs/2026-10-22-milestone9-firmware-upgrade-design.md`).
Milestone 9 finished (main @ `0d56ba5`); the measurements below were
re-taken on v392 at kickoff (2026-10-29, "Measured on v392"). The human
partner approves this spec, then a plan follows.

## Why

The human partner's gap: "not all built in apps open … it just freezes
the whole emulator." On v392, after Milestone 9's software reset, the
apps left are the Bluetooth ones: Connect, and Sync after its reboot.
Both stop at mask-ROM `0x4000_08ec`, `btdm_controller_rom_data_init`
(`esp32c3.rom.bt_funcs.ld`, ESP-IDF v5.5.3), which has no stub. Before
Milestone 9 that was a Guru Meditation and then "Panic handler entered
multiple times" forever. On v392 with the software reset, the panic
handler prints the register dump, then `Rebooting...`, and the badge
boots once into My Badge (`boot: reset reason=4`, no loop; measured,
not pinned, R-T4-2 in `docs/milestone-9-decisions.md`). Either way the
app never opens.

## What the firmware's radio does (strings re-read on v392, 2026-10-29)

- **Stack:** ESP-IDF's NimBLE host over Espressif's BLE controller
  (`nimble_port_init`, "waiting for host sync", "host sync timeout",
  `BLE_INIT` "Feature Config …"). On the ESP32-C3 the controller is
  ESP-IDF's closed `libbtdm_app` blob in the app image calling a large
  controller in mask ROM (`esp32c3.rom.bt_funcs.ld`), which this project
  does not have and cannot run. The host talks to it through ESP-IDF's
  VHCI transport (`components/bt/host/nimble/esp-hci/src/esp_nimble_hci.c`):
  HCI packets in H4 framing, sent with `esp_vhci_host_send_packet` and
  received through a registered `notify_host_recv` callback.
- **Connectionless only:** a random static address
  (`ble_hs_id_set_rnd`), extended advertising
  (`ble_gap_ext_adv_configure`/`_set_data`/`_start`) and extended
  discovery (`ble_gap_ext_disc`). Payloads must fit one HCI advertising
  report (229 bytes); fragmented reports are dropped with a warning
  ("dropped a fragmented advertisement … over the 229-byte single-report
  limit"). No GATT, no connections were seen.
- **The address comes from the eFuse MAC, not the RNG** (measured, below):
  the host-sync callback reads `esp_read_mac(…, ESP_MAC_WIFI_STA)`, sets
  the top two bits and passes it to `ble_hs_id_set_rnd`.
- **Connect** (v392 text): home "CONNECT BADGES", "Bump badges or\nPress
  A", "Contacts: %u", "Hold A for up to 2 seconds\nB/Start  Contacts
  Home  Apps". Pairing by a bump (accelerometer; settings `threshold_mg`,
  `require_bump`) or by "Both press or hold A\nStay within 10 cm", gated
  on proximity (`rssi_min`, "Hold badges within 10 cm"). Then
  "PAIRING...", "SYNCED" / "Already connected" / "Already connected: %s"
  / "Conversation Starters unlocked!", or "No badge found,\ntry again".
  "NO RADIO" also exists.
- **Sync:** "Ready to sync", "Find a sync station", "A: sync now   -
  HOME: home", "looking for a station...", "station nearby   %d dBm",
  "No sync station in range", then a multi-frame upload protocol to a
  desk ("filling gaps  -  %d frame%s", "%u repair%s").
- The USB console has a `radio [probe]` diagnostic ("probe starts no
  advertisements or scan") and `uitree` (LVGL tree dump), useful for
  tests.

## Measured on v392 (2026-10-29)

Scratch probes (not committed) from role hacker's launcher, as
`press_a_on_cell` in `boot_progress.rs` does, with a shadow call stack
from `hal_radio: init_once` to the fault. Addresses in the app image were
then read from a disassembly of `firmware.bin`'s segments (llvm-objdump),
against ESP-IDF v5.5.3's source.

**The fault.** Both apps: `MEPC 0x400008ec`, `RA 0x420806fe` (v335's was
`0x4207dd0a`), `SP` in the calling task's stack.

| App | `hal_radio: init_once` after A's release | Fetch of `0x4000_08ec` | Soft resets first |
|---|---|---|---|
| Connect (cell 1) | by 0.60M steps | 660,083 steps | 0 |
| Sync (cell 3) | by 6.70M steps | 6,702,968 steps | 1 (`reboot into sync`) |

The register dump follows at 0.75M (Connect) and 6.75M (Sync), as the
Milestone 9 rungs pin.

**The call path** (the shadow stack and the panic's stack memory agree):

| Function | Entry | Call site | Identified by |
|---|---|---|---|
| hal_radio's init | — | `0x42010476` | `hal_radio` tag; logs "init_once", "waiting for host sync", "BLE radio ready" |
| `nimble_port_init` | `0x42085848` | `0x4208588e` | copies the 0x48-byte default controller config, then init / enable / `esp_nimble_init`, with "controller init failed" and "controller enable failed" |
| `esp_bt_controller_init` | `0x42080ce4` | `0x42080d5a` | bt.c's checks in source order: `btdm_controller_status` (`0x3fca089c`) idle, task priority 0x17, stack ≥ 0x1000, BLE-only, `ble_max_act`, sleep mode; magic `0x5a5aa5a5`; then `esp_phy_modem_init`, `esp_bt_power_domain_on`, `btdm_controller_mem_init`; "BT controller compile version" under `BLE_INIT` |
| `btdm_controller_mem_init` | `0x420806f2` | `0x420806fa` | bt.c's one-call wrapper around the ROM function |
| `btdm_controller_rom_data_init` | `0x400008ec` (ROM) | — | `esp32c3.rom.bt_funcs.ld` |

**The VHCI seam** in the app image. Each of these reads or writes
`btdm_controller_status` at `0x3fca089c`, and their bodies match
`components/bt/controller/esp32c3/bt.c` (v5.5.3) line for line:

| bt.c function | Entry | Calls into the closed blob | Called from |
|---|---|---|---|
| `esp_vhci_host_check_send_available` | `0x42080c64` | `API_vhci_host_check_send_available` `0x42098f32` | `0x42087bb2` |
| `esp_vhci_host_send_packet` | `0x42080c84` | `API_vhci_host_send_packet` `0x42098f76` | `0x42087b8c` |
| `esp_vhci_host_register_callback` | `0x42080cbe` | `API_vhci_host_register_callback` `0x42099048` | `0x42087d68` |
| `esp_bt_controller_init` | `0x42080ce4` | `btdm_osi_funcs_register` `0x42097f2c`, `btdm_controller_init` `0x4208927c` | `nimble_port_init` |
| `esp_bt_controller_enable` | `0x42080fc8` | (blob) | `nimble_port_init` |
| `esp_bt_controller_deinit` | `0x42080f88` | (blob) | `nimble_port_init`'s failure paths only |
| `esp_bt_controller_disable` | `0x42081140` | (blob) | `nimble_port_init`'s failure path only |
| `esp_bt_controller_get_status` | `0x42081218` | — | hal_radio (`0x42010b80`) |
| `bt_controller_deinit_internal` | `0x42080c26` | — | deinit, enable's failure path |

The three VHCI callers sit in one function group, `0x42087b8c` to
`0x42087d68`, which is `esp_nimble_hci.c`: `ble_hci_trans_hs_cmd_tx`/
`_acl_tx` (check, then send) and `esp_nimble_hci_init` (register
`vhci_host_cb = {controller_rcv_pkt_ready, host_rcv_pkt}`). Nothing
outside `nimble_port_init`'s failure paths disables or deinitializes the
controller, so once the radio is up it stays up (hal_radio's
`init_once`).

**hal_radio's init** (`0x42010416` onwards): `nvs_flash_init`
(`0x42101718`, erase and retry on `ESP_ERR_NVS_NO_FREE_PAGES` /
`_NEW_VERSION_FOUND`), `nimble_port_init`, `ble_hs_cfg`'s reset callback
(`0x420102dc`) and sync callback (`0x4201039e`),
`nimble_port_freertos_init` (`0x42085a16`), "waiting for host sync",
then a poll of the synced flag every 2 ticks for up to 5000 ms before
"host sync timeout". The sync callback builds the address with
`0x42010344`: `esp_read_mac(buf, 0)` (`0x42100a64`, tag `system_api`,
"mac address param is NULL"), ORs `0xC0` into byte 5, and calls
`ble_hs_id_set_rnd` (`0x420812aa`), which rejects a random part with
0 or 46 one-bits (`li a5, 0x2e`), as Core v5.x Vol 6 Part B 1.3.2.1
requires. The emulator does not model the eFuse block (notes, "Known
limitations"), so every runtime reads the same MAC, and if it reads
all zeros, the address is rejected and Connect waits out the 5000 ms
"host sync timeout" even with a working controller. Part 1 confirms
which MAC the emulator reads today and whether it comes from eFuse
registers or a ROM call.

**Lines that will not print:** `BLE_INIT` "BT controller compile version"
(in `esp_bt_controller_init`) and "Feature Config …" (bt.c's
`btdm_funcs_table_ready_wrapper`, an OSI callback the blob calls) come
from code the seam replaces.

## What the browser can do (the human partner's constraints)

- No browser can **advertise** BLE. Web Bluetooth can only scan
  (`requestLEScan`, Chrome behind the experimental-web-platform flag;
  Safari has no Web Bluetooth). So a real badge or desk can never hear
  the emulator.
- **A signaling server on Vercel, and no relay** (decided 2026-10-29,
  overriding 2026-10-22's "no servers whatsoever"): the site's own
  Vercel deployment hosts a small signaling function that passes WebRTC
  offers and answers between two tabs, replacing the copy-paste. Radio
  traffic never touches it: it flows browser to browser over WebRTC.
  Vercel Functions cannot hold WebSockets or share memory between
  invocations, so the function is plain HTTP, and pending offers and
  answers live in a store with a short expiry.

## Goals

1. **Connect and Sync open to their home screens** and never freeze
   (Part 1).
2. **Emulated badges on different devices hear each other**, through a
   WebRTC link set up through the Vercel signaling function. **One active emulator per device,
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
- Any server other than the Vercel signaling function; any relay; radio
  traffic through any server. (STUN/TURN and the signaling store are
  open below.)
- Browser or network input that can panic the emulator, or a malformed
  peer packet that can crash a tab.
- Guessed register layouts, ROM addresses or app-image addresses.

## Success criteria

1. Rungs, with a fixed fake MAC set: Connect and Sync open (`launched
   Connect`, `launched Sync`), no panic text and no `host sync timeout`,
   the firmware's radio-ready log (`BLE radio ready`), and their home
   screens' frames pinned after the human partner's eyeball check. They
   replace `connect_stops_at_the_bluetooth_controller` and
   `sync_reboots_then_stops_at_the_bluetooth_controller`, with the
   reason recorded.
2. A native two-runtime rung: two `FirmwareRuntime`s with different
   committed fake identities exchange advertisements through the fake
   controller, bump (or both hold A), and both reach "SYNCED"; the
   contact count goes to 1.
3. Sync with no peer reaches "No sync station in range" (or the
   firmware's own empty-room result) without a panic.
4. Every existing suite and pin unchanged; release `boot_progress` under
   90 s.
5. **Human gates:** Connect's and Sync's home screens; a bump between
   emulated badges on two devices, linked through the deployed signaling
   function, ending in
   "SYNCED" on both.

## Design

### Part 1 — A fake BLE controller in emulator-core (home screens)

The ROM controller cannot be run, so it is replaced as a whole by a
high-level model at the **HCI boundary**: the firmware's NimBLE host
sends standard HCI commands and receives HCI events, and the model
answers them. HCI is a published standard (Bluetooth Core Specification,
Vol 4 Part E), so the model's behaviour is specified, not guessed.

- **The seam is bt.c's VHCI layer in the app image**, at the addresses
  in "Measured on v392": `esp_bt_controller_init` and
  `esp_bt_controller_enable` succeed without running the blob and set
  `btdm_controller_status` (`0x3fca089c`) as bt.c would (INITED, then
  ENABLED), so the unhooked readers (`get_status`, the failure paths)
  stay consistent; and the three `esp_vhci_host_*` functions become the
  model's input and output. Hooking bt.c's wrappers, not the blob's
  `API_vhci_*` behind them, keeps out `async_wakeup_request` and the
  blob's sleep logic. The ROM entry points are not used: the blob calls
  hundreds of them (`esp32c3.rom.bt_funcs.ld`). The firmware has no
  symbols, so the module doc cites the evidence (call graph from `RA
  0x420806fe`, `btdm_controller_status`'s readers, the `BLE_INIT`
  strings, bt.c and `esp_nimble_hci.c` v5.5.3), the same standard as ROM
  addresses. A new hook mechanism for app-image addresses must stay
  chip-agnostic in `cpu/` (the addresses live in chip data, like
  `rom.rs`). The hooks are keyed to v392's addresses, so a firmware swap
  re-identifies them.
- **Events reach the host in task context.** The real controller calls
  `notify_host_recv` (`host_rcv_pkt`) from its own FreeRTOS task, and the
  host copies the packet and queues it for the NimBLE host task. The model
  has no task, so it must inject that call into a guest context where it is
  safe: never inside an interrupt, never where the guest cannot block or
  allocate, with every register the interrupted code needs preserved. The
  builder chooses the mechanism with evidence. Candidates: a command's
  Command Complete or Command Status delivered within the
  `esp_vhci_host_send_packet` hook (the host waits on a semaphore that
  this gives); and asynchronous events (advertising reports) delivered
  through a guest-executed trampoline (a code blob, as `rom.rs` already
  has) entered from an idle-task hook point. Each event is delivered
  exactly once, in order, at the receiving badge's emulated time.
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
- **Each badge needs its own radio identity, and it is the eFuse MAC.**
  The address is `esp_read_mac(WIFI_STA)` with the top bits set ("Measured
  on v392"), and the emulator's eFuse block is unmodeled, so every runtime
  gets the same address, probably one `ble_hs_id_set_rnd` rejects. The
  emulator gets an **opt-in per-runtime factory MAC** (host input),
  served where the firmware reads it: the eFuse registers with the layout
  from ESP-IDF v5.5.3's `efuse_reg.h`, or the ROM call, as Part 1
  measures. Part 1 also measures whether anything reads the MAC before
  the radio starts. If nothing does, rungs set it on the shared launcher
  checkpoint and the checkpoints stay shared. If something does, it is
  set at power-on, and the radio rungs walk from their own boot. With
  none set, the eFuse reads as today, so behaviour and every pin are
  unchanged. The RNG and D-M5-3 are untouched. Rungs
  that open a radio app set a fixed MAC (two different ones in Part 2);
  the browser picks a random MAC once per device and keeps it, as a real
  badge keeps its MAC.

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

### Part 4 — WebRTC link, signaled through Vercel

- **The signaling function** lives in the frontend's Vercel project
  (`frontend/api/`, next to `frontend/vercel.json`). Tab A creates a
  room and shows a short code; tab B enters it. A posts its offer, B
  fetches it and posts its answer, and A polls for the answer. Offers
  wait for ICE gathering to finish, so one exchange each way suffices
  (no trickle ICE). The function only stores and returns bounded,
  validated SDP blobs under unguessable room codes, which expire after
  minutes. It never sees radio traffic and keeps no logs of SDP.
- A tab can hold several links. Links do not forward to each other
  (radio does not hop): a badge hears only badges it is directly linked
  to.
- Peer data is validated (size, shape) before it reaches the Worker.
- `bun run dev` serves the function locally too, so development and
  tests do not need a deployment.
- Open (the human partner's choice): the signaling store; whether the
  copy-paste codes stay as a fallback; whether ICE stays host/mDNS only
  (`iceServers: []`, same network only) or adds STUN for links across
  the internet.

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
- Milestones 3 to 9 decisions stand.
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
- No servers whatsoever; WebRTC with copy-paste accepted. **Overridden
  2026-10-29:** a signaling server on Vercel replaces the copy-paste; no
  relay.
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
- Links do not forward.
- A linked peer's RSSI is a fixed strong value.
- Bump hits every linked emulated badge at once.
- An opt-in per-runtime factory MAC (eFuse) as the radio identity,
  default unchanged; it replaces the earlier "radio seed" idea, because
  the measured address comes from the MAC, not the RNG (D-M5-3
  untouched).
- The browser's MAC is random once per device and kept (per-viewer
  browser storage), so a badge keeps its identity across reloads.
- The seam is bt.c's VHCI wrappers plus controller init/enable, not the
  blob's `API_vhci_*` and not the ROM.
- Events are injected into task context only (command replies inside the
  send hook, asynchronous events from an idle-task hook point through a
  guest-executed trampoline), unless Part 1's evidence picks a better
  point.
- A pairing that turns out to need more than connectionless radio, or a
  signature fake identities can't pass, is a stop-and-ask gate, not in
  scope by default.

## Left to the builder

- The app-image hook mechanism, the event-injection point and its
  evidence; the MAC's read path; the HCI command set the model covers.
- The host API shape for advertisements; the Bump pulse; code encoding
  (compression) for WebRTC; the hub's UI placement.
- Test structure, beyond the rungs in the success criteria.

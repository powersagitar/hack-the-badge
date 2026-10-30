# Milestone 10: decisions and the Milestone 11 backlog

Milestone 10 emulates the badge radio, so Connect and Sync open and two
emulated badges can pair. Spec:
`docs/superpowers/specs/2026-10-22-milestone10-badge-radio-design.md`;
plan: `docs/superpowers/plans/2026-10-29-milestone10-badge-radio.md`.

## Why

On v392, after Milestone 9's software reset, the launcher apps that do
not open are the Bluetooth ones: Connect, and Sync after its reboot.
Both stop at mask-ROM `0x4000_08ec`, `btdm_controller_rom_data_init`
(`esp32c3.rom.bt_funcs.ld`, ESP-IDF v5.5.3), which has no stub. The spec
replaces the closed BLE controller with a fake one behind the VHCI seam,
and links emulated badges browser to browser over WebRTC, set up through
a Vercel signaling function. Three Milestone 9 backlog items ride along,
the first being the native slowdown from the per-step reset check
(Task 1, below).

## Measurements

### Measured on v392 (2026-10-29, copied from the spec)

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

### Benchmarks: the reset check off the hot loop (Task 1)

`BASE` is `e98c258`, Task 1's parent. Every later equivalence check
diffs against it.

Each row is three alternating pairs in one session (2026-10-30, same
machine), the `b25a68c` run first in each pair, from a detached
worktree of `b25a68c` outside the repo. Native is `cargo test -p
emulator-core --release --test boot_progress bench_redraws -- --ignored
--nocapture`, its `factor`; Bun is `bun run bench:wasm`, the median of
its 3 runs, its `pin_factor`. A factor of 1.0x is the badge's speed.
Every run had the same step counts on both sides (native HOME 18,292,336,
DOWN 7,942,619, onboarding 2,547,134, walk 342,600,776; Bun HOME
18,317,242, DOWN 7,972,669, onboarding 2,892,375).

**Before the change** (`b25a68c` against `BASE`):

| Redraw | Native, `b25a68c` | Native, `BASE` | Bun, `b25a68c` | Bun, `BASE` |
|---|---|---|---|---|
| HOME | 1.123x, 1.118x, 1.137x | 0.998x, 1.001x, 1.006x | 0.655x, 0.648x, 0.639x | 0.662x, 0.651x, 0.649x |
| DOWN | 1.187x, 1.182x, 1.210x | 1.051x, 1.054x, 1.058x | 0.883x, 0.872x, 0.843x | 0.911x, 0.887x, 0.890x |
| Onboarding | 1.679x, 1.674x, 1.711x | 1.493x, 1.498x, 1.506x | 1.159x, 1.146x, 1.120x | 1.188x, 1.175x, 1.164x |
| Walk, M steps/s | 72.17, 72.03, 72.24 | 66.61, 66.72, 66.12 | 42.20, 42.11, 41.55 | 42.64, 42.34, 42.05 |

This repeats Milestone 9's native finding (about 10 % slower since its
Task 3) and adds the WASM A/B Milestone 9 lacked: in Bun's WASM, `BASE`
is not slower than `b25a68c` (`BASE` ahead in every pair, by 0.5 to 6 %). The per-step reset check cost only the native
build.

**After the change** (`b25a68c` against Task 1's commit):

| Redraw | Native, `b25a68c` | Native, Task 1 | Bun, `b25a68c` | Bun, Task 1 |
|---|---|---|---|---|
| HOME | 1.104x, 1.103x, 1.109x | 1.122x, 1.119x, 1.124x | 0.635x, 0.622x, 0.637x | 0.655x, 0.653x, 0.647x |
| DOWN | 1.164x, 1.171x, 1.181x | 1.197x, 1.188x, 1.202x | 0.845x, 0.819x, 0.853x | 0.897x, 0.895x, 0.884x |
| Onboarding | 1.659x, 1.662x, 1.674x | 1.694x, 1.676x, 1.684x | 1.119x, 1.109x, 1.133x | 1.172x, 1.163x, 1.157x |
| Walk, M steps/s | 70.57, 70.29, 70.93 | 69.34, 68.89, 69.02 | 41.44, 41.08, 41.44 | 42.03, 42.00, 41.88 |

Natively the redraws are back to `b25a68c` (each pair 0.6 to 3 % ahead
of it); the whole walk is 1.7 to 2.7 % behind it (R-T1-2). Bun matches
`BASE`'s numbers above, so the change did not move WASM; it stays ahead
of `b25a68c` in every pair.

Release `boot_progress` after the change: 82 passed, 1 ignored, real
11.20 s (`/usr/bin/time -p`, warm).

## Rulings

- Ruling R-T1-1 (Task 1, `perf: take the soft-reset check off the hot
  loop`): the attention flag is cleared by the driver, not by
  `take_reset_request`, which stays as it was. `Cpu::clear_attention`
  is the one addition to the planned interface: `attend` clears the
  flag first, then takes each kind of request, so Task 3's hook hits
  join the same flag without a request being dropped by another's
  take — cost if wrong: one more public `Cpu` method; a driver that
  takes a request without clearing the flag pays one empty `attend`.
- Ruling R-T1-2 (Task 1): the redraw rows are recovered (native at or
  above `b25a68c` in every pair); the native walk throughput stays
  about 2 % under it (68.9M to 69.3M against 70.3M to 70.9M steps/s),
  and no behaviour was changed to chase it — cost if wrong: the walk,
  which only tests and benchmarks run, is about 2 % slower than it
  could be.

## Replaced rungs

| Rung | Why |
|---|---|

## Milestone 11 backlog

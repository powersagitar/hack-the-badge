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

### Benchmarks: app-image hooks (Task 3)

Three alternating pairs in one session (2026-10-30, same machine), Task
1's commit `c9d1c42` first in each pair, from a detached worktree of it
outside the repo, against Task 3's commit. The commands and columns are
Task 1's (native `factor`; Bun's median `pin_factor`). Every run had the
same step counts on both sides (native HOME 18,292,336, DOWN 7,942,619,
onboarding 2,547,134, walk 342,600,776; Bun HOME 18,317,242, DOWN
7,972,669, onboarding 2,892,375, walk 341,368,630). No hook table is
installed, so this is the cost of the check alone.

| Redraw | Native, `c9d1c42` | Native, Task 3 | Bun, `c9d1c42` | Bun, Task 3 |
|---|---|---|---|---|
| HOME | 1.145x, 1.132x, 1.134x | 1.109x, 1.102x, 1.111x | 0.652x, 0.654x, 0.651x | 0.648x, 0.640x, 0.631x |
| DOWN | 1.220x, 1.203x, 1.205x | 1.177x, 1.172x, 1.178x | 0.889x, 0.892x, 0.890x | 0.879x, 0.861x, 0.847x |
| Onboarding | 1.723x, 1.708x, 1.709x | 1.676x, 1.662x, 1.667x | 1.169x, 1.171x, 1.156x | 1.159x, 1.146x, 1.132x |
| Walk, M steps/s | 70.47, 70.32, 70.11 | 68.38, 68.11, 68.53 | 42.20, 42.32, 41.92 | 41.80, 41.50, 41.29 |

Natively Task 3 is 2.0 to 3.5 % behind `c9d1c42` in every pair; in
Bun's WASM, 0.6 to 4.8 % (the Task 3 runs drifted down across the
session while `c9d1c42`'s held). Not within noise: ruling R-T3-5.

The first build was about 9 % behind natively (HOME 1.050x, 1.036x
against 1.144x, 1.155x), and it stayed about 6 % behind with the hook
check compiled out. The cause was not the check: `run_impl` stopped
inlining `boot::idle_until`, which `run_until` calls before every step
(the hot-function sizes showed it, 1,444 bytes for `run_impl` before
against 860 after, with a call to `idle_until`). Things ruled out on the
way: the `Cpu` field order (a `repr(C)` layout keeping every hot field
at `c9d1c42`'s offset measured the same), the heap pointer in the table
(a fixed-capacity inline table measured the same) and code alignment
(both sides built with 64-byte function and block alignment kept the
gap). Forcing `idle_until` inline (ruling R-T3-4) recovered all but the
2 to 3.5 % above.

Release `boot_progress` after the change: 82 passed, 1 ignored, real
11.49 s (`/usr/bin/time -p`, warm).

### Benchmarks: a page filter in front of both checks (Task 3, fix round 1, not kept)

Orchestrator ruling R-T3-O1 asked for one coarse filter in place of
both the ROM-stub range test and the two hook ranges. A filter hit
would go to a cold path that runs the exact ROM-stub check, then the
exact hook check. Two designs were built from the tables the `Cpu`
already receives, with no chip constant:

- **A 1,024-bit bitmap** keyed by a hash of `pc >> 12`. It lost to
  `ff14e3c` natively (HOME 1.076x). A probe counted 10.3M false
  cold-path visits per launcher walk: hot flash page `0x420aa` hashed
  onto a ROM-stub page's bit. The 125 ROM stubs sit on two pages,
  `0x40000` and `0x40001`.
- **A 64-slot tagged table.** Each slot holds an exact region key
  (`pc >> shift`). On a collision between two registered keys, the
  build coarsens the key. This removes the aliasing; its hot-path test
  is ten AArch64 instructions (load the shift, shift, multiply, shift,
  load the tag, compare, branch).

The tagged table against `ff14e3c` and `c9d1c42` (2026-10-30, one
session, three rounds, each round in the order `c9d1c42`, `ff14e3c`,
filter, from detached scratchpad worktrees). The step counts were the
same in every run.

| Redraw | Native, `c9d1c42` | Native, `ff14e3c` | Native, filter | Bun, `c9d1c42` | Bun, `ff14e3c` | Bun, filter |
|---|---|---|---|---|---|---|
| HOME | 1.151x, 1.155x, 1.161x | 1.122x, 1.137x, 1.134x | 1.096x, 1.091x, 1.097x | 0.663x, 0.659x, 0.650x | 0.647x, 0.643x, 0.639x | 0.654x, 0.649x, 0.652x |
| DOWN | 1.224x, 1.226x, 1.233x | 1.191x, 1.197x, 1.202x | 1.157x, 1.152x, 1.157x | 0.914x, 0.900x, 0.887x | 0.889x, 0.876x, 0.867x | 0.894x, 0.889x, 0.895x |
| Onboarding | 1.728x, 1.727x, 1.743x | 1.672x, 1.693x, 1.696x | 1.643x, 1.634x, 1.648x | 1.192x, 1.173x, 1.170x | 1.158x, 1.145x, 1.139x | 1.179x, 1.163x, 1.167x |
| Walk, M steps/s | 71.11, 70.98, 70.78 | 69.00, 69.46, 69.55 | 68.45, 68.15, 68.54 | 42.83, 42.63, 42.12 | 41.81, 41.63, 41.49 | 42.59, 42.11, 42.35 |

- **Native:** the filter is 0.8 to 4.0 % behind `ff14e3c` in every round.
- **Bun's WASM:** the filter is 0.6 to 3.2 % ahead of `ff14e3c` in every
  round. Against `c9d1c42` it is 0.6 to 2.2 % behind in the first two
  rounds and ahead on HOME, DOWN and the walk in the third.
- **A cheaper xor-fold hash** in place of the multiply did worse natively
  (HOME 1.079x).

It was not faster than `ff14e3c` on both builds, so `ff14e3c`'s design
stays, as R-T3-O1 directs. If the browser (WASM) number is the one that
counts, the human partner may prefer the tagged filter at the Task 6
gate; R-T3-5 records the choice.

### The eFuse factory MAC (Task 4)

A scratch probe (not committed) single-stepped role hacker's whole walk,
from power-on through provisioning and onboarding to the launcher, then
A on Connect to the `0x4000_08ec` fault, exactly as `press_a_on_cell(1)`
and the Milestone 9 rung do. Before each step it decoded the instruction
from `firmware.bin`'s code segments and recorded every load or store
whose address fell in `0x6000_8800..0x6000_8a00` (`DR_REG_EFUSE_BASE`,
`soc/reg_base.h` v5.5.3), with the value loaded; it also counted every
ROM-stub call and recorded the registers at `esp_read_mac`
(`0x42100a64`), the address builder (`0x42010344`, at `0x4201035a`,
`0x4201035e` and `0x4201037a`), its caller `0x42010fc2` and hal_radio's
init (`0x42010416`). Single steps are the same run as the walk's
chunked runs: the step counts below are the rungs' (launcher at
342,600,776, `HACKER_LAUNCHER_STEPS`).

**1. Which registers carry the factory MAC: two eFuse registers, no ROM
call.** `esp_read_mac` reads `EFUSE_RD_MAC_SPI_SYS_1_REG` (`+0x48`,
twice) and `EFUSE_RD_MAC_SPI_SYS_0_REG` (`+0x44`, four times), all from
the one `lw` at `0x4200094c` (ESP-IDF's eFuse read utility), at steps
27,121,080 to 27,121,825. The field layout is `esp_efuse_table.csv`'s
`MAC_FACTORY` (block 1, six bytes at bit offsets 40, 32, 24, 16, 8, 0),
so `mac[0..2]` is `SYS_1[15:0]`, high byte first, and `mac[2..6]` is
`SYS_0`, high byte first (`emulator-core/src/peripherals/efuse.rs`). No
eFuse ROM function is called on the way: the only eFuse ROM stubs the
whole walk calls are `ets_efuse_get_spiconfig` (28 calls) and
`ets_efuse_get_wp_pad` (1), both in early boot and both unrelated to the
MAC. The other eFuse loads in the walk are early-boot reads of `+0x34`,
`+0x3c`, `+0x40`, `+0x50`, `+0x58` and `+0x6c` (block 0 and
`SYS_3`/`SYS_5`: chip revision, flash and security fields), all reading
0, plus one read of `+0x34` at step 359,410,702, inside the panic
handler. No store touches the page.

**2. Whether anything reads the MAC before `hal_radio: init_once`: yes,
during provisioning.** The only call of the address builder
(`0x42010344`) in the whole walk is at step 27,120,644, between the
first-run screen (stable by 16.5M) and onboarding page 1, long before
the launcher. It is called through `0x42010fc2` (hal_radio's "own
address": the address the sync callback stored at `0x3fcb5db4` once the
host has synced, otherwise the builder) from `0x4200bee4`
(`0x4200beda`, which returns that address and, if it is all zeros,
calls `0x420109c6` and retries). `esp_read_mac` caches what it reads in
`s_mac_table` (`components/esp_hw_support/mac_addr.c`,
`get_mac_addr_from_mac_table` line 114 and `esp_read_mac` line 354), so
on v392 the MAC is fixed for the boot at step 27.12M. From the launcher
to the fault there is no `esp_read_mac` call and no MAC-register read;
`0x42010416` is reached at step 359,285,221 and nothing after it reads
the MAC before the fault, which is before the host-sync callback
(`0x4201039e`) that calls the builder again.

**3. What `esp_read_mac` returns today: `ESP_OK` and
`00:00:00:00:00:00`.** At `0x4201035a` (the first instruction after the
call) `a0` is `0`; the buffer at `sp + 8` reads `00 00 00 00 00` and byte
5 `00` (registers at `0x4201037a` and `0x4201035e`), so the builder's
address is `00:00:00:00:00:C0`, whose 46-bit random part is all zeros,
which `ble_hs_id_set_rnd` (`0x420812aa`) rejects. Every runtime reads
the same.

**With `set_factory_mac(02:00:00:00:10:01)` before the first step**
(the same probe, Step 5): the same `lw` at `0x4200094c` reads
`SYS_1 = 0x200` and `SYS_0 = 0x1001`; at `0x4201035a` `a0` is `0`
(`ESP_OK`); the buffer reads `02 00 00 00 10` (`a0..a4` at
`0x4201037a`) and byte 5 `01` (`a5` at `0x4201035e`, before the
`ori 0xC0` makes it `0xC1`). So `esp_read_mac` returns exactly the set
MAC, and the builder's address is `02:00:00:00:10:C1`. The walk with the
MAC passed every frame assertion of `launcher_for` (role hacker's
onboarding pages, registered screen, launcher), but reached the launcher
95 steps earlier (342,600,681 against 342,600,776) and A's release at
358,750,075; Connect then stopped at the same fault, by 359,500,075.
Without a MAC every count matched the rungs.

### Connect and Sync through the fake controller (Task 6)

Scratch probes (copies of `boot_progress.rs` with an ignored probe test,
deleted before the commit) from role hacker's launcher walked with MAC
`02:00:00:00:10:01` (`radio_launcher`, 342,600,681 steps, 95 fewer than
without a MAC, as R-T4-1 measured), A on the cell as
`press_a_on_cell` does.

**Step 1, the command trace.** Phase 1, the hooks answering and
logging with no event delivered: init (`ra 0x42085892`), enable
(`ra 0x4208589c`), register callback (`ra 0x42087d6c`), then one
`check_send_available` and one `send_packet`, HCI Reset (`01 03 0c 00`),
and nothing more: the host waits for its reply, so the rest of the
sequence needs delivery. Phase 2, with delivery, Connect's open sends,
in order (H4 bytes; every reply Command Complete, Status `0x00` once the
model covers the command):

| # | Opcode | Command | Parameters (hex) |
|---|---|---|---|
| 1 | `0x0C03` | Reset | — |
| 2 | `0x1001` | Read Local Version Information | — |
| 3 | `0x1002` | Read Local Supported Commands | — |
| 4 | `0x1003` | Read Local Supported Features | — |
| 5 | `0x0C01` | Set Event Mask | `9088000200800020` (`0x2000_8000_0200_8890`: bit 61, LE Meta, set) |
| 6 | `0x0C63` | Set Event Mask Page 2 | `0000800000000000` |
| 7 | `0x2001` | LE Set Event Mask | `7ffe0f0000000000` (`0x000F_FE7F`: bit 12, Extended Advertising Report, set) |
| 8 | `0x2003` | LE Read Local Supported Features | — |
| 9 | `0x1009` | Read BD_ADDR | — |
| 10, 11 | `0x2018` | LE Rand (twice: the local IRK) | — |
| 12 | `0x202D` | LE Set Address Resolution Enable | `00` |
| 13 | `0x2029` | LE Clear Resolving List | — |
| 14 | `0x202D` | LE Set Address Resolution Enable | `01` |
| 15 | `0x2027` | LE Add Device To Resolving List | type 0, address 0, peer IRK 0, local IRK (the two LE Rand answers) |
| 16 | `0x204E` | LE Set Privacy Mode | type 0, address 0, mode `01` |
| 17 | `0x2005` | LE Set Random Address | `0200000010c1` (`C1:10:00:00:00:02`, the MAC with `0xC0`) |
| 18 | `0x2036` | LE Set Extended Advertising Parameters | handle 0, properties `0x0000` (non-connectable, non-scannable, extended), 100 ms, own address random, 1M/1M |
| 19 | `0x2037` | LE Set Extended Advertising Data | handle 0, complete, `03 ff ff ff` (manufacturer data, company `0xFFFF`, empty) |
| 20 | `0x2035` | LE Set Advertising Set Random Address | handle 0, the same address |
| 21 | `0x2039` | LE Set Extended Advertising Enable | enable, one set, no duration |
| 22 | `0x2041` | LE Set Extended Scan Parameters | own random, filter policy 0, 1M, **passive**, 150 ms / 30 ms |
| 23 | `0x2042` | LE Set Extended Scan Enable | enable, no duplicate filter, no duration |
| 24 | `0x2042` | LE Set Extended Scan Enable | disable (Connect's home) |
| 25 | `0x2039` | LE Set Extended Advertising Enable | disable |
| 26 | `0x203C` | LE Remove Advertising Set | handle 0 |

Sync's open sends 1 to 23 and stays advertising and scanning. Six of
these were not in Task 5's model and were answered Unknown HCI Command
until added (ruling R-T6-1): `0x2018` (NimBLE logged "Failed to generate
local IRK"), `0x202D`, `0x2029`, `0x2027`, `0x204E` and `0x203C`. No
vendor command, no `0x203D`. With all 26 answered the console shows no
`BLE_ERR_UNKNOWN_HCI_CMD`. Both event masks enable what R-T5-3 gates
reports on. No command asks for a connection, a scan request or GATT.

Two mask-ROM libgcc calls followed, each observed faulting and now a real
stub (ruling R-T6-4): `__popcountsi2` (`0x4000_0888`, from
`ble_hs_id_set_rnd` at `RA 0x420812d4`, counting the random address's
one-bits), then, once Connect drew, `__lesf2` (`0x4000_082c`,
`RA 0x4201ca50`).

**Step 2, the idle hook point.** `esp_vApplicationIdleHook`
(`freertos_hooks.c` line 41) is `0x4212_ac42`: it loops over eight
`idle_cb` slots (`li a5, 0x7; blt a5, s0`, table `0x3fcb5704`), then
calls `esp_pm_impl_idle_hook` (`0x420fcf68`) and `esp_pm_impl_waiti`
(`0x420fcfb0`), which calls `0x4038b8a4`, holding the image's only `wfi`
(`0x4038b8b8`). Its one caller is `prvIdleTask` (`0x40390a76`:
`prvCheckTasksWaitingTermination` at `0x4039092c`, then it, forever;
`tasks.c` line 4284), whose address goes with the name `"IDLE"`
(`0x3c133f3c`) to the task creation at `0x403926d0`. The idle task's
stack is `0x600` bytes (`vApplicationGetIdleTaskMemory`, `0x4038fbe2`:
`li a5, 0x600; sw a5, 0(s1)`).

**The asynchronous path, measured.** With Sync open (scanning), 20
advertisements were delivered one at a time (alternately 4 and 229 bytes
of data, the largest a report can carry), each followed by 200,000 single
steps: every one was queued, the queue drained through the idle hook
within 250,000 steps, nothing panicked, and the host printed no
"exceeds maximum configured HCI event buffer size", so v392's
`BT_NIMBLE_TRANSPORT_EVT_SIZE` is at least 257 (a 255-byte event plus its
2-byte header). The lowest `sp` seen in the idle task was 624 bytes below
its `sp` at the hook's entry, with the 288-byte frame of the largest
report included, against the task's 1,536.

**Sync's readiness check** (ruling R-T6-5). "A: sync now" calls
`0x4200f12e` (from `0x42043bf2`), which reads NVS namespace
`badge_upload` (`0x3c136740`), key `credential` (`0x3c136750`), a
`0x99`-byte blob, and requires its 64-byte token to be lowercase hex,
followed by a byte 0, the badge ID (`0x3fca3313`, the identity's
`badge_id`) and the claim ID (`0x3fca33c8`, `claim_id`). The fake
identities have no credential, so Sync answers "Badge not ready to sync"
/ "Visit registration to set up sync" (`0x3c13cfe8`, `0x3c13cfc4`,
branch at `0x42043bf6`) without scanning for a station; "No sync station
in range" (`0x3c13d164`, `0x42044030`) is past that check. The credential
is set over USB by `badge_token set` (`0x3c13618c`; it prints
`BADGE_TOKEN READY`, then reads 64 raw bytes from stdin, `0x4200c5c0` on).
A probe that set a throwaway token got `BADGE_TOKEN READY`, then the NVS
write faulted in ROM `crc32_le` (`0x4000_0628`, `esp32c3.rom.ld` line 415),
which has no stub.

**Frames.** Connect's home animates its rings without pause: across 240
samples 60M steps apart from 9.5M after A, 109 distinct frames, with every
change inside rows 27 to 213 and columns 67 to 253. Its title (rows 0 to
26) and footer (rows 214 to 239) never change, so the rung waits for
those bands to hold (ruling R-T6-6). After HOME the launcher comes back
with Connect selected (no reboot keeps the selection): by eye the fresh
launcher's cell 1, by pixel 143 pixels inside the icons differ in their
lowest colour bits (for example `0x9cd3` against `0x9cd4` at (40, 51)).

**Rung budgets** (each: measured, then `AFTER_PRESS_STABLE_FOR`, then
+ 25 %, rounded up to 1,000,000):

| Rung value | Measured | Budget |
|---|---|---|
| Connect: A's release (358,750,075) to bands held | 5.75M | `CONNECT_RESPONSE_STEPS` 18M |
| Connect: HOME's release (379,556,137) to the stable launcher | 13.75M | `CONNECT_HOME_RESPONSE_STEPS` 28M |
| Sync: A's release (386,878,568) to the stable home, across the reboot | 10.25M | `SYNC_RESPONSE_STEPS` 23M |
| Sync: A to "Badge not ready to sync" | drawn within the 100 ms hold | `SYNC_NOW_RESPONSE_STEPS` 10M |

Pinned (measured on v392; Connect's home, Sync's home and Sync's answer
checked by eye, saved under `local/m10-frames/`):
`CONNECT_BANDS = (0x8b40_6f5a_cbce_e924, 0xf989_f1fd_4dfa_4e43)`,
`CONNECT_HOME_HASH = 0x0fdc_677e_aff3_ed53` (the whole frame at step
372,500,075), `CONNECT_HOME_TO_LAUNCHER = 0x0f8c_1a72_61c1_3a16`,
`SYNC_HOME_HASH = 0x93d9_3053_90e7_de23`,
`SYNC_NOT_READY_HASH = 0x98b7_a768_e62d_a0e1`, `RADIO_LAUNCHER_STEPS =
342_600_681`, `RADIO_LAUNCHER_US = 94_521_615`.

Release `boot_progress` after the change: 86 passed, 1 ignored, real
11.68 s (`/usr/bin/time -p`, warm).

### Connect's pairing exchange (Task 6, Step 7: the gate reading)

Read from the firmware and confirmed in a scratch probe: two runtimes in
lockstep (1 ms slices of emulated time), hacker with MAC `…10:01` and
organizer with MAC `…10:02`, each walked to Connect, each one's
`HciController::advertisement` delivered to the other at a fixed RSSI
through `Radio::deliver` (a temporary passthrough, deleted).

- **Connectionless only.** Every command either app sends is in the
  trace above or its repeats: extended advertising (non-connectable,
  non-scannable) and **passive** extended scanning. No LE Create
  Connection, no scan request (passive scan), no ACL, no GATT. Both
  badges reached "SYNCED" over advertising reports alone.
- **The advertising payload** is one manufacturer-specific AD structure,
  company `0xFFFF` (`ff ff`), set with LE Set Extended Advertising Data:
  - idle: `03 ff ff ff` (no payload);
  - after a bump: `2c ff ff ff` + a `BMP2` card;
  - while A is held: `38 ff ff ff` + `BPA1` (`42 50 41 31`) + 8 bytes
    (seen: `31 27 13 22 b1 00 00 00`; the reading did not decode them)
    + the same `BMP2` card.

  The card (`0x42049d9e` builds it): `BMP2` (`0x3c1383d8`), version
  `02`, a flags byte, then type-length-value strings: type 1 the
  `badge_id` (`01 14 "test-fake-badge-0001"`), type 3 the display name
  (`03 0b "Test Hacker"`), and, when set, types 2 and 4 to 9 and `0x0A`
  from the identity's other fields (the fake identities have them empty,
  so the whole AD structure is 45 bytes after a bump and 57 while A is
  held, well under the 229-byte single-report limit).
- **Both hold A** advertises the `BPA1` card for about 2 s (the
  firmware stops at about +34M steps) while scanning. The receive loop
  (`0x4201ed4e`) takes a report only if its RSSI is not `0x7F` and is at
  least −64 dBm (`li a4, -0x40; blt a5, a4`), its data starts with
  `BPA1` (`0x3c155834`), it falls in a 500,000 µs window against the
  local press (`lui a5, 0x7a; addi a5, a5, 0x120`), its card decodes
  (`0x4201cc2a`, then `0x42049ec6`), its badge ID is not this badge's
  (the compare with `s0 + 0x24`), and `0x4201ec46` accepts it; then the
  contact is saved (`0x4201d98a`, `contacts_store: saved contact
  test-fake-badge-0002`). Probe: both held A from the same emulated
  time; at 50 ms "PAIRING..." / "Both press or hold A\nStay within 10
  cm", at 100 ms "SYNCED" / "Conversation Starters unlocked!" /
  "Contacts: 1" on both badges (`uitree`). At a fixed RSSI it paired at
  −40, −55, −60, −61, −62, −63 and −64, and stayed on "PAIRING..." at
  −65, −66 and −70.
- **A bump** advertises the bare `BMP2` card (no `BPA1`) for about 1 s.
  A probe's ±3 g shake of four 300,000-step halves started it on one
  badge; a shake of four 4 ms halves in lockstep did not, so the bump's
  pulse is left to Task 8 to measure; its receive branch is
  `0x4201ee96`.
- **No signature or organizer key.** The receive path compares bytes
  and strings only. The image's one signing routine is the Solana
  wallet's keypair generation (`crypto_sign_seed_keypair`, tag
  `solana`, `0x42058508`), not on this path, and the fake identities
  paired.
- **`rssi_min` and `threshold_mg`.** These keys (`0x3c138280`,
  `0x3c138270`, with `require_bump`) are read only by Chess's setup
  (`0x4201c1a2` on, section `chess`, `0x3c135e88`, through the config
  getter `0x4200baf6` with defaults `threshold_mg` 300
  (`li a2, 0x12c`), `rssi_min` −60 (`li a2, -0x3c`), `require_bump` 1),
  which feeds the shared bump-pairing module at `0x420514d6`. Connect
  does not read them: its proximity gate is the constant −64 dBm above,
  and its bump detector's threshold was not read here.
- **Event_Type.** The reports R-T5-4 builds (Event_Type `0x0000`) were
  accepted: the pairing above used them.

### The Task 6 gate: the human partner's answers (2026-10-30)

1. **The Connect and Sync home frames look right**
   (`local/m10-frames/connect-home.png`, `sync-home.png`,
   `sync-not-ready.png`).
2. **Sync's success criterion 3 is amended**: with the committed fake
   identities, "Badge not ready to sync" is accepted in place of "No
   sync station in range". Ruling R-T6-5 stands; the empty-room path is
   in the Milestone 11 backlog.
3. **Pairing scope is confirmed connectionless.** Task 7 (a connection
   model) is skipped, and a linked peer is heard at
   `LINKED_PEER_RSSI = -40` dBm.
4. **The two-range hook design and its cost are accepted** (ruling
   R-T3-5). `721f4df` against `c9d1c42`: native 1.7 to 4.4 % slower,
   Bun's WASM 0.8 to 3.9 % slower
   (`.superpowers/sdd/2026-10-29-milestone10-badge-radio/bench-721f4df.md`).
   A cheaper check, or the block cache, is in the Milestone 11 backlog.

### Two badges pair (Task 8)

Measured on v392 in release, with scratch probes (copies of
`boot_progress.rs`, deleted) and the committed rung
`two_badges_pair_to_synced`.

- **The organizer badge.** A `FIRST_RUN` clone with MAC `…10:02`,
  provisioned with the committed fake organizer identity and walked as
  `launcher_for` walks, reaches the launcher at step 342,889,379 and
  94,541,620 µs (`ORGANIZER_LAUNCHER_STEPS`, `ORGANIZER_LAUNCHER_US`).
  Hacker's MAC walk is 342,600,681 steps and 94,521,615 µs. From there
  A on cell 1 opens Connect. Its bands hold 5,750,000 steps after A's
  release, the same as hacker's, and the whole frame there is
  `CONNECT_HOME_HASH`, because Connect's home shows no name.
- **Connect's home neither advertises nor scans.** Over 3 s on the open
  Connect, `radio_advertisement()` stayed `None`. Task 6's trace shows
  the home stopping both (`0x2042` disable, `0x2039` disable, `0x203C`).
  The radio starts only on A or a bump. In the Task 6 probe, the idle
  payload `03 ff ff ff` appeared 32 ms into an A hold, and the `BPA1`
  card 6 ms later.
- **The bump detector.**
  - Connect's tick (`0x4201eff8`) calls `0x4201d250`, which
    reads the accelerometer directly (`0x4200ae2e`, under its mutex,
    `0x4200a9f2`: OUT_X..OUT_Z as `(i16)(H << 8 | L) >> 4` counts,
    converted to float, no scaling).
  - It passes the sample to `0x4201c994`, with the threshold
    `400.0f32` from `0x3c1551a4` and a hold-off of 8.
  - `0x4201c994` keeps the previous sample (state at Connect + `0x1e4`;
    the first sample only primes it). It reports a bump when the squared
    change between two consecutive samples is greater than the squared
    threshold and no hold-off is running, and then starts the 8-sample
    hold-off.
  - At the ±2 g full scale the firmware sets (`0x4200acb6` writes
    `0x80` to `CTRL_REG4`, `0x23`, through `0x4200aab4`), a
    count is 1 mg, so a bump is a change of **more than 400 mg** between
    samples. A 400 mg step never bumped; a (300, 300, 0) mg step
    (424 mg) did.
  - A bump with the radio up sets a 1,000,000 µs card window (Connect +
    `0x210`, `lui a4, 0xf4; addi a4, a4, 0x240` at `0x4201f1ce`), and
    the badge advertises the bare `BMP2` card while scanning.
  - `rssi_min` and `threshold_mg` are Chess's keys, as Task 6 read.
    Connect reads neither.
- **The sample rate.** Connect samples at its tick. Over 2 s idle, the
  gaps between samples were 40, 50, 60 and 70 ms, in runs (21 samples
  in 1 s).
  - A step held for H ms bumped only when a sample fell inside it: from
    20 phase offsets 2 ms apart, 1 ms caught 0, 4 ms caught 2, 20 ms
    caught 10, 30 ms caught 15, and 40 ms or more caught 20.
  - The first sample inside the step reports the bump. The return to
    rest falls in the hold-off, at least 8 × 40 ms = 320 ms.
  - This most likely explains why Task 6's lockstep shake of four 4 ms
    halves did not bump. A 16 ms pulse contains a sample only about a
    third of the time, and that run's phase had none.
- **The pulse chosen** (ruling R-T8-2): +1,000 mg on x, on top of the
  face-up rest (0, 0, +1,000), for 100 ms, then rest.
  - Both badges were bumped from the same emulated time, 20 ms into
    lockstep. Both advertised their `BMP2` card by the end of the pulse.
  - With 1 ms lockstep slices after it, each had saved the other's
    badge ID 36 ms later (`contacts_store: saved contact
    test-fake-badge-0002` / `-0001`). Both showed "SYNCED" (one frame,
    `SYNCED_HASH = 0x7c01_8b7f_3e8e_84e6`, the same on both badges) 93 ms
    after the pulse.
  - "SYNCED" stayed about 3.1 s. Connect's home then returned with
    "Contacts: 1" and "Conversation Starters unlocked!" on each badge.
  - Both holding A (no bump) paired too, as in Task 6. The contacts
    were saved about 120 to 130 ms into the hold (console timestamps).
    Both badges showed `SYNCED_HASH` from about 210 ms.
  - Both bumping with the swaps running during the pulse also paired:
    the contacts were saved at 85 ms after the pulse began.

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
- Ruling R-T3-1 (Task 3, `feat: chip-agnostic app-image hooks`): the
  hook check's two ranges are learned from the table, by splitting its
  sorted addresses at their largest gap, and tested as
  `pc.wrapping_sub(base) < len` each (an empty range has length 0, so an
  empty table never hits). The table is `Vec`-backed inside `Cpu`.
  `StepInfo` and `RunSummary` are unchanged: a hit is seen only through
  `needs_attention` and `take_hook_hit` — cost if wrong: a table whose
  hooks form three or more far-apart clusters makes the code between two
  of them pay the binary search on every step; a caller that wants hits
  in `StepInfo` adds a field later.
- Ruling R-T3-2 (Task 3): a pass-through belongs to the next *arrival*
  at its `pc`, not the next step: an interrupt taken in between does not
  spend it, and the hit is not reported again when the handler returns
  (pinned by `pass_through_survives_an_interrupt_before_it`).
  `set_app_hooks` drops a pending pass-through. A soft reset rebuilds the
  `Cpu`, so it drops the table, any hit and any pass-through; Task 6
  must install its table on every boot, soft resets included — cost if
  wrong: if an ISR reaches the same hooked `pc` first, the ISR's call
  runs the guest code and the interrupted call is hooked again when it
  resumes.
- Ruling R-T3-3 (Task 3): `attend` returns whether it reset, and
  `run_impl` re-derives `stop_at_ticks` only then. Behaviour is
  unchanged: before Task 3 a reset was the only thing that raised the
  flag, so the re-derivations happen at the same steps; and a
  re-derivation without a reset is a pure read of the same clock and
  `time_base_us` — cost if wrong: none for behaviour (pinned by
  `an_empty_attend_changes_nothing`).
- Ruling R-T3-4 (Task 3): `boot::idle_until` is `#[inline(always)]`.
  Adding the hooks flipped the compiler's choice for it to an
  out-of-line call before every `run_until` step, 6 to 8 % of native
  redraw speed by itself — cost if wrong: none for behaviour; a later
  change that grows `idle_until` grows `run_impl` with it.
- Ruling R-T3-5 (Task 3): the per-step hook check (`ff14e3c`'s design,
  seven AArch64 instructions: two load pairs, two subtracts, a compare,
  a conditional compare and one branch) costs, against `c9d1c42`:
  - 2.0 to 3.5 % native and 0.6 to 4.8 % in Bun's WASM (first A/B);
  - 1.6 to 3.2 % native and 1.5 to 2.9 % in Bun's WASM (fix round 1's
    session).

  This misses the plan's Step 5 "within noise". It is not accepted
  here. It is recorded by the orchestrator (R-T3-O1) for the human
  partner's decision at the Task 6 gate, together with fix round 1's
  tagged page filter: slower natively, but faster than `ff14e3c` and
  near `c9d1c42` in WASM (measurements above). No behaviour was changed
  to chase it — cost if wrong: before any hook table is installed,
  Snake and the launcher run that much slower than they could.
- Ruling R-T4-1 (Task 4, `feat: opt-in eFuse factory MAC`): the
  firmware reads the MAC before the radio (measurement 2, step
  27,120,644, during provisioning, and `esp_read_mac` caches it for the
  boot), so a rung that needs a MAC sets it at power-on, on a fresh
  `FirmwareRuntime::from_image` before its first step, and the radio
  rungs walk from their own boot: they cannot share the `REGISTERED` or
  `LAUNCHER` checkpoints, which were reached without a MAC, and setting
  a MAC on a clone of one changes nothing the firmware uses until the
  next reset. The walk with a MAC passes the same frame assertions but
  takes 95 fewer steps to the launcher, so a MAC rung pins no step
  count from the shared walk. Measured but not used here: nothing reads
  the MAC registers before step 27.12M, so a clone of the `FIRST_RUN`
  checkpoint (stable by 16.5M) with the MAC set runs as a power-on with
  it; Task 6 may start its walk there. The browser sets the MAC before
  the Worker's first run — cost if wrong: each radio rung (and each
  badge of Task 8's pair) pays its own walk to the launcher, about 5 s
  in release instead of a clone.
- Ruling R-T4-2 (Task 4): the eFuse controller is a new `FirmwareBus`
  field, `efuse`, after `rtc_cntl`, and `RTC_CNTL_RANGE` is narrowed to
  `0x6000_8000..0x6000_8800` so the two ranges are disjoint
  (`EFUSE_RANGE` is `0x6000_8800..0x6000_9000`, `DR_REG_EFUSE_BASE` to
  `DR_REG_IO_MUX_BASE`). Before, the eFuse half fell in RTC_CNTL's range
  past its modeled words: it read 0, dropped writes and logged every
  access. With no MAC set `Efuse` does exactly that, including the
  unmapped log; with a MAC set only `+0x44` and `+0x48` are served (and
  not logged), and they stay read-only. The word fast path is unchanged.
  `emulator-wasm` gets its `set_factory_mac` passthrough in Task 8, as
  the plan lists it there — cost if wrong: none for behaviour (every pin
  and both equivalence rungs are unchanged); a later eFuse field (chip
  revision) goes into the same module.
- Ruling R-T5-1 (Task 5, `feat: HCI controller model`): the model's
  sources are the Bluetooth Core Specification v5.3, Vol 4 Part E, cited
  per command in `emulator-core/src/radio/hci.rs`'s module doc from the
  specification as known, not fetched. NimBLE's `ble_hs_startup.c` lives
  in ESP-IDF's `nimble` submodule, which `raw.githubusercontent.com/
  espressif/esp-idf/v5.5.3/…` does not serve (curl failed), so the
  command set is the brief's list plus one: LE Set Advertising Set
  Random Address (`0x2035`, 7.8.52), the extended-advertising form of
  "LE Set Random Address", without which a set with a random own address
  has no address to send. LE Remove / Clear Advertising Sets and every
  other command answer Unknown HCI Command (`0x01`) until Task 6's trace
  shows them — cost if wrong: a command NimBLE's startup needs fails
  with `0x01` and the host resets; Task 6's trace names it and adds it.
- Ruling R-T5-2 (Task 5): the controller reports itself as an LE-only
  Core 5.0 controller (HCI/LMP version `0x09`, company `0xFFFF`, the
  Assigned Numbers testing value, so it claims no vendor), LE features
  bit 12 (Extended Advertising) only, buffers 251 bytes by 4 packets for
  both Read Buffer Size commands, 4 advertising sets, and a maximum
  advertising data length of 229 (the firmware's single-report limit):
  more is Memory Capacity Exceeded (`0x07`, 7.8.54). Supported Commands
  sets exactly the implemented commands' bits, from Core 5.3's table
  6.27 as known — cost if wrong: a host check on a version, feature or
  command bit takes another path; Task 6's trace shows it, and each
  value is one constant.
- Ruling R-T5-3 (Task 5): reports obey the event masks, as 7.7.65
  requires: Event Mask bit 61 (LE Meta) and LE Event Mask bit 12
  (subevent `0x0D`). With the masks at their Reset defaults no report is
  sent, so the host must enable them, as NimBLE does for the events it
  uses — cost if wrong: if the firmware's masks leave bit 12 clear,
  `deliver` returns false and Task 6's rung shows no report; the gate is
  one `if`.
- Ruling R-T5-4 (Task 5): a delivered advertisement becomes a report
  with Event_Type `0x0000` (7.7.65.13: not connectable, not scannable,
  not directed, not a scan response, not legacy, data complete), since
  the badges advertise without connections or scan responses; primary
  and secondary PHY LE 1M, SID 0, TX power `0x7F`, the RSSI clamped to
  -127..=20, no periodic interval, no direct address. The receiver sees
  the same type whatever the sender's Advertising_Event_Properties —
  cost if wrong: a firmware filter on the event type (legacy or
  connectable) drops the report; `Advertisement` then carries the type.
- Ruling R-T5-5 (Task 5): the event queue holds 32 events. A new event
  into a full queue drops the oldest advertising report; with none to
  drop, a report is refused (`deliver` returns false) and a command is
  refused unexecuted (`host_to_controller` returns false), since only a
  host ignoring its one-command credit (4.4) fills the queue with
  replies, and a command not executed owes no reply — cost if wrong:
  none for NimBLE, which waits for each reply before the next command.
- Ruling R-T5-6 (Task 5): framing errors (a length byte that disagrees
  with the packet, a packet shorter than a header) and every H4 type but
  a command (ACL, SCO, ISO, events, unknown) are refused with `false`
  and no reply; there are no connections for ACL. A known command whose
  parameters are the wrong length or out of range answers Invalid HCI
  Command Parameters (`0x12`), and every error reply carries the Status
  alone — cost if wrong: none for a correct host.
- Ruling R-T5-7 (Task 5): the duplicate filter keys on (address,
  address type, data), is reset by every Scan Enable with Enable 1,
  treats Filter_Duplicates 2 as 1 (no periods), and remembers the last
  64 keys (oldest forgotten first). A report later dropped from a full
  queue stays remembered, as a controller's filter does once it has
  sent the report — cost if wrong: a peer's unchanged advertisement
  lost to queue overflow is not heard again until scanning restarts.
- Ruling R-T5-8 (Task 5): not modeled: air timing (intervals, windows,
  Duration and Period, hence no Advertising Set Terminated or Scan
  Timeout event), PHYs other than LE 1M (a scan without 1M hears
  nothing), filter accept lists (policies validated, then ignored),
  privacy (own address types 2 and 3 act as 0 and 1), scan requests and
  responses (scan response data is stored, never sent). A random own
  address uses the set's random address, else LE Set Random Address's
  — cost if wrong: if the firmware starts advertising or scanning with
  a Duration, the timeout event it waits for never comes; Task 6's trace
  shows the parameters.
- Ruling R-T6-1 (Task 6, `feat: Connect and Sync open through a fake
  BLE controller`): the command set is the trace's. Six commands join
  Task 5's model, each with a test: LE Rand (`0x2018`, 7.8.23;
  deterministic, splitmix64 over a per-BD_ADDR seed, restarted by
  Reset), LE Set Address Resolution Enable (`0x202D`, 7.8.44), LE Clear
  Resolving List (`0x2029`, 7.8.40), LE Add Device To Resolving List
  (`0x2027`, 7.8.38), LE Set Privacy Mode (`0x204E`, 7.8.77) and LE
  Remove Advertising Set (`0x203C`, 7.8.59), with their Supported
  Commands bits (Core 5.3 table 6.27 as known: 27.7, 35.1, 34.5, 34.3,
  39.2, 37.0). R-T5-8 is revised: the resolving list now keeps
  identities (address type and address, at most 8), because LE Set
  Privacy Mode must answer Unknown Connection Identifier (`0x02`) for a
  device not on it; IRKs are not kept and nothing resolves — cost if
  wrong: a later firmware path that relies on address resolution (none
  seen) would see unresolved addresses; a wrong Supported Commands bit
  only matters to a host that checks it, and v392's sends each command
  regardless.
- Ruling R-T6-2 (Task 6): events reach the host through a guest-executed
  trampoline, `crate::rom::RADIO_CALL_TRAMPOLINE` (11 instructions built
  with `cpu/encode.rs`), mapped at `RADIO_CALL_TRAMPOLINE_ADDR`, the next
  free words of `ROM_CODE_FREE_RANGE` (`0x4003_f000..0x4004_0000`, whose
  doc shows no `esp32c3.rom*.ld` symbol points into it). At a hooked
  function's entry the host lowers `sp` by a 16-byte-aligned frame
  holding the call's `ra`, the old `sp`, up to two function pointers and
  the packet, and sets `pc` to the trampoline, which calls them and
  returns to `ra`. Context: a hooked entry is an ABI call boundary, so
  only caller-saved registers change and the callees preserve the rest;
  the packet sits in the calling task's own frame, so deliveries in two
  tasks never share memory, and an interrupt taken in the trampoline
  saves its context below the frame. Command replies are delivered
  inside the `esp_vhci_host_send_packet` hook, in the NimBLE host task,
  after `notify_host_send_available` (which gives back
  `esp_nimble_hci.c`'s `vhci_send_sem`). Asynchronous events (reports)
  are delivered from the idle task at `esp_vApplicationIdleHook`'s entry
  (`0x4212_ac42`), one per idle pass, skipping that pass's body (it
  waits for an interrupt and runs the idle callbacks; the next pass
  runs it). The idle hook is in the table only while an event waits for
  registered callbacks (`Radio` swaps the table; it never makes a
  pass-through, which is the only thing a swap drops), so without radio
  traffic it never hits. Stack: the idle task's is `0x600` bytes; the
  measured worst case, a 229-byte report, went 624 bytes below the
  hook's `sp` — cost if wrong: `host_rcv_pkt` ends in a FreeRTOS queue
  send that may block if NimBLE's event queue is full, which the idle
  task must never do. The idle task only runs when every other task is
  blocked, and each put wakes the higher-priority host task, which
  empties the queue, so the queue would have to fill while the host task
  is blocked on something else. If that is ever seen, delivery moves to
  a hook in the host task.
- Ruling R-T6-3 (Task 6): the hook table is installed on every boot
  (`from_image`, `reset` and the soft reset), and it is present on every
  run, radio app or not. It covers bt.c's `0x42080c64..0x42080fc8` and
  nothing else (the idle hook joins only with traffic), so outside
  Connect and Sync no hook is ever hit: `no_radio_without_a_radio_app`
  shows zero hits and zero inits on hacker's whole walk, and both
  equivalence rungs pass unchanged. The per-step check is the same two
  range tests as with Task 3's empty table; it was not benchmarked again
  — cost if wrong: a speed change from the non-empty ranges, which
  `bench_redraws` would show.
- Ruling R-T6-4 (Task 6): two mask-ROM libgcc stubs, each observed
  faulting once the radio was up: `__popcountsi2` (`0x4000_0888`, a new
  `Int32UnaryOp::Popcount`; `ble_hs_id_set_rnd`) and `__lesf2`
  (`0x4000_082c`, mapped to the existing `SoftFloatOp::Lt`, since
  libgcc builds `__lesf2` and `__ltsf2` from the same source;
  Connect's drawing). Both are real implementations, as the module's
  rule requires — cost if wrong: none for any earlier path, which never
  called them.
- Ruling R-T6-5 (Task 6): the planned `sync_finds_no_station` is not
  reachable with committed data, so it is `sync_now_without_a_credential_is_not_ready`
  instead: Sync checks an upload credential (NVS `badge_upload` /
  `credential`, tied to the identity's `badge_id` and `claim_id`) before
  it looks for a station, and the fake identities have none. Setting one
  needs a token typed into `badge_token set` and a ROM `crc32_le` stub
  for the NVS write; neither is done, because a committed token is
  credential data beyond the eleven fake identities (Data-handling) —
  cost if wrong: "No sync station in range" and the upload path stay
  untested; the human partner decides at the gate whether a fake token
  and the stub are wanted.
- Ruling R-T6-6 (Task 6): Connect's home animates without pause, so it
  is pinned by its title and footer bands held for
  `AFTER_PRESS_STABLE_FOR`, plus the whole frame at the first step they
  are seen held, which is deterministic. The reopened Connect is checked
  by the bands only (its ring phase differs). The launcher after HOME
  from Connect gets its own hash (`CONNECT_HOME_TO_LAUNCHER`): it keeps
  the selection, and differs from the fresh cell-1 frame in 143
  low-colour-bit icon pixels — cost if wrong: a change confined to the
  rings' rows does not move the bands, only `CONNECT_HOME_HASH`.
- Ruling R-T6-7 (Task 6): the radio rungs share one checkpoint,
  `RADIO_LAUNCHER`: a `FIRST_RUN` clone with `HACKER_MAC` set, then the
  same provisioning and walk as `launcher_for`, with every frame
  assertion, pinned at its own `RADIO_LAUNCHER_STEPS` (342,600,681) and
  `RADIO_LAUNCHER_US` (94,521,615). Beyond the planned rungs,
  `sync_after_connect_initializes_the_radio_again` pins Review Focus 2
  on the firmware itself: Connect's init, HOME, then Sync's reboot and a
  second clean init (`radio_init_count() == 2`) — cost if wrong: about
  one more walk's time in the suite (it stays at 11.68 s).
- Ruling R-T6-8 (Task 6): additions to the planned interface:
  `FirmwareRuntime::radio()` (read-only), `Radio::hook_hits`,
  `Radio::controller`, `Radio::deliver(cpu, adv, rssi)` (Task 8's
  runtime API calls it), `Radio::install`, `Radio::set_mac`/`mac` and
  `HciController::has_events`. The controller's BD_ADDR is the factory
  MAC with 2 added to its last byte (`esp_read_mac(ESP_MAC_BT)`,
  `mac_addr.c` lines 17 and 392 to 396), in HCI byte order; the
  firmware advertises from its random address, so nothing shows it —
  cost if wrong: none visible; one function.
- Ruling R-T8-1 (Task 8, `feat: two emulated badges pair to SYNCED`):
  the radio host API. `LINKED_PEER_RSSI` is `-40` dBm, approved by the
  human partner at the Task 6 gate. It lives in `emulator_core::radio`,
  and its doc cites Connect's −64 dBm gate (`0x4201ed4e`, `li a4,
  -0x40`). The peers' wire form is `addr(6) ‖ addr_type(1) ‖ data`,
  with `addr` in HCI byte order as `Advertisement` holds it, encoded
  and decoded in `emulator_core::radio` (`advertisement_to_bytes`,
  `advertisement_from_bytes`, `ADVERTISEMENT_HEADER_LEN`). The decoder
  rejects fewer than 7 bytes, more than 229 data bytes and an
  Address_Type over 3, and nothing else, so validation stays in one
  place. `FirmwareRuntime` gains `radio_advertisement`,
  `radio_advertisement_bytes` (empty while not advertising),
  `deliver_advertisement` and `deliver_advertisement_bytes`. The wasm
  passthroughs are one line each and named in the shim's camelCase
  style: `radioAdvertisement()`, `deliverAdvertisement(bytes, rssi)`
  and `setFactoryMac(mac)` — cost if wrong: Task 9 calls these three
  names; a rename touches only the shim and `bridge.ts`.
- Ruling R-T8-2 (Task 8): the bump pulse, mirrored by Task 9 in
  TypeScript, is +1,000 mg on x on top of the face-up rest, (1000, 0,
  1000) mg, held for 100 ms, then back to (0, 0, 1000). It is chosen
  from Connect's detector, not from `threshold_mg` (Chess's key, 300).
  The detector reports a change of more than 400 mg between two
  consecutive samples (measurements above). 1,000 mg is 2.5 times that
  and inside the ±2 g full scale. 100 ms is longer than the longest gap
  between samples (70 ms, measured), so every phase is sampled. It is
  shorter than the 8-sample hold-off (at least 320 ms), so the return
  to rest is not a second bump. It differs from `shake`, which is six
  alternating ±2 g halves of 62.5 ms. Measured with 500 and 1,000 mg
  steps: every hold of 40 ms or more bumped at all 20 phase offsets
  tried (0 to 38 ms) — cost if wrong: on a firmware
  whose tick slows below one sample per 100 ms, a bump is sometimes
  missed. Task 9's user then bumps again, and the pairing rung fails
  first.
- Ruling R-T8-3 (Task 8): `two_badges_pair_to_synced` pairs by bump,
  not by holding A, because the bump is now reliable. `bump(rt)` is a
  blocking pulse, run on each badge in turn from the same emulated
  time. The two advertisements are therefore swapped only after the
  pulse, at its end (one 100 ms slice), and then in 1 ms lockstep
  slices. Task 9's hub posts each change at once (R-PF-3), so in the
  browser the cards arrive sooner than in the rung. A probe with swaps
  every 1 ms during the pulse paired as well. The rung pins
  the "SYNCED" frame by its whole hash, since it holds still for about
  3.1 s. It pins the home after it by Connect's bands plus `uitree`
  ("Contacts: 1"), since the home animates (R-T6-6) — cost if wrong: a
  pairing that only works with reports arriving during the pulse is
  not exercised, and the hold-A path is covered only by Task 6's
  probe.
- Ruling R-T8-4 (Task 8): the organizer badge has its own checkpoint,
  `ORGANIZER_LAUNCHER`, walked by the shared `launcher_with_mac(role,
  mac)` (now also behind `RADIO_LAUNCHER`, unchanged) and pinned at its
  own measured `ORGANIZER_LAUNCHER_STEPS` and `ORGANIZER_LAUNCHER_US`.
  Connect opens through the shared `open_connect_from(rt)` (now also
  behind `CONNECT_OPEN`, unchanged). That function asserts the same
  `CONNECT_BANDS` and `CONNECT_HOME_HASH`, measured identical for the
  organizer, within the same `CONNECT_RESPONSE_STEPS` budget (a cap,
  not a step pin; both open in 5.75M steps). The runtime test hook
  id 1 in `runtime.rs` (`hooked_count_loop`), which collides with the
  radio's `HOOK_INIT`, is left as it was. Changing it to an id the
  radio does not know would trip the radio's `debug_assert` in debug
  builds — cost if wrong: if Connect's home ever shows the identity,
  the organizer's open fails on the hash and needs its own pin.
- Ruling R-T9-1 (Task 9, `feat: browser radio hub, one badge per
  browser, Bump`): peer input is checked twice. The hub
  (`frontend/src/radio/hub.ts`) accepts only a plain object whose `t` is
  `"adv"` or `"bump"`, and for `"adv"` a canonical padded base64 `b` of
  at most 316 characters that decodes to 7 to 236 bytes (the core
  decoder's bounds, R-T8-1); a hostile getter is caught. The Worker
  checks again that `advIn` is a `Uint8Array` of at most 236 bytes with
  an integer RSSI in -128..127, so the wasm `i8` never wraps. The core's
  decoder stays the one place that reads the bytes. The client posts
  `advIn` only while the runtime runs and the page is visible, and drops
  it otherwise, never queues it, even before `ready`. An empty local
  advertisement (stopped advertising) is posted by the Worker as a
  change but not sent to links — cost if wrong: a peer cannot tell "stopped
  advertising" from "link quiet"; its controller keeps no report state,
  so nothing waits on it.
- Ruling R-T9-2 (Task 9): the bump and the shake share one motion
  schedule in `firmware-runtime.ts`. Starting one replaces the other, and
  its first acceleration is applied at the call, not at the end of the
  next slice as the shake's was. The bump's pulse therefore starts at
  once and ends at the first slice boundary at or past `BUMP_US`, so it
  lasts 100 ms to 150 ms of emulated time (a slice is at most 50 ms),
  inside Connect's 320 ms hold-off — cost if wrong: a shake now plays its
  first half-period whole, not shortened by up to one slice. Nothing pins
  the old timing.
- Ruling R-T9-3 (Task 9): a bump the badge cannot feel is dropped, not
  replayed: a peer's bump while the hub is paused, and `bump()` on a
  client that is stopped (Lua mode, a lost badge) or hidden — cost if
  wrong: a peer bumped while this tab was hidden does not pair with it;
  the user bumps again.
- Ruling R-T9-4 (Task 9): Milestone 7's hidden-page pause (its ruling
  R-T8-10) also pauses the hub, through the client's `onPausedChange`:
  `true` before its `stop`, `false` after its `start`, so the
  advertisements the hub held (the latest per peer, in first-heard
  order) are delivered to a running badge. A `stop()` during the pause
  (a takeover's `onLost` in a hidden tab) ends it and also calls
  `onPausedChange(false)`, so the hub never stays paused after the
  runtime stops; what it held then reaches a stopped client and is
  dropped. A runtime started while the
  page is already hidden does not pause the hub; the client drops what
  arrives until the page is visible — cost if wrong: in that one case a
  peer's last advertisement is lost rather than held, and the peer's
  next re-post (at most 100 ms later, R-PF-3) replaces it.
- Ruling R-T9-5 (Task 9): one badge per browser over the Web Lock
  `htn-badge`, `ifAvailable` to acquire. `takeOver()` steals the lock and
  frees it at once; the page then acquires it as usual. The old tab's
  request ends with an `AbortError`, which runs its `onLost` (stop the
  runtime, show the notice). Across tabs that runs in the old tab's
  event loop, so the new tab can start a few milliseconds before the old
  one has stopped; within a tab, and in the fake `LockManager` test,
  the order is strict. With no `navigator.locks`, or one that throws or
  rejects, the badge runs here. The firmware is started only while the
  tab holds the badge — cost if wrong: two tabs briefly run one MAC at a
  takeover, or always where Web Locks are missing; each is its own
  emulator, so only a peer could notice.
- Ruling R-T9-6 (Task 9): the factory MAC is set once. The Worker takes
  the first well-formed `setMac` (6 bytes) before its first `start` and
  ignores every later one, and `main.ts` sends it right after creating
  the client, so it is the first queued message. It is kept in
  `localStorage["htn-badge-mac"]` as lower-case colon hex; a stored value
  that is malformed or not locally administered unicast is replaced —
  cost if wrong: a MAC changed in storage takes effect on the next page
  load only.
- Ruling R-T9-7 (Task 9): the shell's provisioning controls and the
  "Badge running in another tab — Run here" notice are built by exported
  `buildProvisionControls` and `buildBadgeNotice`, which take the
  container and a `doc`, so `test/shell.test.ts` drives them with a small
  fake DOM instead of a new DOM dependency. The notice lives in a new
  `#badge-notice` element, hidden until the badge is found taken — cost
  if wrong: the fake DOM covers only what the builders use.
- Ruling R-T10-1 (Task 10, `feat: WebRTC radio links signaled through
  Vercel, with copy-paste fallback`; the orchestrator's R-PF-T10-1):
  `hub.bump()` sends `{t:"bump"}` to every link even when the local
  runtime is not running (Lua mode, a lost badge, a hidden tab), because
  a bump is physical contact: the peer felt it whether or not this
  badge did. Only the local half is dropped (R-T9-3) — cost if wrong: a
  running peer's Connect feels a bump from a tab that cannot answer, and
  its pairing attempt times out alone, as a one-sided bump would.
- Ruling R-T10-2 (Task 10): the signaling protocol, beyond the plan's
  four calls. `GET want=answer` and `POST op:"answer"` answer 404 when
  the room's offer is gone (unknown or expired), so the host stops
  polling a dead room and the joiner gets a clear message; any method
  other than GET and POST is 405; a create that finds 8 random codes
  taken is 503. The body cap is `2 × 16 KiB + 1 KiB`, since JSON
  escaping doubles an SDP's CRLFs; the SDP itself is capped at 16 KiB
  characters. Keys are `htn-signal:<room>:offer|answer`, each with its
  own 300 s TTL from when it was written. Every reply is
  `Cache-Control: no-store`. A store failure is a bare 500; nothing is
  ever logged — cost if wrong: an answer written at 299 s outlives its
  offer by up to 300 s, harmless since nobody polls it.
- Ruling R-T10-3 (Task 10): `frontend/api/signal.ts` uses the `fetch`
  Web Standard export (`export default { fetch(request) }`), per
  https://vercel.com/docs/functions/functions-api-reference (checked
  2026-11-02). It reads only `KV_REST_API_URL` and `KV_REST_API_TOKEN`
  (a missing one is a bare 500), reaches `process.env` through
  `globalThis` (no Node types), and imports `../src/radio/signal-handler.js`
  with an explicit extension; `signal-handler.ts` imports nothing, so
  Node ESM resolves the function unbundled. `tsconfig.json` now includes
  `api/`. Upstash goes through its REST API over `fetch`, with no
  client library — cost if wrong: if Vercel's TypeScript build does not
  map the `.js` specifier to the `.ts` file, `/api/signal` fails at
  import on the first preview deploy, and the fix is a one-file inline.
- Ruling R-T10-4 (Task 10): a link joins the hub as soon as it exists
  (its `send` drops everything until the channel opens) and leaves the
  hub when it closes; a closed link stays in the list, "disconnected",
  until its Remove. One hosted room at a time: Host link again cancels a
  room nobody joined yet. The 120 s budget (1 s polls) runs from the
  start of `hostRoom`/`joinRoom` and covers both the answer and the
  channel opening; at its end the link is closed with "Could not
  connect to the peer: no direct path…". A join resolves only once the
  channel is open, so a joining link enters the list already
  connected. Typed room codes follow Crockford's decoding (case-free,
  hyphens and spaces ignored, I/L → 1, O → 0) — cost if wrong: a slow
  ICE path that would open after 120 s is given up on; the user links
  again.
- Ruling R-T10-5 (Task 10): manual codes are `offer:` or `answer:` plus
  the SDP, `deflate-raw` compressed and base64url without padding, so a
  pasted code says which step it is, and an answer pasted where no offer
  waits gets a message. Whitespace in a pasted code is ignored. Decoding
  never rejects: a code over 32 KiB, bad base64url, a deflate error, an
  inflated size over 16 KiB (a deflate bomb is cut off while reading),
  or invalid UTF-8 gives `null`. A manual link has no timeout; it stays
  "connecting…" until removed — cost if wrong: an abandoned manual offer
  holds one idle `RTCPeerConnection` until Remove or unload.
- Ruling R-T10-6 (Task 10): a `PeerLink` passes on only a string data
  message of at most 1024 characters that parses as JSON (the largest
  real one is about 330); the hub then validates the shape (R-T9-1). A
  throwing handler is caught inside the channel callback. ICE gathering
  is waited on for at most 10 s, after which the SDP goes out with the
  candidates gathered so far — cost if wrong: a future, larger hub
  message over 1024 characters would be dropped silently; the cap must
  grow with it.
- Ruling R-T10-7 (Task 10): the link panel lives in a new `#link-panel`,
  shown only in real-firmware mode like the provisioning controls, and
  is built by an exported `buildLinkPanel(container, doc, handlers)` in
  `shell.ts` (the R-T9-7 pattern). Its order: "Host link", the room code
  input, "Join link", "Manual", the status line, the manual section
  ("Create code", the code to copy, the code to paste, "Use code"),
  then the link list, each row "Link n (room CODE|manual) — state" with
  "Remove". The dev server serves `/api/signal` through a Vite plugin
  (`apply: "serve"`) over a `MemoryStore` — cost if wrong: dev rooms
  vanish when the dev server restarts.

## Replaced rungs

| Rung | Why |
|---|---|
| `connect_stops_at_the_bluetooth_controller` | Pinned Milestone 9's stop: Connect fetching mask-ROM `0x4000_08ec` (`btdm_controller_rom_data_init`). The fake controller replaces that path at bt.c's VHCI seam, so the fetch no longer happens; replaced by `connect_opens_with_the_radio_up` (and `connect_reopens_after_home`). |
| `sync_reboots_then_stops_at_the_bluetooth_controller` | Pinned the same stop after Sync's reboot. Replaced by `sync_opens_after_its_reboot` (and `sync_after_connect_initializes_the_radio_again`); the reboot line and the one soft reset are still asserted. |
| `run_to_the_bluetooth_fault` (helper, with `BLUETOOTH_ROM_FAULT_PC` and `BLUETOOTH_ROM_FAULT_MEPC`) | Used only by the two rungs above; nothing reaches the fault any more. |

## Milestone 11 backlog

- **Sync's empty-room path** ("No sync station in range"). Sync checks
  an NVS upload credential (`badge_upload` / `credential`) before it
  looks for a station (ruling R-T6-5). Reaching that screen needs a fake
  credential typed through `badge_token set`, cleared with the
  organizers like the fake identities. It also needs a ROM `crc32_le`
  stub (`0x4000_0628`) for the NVS write.
- **A cheaper hook check, or the block cache.** The two-range check
  costs 1.7 to 4.4 % natively and 0.8 to 3.9 % in Bun's WASM
  (`721f4df` against `c9d1c42`), accepted at the Task 6 gate. A block
  cache (shelved in Milestone 8) would test the hook ranges once per
  block, not once per step.

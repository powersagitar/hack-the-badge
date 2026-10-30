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

## Replaced rungs

| Rung | Why |
|---|---|

## Milestone 11 backlog

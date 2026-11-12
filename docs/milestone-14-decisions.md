# Milestone 14: decisions

Milestone 14 redesigns the badge page. Spec:
`docs/superpowers/specs/2026-11-12-milestone14-badge-page-redesign-design.md`;
plan: `docs/superpowers/plans/2026-11-12-milestone14-badge-page-redesign.md`.

## Human decisions (2026-11-12)

- **Friendly identities override Milestone 5's naming.** Milestone 5
  recorded that the committed fixtures are named "Test <Role>"
  (`docs/milestone-5-decisions.md`). The human partner overrode that
  decision on 2026-11-12 in favour of the spec's friendly names
  (historical figures only, no living person; "Friendly identities").
  Everything else Milestone 5 required still holds: `example.com`
  addresses, `test-` badge IDs, the same attendee and claim IDs, empty
  contact fields, and no data from the flash dump or the boot log.
  `frontend/test/test-identities.test.ts` now checks each role's
  `display_name` and `account_email` exactly against the spec's table,
  in place of the old `"Test "` prefix check.
- **Task 1's re-pins approved.** The human partner approved the table
  below after viewing the renamed frames (all 11 roles' onboarding page
  1 and registered screen, and hacker's pages 2 and 7). No name is
  truncated; "Katherine Johnson", the longest, renders whole.

## Re-pins (Task 1)

Every new value was measured with the renamed fixtures, never derived.
The reason for every row is **fixture rename (spec: Friendly
identities)**. Each moved pin is on a chain that types the identity into
the console. The frame pins show the name. The other pins move because
the identity bytes differ, so the provisioning, and everything after it,
runs a different number of steps. No emulator code changed.
`execution_digest_is_unchanged` (20M steps of a blank-flash boot, with
no provisioning) did not move, and neither did any rung that never
provisions.

All constants are in `emulator-core/tests/boot_progress.rs`.

### Frames that show the name

| constant | old | new |
|---|---|---|
| `ROLE_ROWS` hacker page 1 | `0xbf9c_cb7e_80b4_4b1a` | `0x1c45_937d_99c0_d2ec` |
| `ROLE_ROWS` hacker registered | `0xbd6d_7613_08a7_bf24` | `0x83ca_0950_da3c_55da` |
| `ROLE_ROWS` organizer page 1 | `0x3c31_f95a_a12e_406f` | `0xbc11_2320_8e0c_d357` |
| `ROLE_ROWS` organizer registered | `0x1974_c49f_a562_08a3` | `0xaf2c_3ce0_0305_d503` |
| `ROLE_ROWS` sponsor page 1 | `0xc5ba_681b_7473_21bc` | `0x48c4_61eb_3bb4_f4d0` |
| `ROLE_ROWS` sponsor registered | `0x0bc2_dee2_1a99_c4c0` | `0xa70c_5cc0_123b_93d4` |
| `ROLE_ROWS` judge page 1 | `0x192b_1f45_de51_5d49` | `0x4d1c_e946_d7eb_3e87` |
| `ROLE_ROWS` judge registered | `0x54f8_e3d0_65a3_04c1` | `0x68a9_5319_23e9_6293` |
| `ROLE_ROWS` mentor page 1 | `0x4929_00a1_98c0_3e4d` | `0xae1a_f160_422b_d853` |
| `ROLE_ROWS` mentor registered | `0x03a9_7611_bf45_9332` | `0xa043_1930_243e_4ff0` |
| `ROLE_ROWS` volunteer page 1 | `0xd370_a34f_7bc7_d738` | `0x017b_6081_6992_ce44` |
| `ROLE_ROWS` volunteer registered | `0xfd95_a0e7_844f_4019` | `0xf09f_89b2_0ac6_9c99` |
| `ROLE_ROWS` media page 1 | `0xa0b0_7c3c_ff8f_289d` | `0x652d_f154_e30b_f6c7` |
| `ROLE_ROWS` media registered | `0x5092_210f_e8d8_f1d2` | `0x04f3_99ce_2b61_f4fc` |
| `ROLE_ROWS` staff page 1 | `0xe912_c748_aab7_152c` | `0x65db_0aa2_2612_d47c` |
| `ROLE_ROWS` staff registered | `0xc68e_18b7_16ef_40ea` | `0x5dc5_e488_daae_02b2` |
| `ROLE_ROWS` general page 1 | `0x0c04_ea57_eeb8_f8ec` | `0xa713_09fb_3fff_a163` |
| `ROLE_ROWS` general registered | `0x4574_d9ed_d13c_330a` | `0xc566_be81_c7bf_1829` |
| `ROLE_ROWS` workshop_lead page 1 | `0x96a7_9955_fffc_187c` | `0x7601_ff30_62bc_d767` |
| `ROLE_ROWS` workshop_lead registered | `0xf8e2_3246_19a6_4d76` | `0x2cf2_489d_7b3e_31e1` |
| `ROLE_ROWS` visitor page 1 | `0x2770_ab89_5a47_e576` | `0x26c7_39db_80ec_f6be` |
| `ROLE_ROWS` visitor registered | `0x647a_71d4_5ab8_cf23` | `0xea69_0e22_0d10_1b23` |
| `TO_SHAKE_PAGE[0]` (hacker page 2, "This is you") | `0xe788_a781_9328_1b08` | `0xfea7_9ef1_3a7f_bbea` |
| `ONBOARDING_PAGE7_HASH` (hacker page 7, "Badge Connect", its "Badge name:" row) | `0x110e_8782_bc4e_5ff0` | `0x7f0c_e585_b026_c6c0` |

None of hacker's other 15 walk frames moved, and neither did any
launcher or app frame. The `ROLE_ROWS` doc comment now says that
hacker's row was compared with the physical badge for layout on v335,
and that the renamed frames were checked by eye in Milestone 14.

### Chain totals

| constant | old | new |
|---|---|---|
| `HACKER_LAUNCHER_STEPS` (`hacker_walk_totals_are_unchanged`) | `342_600_776` | `342_657_333` |
| `HACKER_LAUNCHER_US` (`hacker_walk_totals_are_unchanged`) | `94_521_615` | `94_501_734` |
| `HACKER_LAUNCHER_CONSOLE_HASH` (`hacker_walk_totals_are_unchanged`; the console echoes the typed identity) | `0xabcd_cc17_7a35_80fe` | `0xd9e7_08f4_1965_a9b8` |
| `RADIO_LAUNCHER_STEPS` | `342_600_681` | `342_657_243` |
| `RADIO_LAUNCHER_US` | `94_521_615` | `94_501_734` |
| `CREDENTIAL_LAUNCHER_STEPS` | `342_894_540` | `342_985_252` |
| `CREDENTIAL_LAUNCHER_US` | `94_521_622` | `94_521_739` |
| `ORGANIZER_LAUNCHER_STEPS` | `342_889_379` | `342_921_401` |
| `ORGANIZER_LAUNCHER_US` | `94_541_620` | `94_601_811` |
| `SKIP_REGISTERED_AT_STEPS` | `84_392_000` | `84_742_000` |
| `SKIP_REGISTERED_AT_US` | `8_855_536` | `8_893_591` |

`RADIO_LAUNCHER_STEPS` is now 90 steps fewer than
`HACKER_LAUNCHER_STEPS`; ruling R-T4-1 had measured 95.

### Console needle

| constant | old | new |
|---|---|---|
| `SYNC_SNAPSHOT_LINE` | `"sync: snapshot 1026 bytes, crc ec2f90df,"` | `"sync: snapshot 1028 bytes, crc f7cd2dea,"` |

Sync's snapshot packs the identity: the name and the email are each one
byte longer than before. The doc comment now quotes the new full line,
`sync: snapshot 1028 bytes, crc f7cd2dea, 37ms, complete=1`, and keeps
the old values.

### Redraw pins (bench only)

The ignored `bench_redraws` checks these; they all run on hacker's
provisioned chain. They are copied into `frontend/src/bench/redraw-bench.ts`,
and `frontend/test/redraw-bench.test.ts` pins DOWN's.

| constant | old | new |
|---|---|---|
| `HOME_REDRAW.steps` | `18_292_336` | `18_318_598` |
| `DOWN_REDRAW.emulated_us` | `109_000` | `129_000` |
| `DOWN_REDRAW.steps` | `7_942_619` | `7_978_324` |
| `ONBOARDING_REDRAW.steps` | `2_547_134` | `2_577_629` |

DOWN's last frame change moved by 20 ms on an unchanged launcher frame
(`LAUNCHER_AFTER_NAV_HASH`). The chain reaches the launcher at a
different emulated time, so DOWN's redraw ends at a different point;
this is not a frame that shows the name. The end hashes are unchanged.

### Frontend

- `frontend/src/bench/walk.ts`: `OWN_ROLE_PAGE1_HASH`,
  `OWN_ROLE_REGISTERED_HASH`, `TO_SHAKE_PAGE[0]` and
  `AFTER_SHAKE_PAGES[0]` take the same new values as above. The WASM
  twins in `test/cpu-wasm.test.ts` use them.
- `frontend/test/test-identities.test.ts`: the exact name/email table
  (see "Human decisions").
- `frontend/test/provisioner.test.ts`: the fake console lines say
  "Ada Lovelace" and "Grace Hopper".
- No frontend test pinned a fixture byte count.

## Dependencies added

All pinned exactly in `frontend/package.json`.

| package | exact version | why |
|---|---|---|
| `react` | 19.3.1 | The page's UI (Milestone 14 replaces `ui/shell.ts`'s DOM building with components). |
| `react-dom` | 19.3.1 | Renders the React page into `#root`. |
| `matter-js` | 0.20.0 | The lanyard and badge physics (the design's swing). |
| `radix-ui` | 1.8.1 | Primitives under shadcn's Button (`Slot`), Select and ScrollArea; added by the shadcn CLI. |
| `class-variance-authority` | 0.7.1 | shadcn's Button variants (`cva`). |
| `clsx` | 2.1.1 | shadcn's `cn` (`src/lib/utils.ts`). |
| `tailwind-merge` | 3.7.0 | shadcn's `cn` (`src/lib/utils.ts`). |
| `lucide-react` | 1.72.0 | The Select's chevron and check icons. |
| `tailwindcss` (dev) | 4.3.5 | The styling and theme tokens. |
| `@tailwindcss/vite` (dev) | 4.3.5 | Tailwind v4's Vite plugin. |
| `@vitejs/plugin-react` (dev) | 6.3.0 | JSX/React in Vite 8. |
| `@types/react` (dev) | 19.3.0 | React's types. |
| `@types/react-dom` (dev) | 19.3.0 | react-dom's types. |
| `@types/matter-js` (dev) | 0.20.2 | matter.js's types. |
| `@testing-library/react` (dev) | 16.3.3 | Component tests over happy-dom. |
| `@testing-library/dom` (dev) | 10.4.2 | `@testing-library/react`'s required peer. |

The shadcn components came from the CLI, `bunx --bun shadcn@4.23.0 add
button select input scroll-area` (not a dependency). Its current
`new-york-v4` registry imports `cn` from shadcn's new npm package `cn`
(0.5.2, published 2026-11-02) and lists it as a dependency. We kept the
classic `src/lib/utils.ts` (`clsx` + `tailwind-merge`) instead, as the
plan asks, rewrote the four components' import to `@/lib/utils`, and did
not add `cn`. The CLI also resolves the `@` alias only with a tsconfig
`baseUrl`, which TypeScript 7 removed (TS5102), so `baseUrl` was present
only while the CLI ran; `paths` alone serves `tsc`, Bun and Vite (whose
`@` alias is in `vite.config.ts`).

Removed: `fengari` (^0.1.5) and `fengari-interop` (^0.1.4), removed in
Task 4 with the Lua sandbox. Task 1 added none.

## Build size (Task 7)

`bun run build`, `frontend/dist/`:

- Before (main.ts page): JS 27.40 kB (gzip 10.31 kB) + bridge 6.24 kB +
  worker 10.81 kB, CSS 2.71 kB, wasm 143.64 kB; `dist/` 3380 KiB.
- After (the empty React shell): JS 222.71 kB (gzip 69.81 kB), CSS
  22.03 kB (gzip 5.12 kB); `dist/` 3428 KiB.

Until Task 12 wires the controller, the page does not reference the
firmware Worker, so the worker, bridge and wasm chunks are absent from
the build; they return then.

# Milestone 5 — Provisioned Boot to the App Launcher — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provision the emulated badge through the real firmware's own USB console (`put` + `prov apply`) and boot from My Badge's first-run screen to the app launcher, for every role the firmware knows, proven by native rungs, a WASM twin and a human comparison; stretch: open a built-in app.

**Architecture:** Task 1 traces the provisioning path in `factory.bin` (no emulator code). Task 2 adds the USB-Serial-JTAG receive direction (host queue, paced 64-byte OUT packets, `INT_ST`, interrupt source 26), which should also unblock `app_main`'s console start. Task 3 adds the shared provisioning test helper and the identity-free console rungs. Task 4 explores to the launcher with local-only fake identities; Task 5 (frames) and Task 6 (organizer permission) are human gates; Task 6 commits fixtures and identity rungs; Task 7 is the frontend; Task 8 docs; Task 9 the stretch app.

**Tech Stack:** Rust 2021 (`emulator-core`, `emulator-wasm` via wasm-bindgen/wasm-pack), Bun + TypeScript frontend, ESP-IDF v5.5.3 sources as the register authority.

**Spec:** `docs/superpowers/specs/2026-10-07-milestone5-provisioned-launcher-design.md` — read it before starting any task, including its "Found while planning" paragraph and the rulings R-M5-1..5. Also read `docs/milestone-3-decisions.md` and `docs/milestone-4-decisions.md` (decisions that stand) and `docs/firmware-emulator-notes.md` "Known limitations" and "Data-handling note".

## Handoff state (as of 2026-10-07)

- Branch `milestone-5`, based on `main` @ `f7ee032` (= `origin/main`; M4 is merged and pushed). The spec is committed on it. Work happens in the worktree `.claude/worktrees/milestone-5` on that branch (`git worktree add .claude/worktrees/milestone-5 milestone-5`); copy the repo-root `local/` into it (gitignored; never commit it).
- Baseline: `cargo test --workspace`, `cargo test -p emulator-core --release --test boot_progress` (35 tests, ~2.6 s), `bun run build:wasm`, `bun test`, `bun run typecheck` all green at `f7ee032`; `cargo clippy --workspace --all-targets` has 3 pre-existing warnings.
- Measured while planning: on blank flash the emulator prints `app_reg: launched My Badge` (~12.29M steps) and never `hal_console: console started`, even at 60,000,000 steps. On the physical badge that line follows the launch by about ten untagged lines and is followed by `main_task: Returned from app_main()`. The REPL prompt is `badge> `.
- Strings in `factory.bin` relevant here (firmware data, safe to cite): `usage: put <path> <size>`, `Upload file: put <path> <size> then send <size> bytes`, `OK %ld`, `bad size`, `write error`, `short read: %ld bytes missing`, `PROV OK id=%s`, `PROV FAIL invalid or missing %s`, `provisioned=%d`, `usage: prov <apply|show|mac|erase confirm>`, `  provision flow: put %s <size> ... then: prov apply`, `name=%s role=%s attendee=%lu tutorial=reset`, `PROV FAIL display busy`, `hal_identity`'s `identity.json size %ld out of range`, `%s: %u bytes, max %u`, `unknown role '%s', defaulting to hacker`, `badge_id is missing or empty`, `display_name is missing or empty`, and the role strings `hacker organizer sponsor judge mentor volunteer media staff general workshop_lead visitor`.
- `local/` (gitignored) holds `full_flash_dump.bin` and `boot_log.txt` (personal data: never commit, quote or print values; never paste into a subagent brief), `local/.venv/` (the only Python; littlefs-python installed), `local/rom-elfs/`, `local/m5-identity/FINDINGS.md` (structure only), `local/m4-sdd-ledger.md`.
- Orchestrator ledger: `local/m5-sdd-ledger.md` (gitignored), one line per task/review/fix round/ruling, stall facts, step counts.
- ESP-IDF sources: fetch from `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>` into the session scratchpad, never into the repo.

## Global Constraints

- Register-faithful: every modeled register behavior cites its ESP-IDF v5.5.3 header / HAL / LL source in the module doc comment. Never guess an offset or bit.
- No PC-based interception of ESP-IDF (non-ROM) code. ROM functions only, via `src/rom.rs` + `cpu/rom_stubs.rs`, addresses from `components/esp_rom/esp32c3/ld/esp32c3.rom*.ld`. An observed ROM call that faults gets a real-implementation stub in the task that hits it (M4 R10/R12).
- Finish-line tests are never narrowed or loosened; a hash changes only with a documented display-model reason.
- `FirmwareBus` dispatch stays an ordered sequence of concrete named fields; no `dyn` dispatch. `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays logic-free (one-line passthroughs).
- Unmapped data access never panics (reads 0, writes drop, logged); unmapped instruction fetch always traps. Browser-supplied input (image bytes, serial bytes) never panics the emulator.
- Never commit, quote or print values from `local/full_flash_dump.bin`, `local/boot_log.txt` or anything derived from them (including subagent briefs and reports). Contact filenames are other attendees' badge IDs. Committed tests depend only on `factory.bin` plus committed synthetic data.
- R-M5-1: every committed identity-format fact comes from tracing `factory.bin` and cites a firmware address; `local/m5-identity/FINDINGS.md` is a local cross-check only.
- R-M5-2: fake identities live only under `local/m5-identities/` until the human partner relays the organizers' permission (Task 6). Pre-gate commits contain no identity and no identity-using test.
- R-M5-3: an offline signature/HMAC check, or binding to the chip MAC/eFuse, found anywhere on the provisioning path → stop and ask. No workaround.
- R-M5-5: `EmulatedFlash` starts blank (partition table + `factory.bin`); the emulator never writes littlefs or NVS contents itself.
- `docs/milestone-3-decisions.md` and `docs/milestone-4-decisions.md` stand; overriding one needs the human partner's OK and a recorded reason.
- Python via `local/.venv/bin/python` only. JS/TS via Bun only.
- After any change under `emulator-core/` or `emulator-wasm/`, before commit: `cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck`, all green, no new clippy warnings.
- Pushing is the human partner's call. Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A word read of `EP1_REG` pops exactly one byte.** `lw` reaches the peripheral as four ascending `read_byte`s; only byte lane 0 is `RDWR_BYTE`. Popping on every lane would silently eat three of every four input bytes. Pinned by Task 2's `word_read_of_ep1_pops_exactly_one_byte`.
2. **Serial input must wake a core idling in WFI.** The fast-forward jumps to the next SYSTIMER alarm; if it ignores a pending packet, a typed command waits for the next FreeRTOS tick at best, forever at worst. Pinned by Task 2's `idle_fast_forward_stops_at_the_next_host_packet`.
3. **A huge or garbage serial input from the browser** (megabytes, NULs, invalid UTF-8) must not panic, freeze the step loop or grow memory without bound. Pinned by Task 2's `host_queue_is_capped_and_refuses_the_excess`.
4. **Clicking "Provision" before the console is up** (during the boot splash) must still work: the provisioner waits for the `badge> ` prompt instead of typing into a REPL that does not exist yet. Pinned by Task 7's `provisioner waits for the prompt before typing`.
5. **Reset or a second click mid-provisioning** must not leave a stale provisioner typing into the rebooted firmware. Pinned by Task 7's `reset abandons an in-flight provisioner`.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `docs/milestone-5-decisions.md` | Create (Task 1), grow per task | Rulings, trace facts, decisions with "if wrong", M6 backlog |
| `docs/milestone-4-decisions.md` | Modify (Task 1) | Point the "never from the dump" sentence at R-M5-1 |
| `emulator-core/src/peripherals/usb_serial_jtag.rs` | Modify (Task 2) | RX: host queue, OUT FIFO, pacing, `INT_RAW`/`INT_ST`/`INT_CLR`, interrupt source |
| `emulator-core/src/mem/soc.rs` | Modify (Task 2) | `SRC_USB_SERIAL_JTAG` |
| `emulator-core/src/mem/bus.rs` | Modify (Task 2) | `pending_sources`, `tick_peripherals`, `ticks_until_next_event`, `advance_idle` |
| `emulator-core/src/boot.rs` | Modify (Task 2) | Idle fast-forward through the two bus methods |
| `emulator-core/src/runtime.rs` | Modify (Task 2) | `serial_input`, `serial_pending` |
| `emulator-core/tests/common/mod.rs`, `emulator-core/tests/common/provision.rs` | Create (Task 3) | Console-driving helpers shared by rungs |
| `emulator-core/tests/boot_progress.rs` | Modify (Tasks 3, 6, 9) | Console rungs, per-role rungs, launcher rungs, app rung |
| `frontend/public/firmware/test-identities/*.json` | Create (Task 6) | One fake identity per role |
| `emulator-wasm/src/lib.rs` | Modify (Task 6) | `serialInput`, `serialPending` passthroughs |
| `frontend/src/cpu/bridge.ts` | Modify (Task 6) | Handle gains `serialInput`, `serialPending`, `consoleOutput` |
| `frontend/src/runtime/provisioner.ts` | Create (Task 6) | Console-typing state machine, shared by the WASM twin and the UI |
| `frontend/test/cpu-wasm.test.ts`, `frontend/test/provisioner.test.ts`, `frontend/test/test-identities.test.ts` | Modify/Create (Task 6) | WASM twin, provisioner unit tests, fixture guard |
| `frontend/src/runtime/test-identities.ts` | Create (Task 6) | Role list + fixture URL |
| `frontend/src/runtime/firmware-runtime.ts`, `frontend/src/ui/shell.ts`, `frontend/src/main.ts`, `frontend/index.html`, `frontend/src/ui/shell.css` | Modify (Task 7) | Role picker + "Provision test badge" |
| `docs/firmware-emulator-notes.md`, `CLAUDE.md` | Modify (every task touches notes; Task 8 finalizes) | Current state, history, architecture |

---

### Task 1: Trace the provisioning path (no emulator code)

**Files:**
- Create: `docs/milestone-5-decisions.md`
- Modify: `docs/milestone-4-decisions.md` (the backlog bullet "Reaching the app launcher needs a provisioned identity", ~L316–327)
- Modify: `docs/firmware-emulator-notes.md` (new history entry "Milestone 5 Task 1: the provisioning path")
- Local only: `local/m5-trace/` (scratch disassembly, notes), `local/m5-identities/<role>.json`

**Interfaces:**
- Produces (in `docs/milestone-5-decisions.md`, section "Trace facts", one bullet each with the firmware address it came from; Tasks 3, 4, 6 and 7 read these instead of re-deriving):
  - `PROVISIONED_FLAG_WRITER`: the code that sets `0x3fca_92b2` and its inputs (identity.json only, or also NVS `badge_upload/credential`).
  - `VALIDATION`: size range; required non-empty fields; per-field maximum lengths (a table field → max bytes); numeric fields and their accepted ranges; the role lookup and its fallback; `version` handling.
  - `ROLE_TABLE`: the ordered role strings the lookup accepts.
  - `CONSOLE_START`: who calls `hal_console_start()` and the condition, and what `app_main` is blocked on in the emulator (with hot PCs / the FreeRTOS object it waits on).
  - `REPL_LINE_END`: what ends a command line (`\n`, `\r` or either) given linenoise's mode with no terminal answering its probe.
  - `PUT_READ`: how `put` reads its `<size>` bytes (stdin `FILE` via `fread`/`fgetc`, or raw `read()` on the fd), and whether bytes sent before the command starts reading can be lost.
  - `APPLY_EFFECTS`: what `prov apply` does after `PROV OK` (app switch, redraw, `tutorial=reset` meaning, onboarding, `esp_restart`).
  - `R_M5_3`: the explicit finding that no signature/HMAC verification and no MAC/eFuse binding exists on this path (or the stop).

- [ ] **Step 1: Extract and index the image.** Write a scratch Rust example `emulator-core/examples/local_m5_trace.rs` (add the line `emulator-core/examples/local_*` to `.git/info/exclude` first so no `local_*` example is ever committed) that parses `factory.bin` with `emulator_core::mem::image` and writes each segment to `local/m5-trace/seg_<load_addr>.bin`. Disassemble the IROM segment with `riscv64-unknown-elf-objdump -b binary -m riscv:rv32 -M no-aliases -D --adjust-vma=<load_addr> local/m5-trace/seg_42000020.bin > local/m5-trace/irom.S` (if objdump is unavailable, add a disassembly mode to the scratch example using `emulator_core::cpu`'s decoder). Locate each string from the Handoff list in the DROM segment and record its address in `local/m5-trace/strings.txt`; find its references in `irom.S` (the `lui`/`addi` pairs that form the address).

- [ ] **Step 2: `CONSOLE_START`.** From the reference to the string `void hal_console_start()` (an assert function-name string) and `hal_console: console started`, find `hal_console_start`'s entry and its caller(s) in `app_main`. Then run `cargo run -p emulator-core --release --example boot-probe -- --steps 20000000 --window 2000000` and identify where the `main_task` (app_main) task is blocked: from the hot PCs and, if needed, a scratch example that single-steps and logs when the PC enters `hal_console_start`, `usb_serial_jtag_driver_install`, `esp_console_new_repl_usb_serial_jtag`, `usb_serial_jtag_write_bytes` and `xRingbufferSend`/`xQueueSemaphoreTake` (find these by their assert/log strings in v5.5.3 `esp_driver_usb_serial_jtag/src/usb_serial_jtag.c` and `esp_console/esp_console_repl_chip.c`). Expected (the spec's hypothesis): blocked waiting for TX ring-buffer space or a TX-done semaphore that only the USB-Serial-JTAG ISR (source 26, not modeled) releases. Record what you find either way; if it is something else, record it as the first Task D for Task 3.

- [ ] **Step 3: `PROVISIONED_FLAG_WRITER`, `VALIDATION`, `ROLE_TABLE`.** From M4's flag byte `0x3fca_92b2` (reader `0x4200_ee06`) find every store to it; follow back to `hal_identity`'s load function (references to `/littlefs/identity.json`, `identity.json size %ld out of range`, `%s: %u bytes, max %u`, `badge_id is missing or empty`). Record the size range, every `max %u` per field (the immediates passed alongside each field-name string), the required fields, numeric parsing, and the role table (the array the `unknown role` path searches). Check whether `badge_upload`/`credential` (NVS) is read anywhere on the path that sets the flag.

- [ ] **Step 4: `R_M5_3`.** On every function reachable from the flag writer, `prov apply`'s handler and `hal_identity`'s loader, check for calls into mbedTLS / ROM SHA / HMAC / `esp_hmac_*` / `esp_ds_*` / ECDSA / ed25519 (by their v5.5.3 assert/log strings and ROM addresses from `esp32c3.rom.ld`) and for reads of the base MAC (`esp_efuse_mac_get_default`, `esp_read_mac`) or eFuse blocks compared against identity fields. If any is found: **stop**, record it in the ledger, and report to the orchestrator (the human partner rules). Otherwise record the negative finding with the functions checked.

- [ ] **Step 5: `REPL_LINE_END`, `PUT_READ`, `APPLY_EFFECTS`.** Find the `put` command handler (references to `usage: put <path> <size>`, `short read: %ld bytes missing`) and record how it reads the payload. Find `prov`'s handler (`PROV OK id=%s`, `PROV FAIL display busy`, `tutorial=reset`) and record what `apply` does after validation (calls into the app manager, the LVGL lock behind "display busy", `esp_restart`). From v5.5.3 `esp_console/esp_console_repl_chip.c` and `console/linenoise/linenoise.c`, record how the REPL reads a line when the terminal never answers linenoise's status probe (dumb mode) and which byte ends it.

- [ ] **Step 6: Local fixtures.** Write one file per role in `ROLE_TABLE` to `local/m5-identities/<role>.json`, by hand from the traced schema (never from the dump), compact JSON on one line, no trailing newline. Template for `hacker` (adjust every value to the traced constraints: field set, maximum lengths, `version` value, numeric ranges, `badge_id` shape):

```json
{"version":1,"badge_id":"test-fake-badge-0001","attendee_id":1,"role":"hacker","account_email":"test.hacker@example.com","provisioned_unix":1767225600,"claim_id":"TEST01","display_name":"Test Hacker","net_email":"","net_phone":"","net_linkedin":"","net_discord":"","net_instagram":"","net_x":""}
```

  For role number `k` (1-based, in `ROLE_TABLE` order) use `badge_id` `test-fake-badge-000k` (zero-padded to keep the length fixed), `attendee_id` `k`, `role` the role string, `account_email` `test.<role with _ replaced by .>@example.com`, `display_name` `Test <Role in title case, _ as space>` (e.g. `Test Workshop Lead`), and the same `claim_id`/`provisioned_unix`/empty `net_*` fields.

- [ ] **Step 7: Cross-check (R-M5-1).** Compare the fixtures' key set and value types with FINDINGS.md's `/identity.json` structure (keys and types only; FINDINGS holds no values). Any key in FINDINGS missing from the trace, or vice versa, is a trace question to resolve in the firmware, not something to copy from FINDINGS. Record "cross-checked, consistent" or the resolved difference in the ledger (not in committed docs beyond "cross-checked against the structure of a registered badge's record").

- [ ] **Step 8: Docs and commit.**
  - Create `docs/milestone-5-decisions.md` in the M4 format: title; intro naming the spec and plan and "where they disagree with this file or the code, this file and the code win"; a "Rulings" section (R-M5-1..5, copied from the spec, each with "if wrong"); the "Trace facts" section (Interfaces above, with addresses); an empty "Milestone 6 backlog" heading.
  - In `docs/milestone-4-decisions.md`, replace "(its format to be read from the firmware, never from the physical badge's dump)" with "(its format read from the firmware; see `milestone-5-decisions.md`, ruling R-M5-1)".
  - Notes: history entry "Milestone 5 Task 1: the provisioning path" summarizing the trace facts (no identity values; only firmware facts).

```bash
git add docs
git commit -m "docs: trace the badge's provisioning path in factory.bin (Milestone 5)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: USB-Serial-JTAG receive direction and interrupt source

**Files:**
- Modify: `emulator-core/src/peripherals/usb_serial_jtag.rs` (struct, `read_byte`, `write_byte`, module doc, tests)
- Modify: `emulator-core/src/mem/soc.rs` (`SRC_USB_SERIAL_JTAG` next to `SRC_RMT`/`SRC_I2C_EXT0`, ~L150)
- Modify: `emulator-core/src/mem/bus.rs` (`pending_sources` ~L529, `tick_peripherals` ~L555, two new methods, module doc tier 8)
- Modify: `emulator-core/src/boot.rs` (`step_with_interrupts` ~L262–285)
- Modify: `emulator-core/src/runtime.rs` (two methods after `console_output`)
- Modify: `docs/milestone-5-decisions.md`, `docs/firmware-emulator-notes.md`

**Interfaces:**
- Consumes: `crate::peripherals::systimer::TICKS_PER_STEP: u64`; `Systimer::{ticks_until_next_alarm(&self) -> Option<u64>, advance_by(&mut self, u64)}`; `crate::peripherals::set_byte`.
- Produces:
  - `crate::mem::soc::SRC_USB_SERIAL_JTAG: u32 = 26`
  - `crate::peripherals::usb_serial_jtag::{INT_ST_REG, INT_ENA_REG, SERIAL_OUT_RECV_PKT_INT, OUT_EP_MAX_PACKET, PACKET_TICKS, HOST_QUEUE_CAPACITY}` and `UsbSerialJtag::{host_send(&mut self, &[u8]) -> usize, host_pending(&self) -> usize, advance(&mut self, ticks: u64), ticks_until_next_packet(&self) -> Option<u64>, pending_sources(&self) -> u64}`
  - `FirmwareBus::{ticks_until_next_event(&self) -> u64, advance_idle(&mut self, ticks: u64)}`
  - `FirmwareRuntime::{serial_input(&mut self, bytes: &[u8]) -> usize, serial_pending(&self) -> usize}`

- [ ] **Step 1: Confirm the register facts.** Fetch v5.5.3 `components/soc/esp32c3/register/soc/usb_serial_jtag_reg.h`, `components/hal/esp32c3/include/hal/usb_serial_jtag_ll.h`, `components/soc/esp32c3/include/soc/interrupts.h`, `components/esp_driver_usb_serial_jtag/src/usb_serial_jtag.c`. Confirm: `INT_ST_REG` = +0x0C, `INT_ENA_REG` = +0x10, `INT_CLR_REG` = +0x14; `SERIAL_OUT_RECV_PKT` is bit 2 in RAW/ST/ENA/CLR; INT_RAW fields are R/WTC/SS (write 1 clears); `usb_serial_jtag_ll_read_rxfifo` loops on `ep1_conf.serial_out_ep_data_avail` reading `ep1.rdwr_byte`; `ETS_USB_SERIAL_JTAG_INTR_SOURCE` = 26; the driver ISR's TX path enables `SERIAL_IN_EMPTY` only while its ring buffer holds bytes and disables it when drained; the RX path clears `SERIAL_OUT_RECV_PKT` then reads up to 64 bytes. If any differs, use the header's value and note the difference in the ledger.

- [ ] **Step 2: Write the failing unit tests** (append to `usb_serial_jtag.rs`'s `mod tests`; the existing tests stay):

```rust
    fn tick(u: &mut UsbSerialJtag, ticks: u64) {
        u.advance(ticks);
    }

    #[test]
    fn host_bytes_arrive_as_a_64_byte_packet_on_the_next_tick() {
        let mut u = UsbSerialJtag::new();
        let input: Vec<u8> = (0..100u8).collect();
        assert_eq!(u.host_send(&input), 100);
        assert_eq!(read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL, 0, "not before a tick");
        tick(&mut u, 1);
        assert_ne!(read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL, 0);
        let mut got = Vec::new();
        while read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL != 0 {
            got.push(u.read_byte(EP1_REG));
        }
        assert_eq!(got, (0..64u8).collect::<Vec<_>>(), "one full-speed packet");
        assert_eq!(u.host_pending(), 36);
    }

    #[test]
    fn word_read_of_ep1_pops_exactly_one_byte() {
        let mut u = UsbSerialJtag::new();
        u.host_send(b"ab");
        tick(&mut u, 1);
        assert_eq!(read_word(&mut u, EP1_REG), u32::from(b'a'), "lane 0 is RDWR_BYTE; lanes 1..3 read 0");
        assert_eq!(read_word(&mut u, EP1_REG), u32::from(b'b'));
        assert_eq!(read_word(&mut u, EP1_REG), 0, "empty FIFO reads 0");
        assert_eq!(u.host_pending(), 0);
    }

    #[test]
    fn next_packet_waits_for_an_empty_fifo_and_one_packet_time() {
        let mut u = UsbSerialJtag::new();
        u.host_send(&[7u8; 130]);
        tick(&mut u, 1);
        for _ in 0..64 {
            u.read_byte(EP1_REG);
        }
        // The first packet loaded at the tick above; the next is due
        // PACKET_TICKS after it.
        tick(&mut u, PACKET_TICKS - 1);
        assert_eq!(read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL, 0, "packet time not elapsed");
        tick(&mut u, 1);
        assert_ne!(read_word(&mut u, EP1_CONF_REG) & SERIAL_OUT_EP_DATA_AVAIL, 0);
        // A full FIFO blocks the next packet however long the host waits.
        tick(&mut u, 10 * PACKET_TICKS);
        assert_eq!(u.host_pending(), 130 - 64, "FIFO (64) + queue (2) unread");
    }

    #[test]
    fn recv_pkt_raw_is_set_per_packet_and_cleared_by_a_byte_split_int_clr() {
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        assert_eq!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        u.host_send(b"x");
        tick(&mut u, 1);
        assert_ne!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        write_word(&mut u, &mut c, INT_CLR_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        // Writing 1 to INT_RAW (R/WTC) also clears; writing 0 does not.
        u.host_send(b"y");
        u.read_byte(EP1_REG);
        tick(&mut u, PACKET_TICKS);
        write_word(&mut u, &mut c, INT_RAW_REG, 0);
        assert_ne!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        write_word(&mut u, &mut c, INT_RAW_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(read_word(&mut u, INT_RAW_REG) & SERIAL_OUT_RECV_PKT_INT, 0);
        assert_eq!(read_word(&mut u, INT_CLR_REG), 0, "INT_CLR is write-only");
    }

    #[test]
    fn int_st_is_raw_masked_by_ena_and_drives_the_interrupt_source() {
        use crate::mem::soc::SRC_USB_SERIAL_JTAG;
        let (mut u, mut c) = (UsbSerialJtag::new(), Console::new());
        // Nothing enabled: forced SOF/IN_EMPTY raw bits do not reach INT_ST.
        assert_eq!(read_word(&mut u, INT_ST_REG), 0);
        assert_eq!(u.pending_sources(), 0);
        write_word(&mut u, &mut c, INT_ENA_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(read_word(&mut u, INT_ENA_REG), SERIAL_OUT_RECV_PKT_INT, "ENA reads back");
        assert_eq!(u.pending_sources(), 0, "enabled but not raised");
        u.host_send(b"z");
        tick(&mut u, 1);
        assert_eq!(read_word(&mut u, INT_ST_REG), SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(u.pending_sources(), 1u64 << SRC_USB_SERIAL_JTAG);
        write_word(&mut u, &mut c, INT_CLR_REG, SERIAL_OUT_RECV_PKT_INT);
        assert_eq!(u.pending_sources(), 0, "level drops when cleared");
        // The TX path: enabling SERIAL_IN_EMPTY asserts at once (TX is always empty).
        write_word(&mut u, &mut c, INT_ENA_REG, SERIAL_IN_EMPTY_INT_RAW);
        assert_eq!(u.pending_sources(), 1u64 << SRC_USB_SERIAL_JTAG);
        write_word(&mut u, &mut c, INT_ENA_REG, 0);
        assert_eq!(u.pending_sources(), 0);
    }

    #[test]
    fn ticks_until_next_packet_is_none_when_nothing_can_arrive() {
        let mut u = UsbSerialJtag::new();
        assert_eq!(u.ticks_until_next_packet(), None, "idle host");
        u.host_send(&[1u8; 65]);
        assert_eq!(u.ticks_until_next_packet(), Some(1));
        tick(&mut u, 1);
        assert_eq!(u.ticks_until_next_packet(), None, "FIFO full: waits on the firmware, not on time");
        for _ in 0..64 {
            u.read_byte(EP1_REG);
        }
        assert_eq!(u.ticks_until_next_packet(), Some(PACKET_TICKS), "a packet time after the last load");
    }

    #[test]
    fn host_queue_is_capped_and_refuses_the_excess() {
        let mut u = UsbSerialJtag::new();
        let big = vec![0u8; HOST_QUEUE_CAPACITY + 12_345];
        assert_eq!(u.host_send(&big), HOST_QUEUE_CAPACITY);
        assert_eq!(u.host_send(b"more"), 0);
        assert_eq!(u.host_pending(), HOST_QUEUE_CAPACITY);
        // Non-UTF-8 and NUL bytes are just bytes.
        let mut v = UsbSerialJtag::new();
        v.host_send(&[0x00, 0xff, 0xfe]);
        tick(&mut v, 1);
        assert_eq!([v.read_byte(EP1_REG), v.read_byte(EP1_REG), v.read_byte(EP1_REG)], [0x00, 0xff, 0xfe]);
    }
```

  Also update the existing `ep1_conf_always_reports_tx_free_and_no_rx` name and doc to `ep1_conf_reports_tx_free_and_no_rx_while_the_host_is_idle` (its assertions still hold with an idle host).

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p emulator-core --lib peripherals::usb_serial_jtag`
Expected: compile errors (`host_send`, `advance`, `INT_ST_REG`, `SRC_USB_SERIAL_JTAG`, ... not found).

- [ ] **Step 4: Implement.** In `soc.rs` after `SRC_I2C_EXT0`:

```rust
pub const SRC_USB_SERIAL_JTAG: u32 = 26; // ETS_USB_SERIAL_JTAG_INTR_SOURCE (soc/esp32c3/include/soc/interrupts.h)
```

  In `usb_serial_jtag.rs` (keep existing constants; replace the struct and the two methods; extend the module doc with an "RX" section citing the Step 1 sources, the pacing derivation below, and that `INT_RAW`/`INT_CLR`/`INT_ST`/`INT_ENA` are now real):

```rust
use std::collections::{HashMap, VecDeque};

use crate::mem::soc::SRC_USB_SERIAL_JTAG;

/// `USB_SERIAL_JTAG_INT_ST_REG` (RO): `INT_RAW & INT_ENA`.
pub const INT_ST_REG: u32 = 0x0C;
/// `USB_SERIAL_JTAG_INT_ENA_REG` (R/W).
pub const INT_ENA_REG: u32 = 0x10;
/// `USB_SERIAL_JTAG_SERIAL_OUT_RECV_PKT_INT_*`, bit 2 of RAW/ST/ENA/CLR:
/// "a packet was received by the OUT endpoint".
pub const SERIAL_OUT_RECV_PKT_INT: u32 = 1 << 2;
/// Full-speed bulk max packet size of the CDC OUT endpoint (the FIFO depth).
pub const OUT_EP_MAX_PACKET: usize = 64;
/// SYSTIMER ticks (16 MHz) one OUT packet occupies on a full-speed bus: a
/// 64-byte DATA packet plus its OUT token and ACK handshake is ~616 bits,
/// ~51.3 µs at 12 Mbit/s, x 16 ticks/µs = 821. A host cannot send the next
/// packet sooner; see `docs/milestone-5-decisions.md`.
pub const PACKET_TICKS: u64 = 821;
/// Bytes the emulated USB host will buffer before refusing more (a browser
/// must not grow emulator memory without bound).
pub const HOST_QUEUE_CAPACITY: usize = 1 << 20;

/// Raw bits this model holds set on every read (see the module doc).
const FORCED_RAW: u32 = SERIAL_IN_EMPTY_INT_RAW | SOF_INT_RAW;

#[derive(Default)]
pub struct UsbSerialJtag {
    /// Plain word storage for offsets without behavior of their own.
    regs: HashMap<u32, u32>,
    /// Latched (non-forced) `INT_RAW` bits.
    int_raw: u32,
    /// Bytes the host has buffered and not yet sent as a packet.
    host_queue: VecDeque<u8>,
    /// The OUT endpoint FIFO: the current packet, popped by `EP1_REG` reads.
    out_fifo: VecDeque<u8>,
    /// Ticks left before the host may send another packet.
    ticks_to_next_packet: u64,
}

impl UsbSerialJtag {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues bytes the USB host sends; returns how many were accepted
    /// (fewer than `bytes.len()` once [`HOST_QUEUE_CAPACITY`] is reached).
    pub fn host_send(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(HOST_QUEUE_CAPACITY - self.host_queue.len());
        self.host_queue.extend(&bytes[..n]);
        n
    }

    /// Bytes the firmware has not read yet (host queue plus FIFO).
    pub fn host_pending(&self) -> usize {
        self.host_queue.len() + self.out_fifo.len()
    }

    /// Advances bus time by `ticks` SYSTIMER ticks: the host sends its next
    /// packet once the FIFO is empty and a packet time has passed.
    pub fn advance(&mut self, ticks: u64) {
        self.ticks_to_next_packet = self.ticks_to_next_packet.saturating_sub(ticks);
        if self.ticks_to_next_packet == 0 && self.out_fifo.is_empty() && !self.host_queue.is_empty() {
            let n = self.host_queue.len().min(OUT_EP_MAX_PACKET);
            self.out_fifo.extend(self.host_queue.drain(..n));
            self.int_raw |= SERIAL_OUT_RECV_PKT_INT;
            self.ticks_to_next_packet = PACKET_TICKS;
        }
    }

    /// Ticks until [`UsbSerialJtag::advance`] would deliver a packet, or
    /// `None` if none can arrive without the firmware reading first (idle
    /// host, or a non-empty FIFO). Bounds the WFI fast-forward.
    pub fn ticks_until_next_packet(&self) -> Option<u64> {
        if self.host_queue.is_empty() || !self.out_fifo.is_empty() {
            None
        } else {
            Some(self.ticks_to_next_packet.max(1))
        }
    }

    fn int_ena(&self) -> u32 {
        self.regs.get(&INT_ENA_REG).copied().unwrap_or(0)
    }

    fn int_st(&self) -> u32 {
        (self.int_raw | FORCED_RAW) & self.int_ena()
    }

    /// `ETS_USB_SERIAL_JTAG_INTR_SOURCE` as a level while `INT_ST != 0`.
    pub fn pending_sources(&self) -> u64 {
        if self.int_st() != 0 {
            1u64 << SRC_USB_SERIAL_JTAG
        } else {
            0
        }
    }

    pub fn read_byte(&mut self, offset: u32) -> u8 {
        let word_offset = offset & !0b11;
        let idx = (offset & 0b11) as usize;
        let word = match word_offset {
            // Only lane 0 is RDWR_BYTE; reading it pops one byte.
            EP1_REG if idx == 0 => return self.out_fifo.pop_front().unwrap_or(0),
            EP1_REG => 0,
            EP1_CONF_REG => {
                SERIAL_IN_EP_DATA_FREE
                    | if self.out_fifo.is_empty() { 0 } else { SERIAL_OUT_EP_DATA_AVAIL }
            }
            INT_RAW_REG => self.int_raw | FORCED_RAW,
            INT_ST_REG => self.int_st(),
            INT_CLR_REG => 0,
            _ => self.regs.get(&word_offset).copied().unwrap_or(0),
        };
        word.to_le_bytes()[idx]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8, console: &mut Console) {
        let word_offset = offset & !0b11;
        let idx = offset & 0b11;
        match word_offset {
            EP1_REG => {
                if idx == 0 {
                    console.push(val);
                }
            }
            EP1_CONF_REG | INT_ST_REG => {}
            // R/WTC (INT_RAW) and WT (INT_CLR): a 1 clears the raw bit. The
            // forced bits re-assert on the next read.
            INT_RAW_REG | INT_CLR_REG => self.int_raw &= !(u32::from(val) << (8 * idx)),
            _ => {
                let mut word = self.regs.get(&word_offset).copied().unwrap_or(0);
                set_byte(&mut word, idx, val);
                self.regs.insert(word_offset, word);
            }
        }
    }
}
```

  In `bus.rs`: `pending_sources` gains `p |= self.usb_serial_jtag.pending_sources();`; `tick_peripherals` gains `self.usb_serial_jtag.advance(TICKS_PER_STEP);` (import it from `crate::peripherals::systimer`); add:

```rust
    /// How far an idle (WFI) core may fast-forward: to the next SYSTIMER
    /// alarm or the next USB host packet, whichever is first; at least one
    /// step's worth of ticks.
    pub fn ticks_until_next_event(&self) -> u64 {
        [self.systimer.ticks_until_next_alarm(), self.usb_serial_jtag.ticks_until_next_packet()]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(0)
            .max(TICKS_PER_STEP)
    }

    /// Advances every timed peripheral by `ticks` during an idle fast-forward.
    pub fn advance_idle(&mut self, ticks: u64) {
        self.systimer.advance_by(ticks);
        self.usb_serial_jtag.advance(ticks);
    }
```

  In `boot.rs`'s `step_with_interrupts`, replace the fast-forward body with:

```rust
    if fast_forward {
        let ticks = bus.ticks_until_next_event();
        bus.advance_idle(ticks);
        lines = bus.asserted_lines();
    }
```

  and update its doc comment (it now stops at USB host packets too). In `runtime.rs`, after `console_output`:

```rust
    /// Bytes a USB host sends to the badge's console (USB-Serial-JTAG OUT).
    /// Returns how many were accepted; see
    /// `crate::peripherals::usb_serial_jtag::HOST_QUEUE_CAPACITY`.
    pub fn serial_input(&mut self, bytes: &[u8]) -> usize {
        self.bus.usb_serial_jtag.host_send(bytes)
    }

    /// Serial input the firmware has not read yet.
    pub fn serial_pending(&self) -> usize {
        self.bus.usb_serial_jtag.host_pending()
    }
```

- [ ] **Step 5: Bus test for the fast-forward bound** (append to `bus.rs`'s `mod tests`):

```rust
    #[test]
    fn idle_fast_forward_stops_at_the_next_host_packet() {
        use crate::peripherals::usb_serial_jtag::PACKET_TICKS;
        let mut bus = bus_with(vec![]);
        // No alarm armed and no host input: one step's worth.
        assert_eq!(bus.ticks_until_next_event(), TICKS_PER_STEP);
        bus.usb_serial_jtag.host_send(&[b'a'; 65]);
        assert_eq!(bus.ticks_until_next_event(), 1, "first packet is due now");
        bus.advance_idle(1);
        // Firmware drains the packet; the next one is a packet time away.
        for _ in 0..64 {
            bus.read8(USB_SERIAL_JTAG_RANGE.start);
        }
        assert_eq!(bus.ticks_until_next_event(), PACKET_TICKS, "no SYSTIMER alarm is armed on a fresh bus");
        bus.advance_idle(PACKET_TICKS);
        assert_eq!(bus.read8(USB_SERIAL_JTAG_RANGE.start + 4) & 0b100, 0b100, "DATA_AVAIL");
    }
```

  (Import `TICKS_PER_STEP` and `USB_SERIAL_JTAG_RANGE` in the test module if not already in scope.)

- [ ] **Step 6: Run unit tests**

Run: `cargo test -p emulator-core --lib`
Expected: PASS, including the 7 new `usb_serial_jtag` tests, the bus test and every existing test.

- [ ] **Step 7: Run the boot ladder and look at the console**

Run: `cargo test -p emulator-core --release --test boot_progress` then `cargo run -p emulator-core --release --example boot-probe -- --steps 20000000 --window 2000000`
Expected: all 35 rungs pass with unchanged hashes (`SPLASH_HASH`, `FIRST_RUN_HASH`, `SELF_TEST_BUTTONS_HASH`). **If any hash or rung changes, stop and report**: the interrupt source changed pre-input behavior. In the probe output, look for `hal_console: console started` and `badge> `. Record in the ledger whether they appear and at which step (expected per the spec's hypothesis; if not, Task 3 Step 1 handles it as a Task D with Task 1's `CONSOLE_START` finding). If the hot-PC list is dominated by the USB-Serial-JTAG ISR with no console progress (an interrupt storm), **stop and report** (spec Part 2).

- [ ] **Step 8: Docs, full suite, commit.**
  - Decisions doc, section "USB-Serial-JTAG receive (Task 2)": pacing at full-speed line rate (if wrong: input arrives faster or slower than a real host; a faster model would overflow the driver's ring buffer); host queue cap 1 MiB (if wrong: a client sending more is refused, a real host would block); `INT_RAW` WTC semantics and forced bits unchanged; the interrupt source now exists (resolves M4's "No USB-Serial-JTAG interrupt source yet"); no USB bus reset, no JTAG, no EP2.
  - Notes: history entry "Milestone 5 Task 2"; Step 7's console findings; open limitation about console input removed from the M4 backlog list.

Run: the Global Constraints full suite.

```bash
git add -A emulator-core docs
git commit -m "feat(emulator-core): USB-Serial-JTAG receive direction and interrupt source

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Console rungs without an identity

**Files:**
- Create: `emulator-core/tests/common/mod.rs`, `emulator-core/tests/common/provision.rs`
- Modify: `emulator-core/tests/boot_progress.rs` (add `mod common;` after the `use` lines; three rungs after `first_run_screen_responds_to_start`)
- Modify: `docs/firmware-emulator-notes.md`, `docs/milestone-5-decisions.md`

**Interfaces:**
- Consumes: Task 2's `FirmwareRuntime::{serial_input, serial_pending}`; Task 1's `REPL_LINE_END` and `PUT_READ`; `boot_progress.rs`'s `factory()`, `run_until_stable_frame`, `FIRST_RUN_HASH`, `FIRST_RUN_MAX_STEPS`, `BOOT_STABLE_FOR`, `SPLASH_HASH`, `assert_no_panic_text`.
- Produces (used by Tasks 4, 6, 9): module `common::provision` with
  - `pub const PROMPT: &str = "badge> ";`
  - `pub const IDENTITY_PATH: &str = "/littlefs/identity.json";`
  - `pub enum Outcome { Done(String), Timeout(String) }` (`Done` carries the console text after the marker)
  - `pub fn wait_for_any(rt: &mut FirmwareRuntime, marker: usize, needles: &[&str], deadline: u64) -> Option<(usize, String)>`
  - `pub fn type_line(rt: &mut FirmwareRuntime, line: &str, deadline: u64) -> Outcome` (waits for the next prompt)
  - `pub fn provision(rt: &mut FirmwareRuntime, identity_json: &[u8], deadline: u64) -> Result<String, String>` (`Ok` = text containing `PROV OK`, `Err` = `PROV FAIL ...` text or a timeout description)

- [ ] **Step 1: Console-start rung.** Add to `boot_progress.rs`:

```rust
/// Milestone 5 Task 3: `app_main` reaches `hal_console_start()` on blank
/// flash, as on the physical badge (the line follows `app_reg: launched My
/// Badge`); the console runs the interrupt-driven USB-Serial-JTAG driver,
/// so this needs Task 2's interrupt source.
#[test]
fn boot_starts_the_console() {
    assert_reaches("hal_console: console started", 20_000_000);
}
```

Run: `cargo test -p emulator-core --release --test boot_progress boot_starts_the_console`
Expected: PASS if Task 2 unblocked it. If it fails, this is the first Task D (template below) seeded with Task 1's `CONSOLE_START` finding; do not proceed to Step 2 until it passes. Tighten the budget to the measured first step + ~25%, rounded to 1,000,000, and note the measurement in the doc comment.

- [ ] **Step 2: The helper module.** Create `emulator-core/tests/common/mod.rs` containing `pub mod provision;` and `emulator-core/tests/common/provision.rs` (set `LINE_END` from Task 1's `REPL_LINE_END`; the default below is `"\n"`; if Task 1's `PUT_READ` says payload bytes sent before the command starts reading are lost, keep the `PUT_SETTLE_STEPS` wait, otherwise set it to 0 and say so in its doc):

```rust
//! Driving the real firmware's console REPL from tests, as a USB host
//! would. The REPL, its `put` and `prov` commands and their output are the
//! firmware's (`docs/milestone-5-decisions.md`, "Trace facts"); this module
//! only types and waits. The console buffer is a 256 KiB ring, far above
//! what boot and provisioning print, so byte offsets into it are stable.
#![allow(dead_code)] // each test binary uses a subset

use emulator_core::runtime::FirmwareRuntime;

pub const PROMPT: &str = "badge> ";
pub const IDENTITY_PATH: &str = "/littlefs/identity.json";
/// What ends a REPL line (Task 1, `REPL_LINE_END`).
pub const LINE_END: &str = "\n";
/// Steps between the `put` command line and its payload (Task 1, `PUT_READ`).
pub const PUT_SETTLE_STEPS: u32 = 200_000;
const CHUNK: u32 = 50_000;

pub enum Outcome {
    Done(String),
    Timeout(String),
}

fn console_since(rt: &FirmwareRuntime, marker: usize) -> String {
    let text = rt.console_output();
    text.get(marker..).unwrap_or(&text).to_string()
}

/// Runs until the console text after byte `marker` contains one of
/// `needles` (returns its index and that text), or `deadline` total steps.
pub fn wait_for_any(
    rt: &mut FirmwareRuntime,
    marker: usize,
    needles: &[&str],
    deadline: u64,
) -> Option<(usize, String)> {
    loop {
        let since = console_since(rt, marker);
        if let Some(i) = needles.iter().position(|n| since.contains(n)) {
            return Some((i, since));
        }
        if rt.total_steps() >= deadline {
            return None;
        }
        let s = rt.run(CHUNK);
        assert_eq!(s.last_instruction_fault, None, "{s:?}");
    }
}

/// Types `line` and waits for the REPL's next prompt.
pub fn type_line(rt: &mut FirmwareRuntime, line: &str, deadline: u64) -> Outcome {
    let marker = rt.console_output().len();
    let input = format!("{line}{LINE_END}");
    assert_eq!(rt.serial_input(input.as_bytes()), input.len());
    match wait_for_any(rt, marker, &[PROMPT], deadline) {
        Some((_, text)) => Outcome::Done(text),
        None => Outcome::Timeout(console_since(rt, marker)),
    }
}

/// The registration-desk flow: `put <IDENTITY_PATH> <len>`, the bytes,
/// then `prov apply`. Waits for the first prompt before typing.
pub fn provision(rt: &mut FirmwareRuntime, identity_json: &[u8], deadline: u64) -> Result<String, String> {
    if wait_for_any(rt, 0, &[PROMPT], deadline).is_none() {
        return Err(format!("no `{PROMPT}` prompt by step {deadline}"));
    }
    let marker = rt.console_output().len();
    let header = format!("put {IDENTITY_PATH} {}{LINE_END}", identity_json.len());
    rt.serial_input(header.as_bytes());
    while rt.serial_pending() > 0 && rt.total_steps() < deadline {
        rt.run(CHUNK);
    }
    rt.run(PUT_SETTLE_STEPS);
    assert_eq!(rt.serial_input(identity_json), identity_json.len());
    let ok = format!("OK {}", identity_json.len());
    match wait_for_any(rt, marker, &[&ok, "bad size", "write error", "short read"], deadline) {
        Some((0, _)) => {}
        Some((_, text)) => return Err(format!("put failed:\n{text}")),
        None => return Err(format!("put timed out:\n{}", console_since(rt, marker))),
    }
    let marker = rt.console_output().len();
    rt.serial_input(format!("prov apply{LINE_END}").as_bytes());
    match wait_for_any(rt, marker, &["PROV OK", "PROV FAIL"], deadline) {
        Some((0, text)) => Ok(text),
        Some((_, text)) => Err(text),
        None => Err(format!("prov apply timed out:\n{}", console_since(rt, marker))),
    }
}
```
- [ ] **Step 3: Write the two console rungs** (in `boot_progress.rs`, after `boot_starts_the_console`):

```rust
use common::provision::{self, Outcome};

/// [`console_answers_prov_show_on_the_first_run_screen`] and later
/// provisioning rungs: total-step deadline from a stable first-run screen.
const CONSOLE_DEADLINE: u64 = FIRST_RUN_MAX_STEPS + 20_000_000;

/// Boots to the stable first-run screen (blank flash).
fn first_run_screen() -> FirmwareRuntime {
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let (hash, _) = run_until_stable_frame(&mut rt, FIRST_RUN_MAX_STEPS, BOOT_STABLE_FOR, &[SPLASH_HASH]);
    assert_eq!(hash, FIRST_RUN_HASH);
    rt
}

/// The console takes input on the first-run screen: `prov show` reports an
/// unprovisioned badge, as the registration desk would see it.
#[test]
fn console_answers_prov_show_on_the_first_run_screen() {
    let mut rt = first_run_screen();
    assert!(provision::wait_for_any(&mut rt, 0, &[provision::PROMPT], CONSOLE_DEADLINE).is_some(), "no prompt");
    match provision::type_line(&mut rt, "prov show", CONSOLE_DEADLINE) {
        Outcome::Done(text) => assert!(text.contains("provisioned=0"), "{text}"),
        Outcome::Timeout(text) => panic!("no prompt after `prov show`:\n{text}"),
    }
    assert_no_panic_text(&rt);
}

/// The whole `put` / `prov apply` path, with a record the firmware must
/// reject: an empty JSON object. Nothing identity-shaped is involved.
#[test]
fn console_rejects_an_empty_identity() {
    let mut rt = first_run_screen();
    let err = provision::provision(&mut rt, b"{}", CONSOLE_DEADLINE).expect_err("`{}` must not provision");
    assert!(err.contains("PROV FAIL invalid or missing"), "{err}");
    let (hash, _) = run_until_stable_frame(&mut rt, CONSOLE_DEADLINE, BOOT_STABLE_FOR, &[]);
    assert_eq!(hash, FIRST_RUN_HASH, "still unregistered");
    assert_no_panic_text(&rt);
}
```

  (Put the `use common::provision::{self, Outcome};` line with the other `use` lines at the top, and `mod common;` right after them.)

- [ ] **Step 4: Run**

Run: `cargo test -p emulator-core --release --test boot_progress console_`
Expected: both PASS. A failure is a Task D (template below): diagnose with the console text the assertion prints and boot-probe; do not loosen the assertion. If `PROV FAIL` names a different message than `invalid or missing` (Task 1's trace says which check fires first on `{}`), assert that message instead and say why in the doc comment. Then set `CONSOLE_DEADLINE` to the measured need + ~25%, rounded to 1,000,000.

- [ ] **Step 5: Suite time.** `time cargo test -p emulator-core --release --test boot_progress`. If above ~10 s, do Task 8 Step 1 (checkpoint fixture) now, as its own commit, before continuing.

- [ ] **Step 6: Docs, full suite, commit.** Notes: "Current state" bullet (console starts; REPL answers); history entry "Milestone 5 Task 3" with measured steps. Decisions: the `PUT_SETTLE_STEPS` choice and why.

```bash
git add -A emulator-core docs
git commit -m "test(emulator-core): the console REPL answers on the first-run screen

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task D (template, repeat per stall)

Every stall from Task 2 on gets a numbered task created from this template by the orchestrator (`Task D-M5-<n>`).

**Files:** the module that owns the registers (new `emulator-core/src/peripherals/<name>.rs` + `peripherals/mod.rs` + `mem/soc.rs` + `mem/bus.rs` routing if new), or `src/rom.rs` for a ROM call; `tests/boot_progress.rs` (or the held-rung file, below); `docs/firmware-emulator-notes.md`; `docs/milestone-5-decisions.md` if a decision was taken.

- [ ] Step 1: Reproduce with `boot-probe` (`--steps`, `--window`) or the Task 4 scratch example; record the last console line, hot PCs, unmapped tail, fault, frame in the brief and ledger. Briefs never contain identity values; refer to fixtures by path.
- [ ] Step 2: Identify the registers / ROM call and the ESP-IDF v5.5.3 source (header + HAL/LL) that reads them. Disassemble around the hot PCs (Task 1 Step 1's `local/m5-trace/irom.S`). ROM addresses from `rom*.ld` and `local/rom-elfs/`. Do not guess.
- [ ] Step 3: Failing unit test with literal register writes/reads mirroring the cited LL/HAL function (byte-split word writes); run → FAIL.
- [ ] Step 4: Register-faithful minimal implementation; module doc cites sources; run → PASS.
- [ ] Step 5: Ratchet rung. Pre-gate, a stall reached without an identity gets a rung in `boot_progress.rs`; a stall reached only after provisioning gets its rung written into `local/m5-held-rungs.rs` (gitignored), to land in Task 6. No exact step counts beyond a budget.
- [ ] Step 6: Notes history entry, decisions entry if a choice was made, Global Constraints full suite, commit `feat(emulator-core): model <thing> (<what it unblocks>)`.

Rules: the smallest *correct* model, never a value chosen to make the firmware proceed. ROM stubs only when observed, with header-documented semantics. R-M5-3 applies. If a fix would override a recorded decision or the spec, stop and ask the human partner through the orchestrator.

---

### Task 4: Local exploration to the launcher (identities stay local)

**Files:**
- Local only: `emulator-core/examples/local_m5_explore.rs` (excluded via `.git/info/exclude`, Task 1 Step 1), `local/m5-frames/*.png`, `local/m5-held-rungs.rs`
- Committed only through Task D instances and the conditional tasks C1/C2 below

**Interfaces:**
- Consumes: Task 3's helper logic (copy `provision.rs`'s functions into the example; examples cannot import `tests/common`), `local/m5-identities/<role>.json`, `FirmwareRuntime::{set_raw_button, framebuffer}`, boot-probe's PNG writer (the `png` dev-dependency; copy its `dump_frame` function into the example).
- Produces (ledger + `local/m5-held-rungs.rs`, consumed by Tasks 5 and 6): per role, the measured `PROV OK` step, the registered-screen hash and its first stable step; the input sequence from `PROV OK` to the launcher (with step windows); the launcher hash; the navigation button (4 = DOWN or 6 = RIGHT) and its hash; the stability windows used.

- [ ] **Step 1: Provision one role.** The example boots to the first-run screen, provisions `local/m5-identities/hacker.json`, prints the console text after the marker (it contains the fake identity only), then runs `run_until_stable_frame`-equivalent sampling (250,000-step chunks; try a 1,000,000 window, widen if a busy phase like M4's ~6M appears) and dumps every new stable frame to `local/m5-frames/hacker-<n>.png`.
  Run: `cargo run -p emulator-core --release --example local_m5_explore -- hacker`
  Expected: `PROV OK` and a registered My Badge screen (or onboarding). A stall → Task D. A restart (boot banner printed again, or the PC back at the entry point) → conditional task C1.

- [ ] **Step 2: Walk to the launcher.** From the registered screen, follow `APPLY_EFFECTS` and what the screen says: any onboarding the firmware shows (the strings say it asks for every input, including flipping the Aux1 slide, slot 8), then HOME (slot 3). Hold each button 100,000 steps, release, wait for the next stable frame, dump it. Record the sequence. Then press DOWN (slot 4) and, if the frame does not change, RIGHT (slot 6); record which moves the selection.

- [ ] **Step 3: Every role.** Repeat Steps 1–2 for each role in `ROLE_TABLE` (parameterized example). Record each role's registered-screen hash and launcher hash; note which roles share a launcher hash (a role-independent launcher needs one rung; differing ones need one each).

- [ ] **Step 4: Held rungs.** Write the Task 6 rungs (exact code as in Task 6 Step 3, with the measured hashes and windows filled in) into `local/m5-held-rungs.rs`, and run them by temporarily appending them to `boot_progress.rs` with the fixture directory pointed at `local/m5-identities` (`BADGE_TEST_IDENTITIES=local/m5-identities`). Expected: all pass. **Revert the temporary append before any commit** (`git diff --stat` must not show `boot_progress.rs` changes from this step).

- [ ] **Step 5: Ledger.** Record all of Produces. No commit unless a Task D/C1/C2 instance made one.

**Conditional task C1: reset that keeps flash** (trigger: Step 1 or 2 shows the chip restarting).
- [ ] Find the register or ROM call that restarts (v5.5.3 `esp_system/port/soc/esp32c3/system_internal.c` `esp_restart_noos`: `RTC_CNTL_OPTIONS0_REG`'s `SW_SYS_RST` or the ROM `software_reset_cpu`; Task 1's `APPLY_EFFECTS` address says which).
- [ ] Failing unit test: a write of the reset bit (byte-split) or the ROM call sets a "reset requested" flag; `FirmwareRuntime` then reboots through `boot_from_factory_image_with_rom_stubs` with the **current** `bus.flash_chip` (add a `FirmwareRuntime` test: program a byte in `storage`, trigger the reset, read it back through the new bus's flash).
- [ ] Implement with the smallest change to `runtime.rs`/`boot.rs` that carries `flash_chip` across (e.g. a `boot_..._with_flash(image, flash: EmulatedFlash)` variant). Blank-flash rungs unchanged. Decisions entry (what survives: flash only; RAM, peripherals, RTC memory reset; if wrong, firmware relying on RTC memory across reset sees zeros). Commit `feat(emulator-core): chip reset keeps the flash contents`.

**Conditional task C2: ST7789 `MADCTL`** (trigger: Task 5 finds a frame rotated or mirrored): M4 plan Task 6 (`docs/superpowers/plans/2026-10-06-milestone4-app-launcher.md`, "Task 6: ST7789 MADCTL (conditional)"), as written there.

---

### Task 5: Human frame gate

**Files:** `docs/firmware-emulator-notes.md` (Task 6 records the outcome; nothing committed here).

- [ ] **Step 1:** The orchestrator asks the human partner to compare `local/m5-frames/` (the registered screen, any onboarding screens, the launcher, the navigation frame) for the role on their physical badge with the badge itself: content, layout and orientation. Their own name and details will differ from "Test …"; layout and everything not identity-derived should match.
- [ ] **Step 2:** Outcome to the ledger with the date. Mismatch on orientation → C2; on content → Task D; then repeat this gate for the changed frames.

---

### Task 6: Permission gate — fixtures, identity rungs, WASM twin

Start only when the human partner has relayed the organizers' permission to commit the fake identities (R-M5-2). Record the date and wording (no personal data) in the ledger.

**Files:**
- Create: `frontend/public/firmware/test-identities/<role>.json` (copied from `local/m5-identities/`), `frontend/src/runtime/test-identities.ts`, `frontend/src/runtime/provisioner.ts`, `frontend/test/test-identities.test.ts`, `frontend/test/provisioner.test.ts`
- Modify: `emulator-core/tests/common/provision.rs` (fixture loader), `emulator-core/tests/boot_progress.rs` (rungs), `emulator-wasm/src/lib.rs`, `frontend/src/cpu/bridge.ts`, `frontend/test/cpu-wasm.test.ts`, `frontend/test/firmware-runtime.test.ts` (fake handle gains the new methods), docs

**Interfaces:**
- Consumes: Task 4's measured hashes, input sequence and windows (`local/m5-held-rungs.rs`); Task 3's helper.
- Produces:
  - Rust: `common::provision::identity_fixture(role: &str) -> Vec<u8>`.
  - WASM: `FirmwareEmulator::{serial_input(&mut self, bytes: &[u8]) -> usize (js_name serialInput), serial_pending(&self) -> usize (js_name serialPending)}`.
  - TS: `FirmwareEmulatorHandle.{serialInput(bytes: Uint8Array): number; serialPending(): number; consoleOutput(): string}`; `TEST_IDENTITY_ROLES: readonly string[]`, `testIdentityUrl(role: string): string`; `createProvisioner(handle: FirmwareEmulatorHandle, identityJson: string): Provisioner` with `Provisioner.poll(): ProvisionState`, `type ProvisionState = { kind: "waiting" | "typing" } | { kind: "ok"; line: string } | { kind: "failed"; reason: string }`.

- [ ] **Step 1: Commit the fixtures and their guard test.** Copy the files; write `frontend/src/runtime/test-identities.ts`:

```ts
/**
 * Obviously fake badge identities, one per role in the firmware's role
 * table (docs/milestone-5-decisions.md, "Trace facts"; ruling R-M5-4).
 * Committed with the Hack the North organizers' permission (R-M5-2). They
 * are typed into the emulated badge's own console to provision it.
 */
export const TEST_IDENTITY_ROLES = [
  "hacker", "organizer", "sponsor", "judge", "mentor", "volunteer",
  "media", "staff", "general", "workshop_lead", "visitor",
] as const; // replace with Task 1's ROLE_TABLE, in its order

export function testIdentityUrl(role: string): string {
  return `/firmware/test-identities/${role}.json`;
}
```

  and `frontend/test/test-identities.test.ts` (fill `MAX_LEN` from Task 1's `VALIDATION` table; the guard keeps every fixture obviously fake):

```ts
import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { TEST_IDENTITY_ROLES } from "../src/runtime/test-identities";

const DIR = new URL("../public/firmware/test-identities/", import.meta.url);
/** Per-field maximum bytes, from docs/milestone-5-decisions.md "Trace facts". */
const MAX_LEN: Record<string, number> = {
  badge_id: 0, display_name: 0, account_email: 0, claim_id: 0, role: 0,
  net_email: 0, net_phone: 0, net_linkedin: 0, net_discord: 0, net_instagram: 0, net_x: 0,
}; // every 0 replaced by the traced maximum before commit

describe("committed test identities", () => {
  test("one file per role, and nothing else", () => {
    const files = readdirSync(DIR).filter((f) => f.endsWith(".json")).sort();
    expect(files).toEqual([...TEST_IDENTITY_ROLES].map((r) => `${r}.json`).sort());
  });

  for (const role of TEST_IDENTITY_ROLES) {
    test(`${role} is obviously fake and within the firmware's limits`, () => {
      const id = JSON.parse(readFileSync(new URL(`${role}.json`, DIR), "utf8"));
      expect(id.role).toBe(role);
      expect(id.display_name.startsWith("Test ")).toBe(true);
      expect(id.badge_id.startsWith("test-")).toBe(true);
      expect(id.account_email.endsWith("@example.com")).toBe(true);
      for (const k of ["net_email", "net_phone", "net_linkedin", "net_discord", "net_instagram", "net_x"]) {
        expect(id[k]).toBe("");
      }
      for (const [k, max] of Object.entries(MAX_LEN)) {
        expect(max).toBeGreaterThan(0);
        expect(new TextEncoder().encode(id[k]).length).toBeLessThanOrEqual(max);
      }
    });
  }
});
```

  Before committing, replace the role list with `ROLE_TABLE` and every `0` in `MAX_LEN` with the traced maximum; the `toBeGreaterThan(0)` assertion fails if one is left. Run `bun test test/test-identities.test.ts` → PASS.

- [ ] **Step 2: Fixture loader** (append to `emulator-core/tests/common/provision.rs`):

```rust
/// A committed test identity (`frontend/public/firmware/test-identities/`),
/// or from `$BADGE_TEST_IDENTITIES` when set (Task 4's local runs).
pub fn identity_fixture(role: &str) -> Vec<u8> {
    let dir = std::env::var_os("BADGE_TEST_IDENTITIES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../frontend/public/firmware/test-identities")
        });
    std::fs::read(dir.join(format!("{role}.json"))).unwrap_or_else(|e| panic!("fixture {role}: {e}"))
}
```

- [ ] **Step 3: The identity rungs** (in `boot_progress.rs`). Every constant marked "measured" is written with Task 4's value from `local/m5-held-rungs.rs` (which Task 4 Step 4 already ran green); **no constant may be committed as `0`**, and the reviewer checks this. Doc comments say which frames were human-compared on which date and which were not.

```rust
/// Total-step deadline for provisioning and settling from the first-run
/// screen: Task 4's measured need + ~25%, rounded to 1,000,000. (measured)
const PROVISION_DEADLINE: u64 = 0;
/// How long the registered screen must hold (Task 4's window; M4 needed
/// 8,000,000 after START, so a busy phase after `PROV OK` is plausible). (measured)
const REGISTERED_STABLE_FOR: u64 = 0;
/// The role on the human partner's physical badge (Task 5). (measured)
const OWN_ROLE: &str = "hacker";
/// Its registered My Badge screen. (measured)
const OWN_ROLE_REGISTERED_HASH: u64 = 0;
/// Raw slots pressed, in order, from the registered screen to the
/// launcher (Task 4 Step 2; e.g. `&[3]` for HOME alone). (measured)
const TO_LAUNCHER_SLOTS: &[usize] = &[];
/// Stable frames between those presses, one per press except the last
/// (empty if HOME goes straight to the launcher). (measured)
const TO_LAUNCHER_INTERMEDIATE_HASHES: &[u64] = &[];
/// The launcher (Task 5-compared). (measured)
const LAUNCHER_HASH: u64 = 0;
/// Total-step cap for reaching the launcher. (measured)
const LAUNCHER_MAX_STEPS: u64 = 0;
/// 4 (DOWN) or 6 (RIGHT): the one that moves the launcher's selection. (measured)
const NAV_SLOT: usize = 0;
/// The launcher after one `NAV_SLOT` press. (measured)
const LAUNCHER_AFTER_NAV_HASH: u64 = 0;

/// Provisions `role` from the stable first-run screen and returns the
/// runtime at the registered screen, after asserting its hash.
fn provisioned(role: &str, registered_hash: u64) -> FirmwareRuntime {
    let mut rt = first_run_screen();
    let text = provision::provision(&mut rt, &provision::identity_fixture(role), PROVISION_DEADLINE)
        .unwrap_or_else(|e| panic!("{role}: {e}"));
    assert!(text.contains("PROV OK id=test-"), "{text}");
    let (hash, step) = run_until_stable_frame(&mut rt, PROVISION_DEADLINE, REGISTERED_STABLE_FOR, &[FIRST_RUN_HASH]);
    assert_eq!(hash, registered_hash, "{role}: registered screen, stable at step {step}");
    assert_no_panic_text(&rt);
    rt
}

macro_rules! provisions_role {
    ($name:ident, $role:literal, $hash:expr) => {
        /// Provisioning through the console registers the badge (see `provisioned`).
        #[test]
        fn $name() {
            provisioned($role, $hash);
        }
    };
}

// One line per role in Task 1's ROLE_TABLE, in its order, each with its
// measured registered-screen hash, e.g.:
provisions_role!(provisions_hacker_through_the_console, "hacker", 0);
provisions_role!(provisions_organizer_through_the_console, "organizer", 0);
provisions_role!(provisions_sponsor_through_the_console, "sponsor", 0);
// ... judge, mentor, volunteer, media, staff, general, workshop_lead, visitor

/// Press and release `slot` (held 100,000 steps), asserting no fault.
fn press(rt: &mut FirmwareRuntime, slot: usize) {
    rt.set_raw_button(slot, true);
    let s = rt.run(100_000);
    assert_eq!(s.last_instruction_fault, None, "{s:?}");
    rt.set_raw_button(slot, false);
}

/// Provisions [`OWN_ROLE`] and walks [`TO_LAUNCHER_SLOTS`] to the stable
/// launcher, asserting every intermediate frame.
fn launcher() -> FirmwareRuntime {
    let mut rt = provisioned(OWN_ROLE, OWN_ROLE_REGISTERED_HASH);
    let mut previous = OWN_ROLE_REGISTERED_HASH;
    let (last, before_last) = TO_LAUNCHER_SLOTS.split_last().expect("at least one press");
    for (&slot, &expected) in before_last.iter().zip(TO_LAUNCHER_INTERMEDIATE_HASHES) {
        press(&mut rt, slot);
        let (hash, step) = run_until_stable_frame(&mut rt, LAUNCHER_MAX_STEPS, AFTER_PRESS_STABLE_FOR, &[previous]);
        assert_eq!(hash, expected, "after slot {slot}, stable at step {step}");
        previous = hash;
    }
    press(&mut rt, *last);
    let (hash, step) = run_until_stable_frame(&mut rt, LAUNCHER_MAX_STEPS, AFTER_PRESS_STABLE_FOR, &[previous]);
    assert_eq!(hash, LAUNCHER_HASH, "launcher, stable at step {step}");
    rt
}

/// Milestone 5 finish line: a provisioned badge reaches the app launcher.
/// The input sequence and the frame were compared by eye with the physical
/// badge (the notes' "Current state", Task 5's date).
#[test]
fn boots_to_launcher() {
    let rt = launcher();
    assert_no_panic_text(&rt);
}

/// The launcher takes input: one [`NAV_SLOT`] press moves the selection,
/// as on the badge.
#[test]
fn launcher_responds_to_navigation() {
    let mut rt = launcher();
    press(&mut rt, NAV_SLOT);
    let cap = rt.total_steps() + AFTER_PRESS_STABLE_FOR + 2_000_000;
    let (hash, step) = run_until_stable_frame(&mut rt, cap, AFTER_PRESS_STABLE_FOR, &[]);
    assert_ne!(hash, LAUNCHER_HASH, "slot {NAV_SLOT} changed nothing");
    assert_eq!(hash, LAUNCHER_AFTER_NAV_HASH, "stable at step {step}");
    assert_no_panic_text(&rt);
}
```

  If `AFTER_PRESS_STABLE_FOR` (8,000,000) is longer than Task 4 found necessary after these presses, keep it anyway (a longer window only costs time). If roles differ in launcher hash (Task 4 Step 3), add `boots_to_launcher_as_<role>` for each differing role, built like `launcher()` with that role's constants. Also land the held Task D rungs from `local/m5-held-rungs.rs`.

  Run: `cargo test -p emulator-core --release --test boot_progress` → all PASS. Suite time over ~10 s → Task 8 Step 1 first.

- [ ] **Step 4: WASM passthroughs.** In `emulator-wasm/src/lib.rs`, after `console_output`:

```rust
    /// Bytes a USB host sends to the badge's console; returns how many
    /// were accepted (see `FirmwareRuntime::serial_input`).
    #[wasm_bindgen(js_name = serialInput)]
    pub fn serial_input(&mut self, bytes: &[u8]) -> usize {
        self.inner.serial_input(bytes)
    }

    /// Serial input the firmware has not read yet.
    #[wasm_bindgen(js_name = serialPending)]
    pub fn serial_pending(&self) -> usize {
        self.inner.serial_pending()
    }
```

  In `frontend/src/cpu/bridge.ts`, add to `FirmwareEmulatorHandle`:

```ts
  /** Bytes a USB host sends to the badge's console; returns how many were accepted. */
  serialInput(bytes: Uint8Array): number;
  /** Serial input the firmware has not read yet. */
  serialPending(): number;
  /** Everything the firmware has printed to its console. */
  consoleOutput(): string;
```

  and to the object `createFirmwareEmulator` returns (same `assertLive()` pattern as its other methods):

```ts
    serialInput(bytes: Uint8Array): number {
      assertLive();
      return wasm.serialInput(bytes);
    },
    serialPending(): number {
      assertLive();
      return wasm.serialPending();
    },
    consoleOutput(): string {
      assertLive();
      return wasm.consoleOutput();
    },
```

  Add the three methods to the fake handle in `frontend/test/firmware-runtime.test.ts` (`serialInput: () => 0, serialPending: () => 0, consoleOutput: () => ""`) so typecheck passes.

- [ ] **Step 5: The provisioner, test first.** `frontend/test/provisioner.test.ts`:

```ts
import { describe, expect, test } from "bun:test";
import { createProvisioner } from "../src/runtime/provisioner";
import type { FirmwareEmulatorHandle } from "../src/cpu/bridge";

/** A fake console: records typed bytes; `print` appends firmware output. */
function fakeHandle() {
  let console = "";
  let steps = 0;
  const typed: string[] = [];
  const handle = {
    serialInput(bytes: Uint8Array) { typed.push(new TextDecoder().decode(bytes)); return bytes.length; },
    serialPending: () => 0,
    consoleOutput: () => console,
    totalSteps: () => steps,
    run(n: number) { steps += n; return { steps: n, traps: 0, romStubCalls: 0, pc: 0, lastInstructionFault: undefined }; },
  } as unknown as FirmwareEmulatorHandle;
  return { handle, typed, print: (s: string) => (console += s), advance: (n: number) => (steps += n) };
}

describe("provisioner", () => {
  test("provisioner waits for the prompt before typing", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, '{"a":1}');
    expect(p.poll().kind).toBe("waiting");
    expect(f.typed).toEqual([]);
    f.print("badge> ");
    expect(p.poll().kind).toBe("typing");
    expect(f.typed).toEqual(["put /littlefs/identity.json 7\n"]);
  });

  test("types put, the payload after the settle steps, then prov apply", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, '{"a":1}');
    f.print("badge> ");
    p.poll();
    p.poll();
    expect(f.typed.length).toBe(1);
    f.advance(200_000);
    p.poll();
    expect(f.typed[1]).toBe('{"a":1}');
    f.print("OK 7\r\nbadge> ");
    p.poll();
    expect(f.typed[2]).toBe("prov apply\n");
    f.print("PROV OK id=test-fake-badge-0001\r\n");
    expect(p.poll()).toEqual({ kind: "ok", line: "PROV OK id=test-fake-badge-0001" });
  });

  test("reports PROV FAIL and put errors", () => {
    const f = fakeHandle();
    const p = createProvisioner(f.handle, "{}");
    f.print("badge> ");
    p.poll();
    f.advance(200_000);
    p.poll();
    f.print("short read: 2 bytes missing\r\n");
    expect(p.poll()).toEqual({ kind: "failed", reason: "short read: 2 bytes missing" });
  });
});
```

  Run: `bun test test/provisioner.test.ts` → FAIL (module not found). Then `frontend/src/runtime/provisioner.ts` (keep `LINE_END`, `PUT_SETTLE_STEPS` and `IDENTITY_PATH` equal to the Rust helper's; it is a deliberate copy of a ~10-line wire protocol that belongs to the firmware):

```ts
/**
 * Types the registration-desk provisioning flow into the emulated badge's
 * USB console: `put /littlefs/identity.json <len>`, the JSON bytes, then
 * `prov apply` — the firmware's own commands (docs/milestone-5-decisions.md,
 * "Trace facts"). Mirrors emulator-core/tests/common/provision.rs; polled
 * once per frame by the firmware runtime and step by step by the WASM twin.
 */
import type { FirmwareEmulatorHandle } from "../cpu/bridge";

export const PROMPT = "badge> ";
export const IDENTITY_PATH = "/littlefs/identity.json";
export const LINE_END = "\n";
export const PUT_SETTLE_STEPS = 200_000;

export type ProvisionState =
  | { kind: "waiting" | "typing" }
  | { kind: "ok"; line: string }
  | { kind: "failed"; reason: string };

export interface Provisioner {
  poll(): ProvisionState;
}

const PUT_ERRORS = ["bad size", "write error", "short read"];

function lineContaining(text: string, needle: string): string {
  const at = text.indexOf(needle);
  const end = text.slice(at).search(/\r?\n/);
  return (end < 0 ? text.slice(at) : text.slice(at, at + end)).trim();
}

export function createProvisioner(handle: FirmwareEmulatorHandle, identityJson: string): Provisioner {
  const payload = new TextEncoder().encode(identityJson);
  const send = (s: string | Uint8Array) =>
    handle.serialInput(typeof s === "string" ? new TextEncoder().encode(s) : s);
  type Phase = "prompt" | "settle" | "putReply" | "applyReply" | "done";
  let phase: Phase = "prompt";
  let marker = 0;
  let settleUntil = 0;
  let result: ProvisionState = { kind: "waiting" };

  function since(): string {
    return handle.consoleOutput().slice(marker);
  }

  return {
    poll(): ProvisionState {
      switch (phase) {
        case "prompt":
          if (!handle.consoleOutput().includes(PROMPT)) return result;
          marker = handle.consoleOutput().length;
          send(`put ${IDENTITY_PATH} ${payload.length}${LINE_END}`);
          settleUntil = handle.totalSteps() + PUT_SETTLE_STEPS;
          phase = "settle";
          return (result = { kind: "typing" });
        case "settle":
          if (handle.serialPending() > 0 || handle.totalSteps() < settleUntil) return result;
          send(payload);
          phase = "putReply";
          return result;
        case "putReply": {
          const text = since();
          const err = PUT_ERRORS.find((e) => text.includes(e));
          if (err) {
            phase = "done";
            return (result = { kind: "failed", reason: lineContaining(text, err) });
          }
          if (!text.includes(`OK ${payload.length}`)) return result;
          marker = handle.consoleOutput().length;
          send(`prov apply${LINE_END}`);
          phase = "applyReply";
          return result;
        }
        case "applyReply": {
          const text = since();
          if (text.includes("PROV OK")) {
            phase = "done";
            return (result = { kind: "ok", line: lineContaining(text, "PROV OK") });
          }
          if (text.includes("PROV FAIL")) {
            phase = "done";
            return (result = { kind: "failed", reason: lineContaining(text, "PROV FAIL") });
          }
          return result;
        }
        case "done":
          return result;
      }
    },
  };
}
```

  The second test calls `poll()` twice before the settle steps pass; the second call must not type again (it is in `settle`). Run: `bun test test/provisioner.test.ts` → PASS.

- [ ] **Step 6: WASM twin** (append to `frontend/test/cpu-wasm.test.ts`; add `import { createProvisioner, type ProvisionState } from "../src/runtime/provisioner";` to its imports). The constants are copied from `boot_progress.rs` (same values, same names in camel-free `SCREAMING_CASE`; no `0n` left):

```ts
/** boot_progress.rs's constants, copied (Milestone 5 finish line). */
const FIRST_RUN_MAX_STEPS = 17_500_000;
const BOOT_STABLE_FOR = 1_000_000;
const AFTER_PRESS_STABLE_FOR = 8_000_000;
const PROVISION_DEADLINE = 0; // boot_progress.rs's value
const REGISTERED_STABLE_FOR = 0; // boot_progress.rs's value
const OWN_ROLE = "hacker"; // boot_progress.rs's value
const OWN_ROLE_REGISTERED_HASH = 0n; // boot_progress.rs's value
const TO_LAUNCHER_SLOTS: number[] = []; // boot_progress.rs's value
const TO_LAUNCHER_INTERMEDIATE_HASHES: bigint[] = []; // boot_progress.rs's value
const LAUNCHER_MAX_STEPS = 0; // boot_progress.rs's value
const LAUNCHER_HASH = 0n; // boot_progress.rs's value

/** boot_progress.rs's run_until_stable_frame, over the bridge. */
function runUntilStable(
  handle: FirmwareEmulatorHandle,
  maxSteps: number,
  stableFor: number,
  skip: bigint[],
): bigint {
  let hash = fnv1a(handle.framebuffer());
  let since = handle.totalSteps();
  while (handle.totalSteps() < maxSteps) {
    const r = handle.run(250_000);
    expect(r.lastInstructionFault).toBeUndefined();
    const fb = handle.framebuffer();
    const h = fnv1a(fb);
    if (h !== hash) {
      hash = h;
      since = handle.totalSteps();
    } else if (handle.totalSteps() - since >= stableFor && fb.some((px) => px !== fb[0]) && !skip.includes(h)) {
      return hash;
    }
  }
  throw new Error(`no stable frame by ${maxSteps}; last hash ${hash.toString(16)}`);
}

function press(handle: FirmwareEmulatorHandle, slot: number): void {
  handle.setRawButton(slot, true);
  expect(handle.run(100_000).lastInstructionFault).toBeUndefined();
  handle.setRawButton(slot, false);
}

describe("emulator-wasm provisioned boot", () => {
  test(
    "provisions the test identity through the console and reaches the same launcher as the native finish line",
    () => {
      initCpuWasmSync(readFileSync(new URL("../src/cpu/wasm-pkg/emulator_wasm_bg.wasm", import.meta.url)));
      const image = readFileSync(new URL("../public/firmware/factory.bin", import.meta.url));
      const handle = createFirmwareEmulator(new Uint8Array(image));
      try {
        expect(runUntilStable(handle, FIRST_RUN_MAX_STEPS, BOOT_STABLE_FOR, [SPLASH_HASH])).toBe(FIRST_RUN_HASH);

        const json = readFileSync(
          new URL(`../public/firmware/test-identities/${OWN_ROLE}.json`, import.meta.url),
          "utf8",
        );
        const p = createProvisioner(handle, json);
        let state: ProvisionState = p.poll();
        while ((state.kind === "waiting" || state.kind === "typing") && handle.totalSteps() < PROVISION_DEADLINE) {
          expect(handle.run(50_000).lastInstructionFault).toBeUndefined();
          state = p.poll();
        }
        expect(state.kind).toBe("ok");

        let previous = runUntilStable(handle, PROVISION_DEADLINE, REGISTERED_STABLE_FOR, [FIRST_RUN_HASH]);
        expect(previous).toBe(OWN_ROLE_REGISTERED_HASH);
        TO_LAUNCHER_SLOTS.forEach((slot, i) => {
          press(handle, slot);
          const h = runUntilStable(handle, LAUNCHER_MAX_STEPS, AFTER_PRESS_STABLE_FOR, [previous]);
          expect(h).toBe(i < TO_LAUNCHER_INTERMEDIATE_HASHES.length ? TO_LAUNCHER_INTERMEDIATE_HASHES[i] : LAUNCHER_HASH);
          previous = h;
        });
        expect(previous).toBe(LAUNCHER_HASH);
      } finally {
        handle.dispose();
      }
    },
    { timeout: 600_000 },
  );
});
```

  Add `import type { FirmwareEmulatorHandle } from "../src/cpu/bridge";` if not already imported. Fill every `// boot_progress.rs's value` constant from the native rung (no `0`, `0n` or empty array left). Run: `bun run build:wasm && bun test test/cpu-wasm.test.ts` → PASS.

- [ ] **Step 7: Docs, full suite, commit.** Notes: "Data-handling note" gains a paragraph: what `test-identities/` holds (obviously fake records, one per role; built from the traced schema, never from a dump), that the organizers permitted committing them (date), and that the guard test keeps them fake. Decisions: the held-rung landing, any role-specific launcher finding.

```bash
git add -A emulator-core emulator-wasm frontend docs
git commit -m "test: provision through the console and boot to the launcher (Milestone 5)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Frontend — role picker and "Provision test badge"

**Files:**
- Modify: `frontend/src/runtime/firmware-runtime.ts` (`FirmwareRuntime` gains `provision`/`provisionState`; `reset` drops the provisioner; the frame step polls it)
- Modify: `frontend/src/ui/shell.ts` (`mountProvisionControls`), `frontend/src/main.ts` (wiring), `frontend/index.html` (`<div id="provision-controls">` after `#mode-toggle`), `frontend/src/ui/shell.css`
- Test: `frontend/test/firmware-runtime.test.ts`, `frontend/test/shell.test.ts` (create)

**Interfaces:**
- Consumes: Task 6's `createProvisioner`, `ProvisionState`, `TEST_IDENTITY_ROLES`, `testIdentityUrl`, `FirmwareEmulatorHandle`.
- Produces: `FirmwareRuntime.{provision(identityJson: string): void; provisionState(): ProvisionState | null}`; `ShellHandles.mountProvisionControls(roles: readonly string[], onProvision: (role: string) => void): { setStatus(text: string): void; setVisible(visible: boolean): void }`.

- [ ] **Step 1: Failing runtime tests** (append to `frontend/test/firmware-runtime.test.ts`, reusing its fake handle and fake scheduler; extend the fake handle so `consoleOutput` returns a settable string and `serialInput` records calls):

```ts
/** A fake console handle plus a manual frame scheduler. */
function provisioningRig() {
  let consoleText = "";
  const typed: string[] = [];
  const { handle } = makeFakeHandle({
    serialInput(bytes: Uint8Array): number {
      typed.push(new TextDecoder().decode(bytes));
      return bytes.length;
    },
    serialPending: () => 0,
    consoleOutput: () => consoleText,
  });
  const queued: Array<(t: number) => void> = [];
  const rt = createFirmwareRuntime(handle, makeStubCtx().ctx, {
    scheduleFrame: (cb) => queued.push(cb),
    cancelFrame: () => {},
  });
  const frame = () => queued.shift()?.(0);
  return { rt, typed, frame, print: (s: string) => (consoleText += s) };
}

describe("FirmwareRuntime provisioning", () => {
  test("provision() types once the prompt appears, polled by the frame loop", () => {
    const { rt, typed, frame, print } = provisioningRig();
    rt.provision('{"a":1}');
    rt.start();
    frame();
    expect(typed).toEqual([]);
    expect(rt.provisionState()?.kind).toBe("waiting");
    print("badge> ");
    frame();
    expect(typed).toEqual(["put /littlefs/identity.json 7\n"]);
  });

  test("reset abandons an in-flight provisioner", () => {
    const { rt, typed, frame, print } = provisioningRig();
    rt.provision('{"a":1}');
    rt.start();
    print("badge> ");
    frame();
    expect(typed.length).toBe(1);
    rt.reset();
    expect(rt.provisionState()).toBeNull();
    print("OK 7\r\nbadge> ");
    frame();
    frame();
    expect(typed.length).toBe(1);
  });

  test("a second provision() replaces the first", () => {
    const { rt, typed, frame, print } = provisioningRig();
    rt.provision('{"a":1}');
    rt.provision('{"b":22}');
    rt.start();
    print("badge> ");
    frame();
    expect(typed).toEqual(["put /littlefs/identity.json 8\n"]);
  });
});
```

  The fake handle's `totalSteps()` returns 0, so these tests never pass the settle phase; that is intended (they test start, reset and replacement only). Also add `serialInput: () => 0, serialPending: () => 0, consoleOutput: () => ""` to `makeFakeHandle`'s defaults if Task 6 Step 4 has not already. Run: `bun test test/firmware-runtime.test.ts` → FAIL (`provision` is not a function).

- [ ] **Step 2: Implement in `firmware-runtime.ts`.** Add to the `FirmwareRuntime` interface:

```ts
  /** Starts typing the provisioning flow for `identityJson`, replacing any in flight. */
  provision(identityJson: string): void;
  /** The current provisioning state, or `null` if none was started since the last reset. */
  provisionState(): ProvisionState | null;
```

  In `createFirmwareRuntime`: hold `let provisioner: Provisioner | null = null; let lastState: ProvisionState | null = null;`; `provision(json) { provisioner = createProvisioner(handle, json); lastState = { kind: "waiting" }; }`; in the frame callback, after `stepFirmwareFrame(...)`, `if (provisioner) lastState = provisioner.poll();`; `provisionState: () => lastState`; in `reset()`, `provisioner = null; lastState = null;` before `handle.reset()`. Run the tests → PASS.

- [ ] **Step 3: Shell controls.** `bun test` has no DOM and `shell.ts` has no DOM tests today (`ui.test.ts` covers the Lua `badge.ui` module), so the DOM glue is checked by Step 5's manual run and only its pure part is unit-tested. Test first, in a new `frontend/test/shell.test.ts`:

```ts
import { describe, expect, test } from "bun:test";
import { roleLabel } from "../src/ui/shell";

describe("roleLabel", () => {
  test("shows a firmware role string as words", () => {
    expect(roleLabel("hacker")).toBe("hacker");
    expect(roleLabel("workshop_lead")).toBe("workshop lead");
  });
});
```

  Run: `bun test test/shell.test.ts` → FAIL (no export). Implement in `shell.ts`:

```ts
/** A firmware role string (`workshop_lead`) as a menu label (`workshop lead`). */
export function roleLabel(role: string): string {
  return role.replaceAll("_", " ");
}
```

  and `mountProvisionControls(roles, onProvision)` on the returned `ShellHandles`: inside `#provision-controls` (return no-op handles if the element is missing, as `mountModeToggle` does), a `<select>` with one `<option value=role>` per role labelled `roleLabel(role)`, a `<button type="button">Provision test badge</button>` whose click calls `onProvision(select.value)`, and a `<span class="provision-status">`; `setStatus(text)` sets the span's `textContent`; `setVisible(v)` sets the container's `style.display` to `""` or `"none"`. Add the method to the `ShellHandles` interface with a doc comment. Minimal CSS in `shell.css` reusing `.mode-btn`'s look for the button. Run → PASS; `bun run typecheck` → PASS.

- [ ] **Step 4: Wire in `main.ts`.** After the mode toggle is mounted: `const provisionControls = shell.mountProvisionControls(TEST_IDENTITY_ROLES, async (role) => { const rt = await ensureFirmwareRuntime(); const json = await (await fetch(testIdentityUrl(role))).text(); rt.provision(json); });`; `provisionControls.setVisible(mode === "firmware")` initially and in `setMode`; in the firmware paint path (or a 250 ms `setInterval` active only in firmware mode), `setStatus` from `firmwareRuntime?.provisionState()`: `waiting` → "waiting for the console…", `typing` → "provisioning…", `ok` → the `PROV OK` line, `failed` → "failed: " + reason, `null` → "". A failed `fetch` sets the status to "could not load <role>".

- [ ] **Step 5: Manual check.** `bun run build:wasm && bun run dev`; in firmware mode pick `hacker`, click the button during the splash; expect "waiting for the console…", then `PROV OK id=test-…`, the registered screen, and the launcher after the Task 4 input sequence on the button pad. Use the `run` skill or the browser to confirm; record the outcome in the ledger.

- [ ] **Step 6: Full suite, commit.**

```bash
git add -A frontend
git commit -m "feat(frontend): provision the emulated badge with a test identity

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Suite time and docs

**Files:** `emulator-core/src/**` (only if Step 1 triggers), `emulator-core/tests/boot_progress.rs`, `docs/firmware-emulator-notes.md`, `docs/milestone-5-decisions.md`, `CLAUDE.md`, module docs of `usb_serial_jtag.rs`/`bus.rs`/`runtime.rs`.

- [ ] **Step 1: Checkpoint fixture (only if `time cargo test -p emulator-core --release --test boot_progress` exceeds ~10 s).** M4 spec Part 4, as its own commit: `#[derive(Clone)]` on `FirmwareRuntime`, `Cpu`, `FirmwareBus` and every peripheral (the ROM stub table is plain data); a `static FIRST_RUN: OnceLock<FirmwareRuntime>` in `boot_progress.rs` booted once to the stable first-run screen; `first_run_screen()` returns `FIRST_RUN.get_or_init(...).clone()`; a unit test in `runtime.rs` that a clone and its original, each run 100,000 more steps, have equal `pc()`, `total_steps()` and framebuffers. Commit `test(emulator-core): share one first-run checkpoint across boot rungs`.

- [ ] **Step 2: Docs.**
  - Notes: "Current state (end of Milestone 5)" replacing M4's (M4's moves under history): console starts, provisioning through the console, per-role registered screens, the launcher and navigation, human comparison date, what is still unmodeled; open limitations renumbered (the identity one resolved).
  - `docs/milestone-5-decisions.md`: complete every task's decisions; "Remarks" (finish-line tests and hashes, the permission date); "Milestone 6 backlog": the M4 backlog items still open (accelerometer, LED decoding, NFC, START busy phase, MADCTL if skipped, the M3 carry-overs), plus new ones (a console pane, anything Task 4 found).
  - `CLAUDE.md`: the "Current state" paragraph (launcher reached by provisioning through the console; next blocker), the `peripherals/` entry (USB-Serial-JTAG RX, interrupt source 26, pacing), `runtime.rs` (`serial_input`), the frontend entries (`provisioner.ts`, `test-identities.ts`, the controls), the Commands section's finish-line rung names, and the "Local-only data" paragraph (fake identities committed under `frontend/public/firmware/test-identities/` with permission; real ones never).
  - `tests/boot_progress.rs` module doc: the finish line is now the provisioning and launcher rungs; history stays in the notes.

- [ ] **Step 3: Full suite, commit.**

```bash
git add -A docs CLAUDE.md emulator-core
git commit -m "docs: Milestone 5 current state, decisions and M6 backlog

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9 (stretch): open a built-in app

Droppable without failing M5; if dropped, it heads the M6 backlog. Start after Task 8.

**Files:** `emulator-core/tests/boot_progress.rs`, docs; Task D instances as needed.

- [ ] **Step 1:** In the Task 4 scratch example, from the stable launcher, navigate to Snake or Dice (whichever needs fewer presses), press A (slot 1) or the launcher's confirm button, and dump stable frames. Then one input (a direction for Snake, A for Dice). Stalls → Task D.
- [ ] **Step 2:** Human comparison of the app frames with the badge (as Task 5).
- [ ] **Step 3:** Rung `launcher_opens_<app>` (from the launcher state of `boots_to_launcher`: the navigation presses, the open press, its pinned first stable frame, one input, a second pinned hash, no fault, no panic text). Docs (notes current state, decisions remarks), full suite, commit `test: open <app> from the launcher (Milestone 5 stretch)`.

---

## Orchestrator notes

- Order: Task 1 → 2 → 3 → (Task D instances as boot needs) → 4 (+ C1/C2/Task D as triggered) → 5 → 6 (waits on permission) → 7 → 8 → 9 (stretch) → whole-branch review → `superpowers:finishing-a-development-branch` (the human partner decides merge/PR/push).
- If permission has not arrived when Task 5 is done, pause and ask the human partner for the fallback (R-M5-2); do not start Task 6.
- Give each implementer: this plan's Global Constraints + Review Focus, their task text, the spec path, the "Trace facts" section, the latest ledger findings. Never paste fixture contents or any `local/` data into a brief; give paths.
- Reviewers check: header citations present and correct, byte-split register behavior, never-panic on browser input, no exact step counts in rungs beyond budgets, no `local_*` example or `local/` file committed, no identity in any pre-gate commit (`git log -p` on the task's range must not contain `test-identities`, `identity.json` payloads or `PROV OK id=`), docs updated in the same commit.
- Human partner checkpoints: R-M5-3 stops; the REPL fallback (spec: if the console cannot run on the first-run screen); Task 5 (frames); Task 6 (permission); C2 if triggered; Task 9 Step 2; any override of a recorded decision.

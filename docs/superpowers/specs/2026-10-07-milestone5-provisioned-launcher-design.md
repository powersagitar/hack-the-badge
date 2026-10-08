# Milestone 5 — Provisioned boot to the app launcher

Status: approved design (2026-10-07). Implementation plan:
`docs/superpowers/plans/2026-10-07-milestone5-provisioned-launcher.md`.

## Goal

Take the real-firmware emulator from Milestone 4's end state (blank-flash
boot settles on My Badge's "Not registered yet" first-run screen; START
opens the hardware self-test; the launcher is unreachable because the
badge is unprovisioned) to the firmware's **app launcher**, by
provisioning the emulated badge the way a registration desk does: over
the USB console, through the real firmware's own `put` / `prov apply`
REPL commands. Prove it natively, through the WASM build and by a human
comparison, for every role the firmware knows. Stretch: open a built-in
app from the launcher.

### Why the console, not a pre-built filesystem

`factory.bin` carries a provisioning REPL (strings: `prov
<apply|show|mac|erase confirm>`, `provision flow: put %s <size> ... then:
prov apply`, `PROV OK id=%s`, `PROV FAIL invalid or missing %s`), and the
first-run screen tells the attendee to "Bring it to a registration desk".
So a real unprovisioned badge is provisioned over USB-Serial-JTAG while it
shows that screen. Driving that path means the firmware validates the
record and writes it through its own littlefs; the emulator needs only
the USB-Serial-JTAG receive direction (a register model, already a
backlog item), no littlefs code of its own and no committed binary image.
**Found while planning (2026-10-07).** The console is
`hal_console_start()` (`./main/hal/hal_console.cpp`), which calls
`esp_console_new_repl_usb_serial_jtag` (the interrupt-driven
`usb_serial_jtag` driver, prompt `badge> `) and logs `hal_console:
console started`. On the physical badge that line follows `app_reg:
launched My Badge` by about ten lines (untagged REPL output), then
`main_task: Returned from app_main()`. The emulator never prints it (60M
steps on blank flash): `app_main` is held up between the first app launch
and the console start. The likely cause is the one
`milestone-4-decisions.md` flagged: the driver queues console bytes in a
ring buffer that only its ISR moves to the FIFO, and no USB-Serial-JTAG
interrupt source is modeled. So the receive model below includes the
interrupt source unconditionally, and Task 3's first rung is the console
starting. The REPL's own commands include `put <path> <size>` ("then send
<size> bytes", answering `OK <n>`), `cat`, `ls` and `press <button>`.

Rejected alternatives: a committed littlefs image built by littlefs-python
(commits a ~1.25 MiB binary or puts Python in the build; a config mismatch
makes the firmware silently reformat), and a littlefs v2 writer in Rust
(large, for one small file). If Task 1 finds the REPL does not run on the
first-run screen, the plan stops and asks (the image approach is the
fallback the human partner would rule on).

### Success criteria

1. **Trace recorded** (Task 1, notes and decisions doc only): which code
   sets the provisioned flag (`0x3fca_92b2`) and from what;
   `hal_identity`'s validation rules (size range, JSON, per-field maximum
   lengths, required non-empty `badge_id` and `display_name`, role
   lookup with the "unknown role, defaulting to hacker" fallback); the
   firmware's role table; whether the console REPL runs on blank flash
   and through which driver (polling VFS or the interrupt-driven
   `usb_serial_jtag` driver); the exact `put` syntax and where its bytes
   go; what `prov apply` does next (redraw, onboarding, reboot); and why
   `app_main` does not reach `hal_console_start()` in the emulator (see
   "Found while planning"). Every fact cites a firmware address.
2. **The console accepts input.** The USB-Serial-JTAG RX direction is
   modeled register-faithfully; rung `console_answers_prov_show_on_the_first_run_screen`:
   `prov show` typed on the first-run screen prints `provisioned=0`.
3. **The put/apply path works with no identity.** Rung
   `console_rejects_an_empty_identity`: a `put` of `{}` then `prov apply`
   prints `PROV FAIL invalid or missing` (the field named is whatever the
   trace shows is checked first), the frame stays on the first-run hash,
   no fault.
4. **Every role provisions** (post-gate): for each role in the traced
   role table, `provisions_<role>_through_the_console` sees `PROV OK`, then
   a stable My Badge registered screen pinned by hash, with no fault and
   no panic text.
5. **`boots_to_launcher`** (post-gate): for the role the human partner
   holds on the physical badge, provisioning then the input that leaves
   My Badge (HOME, or whatever onboarding requires first) reaches a stable
   launcher frame pinned by hash. Other roles get their own launcher hash
   only if their launcher differs (e.g. a role-dependent app set).
6. **`launcher_responds_to_navigation`** (post-gate): from the stable
   launcher, a DOWN (slot 4) or RIGHT (slot 6) press, whichever moves the
   selection, settles on a second pinned hash.
7. **Blank-flash rungs unchanged**: `boots_to_first_real_frame`,
   `boots_to_first_run_screen`, `first_run_screen_responds_to_start` and
   their WASM twins keep their hashes.
8. **WASM twin** in `frontend/test/cpu-wasm.test.ts`: provisioning the
   same role through the bridge reaches the same launcher hash.
9. **Frontend**: in firmware mode, a role picker and a "Provision test
   badge" button type the selected fixture's provisioning sequence into
   the emulated console through a logic-free `serialInput(bytes)`
   passthrough.
10. **Human comparison** (the one manual gate on frames): the human
    partner compares the dumped launcher, the navigation frame and the
    registered screen's layout (their own name will differ) with the
    physical badge and confirms a match. Roles other than the partner's
    are pinned and documented as "not compared with hardware".
11. All suites green: `cargo test --workspace`, `cargo test -p
    emulator-core --release --test boot_progress`, `cargo clippy
    --workspace --all-targets` (no new warnings), `bun run build:wasm`,
    `bun test`, `bun run typecheck`.

**Stretch (Task 9, droppable without failing M5):** open Snake or Dice
(whichever the launcher offers with the fewest presses) and pin its first
stable frame plus one input response.

### Out of scope (stays in the backlog unless a stall implicates it)

The SC7A20H accelerometer model, RMT LED decoding and a frontend LED
view, NFC, START's ~6M-step busy phase, a free-text console pane in the
frontend, real-time calibration (`TICKS_PER_STEP`, `CPU_FREQ_MHZ`), the
eFuse block. ST7789 `MADCTL` is conditional, as in M4: only if a launcher
or app frame fails the human comparison on orientation.

## Rulings (recorded in `docs/milestone-5-decisions.md`)

- **R-M5-1, format source.** Every committed fact about the identity
  format (field names, types, maximum lengths, the role table, the
  validation rules) is derived from tracing `factory.bin` and cites the
  firmware address it came from. `local/m5-identity/FINDINGS.md` (the
  structure of the human partner's registered-badge dump, parsed locally
  with their permission) may be used only locally, to cross-check the
  trace and to notice fields the trace missed; it is never the source of
  a committed value or byte. This replaces `milestone-4-decisions.md`'s
  "its format to be read from the firmware, never from the physical
  badge's dump" (Task 1 edits that sentence to point here).
- **R-M5-2, permission gate.** Fake identities exist only under `local/`
  (gitignored) until the human partner relays the Hack the North
  organizers' permission to commit them. One gate task (Task 6) then
  commits the fixtures and every test that needs them. Pre-gate tasks
  commit only code, unit tests and rungs that use no identity. If the
  permission has not arrived when the pre-gate work is done, the
  milestone pauses at the gate; what to ship without it (e.g. a
  test-time-generated identity, or nothing identity-shaped) is the human
  partner's decision then, not pre-decided here.
- **R-M5-3, technical stop conditions.** If the trace finds an offline
  signature, HMAC or other cryptographic check of the identity or the
  `badge_upload/credential` token, or a binding of the identity to the
  chip's MAC address or eFuse, the work stops and asks the human partner.
  No workaround (no forged signature, no patched check, no PC
  interception).
- **R-M5-4, fixtures.** One fixture per role in the traced role table
  (from the strings, expected: `hacker`, `organizer`, `sponsor`,
  `judge`, `mentor`, `volunteer`, `media`, `staff`, `general`,
  `workshop_lead`, `visitor`; the trace decides). Each is written by hand
  from the traced schema, never derived from the dump, and is obviously
  fake: `display_name` "Test <Role>", addresses at `example.com` (RFC
  2606), a `badge_id` beginning `test-` in the traced shape, empty
  optional `net_*` fields, numeric fields set to small obviously
  synthetic values, every string within the traced maximum. Committed at
  the gate as `frontend/public/firmware/test-identities/<role>.json`, the
  single copy both the Rust tests and the frontend read.
- **R-M5-5, flash stays synthetic.** `EmulatedFlash` still starts blank
  (partition table + `factory.bin`). The provisioned state only ever comes
  from what the firmware itself writes in response to console input; the
  emulator never writes littlefs or NVS contents itself.

## Constraints (carried from Milestone 4, unchanged unless noted)

- ESP-IDF **v5.5.3** headers/HAL/LL sources are the register authority.
  Every modeled register cites its source in the module doc. Fetch from
  `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`
  into the session scratchpad; never guess an offset.
- No PC-based interception of ESP-IDF (non-ROM) code. ROM stubs only, via
  `src/rom.rs` + `cpu/rom_stubs.rs`, addresses from the `rom*.ld`
  scripts. An observed ROM call that faults gets a real-implementation
  stub in the task that hits it (M4 rulings R10/R12).
- Finish-line tests are never narrowed or loosened.
- `FirmwareBus` dispatch stays an ordered sequence of concrete named
  fields; no `dyn` dispatch. `emulator-core` has no wasm-bindgen
  dependency; `emulator-wasm` stays logic-free.
- Unmapped data access never panics (reads 0, writes drop, logged);
  unmapped instruction fetch always traps. Browser-supplied input (an
  image, or now serial bytes) must never panic the emulator.
- Never commit, quote or print values from `local/full_flash_dump.bin`,
  `local/boot_log.txt` or anything derived from them, including in
  subagent briefs and reports. Contact filenames in the dump are other
  attendees' badge IDs: personal data too. Committed tests depend only on
  `factory.bin` plus committed synthetic data.
- Any committed identity is obviously fake, non-personal, built from
  scratch, and committed only under R-M5-2.
- Python via `local/.venv/bin/python` only (littlefs-python is installed
  there, for local inspection of emulated flash if needed); JS/TS via Bun
  only.
- The decisions in `docs/milestone-3-decisions.md` and
  `docs/milestone-4-decisions.md` stand unless the human partner OKs an
  override, recorded with a reason. (The R-M5-1 edit above is such an
  override, approved 2026-10-07.)
- After any change under `emulator-core/` or `emulator-wasm/`, the full
  suite (criterion 11) passes before commit.
- Pushing is the human partner's call.

## Design

### Part 1 — Trace (Task 1)

Disassemble `factory.bin` (the crate's own decoder or
`riscv64-unknown-elf-objdump` on extracted segments; `boot-probe` with
breakpoint-style scratch examples, never committed) starting from the
strings `hal_identity`, `/littlefs/identity.json`, `prov`, `put`,
`PROV OK`, `badge_upload`, `credential`, `tutorial=reset`, `app_mode`,
`onboard_build`, and from M4's addresses (flag byte `0x3fca_92b2`, its
reader `0x4200_ee06`, My Badge's HOME hook `0x4201_3920`, button handler
`0x4201_4130`, launcher entry `0x4203_8cba`). Answer every point of
criterion 1, recording each in the notes ("Milestone 5 Task 1: the
provisioning path") and the decisions doc. Apply R-M5-3. Write the local
fixtures (R-M5-4) under `local/m5-identities/` and cross-check them
against FINDINGS.md's structure (R-M5-1). No emulator code changes.

### Part 2 — USB-Serial-JTAG receive (Task 2)

In `emulator-core/src/peripherals/usb_serial_jtag.rs`, cited against
v5.5.3 `soc/esp32c3/register/soc/usb_serial_jtag_reg.h` and
`hal/esp32c3/include/hal/usb_serial_jtag_ll.h`:

- **Host queue.** An unbounded `VecDeque<u8>` stands for bytes the USB
  host has buffered to send. It feeds a 64-byte OUT FIFO one packet
  (≤ 64 bytes, the endpoint's max packet size) at a time: when the FIFO
  is empty and the host queue is not, the next packet moves in and
  `SERIAL_OUT_RECV_PKT_INT_RAW` is set.
- **FIFO reads.** While the FIFO holds data, `EP1_CONF_REG`'s
  `SERIAL_OUT_EP_DATA_AVAIL` reads 1 and each byte-0 read of `EP1_REG`
  pops one byte; empty, it reads 0 (today's behavior). Byte reads of
  `EP1_REG` other than byte 0 do not pop.
- **Interrupt status.** `INT_ST` becomes `INT_RAW & INT_ENA` instead of
  plain storage; `INT_CLR` clears raw bits (the forced ones re-assert on
  the next read, as today). `ETS_USB_SERIAL_JTAG_INTR_SOURCE` (number
  from `soc/esp32c3/include/soc/interrupts.h`) is asserted as a level in
  the bus's `pending_sources()` while `INT_ST` is non-zero. The
  forced `SERIAL_IN_EMPTY_INT_RAW` is what the driver's TX path wants: its
  ISR enables `SERIAL_IN_EMPTY` only while its ring buffer holds bytes,
  moves them to the FIFO (always empty here) and disables it again when
  the ring buffer drains (v5.5.3 `esp_driver_usb_serial_jtag/src/usb_serial_jtag.c`).
  Decision to record: a source that stays asserted while its ISR makes no
  progress (an interrupt storm: the ISR's PCs dominate the hot-PC list
  and no console byte or frame changes), e.g. if the firmware enabled the
  forced `SOF_INT`, makes the implementer stop and report rather than
  model around it.
- **Runtime API.** `FirmwareRuntime::serial_input(&mut self, bytes:
  &[u8]) -> usize` appends to the host queue and returns how many bytes
  it accepted; `serial_pending(&self) -> usize` reports bytes the
  firmware has not read yet (host queue plus FIFO). No interpretation of
  the bytes. The host queue is capped at 1 MiB (excess is refused, never
  a panic) so a browser cannot grow emulator memory without bound.
- **Packet pacing.** A packet moves into the FIFO only when the FIFO is
  empty and at least one packet time has passed since the previous one:
  821 SYSTIMER ticks, a 64-byte full-speed bulk packet with its token and
  handshake (~616 bits at 12 Mbit/s, ~51 µs) at SYSTIMER's 16 MHz.
  Delivering packets back-to-back in zero time would let the driver's ISR
  fill its receive ring buffer before the console task runs, dropping
  bytes a real host's line rate never would. The WFI fast-forward stops
  at the next packet time as it does at the next SYSTIMER alarm, so input
  wakes an idle core.

Unit tests drive the registers with byte-split word writes in the LL
functions' order: packet loading and the 64-byte boundary, `DATA_AVAIL`
transitions, pop order, `INT_ST` masking, clear-then-reload of the
packet interrupt, and an empty-queue read returning 0.

### Part 3 — Console rungs and the provisioning helper (Task 3)

A shared test module (`emulator-core/tests/common/provision.rs`, used by
`boot_progress.rs`) with:

- `type_line(rt, text)`: queue `text` + the line ending the REPL expects
  (from Task 1), then run until `serial_pending() == 0` and the console
  shows the REPL's response or prompt, within a budget.
- `provision(rt, json) -> ProvisionOutcome`: from the stable first-run
  screen, type the traced `put` command with the byte length, send the
  JSON bytes, type `prov apply`, and run until the console contains
  `PROV OK` or `PROV FAIL` (returning which, plus the line).

Rungs: `boot_starts_the_console` (`hal_console: console started` within
a budget, and the blank-flash first-run hash unchanged), then criterion 2
and criterion 3. All use no identity and are committed pre-gate. Any
stall on the way is a Task D.

### Part 4 — Local exploration to the launcher (Task 4)

With the local fixtures, provision each role in a scratch run (examples
under `local/`, never committed), then explore from `PROV OK` to the
launcher: the registered My Badge screen, any onboarding or tutorial
("tutorial=reset"; system.cfg's `onboard_build`; the Aux1 slide the
strings say onboarding asks for), HOME, the launcher, and DOWN/RIGHT.
Frames and step counts go to `local/` and the ledger. Stalls become Task
D instances: their fixes commit with unit tests; the boot rungs that
prove them need an identity, so they are held locally and land at the
gate.

Two conditional tasks live here:

- **Reset that keeps flash** (trigger: `prov apply`, onboarding or the
  launcher restarts the chip). Model the reset where the hardware does it
  (the SYSTEM/RTC_CNTL software-reset register, or ROM
  `software_reset_cpu`, whichever the trace shows), re-running the
  shortcut boot while keeping `flash_chip`, as a real chip reset keeps
  its flash. Unit-tested; the existing blank-flash rungs must not change.
- **ST7789 `MADCTL`** (trigger: a frame failing the human comparison on
  orientation): M4 plan Task 6 as written.

### Part 5 — Gates (Tasks 5 and 6)

- **Task 5, human frame gate.** The orchestrator asks the human partner
  to compare the dumped registered screen, launcher and navigation frames
  (and any onboarding frames) with the physical badge. Needs no organizer
  permission: frames stay in `local/`.
- **Task 6, permission gate.** Once the human partner relays the
  organizers' OK: commit the fixtures, `provisions_<role>_through_the_console`
  for every role, `boots_to_launcher`, `launcher_responds_to_navigation`,
  the held Task D rungs and the WASM twin. Hashes are the ones measured in
  Task 4 and confirmed in Task 5.

### Part 6 — Frontend (Task 7)

- `emulator-wasm`: `serialInput(bytes: &[u8])`, a one-line passthrough.
- `frontend/src/runtime/firmware-runtime.ts`: `provision(identityJson:
  string)` types the same sequence as the Rust helper (a deliberate
  ~10-line duplicate; the wire protocol is the firmware's, not ours).
- `frontend/src/ui/shell.ts`: a role `<select>` and a "Provision test
  badge" button, visible only in firmware mode, enabled once the
  firmware runtime is running; it fetches
  `firmware/test-identities/<role>.json` and calls `provision`.
- `bun test` covers `provision()` against a fake emulator (bytes sent, in
  order) and the shell wiring.

Lands after the gate (it needs the fixtures).

### Part 7 — Docs (Task 8)

- `docs/milestone-5-decisions.md` (created in Task 1, grown per task),
  in the M4 format: decisions with "if wrong", remarks (finish-line
  tests and hashes), and the Milestone 6 backlog carrying forward the
  untouched M4 items.
- `docs/firmware-emulator-notes.md`: "Current state (end of Milestone 5)"
  (M4's moves under history), history entries per task, the
  USB-Serial-JTAG RX model, and a "Data-handling note" paragraph on the
  committed test identities (what they are, why they are safe, R-M5-2's
  permission and its date).
- `CLAUDE.md`: the state paragraph, the `peripherals/` entry
  (USB-Serial-JTAG RX), `runtime.rs` (`serial_input`), the frontend
  entries, the finish-line rung names in Commands.

### Part 8 — Stretch: a built-in app (Task 9)

From the stable launcher, navigate to Snake or Dice (fewest presses),
open it, pin its first stable frame and one input response (a direction
for Snake, a roll for Dice), plus a human comparison. Stalls are Task D
instances. Droppable without failing M5; if dropped, it heads the M6
backlog.

## Testing strategy

Native `cargo test -p emulator-core` is the primary loop; boot-ladder
rungs run with `--release` and reuse M4's `run_until_stable_frame`
(250,000-step samples, stable for 1,000,000 steps, uniform frames and
skip-listed hashes never count). Post-input rungs use a window measured
in Task 4 (M4 needed 8,000,000 after START). Register behavior is
unit-tested with byte-split word writes. Rungs assert reaching a line or
a stable hash within a budget, never exact step counts.

The per-role rungs each boot from scratch to the first-run screen (~16M
steps) and run in parallel. If the `--release` `boot_progress` suite
exceeds ~10 s, add M4 spec Part 4's shared-checkpoint fixture as its own
commit: `#[derive(Clone)]` on `FirmwareRuntime`, `Cpu`, `FirmwareBus` and
the peripherals; one `OnceLock<FirmwareRuntime>` booted to the stable
first-run screen, cloned per test; a test that a clone continues
identically to the original for 100,000 steps.

## Execution model

Work happens in the worktree `.claude/worktrees/milestone-5` on branch
`milestone-5` (the spec and plan are committed there first; based on
`main` @ `f7ee032` = `origin/main`), executed by an orchestrator using
`superpowers:subagent-driven-development`: one implementer subagent per
task, spec and code review after each, a task ledger in
`local/m5-sdd-ledger.md` (gitignored). Task D instances are created from
the M4 template as stalls appear. The human partner is pulled in at the
frame gate (Task 5), the permission gate (Task 6), the stop conditions
of R-M5-3, the fallback in Part 1 if the REPL is not running, and any
override of a recorded decision. Subagent briefs never contain values
from `local/` data; fixture paths are passed, not their contents.

# Milestone 4 — Real-firmware boot to the app launcher

Status: approved design (2026-10-06). Implementation plan:
`docs/superpowers/plans/2026-10-06-milestone4-app-launcher.md`.

## Goal

Take the real-firmware emulator from Milestone 3's end state (boot splash
drawn, then a silent stall in `load_partitions()` because the flash MMU is
not modeled) to the firmware's **app launcher**, rendered and responding
to buttons, natively and in the browser.

The first task is a real flash MMU model. Everything after it follows
Milestone 3's discovery loop (boot-probe, find the stall, fix the smallest
correct thing, add a ratchet rung), with the M3 backlog
(`docs/milestone-3-decisions.md`) as the seeded list of suspects.

### Success criteria

1. **MMU rung.** Real `load_partitions()` reads the synthesized partition
   table through the firmware's own `spi_flash_mmap` window and accepts
   its MD5 entry: boot reaches the first point past partition loading
   (a console line once console output after the scheduler works, or a
   no-`ESP_ERR_NOT_FOUND` / hot-PC-escape assertion before then).
2. **No splash regression.** `boots_to_first_real_frame` still passes
   after the MMU replaces Task D5's page mapping. Its hash may change only
   for a documented display-model reason (`MADCTL`), never as a side
   effect of the MMU.
3. **`boots_to_launcher`** (`emulator-core/tests/boot_progress.rs`): the
   framebuffer settles on a stable launcher frame, pinned by FNV-1a hash
   the same way as `boots_to_first_real_frame` (sampled in chunks, stable
   for 1,000,000 steps).
4. **`launcher_responds_to_buttons`**: from the stable launcher frame, a
   navigation press and release through `FirmwareRuntime::set_raw_button`
   (slot 4 = `DOWN` or slot 6 = `RIGHT`, whichever moves the launcher's
   selection; slot mapping in `frontend/src/runtime/firmware-runtime.ts`)
   changes the frame to a second pinned stable hash, with no fault or
   panic text.
5. **WASM twin** in `frontend/test/cpu-wasm.test.ts` reaches the same
   launcher hash through the bridge.
6. **Human comparison**: the human partner compares the dumped launcher
   frame (`boot-probe --dump-frame`, written under `local/`) with the
   physical badge's launcher and confirms a match. Recorded in the notes.
   This is the one manual gate.
7. All suites green: `cargo test --workspace`, `cargo test -p
   emulator-core --release --test boot_progress`, `bun run build:wasm`,
   `bun test`, `bun run typecheck`.

### Out of scope (Milestone 5+)

Running or playing a built-in app; real-time calibration
(`TICKS_PER_STEP`, `CPU_FREQ_MHZ`) unless a stall implicates it; the GDMA
RX in-link; edge-type interrupts; a cache model; frontend UI changes
(console pane, debugging UI); the small code duplications listed in the
M3 backlog.

## Constraints (carried from Milestone 3, unchanged)

- ESP-IDF **v5.5.3** headers/HAL/LL sources are the register authority.
  Every modeled register cites its source in the module doc. Fetch from
  `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`;
  never guess an offset.
- No PC-based interception of ESP-IDF (non-ROM) code. ROM stubs only, via
  `src/rom.rs` + `cpu/rom_stubs.rs`, addresses from the `rom*.ld` scripts.
- `FirmwareBus` dispatch stays an ordered sequence of concrete named
  fields; no `dyn` dispatch. `emulator-core` has no wasm-bindgen
  dependency; `emulator-wasm` stays logic-free.
- Unmapped data access never panics (reads 0, writes drop, logged);
  unmapped instruction fetch always traps. A browser-supplied image must
  never be able to panic the bus (checked arithmetic).
- The full dump and the real boot log stay in `local/`, are never
  committed or quoted (identity lines especially), and committed tests
  depend only on `factory.bin`. Python via `local/.venv` only; JS/TS via
  Bun only.
- Emulated flash is synthetic (blank + synthesized partition table +
  `factory.bin` at `0x10000`); writes stay in memory.
- The M3 design decisions in `docs/milestone-3-decisions.md` stand (e.g.
  interrupt threshold `>=`). Overriding one needs a recorded reason.

## Design

### Part 1 — Flash MMU (Task 1)

**Header facts** (v5.5.3; `soc/esp32c3/register/soc/reg_base.h`,
`soc/esp32c3/include/soc/ext_mem_defs.h`, `hal/esp32c3/include/hal/mmu_ll.h`):

- `DR_REG_MMU_TABLE = 0x600c5000`; `SOC_MMU_ENTRY_NUM = 128` 32-bit
  entries (`0x600c_5000..0x600c_5200`).
- Entry index = `(vaddr & SOC_MMU_VADDR_MASK) >> 16`,
  `SOC_MMU_VADDR_MASK = 0x7FFFFF` (`mmu_ll_get_entry_id`). The DBUS
  aperture (`0x3C00_0000..0x3C80_0000`) and IBUS aperture
  (`0x4200_0000..0x4280_0000`) **share** the 128 entries.
- Entry value: bits 0..7 physical page (`SOC_MMU_VALID_VAL_MASK = 0xff`),
  bit 8 `SOC_MMU_INVALID`; `SOC_MMU_VALID = SOC_MMU_ACCESS_FLASH = 0`.
  `mmu_ll_write_entry` stores `mmu_val | ACCESS_FLASH | VALID`; unmapping
  stores `SOC_MMU_INVALID`.
- Page size is fixed at 64 KiB (`mmu_ll_get_page_size`, already
  `mem::soc::MMU_PAGE_SIZE`).
- The bootloader (`bootloader_support/src/bootloader_utility.c`,
  `set_cache_and_start_app`, ~lines 1064–1105) calls
  `mmu_hal_unmap_all()`, then maps the DROM range
  (`drom_load_addr_aligned` → `drom_addr_aligned`, size rounded up to
  pages), then **one extra page**: `MMU_DROM_END_ENTRY_VADDR` (entry 127)
  → `drom_addr_aligned` ("for app to find the boot partition"), then the
  IROM range.

**Units**

- **`emulator-core/src/peripherals/mmu.rs`, `FlashMmu`**: `[u32; 128]`
  register storage for `0x600c_5000..0x600c_5200` with the usual
  byte-granular `read_byte`/`write_byte`, reset value `SOC_MMU_INVALID` in
  every entry (what `mmu_hal_unmap_all()` leaves). A pure
  `translate(vaddr) -> Option<u32>` returns the physical flash address
  `((entry & 0xff) << 16) | (vaddr & 0xffff)` for a valid entry, `None`
  for an invalid one. Bits other than 0..8 are stored and read back but
  ignored by translation. Offsets past the table inside its 4 KiB block go
  to the logged catch-all.
- **`mem::soc`**: `MMU_TABLE_RANGE`, the two aperture ranges (already
  present as XIP ranges), `MMU_ENTRY_NUM`, `MMU_INVALID`, `MMU_VADDR_MASK`
  constants with citations.
- **`FirmwareBus` tier 1 (XIP)** becomes: if the address is in the DBUS or
  IBUS aperture, `mmu.translate(addr)`; `Some(paddr)` →
  `flash_chip.read(paddr)`; `None` → data read returns 0 and is logged as
  unmapped, instruction fetch returns `None` (trap). Writes into the
  apertures stay dropped silently. `is_mapped` (fetchability) uses the
  same translation.
- **Removed**: `FirmwareBus::flash: Arc<[u8]>`, `XipRegion`,
  `xip_page_window` and its tests (their behavior is re-tested through the
  MMU). `EmulatedFlash` becomes the only flash store, so a SPIMEM1
  erase/program is immediately visible through XIP (no cache is modeled;
  the `Cache_*` ROM stubs stay no-ops, which is trivially coherent).
- **MMU seeding**: `FirmwareBus::from_segments` (which already receives
  the segment table, and is reached directly by browser-supplied images
  via `FirmwareRuntime::from_image`) programs the MMU exactly as
  `set_cache_and_start_app` would, through `FlashMmu`'s register writes:
  for each XIP segment, `paddr = APP_OFFSET (0x10000) + file_offset`;
  entries from `load_addr & !0xffff` for
  `ceil(((load_addr & 0xffff) + len) / 64 KiB)` pages, mapping to
  consecutive physical pages starting at `paddr & !0xffff`; plus entry 127
  → the first DROM segment's aligned physical page. Real ESP32-C3 apps
  have one DROM and one IROM segment (`factory.bin` does); if an image has
  more, each is mapped in segment order and the entry-127 page follows the
  first DROM segment. A segment whose `load_addr` and `paddr` disagree in
  their low 16 bits is **not mapped** (the real bootloader's image
  verification rejects such an image): its fetches trap loudly instead of
  executing the wrong bytes. All arithmetic is checked; overflow skips the
  segment.

**Decisions** (each recorded in `docs/milestone-4-decisions.md` with what
breaks if wrong):

- A physical address past the 4 MiB chip (pages 64..255) **wraps modulo
  the chip size**: the SPI flash chip ignores address bits above its
  capacity. This is a reconstruction, not a header fact.
  `EmulatedFlash::read` currently returns `0xFF` past the end; the MMU
  path masks the address before calling it.
- An access through an invalid entry raises a cache-error interrupt on
  real hardware; not modeled: reads 0 and is logged, fetch traps.
- The table is plain storage; the firmware's own `spi_flash_mmap` writes
  (entry 39 → `0x3c27_0000` in the observed stall) are what make dynamic
  windows work. No HLE of `spi_flash_mmap`.

**Tests (Task 1)**

- `mmu.rs` unit tests: reset state all invalid; byte-split word writes;
  `translate` valid/invalid; DBUS and IBUS addresses aliasing the same
  entry; page offset preserved; bits above 8 ignored; wrap past 4 MiB.
- Bus tests rewritten against MMU seeding: XIP reads hit the segment's
  bytes; the leading page bytes before `load_addr` read from flash (the
  D5 `cpu_start` header case, now read from the real chip bytes before the
  segment); entry 127 aliases the DROM's first page; an invalid entry's
  read returns 0 and is logged, its fetch returns `None`; a SPIMEM1
  program becomes visible through XIP; a misaligned segment stays
  unmapped; adversarial segment values do not panic.
- `boots_to_first_real_frame` passes unchanged (hash `0x5599c270ab0429fa`).
- New boot-ladder rung past `load_partitions()` (criterion 1). It
  replaces `rom_stub_boot.rs`'s pinned-stall test
  `boot_stubs_rom_md5init_then_load_partitions_reads_zeros_through_the_unmapped_flash_mmu_window`
  and turns `rom.rs`'s stand-in
  `rom_md5_stubs_accept_the_synthesized_partition_tables_md5_entry` into a
  unit test of the stubs only (its doc stops claiming it stands in for
  boot).
- Performance: record `boots_to_first_real_frame`'s `--release` wall time
  before and after; a regression above ~25% needs a fix (e.g. caching the
  last translated page) before Task 1 closes.

### Part 2 — The stall loop (Tasks 2..N)

Each stall after the MMU becomes a numbered "Task D" (the M3 template,
repeated in the plan): reproduce with `boot-probe`, identify the
registers / ROM call and their v5.5.3 source (disassemble `factory.bin`
if needed; never guess), failing test, minimal register-faithful fix,
ratchet rung, notes history entry, full suite, commit. Fixes land in the
order boot hits them.

**Seeded suspects, in the order to check them:**

1. **Console after the scheduler starts** (do right after the MMU even if
   it is not what stalls: every later diagnosis needs logs). Trace how
   `esp_log_write` output leaves the chip once the scheduler runs (newlib
   stdout → VFS → USB-Serial-JTAG driver), and model what it waits on
   (likely the USB-Serial-JTAG TX interrupt / `SERIAL_IN_EMPTY` status the
   emulator does not raise yet). Success: `load_partitions()`-era and
   later log lines reach the emulated console; earlier ladder rungs
   unchanged. Add console-line rungs for the newly visible boot lines.
2. **NVS on blank partitions.** `nvs_flash_init` on blank `nvs` will
   erase/program through the flash driver. SPIMEM1's dedicated
   `SPI_MEM_FLASH_*` command bits (`CMD` bits 19..31) must self-clear and
   perform their effect on `EmulatedFlash` (sector/block/chip erase, page
   program, read-status, write-enable), cited from `spi_mem_reg.h` /
   `spimem_flash_ll.h`. Also the likely path for the `storage` partition.
3. **ST7789 `MADCTL`** (`0x20`, then `0x60`): the launcher is unlikely to
   fit the default order by coincidence. Model `MV`/`MX`/`MY` in the
   address-window → framebuffer mapping in `spi.rs`, before the human
   comparison. This is expected to change the splash hash; update it in
   the same commit with the reason, after re-confirming the dumped splash
   still looks right.
4. **eFuse, `CPU_FREQ_MHZ`, timing, flagged ROM-stub guesses**: only if a
   stall implicates them.

Each suspect is still only a suspect: boot-probe decides the real order.
A stall outside the list is handled by the same template.

### Part 3 — Finish line (last task)

Find the smallest step count where the framebuffer shows a stable
launcher (the splash has been replaced and the frame holds for 1,000,000
steps); dump it; ask the human partner to compare it with the physical
badge; pin `boots_to_launcher` and `launcher_responds_to_buttons`; add
the WASM twin; update docs (Part 5).

If the launcher's appearance depends on NVS/storage contents (e.g. a
first-run screen on blank flash), the test pins what blank-flash boot
shows, and the human comparison is against the badge's behavior after a
factory reset if it differs; record which in the notes.

### Part 4 — Test hygiene (in scope only where M4 touches it)

- Rewrite step-exact pinned-stall tests when the stall they pin moves
  (they would fail anyway). Rungs assert reaching a line or a no-fault
  point, not exact step counts.
- Condense the task-history prose in `tests/boot_progress.rs` and
  `tests/rom_stub_boot.rs` when editing them; history belongs in the
  notes.
- If the `--release` `boot_progress` suite exceeds ~10 s, add a shared
  fixture that boots once to a checkpoint step and clones the
  `FirmwareRuntime` for each test that starts there. That needs
  `#[derive(Clone)]` on `FirmwareRuntime`, `Cpu`, `FirmwareBus` and the
  peripherals (none derive it today; the ROM stub table is deliberately
  plain data, `cpu/rom_stubs.rs`, so nothing blocks it). Not added
  otherwise.

### Part 5 — Docs

- `docs/milestone-4-decisions.md`, same format as M3's: decisions with
  "if wrong", remarks, and the Milestone 5 backlog (carrying forward the
  untouched M3 backlog items).
- `docs/firmware-emulator-notes.md`: "Current state (end of Milestone 4)",
  history entries per task, the MMU in the memory-map description, the
  D5 page mapping marked as superseded.
- `CLAUDE.md`: the state paragraph, the `mem/` and `peripherals/`
  architecture entries (MMU added, D5 window and the separate XIP flash
  buffer removed, the "flash MMU is not modeled" lines).
- Module docs: `bus.rs`'s tier list (tier 1 and the SPIMEM1 tier's "not
  unified yet" note), `rom.rs`'s "Where this gets boot to".

## Testing strategy

Native `cargo test -p emulator-core` is the primary loop; boot-ladder
rungs run with `--release`. After any change under `emulator-core/` or
`emulator-wasm/`: `bun run build:wasm`, then `bun test` and `bun run
typecheck`. Every task ends with the full suite green. Register-level
behavior is unit-tested with byte-split word writes (M3 review focus 1).

## Execution model

Work happens in the worktree `.claude/worktrees/milestone-4` (branch
`worktree-milestone-4`, based on `main` @ `c0e98d2`), executed by an
orchestrator using `superpowers:subagent-driven-development`: one
implementer subagent per task, spec + code review after each, the
orchestrator keeping a task ledger in `local/m4-sdd-ledger.md`
(gitignored). Stall tasks are created from the template as boot-probe
finds them. The human partner is pulled in for the launcher comparison
and for any decision that would override an M3 decision or this spec.

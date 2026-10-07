# Milestone 4 — Real-Firmware Boot to the App Launcher — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Boot `frontend/public/firmware/factory.bin` past Milestone 3's `load_partitions()` stall to the firmware's app launcher, rendered and responding to buttons, proven by native `cargo test` rungs, a WASM twin, and a human comparison with the physical badge.

**Architecture:** Task 1–2 add a real ESP32-C3 flash MMU (`peripherals/mmu.rs`) that every DBUS/IBUS access translates through, backed by the existing `EmulatedFlash`; it replaces Task D5's page-window XIP mapping and the separate XIP flash buffer, and `FirmwareBus::from_segments` seeds it the way the 2nd-stage bootloader does. Task 3 proves `load_partitions()` now succeeds. After that, boot is driven stall by stall (Milestone 3's Task D loop), with seeded tasks for the most likely stalls, ending in a finish-line task.

**Tech Stack:** Rust 2021 (`emulator-core`, `emulator-wasm` via wasm-bindgen/wasm-pack), Bun + TypeScript frontend, ESP-IDF v5.5.3 sources as the register authority.

**Spec:** `docs/superpowers/specs/2026-10-06-milestone4-app-launcher-design.md` — read it before starting any task. Also read `docs/milestone-3-decisions.md` (decisions that stand) and `docs/firmware-emulator-notes.md` "Known limitations".

## Handoff state (as of 2026-10-06)

- Worktree: `.claude/worktrees/milestone-4`, branch `worktree-milestone-4`, based on `main` @ `c0e98d2` (= `origin/main`). Spec committed at `252d669`.
- Baseline green in the worktree: `cargo test -p emulator-core --release` (394 unit + 55 integration/ladder tests), `bun run build:wasm`, `bun test` (100 pass), `bun run typecheck`.
- `boots_to_first_real_frame` takes ~0.5 s with `--release` on this machine (baseline for Task 2's performance check).
- `local/` (gitignored) has been copied into the worktree: `local/full_flash_dump.bin` (personal data; never commit), `local/boot_log.txt` (real serial log; never quote identity lines anywhere committed), `local/.venv/` (esptool; the only Python to use), `local/rom-elfs/` (ESP32-C3 ROM ELFs, for ROM addresses/disassembly), M3's frame PNGs and `m3-sdd-ledger.md`.
- Orchestrator ledger: keep `local/m4-sdd-ledger.md` (gitignored): one line per task/review/fix round, stall facts found, step counts.
- ESP-IDF headers are fetched from `https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>` (download into the session scratchpad, never into the repo).

## Global Constraints

- Register-faithful: every modeled register behavior cites its ESP-IDF v5.5.3 header / HAL / LL source in the module doc comment. Never guess an offset.
- No PC-based interception of ESP-IDF (non-ROM) code. ROM functions only, via `src/rom.rs` + `cpu/rom_stubs.rs`, addresses from `components/esp_rom/esp32c3/ld/esp32c3.rom*.ld`.
- `FirmwareBus` dispatch stays an ordered sequence of concrete named fields; no `dyn` peripheral dispatch. `emulator-core` has no wasm-bindgen dependency; `emulator-wasm` stays logic-free.
- Unmapped data access never panics (reads 0, writes drop, logged); unmapped instruction fetch always traps. A browser-supplied image must never panic the bus: checked arithmetic on every segment-derived value.
- The full dump and the real boot log are never committed, quoted in commits, or required by committed tests. Code touching the dump reads `BADGE_FULL_DUMP` and skips when unset.
- Emulated flash is synthetic (blank `0xFF` + synthesized partition table + `factory.bin` at `0x10000`); writes are in memory only.
- `docs/milestone-3-decisions.md` decisions stand (interrupt threshold `>=`, etc.); overriding one needs the human partner's OK and a recorded reason.
- Python via `local/.venv` only. JS/TS via Bun only.
- After any change under `emulator-core/` or `emulator-wasm/`: `bun run build:wasm`, then `bun test` and `bun run typecheck` must pass, in addition to `cargo test --workspace` and `cargo test -p emulator-core --release --test boot_progress`.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Byte-split register writes.** `FirmwareBus::write32` arrives as four ascending `write_byte`s; an MMU entry must hold the full word after the fourth byte and translate correctly only from then. Pinned by Task 1's `word_write_via_bytes_round_trips_and_translates`.
2. **Addresses inside `DROM_RANGE`/`IROM_RANGE` but outside the 8 MiB cache apertures** (`0x3C80_0000..0x3E00_0000`, `0x4280_0000..0x4400_0000`) must not alias MMU entries via the `0x7FFFFF` mask; they stay catch-all. Pinned by Task 2's `addresses_past_the_8mib_cache_apertures_do_not_alias_mmu_entries`.
3. **Malformed browser-supplied segment tables** (huge `len`, `file_offset` near `usize::MAX`, a segment straddling the aperture end, misaligned `load_addr` vs flash offset) must not panic and must not map garbage. Pinned by Task 2's `adversarial_xip_segments_do_not_panic_and_map_nothing_wrong`.
4. **Firmware remapping a bootloader-seeded entry** (e.g. `spi_flash_munmap` then a new `spi_flash_mmap`): XIP reads must follow the table's current value, not the boot-time mapping. Pinned by Task 2's `remapping_an_entry_redirects_reads_immediately`.
5. **Fetching through DBUS**: the data bus is not executable on the ESP32-C3; a jump into `0x3Cxx_xxxx` must trap, not execute rodata. Pinned by Task 2's `dbus_is_readable_but_never_fetchable`.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `emulator-core/src/peripherals/mmu.rs` | Create (Task 1) | `FlashMmu`: 128-entry MMU table register storage + `translate` |
| `emulator-core/src/peripherals/mod.rs` | Modify (Task 1) | `pub mod mmu;` |
| `emulator-core/src/mem/soc.rs` | Modify (Task 1) | MMU table range, cache aperture ranges, MMU constants |
| `emulator-core/src/mem/bus.rs` | Modify (Task 2) | Route MMU registers; XIP through `mmu` + `flash_chip`; bootloader-style MMU seeding; remove `XipRegion`/`xip_page_window`/`flash` field; rewrite XIP tests |
| `emulator-core/tests/boot_progress.rs` | Modify (Tasks 2–7) | New rungs; finish line |
| `emulator-core/tests/rom_stub_boot.rs` | Modify (Task 3) | Retire the MMU-stall phases of the pinned test |
| `emulator-core/src/rom.rs` | Modify (Task 3) | Module doc "Where this gets boot to"; stand-in MD5 test doc |
| `emulator-core/src/peripherals/usb_serial_jtag.rs` and/or others | Modify (Task 4) | Whatever the post-scheduler console path needs |
| `emulator-core/src/peripherals/flash.rs` | Modify (Task 5) | SPIMEM1 dedicated flash commands + user-command reads |
| `emulator-core/src/peripherals/spi.rs` | Modify (Task 6, conditional) | ST7789 `MADCTL` |
| `frontend/test/cpu-wasm.test.ts` | Modify (Task 7) | WASM launcher twin |
| `docs/milestone-4-decisions.md` | Create (Task 2), grow per task | M4 decisions + M5 backlog |
| `docs/firmware-emulator-notes.md`, `CLAUDE.md` | Modify (every task touches notes; Task 7 finalizes) | Current state, history, architecture |

---

### Task 1: `FlashMmu` peripheral and SoC constants

**Files:**
- Create: `emulator-core/src/peripherals/mmu.rs`
- Modify: `emulator-core/src/peripherals/mod.rs` (add `pub mod mmu;` in alphabetical order after `intc`)
- Modify: `emulator-core/src/mem/soc.rs` (constants after `MMU_PAGE_SIZE`; ranges after `GDMA_RANGE`; tests)

**Interfaces:**
- Consumes: `crate::peripherals::set_byte(word: &mut u32, idx: u32, val: u8)` (existing).
- Produces (used by Task 2):
  - `crate::mem::soc::{MMU_TABLE_RANGE: Range<u32>, DBUS_CACHE_RANGE: Range<u32>, IBUS_CACHE_RANGE: Range<u32>, MMU_ENTRY_NUM: usize, MMU_INVALID: u32, MMU_VALID_VAL_MASK: u32, MMU_VADDR_MASK: u32, MMU_DROM_END_ENTRY_ID: usize}`
  - `crate::peripherals::mmu::FlashMmu` with `new() -> Self`, `handles(offset: u32) -> bool`, `read_byte(&self, offset: u32) -> u8`, `write_byte(&mut self, offset: u32, val: u8)`, `entry(&self, id: usize) -> u32`, `translate(&self, vaddr: u32) -> Option<u32>`, `entry_id(vaddr: u32) -> usize`.

- [ ] **Step 1: Write the failing soc.rs test** (append inside `soc.rs`'s existing `mod tests`)

```rust
    #[test]
    fn mmu_constants_match_esp_idf_v5_5_3() {
        assert_eq!(MMU_TABLE_RANGE.start, 0x600c_5000); // DR_REG_MMU_TABLE
        assert_eq!(MMU_ENTRY_NUM, 128); // SOC_MMU_ENTRY_NUM
        assert_eq!(MMU_INVALID, 1 << 8); // SOC_MMU_INVALID
        assert_eq!(MMU_VALID_VAL_MASK, 0xff); // SOC_MMU_VALID_VAL_MASK
        assert_eq!(MMU_VADDR_MASK, 0x7f_ffff); // SOC_MMU_VADDR_MASK
        assert_eq!(MMU_DROM_END_ENTRY_ID, 127); // MMU_LL_END_DROM_ENTRY_ID
        assert_eq!(DBUS_CACHE_RANGE, 0x3c00_0000..0x3c80_0000); // SOC_DRAM0_CACHE_ADDRESS_LOW/HIGH
        assert_eq!(IBUS_CACHE_RANGE, 0x4200_0000..0x4280_0000); // SOC_IRAM0_CACHE_ADDRESS_LOW/HIGH
        // One entry per 64 KiB page of either aperture.
        assert_eq!(
            (DBUS_CACHE_RANGE.end - DBUS_CACHE_RANGE.start) / MMU_PAGE_SIZE,
            MMU_ENTRY_NUM as u32
        );
        // The cache apertures sit inside the coarse XIP ranges.
        assert!(DROM_RANGE.start <= DBUS_CACHE_RANGE.start && DBUS_CACHE_RANGE.end <= DROM_RANGE.end);
        assert!(IROM_RANGE.start <= IBUS_CACHE_RANGE.start && IBUS_CACHE_RANGE.end <= IROM_RANGE.end);
        // The MMU block does not overlap any other modeled peripheral.
        for r in [&SYSTEM_RANGE, &INTERRUPT_CORE0_RANGE, &GDMA_RANGE, &SPIMEM1_RANGE] {
            assert!(MMU_TABLE_RANGE.end <= r.start || r.end <= MMU_TABLE_RANGE.start);
        }
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p emulator-core --lib mem::soc::tests::mmu_constants_match_esp_idf_v5_5_3`
Expected: compile error, `MMU_TABLE_RANGE` not found.

- [ ] **Step 3: Add the constants** (in `soc.rs`, after `MMU_PAGE_SIZE`; ranges with the other peripheral ranges). Fetch the three headers below into the scratchpad first and confirm every value; the doc comments must cite them.

```rust
/// The flash MMU table: `DR_REG_MMU_TABLE = 0x600c5000`
/// (`components/soc/esp32c3/register/soc/reg_base.h`, ESP-IDF v5.5.3).
/// `SOC_MMU_ENTRY_NUM` (128) 32-bit entries occupy the first `0x200`
/// bytes; the rest of this 4 KiB block is not modeled (catch-all).
pub const MMU_TABLE_RANGE: Range<u32> = 0x600c_5000..0x600c_6000;

/// The data-bus flash-cache aperture the MMU translates:
/// `SOC_DRAM0_CACHE_ADDRESS_LOW..HIGH`
/// (`components/soc/esp32c3/include/soc/ext_mem_defs.h`). Narrower than
/// [`DROM_RANGE`]: addresses in `DROM_RANGE` past this end are not
/// cache-backed at all.
pub const DBUS_CACHE_RANGE: Range<u32> = 0x3C00_0000..0x3C80_0000;

/// The instruction-bus flash-cache aperture:
/// `SOC_IRAM0_CACHE_ADDRESS_LOW..HIGH` (same header). Shares the 128 MMU
/// entries with [`DBUS_CACHE_RANGE`].
pub const IBUS_CACHE_RANGE: Range<u32> = 0x4200_0000..0x4280_0000;

/// `SOC_MMU_ENTRY_NUM` (`ext_mem_defs.h`).
pub const MMU_ENTRY_NUM: usize = 128;
/// `SOC_MMU_INVALID` (`ext_mem_defs.h`): bit 8 set = entry unmapped.
pub const MMU_INVALID: u32 = 1 << 8;
/// `SOC_MMU_VALID_VAL_MASK` (`ext_mem_defs.h`): the physical page number.
pub const MMU_VALID_VAL_MASK: u32 = 0xff;
/// `SOC_MMU_VADDR_MASK` (`ext_mem_defs.h`); `mmu_ll_get_entry_id()`
/// (`hal/esp32c3/include/hal/mmu_ll.h`) is `(vaddr & SOC_MMU_VADDR_MASK) >> 16`.
pub const MMU_VADDR_MASK: u32 = 0x7F_FFFF;
/// `MMU_LL_END_DROM_ENTRY_ID = SOC_MMU_ENTRY_NUM - 1` (`mmu_ll.h`): the
/// entry the bootloader maps to the DROM's first page "for app to find the
/// boot partition" (`bootloader_support/src/bootloader_utility.c`,
/// `set_cache_and_start_app`).
pub const MMU_DROM_END_ENTRY_ID: usize = MMU_ENTRY_NUM - 1;
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p emulator-core --lib mem::soc`
Expected: PASS.

- [ ] **Step 5: Write the failing `mmu.rs` tests.** Create `emulator-core/src/peripherals/mmu.rs` containing only the module doc (Step 7's text), `use` lines and this test module, plus `pub mod mmu;` in `peripherals/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(m: &mut FlashMmu, off: u32, v: u32) {
        for (i, b) in v.to_le_bytes().iter().enumerate() {
            m.write_byte(off + i as u32, *b);
        }
    }

    fn read_word(m: &FlashMmu, off: u32) -> u32 {
        u32::from_le_bytes([0, 1, 2, 3].map(|i| m.read_byte(off + i)))
    }

    #[test]
    fn reset_state_is_every_entry_invalid() {
        let m = FlashMmu::new();
        for id in 0..MMU_ENTRY_NUM {
            assert_eq!(m.entry(id), MMU_INVALID, "entry {id}");
        }
        assert_eq!(m.translate(0x3c00_0000), None);
        assert_eq!(m.translate(0x4200_0000), None);
    }

    #[test]
    fn word_write_via_bytes_round_trips_and_translates() {
        let mut m = FlashMmu::new();
        // Entry 39 -> physical page 0 (spi_flash_mmap's window in the M3 stall).
        write_word(&mut m, 39 * 4, 0);
        assert_eq!(read_word(&m, 39 * 4), 0);
        assert_eq!(m.translate(0x3c27_8000), Some(0x0000_8000));
        // Entry 19 -> page 1 (factory.bin's DROM: vaddr 0x3c130020, paddr 0x10020).
        write_word(&mut m, 19 * 4, 1);
        assert_eq!(m.translate(0x3c13_0020), Some(0x0001_0020));
    }

    #[test]
    fn invalid_bit_unmaps_and_other_high_bits_are_stored_but_ignored() {
        let mut m = FlashMmu::new();
        write_word(&mut m, 0, 0x0000_0105); // INVALID | page 5
        assert_eq!(m.translate(0x4200_0000), None);
        write_word(&mut m, 0, 0xFFFF_FE05); // bit 8 clear, junk above it
        assert_eq!(read_word(&m, 0), 0xFFFF_FE05, "plain storage");
        assert_eq!(m.translate(0x4200_1234), Some(0x0005_1234));
    }

    #[test]
    fn dbus_and_ibus_share_entries() {
        let mut m = FlashMmu::new();
        write_word(&mut m, 2 * 4, 0x15);
        assert_eq!(FlashMmu::entry_id(0x3c02_0000), 2);
        assert_eq!(FlashMmu::entry_id(0x4202_0000), 2);
        assert_eq!(m.translate(0x3c02_abcd), Some(0x0015_abcd));
        assert_eq!(m.translate(0x4202_abcd), Some(0x0015_abcd));
    }

    #[test]
    fn page_numbers_up_to_255_translate_unwrapped() {
        // Wrapping to the chip size is the bus's job (it knows the chip);
        // the MMU reports the raw 8-bit page.
        let mut m = FlashMmu::new();
        write_word(&mut m, 127 * 4, 0xff);
        assert_eq!(m.translate(0x3c7f_0010), Some(0x00ff_0010));
    }

    #[test]
    fn offsets_past_the_table_are_not_handled() {
        assert!(FlashMmu::handles(0));
        assert!(FlashMmu::handles(0x1ff));
        assert!(!FlashMmu::handles(0x200));
        let mut m = FlashMmu::new();
        m.write_byte(0x200, 0xaa); // dropped, must not panic
        assert_eq!(m.read_byte(0x200), 0);
    }
}
```

- [ ] **Step 6: Run to verify they fail**

Run: `cargo test -p emulator-core --lib peripherals::mmu`
Expected: compile error, `FlashMmu` not found.

- [ ] **Step 7: Implement `FlashMmu`** (above the test module):

```rust
//! The ESP32-C3 flash MMU table (`DR_REG_MMU_TABLE = 0x600c_5000`).
//!
//! Sources (ESP-IDF v5.5.3): `components/soc/esp32c3/register/soc/reg_base.h`
//! (base), `components/soc/esp32c3/include/soc/ext_mem_defs.h`
//! (`SOC_MMU_ENTRY_NUM`, `SOC_MMU_INVALID`, `SOC_MMU_VALID_VAL_MASK`,
//! `SOC_MMU_VADDR_MASK`), `components/hal/esp32c3/include/hal/mmu_ll.h`
//! (`mmu_ll_get_entry_id`, `mmu_ll_write_entry`, `mmu_ll_set_entry_invalid`,
//! `mmu_ll_entry_id_to_paddr_base`).
//!
//! 128 32-bit entries, one per 64 KiB virtual page, **shared** by the
//! data-bus (`0x3C00_0000..0x3C80_0000`) and instruction-bus
//! (`0x4200_0000..0x4280_0000`) apertures: entry id =
//! `(vaddr & 0x7FFFFF) >> 16`. Bits 0..7 hold the physical flash page,
//! bit 8 set marks the entry invalid. Entries are plain storage (every bit
//! reads back); translation looks only at bits 0..8.
//!
//! Reset value: every entry invalid, which is what the bootloader's
//! `mmu_hal_unmap_all()` leaves before it maps the app
//! (`crate::mem::bus::FirmwareBus::from_segments` then replays those
//! mappings). No cache is modeled: a table write takes effect on the next
//! access, which is what the firmware's cache invalidation after every
//! table change guarantees on real hardware.

use crate::mem::soc::{MMU_ENTRY_NUM, MMU_INVALID, MMU_PAGE_SIZE, MMU_VADDR_MASK, MMU_VALID_VAL_MASK};

use super::set_byte;

/// Bytes of register space the table occupies.
const TABLE_BYTES: u32 = 4 * MMU_ENTRY_NUM as u32;

#[derive(Clone)]
pub struct FlashMmu {
    entries: [u32; MMU_ENTRY_NUM],
}

impl Default for FlashMmu {
    fn default() -> Self {
        Self::new()
    }
}

impl FlashMmu {
    pub fn new() -> Self {
        Self {
            entries: [MMU_INVALID; MMU_ENTRY_NUM],
        }
    }

    /// `true` iff `offset` (from `DR_REG_MMU_TABLE`) is inside the table.
    pub fn handles(offset: u32) -> bool {
        offset < TABLE_BYTES
    }

    pub fn read_byte(&self, offset: u32) -> u8 {
        if !Self::handles(offset) {
            return 0;
        }
        self.entries[(offset / 4) as usize].to_le_bytes()[(offset % 4) as usize]
    }

    pub fn write_byte(&mut self, offset: u32, val: u8) {
        if !Self::handles(offset) {
            return;
        }
        set_byte(&mut self.entries[(offset / 4) as usize], offset % 4, val);
    }

    /// The raw value of entry `id` (panics if `id >= 128`; callers index
    /// with [`FlashMmu::entry_id`] or a constant).
    pub fn entry(&self, id: usize) -> u32 {
        self.entries[id]
    }

    /// `mmu_ll_get_entry_id`: the entry a cache-aperture address uses.
    pub fn entry_id(vaddr: u32) -> usize {
        ((vaddr & MMU_VADDR_MASK) / MMU_PAGE_SIZE) as usize
    }

    /// The physical flash address `vaddr` maps to, or `None` if its entry
    /// is invalid. The caller must already know `vaddr` is inside one of
    /// the two cache apertures (this does not check).
    pub fn translate(&self, vaddr: u32) -> Option<u32> {
        let e = self.entries[Self::entry_id(vaddr)];
        if e & MMU_INVALID != 0 {
            return None;
        }
        Some((e & MMU_VALID_VAL_MASK) * MMU_PAGE_SIZE + (vaddr % MMU_PAGE_SIZE))
    }
}
```

- [ ] **Step 8: Run to verify they pass**

Run: `cargo test -p emulator-core --lib peripherals::mmu && cargo test -p emulator-core --lib mem::soc`
Expected: PASS (6 + soc tests). `cargo clippy -p emulator-core --all-targets` clean of new warnings.

- [ ] **Step 9: Commit**

```bash
git add emulator-core/src/peripherals/mmu.rs emulator-core/src/peripherals/mod.rs emulator-core/src/mem/soc.rs
git commit -m "feat(emulator-core): ESP32-C3 flash MMU table model

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: XIP through the MMU; remove the D5 page windows

**Files:**
- Modify: `emulator-core/src/mem/bus.rs` (struct fields ~L287–360, `xip_page_window` ~L354–455 deleted, `from_segments` ~L457–570, `read_byte` ~L683, `write_byte` ~L775, `is_mapped` ~L1061, module doc tier 1 and tier 12, tests from ~L1110)
- Modify: `emulator-core/src/boot.rs` (doc references to D5 / `xip_page_window` only, if any)
- Modify: `emulator-core/tests/boot_progress.rs` (no new rung; just confirm)
- Create: `docs/milestone-4-decisions.md`
- Modify: `docs/firmware-emulator-notes.md` (memory-map text that describes D5 windows; history entry)

**Interfaces:**
- Consumes: Task 1's `FlashMmu` and `soc` constants; `crate::peripherals::flash::{EmulatedFlash, APP_OFFSET, FLASH_SIZE}`.
- Produces: `pub mmu: FlashMmu` field on `FirmwareBus` (tests and later tasks read entries via `bus.mmu.entry(id)`). `FirmwareBus::from_segments(flash: Arc<[u8]>, segments: &[SegmentDescriptor]) -> Self` keeps its signature (it still needs the image bytes for RAM segments and `EmulatedFlash::from_app_image`), but no longer stores `flash`.

**Behavior to implement (from the spec, Part 1):**
- XIP read: `DBUS_CACHE_RANGE` or `IBUS_CACHE_RANGE` → `mmu.translate(addr)`; `Some(p)` → `flash_chip.read(p % FLASH_SIZE as u32)`; `None` → log unmapped read, return 0. Checked **first** in `read_byte`, before RAM regions (as the XIP tier is today).
- XIP write (either aperture): dropped silently, as today.
- MMU registers: route `MMU_TABLE_RANGE` like the other peripherals; offsets `FlashMmu::handles` rejects are logged as unmapped.
- Fetchability (`is_mapped`): `IBUS_CACHE_RANGE.contains(&addr) && mmu.translate(addr).is_some()`, or RAM, or ROM code. DBUS is never fetchable.
- Addresses in `DROM_RANGE`/`IROM_RANGE` outside the two cache apertures fall through to the catch-all.
- Seeding in `from_segments`, after building RAM regions and `flash_chip`: replay `set_cache_and_start_app` (v5.5.3 `bootloader_support/src/bootloader_utility.c` ~L1064–1105, `hal/mmu_hal.c` `mmu_hal_map_region`). For each segment with `is_xip_addr(load_addr)`, in segment order: compute `paddr = APP_OFFSET + file_offset` (checked); skip unless `load_addr` is inside `DBUS_CACHE_RANGE` or `IBUS_CACHE_RANGE`, `paddr % 64K == load_addr % 64K`, and `load_addr + len` (checked) ≤ the aperture end; map pages `load_addr/64K ..< ceil((load_addr+len)/64K)` to physical pages starting at `paddr/64K`, by writing `phys_page` (bits 0..7, `VALID = ACCESS_FLASH = 0`) to `MMU_TABLE_RANGE.start + 4*id` **through `write32` on the bus**. After the first DBUS segment is mapped, also write entry `MMU_DROM_END_ENTRY_ID` → that segment's first physical page.
- Delete `XipRegion`, `xip_regions`, `xip_page_window`, the `flash` field and their now-dead imports (`MMU_PAGE_SIZE` stays used by seeding).

- [ ] **Step 1: Rewrite the `bus.rs` test helper so XIP segments are flash-aligned** (replace `bus_with` in `mod tests`):

```rust
    /// Builds a bus from `(load_addr, bytes)` segments laid out the way an
    /// esptool image is: each XIP segment's bytes start at a file offset
    /// with the same low 16 bits as its `load_addr` (padding with `0xFF`),
    /// so `APP_OFFSET + file_offset` and `load_addr` agree mod 64 KiB and
    /// the bootloader-style MMU seeding maps it.
    fn bus_with(segments: Vec<(u32, Vec<u8>)>) -> FirmwareBus {
        let mut flash = Vec::new();
        let mut descriptors = Vec::new();
        for (load_addr, data) in &segments {
            if is_xip_addr(*load_addr) {
                let want = (*load_addr % MMU_PAGE_SIZE) as usize;
                while flash.len() % MMU_PAGE_SIZE as usize != want {
                    flash.push(0xFF);
                }
            }
            let file_offset = flash.len();
            flash.extend_from_slice(data);
            descriptors.push(SegmentDescriptor {
                load_addr: *load_addr,
                file_offset,
                len: data.len(),
            });
        }
        FirmwareBus::from_segments(Arc::from(flash.into_boxed_slice()), &descriptors)
    }
```

- [ ] **Step 2: Replace the D5 XIP tests with MMU-backed tests.** Delete `xip_regions_expose_the_full_containing_64kib_page_not_just_the_declared_segment` and `xip_page_window_clamps_gracefully_when_no_earlier_flash_bytes_exist`. Keep `xip_region_reads_from_flash_bytes_and_drops_writes` and `irom_range_is_also_xip` as they are (they now go through the MMU). Add:

```rust
    #[test]
    fn seeding_maps_each_xip_segment_like_the_bootloader() {
        // factory.bin's shape: DROM at 0x3c13_0020, IROM at 0x4200_0020.
        let drom = vec![0xA1; 0x30];
        let irom = vec![0xB2; 0x30];
        let bus = bus_with(vec![(0x3c13_0020, drom), (0x4200_0020, irom)]);
        // DROM: file offset 0x20 -> paddr 0x10020 -> page 1 at entry 19.
        assert_eq!(bus.mmu.entry(19), 1);
        // IROM: padded to file offset 0x10020 -> paddr 0x20020 -> page 2 at entry 0.
        assert_eq!(bus.mmu.entry(0), 2);
        // Entry 127 -> the DROM's first physical page.
        assert_eq!(bus.mmu.entry(MMU_DROM_END_ENTRY_ID), 1);
        // Everything else stays invalid.
        assert_eq!(bus.mmu.entry(1), MMU_INVALID);
        assert_eq!(bus.mmu.entry(20), MMU_INVALID);
    }

    #[test]
    fn leading_page_bytes_before_load_addr_read_from_the_chip() {
        // Task D5's cpu_start case: the image header sits 0x20 bytes before
        // the DROM load_addr, in the same page. Through the MMU those bytes
        // are simply the chip's bytes at APP_OFFSET.
        let mut bus = bus_with(vec![(0x3c13_0020, vec![0x5A; 4])]);
        // bus_with pads the first 0x20 bytes of the image with 0xFF.
        assert_eq!(bus.read8(0x3c13_0000), 0xFF);
        assert_eq!(bus.read8(0x3c13_0020), 0x5A);
        // Past the image: blank flash, not catch-all 0.
        assert_eq!(bus.read8(0x3c13_ff00), 0xFF);
        // The entry-127 alias reads the same page.
        assert_eq!(bus.read8(0x3c7f_0020), 0x5A);
    }

    #[test]
    fn invalid_entry_reads_zero_logged_and_fetch_traps() {
        let mut bus = bus_with(vec![(0x4200_0000, vec![0x13, 0x00, 0x00, 0x00])]);
        assert_eq!(bus.read32(0x4210_0000), 0, "entry 16 is invalid");
        assert!(bus.unmapped_log().iter().any(|a| a.addr == 0x4210_0000 && !a.is_write));
        assert_eq!(bus.fetch16(0x4210_0000), None);
        assert_eq!(bus.fetch16(0x4200_0000), Some(0x0013));
    }

    #[test]
    fn dbus_is_readable_but_never_fetchable() {
        let mut bus = bus_with(vec![(0x3c00_0000, vec![0x13, 0x00, 0x00, 0x00])]);
        assert_eq!(bus.read16(0x3c00_0000), 0x0013);
        assert_eq!(bus.fetch16(0x3c00_0000), None);
    }

    #[test]
    fn mmu_table_writes_through_the_bus_take_effect() {
        let mut bus = bus_with(vec![]);
        // Map entry 39 to physical page 0 (spi_flash_mmap's window in M3's stall):
        // the synthesized partition table at 0x8000 becomes visible at 0x3c27_8000.
        bus.write32(MMU_TABLE_RANGE.start + 39 * 4, 0);
        assert_eq!(bus.read16(0x3c27_8000), 0x50AA, "ESP_PARTITION_MAGIC, little-endian");
        assert_eq!(bus.read32(MMU_TABLE_RANGE.start + 39 * 4), 0, "reads back");
        // Past the table, inside the block: logged catch-all.
        bus.write32(MMU_TABLE_RANGE.start + 0x200, 1);
        assert!(bus.unmapped_log().iter().any(|a| a.addr == MMU_TABLE_RANGE.start + 0x200));
    }

    #[test]
    fn remapping_an_entry_redirects_reads_immediately() {
        let mut bus = bus_with(vec![(0x3c00_0000, vec![0x11; 4])]);
        assert_eq!(bus.read8(0x3c00_0000), 0x11);
        bus.write32(MMU_TABLE_RANGE.start, 0); // entry 0 -> page 0 (partition-table page)
        assert_eq!(bus.read16(0x3c00_8000), 0x50AA);
        bus.write32(MMU_TABLE_RANGE.start, MMU_INVALID);
        assert_eq!(bus.read8(0x3c00_0000), 0, "unmapped now");
    }

    #[test]
    fn spimem1_flash_program_is_visible_through_xip() {
        let mut bus = bus_with(vec![]);
        bus.write32(MMU_TABLE_RANGE.start + 43 * 4, 0x2b); // entry 43 -> page 0x2b (storage partition start 0x2b0000)
        assert_eq!(bus.read8(0x3c2b_0000), 0xFF, "blank");
        bus.flash_chip.program(0x2b_0000, &[0x00]);
        assert_eq!(bus.read8(0x3c2b_0000), 0x00);
    }

    #[test]
    fn physical_pages_past_the_chip_wrap() {
        let mut bus = bus_with(vec![]);
        // Page 0x40 is 4 MiB: wraps to page 0 on a 4 MiB chip.
        bus.write32(MMU_TABLE_RANGE.start + 5 * 4, 0x40);
        assert_eq!(bus.read16(0x3c05_8000), 0x50AA);
    }

    #[test]
    fn addresses_past_the_8mib_cache_apertures_do_not_alias_mmu_entries() {
        let mut bus = bus_with(vec![(0x3c00_0000, vec![0x77; 4]), (0x4200_0000, vec![0x13, 0, 0, 0])]);
        assert_eq!(bus.read8(0x3c80_0000), 0, "0x3c80_0000 & 0x7fffff would alias entry 0");
        assert!(bus.unmapped_log().iter().any(|a| a.addr == 0x3c80_0000));
        assert_eq!(bus.fetch16(0x4280_0000), None);
    }

    #[test]
    fn misaligned_xip_segment_is_left_unmapped() {
        // load_addr low bits 0x20, but file offset 0 -> paddr 0x10000 (low bits 0).
        let flash: Arc<[u8]> = Arc::from(vec![0x13u8, 0, 0, 0].into_boxed_slice());
        let seg = SegmentDescriptor { load_addr: 0x4200_0020, file_offset: 0, len: 4 };
        let mut bus = FirmwareBus::from_segments(flash, &[seg]);
        assert_eq!(bus.mmu.entry(0), MMU_INVALID);
        assert_eq!(bus.fetch16(0x4200_0020), None, "traps loudly, never runs wrong bytes");
    }

    #[test]
    fn adversarial_xip_segments_do_not_panic_and_map_nothing_wrong() {
        let flash: Arc<[u8]> = Arc::from(vec![0u8; 16].into_boxed_slice());
        let cases = [
            SegmentDescriptor { load_addr: 0x3c00_0000, file_offset: usize::MAX, len: 4 },
            SegmentDescriptor { load_addr: 0x3c00_0000, file_offset: 0, len: usize::MAX },
            // Straddles the end of the DBUS aperture.
            SegmentDescriptor { load_addr: 0x3c7f_0000, file_offset: 0, len: 0x2_0000 },
            // In DROM_RANGE but past the cache aperture.
            SegmentDescriptor { load_addr: 0x3d00_0000, file_offset: 0, len: 4 },
            SegmentDescriptor { load_addr: 0xffff_fff0, file_offset: 0, len: 0x100 },
        ];
        for seg in cases {
            let bus = FirmwareBus::from_segments(flash.clone(), &[seg]);
            for id in 0..MMU_ENTRY_NUM {
                assert_eq!(bus.mmu.entry(id), MMU_INVALID, "{seg:?} entry {id}");
            }
        }
    }
```

  (If `SegmentDescriptor` does not derive `Debug`, drop the `{seg:?}` from the message rather than adding the derive.)

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p emulator-core --lib mem::bus`
Expected: compile errors (`mmu` field, `MMU_TABLE_RANGE`/`MMU_DROM_END_ENTRY_ID`/`MMU_INVALID`/`MMU_ENTRY_NUM` not imported in `bus.rs`).

- [ ] **Step 4: Implement the bus changes.** Concretely:

```rust
// imports: add DBUS_CACHE_RANGE, IBUS_CACHE_RANGE, MMU_DROM_END_ENTRY_ID,
// MMU_TABLE_RANGE (and in tests MMU_ENTRY_NUM, MMU_INVALID) from super::soc;
// crate::peripherals::mmu::FlashMmu; crate::peripherals::flash::{APP_OFFSET, FLASH_SIZE}.

// struct FirmwareBus: delete `flash` and `xip_regions`; add
    /// The flash MMU table (`crate::peripherals::mmu`): every DBUS/IBUS
    /// access translates through it to [`FirmwareBus::flash_chip`].
    pub mmu: FlashMmu,

// from_segments: build ram_regions exactly as today (drop the XIP branch's
// push; XIP segments are handled by seeding), then:
        let flash_chip = EmulatedFlash::from_app_image(&flash);
        let mut bus = Self { /* ..., */ mmu: FlashMmu::new(), flash_chip, /* ... */ };
        bus.seed_mmu_like_bootloader(segments);
        bus

impl FirmwareBus {
    /// Replays the 2nd-stage bootloader's `set_cache_and_start_app()` MMU
    /// programming (ESP-IDF v5.5.3 `bootloader_support/src/bootloader_utility.c`;
    /// page rounding from `hal/mmu_hal.c`'s `mmu_hal_map_region`) for the
    /// app at [`APP_OFFSET`]. See the module doc's tier 1.
    fn seed_mmu_like_bootloader(&mut self, segments: &[SegmentDescriptor]) {
        let mut drom_end_mapped = false;
        for seg in segments.iter().filter(|s| is_xip_addr(s.load_addr)) {
            let Some(first_phys) = self.map_xip_segment(seg) else { continue };
            if DBUS_CACHE_RANGE.contains(&seg.load_addr) && !drom_end_mapped {
                self.write_mmu_entry(MMU_DROM_END_ENTRY_ID, first_phys);
                drom_end_mapped = true;
            }
        }
    }

    /// Maps one XIP segment's pages; returns its first physical page, or
    /// `None` (nothing written) if the segment cannot be mapped faithfully.
    fn map_xip_segment(&mut self, seg: &SegmentDescriptor) -> Option<u32> {
        let aperture = [&DBUS_CACHE_RANGE, &IBUS_CACHE_RANGE]
            .into_iter()
            .find(|r| r.contains(&seg.load_addr))?;
        let paddr = APP_OFFSET.checked_add(u32::try_from(seg.file_offset).ok()?)?;
        if paddr % MMU_PAGE_SIZE != seg.load_addr % MMU_PAGE_SIZE {
            return None;
        }
        let end = seg.load_addr.checked_add(u32::try_from(seg.len).ok()?)?;
        if end > aperture.end {
            return None;
        }
        let first_page = seg.load_addr / MMU_PAGE_SIZE;
        let page_count = end.div_ceil(MMU_PAGE_SIZE) - first_page;
        let first_phys = paddr / MMU_PAGE_SIZE;
        let first_id = FlashMmu::entry_id(seg.load_addr);
        for i in 0..page_count {
            self.write_mmu_entry(first_id + i as usize, first_phys + i);
        }
        Some(first_phys)
    }

    /// `mmu_ll_write_entry`: `phys_page | ACCESS_FLASH (0) | VALID (0)`,
    /// written through the bus like the bootloader's store.
    fn write_mmu_entry(&mut self, id: usize, phys_page: u32) {
        let addr = MMU_TABLE_RANGE.start + 4 * id as u32;
        Bus::write32(self, addr, phys_page & MMU_VALID_VAL_MASK);
    }

    fn in_cache_aperture(addr: u32) -> bool {
        DBUS_CACHE_RANGE.contains(&addr) || IBUS_CACHE_RANGE.contains(&addr)
    }
}

// read_byte, first check (replaces the xip_regions lookup):
        if Self::in_cache_aperture(addr) {
            if let Some(paddr) = self.mmu.translate(addr) {
                return self.flash_chip.read(paddr % FLASH_SIZE as u32);
            }
            self.record_unmapped(addr, false);
            return 0;
        }
// read_byte, with the other peripherals (e.g. just before GDMA):
        if MMU_TABLE_RANGE.contains(&addr) {
            let offset = addr - MMU_TABLE_RANGE.start;
            if !FlashMmu::handles(offset) {
                self.record_unmapped(addr, false);
            }
            return self.mmu.read_byte(offset);
        }

// write_byte, first check:
        if Self::in_cache_aperture(addr) {
            // Flash is read-only through the cache; drop silently.
            return;
        }
// write_byte, peripheral routing:
        if MMU_TABLE_RANGE.contains(&addr) {
            let offset = addr - MMU_TABLE_RANGE.start;
            if !FlashMmu::handles(offset) {
                self.record_unmapped(addr, true);
            }
            self.mmu.write_byte(offset, val);
            return;
        }

// is_mapped:
        (IBUS_CACHE_RANGE.contains(&addr) && self.mmu.translate(addr).is_some())
            || self.ram_regions.iter().any(|r| r.contains(addr))
            || self.rom_code.iter().any(|b| b.contains(addr))
```

  The `page_count` arithmetic cannot underflow (`end >= load_addr`), and `first_id + page_count <= 128` because `end <= aperture.end`. Update the module doc's tier 1 (XIP through the MMU; invalid → logged 0 / fetch trap; DBUS not fetchable; apertures narrower than `DROM_RANGE`/`IROM_RANGE`), add an MMU tier to the numbered list, and remove tier 12's "`flash_chip` and the XIP tier's `flash` buffer are separate for now" paragraph. Rewrite `from_segments`'s doc: drop the D5 widening explanation, keep the `cpu_start` header-read fact as the reason the leading page bytes matter, and describe the seeding. `flash_chip`'s field doc: "the only flash store; XIP reads it through `mmu`".

- [ ] **Step 5: Run unit and integration tests**

Run: `cargo test -p emulator-core --lib && cargo test -p emulator-core --release --tests`
Expected: all PASS. In particular `boots_to_first_real_frame` passes with hash `0x5599c270ab0429fa` unchanged. **If any boot rung or the hash changes, stop**: the MMU changed pre-splash behavior. Find out why with `boot-probe` (likely an MMU-entry read the firmware now sees as invalid/valid differently from the old 0) before touching any expectation, and report it to the orchestrator.

- [ ] **Step 6: Performance check**

Run: `time cargo test -q -p emulator-core --release --test boot_progress boots_to_first_real_frame` (three times, take the median).
Expected: within ~25% of the 0.5 s baseline. If slower, add a one-entry translation cache (last 64 KiB page → physical base, invalidated on any `MMU_TABLE_RANGE` write) with a unit test that a table write invalidates it; re-measure.

- [ ] **Step 7: Docs**
  - Create `docs/milestone-4-decisions.md` in the format of `docs/milestone-3-decisions.md` (title, intro paragraph naming the spec/plan and "where they disagree, this file and the code win"), with a "Flash MMU" section recording: MMU replaces D5 (what breaks if wrong: any segment whose flash offset and vaddr disagree mod 64 KiB is now unmapped and traps); physical pages past 4 MiB wrap (reconstruction; if wrong, a firmware map past the chip would read real data instead of blank); invalid-entry access reads 0 + logged / fetch traps, no cache-error interrupt; DBUS not fetchable; no cache model (table writes are immediately coherent); entry 127 seeded (history item 7 resolved). Add an empty "Milestone 5 backlog" heading for later tasks to fill.
  - `docs/firmware-emulator-notes.md`: update the "Boot strategy" / memory-map prose that describes D5 page windows (search `xip_page_window`, `Task D5`) to say the MMU now does this; mark history item 7 resolved; add a history entry "Milestone 4 Task 2: flash MMU" (what changed, hash unchanged, perf numbers).

- [ ] **Step 8: Full suite and commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck`
Expected: all green.

```bash
git add -A emulator-core docs
git commit -m "feat(emulator-core): serve XIP through the flash MMU, replacing D5 page windows

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `load_partitions()` succeeds — first M4 rung; retire the MMU-stall test

**Files:**
- Modify: `emulator-core/tests/boot_progress.rs` (new rung after `boot_no_longer_faults_at_the_pre_task_d13_md5init_call_site`)
- Modify: `emulator-core/tests/rom_stub_boot.rs:425-736` (the pinned-stall test)
- Modify: `emulator-core/src/rom.rs` (module doc "Where this gets boot to" ~L878–895; doc of `rom_md5_stubs_accept_the_synthesized_partition_tables_md5_entry` ~L3175–3185)
- Modify: `docs/firmware-emulator-notes.md`, `docs/milestone-4-decisions.md`

**Interfaces:**
- Consumes: Task 2's MMU-backed bus; `FirmwareRuntime::{from_image, run, pc, cpu, bus}`; `RunSummary::{traps, last_instruction_fault, rom_stub_calls}`.
- Produces: the rung name `load_partitions_accepts_the_synthesized_table_through_the_flash_mmu` (Task 7's docs cite it).

Known facts from M3 (`rom_stub_boot.rs` and the notes): ROM `MD5Init` = `0x4000_0614`, `MD5Update` = `0x4000_0618`, `MD5Final` = `0x4000_061c` (`esp32c3.rom.ld`); `load_partitions()` epilogue is `0x420f_a878` (`mv a0, s4`, `s4` = err); the first `MD5Init` from it was step 5,555,258.

- [ ] **Step 1: Write the rung** (in `boot_progress.rs`, reusing its `factory()` helper):

```rust
/// Milestone 4 Task 3: with the flash MMU modeled, `load_partitions()`
/// reads the synthesized partition table through its own `spi_flash_mmap`
/// window, feeds the entries to ROM `MD5Update`, checks the digest with
/// `MD5Final`, and returns `ESP_OK` (M3 ended with `ESP_ERR_NOT_FOUND`
/// here because the window read as zeros).
#[test]
fn load_partitions_accepts_the_synthesized_table_through_the_flash_mmu() {
    const ROM_MD5_INIT: u32 = 0x4000_0614;
    const ROM_MD5_UPDATE: u32 = 0x4000_0618;
    const ROM_MD5_FINAL: u32 = 0x4000_061c;
    const LOAD_PARTITIONS_EPILOGUE: u32 = 0x420f_a878;
    const ESP_OK: u32 = 0;

    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let summary = rt.run(5_000_000);
    assert_eq!(summary.last_instruction_fault, None, "{summary:?}");

    // Single-step to load_partitions()'s first MD5Init, then to its epilogue.
    let mut reached_init = false;
    let (mut updates, mut finals) = (0u32, 0u32);
    let mut err = None;
    for _ in 0..1_000_000 {
        match rt.pc() {
            ROM_MD5_INIT => reached_init = true,
            ROM_MD5_UPDATE if reached_init => updates += 1,
            ROM_MD5_FINAL if reached_init => finals += 1,
            LOAD_PARTITIONS_EPILOGUE if reached_init => {
                err = Some(rt.cpu().regs.read(20));
                break;
            }
            _ => {}
        }
        let s = rt.run(1);
        assert_eq!(s.last_instruction_fault, None, "{s:?}");
    }
    assert!(reached_init, "load_partitions() never called MD5Init");
    assert_eq!(err, Some(ESP_OK), "s4 = err at the epilogue");
    // One MD5Update per partition entry (4) is the expected shape; at least one is required.
    assert!(updates >= 1, "MD5Update calls: {updates}");
    assert_eq!(finals, 1, "MD5Final calls");
    assert!(
        !rt.bus()
            .unmapped_log()
            .iter()
            .any(|a| (0x3c00_0000..0x3c80_0000).contains(&a.addr) && !a.is_write),
        "no DBUS read went through an invalid MMU entry"
    );
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p emulator-core --release --test boot_progress load_partitions_accepts`
Expected: PASS if Task 2 is complete. If it fails, do not weaken it: diagnose with `cargo run -p emulator-core --release --example boot-probe -- --steps 5700000` and disassembly around the PCs (Task D protocol, Step 2) and report. A failure where `MD5Final` runs but `err != 0` means the digest disagrees: check `PARTITION_TABLE_MD5` covers exactly the bytes `load_partitions()` hashes (v5.5.3 `components/esp_partition/partition_target.c`).

- [ ] **Step 3: Retire the pinned-stall phases.** In `rom_stub_boot.rs`, rename `boot_stubs_rom_md5init_then_load_partitions_reads_zeros_through_the_unmapped_flash_mmu_window` to `boot_stubs_reach_load_partitions_md5init_after_drawing_the_splash`; delete Phases 11–13 and the constants only they use (`ROM_MD5_UPDATE`, `ROM_MD5_FINAL`, `LOAD_PARTITIONS_MMAP_RA`, `LOAD_PARTITIONS_EPILOGUE`, `MMAP_TABLE_VADDR`, `MMU_TABLE_ENTRY_39`, `ESP_ERR_NOT_FOUND`); end the test after Phase 10's `MD5Init` return; condense its doc comment to what Phases 1–10 assert (history stays in the notes). Phases 1–10 must pass unchanged; if they do not, that is the Task 2 Step 5 situation (pre-splash behavior changed): stop and report.

- [ ] **Step 4: `rom.rs` docs.** Replace the "As of Task D13" paragraph in "Where this gets boot to" with the current state (MD5 stubs now used end to end by `load_partitions()`; point at the new rung), keeping D13 as one history sentence. In `rom_md5_stubs_accept_the_synthesized_partition_tables_md5_entry`'s doc, drop the "stand-in for boot" wording: it is now a unit test of the stubs; the boot rung is the end-to-end proof.

- [ ] **Step 5: Find the next stall** and record it (this feeds the orchestrator's next task choice):

Run: `cargo run -p emulator-core --release --example boot-probe -- --steps 12000000 --window 500000 --dump-frame m4-task3.png`
Record in the ledger and in a notes history entry: last console line, top hot PCs, unmapped-access tail, framebuffer state (still the splash?), fault if any.

- [ ] **Step 6: Docs, full suite, commit.** Notes: "Current state" bullet that partition loading succeeds; history entry "Milestone 4 Task 3" with Step 5's findings. Decisions doc: nothing unless a decision was made.

Run: `cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && bun run build:wasm && bun test && bun run typecheck`

```bash
git add -A emulator-core docs
git commit -m "test(emulator-core): load_partitions() accepts the partition table through the MMU

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task D (template, repeat per stall)

Every stall after Task 3 that is not one of Tasks 4–6 below, and each of those tasks' own sub-stalls, gets a numbered task created from this template by the orchestrator (`Task D-M4-<n>`), dispatched like any other.

**Files:** the module that owns the registers (new `emulator-core/src/peripherals/<name>.rs` + `peripherals/mod.rs` + `mem/soc.rs` + `mem/bus.rs` routing if new), or `src/rom.rs` for a ROM call; `tests/boot_progress.rs`; `docs/firmware-emulator-notes.md`; `docs/milestone-4-decisions.md` if a decision was taken.

- [ ] Step 1: Reproduce with `boot-probe` (`--steps`, `--window`); record last console line, hot PCs, unmapped tail, fault in the task brief and ledger.
- [ ] Step 2: Identify the registers / ROM call and the ESP-IDF v5.5.3 source (header + HAL/LL) that reads them. Disassemble `factory.bin` around the hot PCs if needed (`riscv64-unknown-elf-objdump -b binary -m riscv:rv32 -D --adjust-vma=<load_addr>` on the extracted segment, or a small Rust helper using this crate's decoder). ROM addresses from `rom*.ld` and `local/rom-elfs/`. Do not guess.
- [ ] Step 3: Failing unit test with literal register writes/reads mirroring the cited LL/HAL function (byte-split word writes for registers); run → FAIL.
- [ ] Step 4: Register-faithful minimal implementation; module doc cites sources; run → PASS.
- [ ] Step 5: Ratchet rung in `boot_progress.rs`: a newly reached console line (`assert_reaches(needle, budget)`), or a no-fault / hot-PC-escape assertion if no line appears. No exact step counts beyond a budget.
- [ ] Step 6: Notes history entry (found → resolved), decisions doc entry if a choice was made, full suite (`cargo test --workspace`, `--release --test boot_progress`, `build:wasm`, `bun test`, `typecheck`), commit `feat(emulator-core): model <thing> (<what it unblocks>)`.

Rules: the fix is the smallest *correct* model, never a value chosen to make the firmware proceed. ROM stubs only "when observed", with header-documented semantics. If a fix would override an M3 decision or the spec, stop and ask the human partner through the orchestrator.

---

### Task 4: Console output after the scheduler starts

Do this right after Task 3 whether or not it is what stalls: later diagnosis needs the firmware's own logs.

**Files:** to be determined by the trace; most likely `emulator-core/src/peripherals/usb_serial_jtag.rs`, possibly `src/rom.rs` (a ROM lock/printf helper), `mem/bus.rs` (an interrupt source), `tests/boot_progress.rs`, notes.

**Known facts:** every console byte so far came from the ROM `ets_printf` HLE or direct EP1 writes before/just after the scheduler started; `load_partitions()`'s `ESP_LOGE` (M3) ran `esp_log_write` but produced no byte. The USB-Serial-JTAG model already reports `SERIAL_IN_EP_DATA_FREE = 1` and `SERIAL_IN_EMPTY_INT_RAW = 1`, so "FIFO full" is not the cause by itself. Candidates to check, in order: (a) post-scheduler `esp_log_write` → `vprintf` → newlib `stdout` → VFS → `usb_serial_jtag_vfs.c`'s write path, and whether the console driver was switched to the interrupt-driven `usb_serial_jtag` driver (`esp_driver_usb_serial_jtag/src/usb_serial_jtag.c`), which queues bytes in a ring buffer and only moves them to the FIFO from its ISR (`ETS_USB_SERIAL_JTAG_INTR_SOURCE`, which the bus does not raise); (b) a newlib lock (`_lock_acquire_recursive`) that blocks; (c) log level / line buffering (stdout buffered until `\n` or `fflush`).

- [ ] **Step 1:** Run boot-probe past Task 3's point and find where a known post-scheduler log call's bytes go: set a breakpoint-style check in a scratch example (not committed) or use `run_traced` hot PCs plus disassembly to follow `esp_log_write` into the VFS write function; note whether bytes end up in a RAM ring buffer. Fetch the v5.5.3 sources named above to the scratchpad.
- [ ] **Step 2:** Compare with `local/boot_log.txt`'s lines after `main_task: Calling app_main()` to pick the first line that should appear (do not quote identity lines in code, tests, docs or commits; pick a non-identity line).
- [ ] **Step 3:** Failing test(s): a unit test for whatever peripheral behavior is missing (e.g. if (a): USB-Serial-JTAG `INT_ENA`/`INT_ST` for `SERIAL_IN_EMPTY` as a level interrupt source in `pending_sources()`, with the source number `ETS_USB_SERIAL_JTAG_INTR_SOURCE` cited from `soc/esp32c3/include/soc/interrupts.h`), plus a rung `boot_prints_console_output_after_the_scheduler_starts` asserting that line with `assert_reaches`.
- [ ] **Step 4:** Implement; run → PASS; earlier rungs unchanged (the console-ending assertions in older tests that expect `Calling app_main()` to be the *last* line will now fail by design: update them to assert containment, and say so in the commit).
- [ ] **Step 5:** Add rungs for each further real boot-log line that now appears up to the current stall (one rung per milestone line, not per line).
- [ ] **Step 6:** Notes (resolve open limitation 2), decisions doc, full suite, commit `feat(emulator-core): console output after the scheduler starts`.

---

### Task 5: Flash writes via SPIMEM1 (expected for NVS)

Start when boot-probe shows the firmware spinning on `SPI_MEM_CMD_REG` (SPIMEM1, `0x6000_2000`) or reading SPIMEM1 data registers after a non-RDID command, typically inside `nvs_flash_init` / `esp_flash_*`. Otherwise defer and handle the actual stall with Task D.

**Files:** `emulator-core/src/peripherals/flash.rs` (`Spimem1`, `EmulatedFlash`), `tests/boot_progress.rs`, notes, decisions.

**Known facts:** `Spimem1` runs user-command transactions on the `SPI_MEM_USR` bit and only implements RDID; the dedicated command bits (`SPI_MEM_FLASH_*`, `CMD` bits 19..31) are stored but never self-cleared, so the driver spins. `esp_flash` on ESP32-C3 (`hal/spi_flash_hal_common.inc`, `hal/esp32c3/include/hal/spimem_flash_ll.h`) erases with the dedicated `FLASH_SE`/`FLASH_BE`/`FLASH_CE` bits, programs with `FLASH_PP` (data in `W0..W15`, address in `ADDR`), polls status with `FLASH_RDSR` or a user `RDSR` (0x05), enables writes with `FLASH_WREN`, and reads with a user command (e.g. 0x03/0x0B with `USR_MISO`, `MISO_DLEN`) into `W0..W15`.

- [ ] **Step 1:** Fetch `soc/esp32c3/register/soc/spi_mem_reg.h`, `hal/esp32c3/include/hal/spimem_flash_ll.h`, `hal/spi_flash_hal_common.inc`, `hal/spi_flash_hal.c` (v5.5.3) and list exactly which bits/registers the observed call sequence uses (from disassembly of the spin site). Cite bit positions from the header, not from this plan.
- [ ] **Step 2:** Failing `flash.rs` unit tests, each driving whole-word byte-split writes in the LL function's order and asserting: the command bit reads back 0 after the triggering write (self-clear); sector erase sets the 4 KiB sector to `0xFF`; page program ANDs `W0..` bytes into flash at `ADDR` for the programmed length; a user read returns flash bytes in `W0..` (little-endian per word); `RDSR` returns 0 (never busy, WEL modeled only if the driver checks it); unsupported commands are recorded in `unmodeled_commands` and still self-clear.
- [ ] **Step 3:** Implement in `Spimem1::write_byte` on the byte containing each trigger bit (M3 review focus 1: other fields come from already-applied bytes of the same write). Erase/program only within the 4 MiB chip; out-of-range addresses are ignored, never panic.
- [ ] **Step 4:** XIP coherence is automatic (Task 2's `spimem1_flash_program_is_visible_through_xip`); add a rung for the first boot line past NVS init (or a no-fault assertion).
- [ ] **Step 5:** Notes (backlog item resolved), decisions (what "never busy" means; if wrong, firmware that times busy-waits sees zero-latency flash), full suite, commit `feat(emulator-core): SPIMEM1 erase/program/read commands (NVS init)`.

---

### Task 6: ST7789 `MADCTL` (conditional)

Start when the first dumped launcher frame (or any intermediate frame) is mirrored, rotated, or drawn outside the expected window compared with the physical badge, or when a draw's address window exceeds the current 320×240 order. If the launcher matches the badge without it, skip this task and leave `MADCTL` in the M5 backlog.

**Files:** `emulator-core/src/peripherals/spi.rs`, `tests/boot_progress.rs` (hash updates), `frontend/test/cpu-wasm.test.ts` (hash), notes, decisions.

**Approach:** model the controller's frame memory (GRAM, 240 columns × 320 rows) separately from the 320×240 view the emulator exposes. `CASET`/`RASET`/`RAMWR` addresses are transformed by `MADCTL`'s `MY` (bit 7), `MX` (bit 6), `MV` (bit 5) per the ST7789V datasheet (section "Memory Data Access Control" and the "Frame memory / host interface address mapping" table: `MV` exchanges column and row, `MX` mirrors the column address, `MY` mirrors the row address) into GRAM coordinates. The view is GRAM through one fixed panel-mounting transform, chosen so that **the already human-confirmed splash renders identically**: `boots_to_first_real_frame`'s hash `0x5599c270ab0429fa` is the calibration test. If no fixed transform keeps that hash, the model is wrong; do not change the hash to make it pass.

- [ ] **Step 1:** Record which `MADCTL` values the firmware sends and when (before or after the splash draw) from a boot-probe trace; note it in the ledger.
- [ ] **Step 2:** Failing `spi.rs` unit tests: for `MADCTL` 0x00, 0x20, 0x60 and 0xC0, a 2×1 `CASET`/`RASET` window + 2 pixels lands at the GRAM cells the datasheet table gives; the view of a GRAM cell is the fixed transform.
- [ ] **Step 3:** Implement; `MADCTL` resets the in-progress `RAMWR` cursor like other commands.
- [ ] **Step 4:** `boots_to_first_real_frame` passes with the unchanged hash; the frame that motivated this task now matches the badge (dump and ask the human partner via the orchestrator).
- [ ] **Step 5:** Notes (resolve open limitation 3; the hash *is* now of the panel's view), decisions, full suite, commit `feat(emulator-core): model ST7789 MADCTL`.

---

### Task 7: Finish line — launcher rungs, button response, WASM twin, docs

Start when boot-probe shows the framebuffer replacing the splash with a stable non-splash frame.

**Files:** `emulator-core/tests/boot_progress.rs`, `frontend/test/cpu-wasm.test.ts`, `docs/firmware-emulator-notes.md`, `docs/milestone-4-decisions.md`, `CLAUDE.md`, `emulator-core/src/mem/bus.rs` / `rom.rs` module docs (state lines).

**Interfaces:**
- Consumes: `FirmwareRuntime::{from_image, run, framebuffer, set_raw_button, console_output}`; the existing `framebuffer_fnv1a(fb: &[u16]) -> u64` helper in `boot_progress.rs`. Raw slots: 0 `START`, 1 `A`, 2 `B`, 3 `HOME`, 4 `DOWN`, 5 `LEFT`, 6 `RIGHT`, 7 `UP`, 8 `AUX1` (`frontend/src/runtime/firmware-runtime.ts`).

- [ ] **Step 1: Find the launcher frame.** `boot-probe --steps <N> --dump-frame launcher.png` stepping N up in 250,000 chunks: the smallest N where the frame differs from the splash and stays unchanged for 1,000,000 further steps. Record N, distinct-color count and the hash in the ledger.
- [ ] **Step 2: Human gate.** The orchestrator asks the human partner to compare `local/launcher.png` with the physical badge's launcher (after a factory reset if the badge's NVS content changes what it shows, spec Part 3). If it does not match (orientation → Task 6; content → Task D), loop back. Record the outcome and date in the notes.
- [ ] **Step 3: `boots_to_launcher` rung** (mirror `boots_to_first_real_frame`'s structure exactly: 250,000-step chunks, stop once the hash has held for 1,000,000 steps, cap at N + 2,000,000):

```rust
/// Milestone 4 finish line: blank-flash boot reaches the app launcher.
/// Compared by eye with the physical badge on <date> (notes, "Current state").
#[test]
fn boots_to_launcher() {
    const LAUNCHER_HASH: u64 = 0x0000_0000_0000_0000; // Step 1's measured hash
    let mut rt = FirmwareRuntime::from_image(&factory()).expect("factory.bin boots");
    let (hash, step) = run_until_stable_frame(&mut rt, LAUNCHER_MAX_STEPS);
    assert_eq!(hash, LAUNCHER_HASH, "stable at step {step}");
    let console = rt.console_output();
    for panic_text in ["Guru Meditation Error", "abort()", "Rebooting..."] {
        assert!(!console.contains(panic_text), "console:\n{console}");
    }
}
```

  Extract `run_until_stable_frame(rt: &mut FirmwareRuntime, max_steps: u64) -> (u64, u64)` (returns `(hash, step at which it became stable)`) from `boots_to_first_real_frame`'s loop and make both tests use it; `LAUNCHER_MAX_STEPS` is N + 2,000,000 rounded up to a 250,000 multiple. Replace the placeholder hash with Step 1's measured value before committing (the test must never be committed with `0`).

- [ ] **Step 4: `launcher_responds_to_buttons` rung.** From the stable launcher, press a navigation slot for 100,000 steps, release, run until stable again; assert the new hash differs from `LAUNCHER_HASH` and equals the measured `LAUNCHER_AFTER_<BUTTON>_HASH`, no fault, no panic text. Try slot 4 (`DOWN`) first, then 6 (`RIGHT`); use the one the launcher reacts to (record which in the doc comment, with the human-observed badge behavior). If neither changes the frame, that is a stall: Task D on the input path (GPIO / 74HC165 / its interrupt).
- [ ] **Step 5: WASM twin.** In `frontend/test/cpu-wasm.test.ts`, add "boots factory.bin to the same launcher frame as the native finish-line test", copying the existing first-frame twin's structure and FNV-1a helper, with the same chunking and hash.
- [ ] **Step 6: Suite time.** `time cargo test -p emulator-core --release --test boot_progress`. If over ~10 s, add the shared-checkpoint fixture from the spec Part 4 (`#[derive(Clone)]` on `FirmwareRuntime`, `Cpu`, `FirmwareBus` and the peripherals; one `OnceLock<FirmwareRuntime>` booted to a checkpoint, cloned per test) as its own commit with a test that a clone continues identically to the original for 100,000 steps.
- [ ] **Step 7: Docs.**
  - Notes: "Current state (end of Milestone 4)" replacing M3's (launcher reached; step numbers; hash; human comparison date; what is still unmodeled), M3's current-state section moved under history, open limitations renumbered.
  - `docs/milestone-4-decisions.md`: complete the decisions (one section per task that took one), "Remarks" (finish-line tests and their hashes), "Milestone 5 backlog" (untouched M3 backlog items carried forward + new ones; first item: what blocks launching a built-in app, if known).
  - `CLAUDE.md`: the "Current state" paragraph (launcher reached; next blocker), `mem/` entry (MMU-backed XIP; D5 window and separate XIP flash buffer gone), `peripherals/` entry (`mmu.rs`; the "flash MMU is not modeled" sentence removed; SPIMEM1 commands if Task 5 ran), commands section if a new rung name is the finish line.
  - `bus.rs` / `rom.rs` module-doc state lines.
  - `tests/boot_progress.rs`: condense the per-rung task-history prose (spec Part 4) to what each rung asserts and why; the history lives in the notes. Test bodies unchanged.
- [ ] **Step 8: Full suite and commit.**

Run: `cargo test --workspace && cargo test -p emulator-core --release --test boot_progress && cargo clippy --workspace --all-targets && bun run build:wasm && bun test && bun run typecheck`

```bash
git add -A emulator-core frontend/test docs CLAUDE.md
git commit -m "test: real firmware boots to the app launcher (Milestone 4)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 9:** Orchestrator: whole-branch review, then `superpowers:finishing-a-development-branch` (the human partner decides merge/PR).

---

## Orchestrator notes

- Order: Task 1 → 2 → 3 → 4 → (boot-probe decides: Task 5 / Task 6 / Task D instances) → Task 7. Tasks 5 and 6 are conditional on their stated triggers; a skipped one is recorded in the decisions doc's M5 backlog.
- Give each implementer: this plan's Global Constraints + Review Focus, their task text, the spec path, the latest ledger findings (stall facts from the previous task's boot-probe step). Ask for a report with: what changed, test output summary, boot-probe findings, anything that contradicts this plan.
- Reviewers check: header citations present and correct, byte-split write behavior, never-panic on browser input, no exact step counts in new rungs beyond budgets, docs updated in the same commit, nothing from `local/` committed or quoted.
- Human partner checkpoints: Task 7 Step 2 (launcher comparison), Task 6 Step 4 (if run), and any override of an M3 decision or this spec.

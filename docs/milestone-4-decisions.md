# Milestone 4: design decisions, remarks, and the Milestone 5 backlog

Milestone 4 takes the real-firmware emulator from the boot splash (and the
silent `load_partitions()` stall behind it) toward the app launcher. The
design spec is `docs/superpowers/specs/2026-10-06-milestone4-app-launcher-design.md`
and the plan is `docs/superpowers/plans/2026-10-06-milestone4-app-launcher.md`;
`docs/firmware-emulator-notes.md` holds the forensic findings and the
stall-by-stall history. This file records what those do not make obvious:
the design decisions taken along the way (with what breaks if each one is
wrong) and what is deferred. Where the spec or plan disagree with this file
or the code, this file and the code win.

## Flash MMU (Task 2)

- **The MMU replaces Milestone 3's D5 page windows.** Every access in the
  DBUS (`0x3C00_0000..0x3C80_0000`) and IBUS (`0x4200_0000..0x4280_0000`)
  cache apertures translates through `crate::peripherals::mmu::FlashMmu`
  to a physical flash page and reads `FirmwareBus::flash_chip`; the old
  `XipRegion`/`xip_page_window` machinery and the bus's own copy of the
  image are gone. `FirmwareBus::from_segments` seeds the table by
  replaying the bootloader (`set_cache_and_start_app()` in
  `bootloader_support/src/bootloader_utility.c`, page rounding from
  `mmu_hal_map_region()` in `hal/mmu_hal.c`, both v5.5.3, confirmed
  against the fetched source: vaddr and paddr rounded down to 64 KiB,
  length extended by the vaddr's page offset and rounded up to whole
  pages). If wrong: any XIP segment whose flash offset and `load_addr`
  disagree modulo 64 KiB is now left unmapped and its fetches trap (the
  bootloader derives the paddr independently of the vaddr; this emulator
  derives it from `file_offset`). Esptool images, including `factory.bin`,
  always agree. Hand-built test images must respect this (the synthetic
  images in `boot.rs`/`runtime.rs` tests now load their IROM segment at
  `0x4200_0020`, matching the data's file offset of 32).
- **Physical pages past the 4 MiB chip wrap** (`paddr % FLASH_SIZE`). This
  is a reconstruction, not something read from a datasheet. If wrong: a
  firmware mapping past the chip would read the wrapped real data instead
  of blank/garbage.
- **An access through an invalid entry reads 0 and is logged; a fetch
  traps; no cache-error interrupt.** Real hardware raises a cache error
  interrupt; nothing observed relies on it.
- **The DBUS aperture is never fetchable** (the data bus is not executable
  on the ESP32-C3); only IBUS through a valid entry is.
- **No cache model.** Table writes are immediately coherent; there is no
  cache to invalidate. If wrong: code that relies on stale cached data
  after remapping would behave differently on hardware.
- **Addresses inside `DROM_RANGE`/`IROM_RANGE` but outside the 8 MiB
  apertures are not XIP** and fall to the logged catch-all, so the
  `0x7F_FFFF` entry-id mask can never alias them onto a table entry.
- **Entry 127 (`MMU_DROM_END_ENTRY_ID`) is seeded** with the DROM's first
  physical page, as the bootloader does "for app to find the boot
  partition". This resolves item 7 of the notes' history.

## Known consequence: boot now gets past `load_partitions()`'s read

With the MMU in place the splash is unchanged (frame hash
`0x5599c270ab0429fa`, reached by the 5,750,000-step sample, as before)
but boot no longer stalls silently after it. Just after `MD5Init` (step
5,555,258) the firmware called ROM `strncpy` (`0x4000_0368`); it is now a
real HLE stub (`RomStubEffect::Strncpy`, newlib semantics). The next point
boot reached was ROM `strcmp` (`0x4000_036c`, RA `0x420f_a9de`), now also a
real stub (`RomStubEffect::Strcmp`). With both in place
`boots_to_first_real_frame` passes again with its hash
`0x5599c270ab0429fa` unchanged and no fault through step 6.75M, as does its
WASM twin; the pinned-stall test in `tests/rom_stub_boot.rs` was renamed and
truncated to its Phases 1 to 10. Where boot goes after that (it still never
reaches the launcher) is the next stall-loop task: boot-probe to step
10,000,000 shows no fault, the same console, the splash unchanged.

## USB-Serial-JTAG: a host is always attached (Task 4)

- **`SOF_INT_RAW` (bit 1 of `USB_SERIAL_JTAG_INT_RAW_REG`) always reads
  1.** ESP-IDF's connection monitor
  (`esp_driver_usb_serial_jtag/src/usb_serial_jtag_connection_monitor.c`,
  v5.5.3) treats a tick without a SOF frame as "unplugged", and the VFS
  write then drops every `stdout` byte. A real host sends a SOF every 1 ms,
  and the emulator has no USB frame timing, so the bit is forced on, as
  `SERIAL_IN_EMPTY_INT_RAW` already was. The physical badge's serial log
  was captured over this same USB port, so "connected" is the state to
  match. If wrong: firmware that behaves differently when unplugged (no
  console, or code that waits for a host to go away) would never take that
  path in the emulator; and if firmware ever enables `SOF_INT_ENA`, nothing
  raises `ETS_USB_SERIAL_JTAG_INTR_SOURCE` (no USB-Serial-JTAG interrupt
  source is modeled yet), so an ISR expecting 1 kHz SOF interrupts would
  never run.
- **No USB-Serial-JTAG interrupt source yet.** The brief's first candidate
  (the interrupt-driven `usb_serial_jtag` driver waiting on
  `ETS_USB_SERIAL_JTAG_INTR_SOURCE`) was not the cause: the dropped writes
  returned before reaching any `tx_func`, and the physical badge starts its
  console REPL (the usual installer of that driver) only after the app
  launcher. It will be needed when
  boot gets there. If wrong: an earlier `usb_serial_jtag_driver_install()`
  would queue bytes in its ring buffer and print nothing.
- **Rungs leave the timestamp out.** Post-scheduler timestamps come from
  the FreeRTOS tick count and move with boot timing, so the console rungs
  match the tag and message only.
- **A rung pins an emulator-only line.** `esp_littlefs`'s `mount failed ...
  formatting...` is not in the physical badge's log (its flash holds a
  filesystem); it is what blank synthetic flash makes the real firmware do,
  and it marks the point just before the current stall. If wrong (e.g. a
  later task seeds a littlefs image): change or retire that rung.

## SPIMEM1 flash writes (Task 5)

- **The flash is never busy.** Every SPIMEM1 operation (dedicated command
  or user transaction) completes inside the byte write that starts it: its
  `CMD` bit reads 0 on the first poll, and the chip's status register
  reports `SR_WIP` = 0 on every `RDSR`, straight after an erase or program.
  A real sector erase takes tens of milliseconds and a page program around
  a millisecond, during which `spi_flash_chip_generic_wait_idle` keeps
  polling `RDSR` and calling `delay_us`. If wrong: firmware that times its
  busy-waits, or counts on an erase taking long enough for another task to
  run, sees zero-latency flash; ESP-IDF's erase/program timeouts can never
  fire, and auto-suspend (not modeled) can never trigger.
- **The write enable latch is modeled, as chip state kept in `Spimem1`.**
  `spi_flash_chip_generic_set_write_protect` reads `SR_WREN` back after
  `WREN`/`WRDI` and fails if it did not change, and `wait_idle` treats a
  latch still set after an erase or program as "command not accepted", so
  a model without it fails every write. As on SPI NOR flash, `WREN` sets
  it, `WRDI` clears it, and sector erase and page program run only while it
  is set and clear it. If wrong (the badge's chip ignores the latch): no
  observable difference for ESP-IDF, which always sends `WREN` first.
- **Erase and program addresses are `ADDR[23:0]`; anything past the
  4 MiB chip is ignored**, not wrapped. A real 4 MiB part ignores the upper
  address bits and would wrap. If wrong: a write past the chip lands at
  its start on hardware; ESP-IDF's own bounds checks reject such writes
  before they reach the controller.
- **Page program wraps within its 256-byte page**, the standard NOR page
  program behaviour; the write slicer (`memspi_host_write_data_slicer`)
  never crosses a page, so nothing observed depends on it.
- **The read family is modeled together.** `CMD_READ` 0x03 and the fast
  reads 0x0B/0x3B/0x6B/0xBB/0xEB differ only in line width and dummy
  cycles, which do not change the data, so all six return flash bytes;
  only 0xBB is observed. If wrong: none of the others is issued by this
  firmware today.
- **Dedicated commands without a model** (`FLASH_READ`, `RDID`, `RDSR`,
  `WRSR`, `BE`, `CE`, `DP`, `RES`, `HPM`) complete with no effect and are
  logged as `DEDICATED_COMMAND_BASE | bit` (`0xFF13..0xFF1F`), so a hang
  becomes a visible log entry. `BE` is left out on purpose: `spi_mem_reg.h`
  calls it a 32 KiB erase while ESP-IDF's generic driver erases 64 KiB
  blocks with it, so its size waits for evidence.
- **A lone `SPI_MEM_FLASH_PE`** (bit 17 without `SPI_MEM_USR`) runs
  nothing; it completes and is logged as `DEDICATED_COMMAND_BASE | 17`.
- **Finish-line tests are never narrowed** (coordinator ruling, fix round
  1). An observed ROM libc call that faults gets a real-implementation stub
  in the task that hits it, as `strncpy`/`strcmp` did in Task 2. Task 5
  first narrowed `boots_to_first_real_frame`'s fault window around the
  `strlcat` fault; that was reverted and ROM `strlcat`, `strspn` and
  `strcspn` (all observed: `esp_vfs_littlefs_register`, then littlefs's
  path walk) became real stubs (BSD/newlib semantics) instead.

## Milestone 5 backlog

(To be filled in by later Milestone 4 tasks.)

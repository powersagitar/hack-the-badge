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

## I2C0 with no device attached (Task D-M4-1)

- **No device answers on the bus** (coordinator ruling R11). The
  controller is modeled; the accelerometer (SC7A20H) and any other I2C
  device are not. Every ACK slot reads 1 (SDA released), so an address
  byte sent with ACK checking is NACKed and `hal_accel` logs "accelerometer
  setup failed", a line the physical badge never prints. This is what the
  hardware does with nothing on the bus, not a value chosen to make boot
  proceed; emulating the sensor would add synthetic readings beyond the
  smallest correct model. If wrong (the launcher or an app needs the
  accelerometer): a later task adds a device model behind the address
  phase, which changes this rung's line to the detection line.
- **A transaction completes inside the `TRANS_START` byte write.** Nothing
  on an empty bus can stretch SCL, so the whole command list runs at once
  and its interrupts are raised before the next instruction. The driver
  resets `event_queue` before starting and only reads it afterwards, so
  zero latency is safe for it. If wrong: firmware that times an I2C
  transfer, or expects to run code between start and completion, sees no
  gap; the driver's hardware/software timeouts can never fire.
- **The STOP after a NACK raises `TRANS_COMPLETE_INT` as well as
  `NACK_INT`.** The driver waits for `SR.BUS_BUSY` to drop after a NACK
  ("start->address->nack->stop"), so the STOP itself is certain; whether
  the controller also flags it as a completed transaction is a judgment
  (it detects its own STOP bit). The ISR checks NACK first and clears
  every status bit it read, so the driver cannot tell. If wrong: an ISR
  that tests `TRANS_COMPLETE` before `NACK` would report success.
- **Not modeled:** timeouts, arbitration, watermark interrupts
  (`TXFIFO_WM_INT_RAW` keeps its reset value 1 until cleared), slave
  mode, non-FIFO mode, and the `CONF_UPGATE` shadow-register sync (live
  values are always used). `SCL_RST_SLV_EN` (bus clear) self-clears at
  once. None of these is reached by this firmware today.
- **ROM soft-double helpers are real stubs, like the libgcc integer
  ones** (ruling R10, applied to libgcc as `milestone-3-decisions.md`
  already does for "ROM libc / libgcc helpers"). `__floatunsidf`,
  `__muldf3`, `__divdf3` and `__fixunsdfsi` are computed with host `f64`
  arithmetic, which is bit-identical to libgcc soft-fp's round-to-nearest
  for every non-NaN result; NaN results are canonicalized to soft-fp's
  RISC-V quiet NaN and `__fixunsdfsi` mirrors `_FP_TO_INT`'s NaN case
  (sign clear: all ones). If wrong (the ROM was built with a different
  soft-float library): only NaN payloads or a non-default rounding mode
  could differ; the ESP32-C3 has no FPU to set one.

## RMT as a zero-latency TX engine (Task D-M4-2)

- **A transmission runs in zero emulated time** (coordinator ruling R13).
  The transmitter consumes RMT RAM symbols inside the `TX_START` byte
  write; symbol durations, clock dividers and the carrier are stored but
  never timed. If wrong (firmware that measures how long a frame takes,
  or expects to run code between start and `TX_END`): it sees no gap; an
  LED visualisation would need symbol capture with timestamps.
- **The transmitter pauses at each enabled threshold/loop event until
  the firmware acknowledges it** (clears the raw bit, or clears its
  enable). Zero latency alone is ill-defined in wrap mode: with no time
  passing, the transmitter would lap stale RAM before the ISR could
  refill it. Pausing gives the sequence a real transmitter produces when
  its ISR keeps up: after a threshold event the hardware is still sending
  the other half, so letting the model send that half at the ISR's clear
  and stop at the next boundary sends the same data in the same order,
  and the half the ISR then refills is the next one sent. A disabled
  event never pauses (its raw bit is still set). If wrong (firmware that
  enables `TX_THR_EVENT` but never services it, or relies on an underrun
  re-sending stale data): the model waits where hardware would run on.
- **`TX_LIM` counts words sent since `TX_START` and restarts at every
  event.** TRM §33.3.4.2/§33.3.7 say the event fires when the amount sent
  reaches `TX_LIM`; the driver's ping-pong (`rmt_isr_handle_tx_threshold`
  toggles halves on every event) needs it to recur every `TX_LIM`
  words. An end-marker word is not counted.
- **Continuous mode: the loop counter holds after `TX_LOOP`** until
  `LOOP_COUNT_RESET` or the next `TX_START`; the channel keeps running
  (no `SOC_RMT_SUPPORT_TX_LOOP_AUTO_STOP` on the ESP32-C3) until
  `TX_STOP`. A running channel whose output has become periodic (a
  whole RAM lap with no change to `INT_RAW` or the loop counter) is left
  running without further work, and any later RMT write re-evaluates it.
  If wrong (the counter really wraps and re-fires): only firmware that
  leaves a looping channel running across several loop-count periods
  without stopping it would see fewer `TX_LOOP` events. Unused by this
  firmware (it never sets `TX_CONTI_MODE`).
- **A non-wrap overrun raises `ERR` and `MEM_EMPTY`, not `TX_END`**
  (TRM §33.3.7, register 33.8); `MEM_SIZE` 0 behaves as an exhausted
  RAM. Unused by this firmware.
- **Not modeled:** the APB FIFO access mode (`CHnDATA`, left unhandled
  so the bus logs any use), RX channels (no input ever arrives),
  simultaneous TX (`TX_SIM` is stored; each channel starts on its own
  `TX_START`), the output level, and `STATUS.STATE` (reads 0).
  `CONF_UPDATE` is a no-op: live register values are always used.
- **ROM `strdup` is guest code, `strchr`/`strcpy` are atomic stubs**
  (ruling R10). `strdup` must allocate through the firmware's heap, so,
  like `qsort`, it is real RV32 code in `ROM_CODE_FREE_RANGE`, reaching
  `__getreent` and `_malloc_r` through ROM newlib's `syscall_table_ptr`
  exactly as the ROM's own trampolines do. If wrong (the firmware swaps
  the syscall table after init): the code reads the pointer on every
  call, as the ROM does, so it follows the swap.

## Finish line: the first-run screen, not the launcher (Task 7)

- **The finish line pins what blank-flash boot shows** (spec Part 3; the
  human partner's choice, 2026-10-07). Boot reaches My Badge's
  unregistered first-run screen and stops there by firmware design: an
  unprovisioned My Badge claims HOME and reacts only to START (the notes'
  "Why the app launcher is not reached"). So `boots_to_first_run_screen`
  replaces the planned `boots_to_launcher`, and
  `first_run_screen_responds_to_start` replaces
  `launcher_responds_to_buttons` (START is the one button that screen
  reacts to; DOWN/RIGHT do nothing there, on the badge too). If wrong (the
  launcher was the bar): Milestone 5's first backlog item.
- **No synthetic identity in emulated flash yet.** Provisioning the
  emulated badge would get past My Badge to the launcher, but it is
  synthetic data in the `storage` partition, which the data-handling rules
  keep blank; it needs its own ruling. If wrong: nothing breaks; the
  launcher just waits a milestone.
- **One stable-frame helper.** `run_until_stable_frame` (250,000-step
  samples) serves the splash, first-run and post-press rungs. A uniform
  frame (blank or a solid fill) and any hash in a skip list never count as
  stable; the first-run rung skips the splash, which is itself stable for
  millions of steps before the app draws. It returns the first sampled
  step that showed the stable frame. If wrong (a later frame is uniform on
  purpose): pass a different predicate.
- **An 8,000,000-step window after a button press.** After START the
  firmware computes for about 6,000,000 steps before the self-test frame
  lands, so the boot rungs' 1,000,000-step window reports the unchanged
  first-run frame as stable. The post-press rung does not skip the
  first-run hash, so a press the firmware ignores fails on the hash
  assertion instead of timing out. If wrong (the busy phase grows past
  8M, e.g. with real-time calibration): widen the window.
- **Caps** are the measured first stable sample plus the window plus
  1,000,000, rounded up to 250,000: 17,500,000 for the first-run screen
  (15,500,000 + 2,000,000, as the plan asked) and 31,500,000 for the
  self-test (22,350,000 + 8,000,000 + 1,000,000). They are only reached if
  a frame never stabilizes.
- **MADCTL stays unmodeled (Task 6 skipped).** Its trigger, a frame the
  human comparison finds rotated or mirrored, did not happen: the
  first-run and self-test frames match the badge in orientation, and the
  screen test's corner fiducials are fully lit, so nothing is clipped. If
  wrong (another screen draws through a window the default order gets
  wrong): Task 6 as planned; the pinned hashes should not change for a
  model that keeps today's image.
- **No shared-checkpoint fixture** (spec Part 4). The `--release`
  `boot_progress` suite takes about 2.6 s (35 tests, run in parallel),
  well under the ~10 s trigger, so `FirmwareRuntime` does not derive
  `Clone`.

## Remarks

- **Finish-line tests** (`emulator-core/tests/boot_progress.rs`):
  `boots_to_first_run_screen` pins the first-run screen,
  `0x8c5027cec0f79490` (45 colors, stable from the 15,500,000-step sample,
  found at 16,500,000), and `first_run_screen_responds_to_start` pins the
  self-test's button screen after a 100,000-step START press,
  `0x7f325d838835baaa` (48 colors, stable from the 22,350,000-step sample,
  found at 30,350,000). `boots_to_first_real_frame` still pins the splash,
  `0x5599c270ab0429fa`. The WASM twins in `frontend/test/cpu-wasm.test.ts`
  reach the splash and the first-run hash through the wasm-bindgen build.
- **Human comparison** (2026-10-07): the first-run and self-test frames
  match a factory-reset physical badge in content and orientation (the
  notes' "Current state").
- **The self-test hashes beyond the first** (A lit, B lit, ..., the
  summary screen) were measured in exploration and are not pinned; the
  sequence is in the notes' "Milestone 4 Task 7" entry.
- **Idle time is still compressed** (WFI fast-forward, one step per
  SYSTIMER tick), and RMT, I2C0 and SPIMEM1 complete in zero emulated
  time. Step counts are not real time.

## Milestone 5 backlog

### Launcher and apps

- **Reaching the app launcher needs a provisioned identity in emulated
  flash: the first blocker.** On blank flash `hal_identity` finds no
  `/littlefs/identity.json` and the firmware marks the badge unprovisioned
  (flag byte at `0x3fca_92b2`, read by `0x4200_ee06`). My Badge's
  "handles HOME" hook (`0x4201_3920`) always says yes, and its button
  handler (`0x4201_4130`) reacts only to START while unprovisioned; with
  the flag set, HOME goes to the launcher (`0x4203_8cba`). So a synthetic
  identity in the littlefs `storage` partition (its format to be read from
  the firmware, never from the physical badge's dump) would get there; it
  needs a data-handling ruling first. Then the planned launcher rungs:
  `boots_to_launcher` and a DOWN/RIGHT navigation rung (whichever moves
  the selection).
- Launching and playing a built-in app (Snake, Dice) comes after the
  launcher.
- The ~6,000,000-step busy phase after START (hot PCs `0x420b_ead4`,
  `0x420b_b6a4`, no frame writes) is not traced; it may be real work or an
  artifact of untimed peripherals.

### Peripherals

- No I2C device: the accelerometer probe is NACKed and the self-test reads
  0 mg. An SC7A20H model behind the address phase would change
  `hal_accel`'s line.
- RMT transmissions are consumed but not decoded, so the LEDs are never
  shown; a frontend LED view would need symbol capture.
- NFC is not modeled (the self-test waits for a card forever).
- No USB-Serial-JTAG interrupt source; the console REPL the badge starts
  after the launcher may install the interrupt-driven driver.
- Unmapped accesses at the idle point: ASSIST_DEBUG (`0x600c_e0xx`),
  SYSTEM `0x600c_0058`/`+0x08`, RTC_CNTL `0x6000_80bc`. Harmless so far.
- Zero-latency RMT, I2C0 and SPIMEM1 (see their sections above).

### Carried from the Milestone 3 backlog (untouched)

- Every log timestamp up to `main_task: Calling app_main()` reads `I (0)`
  (likely the unmodeled performance-counter CSR); later ones come from the
  tick count.
- ST7789 `MADCTL` (`0x20`, then `0x60`) is unmodeled; orientation is
  human-confirmed for the frames seen so far (Task 6 skipped, above).
- GPIO0 (the display D/C pin) is routed as SPI2 `FSPIWP`/`FSPIHD`; routing
  is stored only. Revisit if D/C misbehaves.
- eFuse block unmodeled: boot logs chip revision v0.0, the badge is v0.4.
- `CPU_FREQ_MHZ` stub returns 160; the real CPU runs at 80 MHz.
- `TICKS_PER_STEP = 1` is about 10x the real SYSTIMER/CPU ratio; no
  real-time calibration.
- Flagged ROM-stub guesses: `rom_i2c_readReg*` returning 0,
  `Cache_Get_*` returning 0 (notes, history item 5).
- Edge-type interrupts are simplified: `CPU_INT_TYPE_REG` and
  `CPU_INT_CLEAR_REG` are plain storage.
- GDMA: DMA spans that cross into an adjacent RAM region are rejected;
  the RX in-link walk, CPU FIFO push/pop, the `OUT_DSCR*` pre-fetch
  registers and transfer timing are not modeled; a trailing zero-length
  `suc_eof` descriptor is never visited; `RESTART` with nothing to restart
  leaves the channel active.
- `boot_until_console_contains` checks in 250,000-step chunks, so rung
  budgets are looser than they read; the stable-frame step is quantized
  the same way.
- The boot-ladder tests each boot from scratch (the suite takes ~2.6 s
  today); add the shared-checkpoint fixture if it passes ~10 s.
- `MAX_STUB_MEMORY_BYTES` clamps in the `strlen`/`memcmp`/`MD5Update`
  stubs truncate silently.
- Small duplications: the `memcpy`/`memset` and `memcmp`/`strncmp` stub
  arms, and the `header_with_speed_size` test helper in `image.rs` and
  `rom.rs`.

Resolved from the Milestone 3 backlog in Milestone 4: the flash MMU and
real `load_partitions()` MD5 acceptance, the D5 window's replacement and
the extra DROM entry, SPIMEM1's dedicated commands, console output after
the scheduler starts, and the condensed test-history prose.

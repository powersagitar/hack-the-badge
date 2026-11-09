# Milestone 11: decisions and the Milestone 12 backlog

Milestone 11 sweeps the Milestone 11 backlog
(`docs/milestone-10-decisions.md`, "Milestone 11 backlog", with its
Milestone 9 and Milestone 8 carry-overs). Spec:
`docs/superpowers/specs/2026-11-03-milestone11-backlog-sweep-design.md`;
plan: `docs/superpowers/plans/2026-11-04-milestone11-backlog-sweep.md`.

## Why

The human partner chose to clear the whole backlog (2026-11-03). The large
features are set aside (Share's radio transfer and Sync's empty-room path
go to Milestone 12), and the items that need work nobody will do are
closed in writing (below). What is left is of two kinds:

- small faults, mostly from reviews, in the core, the browser and the
  signaling function;
- three places where the emulated reboot differs from the badge's: the
  RTC timer restarts, a reboot takes no time, and a CPU-only reset is
  treated as a full system reset.

Every item ends as a fix with a test, a trace whose result is recorded,
or a written closing that cites who decided it.

## Measurements

### Copied from the spec ("Measured (2026-11-03)")

#### A software reboot on the physical badge (v335)

Two captures, through the badge's own console command `reboot` (its help
text is "Restart the badge"), over `/dev/cu.usbmodem101` with pyserial
from `local/.venv`. The logs are in
`local/m11-badge/reboot_log_{1,2}.txt` (local only). Only the timestamps,
tags and segment sizes were read from them.

- **The reset reason** is `rst:0xc (RTC_SW_CPU_RST)`: `esp_restart()`
  ends in ROM `software_reset_cpu`, a CPU-only reset, as `crate::rom`
  entry 26 says.
- **The USB-Serial-JTAG link survived the reset.** The host port never
  disconnected, and the ROM banner and the bootloader's log arrived on
  the same open port. ESP-IDF's `esp_restart_noos` keeps the BBPLL on
  "so that USB Serial JTAG can log at 1st stage bootloader".
- **The timeline is identical in both runs, to the millisecond:**

  | Stage | Timestamp (ms, the bootloader's log clock) |
  |---|---|
  | 2nd-stage bootloader's first line | 24 |
  | `esp_image` segment 0 (map, 1,277,576 B) | 26 → 230 |
  | segment 1 (load, 33,128 B) | 230 → 236 |
  | segment 2 (map, 1,219,404 B) | 236 → 431 |
  | segment 3 (load, 27,260 B) | 431 → 436 |
  | segment 4 (load, 105,324 B) | 436 → 456 |
  | segment 5 (load, 32 B), then the image hash | 456 → 467 |
  | the app's `cpu_start` (first line) | 468 |

- **The cold boot looks the same.** The real cold boot log
  (`local/boot_log.txt`, `USB_UART_CHIP_RESET`) has the same timeline,
  within 1 ms per line (`cpu_start` at 468).
- **Checking a segment costs a steady amount per byte:** 0.160 µs per
  byte for the two mapped segments (204 ms and 195 ms), and about
  0.18–0.19 µs per byte for the loaded ones.
- **Scaled to v392's segments** (from `firmware.bin`'s header), the delta
  against v335 is:

  | Segments | Size change | Time |
  |---|---|---|
  | Mapped | +13,420 B | +2.1 ms |
  | Loaded | −24,416 B | −4.6 ms |

  So v392's reset-to-app time is about 466 ms, against v335's measured
  468.

#### Upstash's free plan (upstash.com/pricing/redis, fetched 2026-11-03)

| Limit | Value |
|---|---|
| Commands | 500K per month |
| Rate | 10,000 commands per second |
| Data | 250 MB (the comparison table says 256 MB) |
| Bandwidth | 10 GB per month |
| Request size | 10 MB at most |

Today's limiter (`frontend/src/radio/signal-handler.ts`) costs about 7
commands per valid request:
- each request counts against three windows;
- each count is an `INCR` and an `EXPIRE … NX` in one pipeline;
- then the room's own `GET` or `SET`.

What that buys:
- **A typical pairing** (one create, about 10–30 polls, one answer) is
  about 70–210 commands, so the free month covers a few thousand
  pairings.
- **At the global cap** (5,000 requests per 10 min) the month's quota is
  spent in about 2.4 hours.

When the quota is gone, Upstash errors, and every request gets the bare
500 (the store-failure path). The 10K per second rate cannot be reached
under the caps.

#### Facts read for the reset fixes

- **What `esp_restart_noos` resets.** Read from ESP-IDF v5.5.3,
  `components/esp_system/port/soc/esp32c3/system_internal.c`, before the
  CPU reset:
  - it disables the TG0 and TG1 watchdogs;
  - it arms the RTC watchdog for 1 s;
  - it disables the ICache;
  - it rewrites `GPIO_FUNC0..5_IN_SEL_CFG` to `0x30`;
  - `esp_system_reset_modules_on_exit()` pulses three reset groups:

    | Register | What it resets |
    |---|---|
    | `SYSTEM_CORE_RST_EN` | the WiFi, BT and SDIO blocks |
    | `SYSTEM_PERIP_RST_EN0` | `TIMERS`, `SPI01`, `UART`, `SYSTIMER` |
    | `SYSTEM_PERIP_RST_EN1` | `DMA` and the crypto blocks |

  - `rtc_clk_cpu_set_to_default_config()` puts the CPU back on XTAL.

  Nothing else is reset by software. In particular these keep their
  state: SPI2, I2C0, RMT, GPIO (except the six input selects), the
  interrupt matrix, USB-Serial-JTAG and APB_CTRL.
- **What the emulator does today.** `FirmwareRuntime::soft_reset`
  (`emulator-core/src/runtime.rs`) rebuilds every peripheral, whichever
  stub asked. It keeps the flash, `STORE0..7`, RTC fast memory, the
  panel, the console text and the host input, and drops queued serial
  input.
- **No pin depends on a reset before the launcher.** The walk to hacker's
  launcher crosses no soft reset (Connect's rungs assert
  `soft_reset_count() == 0` from the launcher). So
  `execution_digest_is_unchanged` and `hacker_walk_totals_are_unchanged`
  cannot move from Part 1. The hardware-compared frames cannot either,
  since none of them follow a reboot.

**Correction (Task 1).** The paragraph after "What `esp_restart_noos`
resets" above lists SPI2, I2C0 and RMT among what keeps its state. That
holds for `esp_restart_noos` alone. The app's own startup then resets
all three on every boot, CPU resets included (`esp_perip_clk_init`;
see "What a CPU reset keeps", rows SPI2, I2C0 and RMT). Also,
`SYSTEM_TIMERS_RST` in `SYSTEM_PERIP_RST_EN0` is not the timer groups:
the TRM lists TIMG0 and TIMG1 under their own bits (row TIMG).

### Baselines (Task 1, at `5fb5704`)

For Part 7's before-and-after. `bun run build` (Vite; gzip sizes as Vite
printed them):

| File | Bytes | Vite's gzip |
|---|---|---|
| `dist/index.html` | 984 | 0.46 kB |
| `dist/assets/bridge-BE-WtxC7.js` | 5,773 | (not printed) |
| `dist/assets/firmware-worker-BLgjDQZt.js` | 7,636 | (not printed) |
| `dist/assets/emulator_wasm_bg-mdhHNDGh.wasm` | 137,533 | 60.61 kB |
| `dist/assets/index-XRjmGPO9.css` | 2,962 | 1.01 kB |
| `dist/assets/index-DKp2lptT.js` | 276,964 | 92.23 kB |

`bun test`: 275 pass, 0 fail, 17,823 `expect()` calls, 20 files,
10.74 s.

### The fuzz tests (Task 6)

Both are `proptest` runs from a fixed seed (ruling R-T6-3):

| Test | Cases | Per case | Native, release | `cargo test`, debug profile |
|---|---|---|---|---|
| `radio::hci::tests::fuzzed_packets_and_advertisements_stay_bounded` | 512 (`HCI_FUZZ_CASES`) | 1 to 128 operations | 0.04 s | 0.85 s |
| `runtime::tests::deliver_rejects_malformed_bytes` (the serial bytes) | 2,000 | one byte vector, 0 to 299 long, and an RSSI | under 0.01 s | 0.05 s |

Three runs each; `cargo`'s "finished in". The HCI run comes to 33,394
operations: 14,049 advertisements heard (the old test: 10,000), 26,515
packets (the old test: 10,000; these count the well-formed setup
packets), and 1,695 drains. They reach 7,837 reports, the queue at its
bound 2,943 times, and all 256 values at each address byte.

The old test's checks all stay (no panic, the bounded queue, an
advertisement's data length, each drained event's framing); nothing is
dropped. What differs is the shape: it ran one controller through
10,000 steps, and each case now starts a new one for up to 128
operations. Nothing in the controller grows past its queue and its
filter, and the run does fill the queue.

RED, each from one mutation of `emulator-core`:

| Mutation | What fails |
|---|---|
| `open_scanning` sends Enable 0 | HCI: "the fuzz reached the report path" |
| `deliver` always refuses | HCI: the same; `deliver_rejects_malformed_bytes`: the positive control |
| `open_scanning` clears the report's LE mask bit | HCI: the same (2 reports, under the 1,000 floor; with a `> 0` it passed) |
| `hear` repeats one address byte | HCI: "more than one byte value per address" |
| The queue's bound is 33 | HCI: the bound's `op` assertion |
| The serial test's well-formed rule is `<= 2` | serial: the `heard == well_formed` assertion |
| The serial bytes are 0 to 6 long | serial: "the fuzz reached the accepting path" |

### The signaling function's cost (Task 9)

**How Upstash bills a script call.** Upstash's "Costs" page for
`@upstash/ratelimit`
(https://upstash.com/docs/redis/sdks/ratelimit-ts/costs, fetched
2026-11-09) counts the script call itself and every Redis command the
script runs: a fixed-window `limit()` is "EVAL, INCR, PEXPIRE" (3) on a
key's first count in a window, "EVAL, INCR" (2) after it, "EVAL, INCR"
(2) when refused on a cache miss, and 0 when refused from the instance's
cache. The pricing FAQ (https://upstash.com/docs/redis/overall/pricing,
fetched the same day) charges every command but AUTH, HELLO, SELECT,
COMMAND, CONFIG, INFO, PING, RESET and QUIT; it says nothing about
scripts itself.

**Measured.** `upstashDeps` over a `Redis` whose global `fetch` was a
scratch fake Upstash (SET, GET, DEL, EVALSHA/EVAL with the script's
INCRBY and first-count PEXPIRE; NOSCRIPT on the first EVALSHA, as a fresh
database answers), no network. "Billed" is the REST commands plus the
commands inside the scripts, per the page above:

| Request | Status | REST commands | Inside the scripts | Billed |
|---|---|---|---|---|
| `create`, cold (first script use: NOSCRIPT, then EVAL; every window new) | 200 | 6 | INCR ×3, PEXPIRE ×3 | 12 |
| `create`, warm | 200 | 5 (EVALSHA ×3, SET NX, SET token) | INCR ×3 | 8 |
| poll answer, pending | 204 | 5 (EVALSHA ×3, GET ×2) | INCR ×3 | 8 |
| poll answer, answered | 200 | 4 (EVALSHA ×3, GET) | INCR ×3 | 7 |
| poll offer | 200 | 4 (EVALSHA ×3, GET) | INCR ×3 | 7 |
| `answer` | 204 | 5 (EVALSHA ×3, GET, SET NX) | INCR ×3 | 8 |
| `close` | 204 | 5 (EVALSHA ×3, GET, DEL) | INCR ×3 | 8 |
| a bad request | 400 | 0 | — | 0 |
| a write over its cap, first refusal | 429 | 2 (EVALSHA ×2) | INCR ×2 | 4 |
| the same, refused again by the same instance | 429 | 1 (EVALSHA) | INCR | 2 |

A window's first count adds one PEXPIRE (a new room's poll window, a
client's first request in a window, and so on).

- **Per request**, a served one costs 7 or 8 commands. That is today's
  cost (three `INCR` + `EXPIRE NX` pipelines, 6, plus the room's
  command), not the spec's "about 4": a script call bills its `INCR` as
  well as itself. `create` costs one more than before (the token's SET).
- **A typical pairing**, warm, with the host polling N times (the last
  one answered): `create` 8 + (N − 1) × 8 + 7 + the joiner's offer poll
  7 + `answer` 8 + `close` 8 = 8N + 30, plus a few PEXPIREs. For N = 10
  to 30 that is about 110 to 275 commands, so the free month (500K)
  covers about 1,800 to 4,500 pairings. Without `close`, 8N + 22.
- **At the global cap** (5,000 served requests per 10 min, 7 to 8
  commands each) the month's quota is spent in 500,000 / 40,000 to
  500,000 / 35,000 windows: about 2.1 to 2.4 hours. A request refused at
  the global cap still costs its client's and its room's or writes'
  script calls (4 commands; the global refusal itself comes from the
  instance's cache after the first), so a flood past the cap from many
  clients spends the quota faster still.

**Sizes.** `bun run build`'s client bundle is byte-identical before and
after (`dist/assets/index-C2eKYMur.js`, 276,964 B, both at `4bd7b06` and
with this change): the browser takes only the room-code helpers from
`signal-handler.ts`, `valibot`'s schemas are dropped as unused, and the
`@upstash` packages live only in `api/signal.ts`. So `valibot` costs the
client 0 B today. The function, bundled for Node by `bun build
api/signal.ts --target node --minify` (Vercel bundles it its own way;
this is an estimate): 4,150 B (1,828 B gzipped) before, 128,188 B
(32,211 B gzipped) after, almost all of it `@upstash/redis`.

## What a CPU reset keeps

v392 reboots through `esp_restart()`, which ends in `esp_restart_noos()`
and the ROM's `software_reset_cpu` (`crate::rom` entry 26), a CPU reset:
reset reason `0x0C`, "Software CPU reset", triggered by
`RTC_CNTL_SW_PROCPU_RST` (TRM Table 6.1-1, p. 193; the bit is
`RTC_CNTL_OPTIONS0_REG` bit 5, TRM Register 9.1, p. 233). The badge's
captures show `rst:0xc (RTC_SW_CPU_RST)` (Measurements). ROM
`software_reset` is the core reset: reason `0x03`, "Software system
reset", by `RTC_CNTL_SW_SYS_RST` (Table 6.1-1, p. 193; `OPTIONS0` bit 31,
Register 9.1, p. 233). Task 3 implements this table row by row for the
CPU reset. The core reset keeps today's `soft_reset` path, with Task 2's
changes (spec, Part 1.3).

### Sources

All fetched into the session scratchpad, never into the repo.

- **TRM**: *ESP32-C3 Technical Reference Manual*, Version 1.4
  (`https://www.espressif.com/sites/default/files/documentation/esp32-c3_technical_reference_manual_en.pdf`,
  fetched 2026-11-04; text extracted with `pypdf` 6.19.0, already in
  `local/.venv`). Its printed page numbers equal the PDF's page numbers.
  - **§6.1.1, p. 191**: "All reset types mentioned above (except Chip
    Reset) maintain the data stored in internal memory."
  - **§6.1.3, pp. 191–192**: "CPU Reset: Only resets CPU core." "Core
    Reset: Resets the whole digital system except RTC, including CPU,
    peripherals, Wi-Fi, Bluetooth® LE, and digital GPIOs." Also, p. 192:
    "If CPU is reset, PMS registers will be reset, too." (Figure 6.1-1,
    the scope diagram, is a picture and was not read.)
  - **Table 6.1-1, p. 193**: the reset codes above.
  - **§1.1, p. 31**: "CPU core complex has an interrupt-controller
    (INTC), debug module (DM) and system bus (SYS BUS) interfaces".
    **§1.5.1, p. 42**: the INTC's registers are Chapter 8's "CPU
    Interrupt Registers".
  - **§9.3.3, p. 224**: "any reset / sleep other than power-up reset
    will not stop or reset the RTC timer."
  - **§9.4.1, p. 226**: the RTC power domain holds the "Power management
    unit (PMU), including RTC timer, fast memory, Always-on registers".
  - **§12.2.2.2, p. 308** (WDT timeout actions): "CPU reset – Reset a CPU
    core"; "Core reset – … the main system (which includes MWDTs, CPU,
    and all peripherals) will be reset. The power management unit and
    RTC peripheral will not be reset."
  - **Table 16.3-2, pp. 430–431**: each peripheral's reset bit. "TIMER"
    is `SYSTEM_TIMERS_RST`; "Timer Group0" is `SYSTEM_TIMERGROUP_RST`
    and "Timer Group1" `SYSTEM_TIMERGROUP1_RST`; "SPI0 / SPI1" is
    `SYSTEM_SPI01_RST`. Footnote 3: "Reset registers cannot be cleared by
    hardware. Therefore, SW reset clear is required after setting the
    reset registers."
  - **§2.6.1, p. 67**: GDMA is reset through `SYSTEM_DMA_RST`.
  - **§30.3.4, p. 768**: "the USB Serial/JTAG Controller itself is not
    affected by SRST" (SRST being the JTAG line that causes "a digital
    system reset").
- **ESP-IDF v5.5.3**
  (`https://raw.githubusercontent.com/espressif/esp-idf/v5.5.3/<path>`),
  cited by short name and line:
  - `system_internal.c` = `components/esp_system/port/soc/esp32c3/system_internal.c`
    (`esp_system_reset_modules_on_exit` :31–61, `esp_restart_noos`
    :67–116);
  - `clk.c` = `components/esp_system/port/soc/esp32c3/clk.c`
    (`esp_perip_clk_init` :212–327, called by the app at `cpu_start.c`
    :775);
  - `cpu_start.c` = `components/esp_system/port/cpu_start.c`;
  - `bootloader_esp32c3.c` = `components/bootloader_support/src/esp32c3/bootloader_esp32c3.c`
    (`bootloader_init` :127–194);
  - `bootloader_clock_init.c` = `components/bootloader_support/src/bootloader_clock_init.c`;
  - `bootloader_init.c` = `components/bootloader_support/src/bootloader_init.c`;
  - `bootloader_utility.c` = `components/bootloader_support/src/bootloader_utility.c`;
  - `bootloader_console.c` = `components/bootloader_support/src/bootloader_console.c`;
  - `rtc_clk.c`, `rtc_clk_init.c` = `components/esp_hw_support/port/esp32c3/`;
  - `clk_tree_ll.h`, `gpio_ll.h`, `clk_gate_ll.h` = `components/hal/esp32c3/include/hal/`;
  - `mmu_hal.c` = `components/hal/mmu_hal.c`;
  - `system_reg.h` = `components/soc/esp32c3/register/soc/system_reg.h`;
  - `uart_channel.h` = `components/soc/esp32c3/include/soc/uart_channel.h`;
  - `rom/rtc.h` = `components/esp_rom/esp32c3/include/esp32c3/rom/rtc.h`;
  - `esp_system/Kconfig` = `components/esp_system/Kconfig`;
  - `vfs_console.c` = `components/esp_vfs_console/vfs_console.c`.

### Three facts the rows lean on

1. **The model drops reset-bit writes.** `crate::peripherals::system`
   models only `FROM_CPU_0..3`, `CPU_PER_CONF` and `SYSCLK_CONF`;
   `SYSTEM_PERIP_RST_EN0/1` (`system_reg.h` :320, :508) fall through the
   bus's catch-all, so no peripheral sees them. Every reset the guest
   requests by writing them, in `esp_restart_noos` or later, is a no-op
   today. The spec's rule (Part 1.3): such a peripheral is reset at the
   reset step instead, and recorded.
2. **The app resets SPI2, I2C0 and RMT itself on every boot.**
   `esp_perip_clk_init` (`clk.c` :270–287, comment "Reset the
   communication peripherals like I2C, SPI, UART, I2S and bring them to
   known state") always adds `SYSTEM_SPI2_CLK_EN`, `SYSTEM_I2C_EXT0_CLK_EN`,
   `SYSTEM_RMT_CLK_EN` (and others) to the set it gates off and holds in
   reset (:306–307). It never adds `SYSTEM_USB_DEVICE_CLK_EN` (:289). On a
   CPU reset it otherwise touches only clocks that were already off
   (:219–229). This runs as guest code in the emulator, at `cpu_start.c`
   :775, but its writes are dropped (fact 1), so those three are reset at
   the reset step (ruling R-T1-1).
3. **The 2nd-stage bootloader runs the full clock init after this
   reset.** `bootloader_clock_configure` skips `rtc_clk_init` only for
   `RESET_REASON_CPU0_SW` with the stored APB frequency still at least
   `APB_CLK_FREQ`, 80 MHz (`bootloader_clock_init.c` :55;
   `components/soc/esp32c3/include/soc/soc.h` :140). `esp_restart_noos` has just moved the
   CPU to XTAL (`system_internal.c` :109 → `rtc_clk.c` :322–327, :339–348),
   which stores 40 MHz as the APB frequency, so `rtc_clk_init` runs
   (:83). It leaves the CPU at 80 MHz on the PLL (`rtc_clk_init.c` :63 →
   `rtc_clk.c` :186–192), `STORE4` = the XTAL value (`rtc_clk_init.c`
   :52), and `STORE5` = 80 MHz (`rtc_clk.c` :191). The same code the
   shortcut boot already stands in for at a cold boot (`crate::rom`,
   `esp32c3_rom_ram_initializers`, items 3 to 5).

The mask ROM's own reset path is not published. Where a row says nothing
of the ROM, neither the TRM nor ESP-IDF documents a ROM effect on it.

### The table

"Kept" means Task 3 moves the old `FirmwareBus` field into the new bus by
its concrete name. "Reset" means the new boot's fresh field, as today.
Field names are `FirmwareBus`'s (`emulator-core/src/mem/bus.rs`).

| # | Peripheral (`FirmwareBus` field) | CPU reset: kept or reset | Citation | Reproduced how (Task 3) |
|---|---|---|---|---|
| 1 | SYSTIMER (`systimer`) | **Reset** | `system_internal.c` :52–54 pulses `SYSTEM_SYSTIMER_RST` (`system_reg.h` :335, bit 29; TRM Table 16.3-2, p. 430) | Fresh `systimer`, and `clock` restarts at 0 (spec, "My calls") |
| 2 | INTC (`intc`) | **Silent → reset** | MAP half: an inference from §6.1.3 ("Only resets CPU core"; the MAP registers are outside it), which the TRM does not state about them; the app rewrites the MAP of every source below `ETS_MAX_INTR_SOURCE` before interrupts are on (`cpu_start.c` :782 → :171–195). CPU-line half (enable, type, priority, threshold): part of the "CPU core complex" (§1.1, p. 31); whether a CPU reset clears it, the TRM does not say | Fresh `intc`, as today (ruling R-T1-2) |
| 3 | SYSTEM (`system`) | **Kept, with `SYSCLK_CONF` and `CPU_PER_CONF` rewritten as the bootloader does** | Kept: §6.1.3. Rewritten: fact 3; `clk_tree_ll.h` :442–447 (`CPUPERIOD_SEL` = 0), :481–485 (`PRE_DIV_CNT` = 0), :398–406 (`SOC_CLK_SEL` = 1) | Move `old.system`; then set `SYSCLK_CONF`'s `SOC_CLK_SEL` [11:10] = 1 and `PRE_DIV_CNT` [9:0] = 0 (`system_reg.h` :803–814), and `CPU_PER_CONF`'s `CPUPERIOD_SEL` [1:0] = 0 (:61–66), fields only. `FROM_CPU_0..3` stay as they were |
| 4 | APB_CTRL RNG (`apb_ctrl`) | **Kept** | §6.1.3; no reset bit is pulsed (`system_internal.c` :31–61); the app's `periph_module_enable(PERIPH_RNG_MODULE)` (`clk.c` :326) has no reset bit (`clk_gate_ll.h` :87–162, `default: return 0`) | Move `old.apb_ctrl`: the xorshift sequence continues (ruling R-T1-5) |
| 5 | GPIO (`gpio`, without the 74HC165) | **Kept, with GPIO21's output enable cleared as the bootloader does** | Kept: §6.1.3 (only a core reset resets "digital GPIOs"). `esp_restart_noos` writes `GPIO_FUNC0..5_IN_SEL_CFG` = `0x30` (`system_internal.c` :98–103), which the model does not hold. Rewritten: `bootloader_console.c` :28–34 (`release_default_console_io`, called at :138 for a USB-Serial-JTAG console, :41 for none) → `gpio_ll_output_disable(&GPIO, 21)` = `enable_w1tc = 1 << 21` (`gpio_ll.h` :233–236; `uart_channel.h` :14) | Move `old.gpio`; then clear `ENABLE` bit 21, as a `GPIO_ENABLE_W1TC_REG` write of `1 << 21` does (ruling R-T1-4) |
| 6 | 74HC165 (`gpio.hc165`) | **Kept** | External chip, outside the SoC: no reset in the TRM's scope reaches it | Moves with `gpio`; held buttons re-applied, as today |
| 7 | SPI2 (`spi`, without the panel) | **Reset** | Fact 2: `clk.c` :278, :306–307 (`SYSTEM_SPI2_RST`, `system_reg.h` :473, bit 6) | Fresh `spi`, as today |
| 8 | ST7789 panel (`spi.st7789`) | **Kept** | External chip (already kept since Milestone 9, `runtime.rs` module doc) | As today: `spi.st7789 = old.spi.st7789` |
| 9 | GDMA (`gdma`) | **Reset** | `system_internal.c` :58–60 pulses `SYSTEM_DMA_RST` (`system_reg.h` :529, bit 6; TRM §2.6.1, p. 67) | Fresh `gdma`, as today |
| 10 | TIMG0, TIMG1 (`timg0`, `timg1`) | **Kept** | §6.1.3; §12.2.2.2, p. 308. `esp_restart_noos` only disables both MWDTs (`system_internal.c` :82–91), as guest writes the model stores. Its `SYSTEM_TIMERS_RST` (:53) is not the timer groups (Table 16.3-2, p. 430: `SYSTEM_TIMERGROUP_RST`, `SYSTEM_TIMERGROUP1_RST`; `system_reg.h` :503 vs :431, :419). `esp_perip_clk_init`'s always-reset set leaves them out (`clk.c` :271–287) | Move `old.timg0` and `old.timg1` (ruling R-T1-6) |
| 11 | RTC_CNTL (`rtc_cntl`) | **Kept, with `STORE4` and `STORE5` rewritten as the bootloader does** | Kept: §6.1.3 ("except RTC"); §9.3.3, p. 224 (the RTC timer); §9.4.1, p. 226 (always-on registers). Rewritten: fact 3; `STORE4` = `clk_ll_xtal_store_freq_mhz(40)` (`rtc_clk_init.c` :52; `crate::rom::BOOTLOADER_RTC_XTAL_FREQ_REG_VALUE`); `STORE5` = `clk_ll_apb_store_freq_hz(80 MHz)` (`rtc_clk.c` :186–192; `clk_tree_ll.h` :665–669; `rom/rtc.h` :53, :63) = `0x4C4B_4C4B` | Move the whole `old.rtc_cntl` (every stored word, not only `STORE0..7`), keeping Task 2's timer offset; then write `STORE4` (Task 2) and `STORE5` (ruling R-T1-3) |
| 12 | USB-Serial-JTAG (`usb_serial_jtag`) | **Kept**, queued host input included | §6.1.3; §30.3.4, p. 768. `esp_restart_noos` pulses no `SYSTEM_USB_DEVICE_RST` (`system_internal.c` :52–60; bit 23, `system_reg.h` :371) and keeps the BBPLL on "so that USB Serial JTAG can log at 1st stage bootloader" (:107). The app never resets it on a CPU reset (`clk.c` :289; the pad-off branch :260–267 is the non-CPU-reset path). The badge's port stayed open (Measurements) | Move `old.usb_serial_jtag` whole: registers, `INT_RAW`, the host queue, the OUT FIFO and the packet pacing |
| 13 | I2C0 (`i2c0`, without the accelerometer) | **Reset** | Fact 2: `clk.c` :279, :306–307 (`SYSTEM_I2C_EXT0_RST`, `system_reg.h` :467, bit 7) | Fresh `i2c0` |
| 14 | SC7A20H (`i2c0.accel`) | **Kept** | External chip, outside the SoC | Move `old.i2c0.accel` into the fresh `i2c0` (its registers, `CTRL_REG1` and the rest, survive); the host acceleration is re-applied, as today |
| 15 | RMT (`rmt`) | **Reset** | Fact 2: `clk.c` :281, :306–307 (`SYSTEM_RMT_RST`, `system_reg.h` :455, bit 9) | Fresh `rmt`, as today |
| 16 | eFuse (`efuse`) | **Kept** | §6.1.3; no reset bit is pulsed (`system_internal.c` :31–61); the model holds only the host-set factory MAC | As today: the MAC is re-applied (equivalent to moving `old.efuse`) |
| 17 | Flash MMU (`mmu`) | **Kept, with every entry rewritten as the bootloader does** | Kept: §6.1.3. Rewritten: `mmu_hal_init` (`bootloader_esp32c3.c` :164) → `mmu_hal_unmap_all` (`mmu_hal.c` :18–26, :28–39); `set_cache_and_start_app` (`bootloader_utility.c` :1036) unmaps all again (:1064) and maps the app's DROM, entry 127 and IROM (:1082, :1086, :1105). `esp_restart_noos` disables the ICache (`system_internal.c` :94) | Fresh `mmu`, as today: `FirmwareBus::seed_mmu_like_bootloader` already writes exactly the bootloader's table. Mappings the old app added at run time go, as on the badge |
| 18 | SPI1 controller (`spimem1`) | **Reset** | `system_internal.c` :52–54 pulses `SYSTEM_SPI01_RST` (`system_reg.h` :497, bit 1; TRM Table 16.3-2, p. 430, "SPI0 / SPI1"). The bootloader then sets it up again (`bootloader_init_spi_flash`, `bootloader_esp32c3.c` :181) | Fresh `spimem1`; the shortcut boot's ROM `.data` seeds stand for the bootloader's flash setup, as at a cold boot (`crate::rom`, `esp32c3_rom_ram_initializers` items 1 and 2) |
| 19 | Flash chip (`flash_chip`) | **Kept** | External chip | As today: `flash_chip = old.flash_chip` |

The `console` bus field has no row: it is the emulator's own output sink
(`crate::peripherals::console`), not a chip's state, so it stays as it was
(kept by both kinds, since Milestone 9).

**Totals:** 8 kept (rows 4, 6, 8, 10, 12, 14, 16, 19), 4 kept with
registers rewritten (rows 3, 5, 11, 17), 6 reset (rows 1, 7, 9, 13, 15,
18), 1 silent → reset (row 2).

### Each row's three parts

The core-reset scope is the same for every on-chip row: TRM §6.1.3,
"Resets the whole digital system except RTC, including CPU, peripherals,
Wi-Fi, Bluetooth® LE, and digital GPIOs", and §12.2.2.2, p. 308. So every
digital peripheral is reset by a core reset, and RTC_CNTL is kept.
External chips are outside every SoC reset. The spec keeps the core reset
on today's path (Part 1.3); the two places where today's path is narrower
than the TRM are in the Milestone 12 backlog.

1. **SYSTIMER.** CPU reset: not the CPU core, so the reset itself keeps
   it (§6.1.3). `esp_restart_noos`: resets it (`system_internal.c`
   :52–54). Bootloader: nothing. Choice: reset.
2. **INTC.** CPU reset: the interrupt-matrix MAP registers are a
   peripheral (Chapter 8), so kept, by inference from §6.1.3; the TRM
   says nothing about them specifically. The CPU interrupt registers belong to
   the "CPU core complex" (§1.1, p. 31), and §6.1.3 says only that a CPU
   reset "only resets CPU core"; neither the TRM nor ESP-IDF says which
   side of that line they fall on. `esp_restart_noos`: turns interrupts
   off in `mstatus` (`system_internal.c` :70), a CPU register. The app:
   `core_intr_matrix_clear` routes every source below
   `ETS_MAX_INTR_SOURCE` to `ETS_INVALID_INUM` (`cpu_start.c` :171–195,
   called at :782, before the scheduler), so a kept MAP is overwritten
   before it can route anything. Choice: silent → reset.
3. **SYSTEM.** CPU reset: kept. `esp_restart_noos`:
   `rtc_clk_cpu_set_to_default_config` writes `SYSCLK_CONF` for XTAL,
   divider 1 (`system_internal.c` :109; `rtc_clk.c` :322–327, :339–348),
   which runs as guest code. Bootloader: back to 80 MHz on the PLL (fact
   3). `FROM_CPU_0..3` (`system_reg.h` :616–640): nobody writes them.
   Choice: kept, with the clock fields rewritten.
4. **APB_CTRL.** CPU reset: kept. Nothing resets or rewrites the RNG
   register. Bootloader: `bootloader_enable_random` (`bootloader_esp32c3.c`
   :191) turns on an entropy source, not modeled. Choice: kept.
5. **GPIO.** CPU reset: kept; a core reset resets "digital GPIOs".
   `esp_restart_noos`: the six `FUNCn_IN_SEL_CFG` writes (unmodeled, so
   nothing to carry). Bootloader: `release_default_console_io` on the
   USB-Serial-JTAG or no-console build. Which build v392 is: its
   `firmware.bin` holds `"/dev/uart/-1"` at file offset `0x48B8`, the
   `console_access` path `"/dev/uart/" STRINGIFY(CONFIG_ESP_CONSOLE_UART_NUM)`
   (`vfs_console.c` :140), and `CONFIG_ESP_CONSOLE_UART_NUM` is `-1`
   exactly when the console is not a UART (`esp_system/Kconfig` :348–353).
   The remaining choices are USB CDC (ESP32-S2 and S3 only, :283–286), USB
   Serial/JTAG (:287–289) and none (:292–293); the first is impossible on
   this chip, and both others call `release_default_console_io` (:138,
   :41). GPIO21 is `HC165_CLK` on the badge (`crate::peripherals::gpio`'s
   pin map). Its `gpio_ll_func_sel` (IO_MUX) and
   `esp_rom_gpio_connect_in_signal` (`FUNCn_IN_SEL`) parts touch
   registers the model does not hold. Choice: kept, `ENABLE` bit 21
   cleared.
6. **74HC165.** External; no SoC reset reaches it. Choice: kept.
7. **SPI2.** CPU reset: kept by the reset itself. `esp_restart_noos`:
   nothing. The app: reset on every boot (fact 2). Choice: reset.
8. **ST7789.** External. Choice: kept (unchanged since Milestone 9).
9. **GDMA.** CPU reset: kept by the reset itself. `esp_restart_noos`:
   resets it (`system_internal.c` :58–60). Choice: reset.
10. **TIMG0, TIMG1.** CPU reset: kept. `esp_restart_noos`: MWDT0 and
    MWDT1 disabled (guest writes). Bootloader: after the reset the
    flash-boot protection of MWDT0 is on and the bootloader clears it
    (`bootloader_init.c` :73–74, :95–99); the model's watchdog words are
    inert storage, so this is not reproduced. The app: TIMG0 and TIMG1
    are outside the always-reset set; on a CPU reset a peripheral is
    reset only if its clock was already off (`clk.c` :219–229, :306–307),
    and the model has no clock gates (ruling R-T1-6). Choice: kept.
11. **RTC_CNTL.** CPU and core reset: kept (RTC domain). The RTC timer
    keeps counting (§9.3.3), which is Task 2's offset. `STORE2`/`STORE3`
    hold the boot time (`rom/rtc.h` :50–51, :60–61), so the app's time of
    day carries over with them, as on the badge. `esp_restart_noos`: arms
    the RTC watchdog for 1 s (`system_internal.c` :71–80) and writes
    `STORE5` = 40 MHz through `rtc_clk_cpu_set_to_default_config` (fact
    3). Bootloader: `STORE4`, `STORE5` (fact 3), plus RTC clock tuning
    and source words (`rtc_clk_init.c` :36–43, :78–80), the RTC watchdog's
    flash-boot bit (`bootloader_init.c` :78–82) and the super watchdog's
    auto-feed (`bootloader_esp32c3.c` :79–84). The model keeps all words
    other than the timer and `STORE4` as plain storage; of the
    bootloader's writes only `STORE4` and `STORE5` are reproduced
    (ruling R-T1-7). Choice: kept, `STORE4` and `STORE5` rewritten.
12. **USB-Serial-JTAG.** CPU reset: kept. Neither `esp_restart_noos` nor
    the app resets it (row citation). Bootloader: `bootloader_console_init`
    for this console only switches the ROM's output channel
    (`bootloader_console.c` :133–149). Choice: kept, with its queued host
    input, which closes the provisioner's stale-prompt item one way:
    a provision typed across a reboot reaches the new console (Part 4.10
    still scans only text written after the provision started).
13. **I2C0.** As SPI2: the app resets it on every boot (fact 2). Choice:
    reset.
14. **SC7A20H.** External. Choice: kept.
15. **RMT.** As SPI2 (fact 2). Choice: reset.
16. **eFuse.** CPU reset: kept. Nothing rewrites it. Choice: kept.
17. **Flash MMU.** CPU reset: kept by the reset itself. Bootloader:
    unmaps every entry and maps the app (row citation). Choice: kept,
    every entry rewritten, which is the cold boot's seed.
18. **SPI1.** `esp_restart_noos`: resets SPI0 and SPI1
    (`system_internal.c` :52–54). Bootloader: configures it for the
    flash again. Choice: reset.
19. **Flash chip.** External. Choice: kept (unchanged since Milestone 9).

### Not peripherals, for completeness

- **The CPU core** (registers, CSRs, the cycle counter, PMS): reset
  (§6.1.3, and p. 192 for PMS). A fresh `Cpu`, as today.
- **Internal SRAM:** the TRM keeps it (§6.1.1, p. 191). The bootloader
  reloads the image's loaded segments, and the app zeroes `.bss`
  (`cpu_start.c` :424–449). The emulator starts every other RAM byte at
  0, as today, so anything the firmware keeps in a DRAM `.noinit`
  section would be lost. Not seen; recorded for the Milestone 12 backlog.
- **RTC fast memory:** kept, as today (RTC domain, §9.4.1).

## Moved pins

Tasks 2 and 3 record each move here (rung | constant | before | after |
cause).

**Task 2: no pin moved.** All 87 rungs of the release `boot_progress` pass
unchanged, and no constant or frame hash was re-pinned. The step budgets
have about 25 % of slack, so a pass alone would not show a small move. A
scratch probe therefore measured each app that opens by rebooting, at the
commit before Task 2 (`0e393c3`) and with Task 2's change:

| Rung's end state | Steps (before = after) | Frame hash (before = after) | Emulated time before (µs) | After (µs) | Delta (µs) |
|---|---|---|---|---|---|
| Scanner open | 395,720,369 | `SCANNER_OPEN_HASH` | 107,089,383 | 107,555,382 | +465,999 |
| Sync's home | 405,128,568 | `SYNC_HOME_HASH` | 106,947,376 | 107,413,376 | +466,000 |
| Share open | 375,750,142 | `0xfc51_aedc_c0ab_617d` | 103,413,941 | 103,879,941 | +466,000 |
| Sponsors loaded | 467,775,337 | `SPONSORS_LOADED_HASH` | 117,733,891 | 118,199,890 | +465,999 |
| Workshop Quest open | 430,683,605 | `0x894d_5dae_e0d9_8e2a` | 113,654,025 | 114,120,024 | +465,999 |

HOME's response is also unchanged, measured from the release to the first
stable `LAUNCHER_HASH`: Scanner 11.75M steps, Share 12.25M, Sponsors
12.0M, Workshop Quest 12.25M. Their end times move by the same +465,999
or +466,000 µs.

Three of the five deltas are 1 µs short of `REBOOT_US`, with identical
step counts. The likely cause is that some idle fast-forward in the new
boot ended up shorter by under a microsecond, once the guest had read the
RTC timer's new value; this is a hypothesis, not traced. Task 3's
re-measurement leaves every one of these numbers exactly as they are. No pin reads
time at that precision after a reboot.

One unit test changed, for the recorded cause (not a pin):
`emulated_time_is_monotonic_across_a_soft_reset` set a `run_until` target
50 ms past the reset, which now lies inside the reboot's jump. Its target
is now `REBOOT_US` + 50 ms past the reset, and it still checks that the
run stops at the target.

**Task 3: no pin moved.** The CPU reset now keeps SYSTEM, the RNG, GPIO
and the 74HC165, TIMG0/1, RTC_CNTL, USB-Serial-JTAG and the SC7A20H's
registers (the table above), and all 87 rungs of the release
`boot_progress` pass unchanged; no constant or frame hash was re-pinned. A
scratch probe (deleted before the commit) measured the same end states as
Task 2's table, first at `559cc57` and then with this change, and every
value is identical: steps, frame hash, emulated time, the RTC timer's
offset, and the release-to-stable step counts of the open app and of HOME
back to the launcher. A temporary log in `soft_reset` confirmed the probe
exercised the change: all six resets on its walks are `ResetKind::Cpu`, and
the kept state is live at each (for example GPIO `ENABLE` `0x0030_047e`, the
RNG mid-sequence, `SYSCLK_CONF` at XTAL before the rewrite). What the new
boot reads from the kept peripherals changes no step count, time or frame on
these walks. Sync's HOME is the probe's second reset, and is also unchanged.

One unit test changed, for the recorded cause (not a pin):
`console_output_survives_a_soft_reset` asserted that a CPU reset dropped
queued serial input; it now asserts that it stays (row 12).

## Traces

Both traces were made on v392 at `fd96f49`, with scratch probes outside
the repo, since deleted. The firmware image has no symbols. Below,
**proven** means a string or a measurement in this section shows it;
**inferred** means the name comes from matching code shape against the
public source, and is labelled.

Ruling R-T4-1: `PROV FAIL`'s busy steps are the firmware's own work, and
no emulator change follows — cost if wrong: a slow redraw would hide an
emulator effect, but the trace found no MMIO spin and no new unmapped
access. SPI transfers are instant in this model, so the ruling covers the
redraw's CPU work only. The gap's zero idle time and the console's position
relative to the redraw are not hardware claims.

Ruling R-T4-2: both provisioning hosts are valid, neither changes, and
`provisioner_pacing_matches_the_rust_host` pins their shared schedule —
cost if wrong: only test-walk step counts differ, and every pinned frame
matches on both.

### `PROV FAIL`'s 3.0M busy steps: firmware behaviour, not ours

**Result.** The steps are a full-screen LVGL redraw on the firmware's
`taskLVGL`. In the emulator it runs ahead of the console in the middle of
the result line. The redraw repaints identical pixels.

**What this does and does not say about hardware.** The steps are guest
LVGL code, and instant SPI adds none of them, so the redraw's CPU work is
the finding. The eight flushes go through SPI2, and a transfer here takes
no emulated time (`docs/firmware-emulator-notes.md`: "a transfer is
instant at `SPI_USR`"; `peripherals/spi.rs`'s module doc). So `taskLVGL`
(priority 4) may never wait on a flush here. Whether the firmware's flush
wait blocks or spins was not established. On hardware, a blocking wait
could let the console task finish its line during the redraw. The zero
idle time, the uninterrupted console gap and its 35,293 µs below describe
this model, not the badge.

**How it was measured.** A clone of `first_run_screen()`, then
`provision::put_file` with the payload `{}`, then `prov apply\n`, stepping
in slices of 1 to 500 steps and sampling `pc`, `ra`, `sp`, the panel
window, `idle_ticks`, `transaction_count` and the unmapped log. Emulated
time is `emulated_time_us()`.

| Step | Emulated µs | Event |
| --- | --- | --- |
| 15,900,000 | 1,437,136 | `prov apply` typed |
| 15,978,329 | | REPL task takes `lvgl_port_lock` (`ra` `0x4200c054`) |
| 15,978,482 | | identity load `0x4200eb1c` starts |
| 16,239,137 | | REPL task calls `lvgl_port_unlock` (`ra` `0x4200c060`) |
| 16,257,250 | 1,441,602 | console has printed `PROV FAIL ` (10 bytes) |
| 16,260,737 | | `taskLVGL` takes `lvgl_port_lock` (`ra` `0x420a32a4`) |
| 19,078,809 | | `taskLVGL` calls `lvgl_port_unlock` (`ra` `0x420a32b6`) |
| 19,080,750 | 1,476,895 | the console's next byte (`invalid or missing …`) |

The silent gap in the console is 2,823,500 steps and 35,293 µs. The lock
is held for 2,818,072 of those steps. Milestone 9's "3.0M after the
result line" is this gap, plus the rest of the line and the error return,
sampled on a coarse grid: the next prompt was seen by 19,357,500 at
100,000-step sampling.

**In the gap:**

- `idle_ticks` grows by 0 (in this model; see the caveat above), and the
  flash controller makes 0 transactions.
- The unmapped log is identical before and after, so there is no new
  unmapped access.
- The panel window moves through eight 30-line strips, `(0,319,0,29)` to
  `(0,319,210,239)`: the whole 320x240 screen.
- The framebuffer afterwards equals the one before.
- Sampled `sp` values lie in `0x3fcbe5f0..0x3fcbf0c0`, inside `taskLVGL`'s
  stack. Its TCB, found by name at `0x3fcbb06c`, gives `pxStack`
  `0x3fcbb19c` and priority 4.

**Hot code** (2,823,500 traced steps; 87.1 % in `0x420a3200..0x420e2000`,
LVGL and its port; 5.6 % in IRAM):

| Address | Share | What it is |
| --- | --- | --- |
| `0x420b5fd6..0x420b6300` | 13.2 % | Glyph expansion to A8. Inferred: LVGL's `lv_font_get_bitmap_fmt_txt`. Evidence: indexes the tables at `0x3c241f7c` (`0, 85, 170, 255`) and `0x3c241f80` (`0, 17, …, 255`), matching its `opa2_table` and `opa4_table`. |
| `0x420b5b38..0x420b5c60` | 10.9 % | RGB565 byte swap of the draw buffer, in 8-word blocks. Inferred: `lv_draw_sw_rgb565_swap`, from the code shape. |
| `0x420c14b6..0x420c14e0` | 3.1 % | Fill: 32-byte blocks, then a byte tail. Inferred: LVGL's `lv_memset`, from the code shape. |
| The rest | | Spread thin over LVGL's draw code; not named one by one. |

These loops load from RAM and flash. None of them polls a peripheral.

**Names proven by strings:** `0x420a30e4` is `lvgl_port_lock` and
`0x420a314a` is `lvgl_port_unlock`. Each one's assert passes its function
name (`0x3c241a40`, `0x3c241a2c`) and `esp_lvgl_port.c` (`0x3c1496fd`).

**Why it redraws.**

1. The `prov apply` handler (the code around `0x4200c050`; it references
   `PROV FAIL invalid or missing %s\n` at `0x3c134e70`) calls `0x4200eb1c`
   under the LVGL lock.
2. `0x4200eb1c` clears the identity struct at `0x3fca3310` and reads
   `/littlefs/identity.json` (`0x3c134e58`).
3. On its result paths it adds 1 to the word at `0x3fcb5d70`, a generation
   counter, through `0x403975b4` (arguments `(addr, 1, 5)`; the body is a
   critical-section fetch-add; inferred: `__atomic_fetch_add_4`, with 5 as
   `__ATOMIC_SEQ_CST`). The sites in this function are `0x4200eb72`
   (success), `0x4200ec0e` (file missing) and `0x4200ec3e` (parse
   failure), so a failed apply bumps it too. `0x4200f00e` is another
   site, outside this path.
4. `0x4200f3d4` reads the counter. `0x42015398` compares it with a saved
   copy and, on a mismatch, calls `0x4201521c`, which rebuilds the screen.
   Inferred: this is My Badge. Unprovisioned, `0x4201521c` calls
   `0x42014f64`, which references `Not registered yet` (`0x3c1374c0`) and
   sets all six LEDs white. The neighbouring `0x42014d08` references
   `Badge ID: %s` (`0x3c137488`).
5. `0x42014f64` was observed running on `taskLVGL`'s stack (a PC at
   `0x42014f90` at step 16,323,234). Its only direct caller is
   `0x4201525a`, inside `0x4201521c`. That `0x42015398` was the path taken
   is inferred: `0x4201521c` has six direct callers, and the probe did not
   record which one ran.

**Controls, each from the same first-run checkpoint, over 5,000,000 steps
each:**

| Input | Panel window changes | Steps on `taskLVGL`'s stack (100-step sampling) |
| --- | --- | --- |
| None | 0 | 312,900 |
| `prov show` | 0 | 312,400 |
| An empty line | 0 | 310,300 |
| `prov apply`, no file | 8, the first at step 16,999,900 | 2,918,100 |

So the redraw follows `prov apply`, and not `prov show` or an empty line.
With no file, `PROV FAIL` was seen by step 15,800,000 and the prompt by
16,000,000 (50,000-step sampling), and the redraw came after both. In the
`{}` case the redraw delays the prompt, and in the no-file case it does
not. Inferred, not traced: that ordering comes from task scheduling, and
instant SPI transfers may shape it (see the caveat above).

**Not established:** why v335 showed 0.2M here. v335 was not re-traced.
Whether v335 lacks this rebuild, or only scheduled it after the prompt
(as the no-file case above does on v392), is open, and instant SPI could
bear on the second case.

`boot_progress.rs`'s doc on `CONSOLE_DEADLINE` still says "not traced";
no constant changes for this.

### The two provisioning hosts' step difference: both valid

**Result.** The first difference is 2,000 steps, at Chunk 1 in the
table below (the payload's second 64-byte chunk). The bulk of the difference is that the Rust host waits for the
prompt after `PROV OK` and the frontend does not.

**How it was measured.** Each input was logged as (total steps, emulated
µs, event) by wrapping `serial_input` / `serialInput`,
`set_raw_button` / `setRawButton` and the acceleration setter:

- Rust: a scratch copy of `boot_progress.rs`'s walk to
  `launcher_for("hacker")`. It ends at 342,600,776 steps, equal to the
  pinned `HACKER_LAUNCHER_STEPS`.
- Frontend, twin schedule: `test/cpu-wasm.test.ts`'s walk over the WASM
  handle (`provisioner.poll()` every 50,000 steps), where Milestone 7's
  number came from. It ends at 341,368,630 steps on `LAUNCHER_HASH`.
- Frontend, browser schedule: `createFirmwareRuntime` with
  `WORKER_STEPS_PER_SLICE` (400,000) and a fake clock of 2, 8 and 20 ms
  per slice, through `PROV OK` only.

Milestone 7's 740,001 was measured on v335. On v392 the walks are
1,232,146 steps apart. 740,001 was not re-measured.

| Input | Rust: step (µs) | Twin: step (µs) |
| --- | --- | --- |
| `put` command | 15,500,000 (1,427,134) | 15,500,000 (1,427,134) |
| Chunk 0 | 15,850,000 (1,451,785) | 15,850,000 (1,451,785) |
| Chunk 1 (first difference) | 15,898,000 (1,452,385) | 15,900,000 (1,452,410) |
| Chunk 2 | 15,946,000 (1,461,676) | 15,950,000 (1,462,162) |
| Chunk 3 | 15,994,000 (1,471,799) | 16,000,000 (1,477,138) |
| Chunk 4 (39 bytes) | 16,042,000 (1,491,653) | 16,050,000 (1,491,738) |
| `prov apply` | 16,192,000 (1,512,164) | 16,200,000 (1,521,656) |
| Host finished | 28,542,000 (2,282,145) | 27,300,000 (1,662,006) |
| First press | 36,542,000 (6,821,813) | 35,300,000 (6,101,690) |

- **Chunk pacing.** Both send 64-byte chunks. The Rust host runs exactly
  48,000 steps between them. The frontend sends on its first poll at least
  48,000 steps after the last chunk, so the twin's 50,000-step poll grid
  makes its gaps 50,000. The browser schedule's gaps were 53,190 to
  92,981 steps across the three clocks. All are above the 24,000-step
  threshold measured in Milestone 5 and Milestone 6.
- **First-prompt wait.** The same in both: the first `put` goes out at
  step 15,500,000.
- **The end of provisioning.** The Rust `provision()` returns only after
  the prompt that follows `PROV OK`, because a test may type more. The
  frontend's provisioner is done at the complete `PROV OK` line. That is
  1,242,000 steps and 620,139 µs apart at each host's sampling (the Rust
  host on its 50,000-step wait, the twin on its 50,000-step poll).
- **The press schedule.** The same shape: each press comes after the
  stable-frame wait, 8,000,000 steps after each host's start. The offset
  of 1,242,000 at the first press shrinks to 1,232,146 by the end. The
  hold of a press is a fixed emulated time, so its step count depends on
  the phase (for example 2,632,588 against 2,642,375 for the first). The
  remaining 9,854 steps were not attributed press by press.

Neither host is wrong against the firmware: every pinned frame matches
on both. The frontend never types after `PROV OK`, so it does not need
that prompt. `frontend/test/provisioner.test.ts`'s
`provisioner_pacing_matches_the_rust_host` reads the Rust host's source
and pins the chunk size (64), the gap (48,000) and the frontend's sending
at exactly that gap and not one step before.

## Dependencies added

- `@types/bun` 1.4.4 (dev, frontend, Task 5): the type declarations for
  `bun:test` and `node:fs`, so `test/cpu-wasm.test.ts` can join
  `tsconfig.json`'s `include`.
- `proptest` =1.12.0 (dev, `emulator-core`, Task 6): generated inputs and
  shrinking for the HCI and serial fuzz tests, in place of the two
  hand-rolled xorshift generators (spec, Part 7). `default-features =
  false, features = ["std"]`: no `fork` or `timeout`, so no `rusty-fork`
  or `tempfile`. A dev-dependency: neither the WASM build nor
  `emulator-wasm`'s dependency tree has it.
- `@upstash/redis` 1.41.0 (frontend, Task 9): the Upstash REST client, in
  place of the hand-written `UpstashStore` (spec, Part 5.3, Part 7);
  `@upstash/ratelimit` needs it. Brings `uncrypto` 0.1.3. Imported only by
  `api/signal.ts`, never by the browser bundle. Built with
  `automaticDeserialization: false` (values verbatim), `enableTelemetry:
  false` and `enableAutoPipelining: false`.
- `@upstash/ratelimit` 2.2.0 (frontend, Task 9): the four fixed-window
  limiters (`Ratelimit.fixedWindow`), in place of the hand-written
  `INCR`/`EXPIRE NX` counters (spec, Part 5.3; the human partner's answer
  (c) of 2026-11-03). Brings `@upstash/core-analytics` 0.0.10, unused
  (analytics off). Imported only by `api/signal.ts`.
- `valibot` 1.5.0 (frontend, Task 9): the request schemas
  (`v.safeParse`) in `signal-handler.ts`, in place of hand-written type
  guards (spec, Part 7). A failure is a 400, never a throw. The client
  bundle does not carry it today (tree-shaken; "The signaling function's
  cost").
- `happy-dom` 21.0.4 (dev, frontend, Task 12): a real DOM for
  `test/shell.test.ts`, in place of its hand-written `FakeElement` (spec,
  Part 7), so the shell and the badge controller are tested through the
  page's own skeleton (`index.html`'s body, `shell.css` inline). One
  `Window` per test, set as `globalThis.document` and restored after; no
  global registrator. Brings `entities` 7.0.1, `whatwg-mimetype` 3.0.0,
  `ws` 8.22.2, `buffer-image-size` 0.6.4 and the types
  `@types/whatwg-mimetype` 3.0.2 and `@types/ws` 8.18.2 (`@types/node` was
  already there). Tests only: nothing under `src/` imports it.

## Closed in writing

Each closed without work by the human partner's decision of 2026-11-03,
as the spec records it ("Decided by the human partner (2026-11-03)"),
unless another date is given.

- **The block cache, and with it the cheaper hook check**
  (`docs/milestone-10-decisions.md`, R-T3-5; Milestone 8's shelved lead,
  R-M8-1). Closed: speed needs no work, it is normal in the prod build.
- **Mobile Safari's 0.0x–0.2x.** Closed: a dev-build effect (same
  decision).
- **Re-measuring desktop Chrome and Safari (and the dev bench page) on
  v392.** Closed (same decision).
- **The decode cache's untagged size** (`1 << 16` against `1 << 14`,
  Milestone 7). Closed (same decision).
- **Tuning `WORKER_STEPS_PER_SLICE`** (Milestone 7). Closed (same
  decision).
- **The unverified Vercel build of `frontend/api/signal.ts`** (R-T10-3).
  Closed: the human partner checked that the Vercel preview deploy works.
  R-T10-3's "imports nothing" is revisited by Part 5.3, and the new
  dependencies are re-checked on the next preview deploy (Gate 3).
- **Upstash's plan limits** (R-T10-8). Checked: the free plan, measured
  above ("Upstash's free plan").
- **No v392 frame compared with hardware**, and **v392's early log
  timestamps compared with no badge.** Closed: reflashing the badge to
  v392 is dismissed; the badge stays on v335.
- **The tap queue's lag on fast bursts (`RELEASE_GAP_US`).** Closed:
  `RELEASE_GAP_US` stays 40 ms (R-M8-4 stands; the human partner called
  the lag tiny; spec Part 4.9).
- **Hearing real badges.** Stays dropped (ruling R-T11-2, the human
  partner's no-go of 2026-11-02; spec, "Non-goals").

## Rulings

- Ruling R-T1-1: the app's own `esp_perip_clk_init` resets of SPI2, I2C0
  and RMT (`clk.c` :270–287, :306–307) are reproduced at the reset step,
  like `esp_restart_noos`'s, because the model drops
  `SYSTEM_PERIP_RST_EN0/1` writes. The spec's facts list (copied above)
  had these three as kept. — cost if wrong: if the badge kept them, the
  new boot's drivers would read reset values instead of the old boot's;
  they program every register they use before using it, and none of the
  three is touched between the reset and `cpu_start.c` :775.
- Ruling R-T1-2: INTC is reset whole (silent → reset). The MAP half,
  kept by inference from §6.1.3, is rewritten by the app's `core_intr_matrix_clear` before
  interrupts are enabled, and for the CPU-line half the TRM is silent. —
  cost if wrong: a CPU-line enable, type or priority the old boot left
  would differ from the badge's until the new boot allocates that line.
- Ruling R-T1-3: `STORE5` is rewritten with the bootloader's
  `0x4C4B_4C4B` (80 MHz), not only `STORE4`. The carried value is the
  40 MHz that `esp_restart_noos` stored a step before the reset, which the
  badge's bootloader always overwrites (fact 3). It applies to both reset
  kinds, since the bootloader runs after both, so Task 2's test
  `store4_is_the_bootloaders_after_a_soft_reset` should except `STORE5`
  from "keep the guest's values" if the human partner accepts this row.
  A cold boot is unchanged (it leaves `STORE5` at its reset value 0, as
  today), because changing it would move pins before the launcher. —
  cost if wrong: if the badge kept the 40 MHz, a read of
  `rtc_clk_apb_freq_get()` before the app's own clock init would see
  80 MHz in the emulator and 40 MHz on the badge; at a cold boot the
  emulator still reads 0.
- Ruling R-T1-4: GPIO21's output enable is cleared at a CPU reset, as the
  bootloader's `release_default_console_io` does, on the evidence that
  v392 is built with a non-UART console (`"/dev/uart/-1"` in
  `firmware.bin`). — cost if wrong: on a UART-console build the badge keeps GPIO21
  (`HC165_CLK`) an output across the reboot, while the emulator leaves it
  undriven until the app configures it.
- Ruling R-T1-5: the RNG is kept, so its xorshift sequence continues
  across a reset rather than restarting from `RNG_SEED`. The badge's RNG
  never restarts its output. — cost if wrong: random values after a
  reboot differ from the restart-from-seed sequence; neither is the
  badge's entropy.
- Ruling R-T1-6: TIMG0 and TIMG1 are kept, although on the badge one
  whose clock was off at the reset is held in reset by the app
  (`clk.c` :224–229, :306–307), and the model has no clock gates. —
  cost if wrong: the model holds only calibration state and inert
  watchdog words, and the app calibrates again at boot.
- Ruling R-T1-7: of the bootloader's RTC_CNTL writes, only `STORE4` and
  `STORE5` are reproduced. The rest (clock tuning and source words, the
  watchdogs) land in words the model stores but nothing models, and the
  cold boot does not reproduce them either. — cost if wrong: a guest
  read of one of those words after a reboot sees the old app's value
  instead of the bootloader's.
- Ruling R-T2-1: the unmapped-access log's reset marker is a per-log
  counter, `FirmwareBus::unmapped_log_resets()`, not a marker entry in the
  ring. `UnmappedAccess` stays an address and a direction, and the log's
  capacity is not spent on markers. — cost if wrong: the log says how many
  resets it crossed but not which entries came before which reset; a
  diagnosis that needs that split would have to compare the log at the
  reset.
- Ruling R-T5-1: the `send_packet` hook delivers the command's own reply
  ahead of any report queued before it (`HciController::pop_reply`), and
  the report waits for the idle hook, so a reply always reaches the host
  in the task that sent the command (R-T6-2's ordering risk, fixed
  rather than only pinned). No pin moved, `two_badges_pair_to_synced`
  included. — cost if wrong: a report is seen after a command sent later
  than it, which HCI's event order forbids on a real controller; NimBLE
  does not depend on the order, and a report was already delayed to the
  idle pass whenever a reply was in the queue.

- Ruling R-T6-1: LE Set Extended Advertising Enable's duplicate-handle
  check (Core v5.4, Vol 4 Part E, 7.8.56) runs for disabling too, and
  before the Unknown Advertising Identifier check: 7.8.56 states both
  rules before "The remainder of this section only applies if Enable is
  set to 0x01", duplicates first, and leaves the precedence otherwise
  open — cost if wrong: an error code for a host that names an unknown
  handle twice; NimBLE on v392 sends one set.
- Ruling R-T6-2: Filter_Duplicates 2 is refused when Duration or Period
  is zero, not only Period as the spec says: 7.8.65's one sentence names
  "either Period or Duration". Only when Enable is 1: with Enable 0 "the
  remaining parameters shall be ignored" — cost if wrong: none for
  NimBLE on v392, which sends Filter_Duplicates 0 (`docs/milestone-10-decisions.md`,
  the command trace).
- Ruling R-T6-3: the proptest runs are made deterministic in proptest
  1.12's config, not by a hand-built `TestRng`: `rng_seed:
  RngSeed::Fixed(..)`, `rng_algorithm: RngAlgorithm::ChaCha`, a fixed
  `cases` and `failure_persistence: None`, set explicitly so the
  `PROPTEST_*` environment variables do not change them. Checked: the HCI
  run's totals are identical, with `PROPTEST_RNG_SEED`,
  `PROPTEST_RNG_ALGORITHM` and `PROPTEST_CASES` set and with none. The
  tests call `TestRunner::run`, not the `proptest!` macro, so the
  coverage counters can outlive the cases — cost if wrong: none: a
  failure prints its input, and the seed is in the test.
- Ruling R-T6-4: the HCI fuzz test's coverage assertions are a floor on
  reports (over 1,000; the run reaches 7,837), not `> 0`, because hostile
  LE Set Event Mask packets open the masks a few times by themselves — cost
  if wrong: a change that lowers the report count that far fails the
  test; lower the floor with evidence.
- Ruling R-T6-5: `runtime.rs`'s "serial fuzz" is the random-bytes loop of
  `deliver_rejects_malformed_bytes`, the only xorshift in that file (peer
  bytes, in the wire form). Only that loop changed; the rest of the test
  stands — cost if wrong: none.
- `md-5` =0.11.0 (`emulator-core`, Task 7; RustCrypto): `md5.rs`'s
  block compression is its `block_api::compress`, in place of the
  hand-written 64 rounds and their two constant tables (spec, Part 7).
  `default-features = false`: no `alloc` or `oid`. Its tree is `cfg-if`,
  `digest` 0.11.3, `block-buffer`, `crypto-common`, `hybrid-array` and
  `typenum`, all pure Rust and `no_std`; on `wasm32-unknown-unknown` the
  crate takes its `soft` backend (only LoongArch64 has assembly). The
  Plumb context, its 88-byte guest image and every byte-exact write stay
  in `md5.rs`; RFC 1321's suite, the ROM-stub tests and
  `PARTITION_TABLE_MD5` pass unchanged. The WASM grew from 140,003 B to
  141,861 B (+1,858 B, +1.3%), measured with `ls -l` of
  `frontend/src/cpu/wasm-pkg/emulator_wasm_bg.wasm` after `bun run
  build:wasm` at `1c8aeb6` and after the change.
- Ruling R-T7-1: the crate's library is named `md5`, like the module
  `emulator_core::md5`. `md5.rs` calls it by the absolute path
  `::md5::block_api::compress`, with no rename in `Cargo.toml` and no
  `use` alias — cost if wrong: none; a build error if it were ambiguous.
- `object` =0.40.0 (dev, `emulator-core`, Task 7): `riscv_integration.rs`
  flattens each fixture ELF by copying its `PT_LOAD` segments' file bytes
  to their physical addresses, in place of shelling out to
  `rust-objcopy`/`llvm-objcopy`, so the `llvm-tools-preview` component is
  no longer needed (spec, Part 7). `default-features = false, features =
  ["read_core", "elf"]`: no `std`, `compression` or other formats. A
  dev-dependency: neither the WASM build nor `emulator-wasm`'s tree has
  it. Checked against the objcopy route at `1c8aeb6`: both fixtures' flat
  images are byte-identical (`toy_sum` 42 B, `toy_mul` 489 B in two
  segments), with the same entry points (0 and 2). No segment has
  `p_memsz` above `p_filesz`. The loader, like `objcopy -O binary`,
  stops at the last file byte, and `FlatBus` starts zeroed anyway.
- Ruling R-T8-1: the reboot's debt is paid by each frame's *clamped*
  wall delta (at most `MAX_FRAME_DELTA_MS`), as new emulated time is, so
  a tab back from the background pays 50 ms of the pause, not all of it;
  a cold `reset()` drops the debt — cost if wrong: a pause up to 466 ms
  longer after a backgrounded reboot.
- Ruling R-T8-2: the speed reading counts the debt a frame paid as
  emulated time (and leaves the jump out), so the readout stays at the
  core's speed through the pause instead of dropping to 0x — cost if
  wrong: a readout that hides a 466 ms freeze, which is the badge's own.
- Ruling R-T8-3: only the tap queue's `downSince`/`upSince` move past the
  jump; a shake or bump playing across a reboot is not shifted, so its
  remaining pulse is cut short — cost if wrong: a shake or bump timed
  across a reboot misses its last periods (v392's five apps that open by
  rebooting are entered by a button, and Connect's bump does not reboot).
- Ruling R-T9-1: the four windows are `Ratelimit.fixedWindow`'s, fixed to
  the clock (window number `floor(now / window)`), not opened by a key's
  first request as the old counters were; `memoryLimiter` buckets the
  same way, and a 429's `Retry-After` is the whole seconds left until the
  limiter's `reset` (at least 1), no longer the whole window. The caps
  and windows are unchanged (R-T10-8) — cost if wrong: across a window
  boundary a key can spend up to twice its cap in a short burst (60
  writes in a few seconds around a minute boundary; a room's 300 s life
  can span two poll windows, so up to 400 polls; up to 10,000 global
  requests around a 10 min boundary, which spends the quota faster). At
  toy scale this is accepted; a sliding window costs more commands.
- Ruling R-T9-2: the Upstash limiters run with `timeout: 0`, so a slow
  or failing store is waited for or fails (a bare 500), never waved
  through as the library's default 5 s timeout would (it answers
  `success: true`); analytics, telemetry and the deny list stay off; the
  library's per-instance cache of refused keys stays on (a key refused
  in a window is refused again by that instance with no command). Cost
  if wrong: a hung Upstash holds each request until Vercel's function
  timeout instead of serving it unlimited after 5 s.
- Ruling R-T9-3: `clientKey` parses the address strictly: IPv4 whole,
  IPv6 its /64 (four hextets, expanded, lower case), an IPv4-mapped IPv6
  address (`::ffff:a.b.c.d`) as its IPv4 address (else every such client
  would share `0000:0000:0000:0000`), and anything else, a zone index
  included, `unknown`; the `x-real-ip` fallback stays — cost if wrong: a
  proxy that forwards non-canonical addresses lumps its clients under
  `unknown`, which shares one set of client windows.
- Ruling R-T9-4: `api/signal.ts` checks the credentials `Redis.fromEnv`
  will read, in its order (`UPSTASH_REDIS_REST_URL` or else
  `KV_REST_API_URL`, the same for the token), and also answers a bare
  500 when one has surrounding whitespace or a line break, since
  `@upstash/redis` would print a console warning for it, and nothing is
  ever logged — cost if wrong: a deploy whose credential has a trailing
  newline answers 500 instead of trying Upstash.
- Ruling R-T9-5: `create` writes the offer (`SET NX`, which claims the
  code), then the token (`SET`, same TTL); the Upstash adapters live in
  `api/signal.ts`, not the handler, so the browser bundle cannot carry
  them — cost if wrong: if the token's `SET` fails after the offer's,
  the host gets a 500 and the room, which cannot be closed, expires
  after its 300 s.
- Ruling R-T10-1 (Milestone 11's numbering; Milestone 10's doc has its
  own R-T10-k): a 429's message takes `Retry-After` only as whole
  seconds (1 to 9 digits, surrounding spaces allowed), giving "Signaling
  server busy, try again in N min." with N = ceil(seconds / 60), at
  least 1; a missing or malformed header, an HTTP date or a negative
  number included, gives "Signaling server busy, try again later."
  without throwing. A 429 or any 5xx during host or join polling ends
  that side's signaling, as every other error does — cost if wrong: a
  proxy that rewrites `Retry-After` as a date loses the minute count
  (this server only sends seconds).
- Ruling R-T10-2 (Milestone 11's numbering): `waitOpen` races
  `PeerLink.opened` against one deadline wait (`sleep(remaining)`,
  cleared through an `AbortSignal` once the wait ends) and the cancel;
  the deadline still goes through the injected `sleep`, since the
  120 s give-up tests run on a fake clock. So
  `waitOpen_resolves_on_the_open_event` asserts that the joiner's clock
  never moves and its `sleep` is called at most once, for the whole
  deadline, never a poll interval (its sleeps never resolve), rather
  than that `sleep` is never called — cost if wrong: none in the
  browser; a test that wants a sleep-free open would need a second
  injected timer.
- Ruling R-T11-1 (Task 11, `fix: gate the bump on the running badge;
  coalesce advertisements`): Milestone 10's R-T10-1 (bump ungated) is
  revised, as the spec's "Part 4: Browser items" item 4 says, by the
  human partner's choice. `hub.bump()` sends no `{t:"bump"}` to any
  link, does not bump the local badge, and returns `false` unless the
  local runtime is running *and* the hub is not paused: running is
  `Hub.setRunning`, which `main.ts` sets at each `rt.start()` and
  `firmwareRuntime?.stop()` (a mode switch, a lost badge); hidden is the
  hub's existing pause (`onPausedChange`). The two combine as an AND,
  and neither is remembered: a bump refused while stopped or hidden is
  not sent later. A peer's bump is unchanged (it reaches the local badge,
  which drops it when stopped or hidden, R-T9-3). So that a `start()`
  made while the page is hidden also gates it, the Worker client now
  calls `onPausedChange(true)` at that start, not only at a later hide —
  cost if wrong: two people who tap Bump in a tab whose badge is not
  running see nothing happen on either side, where before the peer felt
  it.
- Ruling R-T11-2 (Task 11): the Worker posts `advOut` at most every
  `ADV_REPOST_MS` (100 ms). It reads the advertisement only when that
  much time has passed since its last post, and then posts it if it
  changed or is non-empty; the first change after a quiet window still
  goes out at once. Twenty changes inside 100 ms are therefore at most
  two posts, the second the latest, and a last change (stopping
  advertising included) goes out on the next tick, up to 100 ms later
  than before — cost if wrong: a peer hears an advertisement change up
  to 100 ms later, inside Connect's scan windows.
- Ruling R-T11-3 (Task 11): `macLocked` is set only when the emulator's
  `setFactoryMac` returns `true`; a refusal or a throw (posted as
  `error`) leaves the next `setMac` free to try. `start` still locks it
  — cost if wrong: none; the browser sends one MAC per Worker.
- Ruling R-T12-1 (Task 12, `fix: release the badge on Lua mode or a
  failed start; happy-dom for the shell tests`): `main.ts` runs itself on
  import and reaches `fetch`, `Worker` and `navigator.locks` directly, so
  its badge wiring moved into `src/ui/badge-controller.ts`:
  `createBadgeController(deps)` (the mode switch, the Worker, the lock and
  its notice, the provisioning controls, the bump gate's three
  `hub.setRunning` calls with their markers, `onPausedChange` to
  `hub.setPaused`) and `mountLinks` (with Task 10's `cancelHosting`).
  `main.ts` passes them the browser's and keeps the Lua paint loop, the
  log panel and unload — cost if wrong: none at run time; a later edit to
  the page wiring is made in two files.
- Ruling R-T12-2 (Task 12): the badge is freed (released once, then
  dropped) on a switch to Lua mode, when the first boot fails, at unload,
  and when a claim that was in flight lands after a switch to Lua. Provision
  claims the badge before it boots anything: without it, it shows the
  notice and creates no Worker. A waiting tab still needs "Run here" (the
  spec's call) — cost if wrong: none found; a tab that switches to Lua and
  back retakes the badge, and loses it if another tab took it meanwhile.
- Ruling R-T12-3 (Task 12): showing the badge notice clears the canvas
  (`clearRect`, then a `fillRect` of the whole canvas) in `shell.css`'s
  `--bg`, read from the page's computed style (the body's computed
  background if the token is missing; only `clearRect` if both are) —
  cost if wrong: the canvas reads as the page behind the badge frame's
  darker `#05070a`, not as an off screen.
- Ruling R-T12-4 (Task 12): pressing "Host link" again, or cancelling,
  while a room is still being created now closes that room as soon as
  `hostRoom` returns (a counter of presses and cancels), instead of
  orphaning it until its TTL; its failure no longer overwrites the newer
  status (Task 10's review) — cost if wrong: none; the room was never
  shown.
- Ruling R-T13-1 (Task 13, `fix: tap sources, taps across stop, and the
  provisioner's prompt scan`): a button's sources are a set per button
  (`"key"`, `"pad"`), not a count: a second press from the same source
  is a repeat and ignored, a second source joins the held tap instead of
  queueing a new one, the tap is let go when the set empties, and a
  release from a source not in the set changes nothing. The set is
  non-empty exactly while the last tap is unreleased; a press dropped by
  `MAX_QUEUED_TAPS` adds no source, and `reset()` clears the set with
  the taps it drops. A missing or unknown `source` (the Worker message's
  field is optional) is the pad — cost if wrong: two pads or two
  keyboards on one button would act as one source; the page has one of
  each.
- Ruling R-T13-2 (Task 13): `stop()` still releases a tap that is down
  in the emulator and let go (R-M8-6's deferred release, so nothing is
  left down while stopped), but keeps the taps queued behind it; they
  play after the next `start()` with their full `PRESS_HOLD_US` and the
  `RELEASE_GAP_US` from that release, all in emulated time, which does
  not run while stopped (so non-paused time, as Task 8's reboot shift
  counts non-jump time). A release that arrives while stopped is
  honoured. `reset()` still drops the queue (R-M8-6) — cost if wrong:
  the tap that was down at the stop gets a hold shorter than
  `PRESS_HOLD_US`, as before; a burst replays after a resume.
- Ruling R-T13-3 (Task 13): the provisioner's first prompt is matched
  from the console offset where the *current boot* began, not from the
  console's length at `provision()`. Measured: with the brief's
  start-length scan, the WASM twin (`test/cpu-wasm.test.ts`, "provisions
  the test identity…") stays `waiting`, because the REPL prints its
  prompt once at boot and none again until it gets a line, so an idle
  badge (the browser's usual case, a click well after boot) never
  prompts after the start. The runtime records the boot's start as the
  console's length in the frame that ended at a soft reset (`run_until`
  stops there, since a reboot's 466 ms jump passes any frame's 50 ms
  target), 0 at power-on and after a cold `reset()`; the provisioner
  reads it at each poll, so a reboot while it waits also drops the old
  boot's prompt. The console text is kept across a soft reset and never
  shrinks (only a cold reset empties it); it is trimmed only past its
  256 KiB ring (`CONSOLE_CAPACITY`), where every provisioner offset is
  already a shifted window (the existing limit) — cost if wrong: a
  provision started after 256 KiB of console output could miss the new
  boot's prompt and time out; an idle badge's console holds about
  2.3 KB.

## Milestone 12 backlog

Started by Task 1; later tasks add what they leave open.

- **Share's radio transfer** (the human partner, 2026-11-03).
- **Sync's empty-room path:** the fake upload credential and the ROM
  `crc32_le` stub (the human partner, 2026-11-03).
- **A cold power-on's boot time.** The shortcut boot starts the app at
  0 µs; the badge takes about 466 ms (spec, "Non-goals").
- **The core reset is narrower than the TRM.** A core reset keeps the
  whole RTC domain (§6.1.3), but `soft_reset`'s core path keeps only
  `STORE0..7` and the timer; and it drops USB-Serial-JTAG, which §30.3.4
  says a digital system reset through SRST does not touch. v392 is not
  seen to use a core reset.
- **The SYSTEM clock-gate and reset bits are not modeled**
  (`SYSTEM_PERIP_CLK_EN0/1`, `SYSTEM_PERIP_RST_EN0/1`), so a driver's own
  reset pulse after boot does nothing. Table rows 1, 7, 9, 13, 15 and 18
  stand in for the resets around a reboot only.
- **DRAM `.noinit` data** is not kept across a reset (the TRM keeps
  internal SRAM).
- **LE Set Extended Scan Enable's other parameter rules** (Core v5.4,
  7.8.65): Duration not less than a non-zero Period is `0x12`, and a
  disable (Enable 0) should ignore an out-of-range Filter_Duplicates, not
  refuse it. Neither is a form NimBLE on v392 sends. (Task 6)
- **The RTC and timer-group watchdogs** are inert storage, so the 1 s RTC
  watchdog `esp_restart_noos` arms is never checked.

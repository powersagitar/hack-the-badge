# Documentation index

The project wrapped up on 2026-11-15, after Milestone 14. This folder
holds everything known about it. Start with the first four files. The
rest is the detailed record they point into.

## Start here

| File | What it is |
| ---- | ---------- |
| [`handoff.md`](handoff.md) | The project at its end: what works, what is verified and what is not, the known limitations, the open work ranked, and how to pick it up again. |
| [`architecture.md`](architecture.md) | How the emulator and the page work: the boot, the core, the peripherals, the ROM stubs, time, resets, the radio, the browser runtime, motion, and the data flow between them. |
| [`development-guide.md`](development-guide.md) | How to work on it: setup, the test suites and what each one guards, the pin and re-pin rules, debugging a stuck boot, benchmarks, deploying, and the working process. |
| [`project-history.md`](project-history.md) | Milestones 1 to 14: dates, merge commits, what each one did and why, and where its record is. |

The repo root has two more entry points: [`../README.md`](../README.md)
(the public overview, with screenshots) and [`../CLAUDE.md`](../CLAUDE.md)
(the rules and a file-by-file architecture map).

## The detailed record

| File | What it is |
| ---- | ---------- |
| [`firmware-emulator-notes.md`](firmware-emulator-notes.md) | The main technical notes (~3,900 lines). Forensics of the flash dump, the mask-ROM problem, the pin map, the **Timing model**, **Known limitations** (the current state, item by item), ground truth from the physical badge, the stall-by-stall history of the boot, the emulated flash, and the **Data-handling note**. |
| [`milestone-3-decisions.md`](milestone-3-decisions.md) … [`milestone-14-decisions.md`](milestone-14-decisions.md) | One per milestone: rulings with their reasons, measurements, traces, re-pins, the human partner's gate answers, the dependencies added, and the next milestone's backlog. Milestone 14's ends with the final (Milestone 15) backlog. |
| [`superpowers/specs/`](superpowers/specs/) | The design spec of each milestone from 3 on (none for 8, which was approved in chat). |
| [`superpowers/plans/`](superpowers/plans/) | The task-by-task implementation plan of each milestone from 3 on. |
| [`images/`](images/) | The README's screenshot and demo GIFs. |

Where a spec or plan disagrees with its milestone's decisions doc or
with the code, the decisions doc and the code win; specs and plans are
historical. Milestones 1 and 2 predate the decisions docs. Milestone 2's
findings open `firmware-emulator-notes.md`.

## Finding a specific fact

| Looking for | Where |
| ----------- | ----- |
| Why built-in apps are emulated, not extracted | notes, "Why real firmware is emulated instead of extracted" |
| A mask-ROM stub and why it exists | notes, "The mask-ROM problem"; `emulator-core/src/rom.rs` |
| How steps become time, pacing in the browser | notes, "Timing model"; Milestone 6 and 7 decisions |
| What a reboot keeps | Milestone 11 decisions, "What a CPU reset keeps" |
| The fake BLE controller | notes, "The radio (Milestone 10)"; Milestone 10 decisions, "Rulings" |
| Sync's and Share's traces | Milestone 12 decisions, "Traces" |
| v335 → v392 changes | Milestone 9 decisions, "Re-pins (v335 → v392)" |
| Speed: benchmarks and profiles | Milestone 7 decisions, "Measurements" and "Profiles" |
| LEDs, axes, physics, phone motion, the photo | Milestone 14 decisions |
| What may and may not be committed | notes, "Data-handling note"; `CLAUDE.md`, "Local data" |

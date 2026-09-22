//! `boot-probe`: a standalone diagnostic CLI that boots the real dumped
//! firmware (`frontend/public/firmware/factory.bin`) through
//! [`emulator_core::runtime::FirmwareRuntime`] for a configurable number of
//! steps and prints everything Task 2's "why is boot stuck" investigation
//! needs: the firmware's own console output, a run summary (traps, ROM stub
//! calls, the last instruction-fetch fault), a "hot PCs" histogram over the
//! trailing window (via [`emulator_core::runtime::FirmwareRuntime::run_traced`]),
//! a deduplicated tail of the bus's unmapped-access log (via
//! [`emulator_core::peripherals::systimer::SysTimer::handles`]/
//! [`emulator_core::peripherals::intc::InterruptController::handles`]-gated
//! logging in `crate::mem::bus::FirmwareBus`), and the framebuffer's pixel
//! diversity.
//!
//! ## Usage
//!
//! ```text
//! cargo run -p emulator-core --release --example boot-probe -- \
//!     [--steps N] [--window W] [--dump-frame PATH]
//! ```
//!
//! - `--steps N` (default 20,000,000): total instruction steps to run.
//! - `--window W` (default 200,000): how many of the *trailing* steps are
//!   PC-traced for the hot-PCs histogram (kept separate from the main run so
//!   the bulk of a long run stays allocation-free, per `run`/`run_traced`'s
//!   split).
//! - `--dump-frame PATH`: also write the final framebuffer as an 8-bit RGB
//!   PNG. A relative `PATH` is resolved under the repo's `local/` directory
//!   (gitignored, personal-data-safe) and refused if it would land outside
//!   it; an absolute `PATH` is allowed as given. Either way, a warning is
//!   printed that a dumped frame may show personal data (whatever the badge
//!   happened to be displaying).
//!
//! ## `BADGE_FULL_DUMP`
//!
//! If the `BADGE_FULL_DUMP` env var is set to a readable file path, this
//! prints that file's ESP-IDF partition table (parsed from the 32-byte
//! entries starting at flash offset `0x8000`, while the `0xAA 0x50` magic
//! keeps matching) under its own section. This is the *only* content of that
//! file this tool ever prints — partition names/offsets/sizes/types are not
//! personal data, but nothing else about the dump is read or shown. See
//! `CLAUDE.md`'s "Data-handling note" / this task's brief for why the full
//! dump itself is never committed.

use std::collections::HashMap;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use emulator_core::runtime::FirmwareRuntime;

const CHUNK: u32 = 1_000_000;
const DEFAULT_STEPS: u32 = 20_000_000;
const DEFAULT_WINDOW: u32 = 200_000;
/// How many top hot-PC entries to print.
const TOP_N_HOT_PCS: usize = 20;
/// How many of the most recent unmapped-log entries to consider for the
/// deduplicated tail.
const UNMAPPED_TAIL: usize = 64;

struct Args {
    steps: u32,
    window: u32,
    dump_frame: Option<String>,
}

fn parse_args() -> Args {
    let mut steps = DEFAULT_STEPS;
    let mut window = DEFAULT_WINDOW;
    let mut dump_frame = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--steps" => {
                steps = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_STEPS);
            }
            "--window" => {
                window = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_WINDOW);
            }
            "--dump-frame" => {
                dump_frame = args.next();
            }
            other => {
                eprintln!("warning: ignoring unrecognized argument {other:?}");
            }
        }
    }

    Args {
        steps,
        window,
        dump_frame,
    }
}

/// Lexically normalizes a path (resolves `.`/`..` components without
/// touching the filesystem -- the target file doesn't exist yet, so
/// `Path::canonicalize` isn't usable here). A `..` that pops past the start
/// of the path is simply dropped from `out`, which is exactly what makes the
/// `starts_with` escape check below work: an attempt to climb out of
/// `local/` collapses `out` down past `local/` itself, so it stops matching
/// the (separately normalized) `local/` prefix.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolves `--dump-frame`'s raw argument per the brief's rule: an absolute
/// path is used as given; a relative path is resolved under the repo's
/// `local/` directory and rejected (returns `Err`) if the result would fall
/// outside it (e.g. via a `..` escape).
fn resolve_dump_path(raw: &str) -> Result<PathBuf, String> {
    let requested = Path::new(raw);
    if requested.is_absolute() {
        return Ok(requested.to_path_buf());
    }

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let local_dir = normalize_lexically(&repo_root.join("local"));
    let joined = normalize_lexically(&local_dir.join(requested));

    if joined.starts_with(&local_dir) {
        Ok(joined)
    } else {
        Err(format!(
            "refusing to write {raw:?} outside the repo-relative local/ directory \
             (resolved to {joined:?}, which escapes {local_dir:?}); pass an absolute \
             path if you really mean somewhere else"
        ))
    }
}

/// Expands one RGB565 pixel to 8-bit RGB, per the brief's exact formula.
fn rgb565_to_rgb888(px: u16) -> [u8; 3] {
    let r = (px >> 11) & 0x1F;
    let g = (px >> 5) & 0x3F;
    let b = px & 0x1F;
    let r8 = ((r << 3) | (r >> 2)) as u8;
    let g8 = ((g << 2) | (g >> 4)) as u8;
    let b8 = ((b << 3) | (b >> 2)) as u8;
    [r8, g8, b8]
}

fn dump_frame(rt: &FirmwareRuntime, raw_path: &str) {
    eprintln!(
        "warning: a dumped frame may show personal data (whatever the badge's screen \
         happened to be displaying at the moment it was captured)"
    );
    let path = match resolve_dump_path(raw_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("error: could not create {parent:?}: {e}");
            return;
        }
    }
    let width = rt.screen_width();
    let height = rt.screen_height();
    let mut rgb = Vec::with_capacity(width * height * 3);
    for px in rt.framebuffer() {
        rgb.extend_from_slice(&rgb565_to_rgb888(*px));
    }

    let file = match fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: could not create {path:?}: {e}");
            return;
        }
    };
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    match encoder.write_header() {
        Ok(mut writer) => {
            if let Err(e) = writer.write_image_data(&rgb) {
                eprintln!("error: could not write PNG data to {path:?}: {e}");
                return;
            }
            println!("wrote frame to {}", path.display());
        }
        Err(e) => eprintln!("error: could not write PNG header to {path:?}: {e}"),
    }
}

/// Prints `BADGE_FULL_DUMP`'s partition table, if the env var is set and
/// points at a readable file -- and *only* the partition table; see the
/// module doc's "BADGE_FULL_DUMP" section.
fn print_full_dump_partition_table() {
    let Ok(path) = std::env::var("BADGE_FULL_DUMP") else {
        return;
    };
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("warning: BADGE_FULL_DUMP={path:?} set but unreadable: {e}");
            return;
        }
    };
    println!("\n== full dump partition table ==");
    const TABLE_OFFSET: usize = 0x8000;
    const ENTRY_LEN: usize = 32;
    let mut offset = TABLE_OFFSET;
    while offset + ENTRY_LEN <= bytes.len() {
        let entry = &bytes[offset..offset + ENTRY_LEN];
        if entry[0] != 0xAA || entry[1] != 0x50 {
            break;
        }
        let part_type = entry[2];
        let subtype = entry[3];
        let part_offset = u32::from_le_bytes(entry[4..8].try_into().unwrap());
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap());
        let label_bytes = &entry[12..28];
        let label_len = label_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let label = String::from_utf8_lossy(&label_bytes[..label_len]);
        println!(
            "  {label:<16} type=0x{part_type:02x} subtype=0x{subtype:02x} \
             offset=0x{part_offset:08x} size=0x{size:08x}"
        );
        offset += ENTRY_LEN;
    }
}

fn main() {
    let args = parse_args();

    let image_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/public/firmware/factory.bin");
    let image = fs::read(&image_path).unwrap_or_else(|e| {
        panic!("could not read firmware image at {image_path:?}: {e}");
    });

    let mut rt =
        FirmwareRuntime::from_image(&image).expect("factory.bin should parse as an app image");

    let window = args.window.min(args.steps);
    let non_traced = args.steps - window;

    let mut total_traps: u32 = 0;
    let mut total_rom_stub_calls: u32 = 0;
    let mut last_fault: Option<u32> = None;

    let mut remaining = non_traced;
    while remaining > 0 {
        let chunk = remaining.min(CHUNK);
        let summary = rt.run(chunk);
        total_traps += summary.traps;
        total_rom_stub_calls += summary.rom_stub_calls;
        if summary.last_instruction_fault.is_some() {
            last_fault = summary.last_instruction_fault;
        }
        remaining -= chunk;
    }

    let mut hist: HashMap<u32, u64> = HashMap::new();
    let summary = rt.run_traced(window, &mut hist);
    total_traps += summary.traps;
    total_rom_stub_calls += summary.rom_stub_calls;
    if summary.last_instruction_fault.is_some() {
        last_fault = summary.last_instruction_fault;
    }
    let pc = summary.pc;

    println!("== console ==");
    println!("{}", rt.console_output());

    println!("\n== summary ==");
    println!("total steps: {}", non_traced + window);
    println!("pc: 0x{pc:08x}");
    println!("traps: {total_traps}");
    println!("rom_stub_calls: {total_rom_stub_calls}");
    match last_fault {
        Some(addr) => println!("last_instruction_fault: 0x{addr:08x}"),
        None => println!("last_instruction_fault: none"),
    }

    println!("\n== hot PCs (last {window} steps) ==");
    let mut hot: Vec<(u32, u64)> = hist.into_iter().collect();
    hot.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (addr, count) in hot.into_iter().take(TOP_N_HOT_PCS) {
        println!("0x{addr:08x}  {count}");
    }

    println!("\n== unmapped (tail) ==");
    let log = rt.bus().unmapped_log();
    let tail: Vec<_> = log.iter().rev().take(UNMAPPED_TAIL).collect();
    let mut dedup: Vec<((u32, bool), u64)> = Vec::new();
    for access in tail.iter().rev() {
        let key = (access.addr & !3, access.is_write);
        if let Some(entry) = dedup.iter_mut().find(|(k, _)| *k == key) {
            entry.1 += 1;
        } else {
            dedup.push((key, 1));
        }
    }
    for ((addr, is_write), count) in &dedup {
        let rw = if *is_write { "W" } else { "R" };
        println!("0x{addr:08x}  {rw}  count={count}");
    }

    println!("\n== framebuffer ==");
    let distinct: std::collections::HashSet<u16> = rt.framebuffer().iter().copied().collect();
    println!("distinct pixel values: {}", distinct.len());

    if let Some(raw_path) = args.dump_frame {
        dump_frame(&rt, &raw_path);
    }

    print_full_dump_partition_table();
}

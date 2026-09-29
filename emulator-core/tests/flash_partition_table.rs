//! Gated check that `EmulatedFlash`'s synthesized partition table matches the
//! real badge's, byte for byte (Milestone 3 Task 8).
//!
//! Needs the local-only full flash dump: set `BADGE_FULL_DUMP` to its path.
//! Without it the test prints a note and passes vacuously, so no committed
//! test depends on the dump (see `docs/firmware-emulator-notes.md`'s
//! data-handling note). Only the partition-table region (`0x8000`, up to
//! `ESP_PARTITION_TABLE_MAX_LEN` bytes) is read and compared; on a mismatch
//! only the first differing offset is reported, never the dump's bytes.

use emulator_core::peripherals::flash::{
    EmulatedFlash, PARTITION_TABLE_MAX_LEN, PARTITION_TABLE_OFFSET,
};

#[test]
fn synthesized_partition_table_matches_the_real_chip() {
    let Some(path) = std::env::var_os("BADGE_FULL_DUMP") else {
        eprintln!("BADGE_FULL_DUMP not set; skipping the real-partition-table comparison");
        return;
    };
    let dump = std::fs::read(&path).expect("BADGE_FULL_DUMP is set but unreadable");
    let start = PARTITION_TABLE_OFFSET as usize;
    let end = start + PARTITION_TABLE_MAX_LEN as usize;
    assert!(
        dump.len() >= end,
        "dump too short to hold a partition table"
    );

    let flash = EmulatedFlash::from_app_image(&[]);
    let first_diff = (start..end).find(|&off| flash.read(off as u32) != dump[off]);
    assert_eq!(
        first_diff, None,
        "synthesized partition table differs from the real chip's (first differing flash offset shown)"
    );
}

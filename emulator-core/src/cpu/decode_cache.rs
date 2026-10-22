//! A direct-mapped cache of decoded instructions, validated by the raw
//! instruction bits on every hit (Milestone 7, Task 7).
//!
//! [`super::Cpu::step`] still fetches every instruction through
//! [`crate::mem::Bus::fetch16`], exactly as without the cache, so a fetch
//! fault traps as before. Only the decode is cached: `step` looks the
//! fetched `(pc, raw)` up and decodes on a miss.
//!
//! **No invalidation is needed.** Decoding is a pure function of the raw
//! bits (`decode_16` and `decode_32` take nothing else: no `pc`, no machine
//! state), and a hit requires the bits fetched now to equal the bits
//! decoded then. So a store over a cached instruction, a ROM-stub memory
//! write, a flash erase or program, and an MMU remap all run as the new
//! code: the fetch returns the new bits, the lookup misses, and the new
//! bits are decoded.
//!
//! The table is indexed by `pc` (direct-mapped, `(pc >> 1) & mask`) but
//! tagged by `raw` alone (Milestone 7 ruling R-T7-3): since the decode
//! depends only on the bits, an entry another `pc` left in the slot is
//! still the true decode when its bits match, so the hit is correct and no
//! `pc` tag is stored or compared. Every entry, including the initial ones
//! (the bits 0), holds a true decode of its `raw`, so the table needs no
//! valid flag.
//!
//! `raw` is the 16-bit halfword zero-extended for a compressed instruction
//! (its low two bits are never `0b11`) and the whole 32-bit word for a
//! full-width one (low bits `0b11`), so the bits also determine the length.
//!
//! The table is a plain field of the CPU, so a cloned CPU (and a cloned
//! `FirmwareRuntime`) carries a copy: a clone is still a full snapshot.

use super::decode::{decode_16, decode_32, Instruction};

/// log2 of the number of entries.
const INDEX_BITS: u32 = 14;
const ENTRIES: usize = 1 << INDEX_BITS;
const INDEX_MASK: u32 = (ENTRIES as u32) - 1;

#[derive(Clone, Copy)]
struct Entry {
    raw: u32,
    instr: Instruction,
    len: u8,
}

#[derive(Clone)]
pub(crate) struct DecodeCache {
    entries: Box<[Entry; ENTRIES]>,
}

impl Default for DecodeCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Decodes `raw` (see the module doc for its encoding) and returns the
/// instruction with its length in bytes.
#[inline]
pub(crate) fn decode_raw(raw: u32) -> (Instruction, u8) {
    if raw & 0b11 != 0b11 {
        (decode_16(raw as u16), 2)
    } else {
        (decode_32(raw), 4)
    }
}

impl DecodeCache {
    pub(crate) fn new() -> Self {
        // An unused slot holds the true decode of the bits 0, so a hit on
        // it is as correct as any other and no valid flag is needed.
        let (instr, len) = decode_raw(0);
        let empty = Entry { raw: 0, instr, len };
        let entries: Box<[Entry]> = vec![empty; ENTRIES].into_boxed_slice();
        Self {
            entries: entries
                .try_into()
                .unwrap_or_else(|_| unreachable!("the vec has ENTRIES elements")),
        }
    }

    #[inline]
    fn index(pc: u32) -> usize {
        ((pc >> 1) & INDEX_MASK) as usize
    }

    /// The cached decode of `raw`, if `pc`'s slot holds a decode of
    /// exactly these bits (from `pc` or from another address that shares
    /// the slot; the decode is the same, see the module doc).
    #[inline]
    pub(crate) fn lookup(&self, pc: u32, raw: u32) -> Option<(Instruction, u8)> {
        let e = &self.entries[Self::index(pc)];
        if e.raw == raw {
            Some((e.instr, e.len))
        } else {
            None
        }
    }

    /// Records `instr` (`len` bytes) as the decode of `raw` at `pc`,
    /// replacing whatever shared its slot.
    #[inline]
    pub(crate) fn insert(&mut self, pc: u32, raw: u32, instr: Instruction, len: u8) {
        self.entries[Self::index(pc)] = Entry { raw, instr, len };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_cache_misses_and_an_insert_hits_only_its_bits() {
        let mut c = DecodeCache::new();
        assert_eq!(c.lookup(0x100, 0x0013), None);
        let (instr, len) = decode_raw(0x0000_0013);
        c.insert(0x100, 0x0000_0013, instr, len);
        assert_eq!(c.lookup(0x100, 0x0000_0013), Some((instr, 4)));
        assert_eq!(c.lookup(0x100, 0x0010_0013), None, "other bits miss");
        // Another pc in the same slot hits only on the same bits, which
        // decode the same anywhere; other bits miss, and an insert
        // replaces the slot.
        let alias = 0x100 + 2 * ENTRIES as u32;
        assert_eq!(c.lookup(alias, 0x0000_0013), Some((instr, 4)));
        assert_eq!(c.lookup(alias, 0x0001), None);
        assert_eq!(c.lookup(0x102, 0x0000_0013), None, "another slot misses");
        c.insert(alias, 0x0001, decode_16(0x0001), 2);
        assert_eq!(c.lookup(0x100, 0x0000_0013), None);
        assert_eq!(c.lookup(0x100, 0x0001), Some((decode_16(0x0001), 2)));
        assert_eq!(c.lookup(alias, 0x0001), Some((decode_16(0x0001), 2)));
    }

    #[test]
    fn a_clone_is_a_snapshot() {
        let mut c = DecodeCache::new();
        c.insert(0, 0x0001, decode_16(0x0001), 2);
        let snap = c.clone();
        c.insert(0, 0x0005, decode_16(0x0005), 2);
        assert_eq!(snap.lookup(0, 0x0001), Some((decode_16(0x0001), 2)));
        assert_eq!(c.lookup(0, 0x0001), None);
    }
}

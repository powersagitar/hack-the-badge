//! App-image hooks (Milestone 10 Task 3): addresses *inside the loaded
//! app image* whose arrival the driver wants to intercept, so a host-side
//! model can stand in for a guest function (chip-specific code, such as a
//! high-level model of a peripheral's driver, can replace guest functions
//! this way).
//!
//! This is the counterpart of [`super::rom_stubs`] with the work moved out
//! of the core: a ROM stub runs its effect inside [`super::Cpu::step`],
//! while a hook only reports the arrival. A step whose `pc` is hooked
//! consumes the step without fetching (as a ROM stub does), records a
//! [`HookHit`], raises the core's attention flag
//! ([`super::Cpu::needs_attention`]) and leaves `pc` on the hooked address.
//! The driver then collects the hit ([`super::Cpu::take_hook_hit`]) and
//! must do exactly one of:
//!
//! - [`super::Cpu::hook_return`]: return to `ra`, optionally with `a0`
//!   (the hooked function is fully replaced);
//! - [`super::Cpu::hook_pass_through`]: the next arrival at that `pc`
//!   fetches and executes normally, once (the guest function runs);
//! - a redirect: the driver writes `pc` itself.
//!
//! A hit nobody handles hits again on the next step, since `pc` stays put.
//!
//! **Ordering in a step:** the WFI park, then a pending interrupt, then a
//! ROM stub, then a hook, then the fetch. An interrupt is therefore taken
//! at the boundary before a hooked call, with `mepc` on the hook, exactly
//! as for a ROM stub; a pass-through is kept until the next arrival at its
//! `pc`, so an interrupt taken in between does not spend it. An address in
//! both tables is a ROM stub.
//!
//! **Cost when nothing is hooked near `pc`:** two range tests, each a
//! subtract and a compare, with one branch ([`AppHookTable::in_ranges`];
//! seven AArch64 instructions in the release build). The
//! registered addresses are split into two ranges at their largest gap, so
//! a typical table (a cluster of entries in one module
//! plus one outlier elsewhere in the image) covers its cluster and
//! its outlier but not the code between them. An empty range has length 0
//! and matches nothing, so an empty table (the default) never hits and
//! changes nothing a step does. Only a `pc` inside a range pays the
//! binary search.
//!
//! No address or chip constant lives here: the table is learned from what
//! the chip-specific code registers (Milestone 10 ruling R-PF-1).

/// What a hooked step reports ([`super::Cpu::take_hook_hit`]): the hook's
/// id as registered, and the `pc` it was hit at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookHit {
    pub id: u16,
    pub pc: u32,
}

/// One address range, `base..base + len`, tested as
/// `pc.wrapping_sub(base) < len` (one subtract, one compare). `len == 0`
/// is empty.
#[derive(Debug, Clone, Copy, Default)]
struct Range {
    base: u32,
    len: u32,
}

impl Range {
    /// The range covering `lo..=hi`. A length of `2^32` cannot occur: the
    /// two ranges are split at a gap, so one range never holds both `0`
    /// and `u32::MAX` unless it is a single address (see
    /// [`AppHookTable::recompute_ranges`]); saturating keeps even that
    /// case from wrapping to an empty range.
    fn covering(lo: u32, hi: u32) -> Self {
        Self {
            base: lo,
            len: (hi - lo).saturating_add(1),
        }
    }

    #[inline(always)]
    fn contains(self, addr: u32) -> bool {
        addr.wrapping_sub(self.base) < self.len
    }
}

/// Address → hook id table. Entries are kept sorted by address; the two
/// [`Range`]s are recomputed on every insert (tables are a handful of
/// entries, built once).
#[derive(Debug, Clone, Default)]
pub struct AppHookTable {
    /// `(address, id)`, sorted by address, one entry per address.
    entries: Vec<(u32, u16)>,
    ranges: [Range; 2],
}

impl AppHookTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers hook `id` at `addr`, replacing any previous entry there.
    /// Returns `&mut self` so a whole table can be built as one chained
    /// expression.
    pub fn insert(&mut self, addr: u32, id: u16) -> &mut Self {
        match self.entries.binary_search_by_key(&addr, |&(a, _)| a) {
            Ok(i) => self.entries[i].1 = id,
            Err(i) => self.entries.insert(i, (addr, id)),
        }
        self.recompute_ranges();
        self
    }

    /// Splits the sorted addresses at their largest gap: the first range
    /// covers the addresses below it, the second those above. One address
    /// gets the first range and leaves the second empty.
    fn recompute_ranges(&mut self) {
        let (Some(&(first, _)), Some(&(last, _))) = (self.entries.first(), self.entries.last())
        else {
            self.ranges = [Range::default(); 2];
            return;
        };
        let split = self
            .entries
            .windows(2)
            .enumerate()
            .max_by_key(|(_, w)| w[1].0 - w[0].0)
            .map(|(i, _)| i);
        self.ranges = match split {
            None => [Range::covering(first, last), Range::default()],
            Some(i) => [
                Range::covering(first, self.entries[i].0),
                Range::covering(self.entries[i + 1].0, last),
            ],
        };
    }

    /// `true` if `addr` lies inside either range, so it may be hooked.
    /// This is the per-step test [`super::Cpu::step`] pays.
    #[inline(always)]
    pub fn in_ranges(&self, addr: u32) -> bool {
        self.ranges[0].contains(addr) || self.ranges[1].contains(addr)
    }

    /// The hook id registered at `addr`, if any. Outside both ranges
    /// (always, for an empty table) this is `None` without a search.
    #[inline]
    pub fn lookup(&self, addr: u32) -> Option<u16> {
        if !self.in_ranges(addr) {
            return None;
        }
        self.search(addr)
    }

    #[cold]
    #[inline(never)]
    fn search(&self, addr: u32) -> Option<u16> {
        self.entries
            .binary_search_by_key(&addr, |&(a, _)| a)
            .ok()
            .map(|i| self.entries[i].1)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::TestBus;
    use super::super::{encode, Cpu};
    use super::*;

    const HOOKED: u32 = 0;
    const HOOK_ID: u16 = 3;

    /// A core hooked at reset's `pc`, parked on a hit it has not answered:
    /// the program is `addi x7, x7, 1; j .`.
    fn hit_cpu() -> (Cpu, TestBus) {
        let mut cpu = Cpu::new();
        let mut bus = TestBus::with_program(&[encode::addi(7, 7, 1), encode::jal(0, 0)]);
        let mut table = AppHookTable::new();
        table.insert(HOOKED, HOOK_ID);
        cpu.set_app_hooks(table);
        assert_eq!(cpu.regs.pc, HOOKED);
        cpu.step(&mut bus);
        (cpu, bus)
    }

    #[test]
    fn empty_table_never_hits() {
        let table = AppHookTable::new();
        assert!(table.is_empty());
        for addr in [0, 2, 0x1000_1000, 0x1000_1400, u32::MAX - 1, u32::MAX] {
            assert_eq!(table.lookup(addr), None, "{addr:#x}");
        }
    }

    #[test]
    fn insert_and_lookup_round_trip_and_replace() {
        let mut table = AppHookTable::new();
        table.insert(0x1000_1000, 1).insert(0x1000_1020, 2);
        assert_eq!(table.lookup(0x1000_1000), Some(1));
        assert_eq!(table.lookup(0x1000_1020), Some(2));
        assert_eq!(
            table.lookup(0x1000_1002),
            None,
            "inside the range, not hooked"
        );
        table.insert(0x1000_1000, 9);
        assert_eq!(table.lookup(0x1000_1000), Some(9), "an insert replaces");
        assert_eq!(table.len(), 2);
    }

    /// A realistic shape: a cluster of entries plus one address far away.
    /// Every hooked address hits, and the two ranges cover nothing between
    /// the cluster and the outlier.
    #[test]
    fn a_cluster_and_an_outlier_get_one_range_each() {
        let mut table = AppHookTable::new();
        table
            .insert(0x1000_1000, 0)
            .insert(0x1000_1020, 1)
            .insert(0x1000_1400, 2)
            .insert(0x2000_0000, 3);
        for (addr, id) in [
            (0x1000_1000, 0),
            (0x1000_1020, 1),
            (0x1000_1400, 2),
            (0x2000_0000, 3),
        ] {
            assert_eq!(table.lookup(addr), Some(id), "{addr:#x}");
        }
        assert!(table.in_ranges(0x1000_1100), "inside the cluster's range");
        assert!(
            !table.in_ranges(0x1800_0000),
            "the gap between is uncovered"
        );
        assert!(!table.in_ranges(0x1fff_fffe));
        assert!(!table.in_ranges(0x2000_0002));
        assert!(!table.in_ranges(0x1000_0ffe));
        assert!(!table.in_ranges(0x1000_1402));
    }

    #[test]
    fn a_single_entry_covers_only_itself() {
        let mut table = AppHookTable::new();
        table.insert(0x100, 5);
        assert_eq!(table.lookup(0x100), Some(5));
        assert!(!table.in_ranges(0xfe));
        assert!(!table.in_ranges(0x102));
    }

    #[test]
    fn extreme_addresses_do_not_overflow_the_ranges() {
        let mut table = AppHookTable::new();
        table.insert(0, 1).insert(u32::MAX, 2);
        assert_eq!(table.lookup(0), Some(1));
        assert_eq!(table.lookup(u32::MAX), Some(2));
        assert!(!table.in_ranges(1));
        assert!(!table.in_ranges(u32::MAX - 1));
        table.insert(0x8000_0000, 3);
        assert_eq!(table.lookup(0x8000_0000), Some(3));
        assert_eq!(table.lookup(0), Some(1));
        assert_eq!(table.lookup(u32::MAX), Some(2));
    }

    /// Swapping the table drops a pass-through nobody used: the next
    /// arrival at that address hits again and the guest code does not run.
    #[test]
    fn set_app_hooks_drops_a_pending_pass_through() {
        let (mut cpu, mut bus) = hit_cpu();
        assert!(cpu.take_hook_hit().is_some());
        cpu.clear_attention();
        cpu.hook_pass_through();

        let mut table = AppHookTable::new();
        table.insert(HOOKED, HOOK_ID);
        cpu.set_app_hooks(table);

        let info = cpu.step(&mut bus);
        assert_eq!(info.instr_len, 0, "hit, not fetched");
        assert_eq!(cpu.regs.read(7), 0, "the guest instruction did not run");
        assert_eq!(
            cpu.take_hook_hit(),
            Some(HookHit {
                id: HOOK_ID,
                pc: HOOKED
            })
        );
    }

    /// A clone taken before the driver collects the hit holds the hit and
    /// the raised flag, and the two are independent afterwards.
    #[test]
    fn a_clone_carries_a_pending_hit() {
        let (mut cpu, _bus) = hit_cpu();
        let mut clone = cpu.clone();
        let hit = HookHit {
            id: HOOK_ID,
            pc: HOOKED,
        };
        assert!(clone.needs_attention());
        assert_eq!(clone.take_hook_hit(), Some(hit));
        assert_eq!(clone.take_hook_hit(), None, "taken once");
        assert_eq!(cpu.take_hook_hit(), Some(hit), "the original keeps its own");
    }

    /// A clone taken after a pass-through is granted runs the guest code
    /// at the next arrival, once, like the original.
    #[test]
    fn a_clone_carries_a_pass_through() {
        let (mut cpu, mut bus) = hit_cpu();
        assert!(cpu.take_hook_hit().is_some());
        cpu.clear_attention();
        cpu.hook_pass_through();

        let mut clone = cpu.clone();
        for c in [&mut clone, &mut cpu] {
            let info = c.step(&mut bus);
            assert_eq!(info.instr_len, 4, "fetched and executed");
            assert_eq!(c.regs.read(7), 1);
            assert!(!c.needs_attention(), "no second hit");
        }
    }

    #[test]
    fn a_clone_is_a_snapshot() {
        let mut table = AppHookTable::new();
        table.insert(0x100, 1);
        let snap = table.clone();
        table.insert(0x200, 2);
        assert_eq!(snap.lookup(0x200), None);
        assert_eq!(table.lookup(0x200), Some(2));
    }
}

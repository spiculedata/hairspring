//! The index over the call-site records the linker collected, and the per-id merge over them. Pure functions over a
//! slice of records, so the same code serves the live section and the tests.

use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;

use crate::fmt::{ident_hash, is_ident_char, ID_MAX};
use crate::lines::EndReason;
use crate::site::{Kind, Site, WitnessSnap};

/// What the catalog index can hold; ids beyond it are dropped and counted.
pub const MAX_DISTINCT: usize = 16384;

/// What the index knows about one record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The first record with the same id, or `None` for a sentinel or a record that found no room.
    pub canon: Option<usize>,
    /// This record gets a `catalog` line: the first of its (id, kind) pair.
    pub emit: bool,
}

#[derive(Debug)]
pub struct Index {
    /// One entry per record, in section order.
    pub entries: Vec<Entry>,
    /// Distinct ids.
    pub n_ids: u64,
    /// Order-independent hash of the distinct (id, kind) pairs.
    pub hash: u64,
    /// Records that had no room in the index.
    pub excess: u64,
}

const KIND_SHIFT: u32 = 56;
const HASH_MIX: u64 = 0x9e37_79b9_7f4a_7c15;

/// The id as it is sent, which is also what two ids are compared by.
fn sent_id(site: &Site) -> Vec<u8> {
    site.id
        .as_bytes()
        .iter()
        .take(ID_MAX)
        .map(|&c| if is_ident_char(c) { c } else { b'_' })
        .collect()
}

fn pair_hash(id_hash: u64, kind: Kind) -> u64 {
    (id_hash ^ (u64::from(kind as u16) << KIND_SHIFT)).wrapping_mul(HASH_MIX)
}

/// Index the records: the first record per id is canonical, the first of each (id, kind) pair gets a `catalog` line,
/// and the distinct pairs are hashed. Records past `MAX_DISTINCT` ids have no room and are counted in `excess`.
pub fn build(sites: &[Site]) -> Index {
    let mut entries = vec![
        Entry {
            canon: None,
            emit: false,
        };
        sites.len()
    ];
    let mut by_id: HashMap<Vec<u8>, usize> = HashMap::new();
    let (mut ids, mut excess, mut hash) = (0u64, 0u64, 0u64);
    for (i, s) in sites.iter().enumerate().filter(|(_, s)| s.is_live()) {
        let key = sent_id(s);
        if let Some(&canon) = by_id.get(&key) {
            entries[i].canon = Some(canon);
        } else if by_id.len() < MAX_DISTINCT {
            let id_hash = ident_hash(s.id.as_bytes(), ID_MAX);
            by_id.insert(key, i);
            entries[i] = Entry {
                canon: Some(i),
                emit: true,
            };
            ids += 1;
            hash = hash.wrapping_add(pair_hash(id_hash, s.kind));
        } else {
            excess += 1;
        }
    }
    // A record that declares the id with a different kind is a conflict the engine must see: it gets its own catalog
    // line (once per kind) and its pair joins the hash.
    for i in 0..sites.len() {
        let Some(canon) = entries[i].canon else {
            continue;
        };
        if canon == i || sites[canon].kind == sites[i].kind {
            continue;
        }
        let earlier =
            (0..i).any(|j| entries[j].canon == Some(canon) && sites[j].kind == sites[i].kind);
        if earlier {
            continue;
        }
        entries[i].emit = true;
        hash = hash.wrapping_add(pair_hash(
            ident_hash(sites[i].id.as_bytes(), ID_MAX),
            sites[i].kind,
        ));
    }
    Index {
        entries,
        n_ids: ids,
        hash,
        excess,
    }
}

/// The counts for one record, with its same-kind siblings folded in when it is the canonical record of its id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub trues: u64,
    pub falses: u64,
    pub pick_true: Option<WitnessSnap>,
    pub pick_false: Option<WitnessSnap>,
}

/// Keep `cur` unless `cand` is an earlier evaluation by the same thread. Witnesses of different threads cannot be
/// ordered (ordinals are per thread), so the first in section order stands.
fn prefer(cur: Option<WitnessSnap>, cand: Option<WitnessSnap>) -> Option<WitnessSnap> {
    match (cur, cand) {
        (None, c) => c,
        (c, None) => c,
        (Some(cur), Some(cand)) => Some(if cand.slot == cur.slot && cand.ord < cur.ord {
            cand
        } else {
            cur
        }),
    }
}

/// Read every record once and fold same-kind siblings into the canonical record of their id. A record of another
/// kind keeps its counts to itself: folding them in would hide the conflict's size.
pub fn sum(sites: &[Site], index: &Index) -> Vec<Totals> {
    let mut totals: Vec<Totals> = sites
        .iter()
        .map(|s| {
            let (trues, falses) = (s.trues.load(Relaxed), s.falses.load(Relaxed));
            Totals {
                trues,
                falses,
                pick_true: if trues > 0 {
                    s.first_true.snapshot()
                } else {
                    None
                },
                pick_false: if falses > 0 {
                    s.first_false.snapshot()
                } else {
                    None
                },
            }
        })
        .collect();
    let own = totals.clone();
    for (i, entry) in index.entries.iter().enumerate() {
        let Some(canon) = entry.canon else {
            continue;
        };
        if canon == i || sites[canon].kind != sites[i].kind {
            continue;
        }
        let c = &mut totals[canon];
        c.trues += own[i].trues;
        c.falses += own[i].falses;
        c.pick_true = prefer(c.pick_true, own[i].pick_true);
        c.pick_false = prefer(c.pick_false, own[i].pick_false);
    }
    totals
}

/// Sum of trues and falses over the live records.
pub fn counted(sites: &[Site]) -> u64 {
    sites
        .iter()
        .filter(|s| s.is_live())
        .map(|s| s.trues.load(Relaxed) + s.falses.load(Relaxed))
        .sum()
}

/// How many of `evals` were never attributed to a record: the shortfall when it exceeds `tolerance`, else 0. A record
/// total above `evals` is not a loss: `evals` of a slot shared by many threads is a racy count.
pub fn audit_missing(evals: u64, counted: u64, tolerance: u64) -> u64 {
    let deficit = evals.saturating_sub(counted);
    if deficit > tolerance {
        deficit
    } else {
        0
    }
}

/// How many evaluations can be counted in a thread slot but not yet in a record: at most one per thread that is
/// inside a call site between its two updates. The reporting thread is not (at exit or in `report`); after a fatal
/// signal it may be, so it is counted.
pub fn in_flight_bound(reason: EndReason, threads: u64) -> u64 {
    if reason == EndReason::Signal || threads == 0 {
        threads
    } else {
        threads - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::Kind;

    fn site(id: &'static str, kind: Kind) -> Site {
        Site::new(id, "", "t.rs", 1, kind)
    }

    fn basic() -> Vec<Site> {
        vec![
            site("basic.always.ok", Kind::Always),
            site("basic.always.bad", Kind::Always),
            site("basic.sometimes", Kind::Sometimes),
            site("basic.reach.hit", Kind::Reachable),
            site("basic.reach.never", Kind::Reachable),
            site("basic.unreach.clean", Kind::Unreachable),
            site("basic.aou.never", Kind::AlwaysOrUnreachable),
        ]
    }

    #[test]
    fn the_catalog_hash_equals_the_c_sdks_for_the_same_ids_and_kinds() {
        // Printed by the C SDK's hs_sample_basic (sdks/c/tests/fixtures/hs_sample.c): "catalog_hash":"fb6cf21a75aad6ef".
        let idx = build(&basic());
        assert_eq!(idx.hash, 0xfb6c_f21a_75aa_d6ef);
        assert_eq!(idx.n_ids, 7);
        assert!(idx.entries.iter().all(|e| e.emit));
    }

    #[test]
    fn the_catalog_hash_does_not_depend_on_order() {
        let mut reversed = basic();
        reversed.reverse();
        assert_eq!(build(&reversed).hash, build(&basic()).hash);
    }

    #[test]
    fn one_id_at_two_call_sites_is_one_catalogued_id() {
        let sites = vec![
            site("a", Kind::Sometimes),
            site("b", Kind::Reachable),
            site("a", Kind::Sometimes),
        ];
        let idx = build(&sites);
        assert_eq!(idx.n_ids, 2);
        assert_eq!(idx.entries[2].canon, Some(0));
        assert!(idx.entries[0].emit && !idx.entries[2].emit);
    }

    #[test]
    fn the_sentinel_is_not_indexed() {
        let sites = vec![site("", Kind::None), site("a", Kind::Always)];
        let idx = build(&sites);
        assert_eq!(
            idx.entries[0],
            Entry {
                canon: None,
                emit: false
            }
        );
        assert_eq!(idx.n_ids, 1);
        assert_eq!(idx.excess, 0);
    }

    #[test]
    fn an_id_declared_with_two_kinds_gets_a_line_for_each_kind_once() {
        let sites = vec![
            site("c", Kind::Always),
            site("c", Kind::Sometimes),
            site("c", Kind::Sometimes),
            site("c", Kind::Always),
        ];
        let idx = build(&sites);
        assert_eq!(idx.n_ids, 1);
        let emit: Vec<bool> = idx.entries.iter().map(|e| e.emit).collect();
        assert_eq!(emit, [true, true, false, false]);
        let only_one_kind = build(&sites[..1]);
        assert_ne!(
            idx.hash, only_one_kind.hash,
            "the second pair joins the hash"
        );
    }

    #[test]
    fn ids_that_differ_only_in_bytes_the_wire_replaces_merge() {
        let sites = vec![site("a b", Kind::Always), site("a_b", Kind::Always)];
        let idx = build(&sites);
        assert_eq!(idx.n_ids, 1);
        assert_eq!(idx.entries[1].canon, Some(0));
    }

    #[test]
    fn records_beyond_the_index_are_dropped_and_counted() {
        let sites: Vec<Site> = (0..MAX_DISTINCT + 3)
            .map(|i| {
                site(
                    Box::leak(format!("id.{i}").into_boxed_str()),
                    Kind::Reachable,
                )
            })
            .collect();
        let idx = build(&sites);
        assert_eq!(idx.n_ids, MAX_DISTINCT as u64);
        assert_eq!(idx.excess, 3);
        assert_eq!(idx.entries[MAX_DISTINCT].canon, None);
    }

    fn hit(s: &Site, trues: u64, falses: u64, slot: u64, ord_t: u64, ord_f: u64) {
        s.trues.store(trues, Relaxed);
        s.falses.store(falses, Relaxed);
        if ord_t > 0 {
            s.first_true.slot.store(slot, Relaxed);
            s.first_true.ord.store(ord_t, Relaxed);
        }
        if ord_f > 0 {
            s.first_false.slot.store(slot, Relaxed);
            s.first_false.ord.store(ord_f, Relaxed);
        }
    }

    #[test]
    fn same_kind_records_are_summed_into_the_canonical_one() {
        let sites = vec![site("m", Kind::Sometimes), site("m", Kind::Sometimes)];
        hit(&sites[0], 1, 1, 0, 5, 6);
        hit(&sites[1], 2, 0, 0, 2, 0);
        let t = sum(&sites, &build(&sites));
        assert_eq!((t[0].trues, t[0].falses), (3, 1));
        // The same thread's earlier ordinal wins.
        assert_eq!(t[0].pick_true.unwrap().ord, 2);
        assert_eq!(t[0].pick_false.unwrap().ord, 6);
    }

    #[test]
    fn witnesses_of_different_threads_keep_the_first_in_section_order() {
        let sites = vec![site("m", Kind::Always), site("m", Kind::Always)];
        hit(&sites[0], 1, 0, 3, 9, 0);
        hit(&sites[1], 1, 0, 4, 1, 0);
        let t = sum(&sites, &build(&sites));
        assert_eq!(
            (t[0].pick_true.unwrap().slot, t[0].pick_true.unwrap().ord),
            (3, 9)
        );
    }

    #[test]
    fn a_record_of_another_kind_keeps_its_counts_to_itself() {
        let sites = vec![site("c", Kind::Always), site("c", Kind::Sometimes)];
        hit(&sites[0], 1, 0, 0, 1, 0);
        hit(&sites[1], 5, 0, 0, 1, 0);
        let t = sum(&sites, &build(&sites));
        assert_eq!(t[0].trues, 1);
        assert_eq!(t[1].trues, 5);
    }

    #[test]
    fn counts_without_a_stored_witness_have_no_pick() {
        let sites = vec![site("r", Kind::Reachable)];
        sites[0].trues.store(2, Relaxed);
        let t = sum(&sites, &build(&sites));
        assert_eq!(t[0].trues, 2);
        assert!(t[0].pick_true.is_none());
    }

    #[test]
    fn counted_adds_trues_and_falses_of_live_records_only() {
        let sites = vec![
            site("", Kind::None),
            site("a", Kind::Always),
            site("b", Kind::Always),
        ];
        hit(&sites[1], 2, 3, 0, 0, 0);
        hit(&sites[2], 4, 0, 0, 0, 0);
        sites[0].trues.store(100, Relaxed);
        assert_eq!(counted(&sites), 9);
    }

    #[test]
    fn a_shortfall_within_the_tolerance_is_not_a_loss() {
        assert_eq!(audit_missing(10, 10, 0), 0);
        assert_eq!(audit_missing(12, 10, 2), 0);
        assert_eq!(audit_missing(13, 10, 2), 3);
        assert_eq!(
            audit_missing(5, 10, 0),
            0,
            "a surplus in the records is not a loss"
        );
    }

    #[test]
    fn the_in_flight_bound_excludes_the_reporting_thread_except_after_a_signal() {
        assert_eq!(in_flight_bound(EndReason::Exit, 4), 3);
        assert_eq!(in_flight_bound(EndReason::Explicit, 1), 0);
        assert_eq!(in_flight_bound(EndReason::Signal, 4), 4);
        assert_eq!(in_flight_bound(EndReason::Exit, 0), 0);
    }
}

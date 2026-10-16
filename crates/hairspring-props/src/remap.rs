//! Moving a digest between catalogs. Runs of one tree can number their ids differently, so before digests from
//! several runs are folded together each is rewritten against one tree catalog.

use crate::digest::TrialDigest;
use std::collections::HashSet;

/// The map value for an index whose id the target catalog does not have.
pub const UNMAPPED_IDX: u32 = u32::MAX;

impl TrialDigest {
    /// This digest with every entry's index replaced by `map[idx]`. Pure: the digest itself is untouched.
    ///
    /// An entry is dropped, and added to `ids_dropped`, when its index is past the end of `map`, maps to
    /// [`UNMAPPED_IDX`], or maps to an index an earlier entry already took (two ids of one catalog cannot share an
    /// index; the first entry wins). The count makes the trial read as degraded, so a lost id is never mistaken for an
    /// id that was not reported. Entry order, flags, witnesses and declarations are kept.
    #[must_use]
    pub fn remap(&self, map: &[u32]) -> TrialDigest {
        let mut taken = HashSet::with_capacity(self.entries.len());
        let mut lost: u64 = 0;
        let entries = self
            .entries
            .iter()
            .filter_map(|e| {
                let target = map.get(e.idx as usize).copied();
                match target {
                    Some(idx) if idx != UNMAPPED_IDX && taken.insert(idx) => {
                        Some(crate::digest::DigestEntry { idx, ..e.clone() })
                    }
                    _ => {
                        lost += 1;
                        None
                    }
                }
            })
            .collect();
        TrialDigest {
            flags: self.flags,
            ids_dropped: self.ids_dropped.saturating_add(lost),
            entries,
        }
    }
}

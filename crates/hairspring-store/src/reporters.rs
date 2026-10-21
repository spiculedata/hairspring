//! Picking the first reporter of each seed from the rows a store returns, in whatever order it returns them. Both
//! stores read only the seed, sequence number and worker columns and leave the choice to this one rule.
use std::collections::BTreeMap;

/// One trial row as the reporter lookup reads it.
pub(crate) struct Reported {
    pub seed: u64,
    pub seq: u64,
    pub worker: String,
}

/// The lowest-sequence reporter seen so far for each seed.
#[derive(Default)]
pub(crate) struct FirstReporters {
    by_seed: BTreeMap<u64, (u64, String)>,
}

impl FirstReporters {
    pub(crate) fn offer(&mut self, row: Reported) {
        match self.by_seed.get(&row.seed) {
            Some((seq, _)) if *seq <= row.seq => {}
            _ => {
                self.by_seed.insert(row.seed, (row.seq, row.worker));
            }
        }
    }

    pub(crate) fn into_workers(self) -> BTreeMap<u64, String> {
        self.by_seed
            .into_iter()
            .map(|(seed, (_, worker))| (seed, worker))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seed: u64, seq: u64, worker: &str) -> Reported {
        Reported {
            seed,
            seq,
            worker: worker.to_owned(),
        }
    }

    #[test]
    fn the_lowest_sequence_wins_whatever_order_the_rows_arrive_in() {
        for order in [[(5, "late"), (2, "early")], [(2, "early"), (5, "late")]] {
            let mut first = FirstReporters::default();
            for (seq, worker) in order {
                first.offer(row(9, seq, worker));
            }
            assert_eq!(first.into_workers()[&9], "early");
        }
    }

    #[test]
    fn seeds_are_kept_apart() {
        let mut first = FirstReporters::default();
        first.offer(row(1, 4, "a"));
        first.offer(row(2, 1, "b"));
        let workers = first.into_workers();
        assert_eq!((workers[&1].as_str(), workers[&2].as_str()), ("a", "b"));
    }
}

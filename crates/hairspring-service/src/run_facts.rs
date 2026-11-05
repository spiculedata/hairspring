//! The figures a run's pages are built from that cost a pass over its stored rows: who reported its trials, its
//! property statistics, its sweep counts and how many trials it holds.
//!
//! Once a run is finished (`passed`, `failed`, `error` or `cancelled`) none of the rows behind them can change: the
//! store refuses trial rows for a run that is not open, in the same transaction that checks the run's state, and the
//! job's results are written in the transaction that ends the run. So a finished run's figures are worked out once and
//! kept in a [`ResultCache`], under the run row they were worked out with. A read is served from memory only when the
//! run row it has just read is equal, field by field, to that row: a run removed by `hairspring gc` reads as no row
//! and is never served. What can change for a finished run (the trust of a worker, the checks of its failures, the
//! workers that exist) is not part of these figures and is read on every call. A run that is still open is never
//! cached.
use crate::properties::{from_rows, unmet_to_unrated};
use crate::result_cache::{Bounds, Computed, ResultCache, Weigh};
use crate::types::RunTrust;
use crate::{completion_of, Result, RunCompletion, Service};
use hairspring_engine::sweep::SweepSummary;
use hairspring_props::PropertyStat;
use hairspring_store::{Contributor, RunRecord};
use std::sync::Arc;

/// Most finished runs whose figures are held at once.
pub(crate) const RUN_FACTS_MAX_ENTRIES: usize = 32;
/// Most estimated bytes of figures held at once. A run with the largest catalog (16384 ids) holds about 7 MB with short
/// ids and messages, and about 23 MB when every id, message and location is as long as allowed, so two such runs fit.
pub(crate) const RUN_FACTS_MAX_BYTES: usize = 64 * 1024 * 1024;

/// A run's statistics paired with their catalog index, sorted by id.
pub(crate) type IndexedStats = Arc<Vec<(u32, PropertyStat)>>;

pub(crate) struct RunFacts {
    trust: RunTrust,
    stats: IndexedStats,
    sweep_counts: Option<SweepSummary>,
    trials_judged: u64,
}

pub(crate) type RunFactsCache = ResultCache<RunRecord, RunFacts>;

pub(crate) fn new_cache() -> RunFactsCache {
    ResultCache::new(Bounds {
        entries: RUN_FACTS_MAX_ENTRIES,
        bytes: RUN_FACTS_MAX_BYTES,
    })
}

fn string_bytes(s: &str) -> usize {
    s.len()
}

fn stat_weight(s: &PropertyStat) -> usize {
    std::mem::size_of::<(u32, PropertyStat)>()
        + string_bytes(&s.id)
        + string_bytes(&s.msg)
        + s.loc.as_ref().map_or(0, |l| {
            string_bytes(&l.file) + l.func.as_deref().map_or(0, string_bytes)
        })
}

impl Weigh for RunFacts {
    fn weight(&self) -> usize {
        let contributors: usize = self
            .trust
            .workers
            .iter()
            .map(|c| std::mem::size_of::<Contributor>() + c.worker_id.len())
            .sum();
        let stats: usize = self.stats.iter().map(|(_, s)| stat_weight(s)).sum();
        let seeds = self.sweep_counts.as_ref().map_or(0, |s| {
            s.first_failing_seeds.len() * std::mem::size_of::<u64>()
        });
        std::mem::size_of::<RunFacts>() + contributors + stats + seeds
    }
}

impl Service {
    /// The figures of a finished run, from memory when its row is unchanged since they were worked out. `None` for a
    /// run that is still open.
    fn finished_facts(&self, run: &RunRecord) -> Result<Option<Arc<RunFacts>>> {
        if !run.state.is_terminal() {
            return Ok(None);
        }
        let facts = self.run_facts.get_or_compute(&run.id, run, || {
            let trust = self.trust_from_rows(run)?;
            let stats = self.stats_from_rows(run, || Ok(trust.all_trusted))?;
            Ok(Computed::kept(RunFacts {
                stats: Arc::new(stats),
                sweep_counts: self.sweep_counts_from_rows(run)?,
                trials_judged: self.store.count_trials(&run.id)?,
                trust,
            }))
        })?;
        Ok(Some(facts))
    }

    /// The run row, and nothing held for a run that has gone (removed by `hairspring gc`).
    pub(crate) fn stored_run(&self, run_id: &str) -> Result<Option<RunRecord>> {
        let run = self.store.get_run(run_id)?;
        if run.is_none() {
            self.run_facts.forget(run_id);
            self.coverage_cache.forget(run_id);
        }
        Ok(run)
    }

    /// Who reported the run's trials, and whether all of them were trusted when they did.
    pub(crate) fn trust_of(&self, run: &RunRecord) -> Result<RunTrust> {
        match self.finished_facts(run)? {
            Some(f) => Ok(f.trust.clone()),
            None => self.trust_from_rows(run),
        }
    }

    /// The run's statistics with their catalog index, sorted by id.
    pub(crate) fn stats_of(&self, run: &RunRecord) -> Result<IndexedStats> {
        match self.finished_facts(run)? {
            Some(f) => Ok(f.stats.clone()),
            None => Ok(Arc::new(self.stats_from_rows(run, || {
                Ok(self.trust_from_rows(run)?.all_trusted)
            })?)),
        }
    }

    /// The verdict counts of a sweep's stored rows (see [`Service::sweep_counts_from_rows`]).
    pub(crate) fn sweep_counts(&self, run: &RunRecord) -> Result<Option<SweepSummary>> {
        match self.finished_facts(run)? {
            Some(f) => Ok(f.sweep_counts.clone()),
            None => self.sweep_counts_from_rows(run),
        }
    }

    /// How many trial rows the run holds.
    pub(crate) fn trials_judged(&self, run: &RunRecord) -> Result<u64> {
        match self.finished_facts(run)? {
            Some(f) => Ok(f.trials_judged),
            None => Ok(self.store.count_trials(&run.id)?),
        }
    }

    /// How many times a finished run's figures were worked out from its rows rather than served from memory. For tests.
    #[doc(hidden)]
    pub fn run_facts_computed(&self) -> u64 {
        self.run_facts.computed()
    }

    fn trust_from_rows(&self, run: &RunRecord) -> Result<RunTrust> {
        let workers = self.store.run_contributors(&run.id)?;
        let all_trusted = !workers.is_empty() && workers.iter().all(|w| w.trusted);
        Ok(RunTrust {
            workers,
            all_trusted,
        })
    }

    /// The stored statistics, with UNMET withheld when the run ended on its time or step limit and not every
    /// contributing worker was trusted (the trust each trial row recorded when it was ingested). Decided on every
    /// read rather than when the run finished, so runs stored before this rule existed are withheld too.
    fn stats_from_rows(
        &self,
        run: &RunRecord,
        all_trusted: impl FnOnce() -> Result<bool>,
    ) -> Result<Vec<(u32, PropertyStat)>> {
        let catalog = self.store.get_catalog(&run.id)?;
        let rows = self.store.list_run_properties(&run.id)?;
        let mut stats = from_rows(&rows, &catalog);
        let completion = completion_of(run);
        if matches!(completion, RunCompletion::TimeOrStepLimited { .. })
            && !completion.allows_unmet(all_trusted()?)
        {
            stats = stats
                .into_iter()
                .map(|(idx, s)| (idx, unmet_to_unrated(s)))
                .collect();
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_props::{AssertKind, Loc, PropertyStatus};

    fn stat(id: &str) -> PropertyStat {
        PropertyStat {
            id: id.to_owned(),
            kind: AssertKind::Always,
            must_hit: false,
            msg: "message".to_owned(),
            loc: Some(Loc {
                file: "file.c".to_owned(),
                line: 1,
                func: Some("f".to_owned()),
            }),
            trials: 0,
            trials_rated: 0,
            trials_unreported: 0,
            trials_degraded: 0,
            trials_perturbing: 0,
            trials_conflicted: 0,
            trials_inconclusive_with_failures: 0,
            trials_claimable: 0,
            trials_hit: 0,
            trials_true: 0,
            trials_false: 0,
            evals: 0,
            evals_true: 0,
            evals_false: 0,
            status: PropertyStatus::Pass,
            rate: None,
            unrated_reason: None,
            first_failure: None,
        }
    }

    fn facts(ids: usize) -> RunFacts {
        RunFacts {
            trust: RunTrust {
                workers: vec![Contributor {
                    worker_id: "w".into(),
                    trusted: true,
                    trials: 1,
                }],
                all_trusted: true,
            },
            stats: Arc::new(
                (0..ids)
                    .map(|i| (i as u32, stat(&format!("id.{i}"))))
                    .collect(),
            ),
            sweep_counts: None,
            trials_judged: 0,
        }
    }

    #[test]
    fn the_weight_grows_with_every_statistic_and_its_text() {
        let one = facts(1).weight();
        let many = facts(1000).weight();
        assert!(many >= one + 999 * std::mem::size_of::<(u32, PropertyStat)>());
        let per_stat = stat_weight(&stat("id.0"));
        assert_eq!(
            per_stat,
            std::mem::size_of::<(u32, PropertyStat)>() + 4 + 7 + 6 + 1
        );
    }

    #[test]
    fn the_largest_catalog_fits_the_byte_bound_several_times() {
        let largest = facts(hairspring_props::limits::MAX_IDS_PER_RUN).weight();
        assert!(largest * 4 < RUN_FACTS_MAX_BYTES, "{largest} bytes");
    }

    #[test]
    fn the_largest_catalog_with_the_longest_text_fits_the_byte_bound_twice() {
        use hairspring_props::limits::{
            MAX_IDS_PER_RUN, MAX_ID_BYTES, MAX_LOC_FILE_BYTES, MAX_LOC_FUNC_BYTES, MAX_MSG_BYTES,
        };
        let mut f = facts(0);
        let longest = |i: usize| {
            let mut s = stat(&format!("{i:0width$}", width = MAX_ID_BYTES));
            s.msg = "m".repeat(MAX_MSG_BYTES);
            s.loc = Some(Loc {
                file: "f".repeat(MAX_LOC_FILE_BYTES),
                line: 1,
                func: Some("g".repeat(MAX_LOC_FUNC_BYTES)),
            });
            (i as u32, s)
        };
        f.stats = Arc::new((0..MAX_IDS_PER_RUN).map(longest).collect());
        let w = f.weight();
        // About 23 MB, against about 7 MB with short text.
        assert!(w > 20_000_000 && w < 26_000_000, "{w} bytes");
        assert!(
            w * 2 < RUN_FACTS_MAX_BYTES && w * 3 > RUN_FACTS_MAX_BYTES,
            "{w} bytes"
        );
    }

    #[test]
    fn what_is_held_for_a_run_that_has_gone_is_dropped_when_it_is_read() {
        let store = Arc::new(hairspring_store::SqliteStore::open_in_memory().unwrap());
        let svc = Service::new(store.clone());
        let new = hairspring_store::NewRun {
            target: "t".into(),
            backend: "sim".into(),
            seed: 1,
            config_json: "{}".into(),
        };
        let stored = hairspring_store::Store::create_run(&*store, &new, 1).unwrap();
        let gone = RunRecord {
            id: "run_gone".into(),
            state: hairspring_store::RunState::Passed,
            ..stored
        };
        svc.trust_of(&gone).unwrap();
        assert_eq!(svc.run_facts.held().0, 1);
        assert!(svc.stored_run("run_gone").unwrap().is_none());
        assert_eq!(svc.run_facts.held(), (0, 0));
    }
}

//! Conformance scenarios for the filtered, paged lists of runs and failures. Both backends must give the same rows in
//! the same order, so a client that pages through either sees every row once.
use super::trials::{batch, row, trial};
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_listing(make: &dyn Fn() -> Arc<dyn Store>) {
    runs_come_newest_first_with_the_later_insert_first_on_a_tie(&*make());
    run_pages_cover_every_row_once(&*make());
    run_filters_match_exactly_and_combine(&*make());
    failures_come_newest_first_and_page_without_gaps(&*make());
    failure_filters_match_the_run_and_the_violated_property_prefix(&*make());
}

fn start(s: &dyn Store, target: &str, mode: &str, now: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: target.into(),
            backend: "sim".into(),
            seed: 7,
            config_json: format!("{{\"mode\":\"{mode}\"}}"),
        },
        now,
    )
    .unwrap()
}

fn ids(runs: Vec<RunRecord>) -> Vec<String> {
    runs.into_iter().map(|r| r.id).collect()
}

fn runs(s: &dyn Store, filter: &RunFilter, offset: u64, limit: u32) -> Vec<String> {
    ids(s.list_runs_page(filter, offset, limit).unwrap())
}

fn all() -> RunFilter {
    RunFilter::default()
}

fn runs_come_newest_first_with_the_later_insert_first_on_a_tie(s: &dyn Store) {
    let a = start(s, "alpha", "search", 10);
    let b = start(s, "alpha", "search", 30);
    let c = start(s, "beta", "search", 30);
    let d = start(s, "beta", "search", 20);
    assert_eq!(
        runs(s, &all(), 0, 10),
        vec![c.id, b.id, d.id, a.id],
        "created_at descending; a tie goes to the run stored later"
    );
}

fn run_pages_cover_every_row_once(s: &dyn Store) {
    let made: Vec<String> = (0..7)
        .map(|i| start(s, "t", "search", 100 + i64::from(i % 3)).id)
        .collect();
    let whole = runs(s, &all(), 0, 100);
    assert_eq!(whole.len(), made.len());
    let mut paged = Vec::new();
    let mut offset = 0;
    loop {
        let page = runs(s, &all(), offset, 3);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 3);
        offset += page.len() as u64;
        paged.extend(page);
    }
    assert_eq!(paged, whole);
    assert!(runs(s, &all(), 7, 3).is_empty(), "past the end is empty");
    assert!(runs(s, &all(), 1_000_000, 3).is_empty());
    assert_eq!(runs(s, &all(), 6, 3), whole[6..].to_vec());
}

fn run_filters_match_exactly_and_combine(s: &dyn Store) {
    let a = start(s, "alpha", "search", 10);
    let b = start(s, "alpha", "sweep", 20);
    let c = start(s, "beta", "sweep", 30);
    let d = start(s, "Alpha", "search", 40);
    s.finish_run(&b.id, RunState::Passed, 1, 1, None, 25)
        .unwrap();
    s.finish_run(&c.id, RunState::Failed, 1, 1, None, 35)
        .unwrap();
    s.mark_running(&d.id, 41).unwrap();

    let by = |state: Option<RunState>, target: Option<&str>, mode: Option<&str>| RunFilter {
        state,
        target: target.map(str::to_owned),
        mode: mode.map(str::to_owned),
    };
    assert_eq!(
        runs(s, &by(Some(RunState::Queued), None, None), 0, 10),
        vec![a.id.clone()]
    );
    assert_eq!(
        runs(s, &by(Some(RunState::Running), None, None), 0, 10),
        vec![d.id.clone()]
    );
    assert_eq!(
        runs(s, &by(Some(RunState::Failed), None, None), 0, 10),
        vec![c.id.clone()]
    );
    assert!(runs(s, &by(Some(RunState::Cancelled), None, None), 0, 10).is_empty());
    assert_eq!(
        runs(s, &by(None, Some("alpha"), None), 0, 10),
        vec![b.id.clone(), a.id.clone()],
        "a target matches by exact, case-sensitive name"
    );
    assert_eq!(
        runs(s, &by(None, None, Some("sweep")), 0, 10),
        vec![c.id.clone(), b.id.clone()]
    );
    assert_eq!(
        runs(s, &by(None, None, Some("search")), 0, 10),
        vec![d.id.clone(), a.id.clone()]
    );
    assert_eq!(
        runs(
            s,
            &by(Some(RunState::Passed), Some("alpha"), Some("sweep")),
            0,
            10
        ),
        vec![b.id.clone()]
    );
    assert!(runs(s, &by(Some(RunState::Passed), Some("beta"), None), 0, 10).is_empty());
    for wild in ["%", "_", "al%", "a_pha", "alpha' OR '1'='1"] {
        assert!(
            runs(s, &by(None, Some(wild), None), 0, 10).is_empty(),
            "{wild} is a name, not a pattern"
        );
    }
    assert_eq!(
        runs(s, &by(None, Some("alpha"), None), 1, 10),
        vec![a.id],
        "the offset counts matching rows only"
    );
}

fn failure(run: &RunRecord, message: &str) -> NewFailure {
    NewFailure {
        run_id: run.id.clone(),
        target: run.target.clone(),
        message: message.into(),
        original_len: 5,
        shrunk_len: 2,
        bundle_hash: "0".repeat(64),
        steps_json: "[]".into(),
    }
}

fn failure_ids(s: &dyn Store, filter: &FailureFilter, offset: u64, limit: u32) -> Vec<String> {
    s.list_failures_page(filter, offset, limit)
        .unwrap()
        .into_iter()
        .map(|f| f.id)
        .collect()
}

fn no_filter() -> FailureFilter {
    FailureFilter::default()
}

fn failures_come_newest_first_and_page_without_gaps(s: &dyn Store) {
    let r = start(s, "t", "search", 1);
    let made: Vec<String> = [100, 300, 300, 200, 100]
        .into_iter()
        .map(|now| s.add_failure(&failure(&r, "m"), now).unwrap().id)
        .collect();
    let expected = vec![
        made[2].clone(),
        made[1].clone(),
        made[3].clone(),
        made[4].clone(),
        made[0].clone(),
    ];
    assert_eq!(failure_ids(s, &no_filter(), 0, 10), expected);
    let mut paged = Vec::new();
    for offset in (0..).step_by(2).take(4) {
        paged.extend(failure_ids(s, &no_filter(), offset, 2));
    }
    assert_eq!(paged, expected);
    assert!(failure_ids(s, &no_filter(), 5, 2).is_empty());
}

/// A finished run whose properties were stored, with one failure.
fn failed_run_with_properties(
    s: &dyn Store,
    target: &str,
    worker: &str,
    props: &[(&str, &str)],
) -> (RunRecord, String) {
    let r = start(s, target, "search", 1);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    let job = s.lease_job(worker, 600, 10).unwrap().unwrap();
    s.mark_running(&r.id, 10).unwrap();
    let catalog = props
        .iter()
        .enumerate()
        .map(|(i, (id, _))| row(i as u32, id, true))
        .collect();
    s.ingest_trials(&job.id, worker, true, &batch(catalog, vec![trial(0)]), 20)
        .unwrap();
    let rows = props
        .iter()
        .enumerate()
        .map(|(i, (_, status))| property(i as u32, status))
        .collect();
    let result = JobResult {
        state: RunState::Failed,
        steps: 1,
        trials: 1,
        error: None,
        failure: Some(failure(&r, "boom")),
        more_failures: Vec::new(),
        event_payload_json: "{}".into(),
        stats: RunStats {
            properties: rows,
            ..RunStats::default()
        },
    };
    assert_eq!(
        s.finish_job(&job.id, worker, &result, 50).unwrap(),
        FinishOutcome::Applied
    );
    let failure_id = s.list_failures(&r.id).unwrap()[0].id.clone();
    (r, failure_id)
}

fn property(idx: u32, status: &str) -> RunPropertyRow {
    RunPropertyRow {
        idx,
        status: status.into(),
        trials: 1,
        trials_rated: 1,
        trials_unreported: 0,
        trials_degraded: 0,
        trials_perturbing: 0,
        trials_conflicted: 0,
        trials_inconclusive_with_failures: 0,
        trials_claimable: 1,
        trials_hit: 1,
        trials_true: 0,
        trials_false: 1,
        evals: 1,
        evals_true: 0,
        evals_false: 1,
        rate_basis: None,
        rate_k: None,
        rate_n: None,
        first_failure_seed: None,
    }
}

fn failure_filters_match_the_run_and_the_violated_property_prefix(s: &dyn Store) {
    let (r1, f1) = failed_run_with_properties(
        s,
        "one",
        "w1",
        &[("db.commit", "violation"), ("db.read", "pass")],
    );
    let (r2, f2) = failed_run_with_properties(
        s,
        "two",
        "w2",
        &[("db.read", "violation"), ("cache.x_y", "violation")],
    );
    let (_r3, f3) = failed_run_with_properties(s, "three", "w3", &[("net.send", "pass")]);
    let with = |run: Option<&str>, prefix: Option<&str>| FailureFilter {
        run_id: run.map(str::to_owned),
        property_prefix: prefix.map(str::to_owned),
    };

    assert_eq!(
        failure_ids(s, &with(Some(&r1.id), None), 0, 10),
        vec![f1.clone()]
    );
    assert!(failure_ids(s, &with(Some("run_0000000000000000"), None), 0, 10).is_empty());
    assert_eq!(
        failure_ids(s, &with(None, Some("db.")), 0, 10),
        vec![f2.clone(), f1.clone()],
        "a run counts through a violated property only: db.read passed in the first run, but db.commit failed there"
    );
    assert_eq!(
        failure_ids(s, &with(None, Some("db.read")), 0, 10),
        vec![f2.clone()],
        "db.read passed in the first run, so only the second run's failure matches"
    );
    assert_eq!(
        failure_ids(s, &with(None, Some("net.")), 0, 10),
        Vec::<String>::new(),
        "a passing property does not link a failure"
    );
    assert_eq!(
        failure_ids(s, &with(Some(&r2.id), Some("cache.")), 0, 10),
        vec![f2.clone()]
    );
    assert!(failure_ids(s, &with(Some(&r1.id), Some("cache.")), 0, 10).is_empty());
    for pattern in ["%", "_", "db_", "DB.", "db.%", "x' OR '1'='1"] {
        assert!(
            failure_ids(s, &with(None, Some(pattern)), 0, 10).is_empty(),
            "{pattern} is a prefix, not a pattern"
        );
    }
    assert_eq!(
        failure_ids(s, &with(None, Some("cache.x_")), 0, 10),
        vec![f2.clone()],
        "an underscore in the prefix is itself"
    );
    assert_eq!(
        failure_ids(s, &no_filter(), 0, 10),
        vec![f3, f2, f1],
        "no filter lists every failure, newest first (created in the order the jobs finished)"
    );
}

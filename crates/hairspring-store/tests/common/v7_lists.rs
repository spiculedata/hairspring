//! A store's failures lists read in every way the service can ask for them, for the schema version 7 tests: the indexes
//! version 7 adds must change how the lists are read, never what they hold.
use super::listing::{failed_run_at, failed_run_with_properties};
use hairspring_store::*;

/// What [`fill`] stored: every run id, so the lists can be asked for each run.
pub struct Fixture {
    pub runs: Vec<String>,
}

fn add_failure(s: &dyn Store, run: &str, now: i64) {
    s.add_failure(
        &NewFailure {
            run_id: run.to_owned(),
            target: "t".into(),
            message: "m".into(),
            original_len: 2,
            shrunk_len: 1,
            bundle_hash: "0".repeat(64),
            steps_json: "[]".into(),
        },
        now,
    )
    .unwrap();
}

/// Runs with and without violations, at shared and distinct times, with failures stored at shared and distinct times
/// (so the order within one millisecond matters), and a queued run with nothing stored.
pub fn fill(s: &dyn Store) -> Fixture {
    let mut runs = Vec::new();
    let props: [&[(&str, &str)]; 5] = [
        &[("db.commit", "violation"), ("db.read", "pass")],
        &[("db.read", "violation"), ("cache.x_y", "violation")],
        &[("net.send", "pass")],
        &[("db.commit", "pass"), ("db.commit2", "violation")],
        &[("cache.x_y", "violation")],
    ];
    for (i, p) in props.iter().enumerate() {
        let (run, _) = if i % 2 == 0 {
            failed_run_with_properties(s, &format!("t{i}"), &format!("w{i}"), p)
        } else {
            failed_run_at(s, 40 + i as i64, &format!("t{i}"), &format!("w{i}"), p)
        };
        for (k, now) in [30, 60, 60, 45].into_iter().enumerate().take(i % 4 + 1) {
            add_failure(s, &run.id, now + k as i64 % 2);
        }
        runs.push(run.id);
    }
    runs.push(
        s.create_run(
            &NewRun {
                target: "queued".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            70,
        )
        .unwrap()
        .id,
    );
    Fixture { runs }
}

fn ids(s: &dyn Store, filter: &FailureFilter, offset: u64, limit: u32) -> Vec<String> {
    s.list_failures_page(filter, offset, limit)
        .unwrap()
        .into_iter()
        .map(|f| f.id)
        .collect()
}

/// Every filter (none, each run, each prefix, each run with each prefix, and windows of several sizes), each read
/// whole and page by page (two rows a page, from every position up to one past the end).
pub fn every_list(s: &dyn Store, fx: &Fixture) -> Vec<(String, Vec<String>)> {
    let prefixes = [
        "db.",
        "db.read",
        "db.commit",
        "cache.x_",
        "net.",
        "zz",
        "%",
        "_",
    ];
    let windows = [FAILURE_PROPERTY_RUNS, 0, 1, 2, 3, 5];
    let mut filters = vec![FailureFilter::default()];
    for run in fx.runs.iter().map(Some).chain([None]) {
        filters.push(FailureFilter {
            run_id: run.cloned(),
            ..FailureFilter::default()
        });
        for prefix in prefixes {
            for runs in windows {
                filters.push(FailureFilter {
                    run_id: run.cloned(),
                    property_prefix: Some(prefix.to_owned()),
                    property_runs: runs,
                });
            }
        }
    }
    let mut out = Vec::new();
    for filter in &filters {
        let whole = ids(s, filter, 0, 1000);
        for offset in 0..=whole.len() as u64 + 1 {
            out.push((format!("{filter:?} @{offset}"), ids(s, filter, offset, 2)));
        }
        out.push((format!("{filter:?} whole"), whole));
    }
    out
}

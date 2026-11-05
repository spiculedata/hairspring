//! Conformance scenarios for the bounded search of ids, seeds and schedule fingerprints. Both backends must give the
//! same hits in the same order, treat `%` and `_` as ordinary characters, and look only where the search says it looks:
//! the recent window of runs, plus a direct lookup of a whole run or failure id.
use super::trials::{batch, row, trial};
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_search(make: &dyn Fn() -> Arc<dyn Store>) {
    run_ids_match_whole_or_by_prefix_newest_first(&*make());
    only_the_recent_window_is_read_but_a_whole_id_is_found_at_any_age(&*make());
    failure_ids_match_whole_or_by_prefix(&*make());
    property_ids_match_by_prefix_once_each_with_their_newest_run(&*make());
    seeds_match_exactly_as_run_seeds_and_trial_seeds(&*make());
    schedule_fingerprints_match_exactly(&*make());
    text_is_never_a_pattern_and_a_group_never_exceeds_its_limit(&*make());
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        seed: None,
        fingerprint: None,
        limit: 10,
        recent_runs: SEARCH_RECENT_RUNS,
        property_runs: SEARCH_PROPERTY_RUNS,
    }
}

fn start(s: &dyn Store, target: &str, seed: u64, now: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: target.into(),
            backend: "sim".into(),
            seed,
            config_json: "{}".into(),
        },
        now,
    )
    .unwrap()
}

fn failure(run: &RunRecord) -> NewFailure {
    NewFailure {
        run_id: run.id.clone(),
        target: run.target.clone(),
        message: "boom".into(),
        original_len: 5,
        shrunk_len: 2,
        bundle_hash: "0".repeat(64),
        steps_json: "[]".into(),
    }
}

/// A run with `props` in its catalog and `trial_count` trials (trial `i` has seed `100 + i` and schedule `fp{i % 3}`).
fn ingested_run(
    s: &dyn Store,
    target: &str,
    worker: &str,
    now: i64,
    props: &[&str],
    trial_count: u64,
) -> RunRecord {
    let r = start(s, target, 7, now);
    s.enqueue_job(&r.id, "run", "{}", now).unwrap();
    let job = s.lease_job(worker, 600, now).unwrap().unwrap();
    s.mark_running(&r.id, now).unwrap();
    let catalog = props
        .iter()
        .enumerate()
        .map(|(i, id)| row(i as u32, id, true))
        .collect();
    let trials = (0..trial_count).map(trial).collect();
    s.ingest_trials(&job.id, worker, true, &batch(catalog, trials), now)
        .unwrap();
    r
}

fn run_ids(h: &SearchHits) -> Vec<String> {
    h.runs.iter().map(|r| r.id.clone()).collect()
}

fn without_last_char(id: &str) -> &str {
    &id[..id.len() - 1]
}

fn run_ids_match_whole_or_by_prefix_newest_first(s: &dyn Store) {
    let a = start(s, "alpha", 1, 10);
    let b = start(s, "beta", 2, 30);
    let c = start(s, "gamma", 3, 30);
    let d = start(s, "delta", 4, 20);

    let all = s.search(&query("run_")).unwrap();
    assert_eq!(
        run_ids(&all),
        vec![c.id.clone(), b.id.clone(), d.id.clone(), a.id.clone()],
        "newest first; a tie goes to the run stored later"
    );
    assert_eq!(
        all.runs[0],
        RunHit {
            id: c.id.clone(),
            target: "gamma".into(),
            state: "queued".into()
        }
    );
    assert_eq!(
        run_ids(&s.search(&query(&a.id)).unwrap()),
        vec![a.id.clone()],
        "a whole id"
    );
    assert_eq!(
        run_ids(&s.search(&query(without_last_char(&a.id))).unwrap()),
        vec![a.id.clone()],
        "a prefix"
    );
    assert_eq!(run_ids(&s.search(&query("r")).unwrap()).len(), 4);
    assert!(s.search(&query("run_zzzz")).unwrap().runs.is_empty());
    assert!(
        s.search(&query("RUN_")).unwrap().runs.is_empty(),
        "case matters"
    );
    assert!(
        s.search(&query("alpha")).unwrap().runs.is_empty(),
        "a target name is not a run id; the service looks names up in the configuration"
    );
}

fn only_the_recent_window_is_read_but_a_whole_id_is_found_at_any_age(s: &dyn Store) {
    let old = start(s, "t", 1, 10);
    let mid = start(s, "t", 1, 20);
    let new = start(s, "t", 1, 30);
    let (f_old, f_new) = (
        s.add_failure(&failure(&old), 11).unwrap().id,
        s.add_failure(&failure(&new), 31).unwrap().id,
    );
    let window = |n: u32, text: &str| SearchQuery {
        recent_runs: n,
        ..query(text)
    };

    assert_eq!(
        run_ids(&s.search(&window(2, "run_")).unwrap()),
        vec![new.id.clone(), mid.id.clone()],
        "a prefix reaches only the two newest runs"
    );
    assert_eq!(
        run_ids(&s.search(&window(2, &old.id)).unwrap()),
        vec![old.id.clone()],
        "a whole run id is found whatever its age"
    );
    assert!(
        s.search(&window(2, without_last_char(&old.id)))
            .unwrap()
            .runs
            .is_empty(),
        "a prefix of an older run's id is outside the window"
    );
    assert_eq!(
        s.search(&window(2, "fail_")).unwrap().failures[0].id,
        f_new,
        "failures are looked for through the runs of the window"
    );
    assert_eq!(s.search(&window(2, "fail_")).unwrap().failures.len(), 1);
    assert_eq!(
        s.search(&window(2, &f_old)).unwrap().failures[0].id,
        f_old,
        "a whole failure id is found whatever its age"
    );
    let seeded = SearchQuery {
        seed: Some(1),
        ..window(1, "zzz")
    };
    assert_eq!(
        s.search(&seeded).unwrap().seeds.len(),
        1,
        "a seed is looked for in the window only"
    );
}

fn failure_ids_match_whole_or_by_prefix(s: &dyn Store) {
    let r1 = start(s, "one", 1, 10);
    let r2 = start(s, "two", 1, 20);
    let f1 = s.add_failure(&failure(&r1), 100).unwrap().id;
    let f2 = s.add_failure(&failure(&r2), 300).unwrap().id;
    let f3 = s.add_failure(&failure(&r1), 300).unwrap().id;

    let hits = s.search(&query("fail_")).unwrap().failures;
    assert_eq!(
        hits.iter().map(|f| f.id.clone()).collect::<Vec<_>>(),
        vec![f3.clone(), f2.clone(), f1.clone()],
        "newest first; a tie goes to the failure stored later"
    );
    assert_eq!(
        hits[1],
        FailureHit {
            id: f2.clone(),
            run_id: r2.id.clone(),
            target: "two".into()
        }
    );
    let one = s.search(&query(&f1)).unwrap().failures;
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, f1);
    assert_eq!(
        s.search(&query(without_last_char(&f2)))
            .unwrap()
            .failures
            .len(),
        1
    );
    assert!(s.search(&query("fail_zzzz")).unwrap().failures.is_empty());
    assert!(s.search(&query("run_")).unwrap().failures.is_empty());
}

fn property_ids_match_by_prefix_once_each_with_their_newest_run(s: &dyn Store) {
    let r1 = ingested_run(s, "one", "w1", 10, &["db.commit", "db.read"], 1);
    let r2 = ingested_run(s, "two", "w2", 20, &["db.read", "cache.x_y"], 1);
    let r3 = ingested_run(s, "three", "w3", 30, &["net.send"], 1);

    let hit = |p: &str, r: &RunRecord| PropertyHit {
        prop_id: p.into(),
        run_id: r.id.clone(),
    };
    assert_eq!(
        s.search(&query("db.")).unwrap().properties,
        vec![hit("db.read", &r2), hit("db.commit", &r1)],
        "each id once, with the newest run that holds it; newest run first"
    );
    assert_eq!(
        s.search(&query("db.read")).unwrap().properties,
        vec![hit("db.read", &r2)]
    );
    assert_eq!(
        s.search(&query("cache.x_")).unwrap().properties,
        vec![hit("cache.x_y", &r2)],
        "an underscore is itself"
    );
    assert_eq!(
        s.search(&query("net")).unwrap().properties,
        vec![hit("net.send", &r3)]
    );
    assert!(s.search(&query("DB.")).unwrap().properties.is_empty());
    let narrow = SearchQuery {
        property_runs: 1,
        ..query("db.")
    };
    assert!(
        s.search(&narrow).unwrap().properties.is_empty(),
        "only the newest run is read: it holds no db. property"
    );
}

fn seeds_match_exactly_as_run_seeds_and_trial_seeds(s: &dyn Store) {
    let r1 = ingested_run(s, "one", "w1", 10, &["p"], 3);
    let r2 = ingested_run(s, "two", "w2", 20, &["p"], 3);
    let seed = |n: u64| SearchQuery {
        seed: Some(n),
        ..query("zzz")
    };

    assert_eq!(
        s.search(&seed(7)).unwrap().seeds,
        vec![
            SeedHit {
                run_id: r2.id.clone(),
                seed: 7,
                trial: None
            },
            SeedHit {
                run_id: r1.id.clone(),
                seed: 7,
                trial: None
            },
        ],
        "the seed a run was started with"
    );
    assert_eq!(
        s.search(&seed(101)).unwrap().seeds,
        vec![
            SeedHit {
                run_id: r2.id.clone(),
                seed: 101,
                trial: Some(1)
            },
            SeedHit {
                run_id: r1.id.clone(),
                seed: 101,
                trial: Some(1)
            },
        ],
        "the seed a trial ran, newest run first"
    );
    assert!(s.search(&seed(10)).unwrap().seeds.is_empty(), "exact only");
    assert!(s.search(&seed(1)).unwrap().seeds.is_empty());
    assert!(
        s.search(&seed(u64::MAX)).unwrap().seeds.is_empty(),
        "a seed no database holds is a miss"
    );
    assert!(s.search(&query("101")).unwrap().seeds.is_empty());
}

fn schedule_fingerprints_match_exactly(s: &dyn Store) {
    let r1 = ingested_run(s, "one", "w1", 10, &["p"], 4);
    let r2 = ingested_run(s, "two", "w2", 20, &["p"], 2);
    let fp = |f: &str| SearchQuery {
        fingerprint: Some(f.into()),
        ..query("zzz")
    };
    let hit = |f: &str, r: &RunRecord, t: u64| ScheduleHit {
        fingerprint: f.into(),
        run_id: r.id.clone(),
        trial: t,
    };

    assert_eq!(
        s.search(&fp("fp1")).unwrap().schedules,
        vec![hit("fp1", &r2, 1), hit("fp1", &r1, 1)],
        "newest run first"
    );
    assert_eq!(
        s.search(&fp("fp0")).unwrap().schedules,
        vec![hit("fp0", &r2, 0), hit("fp0", &r1, 0), hit("fp0", &r1, 3)],
        "then trial order within a run"
    );
    assert!(s.search(&fp("fp")).unwrap().schedules.is_empty());
    assert!(s.search(&fp("FP1")).unwrap().schedules.is_empty());
    assert!(s.search(&fp("fp%")).unwrap().schedules.is_empty());
    assert!(s.search(&fp("fp_")).unwrap().schedules.is_empty());
}

fn text_is_never_a_pattern_and_a_group_never_exceeds_its_limit(s: &dyn Store) {
    let r = ingested_run(s, "one", "w1", 10, &["a.b", "a_b"], 1);
    s.add_failure(&failure(&r), 11).unwrap();
    for hostile in [
        "%",
        "_",
        "run_%",
        "run__",
        "fail_%",
        "a%",
        "a_",
        "x' OR '1'='1",
        "';--",
    ] {
        let q = SearchQuery {
            seed: None,
            fingerprint: Some(hostile.into()),
            ..query(hostile)
        };
        let h = s.search(&q).unwrap();
        let found = h.runs.len() + h.failures.len() + h.seeds.len() + h.schedules.len();
        assert_eq!(found, 0, "{hostile} is text, not a pattern");
        assert!(
            h.properties.iter().all(|p| p.prop_id.starts_with(hostile)),
            "{hostile}"
        );
    }
    assert_eq!(
        s.search(&query("a_")).unwrap().properties.len(),
        1,
        "a_ is a prefix of a_b only; _ does not stand for the dot"
    );

    for _ in 0..4 {
        start(s, "more", 7, 50);
    }
    let limited = SearchQuery {
        seed: Some(7),
        limit: 2,
        ..query("run_")
    };
    let h = s.search(&limited).unwrap();
    assert_eq!((h.runs.len(), h.seeds.len()), (2, 2));
    let none = SearchQuery {
        limit: 0,
        ..query("run_")
    };
    assert!(s.search(&none).unwrap().runs.is_empty());

    // A run's own seed and a trial's seed are two groups of hits joined in one; the limit covers both together.
    start(s, "own", 100, 60);
    let both = SearchQuery {
        seed: Some(100),
        limit: 5,
        ..query("zzz")
    };
    let hits = s.search(&both).unwrap().seeds;
    assert_eq!(
        hits.iter().map(|h| h.trial).collect::<Vec<_>>(),
        vec![None, Some(0)],
        "the run that was started with the seed, then the trial that ran it"
    );
    let one = SearchQuery { limit: 1, ..both };
    assert_eq!(s.search(&one).unwrap().seeds.len(), 1);

    // A whole id outside the window is found by its own lookup, which the limit also covers.
    let old = start(s, "old", 1, 1);
    let outside = SearchQuery {
        limit: 0,
        recent_runs: 1,
        ..query(&old.id)
    };
    assert!(s.search(&outside).unwrap().runs.is_empty());
    let f = s.add_failure(&failure(&old), 2).unwrap().id;
    let outside = SearchQuery {
        limit: 0,
        recent_runs: 1,
        ..query(&f)
    };
    assert!(s.search(&outside).unwrap().failures.is_empty());
}

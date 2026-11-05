//! Results worked out from a run's stored rows are kept in memory, and are never served once those rows, or anything
//! else the answer depends on, may have changed. Every test compares the service that holds the cache with a new
//! service on the same store, which has nothing cached.
mod common;

use common::fixture::*;
use common::*;
use hairspring_core::bundle::Bundle;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::ReportedVerdict;
use hairspring_props::claim_text::{BuildRecord, Label, SeedSelection};
use hairspring_props::trial::failure_basis;
use hairspring_props::AssertKind;
use hairspring_service::{PropertyQuery, SeedQuery, Service};
use hairspring_store::{NewFailure, Store};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

fn declared() -> Label {
    Label {
        build: BuildRecord {
            instrumented: false,
            behaviour_changes: false,
        },
        knowledge: Some(vec![]),
        seed_selection: SeedSelection::Blind,
    }
}

fn json<T: serde::Serialize>(v: T) -> String {
    serde_json::to_string(&v).unwrap()
}

/// Every read the cache serves, as the JSON a caller sees.
fn views(svc: &Service, run_id: &str) -> Vec<String> {
    let page = |q: PropertyQuery| json(svc.property_page(run_id, &q, Some(&declared())).unwrap());
    let detail = |idx: u32, seeds: &str| {
        let q = SeedQuery {
            seeds: Some(seeds.to_owned()),
            ..SeedQuery::default()
        };
        json(svc.property_detail(run_id, idx, &q).unwrap())
    };
    vec![
        json(svc.get_run(run_id).unwrap()),
        json(svc.outcome(run_id).unwrap()),
        json(svc.run_trust(run_id).unwrap()),
        json(svc.property_stats(run_id).unwrap()),
        page(PropertyQuery::default()),
        page(PropertyQuery {
            status: Some("violation".into()),
            ..PropertyQuery::default()
        }),
        page(PropertyQuery {
            sort: Some("rate".into()),
            limit: Some(1),
            cursor: Some("1".into()),
            ..PropertyQuery::default()
        }),
        detail(0, "failing"),
        detail(1, "none"),
        json(svc.coverage(run_id).unwrap()),
    ]
}

/// The cached service answers exactly as a service with nothing cached.
fn assert_current(svc: &Service, store: &Arc<dyn Store>, run_id: &str) {
    let fresh = Service::new(store.clone());
    assert_eq!(views(svc, run_id), views(&fresh, run_id));
}

fn add_failure(store: &Arc<dyn Store>, run_id: &str) -> String {
    let bundle = Bundle::new(3, "t", vec![])
        .with_target("lock-lease")
        .with_param("sched_seed", "3")
        .encode();
    store
        .add_failure(
            &NewFailure {
                run_id: run_id.to_owned(),
                target: "lock-lease".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: store.put_blob(bundle.as_bytes()).unwrap(),
                steps_json: "[]".into(),
            },
            9,
        )
        .unwrap()
        .id
}

fn confirming_checks() -> Vec<CheckResult> {
    let pass = |kind| {
        CheckResult::new(kind, CheckStatus::Pass)
            .with_counts(1, 0, 0)
            .with_times(1, 2)
            .with_environment("host.container", "none")
    };
    vec![
        pass(CheckKind::SameSeedRerun),
        pass(CheckKind::BundleReplay).with_detail("environment_changed", "false"),
    ]
}

#[test]
fn a_finished_runs_figures_are_worked_out_once_for_every_read() {
    let fx = finished(&local());
    let first = views(&fx.svc, &fx.run_id);
    for _ in 0..3 {
        assert_eq!(views(&fx.svc, &fx.run_id), first);
    }
    assert_eq!(fx.svc.run_facts_computed(), 1);
    assert_current(&fx.svc, &fx.store, &fx.run_id);
}

#[test]
fn an_open_run_is_never_served_from_the_cache() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(1, &p), rec.pass(2, &p)])
        .unwrap();
    views(&svc, &job.run_id);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(3, &p)])
        .unwrap();
    assert_current(&svc, &store, &job.run_id);
    assert_eq!(svc.run_facts_computed(), 0);
}

#[test]
fn a_run_read_while_open_is_read_afresh_once_it_finishes() {
    let (svc, store, engine) = setup();
    let job = lease_sweep(&svc, &engine, &local(), 3);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(1, &p)])
        .unwrap();
    views(&svc, &job.run_id);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(2, &p), rec.pass(3, &p)])
        .unwrap();
    finish(&svc, &job, &local(), passed(3, Some(3)));
    assert_current(&svc, &store, &job.run_id);
    let detail = svc.get_run(&job.run_id).unwrap().unwrap();
    assert_eq!(detail.sweep.map(|s| s.trials), Some(3));
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 3);
}

#[test]
fn a_change_of_worker_trust_is_seen_at_once() {
    let (svc, store, engine) = setup();
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    let job = lease_with_budget(&svc, &engine, &remote.id, 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &reports)
        .unwrap();
    finish(&svc, &job, &remote.id, passed(6, Some(6)));
    add_failure(&store, &job.run_id);
    views(&svc, &job.run_id);
    svc.set_worker_trust(&remote.id, true).unwrap();
    assert_current(&svc, &store, &job.run_id);
    svc.set_worker_trust(&remote.id, false).unwrap();
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn checks_added_to_a_runs_failure_are_seen_at_once() {
    let (svc, store, engine) = setup();
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    svc.set_worker_trust(&remote.id, true).unwrap();
    let job = lease_with_budget(&svc, &engine, &remote.id, 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &reports)
        .unwrap();
    finish(&svc, &job, &remote.id, passed(6, Some(6)));
    let failure = add_failure(&store, &job.run_id);
    let before = views(&svc, &job.run_id);
    svc.record_confirmation(&failure, "cli-1", true, &confirming_checks())
        .unwrap();
    let after = views(&svc, &job.run_id);
    assert_ne!(before[0], after[0], "the run's failure carries its finding");
    assert_ne!(before[4], after[4], "the claim follows the checks");
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn a_run_removed_by_retention_is_not_served() {
    let fx = finished(&local());
    views(&fx.svc, &fx.run_id);
    let report = fx
        .store
        .retention_delete(std::slice::from_ref(&fx.run_id), i64::MAX, false)
        .unwrap();
    assert_eq!(report.run_ids, vec![fx.run_id.clone()]);
    let svc = &fx.svc;
    let id = fx.run_id.as_str();
    assert!(svc.get_run(id).unwrap().is_none());
    assert!(svc.outcome(id).unwrap().is_none());
    assert!(svc.run_trust(id).unwrap().is_none());
    assert!(svc.property_stats(id).unwrap().is_empty());
    assert!(svc
        .property_page(id, &PropertyQuery::default(), None)
        .unwrap()
        .is_none());
    assert!(svc
        .property_detail(id, 0, &SeedQuery::default())
        .unwrap()
        .is_none());
    assert!(svc.coverage(id).unwrap().is_none());
}

#[test]
fn a_new_attempt_of_an_open_run_is_never_mistaken_for_the_old_one() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    let mut rec = Recorder::new();
    let old: Vec<_> = (0..3)
        .map(|i| rec.pass(i, &props("old.id", AssertKind::Always, 1, 0)))
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &old).unwrap();
    let first = views(&svc, &job.run_id);
    svc.fail(&job.job_id, &local(), "interrupted").unwrap();
    let again = svc.lease(&local()).unwrap().expect("requeued");
    // As many trials as before, so only the attempt tells them apart.
    let mut rec = Recorder::new();
    let new: Vec<_> = [3, 6, 9]
        .iter()
        .map(|&s| rec.pass(s, &props("new.id", AssertKind::Sometimes, 0, 1)))
        .collect();
    svc.ingest_trials(&again.job_id, &local(), &new).unwrap();
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 3);
    let second = views(&svc, &job.run_id);
    assert_ne!(
        first.last(),
        second.last(),
        "the coverage is the new attempt's"
    );
    assert_current(&svc, &store, &job.run_id);
    finish(&svc, &again, &local(), passed(3, Some(3)));
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn polls_of_an_open_run_with_no_new_trial_scan_nothing() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(1, &p)])
        .unwrap();
    for _ in 0..3 {
        assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 1);
    }
    assert_eq!(svc.coverage_scans(), 1);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(2, &p)])
        .unwrap();
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 2);
    assert_eq!(svc.coverage_scans(), 2);
}

#[test]
fn a_scan_that_races_an_ingest_answers_what_it_saw_and_is_not_kept() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let p = props("c.id", AssertKind::Always, 1, 0);
    let rec = Arc::new(Mutex::new(Recorder::new()));
    let first = rec.lock().unwrap().pass(1, &p);
    svc.ingest_trials(&job.job_id, &local(), &[first]).unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    {
        let (svc2, job_id, rec, fired, p) = (
            svc.clone(),
            job.job_id.clone(),
            rec.clone(),
            fired.clone(),
            p.clone(),
        );
        svc.set_coverage_race_hook(move || {
            if !fired.swap(true, Ordering::SeqCst) {
                let late = rec.lock().unwrap().pass(2, &p);
                svc2.ingest_trials(&job_id, &local(), &[late]).unwrap();
            }
        });
    }
    // Counted one row; the scan saw two.
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 2);
    assert_eq!(svc.coverage_scans(), 1);
    assert_current(&svc, &store, &job.run_id);
    assert_eq!(svc.coverage_scans(), 2, "the raced scan was not kept");
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 2);
    assert_eq!(svc.coverage_scans(), 2, "the next one was");
}

#[test]
fn a_scan_that_races_a_new_attempt_is_not_kept_under_the_old_count() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    let mut rec = Recorder::new();
    let old: Vec<_> = (0..3)
        .map(|i| rec.pass(i, &props("old.id", AssertKind::Always, 1, 0)))
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &old).unwrap();
    let new_job = Arc::new(Mutex::new(None));
    let new_rec = Arc::new(Mutex::new(Recorder::new()));
    let new_trial = {
        let new_rec = new_rec.clone();
        move |s: u64| {
            let p = props("new.id", AssertKind::Sometimes, 0, 1);
            new_rec.lock().unwrap().pass(s, &p)
        }
    };
    {
        let (svc2, job_id, new_job) = (svc.clone(), job.job_id.clone(), new_job.clone());
        let new_trial = new_trial.clone();
        svc.set_coverage_race_hook(move || {
            let mut slot = new_job.lock().unwrap();
            if slot.is_none() {
                // Between the count (three rows of the first attempt) and the scan, a new attempt starts and reports
                // one trial: the read is made under the new attempt with the old count, and scans one row.
                svc2.fail(&job_id, &local(), "interrupted").unwrap();
                let again = svc2.lease(&local()).unwrap().expect("requeued");
                svc2.ingest_trials(&again.job_id, &local(), &[new_trial(3)])
                    .unwrap();
                *slot = Some(again);
            }
        });
    }
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 1);
    let again = new_job.lock().unwrap().clone().expect("the hook ran");
    // The new attempt reaches the count the raced read was made under; a series kept under it would be one row short.
    svc.ingest_trials(&again.job_id, &local(), &[new_trial(6), new_trial(9)])
        .unwrap();
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 3);
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 3);
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn a_waiter_is_not_handed_a_scan_that_saw_fewer_rows_than_it_counted() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    let mut rec = Recorder::new();
    let old: Vec<_> = (0..3)
        .map(|i| rec.pass(i, &props("old.id", AssertKind::Always, 1, 0)))
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &old).unwrap();
    let new_rec = Arc::new(Mutex::new(Recorder::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let new_job = Arc::new(Mutex::new(None));
    let waiter = Arc::new(Mutex::new(None));
    {
        let (svc2, job_id, run_id) = (svc.clone(), job.job_id.clone(), job.run_id.clone());
        let (new_rec, calls, new_job, waiter, store2) = (
            new_rec.clone(),
            calls.clone(),
            new_job.clone(),
            waiter.clone(),
            store.clone(),
        );
        svc.set_coverage_race_hook(move || {
            let trial = |s: u64| {
                let p = props("new.id", AssertKind::Sometimes, 0, 1);
                new_rec.lock().unwrap().pass(s, &p)
            };
            match calls.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    // The first read, after its count (three rows of the first attempt): a new attempt starts and
                    // reports one trial, so the read is made under the new attempt with a count of three.
                    svc2.fail(&job_id, &local(), "interrupted").unwrap();
                    let again = svc2.lease(&local()).unwrap().expect("requeued");
                    svc2.ingest_trials(&again.job_id, &local(), &[trial(3)])
                        .unwrap();
                    *new_job.lock().unwrap() = Some(again);
                }
                1 => {
                    // The first read, after its scan (one row) and before its outcome lands: the new attempt reaches
                    // three rows, and a second read counts them and joins the first read's flight.
                    let again = new_job.lock().unwrap().clone().unwrap();
                    svc2.ingest_trials(&again.job_id, &local(), &[trial(6), trial(9)])
                        .unwrap();
                    let (svc3, run3, store3) = (svc2.clone(), run_id.clone(), store2.clone());
                    let h = std::thread::spawn(move || {
                        let counted = store3.count_trials(&run3).unwrap();
                        let got = svc3.coverage(&run3).unwrap().unwrap().trials;
                        (counted, got)
                    });
                    *waiter.lock().unwrap() = Some(h);
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while svc2.coverage_waiters(&run_id) == 0 {
                        assert!(Instant::now() < deadline, "the second read never joined");
                        std::thread::yield_now();
                    }
                }
                _ => {}
            }
        });
    }
    assert_eq!(svc.coverage(&job.run_id).unwrap().unwrap().trials, 1);
    let (counted, got) = waiter.lock().unwrap().take().unwrap().join().unwrap();
    assert_eq!(counted, 3);
    assert_eq!(
        got, 3,
        "the second read counted three rows of this attempt before it asked"
    );
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn trials_sent_after_a_run_ended_change_nothing_the_cache_holds() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    svc.ingest_trials(&job.job_id, &local(), &[rec.pass(1, &p)])
        .unwrap();
    assert!(svc.cancel_run(&job.run_id).unwrap());
    let before = views(&svc, &job.run_id);
    // The store refuses rows for a run that has ended, whatever the service says about it.
    let _ = svc.ingest_trials(&job.job_id, &local(), &[rec.pass(2, &p)]);
    let _ = svc.complete(&job.job_id, &local(), &passed(2, Some(2)));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 1);
    assert_eq!(views(&svc, &job.run_id), before);
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn concurrent_first_reads_work_the_figures_out_once() {
    let (svc, store, engine) = setup();
    let trials = 400;
    let job = lease_sweep(&svc, &engine, &local(), trials);
    let mut rec = Recorder::new();
    let p = props("c.id", AssertKind::Always, 1, 0);
    let reports: Vec<_> = (0..trials).map(|i| rec.pass(1 + i, &p)).collect();
    for batch in reports.chunks(200) {
        svc.ingest_trials(&job.job_id, &local(), batch).unwrap();
    }
    finish(&svc, &job, &local(), passed(trials, Some(trials)));
    let readers = 8;
    let gate = Arc::new(Barrier::new(readers));
    let answers: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..readers)
            .map(|_| {
                let (svc, gate, run_id) = (svc.clone(), gate.clone(), job.run_id.clone());
                s.spawn(move || {
                    gate.wait();
                    json(svc.get_run(&run_id).unwrap())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(answers.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(svc.run_facts_computed(), 1);
    assert_current(&svc, &store, &job.run_id);
}

#[test]
fn an_errored_run_is_cached_like_any_finished_run_and_stays_current() {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), RunOutcome::error("boom"));
    views(&svc, &job.run_id);
    views(&svc, &job.run_id);
    assert_eq!(svc.run_facts_computed(), 1);
    assert_current(&svc, &store, &job.run_id);
}

/// A finished run of four trials over 30 ids of every kind, some failing and some never true.
fn many_ids() -> (Service, Arc<dyn Store>, String) {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 4);
    let ids: Vec<String> = (0..30).map(|i| format!("p{}.id{i:02}", i % 3)).collect();
    let mut rec = Recorder::new();
    let reports: Vec<_> = (0..4u64)
        .map(|t| {
            let specs: Vec<IdSpec<'_>> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let kind = AssertKind::ALL[i % AssertKind::ALL.len()];
                    let falses = u64::from((i as u64 + t).is_multiple_of(7));
                    let trues = (i as u64 + t) % 3;
                    (id.as_str(), kind, "m", trues, falses)
                })
                .collect();
            let fails = specs.iter().any(|(_, kind, _, trues, falses)| {
                failure_basis(*kind, trues + falses, *falses).is_some()
            });
            let verdict = match fails {
                true => ReportedVerdict::Violation,
                false => ReportedVerdict::Pass,
            };
            rec.trial(t + 1, &props_multi(&specs), verdict, false)
        })
        .collect();
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(4, Some(4)));
    (svc, store, job.run_id)
}

#[test]
fn pages_of_the_property_table_join_up_to_the_whole_table_for_every_filter_and_order() {
    let (svc, store, run_id) = many_ids();
    let fresh = Service::new(store.clone());
    let statuses = [
        None,
        Some("violation"),
        Some("unmet"),
        Some("unrated"),
        Some("pass"),
    ];
    let kinds = [
        None,
        Some("always"),
        Some("sometimes"),
        Some("reachable"),
        Some("unreachable"),
    ];
    let prefixes = [None, Some("p1"), Some("p"), Some("zz")];
    let sorts = [
        None,
        Some("status"),
        Some("id"),
        Some("rate"),
        Some("evals"),
    ];
    let mut tables = 0;
    for status in statuses {
        for kind in kinds {
            for prefix in prefixes {
                for sort in sorts {
                    let q = |limit: u32, cursor: Option<String>| PropertyQuery {
                        status: status.map(str::to_owned),
                        kind: kind.map(str::to_owned),
                        q: prefix.map(str::to_owned),
                        sort: sort.map(str::to_owned),
                        cursor,
                        limit: Some(limit),
                    };
                    let whole = svc
                        .property_page(&run_id, &q(500, None), None)
                        .unwrap()
                        .unwrap();
                    assert!(whole.next_cursor.is_none());
                    let want: Vec<u32> = whole.properties.iter().map(|p| p.idx).collect();
                    assert_eq!(whole.total as usize, want.len());
                    tables += usize::from(!want.is_empty());
                    for limit in [1, 4, 7] {
                        let (mut got, mut cursor) = (Vec::new(), None);
                        loop {
                            let page = svc.property_page(&run_id, &q(limit, cursor.clone()), None);
                            let page = page.unwrap().unwrap();
                            let again = fresh.property_page(&run_id, &q(limit, cursor), None);
                            assert_eq!(json(&page), json(again.unwrap().unwrap()));
                            got.extend(page.properties.iter().map(|p| p.idx));
                            match page.next_cursor {
                                Some(c) => cursor = Some(c),
                                None => break,
                            }
                        }
                        assert_eq!(
                            got, want,
                            "{status:?} {kind:?} {prefix:?} {sort:?} limit {limit}"
                        );
                    }
                }
            }
        }
    }
    assert!(tables > 50, "the filters must leave rows to page through");
    assert_eq!(svc.run_facts_computed(), 1);
}

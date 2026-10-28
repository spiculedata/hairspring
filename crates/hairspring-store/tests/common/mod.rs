//! Backend-agnostic conformance suite. Every `Store` implementation must pass all of these.
#![allow(dead_code)]
use hairspring_store::*;
use std::sync::Arc;

pub mod checks;
pub mod explorer;
#[cfg(feature = "postgres")]
pub mod pg_support;
pub mod terminal_events;
pub mod tree;
pub mod tree_attempts;
pub mod tree_downgrade;
pub mod tree_fields;
pub mod tree_gc_spec;
pub mod tree_identity;
pub mod tree_oracle;
pub mod tree_reads;
pub mod tree_support;
mod trials;
pub mod v1_schema;
pub mod v2_schema;

fn new_run(s: &dyn Store, target: &str, now: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: target.into(),
            backend: "sim".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        now,
    )
    .unwrap()
}

pub fn run_all(make: &dyn Fn() -> Arc<dyn Store>) {
    runs_lifecycle(&*make());
    run_listing_and_missing(&*make());
    failures_and_blobs(&*make());
    jobs_lease_order_and_exclusivity(&*make());
    jobs_expiry_retry_and_permanent_failure(&*make());
    jobs_wrong_worker_cannot_complete(&*make());
    events_are_ordered_and_paged(&*make());
    workers_register_and_touch(&*make());
    cancel_run_cancels_queued_job(&*make());
    hostile_strings_are_stored_verbatim(&*make());
    finish_run_is_guarded_against_terminal_overwrite(&*make());
    expired_exhausted_jobs_are_reported_for_finalisation(&*make());
    finish_job_is_atomic_and_idempotent(&*make());
    finish_job_stores_every_failure_of_a_run_in_order(&*make());
    finish_job_leaves_cancelled_runs_alone(&*make());
    counts_for_admission_control(&*make());
    stale_workers_can_be_pruned(&*make());
    concurrent_leases_never_duplicate(make());
    terminal_events::run_all_terminal_events(make);
    trials::run_all_trials(make);
    explorer::run_all_explorer(make);
    tree::run_all_trees(make);
    checks::run_all_checks(make);
}

pub fn runs_lifecycle(s: &dyn Store) {
    let r = new_run(s, "t1", 1000);
    assert_eq!(
        (r.state, r.created_at, r.started_at),
        (RunState::Queued, 1000, None)
    );
    s.mark_running(&r.id, 2000).unwrap();
    s.update_progress(&r.id, 10, 2).unwrap();
    let g = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!(
        (g.state, g.started_at, g.steps, g.trials),
        (RunState::Running, Some(2000), 10, 2)
    );
    s.finish_run(&r.id, RunState::Failed, 99, 3, Some("boom"), 3000)
        .unwrap();
    let g = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!(
        (
            g.state,
            g.finished_at,
            g.steps,
            g.trials,
            g.error.as_deref()
        ),
        (RunState::Failed, Some(3000), 99, 3, Some("boom"))
    );
}

pub fn run_listing_and_missing(s: &dyn Store) {
    let a = new_run(s, "a", 10);
    let b = new_run(s, "b", 20);
    let ids: Vec<_> = s.list_runs(10).unwrap().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![b.id.clone(), a.id.clone()]);
    assert_eq!(s.list_runs(1).unwrap().len(), 1);
    assert!(s.get_run("nope").unwrap().is_none());
    assert!(s.mark_running("nope", 1).is_err());
}

pub fn failures_and_blobs(s: &dyn Store) {
    let h = s.put_blob(b"abc").unwrap();
    assert_eq!(
        h,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        s.put_blob(b"abc").unwrap(),
        h,
        "content addressing is idempotent"
    );
    assert_eq!(s.get_blob(&h).unwrap().as_deref(), Some(&b"abc"[..]));
    assert!(s.get_blob("0000").unwrap().is_none());
    assert!(s.get_blob("not hex; drop table").unwrap().is_none());

    let r = new_run(s, "t", 1);
    let f = s
        .add_failure(
            &NewFailure {
                run_id: r.id.clone(),
                target: "t".into(),
                message: "m".into(),
                original_len: 9,
                shrunk_len: 3,
                bundle_hash: h.clone(),
                steps_json: "[]".into(),
            },
            5,
        )
        .unwrap();
    assert_eq!(s.get_failure(&f.id).unwrap().unwrap().shrunk_len, 3);
    assert_eq!(s.list_failures(&r.id).unwrap().len(), 1);
    assert_eq!(s.list_recent_failures(5).unwrap()[0].id, f.id);
    assert!(s.get_failure("nope").unwrap().is_none());
}

pub fn jobs_lease_order_and_exclusivity(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let j1 = s.enqueue_job(&r.id, "run", "{\"n\":1}", 100).unwrap();
    let j2 = s.enqueue_job(&r.id, "run", "{\"n\":2}", 200).unwrap();
    let a = s.lease_job("w1", 30, 1000).unwrap().expect("first job");
    assert_eq!(
        (
            a.id.as_str(),
            a.status,
            a.attempts,
            a.lease_owner.as_deref()
        ),
        (j1.id.as_str(), JobStatus::Running, 1, Some("w1"))
    );
    let b = s.lease_job("w2", 30, 1001).unwrap().expect("second job");
    assert_eq!(b.id, j2.id);
    assert!(
        s.lease_job("w3", 30, 1002).unwrap().is_none(),
        "nothing left"
    );
    assert!(s.complete_job(&a.id, "w1").unwrap());
    assert_eq!(s.get_job(&a.id).unwrap().unwrap().status, JobStatus::Done);
}

pub fn jobs_expiry_retry_and_permanent_failure(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let j = s.enqueue_job(&r.id, "run", "{}", 0).unwrap();
    let first = s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    assert_eq!(first.attempts, 1);
    assert!(
        s.lease_job("w2", 10, 5_000).unwrap().is_none(),
        "lease still valid"
    );
    assert!(
        s.heartbeat_job(&j.id, "w1", 100, 6_000).unwrap(),
        "owner can extend"
    );
    assert!(
        s.lease_job("w2", 10, 50_000).unwrap().is_none(),
        "extended lease still valid"
    );
    let second = s
        .lease_job("w2", 10, 200_000)
        .unwrap()
        .expect("expired lease is reclaimed");
    assert_eq!(
        (second.attempts, second.lease_owner.as_deref()),
        (2, Some("w2"))
    );
    assert!(
        !s.heartbeat_job(&j.id, "w1", 10, 200_001).unwrap(),
        "old owner lost the lease"
    );
    assert!(s.fail_job(&j.id, "w2", "transient", 200_002).unwrap());
    let third = s
        .lease_job("w3", 10, 200_003)
        .unwrap()
        .expect("requeued after failure");
    assert_eq!(third.attempts, 3);
    assert!(s.fail_job(&j.id, "w3", "still broken", 200_004).unwrap());
    let end = s.get_job(&j.id).unwrap().unwrap();
    assert_eq!(
        (end.status, end.last_error.as_deref()),
        (JobStatus::Failed, Some("still broken"))
    );
    assert!(
        s.lease_job("w4", 10, 300_000).unwrap().is_none(),
        "permanently failed jobs are not leased"
    );
}

pub fn jobs_wrong_worker_cannot_complete(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let j = s.enqueue_job(&r.id, "run", "{}", 0).unwrap();
    s.lease_job("w1", 10, 10).unwrap().unwrap();
    assert!(!s.complete_job(&j.id, "intruder").unwrap());
    assert!(!s.fail_job(&j.id, "intruder", "x", 11).unwrap());
    assert_eq!(
        s.get_job(&j.id).unwrap().unwrap().status,
        JobStatus::Running
    );
}

pub fn events_are_ordered_and_paged(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let other = new_run(s, "u", 1);
    let e1 = s.add_event(&r.id, "progress", "{\"steps\":1}", 10).unwrap();
    let e2 = s.add_event(&r.id, "progress", "{\"steps\":2}", 11).unwrap();
    s.add_event(&other.id, "progress", "{}", 12).unwrap();
    let e3 = s.add_event(&r.id, "done", "{}", 13).unwrap();
    assert!(e1 < e2 && e2 < e3);
    let all = s.list_events(&r.id, 0, 100).unwrap();
    assert_eq!(
        all.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        ["progress", "progress", "done"]
    );
    let after = s.list_events(&r.id, e1, 100).unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(s.list_events(&r.id, 0, 2).unwrap().len(), 2);
}

pub fn workers_register_and_touch(s: &dyn Store) {
    let w = s
        .register_worker("box-1", "{\"backends\":[\"sim\"]}", 100)
        .unwrap();
    s.touch_worker(&w.id, 500).unwrap();
    let all = s.list_workers().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].name.as_str(), all[0].last_seen), ("box-1", 500));
}

pub fn cancel_run_cancels_queued_job(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    assert!(s.cancel_run(&r.id, 3).unwrap());
    assert_eq!(
        s.get_run(&r.id).unwrap().unwrap().state,
        RunState::Cancelled
    );
    assert!(
        s.lease_job("w", 10, 4).unwrap().is_none(),
        "cancelled run's job is not leasable"
    );
    assert!(!s.cancel_run(&r.id, 5).unwrap(), "already terminal");
}

pub fn hostile_strings_are_stored_verbatim(s: &dyn Store) {
    let evil = "x'); DROP TABLE runs;-- \u{0} \u{1F600} \n\"quoted\"";
    let evil = evil.replace('\u{0}', ""); // NUL is not valid in TEXT on every backend
    let r = s
        .create_run(
            &NewRun {
                target: evil.clone(),
                backend: "sim".into(),
                seed: u64::MAX >> 1,
                config_json: evil.clone(),
            },
            1,
        )
        .unwrap();
    let g = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!((g.target, g.config_json), (evil.clone(), evil));
    assert!(
        s.list_runs(5).unwrap().len() == 1,
        "table still exists and has the row"
    );
}

pub fn concurrent_leases_never_duplicate(s: Arc<dyn Store>) {
    let r = new_run(&*s, "t", 1);
    for i in 0..40 {
        s.enqueue_job(&r.id, "run", &format!("{{\"i\":{i}}}"), i)
            .unwrap();
    }
    let handles: Vec<_> = (0..4)
        .map(|w| {
            let s = s.clone();
            std::thread::spawn(move || {
                let mut got = vec![];
                while let Some(j) = s.lease_job(&format!("w{w}"), 60, 1_000).unwrap() {
                    got.push(j.id);
                }
                got
            })
        })
        .collect();
    let mut all: Vec<String> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    assert_eq!(all.len(), 40);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 40, "every job leased exactly once");
}

pub fn finish_run_is_guarded_against_terminal_overwrite(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    s.mark_running(&r.id, 2).unwrap();
    assert!(s
        .finish_run(&r.id, RunState::Passed, 5, 1, None, 3)
        .unwrap());
    assert!(
        !s.finish_run(&r.id, RunState::Error, 0, 0, Some("late"), 4)
            .unwrap(),
        "a finished run cannot be overwritten"
    );
    let g = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!((g.state, g.steps, g.error), (RunState::Passed, 5, None));
}

pub fn expired_exhausted_jobs_are_reported_for_finalisation(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let j = s.enqueue_job(&r.id, "run", "{}", 0).unwrap();
    for (n, t) in [1_000, 20_000, 40_000].into_iter().enumerate() {
        assert_eq!(
            s.lease_job("w", 10, t).unwrap().expect("leased").attempts as usize,
            n + 1
        );
    }
    assert!(
        s.lease_job("w", 10, 60_000).unwrap().is_none(),
        "attempts exhausted"
    );
    assert!(
        s.fail_expired_jobs(30_000).unwrap().is_empty(),
        "lease from attempt 3 has not expired yet"
    );
    let expired = s.fail_expired_jobs(60_000).unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(
        (expired[0].job_id.as_str(), expired[0].run_id.as_str()),
        (j.id.as_str(), r.id.as_str())
    );
    assert_eq!(s.get_job(&j.id).unwrap().unwrap().status, JobStatus::Failed);
    assert!(
        s.fail_expired_jobs(70_000).unwrap().is_empty(),
        "reported exactly once"
    );
}

fn result(state: RunState, failure: Option<NewFailure>) -> JobResult {
    JobResult {
        state,
        steps: 42,
        trials: 3,
        error: None,
        failure,
        more_failures: Vec::new(),
        event_payload_json: format!("{{\"state\":\"{}\"}}", state.as_str()),
        stats: RunStats::default(),
    }
}

/// The run id in the failure is deliberately bogus: the store must attach failures to the job's own run.
fn failure_for(_run_id: &str) -> NewFailure {
    NewFailure {
        run_id: "run_ignored".into(),
        target: "t".into(),
        message: "m".into(),
        original_len: 9,
        shrunk_len: 3,
        bundle_hash: "h".into(),
        steps_json: "[]".into(),
    }
}

pub fn finish_job_is_atomic_and_idempotent(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w1", 60, 10).unwrap().unwrap();
    s.mark_running(&r.id, 10).unwrap();

    let r_fail = result(RunState::Failed, Some(failure_for(&r.id)));
    assert_eq!(
        s.finish_job(&job.id, "intruder", &r_fail, 20).unwrap(),
        FinishOutcome::NotLeased
    );
    assert!(
        s.list_failures(&r.id).unwrap().is_empty(),
        "nothing written for a non-owner"
    );
    assert_eq!(s.get_run(&r.id).unwrap().unwrap().state, RunState::Running);

    assert_eq!(
        s.finish_job(&job.id, "w1", &r_fail, 30).unwrap(),
        FinishOutcome::Applied
    );
    let g = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!(
        (g.state, g.steps, g.trials, g.finished_at),
        (RunState::Failed, 42, 3, Some(30))
    );
    let fails = s.list_failures(&r.id).unwrap();
    assert_eq!(fails.len(), 1);
    assert_eq!(
        fails[0].run_id, r.id,
        "failure is attached to the job's run, not a caller-supplied one"
    );
    let kinds: Vec<_> = s
        .list_events(&r.id, 0, 10)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, vec!["finished"]);
    assert_eq!(s.get_job(&job.id).unwrap().unwrap().status, JobStatus::Done);

    assert_eq!(
        s.finish_job(&job.id, "w1", &r_fail, 40).unwrap(),
        FinishOutcome::NotLeased,
        "second call is a no-op"
    );
    assert_eq!(
        s.list_failures(&r.id).unwrap().len(),
        1,
        "no duplicate failure"
    );
    assert_eq!(
        s.list_events(&r.id, 0, 10).unwrap().len(),
        1,
        "no duplicate event"
    );
}

pub fn finish_job_stores_every_failure_of_a_run_in_order(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w1", 60, 10).unwrap().unwrap();
    s.mark_running(&r.id, 10).unwrap();
    let named = |message: &str| NewFailure {
        message: message.into(),
        ..failure_for(&r.id)
    };
    let done = JobResult {
        more_failures: vec![named("second"), named("third")],
        ..result(RunState::Failed, Some(named("first")))
    };
    assert_eq!(
        s.finish_job(&job.id, "w1", &done, 30).unwrap(),
        FinishOutcome::Applied
    );
    let stored = s.list_failures(&r.id).unwrap();
    let messages: Vec<_> = stored.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(messages, vec!["first", "second", "third"]);
    assert!(stored.iter().all(|f| f.run_id == r.id));
    let ids: std::collections::BTreeSet<_> = stored.iter().map(|f| &f.id).collect();
    assert_eq!(ids.len(), 3, "each failure has an id of its own");
}

pub fn finish_job_leaves_cancelled_runs_alone(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w1", 60, 10).unwrap().unwrap();
    s.mark_running(&r.id, 10).unwrap();
    assert!(s.cancel_run(&r.id, 15).unwrap());
    let out = s
        .finish_job(
            &job.id,
            "w1",
            &result(RunState::Failed, Some(failure_for(&r.id))),
            20,
        )
        .unwrap();
    assert_eq!(out, FinishOutcome::RunAlreadyFinal);
    assert_eq!(
        s.get_run(&r.id).unwrap().unwrap().state,
        RunState::Cancelled
    );
    assert!(s.list_failures(&r.id).unwrap().is_empty());
    assert!(s.list_events(&r.id, 0, 10).unwrap().is_empty());
    assert_eq!(
        s.get_job(&job.id).unwrap().unwrap().status,
        JobStatus::Done,
        "the job itself is closed"
    );
}

pub fn counts_for_admission_control(s: &dyn Store) {
    assert_eq!(s.queued_job_count().unwrap(), 0);
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 1).unwrap();
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    assert_eq!(s.queued_job_count().unwrap(), 2);
    s.lease_job("w", 10, 5).unwrap().unwrap();
    assert_eq!(
        s.queued_job_count().unwrap(),
        1,
        "leased jobs are not queued"
    );
    assert_eq!(s.worker_count().unwrap(), 0);
    s.register_worker("a", "{}", 1).unwrap();
    assert_eq!(s.worker_count().unwrap(), 1);
}

pub fn stale_workers_can_be_pruned(s: &dyn Store) {
    let old = s.register_worker("old", "{}", 100).unwrap();
    let fresh = s.register_worker("fresh", "{}", 100).unwrap();
    s.touch_worker(&fresh.id, 10_000).unwrap();
    assert_eq!(
        s.prune_workers(5_000).unwrap(),
        1,
        "only the worker not seen since before the cutoff"
    );
    let names: Vec<_> = s
        .list_workers()
        .unwrap()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert_eq!(names, vec!["fresh".to_string()]);
    assert!(s.touch_worker(&old.id, 1).is_err(), "pruned worker is gone");
}

//! Backend-agnostic conformance suite. Every `Store` implementation must pass all of these.
#![allow(dead_code)]
use anthill_store::*;
use std::sync::Arc;

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
    concurrent_leases_never_duplicate(make());
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

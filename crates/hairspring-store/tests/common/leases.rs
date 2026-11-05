//! Conformance scenarios for the read-only view of live leases. Both backends must agree on which leases count as
//! live, how they are ordered and what is left out.
use super::new_run;
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_leases(make: &dyn Fn() -> Arc<dyn Store>) {
    no_jobs_means_no_leases(&*make());
    only_live_leases_are_listed_oldest_job_first(&*make());
    the_limit_caps_the_rows_but_not_the_count(&*make());
    finished_failed_and_queued_jobs_hold_no_lease(&*make());
}

fn no_jobs_means_no_leases(s: &dyn Store) {
    assert!(s.leased_jobs(100, 10).unwrap().is_empty());
    assert_eq!(s.leased_job_count(100).unwrap(), 0);
}

fn only_live_leases_are_listed_oldest_job_first(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    s.enqueue_job(&r.id, "run", "{}", 1).unwrap();
    s.enqueue_job(&r.id, "confirm", "{}", 2).unwrap();
    s.enqueue_job(&r.id, "run", "{}", 3).unwrap();
    // Each lease is for 10 seconds, so these end at 10_005, 10_006 and 10_007 milliseconds.
    s.lease_job("w-a", 10, 5).unwrap().unwrap();
    s.lease_job("w-b", 10, 6).unwrap().unwrap();
    s.lease_job("w-a", 10, 7).unwrap().unwrap();
    let live = s.leased_jobs(10_006, 10).unwrap();
    let seen: Vec<(&str, &str, i64)> = live
        .iter()
        .map(|j| (j.lease_owner.as_str(), j.kind.as_str(), j.lease_until))
        .collect();
    assert_eq!(
        seen,
        vec![("w-b", "confirm", 10_006), ("w-a", "run", 10_007)],
        "a lease that ended before now is not live; one ending exactly now still is"
    );
    assert!(live.iter().all(|j| j.run_id == r.id && j.attempts == 1));
    assert_eq!(s.leased_job_count(10_006).unwrap(), 2);
    assert_eq!(s.leased_job_count(10_008).unwrap(), 0);
}

fn the_limit_caps_the_rows_but_not_the_count(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    for i in 0..4 {
        s.enqueue_job(&r.id, "run", "{}", i).unwrap();
        s.lease_job(&format!("w{i}"), 60, 10).unwrap().unwrap();
    }
    assert_eq!(s.leased_jobs(20, 3).unwrap().len(), 3);
    assert_eq!(s.leased_job_count(20).unwrap(), 4);
}

fn finished_failed_and_queued_jobs_hold_no_lease(s: &dyn Store) {
    let r = new_run(s, "t", 1);
    let done = s.enqueue_job(&r.id, "run", "{}", 1).unwrap();
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    s.lease_job("w", 60, 10).unwrap().unwrap();
    assert!(s.complete_job(&done.id, "w").unwrap());
    // The second job stays queued: nobody has leased it.
    let live = s.leased_jobs(20, 10).unwrap();
    assert!(
        live.is_empty(),
        "done and queued jobs hold no lease: {live:?}"
    );
    assert_eq!(s.leased_job_count(20).unwrap(), 0);
}

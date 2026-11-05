//! The paged, filtered lists of runs and failures: what a query may say, what comes back, and that a page never
//! holds a row twice or leaves one out.
mod common;

use common::setup;
use hairspring_engine::mode::RunMode;
use hairspring_service::{
    FailureListQuery, RunListQuery, ServiceError, StartRun, DEFAULT_LIST_PAGE, MAX_LIST_OFFSET,
    MAX_LIST_PAGE,
};
use hairspring_store::{NewCheck, NewConfirmation, NewFailure, RunState, Store};

fn start(
    svc: &hairspring_service::Service,
    engine: &hairspring_engine::engine::Engine,
    mode: RunMode,
) -> String {
    svc.start_run_with_mode(
        engine,
        &StartRun {
            target: "lock-lease".into(),
            seed: Some(1),
            budget: None,
        },
        mode,
    )
    .unwrap()
    .id
}

fn run_ids(page: &hairspring_service::RunPage) -> Vec<String> {
    page.runs.iter().map(|r| r.id.clone()).collect()
}

fn refused(result: Result<impl std::fmt::Debug, ServiceError>, what: &str) {
    match result {
        Err(ServiceError::InvalidRequest(m)) => {
            assert!(m.contains(&format!("`{what}`")), "{m}");
            assert!(!m.contains("\u{0}"), "{m:?}");
        }
        other => panic!("expected `{what}` to be refused, got {other:?}"),
    }
}

fn query(f: impl FnOnce(&mut RunListQuery)) -> RunListQuery {
    let mut q = RunListQuery::default();
    f(&mut q);
    q
}

#[test]
fn the_default_page_is_documented_and_the_limit_is_clamped() {
    let (svc, _store, engine) = setup();
    for _ in 0..3 {
        start(&svc, &engine, RunMode::Search);
    }
    assert_eq!(DEFAULT_LIST_PAGE, 50);
    assert_eq!(MAX_LIST_PAGE, 500);
    assert_eq!(
        svc.run_page(&RunListQuery::default()).unwrap().runs.len(),
        3
    );
    let one = svc.run_page(&query(|q| q.limit = Some(0))).unwrap();
    assert_eq!(one.runs.len(), 1, "a limit of 0 is raised to 1");
    assert_eq!(one.next_cursor.as_deref(), Some("1"));
    let all = svc.run_page(&query(|q| q.limit = Some(u32::MAX))).unwrap();
    assert_eq!((all.runs.len(), all.next_cursor), (3, None));
}

#[test]
fn pages_follow_next_cursor_without_repeats_or_gaps() {
    let (svc, _store, engine) = setup();
    let made: Vec<String> = (0..5)
        .map(|_| start(&svc, &engine, RunMode::Search))
        .collect();
    let whole = run_ids(&svc.run_page(&RunListQuery::default()).unwrap());
    assert_eq!(whole.len(), made.len());
    let mut walked = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = svc
            .run_page(&query(|q| {
                q.limit = Some(2);
                q.cursor = cursor.clone();
            }))
            .unwrap();
        walked.extend(run_ids(&page));
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!((walked, pages), (whole, 3));
}

#[test]
fn a_page_that_ends_exactly_at_the_last_row_has_no_cursor() {
    let (svc, _store, engine) = setup();
    for _ in 0..4 {
        start(&svc, &engine, RunMode::Search);
    }
    let page = svc.run_page(&query(|q| q.limit = Some(4))).unwrap();
    assert_eq!((page.runs.len(), page.next_cursor), (4, None));
    let past = svc
        .run_page(&query(|q| {
            q.cursor = Some("4".into());
            q.limit = Some(2);
        }))
        .unwrap();
    assert_eq!((past.runs.len(), past.next_cursor), (0, None));
}

#[test]
fn runs_filter_by_state_target_and_the_stored_mode() {
    let (svc, store, engine) = setup();
    let queued = start(&svc, &engine, RunMode::Search);
    let sweep = start(&svc, &engine, RunMode::Sweep);
    let cancelled = start(&svc, &engine, RunMode::Search);
    assert!(store.cancel_run(&cancelled, 99).unwrap());
    let ids = |f: &dyn Fn(&mut RunListQuery)| run_ids(&svc.run_page(&query(|q| f(q))).unwrap());

    assert_eq!(
        ids(&|q| q.state = Some("cancelled".into())),
        vec![cancelled.clone()]
    );
    assert_eq!(
        ids(&|q| q.state = Some("queued".into())),
        vec![sweep.clone(), queued.clone()]
    );
    assert_eq!(ids(&|q| q.mode = Some("sweep".into())), vec![sweep.clone()]);
    assert_eq!(
        ids(&|q| q.mode = Some("search".into())),
        vec![cancelled.clone(), queued.clone()]
    );
    assert_eq!(
        ids(&|q| {
            q.target = Some("lock-lease".into());
            q.state = Some("queued".into());
            q.mode = Some("search".into());
        }),
        vec![queued.clone()]
    );
    assert!(ids(&|q| q.target = Some("no-such-target".into())).is_empty());
    let nothing = svc
        .run_page(&query(|q| q.state = Some("failed".into())))
        .unwrap();
    assert_eq!((nothing.runs.len(), nothing.next_cursor), (0, None));
    assert_eq!(
        store.get_run(&queued).unwrap().unwrap().state,
        RunState::Queued
    );
}

#[test]
fn run_queries_refuse_what_is_not_valid_and_name_only_the_parameter() {
    let (svc, _store, _engine) = setup();
    let over_cap = (MAX_LIST_OFFSET + 1).to_string();
    let bad: [(&str, RunListQuery); 12] = [
        ("state", query(|q| q.state = Some("done".into()))),
        ("state", query(|q| q.state = Some("QUEUED".into()))),
        ("state", query(|q| q.state = Some(String::new()))),
        ("mode", query(|q| q.mode = Some("sweeps".into()))),
        ("mode", query(|q| q.mode = Some(String::new()))),
        ("target", query(|q| q.target = Some(String::new()))),
        ("target", query(|q| q.target = Some("x".repeat(65)))),
        ("target", query(|q| q.target = Some("a\u{0}b".into()))),
        ("target", query(|q| q.target = Some("line\nbreak".into()))),
        ("cursor", query(|q| q.cursor = Some("-1".into()))),
        ("cursor", query(|q| q.cursor = Some("12abc".into()))),
        ("cursor", query(|q| q.cursor = Some(over_cap.clone()))),
    ];
    for (what, q) in bad {
        refused(svc.run_page(&q), what);
    }
    let ok = svc.run_page(&query(|q| q.cursor = Some(MAX_LIST_OFFSET.to_string())));
    assert!(ok.is_ok(), "the largest allowed cursor is fine: {ok:?}");
}

fn failure_of(store: &dyn Store, run: &str, now: i64) -> String {
    store
        .add_failure(
            &NewFailure {
                run_id: run.to_owned(),
                target: "lock-lease".into(),
                message: "boom".into(),
                original_len: 4,
                shrunk_len: 2,
                bundle_hash: "0".repeat(64),
                steps_json: "[]".into(),
            },
            now,
        )
        .unwrap()
        .id
}

fn check(kind: &str, status: &str) -> NewCheck {
    NewCheck {
        kind: kind.into(),
        status: status.into(),
        passed: 1,
        failed: 0,
        inconclusive: 0,
        command: "hermit run".into(),
        detail_json: "{}".into(),
        artefacts_json: "[]".into(),
        environment_json: "{}".into(),
        started_at: 1,
        finished_at: 2,
    }
}

fn confirm(store: &dyn Store, failure: &str, checks: Vec<NewCheck>) {
    let id = NewConfirmation::content_id(failure, "runner", true, &checks);
    store
        .add_confirmation(
            &NewConfirmation {
                id,
                failure_id: failure.to_owned(),
                runner: "runner".into(),
                runner_trusted: true,
                checks,
            },
            10,
        )
        .unwrap();
}

fn failure_ids(page: &hairspring_service::FailurePage) -> Vec<String> {
    page.failures.iter().map(|f| f.id.clone()).collect()
}

fn fquery(f: impl FnOnce(&mut FailureListQuery)) -> FailureListQuery {
    let mut q = FailureListQuery::default();
    f(&mut q);
    q
}

/// Five failures, oldest first: candidate, confirmed, not reproducible, confirmed, candidate.
fn five_failures(
    svc: &hairspring_service::Service,
    store: &dyn Store,
    engine: &hairspring_engine::engine::Engine,
) -> (String, Vec<String>) {
    let run = start(svc, engine, RunMode::Search);
    let ids: Vec<String> = (0..5).map(|i| failure_of(store, &run, 100 + i)).collect();
    let confirmed = vec![
        check("same_seed_rerun", "pass"),
        check("bundle_replay", "pass"),
    ];
    confirm(store, &ids[1], confirmed.clone());
    confirm(store, &ids[3], confirmed);
    confirm(store, &ids[2], vec![check("same_seed_rerun", "fail")]);
    (run, ids)
}

#[test]
fn failures_page_newest_first_and_leave_the_finding_out_unless_a_status_is_asked_for() {
    let (svc, store, engine) = setup();
    let (_run, ids) = five_failures(&svc, &*store, &engine);
    let newest_first: Vec<String> = ids.iter().rev().cloned().collect();
    let first = svc.failure_page(&fquery(|q| q.limit = Some(2))).unwrap();
    assert_eq!(failure_ids(&first), newest_first[..2].to_vec());
    assert!(first.failures.iter().all(|f| f.finding.is_none()));
    let second = svc
        .failure_page(&fquery(|q| {
            q.limit = Some(2);
            q.cursor = first.next_cursor.clone();
        }))
        .unwrap();
    assert_eq!(failure_ids(&second), newest_first[2..4].to_vec());
    let third = svc
        .failure_page(&fquery(|q| {
            q.limit = Some(2);
            q.cursor = second.next_cursor.clone();
        }))
        .unwrap();
    assert_eq!(
        (failure_ids(&third), third.next_cursor),
        (newest_first[4..].to_vec(), None)
    );
}

#[test]
fn a_failure_page_that_ends_exactly_at_the_last_failure_has_no_cursor() {
    let (svc, store, engine) = setup();
    five_failures(&svc, &*store, &engine);
    let all = svc.failure_page(&fquery(|q| q.limit = Some(5))).unwrap();
    assert_eq!((all.failures.len(), all.next_cursor), (5, None));
    let tail = svc
        .failure_page(&fquery(|q| {
            q.limit = Some(2);
            q.cursor = Some("3".into());
        }))
        .unwrap();
    assert_eq!((tail.failures.len(), tail.next_cursor), (2, None));
    let short = svc.failure_page(&fquery(|q| q.limit = Some(4))).unwrap();
    assert_eq!(short.next_cursor.as_deref(), Some("4"));
}

#[test]
fn a_status_filter_counts_only_failures_in_that_state_and_pages_over_them() {
    let (svc, store, engine) = setup();
    let (_run, ids) = five_failures(&svc, &*store, &engine);
    let by = |status: &str, limit: u32, cursor: Option<String>| {
        svc.failure_page(&fquery(|q| {
            q.status = Some(status.into());
            q.limit = Some(limit);
            q.cursor = cursor;
        }))
        .unwrap()
    };

    let first = by("confirmed", 1, None);
    assert_eq!(failure_ids(&first), vec![ids[3].clone()]);
    let finding = first.failures[0]
        .finding
        .as_ref()
        .expect("the finding is known when filtering by it");
    assert_eq!((finding.status.as_str(), finding.checks), ("confirmed", 2));
    assert!(
        first.next_cursor.is_some(),
        "another confirmed failure follows"
    );
    let second = by("confirmed", 1, first.next_cursor);
    assert_eq!(failure_ids(&second), vec![ids[1].clone()]);
    assert_eq!(second.next_cursor, None, "nothing confirmed is left");

    assert_eq!(
        failure_ids(&by("candidate", 10, None)),
        vec![ids[4].clone(), ids[0].clone()]
    );
    assert_eq!(
        failure_ids(&by("not_reproducible", 10, None)),
        vec![ids[2].clone()]
    );
    assert!(by("verified", 10, None).failures.is_empty());
}

#[test]
fn failures_filter_by_run_and_refuse_what_is_not_valid() {
    let (svc, store, engine) = setup();
    let (run, ids) = five_failures(&svc, &*store, &engine);
    let other = start(&svc, &engine, RunMode::Search);
    let only = failure_of(&*store, &other, 500);

    let page = svc
        .failure_page(&fquery(|q| q.run = Some(other.clone())))
        .unwrap();
    assert_eq!(failure_ids(&page), vec![only]);
    let page = svc
        .failure_page(&fquery(|q| q.run = Some(run.clone())))
        .unwrap();
    assert_eq!(page.failures.len(), ids.len());
    let missing = svc
        .failure_page(&fquery(|q| q.run = Some("run_0000000000000000".into())))
        .unwrap();
    assert!(missing.failures.is_empty());

    let bad: [(&str, FailureListQuery); 12] = [
        ("run", fquery(|q| q.run = Some("nope".into()))),
        ("run", fquery(|q| q.run = Some(String::new()))),
        ("run", fquery(|q| q.run = Some(format!("{run}0")))),
        ("property", fquery(|q| q.property = Some(String::new()))),
        ("property", fquery(|q| q.property = Some("x".repeat(129)))),
        ("property", fquery(|q| q.property = Some("a\u{0}".into()))),
        ("property", fquery(|q| q.property = Some("a\tb".into()))),
        ("status", fquery(|q| q.status = Some("reproduced".into()))),
        ("status", fquery(|q| q.status = Some("Confirmed".into()))),
        ("status", fquery(|q| q.status = Some(String::new()))),
        ("cursor", fquery(|q| q.cursor = Some("abc".into()))),
        (
            "cursor",
            fquery(|q| q.cursor = Some((MAX_LIST_OFFSET + 1).to_string())),
        ),
    ];
    for (what, q) in bad {
        refused(svc.failure_page(&q), what);
    }
}

#[test]
fn an_error_never_echoes_the_value_it_refused() {
    let (svc, _store, _engine) = setup();
    let secret = "state-value-that-must-not-come-back";
    match svc.run_page(&query(|q| q.state = Some(secret.into()))) {
        Err(ServiceError::InvalidRequest(m)) => assert!(!m.contains(secret), "{m}"),
        other => panic!("{other:?}"),
    }
}

//! Conformance scenarios for ending a run together with its terminal event: the two are one write, so a reader that
//! sees the run ended also sees the event.
#![allow(dead_code)]
use hairspring_store::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const ROUNDS: usize = 60;

pub fn run_all_terminal_events(make: &dyn Fn() -> Arc<dyn Store>) {
    cancelling_writes_the_event_with_the_state(&*make());
    finalising_as_error_writes_the_event_with_the_state(&*make());
    a_run_that_already_ended_gets_no_second_event(&*make());
    a_missing_run_is_an_error(&*make());
    a_reader_never_sees_a_cancelled_run_without_its_event(make());
    a_reader_never_sees_an_errored_run_without_its_event(make());
    the_reader_detects_a_state_written_before_its_event(make());
}

fn new_run(s: &dyn Store) -> RunRecord {
    s.create_run(
        &NewRun {
            target: "t".into(),
            backend: "sim".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        1,
    )
    .unwrap()
}

fn kinds(s: &dyn Store, run_id: &str) -> Vec<String> {
    s.list_events(run_id, 0, 100)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

fn event<'a>(kind: &'a str) -> NewEvent<'a> {
    NewEvent {
        kind,
        payload_json: "{}",
    }
}

fn cancelling_writes_the_event_with_the_state(s: &dyn Store) {
    let r = new_run(s);
    s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
    assert!(s
        .cancel_run_with_event(&r.id, &event("cancelled"), 3)
        .unwrap());
    let run = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!((run.state, run.finished_at), (RunState::Cancelled, Some(3)));
    assert_eq!(kinds(s, &r.id), ["cancelled"]);
    assert!(
        s.lease_job("w", 10, 4).unwrap().is_none(),
        "the queued job is failed as cancel_run does"
    );
}

fn finalising_as_error_writes_the_event_with_the_state(s: &dyn Store) {
    let r = new_run(s);
    let payload = r#"{"state":"error"}"#;
    let ev = NewEvent {
        kind: "finished",
        payload_json: payload,
    };
    assert!(s.finish_run_as_error(&r.id, "boom", &ev, 5).unwrap());
    let run = s.get_run(&r.id).unwrap().unwrap();
    assert_eq!(
        (run.state, run.error.as_deref(), run.finished_at),
        (RunState::Error, Some("boom"), Some(5))
    );
    let events = s.list_events(&r.id, 0, 10).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.kind.as_str(), e.payload_json.as_str(), e.ts))
            .collect::<Vec<_>>(),
        [("finished", payload, 5)]
    );
}

fn a_run_that_already_ended_gets_no_second_event(s: &dyn Store) {
    let r = new_run(s);
    assert!(s
        .cancel_run_with_event(&r.id, &event("cancelled"), 3)
        .unwrap());
    assert!(!s
        .cancel_run_with_event(&r.id, &event("cancelled"), 4)
        .unwrap());
    assert!(!s
        .finish_run_as_error(&r.id, "late", &event("finished"), 5)
        .unwrap());
    assert_eq!(kinds(s, &r.id), ["cancelled"]);
    assert_eq!(
        s.get_run(&r.id).unwrap().unwrap().state,
        RunState::Cancelled
    );
}

fn a_missing_run_is_an_error(s: &dyn Store) {
    assert!(s
        .finish_run_as_error("run_missing", "x", &event("finished"), 1)
        .is_err());
    assert!(!s
        .cancel_run_with_event("run_missing", &event("cancelled"), 1)
        .unwrap());
}

/// How often a reader saw a run ended without the terminal event. A writer ends runs with `end` while a reader polls
/// the way the event stream does: the run's state first, then its events.
fn gaps_seen(store: Arc<dyn Store>, kind: &str, end: &(dyn Fn(&dyn Store, &str) + Sync)) -> usize {
    let mut gaps = 0;
    for _ in 0..ROUNDS {
        let run = new_run(&*store);
        let done = AtomicBool::new(false);
        gaps += std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                let mut seen = 0;
                while !done.load(Ordering::Acquire) {
                    let ended = store
                        .get_run(&run.id)
                        .unwrap()
                        .is_some_and(|r| r.state.is_terminal());
                    if ended && !kinds(&*store, &run.id).iter().any(|k| k == kind) {
                        seen += 1;
                    }
                }
                seen
            });
            end(&*store, &run.id);
            done.store(true, Ordering::Release);
            reader.join().unwrap()
        });
        assert_eq!(kinds(&*store, &run.id), [kind]);
    }
    gaps
}

fn a_reader_never_sees_a_cancelled_run_without_its_event(store: Arc<dyn Store>) {
    let gaps = gaps_seen(store, "cancelled", &|s, id| {
        assert!(s.cancel_run_with_event(id, &event("cancelled"), 3).unwrap());
    });
    assert_eq!(gaps, 0, "a reader saw a cancelled run without `cancelled`");
}

fn a_reader_never_sees_an_errored_run_without_its_event(store: Arc<dyn Store>) {
    let gaps = gaps_seen(store, "finished", &|s, id| {
        assert!(s
            .finish_run_as_error(id, "boom", &event("finished"), 3)
            .unwrap());
    });
    assert_eq!(gaps, 0, "a reader saw an errored run without `finished`");
}

/// The reader above can tell: ending a run and then adding the event as a second call leaves a gap it sees.
fn the_reader_detects_a_state_written_before_its_event(store: Arc<dyn Store>) {
    let gaps = gaps_seen(store, "cancelled", &|s, id| {
        assert!(s.cancel_run(id, 3).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.add_event(id, "cancelled", "{}", 3).unwrap();
    });
    assert!(gaps > 0, "the reader missed a deliberate gap");
}

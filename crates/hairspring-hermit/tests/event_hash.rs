//! Guest events are part of the behaviour that `confirm()` and replay compare.
mod common;

use common::{engine, scratch, Opts, PASSTHROUGH};
use hairspring_core::bundle::Bundle;
use hairspring_engine::engine::RunRequest;

fn report(ops: u32) -> String {
    format!("HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"work\",\"proc\":\"p1\",\"ops\":{ops}}}\n")
}

/// Fails the same way (stdout and exit status) every time; only the events it reports come from `file`.
fn guest(file: &std::path::Path) -> String {
    format!(
        "cat {}; echo 'HAIRSPRING_VIOLATION: boom'; exit 1",
        file.display()
    )
}

#[test]
fn replay_notices_when_only_the_reported_events_differ() {
    let d = scratch("evhash");
    let file = d.join("events");
    std::fs::write(&file, report(300)).unwrap();
    let e = engine(&d, PASSTHROUGH, &guest(&file), Opts::default(), &[]);
    let f = e
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    assert!(f.message.contains("reproduced 3/3"), "{}", f.message);
    let bundle = Bundle::decode(&f.bundle).unwrap();

    let same = e.replay(&bundle).unwrap();
    assert!(same
        .steps
        .iter()
        .any(|s| s.label.contains("output matches the recording: yes")));

    std::fs::write(&file, report(301)).unwrap();
    let changed = e.replay(&bundle).unwrap();
    assert!(
        changed
            .steps
            .iter()
            .any(|s| s.label.contains("output matches the recording: no")),
        "{:?}",
        changed.steps
    );
    assert!(
        !changed.reproduced,
        "a different property outcome is not the recorded failure"
    );
}

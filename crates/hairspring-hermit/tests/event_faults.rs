//! Configuration, infrastructure and guest faults around guest event capture, and how runs and replays treat them.
mod common;

use common::events::{leftovers_in, rig, rig_with};
use hairspring_core::bundle::Bundle;
use hairspring_engine::engine::RunRequest;
use hairspring_hermit::EventsFault;
use std::os::unix::fs::PermissionsExt;

const FAILS: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";

extern "C" {
    fn geteuid() -> u32;
}

fn is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { geteuid() == 0 }
}

fn run_error(r: common::events::Rig) -> (String, std::path::PathBuf) {
    let (engine, dir) = r.engine();
    let err = engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .expect_err("the run must not start");
    (err.to_string(), dir)
}

#[test]
fn a_run_with_events_on_refuses_to_start_when_the_target_sets_its_own_tmp() {
    let r = rig("cfg-tmp", FAILS, true, vec!["--tmp=/somewhere".to_string()]);
    let (err, dir) = run_error(r);
    assert!(err.contains("--tmp"), "{err}");
    assert!(!dir.join("args-1").exists(), "no trial ran");
}

#[test]
fn binding_something_onto_tmp_counts_as_setting_it() {
    for args in [vec!["--bind=/h:/tmp"], vec!["--bind", "/h:/tmp/x"]] {
        let r = rig(
            "cfg-bind",
            FAILS,
            true,
            args.iter().map(|a| a.to_string()).collect(),
        );
        let (err, _) = run_error(r);
        assert!(err.contains("/tmp"), "{err}");
    }
}

#[test]
fn a_run_with_events_on_refuses_to_start_without_a_scratch_directory() {
    let r = rig_with("cfg-scratch", FAILS, true, vec![], |s| s.summary_dir = None);
    let (err, dir) = run_error(r);
    assert!(err.contains("scratch"), "{err}");
    assert!(!dir.join("args-1").exists(), "no trial ran");
}

#[test]
fn replay_refuses_a_bundle_recorded_with_the_other_events_transport() {
    for (record_on, replay_on, recorded) in [(false, true, "stream"), (true, false, "file")] {
        let (rec, _) = rig("transport-a", FAILS, record_on, vec![]).engine();
        let failure = rec
            .run(&RunRequest::new("svc", 1), &mut |_| true)
            .unwrap()
            .failure
            .unwrap();
        let bundle = Bundle::decode(&failure.bundle).unwrap();
        assert_eq!(
            bundle.params.get("events_transport").map(String::as_str),
            Some(recorded)
        );
        let (other, _) = rig("transport-b", FAILS, replay_on, vec![]).engine();
        let err = other.replay(&bundle).unwrap_err().to_string();
        assert!(err.contains("events transport differs"), "{err}");
        // The same transport replays fine.
        let (same, _) = rig("transport-c", FAILS, record_on, vec![]).engine();
        assert!(same.replay(&bundle).unwrap().reproduced);
    }
}

#[test]
fn a_bundle_from_before_the_setting_existed_counts_as_the_stream_transport() {
    let (rec, _) = rig("transport-old", FAILS, false, vec![]).engine();
    let failure = rec
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    let mut old = Bundle::decode(&failure.bundle).unwrap();
    old.params.remove("events_transport");
    let (stream, _) = rig("transport-old2", FAILS, false, vec![]).engine();
    assert!(stream.replay(&old).unwrap().reproduced);
    let (file, _) = rig("transport-old3", FAILS, true, vec![]).engine();
    assert!(file
        .replay(&old)
        .unwrap_err()
        .to_string()
        .contains("events transport differs"));
}

#[test]
fn events_can_use_their_own_disk_backed_scratch_base() {
    let mut disk = None;
    let r = rig_with(
        "disk",
        "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"",
        true,
        vec![],
        |s| {
            let base = std::env::temp_dir().join(format!("hairspring-disk-{}", std::process::id()));
            std::fs::create_dir_all(&base).unwrap();
            s.summary_dir = None;
            s.events_dir = Some(base.clone());
            disk = Some(base);
        },
    );
    let disk = disk.unwrap();
    let t = r.run(3);
    assert!(t.events_error.is_none(), "{:?}", t.events_error);
    assert_eq!(t.events.unwrap().health.accepted, 5);
    assert!(r.tmp_dir(3).starts_with(&disk));
    assert_eq!(leftovers_in(&disk), 0);
    drop(r);
    let _ = std::fs::remove_dir_all(&disk);
}

#[test]
fn a_failed_cleanup_is_reported_as_an_infrastructure_fault() {
    if is_root() {
        return;
    }
    let r = rig(
        "cleanup",
        "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"",
        true,
        vec![],
    );
    std::fs::write(r.dir.join("lockparent"), "").unwrap();
    let t = r.run(1);
    let parent = r.trial_dir(1).parent().unwrap().to_path_buf();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(&t.events_error, Some(EventsFault::Infra(m)) if m.contains("remove")),
        "{:?}",
        t.events_error
    );
    assert_eq!(
        t.events.unwrap().health.accepted,
        5,
        "the events were read before cleanup"
    );
}

#[test]
fn a_directory_the_guest_made_unreadable_is_still_cleaned_up() {
    if is_root() {
        return;
    }
    let r = rig(
        "chmod000",
        "mkdir \"$(dirname \"$HAIRSPRING_EVENTS_FILE\")/d\" && touch \"$(dirname \"$HAIRSPRING_EVENTS_FILE\")/d/f\" && chmod 000 \"$(dirname \"$HAIRSPRING_EVENTS_FILE\")/d\"",
        true,
        vec![],
    );
    let t = r.run(1);
    assert!(t.events_error.is_none(), "{:?}", t.events_error);
    assert_eq!(r.leftovers(), 0);
}

#[test]
fn trials_running_at_the_same_time_get_their_own_directories_and_leave_nothing() {
    let r = rig(
        "parallel",
        "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"; sleep 0.2",
        true,
        vec![],
    );
    let rr = &r;
    let trials: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (1..=6u64)
            .map(|seed| (seed, scope.spawn(move || rr.run(seed))))
            .collect();
        handles
            .into_iter()
            .map(|(seed, h)| (seed, h.join().unwrap()))
            .collect()
    });
    let mut dirs = std::collections::BTreeSet::new();
    for (seed, t) in &trials {
        assert!(t.events_error.is_none(), "{:?}", t.events_error);
        assert_eq!(t.events.as_ref().unwrap().health.accepted, 5);
        dirs.insert(r.tmp_dir(*seed));
    }
    assert_eq!(dirs.len(), 6);
    assert_eq!(r.leftovers(), 0);
}

#[test]
fn a_trial_killed_at_its_timeout_keeps_what_was_complete_and_counts_the_partial_lines() {
    let mut r = rig(
        "killed",
        "cat $REPORT >> \"$HAIRSPRING_EVENTS_FILE\"; printf 'HAIRSPRING_EVENT {\"v\":1,\"ki' >> \"$HAIRSPRING_EVENTS_FILE\"; \
         printf 'HAIRSPRING_EVENT {\"v\":1,\"kind\":\"wo'; sleep 30",
        true,
        vec![],
    );
    r.set_timeout(1);
    let t = r.run(1);
    assert!(t.timed_out);
    let props = t.events.expect("events");
    assert_eq!(props.health.accepted, 5);
    assert_eq!(
        props.health.malformed, 2,
        "one partial line from the file, one from stdout"
    );
    assert_eq!(r.leftovers(), 0);
}

#[test]
fn a_scratch_directory_that_cannot_be_made_is_an_infrastructure_fault_and_the_guest_still_runs() {
    let r = rig(
        "noscratch",
        "echo ${HAIRSPRING_EVENTS_FILE-unset}",
        true,
        vec![],
    );
    for private in std::fs::read_dir(r.dir.join("shm")).unwrap().flatten() {
        std::fs::remove_dir_all(private.path()).unwrap();
    }
    let t = r.run(1);
    assert_eq!(t.stdout, "unset\n");
    assert!(
        matches!(&t.events_error, Some(EventsFault::Infra(m)) if m.contains("events directory")),
        "{:?}",
        t.events_error
    );
}

#[test]
fn the_host_path_does_not_depend_on_the_seed() {
    let r = rig("width", "true", true, vec![]);
    let _ = (r.run(1), r.run(987_654_321));
    let (a, b) = (r.trial_dir(1), r.trial_dir(987_654_321));
    let (na, nb) = (
        a.file_name().unwrap().to_str().unwrap().to_string(),
        b.file_name().unwrap().to_str().unwrap().to_string(),
    );
    assert_eq!(na.len(), nb.len());
    assert!(!nb.contains("987654321"), "{nb}");
}

#[test]
fn a_reader_that_never_sees_the_end_of_the_stream_marks_the_capture_incomplete() {
    if std::process::Command::new("perl")
        .arg("-v")
        .output()
        .is_err()
    {
        return;
    }
    let r = rig(
        "incomplete",
        "perl -e 'use POSIX; POSIX::setsid(); exec \"sleep\", \"6\"' 2>/dev/null & sleep 0.5; echo done",
        false,
        vec![],
    );
    let t = r.run(1);
    assert!(
        matches!(&t.events_error, Some(EventsFault::Guest(m)) if m.contains("incomplete")),
        "{:?}",
        t.events_error
    );
}

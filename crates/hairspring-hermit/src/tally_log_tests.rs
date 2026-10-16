//! The per-trial records a run reports: one for every judged trial, with a digest the server can fold.
use crate::events::EventsFault;
use crate::exec::Trial;
use crate::summary::Summary;
use crate::tally::Tally;
use crate::trace::Verdict;
use crate::trial_log::{DETAIL_FIRST_TRIALS, DETAIL_SAMPLE_EVERY, OUTPUT_TAIL_BYTES};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::record::TrialRecord;
use hairspring_props::{RunCatalog, RunProps, TrialDigest, TrialEvents, TrialProps};
use std::time::Instant;

fn report(id: &str, kind: &str, trues: u64, falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{kind}","must_hit":true}}"#)),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":null,"first_true":null}}"#, trues + falses)),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

fn trial(props: TrialProps) -> Trial {
    Trial {
        exit_code: Some(0),
        events: Some(props),
        stdout: "out line\n".to_string(),
        stderr: "hermit: noise\nerr line\n".to_string(),
        elapsed_ms: 17,
        schedule: Some(Summary {
            fingerprint: "00ff".to_string(),
            syscalls: 99,
        }),
        ..Trial::default()
    }
}

fn pass(tally: &mut Tally, seq: u64, seed: u64, t: &Trial) {
    tally.on_trial(seq, seed, t, &Verdict::Pass);
    tally.pass();
}

fn rows(tally: &Tally) -> &[TrialReport] {
    tally.trial_log()
}

#[test]
fn every_judged_trial_gets_a_row_in_judging_order_with_its_verdict() {
    let mut tally = Tally::new(None);
    let ok = trial(report("a.id", "always", 3, 0));
    let bad = trial(report("a.id", "always", 2, 1));
    pass(&mut tally, 0, 10, &ok);
    tally.on_trial(
        1,
        11,
        &bad,
        &Verdict::Violation("always 'a.id' was false".into()),
    );
    let hung = Trial {
        timed_out: true,
        ..Trial::default()
    };
    tally.on_trial(2, 12, &hung, &Verdict::Hang("timed out after 5s".into()));
    tally.hang();
    let unusable = Trial {
        events_error: Some(EventsFault::Infra("disk".into())),
        ..Trial::default()
    };
    tally.on_trial(
        3,
        13,
        &unusable,
        &Verdict::Inconclusive("cannot collect".into()),
    );
    tally.unusable();
    let got: Vec<_> = rows(&tally)
        .iter()
        .map(|r| (r.seq, r.seed, r.verdict))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, 10, ReportedVerdict::Pass),
            (1, 11, ReportedVerdict::Violation),
            (2, 12, ReportedVerdict::Inconclusive),
            (3, 13, ReportedVerdict::Unusable),
        ]
    );
    assert_eq!(rows(&tally)[1].reason, "always 'a.id' was false");
    assert_eq!(rows(&tally)[0].reason, "");
}

#[test]
fn a_row_carries_the_trials_schedule_hash_and_timing() {
    let mut tally = Tally::new(None);
    let t = trial(report("a.id", "always", 1, 0));
    pass(&mut tally, 0, 5, &t);
    let r = &rows(&tally)[0];
    assert_eq!(r.exit_code, Some(0));
    assert_eq!(r.schedule_fp.as_deref(), Some("00ff"));
    assert_eq!((r.syscalls, r.elapsed_ms), (99, 17));
    assert!(r.behaviour_hash.is_some());
}

#[test]
fn a_seed_above_two_to_the_fifty_three_is_kept_whole() {
    let mut tally = Tally::new(None);
    pass(
        &mut tally,
        0,
        u64::MAX - 3,
        &trial(report("a.id", "always", 1, 0)),
    );
    assert_eq!(rows(&tally)[0].seed, u64::MAX - 3);
}

#[test]
fn the_digests_and_catalog_updates_fold_to_the_same_statistics_as_the_worker() {
    let mut tally = Tally::new(None);
    for (i, (id, kind, trues, falses)) in [
        ("a.id", "always", 3, 0),
        ("a.id", "always", 0, 2),
        ("b.id", "sometimes", 1, 1),
        ("a.id", "always", 2, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let t = trial(report(id, kind, trues, falses));
        let verdict = if falses > 0 && kind == "always" {
            Verdict::Violation("failed".into())
        } else {
            Verdict::Pass
        };
        tally.on_trial(i as u64, 100 + i as u64, &t, &verdict);
        tally.pass();
    }
    let (worker_stats, worker_summary) = tally.properties();
    let mut catalog = RunCatalog::new();
    let mut digests = Vec::new();
    for r in rows(&tally) {
        for u in &r.catalog {
            catalog = catalog.apply(u, 1000).unwrap();
        }
        digests.push((r.seed, TrialDigest::decode(&r.digest).unwrap()));
    }
    let mut run = RunProps::new();
    for (seed, d) in &digests {
        run = run.add_digest(*seed, d, &catalog);
    }
    assert_eq!(run.summary(), worker_summary);
    assert_eq!(run.finish(), worker_stats);
}

#[test]
fn a_trial_whose_events_could_not_be_collected_is_stored_but_left_out_of_the_statistics() {
    let mut tally = Tally::new(None);
    let faulty = Trial {
        events_error: Some(EventsFault::Infra("disk".into())),
        ..trial(report("a.id", "always", 0, 2))
    };
    tally.on_trial(
        0,
        1,
        &faulty,
        &Verdict::Inconclusive("cannot collect".into()),
    );
    tally.unusable();
    let r = &rows(&tally)[0];
    let d = TrialDigest::decode(&r.digest).unwrap();
    assert!(d.flags.excluded && d.entries.is_empty());
    assert!(r.catalog.is_empty());
    assert_eq!(tally.properties().1.trials, 0);
}

#[test]
fn detail_and_output_are_kept_for_findings_the_first_trials_and_a_sample() {
    let mut tally = Tally::new(None);
    let t = trial(report("a.id", "always", 3, 0));
    let late = DETAIL_FIRST_TRIALS + 8;
    pass(&mut tally, 0, 7, &t);
    pass(&mut tally, late, 7, &t);
    pass(&mut tally, late + 1, DETAIL_SAMPLE_EVERY * 3, &t);
    tally.on_trial(late + 2, 9, &t, &Verdict::Violation("v".into()));
    let kept: Vec<_> = rows(&tally)
        .iter()
        .map(|r| (r.detail.is_some(), r.output.is_some()))
        .collect();
    assert_eq!(
        kept,
        vec![(true, true), (false, false), (true, true), (true, true)]
    );
}

#[test]
fn kept_detail_is_a_valid_record_and_output_is_trimmed_and_cleaned() {
    let mut tally = Tally::new(None);
    let mut t = trial(report("a.id", "always", 3, 0));
    t.stdout = format!("{}tail-marker\n", "x".repeat(OUTPUT_TAIL_BYTES * 2));
    t.stderr = "hermit: internal noise\nreal error \u{1b}[31m\n".to_string();
    pass(&mut tally, 0, 7, &t);
    let r = &rows(&tally)[0];
    let record = TrialRecord::decode(r.detail.as_deref().unwrap().as_bytes()).unwrap();
    assert_eq!(record.props, t.events.clone().unwrap());
    let out: serde_json::Value = serde_json::from_str(r.output.as_deref().unwrap()).unwrap();
    let stdout = out["stdout"].as_str().unwrap();
    assert!(stdout.ends_with("tail-marker\n") && stdout.len() <= OUTPUT_TAIL_BYTES);
    let stderr = out["stderr"].as_str().unwrap();
    assert!(!stderr.contains("internal noise"), "{stderr}");
    assert!(stderr.contains("real error"));
    assert!(r.output.as_deref().unwrap().len() <= 128 * 1024);
}

#[test]
fn output_that_expands_when_escaped_still_fits_the_cap() {
    let mut tally = Tally::new(None);
    let mut t = trial(report("a.id", "always", 3, 0));
    t.stdout = "\u{1}".repeat(OUTPUT_TAIL_BYTES * 2);
    t.stderr = "\u{1}".repeat(OUTPUT_TAIL_BYTES * 2);
    pass(&mut tally, 0, 7, &t);
    let text = rows(&tally)[0].output.as_deref().unwrap();
    assert!(text.len() <= 128 * 1024, "{}", text.len());
    assert!(serde_json::from_str::<serde_json::Value>(text).is_ok());
}

#[test]
fn a_reason_is_one_clean_line_within_the_limit() {
    let mut tally = Tally::new(None);
    let t = trial(report("a.id", "always", 0, 1));
    let nasty = format!("line one\nline two \u{1b}[0m {}", "z".repeat(900));
    tally.on_trial(0, 1, &t, &Verdict::Violation(nasty));
    let reason = &rows(&tally)[0].reason;
    assert!(!reason.chars().any(char::is_control), "{reason:?}");
    assert!(reason.chars().count() <= 500);
    assert!(reason.starts_with("line one line two"));
}

#[test]
fn the_outcome_carries_the_log_of_a_finished_run() {
    let mut tally = Tally::new(None);
    pass(&mut tally, 0, 1, &trial(report("a.id", "always", 3, 0)));
    pass(&mut tally, 1, 2, &trial(report("a.id", "always", 3, 0)));
    let out = tally.finish(Instant::now());
    assert_eq!(out.trial_log.len(), 2);
    assert_eq!(out.trial_log[1].seq, 1);
}

#[test]
fn the_outcome_of_a_run_that_stopped_carries_the_log_too() {
    let mut tally = Tally::new(None);
    tally.on_trial(
        0,
        1,
        &trial(report("a.id", "always", 0, 1)),
        &Verdict::Violation("v".into()),
    );
    let out = tally.with_partial_properties(hairspring_engine::report::RunOutcome::passed(1, 1, 1));
    assert_eq!(out.trial_log.len(), 1);
}

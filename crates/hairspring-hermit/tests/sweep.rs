//! Sweep mode on the Hermit backend, driven through a fake `hermit` script: every trial of the budget runs and is
//! stored, a violation is confirmed but does not end the run, and nothing that is not a pass or a violation is rated.
mod common;
use common::events::rig_with;
use common::props::configure;
use common::{engine, scratch, Opts, PASSTHROUGH};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::mode::RunMode;
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_engine::sweep::{MAX_CONFIRMED_VIOLATIONS, MAX_CONSECUTIVE_INFRA};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};

/// Violates at seed 3 only.
const BUG_AT_3: &str = "[ \"$FAKE_SCHED_SEED\" = 3 ] && { echo 'HAIRSPRING_VIOLATION: balance went negative'; exit 1; }; echo fine";
/// Violates when seed % 5 == 3, so seeds 3 and 8 of a run from seed 1.
const SEEDED_BUG: &str = "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: balance went negative'; exit 1; }; echo fine";

fn opts(trials: u64) -> Opts {
    Opts {
        trials,
        ..Opts::default()
    }
}

fn run(e: &Engine, mode: RunMode) -> RunOutcome {
    e.run(&RunRequest::new("svc", 1).with_mode(mode), &mut |_| true)
        .unwrap()
}

fn verdicts(out: &RunOutcome) -> Vec<ReportedVerdict> {
    out.trial_log.iter().map(|r| r.verdict).collect()
}

fn row(out: &RunOutcome, seed: u64) -> &TrialReport {
    out.trial_log.iter().find(|r| r.seed == seed).unwrap()
}

/// A fake Hermit whose process panics with a PMU skid overshoot (an infrastructure failure) at the seeds in `bad`.
fn infra_at(bad: &[u64]) -> String {
    let test: Vec<String> = bad.iter().map(|s| format!("[ \"$seed\" = {s} ]")).collect();
    format!(
        "#!/bin/sh\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do\n  case \"$1\" in --sched-seed=*) seed=${{1#--sched-seed=}};; esac\n  shift\ndone\nshift\nif {}; then\n  echo \"thread 'main' panicked at timer.rs:1: SKID_OVERSHOOT precise timer overshot its target\" >&2\n  exit 101\nfi\nFAKE_SCHED_SEED=$seed exec \"$@\"\n",
        test.join(" || ")
    )
}

#[test]
fn search_stops_after_trial_3_of_10_and_a_sweep_runs_all_10_and_stores_10_rows_with_one_violation()
{
    let e = engine(&scratch("accept"), PASSTHROUGH, BUG_AT_3, opts(10), &[]);
    let search = run(&e, RunMode::Search);
    assert_eq!(search.status, RunStatus::Failed);
    assert_eq!(search.trials, 3, "search stops at the failing trial");
    assert!(search.stopped_at_first_failure && search.sweep.is_none());

    let sweep = run(&e, RunMode::Sweep);
    assert_eq!(sweep.status, RunStatus::Failed, "{sweep:?}");
    assert_eq!(sweep.trials, 10);
    assert!(!sweep.stopped_at_first_failure);
    assert_eq!(sweep.trial_log.len(), 10);
    let seeds: Vec<u64> = sweep.trial_log.iter().map(|r| r.seed).collect();
    assert_eq!(seeds, (1..=10).collect::<Vec<_>>());
    let seqs: Vec<u64> = sweep.trial_log.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (0..10).collect::<Vec<_>>());
    let violations = verdicts(&sweep)
        .iter()
        .filter(|v| **v == ReportedVerdict::Violation)
        .count();
    assert_eq!(violations, 1);
    assert_eq!(row(&sweep, 3).verdict, ReportedVerdict::Violation);
    let summary = sweep.sweep.as_ref().unwrap();
    assert_eq!(
        (summary.trials, summary.passes, summary.violations),
        (10, 9, 1)
    );
    assert_eq!(summary.first_failing_seeds, vec![3]);
    assert!(sweep.failure.unwrap().message.contains("sched-seed 3 "));
}

#[test]
fn every_row_carries_its_behaviour_hash_exit_status_and_elapsed_time() {
    let e = engine(&scratch("fields"), PASSTHROUGH, BUG_AT_3, opts(5), &[]);
    let out = run(&e, RunMode::Sweep);
    for r in &out.trial_log {
        assert!(r.behaviour_hash.is_some(), "{r:?}");
        assert!(r.exit_code.is_some());
    }
    assert_eq!(row(&out, 3).exit_code, Some(1));
    assert_eq!(row(&out, 2).exit_code, Some(0));
    assert!(row(&out, 3).reason.contains("balance went negative"));
    assert_eq!(row(&out, 2).reason, "");
}

#[test]
fn a_violation_is_confirmed_by_a_rerun_and_the_row_says_so() {
    let e = engine(&scratch("confirm"), PASSTHROUGH, SEEDED_BUG, opts(12), &[]);
    let out = run(&e, RunMode::Sweep);
    let summary = out.sweep.as_ref().unwrap();
    assert_eq!(summary.first_failing_seeds, vec![3, 8]);
    for seed in [3, 8] {
        let reason = &row(&out, seed).reason;
        assert!(reason.contains("balance went negative"), "{reason}");
        assert!(reason.contains("same-output reruns 3/3"), "{reason}");
    }
    assert!(!row(&out, 1).reason.contains("rerun"));
    assert_eq!(out.trials, 12, "reruns are not trials");
}

#[test]
fn a_violation_that_does_not_reproduce_is_stored_as_a_violation_with_that_noted() {
    let d = scratch("flaky");
    let state = d.join("state");
    let script = "if [ ! -f \"$STATE\" ]; then touch \"$STATE\"; echo 'HAIRSPRING_VIOLATION: once'; exit 1; fi; echo ok";
    let e = engine(
        &d,
        PASSTHROUGH,
        script,
        opts(4),
        &[("STATE", state.to_str().unwrap())],
    );
    let out = run(&e, RunMode::Sweep);
    assert_eq!(out.status, RunStatus::Failed);
    assert_eq!(row(&out, 1).verdict, ReportedVerdict::Violation);
    assert!(
        row(&out, 1).reason.contains("same-output reruns 0/3"),
        "{:?}",
        row(&out, 1)
    );
    assert_eq!(out.sweep.unwrap().passes, 3);
}

#[test]
fn only_the_first_violations_are_confirmed_and_the_rest_say_they_were_not() {
    let total = MAX_CONFIRMED_VIOLATIONS as u64 + 3;
    let e = engine(
        &scratch("limit"),
        PASSTHROUGH,
        "echo 'HAIRSPRING_VIOLATION: always'; exit 1",
        opts(total),
        &[],
    );
    let out = run(&e, RunMode::Sweep);
    assert_eq!(out.trials, total);
    assert_eq!(out.sweep.as_ref().unwrap().violations, total);
    let confirmed = out
        .trial_log
        .iter()
        .filter(|r| r.reason.contains("same-output reruns"))
        .count();
    let skipped = out
        .trial_log
        .iter()
        .filter(|r| r.reason.contains("not confirmed"))
        .count();
    assert_eq!(confirmed, MAX_CONFIRMED_VIOLATIONS);
    assert_eq!(skipped, 3);
    assert!(out
        .trial_log
        .iter()
        .all(|r| r.verdict == ReportedVerdict::Violation));
}

#[test]
fn timeouts_are_inconclusive_rows_and_out_of_the_rated_trials() {
    let script = "[ \"$FAKE_SCHED_SEED\" = 2 ] && sleep 30; echo fine";
    let o = Opts {
        trials: 4,
        timeout_secs: Some(1),
        ..Opts::default()
    };
    let out = run(
        &engine(&scratch("timeout"), PASSTHROUGH, script, o, &[]),
        RunMode::Sweep,
    );
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(out.inconclusive, 1);
    assert_eq!(row(&out, 2).verdict, ReportedVerdict::Inconclusive);
    let summary = out.sweep.unwrap();
    assert_eq!(
        (summary.passes, summary.inconclusive, summary.rated()),
        (3, 1, 3)
    );
}

#[test]
fn a_trial_with_a_host_failure_is_an_infra_row_and_the_sweep_goes_on() {
    let e = engine(
        &scratch("infra-once"),
        &infra_at(&[2]),
        "echo fine",
        opts(5),
        &[],
    );
    let out = run(&e, RunMode::Sweep);
    assert_eq!(row(&out, 2).verdict, ReportedVerdict::Infra, "{out:?}");
    assert_eq!(out.trials, 5);
    assert_eq!(out.status, RunStatus::Passed);
    let summary = out.sweep.unwrap();
    assert_eq!((summary.passes, summary.infra, summary.rated()), (4, 1, 4));
    assert_eq!(
        out.inconclusive, 0,
        "infra is neither a pass nor an inconclusive trial"
    );
}

#[test]
fn a_host_that_keeps_failing_ends_the_sweep_as_an_error_and_keeps_the_rows() {
    let bad: Vec<u64> = (1..=20).collect();
    let e = engine(
        &scratch("infra-all"),
        &infra_at(&bad),
        "echo fine",
        opts(20),
        &[],
    );
    let out = run(&e, RunMode::Sweep);
    assert_eq!(out.status, RunStatus::Error, "{out:?}");
    assert_eq!(out.trial_log.len() as u32, MAX_CONSECUTIVE_INFRA);
    assert!(out.error.unwrap().contains("consecutive"));
    assert_eq!(out.sweep.unwrap().infra, u64::from(MAX_CONSECUTIVE_INFRA));
}

#[test]
fn a_good_trial_resets_the_run_of_host_failures() {
    let bad = [1, 2, 4, 5, 7, 8];
    let e = engine(
        &scratch("infra-gaps"),
        &infra_at(&bad),
        "echo fine",
        opts(9),
        &[],
    );
    let out = run(&e, RunMode::Sweep);
    assert_eq!(out.trials, 9, "{out:?}");
    assert_eq!(out.sweep.unwrap().infra, 6);
}

#[test]
fn a_parallel_sweep_stores_every_row_in_seed_order_and_names_the_failing_seeds_lowest_first() {
    let o = Opts {
        trials: 12,
        parallelism: 4,
        ..Opts::default()
    };
    let out = run(
        &engine(&scratch("parallel"), PASSTHROUGH, SEEDED_BUG, o, &[]),
        RunMode::Sweep,
    );
    let seqs: Vec<u64> = out.trial_log.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (0..12).collect::<Vec<_>>());
    let summary = out.sweep.as_ref().unwrap();
    assert_eq!(summary.first_failing_seeds, vec![3, 8]);
    assert!(out.failure.unwrap().message.contains("sched-seed 3 "));
}

#[test]
fn a_sweep_can_be_cancelled_and_keeps_the_trials_it_judged() {
    let o = Opts {
        trials: 12,
        parallelism: 4,
        ..Opts::default()
    };
    let e = engine(&scratch("cancel"), PASSTHROUGH, "echo fine", o, &[]);
    let mut calls = 0;
    let out = e
        .run(
            &RunRequest::new("svc", 1).with_mode(RunMode::Sweep),
            &mut |_| {
                calls += 1;
                calls < 2
            },
        )
        .unwrap();
    assert_eq!(out.error.as_deref(), Some("cancelled"));
    assert_eq!(out.trial_log.len(), 8, "two batches of four were judged");
}

#[test]
fn a_sweep_stops_at_its_wall_clock_budget_and_reports_how_many_trials_it_ran() {
    let o = Opts {
        trials: 50,
        wall_secs: 1,
        ..Opts::default()
    };
    let out = run(
        &engine(
            &scratch("wall"),
            PASSTHROUGH,
            "sleep 0.4; echo fine",
            o,
            &[],
        ),
        RunMode::Sweep,
    );
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert!(out.trials >= 1 && out.trials < 50, "{}", out.trials);
    assert_eq!(out.trial_log.len() as u64, out.trials);
}

#[test]
fn a_sweep_where_every_trial_timed_out_is_an_error_that_keeps_its_rows() {
    let o = Opts {
        trials: 3,
        timeout_secs: Some(1),
        ..Opts::default()
    };
    let out = run(
        &engine(&scratch("alltimeout"), PASSTHROUGH, "sleep 30", o, &[]),
        RunMode::Sweep,
    );
    assert_eq!(out.status, RunStatus::Error, "{out:?}");
    assert!(out.error.unwrap().contains("nothing was tested"));
    assert_eq!(out.trial_log.len(), 3);
    assert_eq!(out.inconclusive, 3);
}

#[test]
fn confirmations_stop_when_the_wall_clock_budget_is_spent() {
    // Four violating trials run side by side in 0.7 s; each confirmation then takes three more runs of 0.7 s, so the
    // one-second budget is gone after the first one.
    let o = Opts {
        trials: 4,
        parallelism: 4,
        wall_secs: 1,
        ..Opts::default()
    };
    let script = "sleep 0.7; echo 'HAIRSPRING_VIOLATION: slow'; exit 1";
    let started = std::time::Instant::now();
    let out = run(
        &engine(&scratch("confirm-wall"), PASSTHROUGH, script, o, &[]),
        RunMode::Sweep,
    );
    assert_eq!(out.trial_log.len(), 4, "every judged trial keeps its row");
    assert!(
        row(&out, 1).reason.contains("same-output reruns"),
        "{:?}",
        row(&out, 1)
    );
    for seed in [2, 3, 4] {
        assert!(
            row(&out, seed)
                .reason
                .contains("not confirmed: out of budget"),
            "{:?}",
            row(&out, seed)
        );
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(out.status, RunStatus::Failed);
    assert!(out.failure.unwrap().message.contains("sched-seed 1 "));
}

#[test]
fn a_cancel_is_noticed_before_a_confirmation_not_only_between_batches() {
    let o = Opts {
        trials: 4,
        parallelism: 4,
        ..Opts::default()
    };
    let script = "echo 'HAIRSPRING_VIOLATION: always'; exit 1";
    let e = engine(&scratch("confirm-cancel"), PASSTHROUGH, script, o, &[]);
    let mut calls = 0;
    let out = e
        .run(
            &RunRequest::new("svc", 1).with_mode(RunMode::Sweep),
            &mut |_| {
                calls += 1;
                calls < 2
            },
        )
        .unwrap();
    assert_eq!(out.error.as_deref(), Some("cancelled"));
    assert_eq!(
        calls, 2,
        "asked before the first and before the second confirmation"
    );
    assert_eq!(out.trial_log.len(), 2, "the batch was not finished");
    assert!(row(&out, 2).reason.contains("not confirmed: cancelled"));
}

fn rig_sweep(
    name: &str,
    script: &str,
    trials: u64,
    parallelism: u32,
    tweak: impl FnOnce(&mut hairspring_hermit::HermitSettings),
) -> RunOutcome {
    let mut r = rig_with(name, script, false, vec![], tweak);
    configure(&mut r, trials, parallelism);
    let (engine, _dir) = r.engine();
    run(&engine, RunMode::Sweep)
}

#[test]
fn a_sweep_whose_digests_reach_their_bound_ends_there_and_keeps_every_row_it_has() {
    let out = rig_sweep("digest-bound", "cat $REPORT", 10, 1, |s| {
        s.sweep_digest_bytes = 1
    });
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(
        out.trials, 1,
        "one digest is already past a bound of one byte"
    );
    assert_eq!(out.trial_log.len(), 1);
    assert!(!out.trial_log[0].digest.is_empty());
}

#[test]
fn a_sweep_may_hold_one_batch_past_its_digest_bound_and_no_more() {
    let out = rig_sweep("digest-batch", "cat $REPORT", 40, 4, |s| {
        s.sweep_digest_bytes = 1
    });
    assert_eq!(out.trials, 4, "the batch in progress is judged whole");
}

#[test]
fn a_sweep_within_its_digest_bound_runs_every_trial() {
    let out = rig_sweep("digest-room", "cat $REPORT", 6, 1, |_| {});
    assert_eq!(out.trials, 6);
    assert_eq!(out.sweep.unwrap().passes, 6);
}

#[test]
fn kept_output_stops_at_its_bound_but_the_rows_do_not() {
    let script = "head -c 600 /dev/zero | tr '\\0' x; echo; exit 1";
    let out = rig_sweep("kept-bound", script, 6, 1, |s| s.sweep_kept_bytes = 1500);
    assert_eq!(out.trial_log.len(), 6);
    assert!(
        out.trial_log[0].output.is_some(),
        "the first fits the bound"
    );
    assert!(out.trial_log[5].output.is_none(), "the last does not");
    assert!(out
        .trial_log
        .iter()
        .all(|r| r.verdict == ReportedVerdict::Violation && r.exit_code == Some(1)));
}

#[test]
fn a_sweep_is_repeatable_apart_from_the_clock() {
    let e = engine(&scratch("repeat"), PASSTHROUGH, SEEDED_BUG, opts(8), &[]);
    let stable = |o: RunOutcome| {
        o.trial_log
            .iter()
            .map(|r| {
                (
                    r.seq,
                    r.seed,
                    r.verdict,
                    r.reason.clone(),
                    r.behaviour_hash.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        stable(run(&e, RunMode::Sweep)),
        stable(run(&e, RunMode::Sweep))
    );
}

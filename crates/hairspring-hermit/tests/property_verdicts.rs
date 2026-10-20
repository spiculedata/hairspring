//! Property verdicts: how guest-reported properties turn into trial and run outcomes, through a fake Hermit.
mod common;

use common::events::rig;
use common::props::*;
use hairspring_engine::engine::RunRequest;
use hairspring_engine::format::to_text;
use hairspring_engine::report::RunStatus;
use hairspring_hermit::{Trial, TrialObserver};
use hairspring_props::{PropertyStatus, TrialProps};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

#[test]
fn an_always_false_property_fails_the_trial_with_the_id_thread_and_ordinal() {
    let script = emit(&report("wal.ckpt", "always", 4, 1, true));
    let o = run_script("always-false", &script, 3);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    let f = o.failure.as_ref().unwrap();
    for part in ["wal.ckpt", "writer-0", "675", "reproduced 3/3"] {
        assert!(f.message.contains(part), "{part} in {}", f.message);
    }
    let b = bundle_of(&o);
    assert_eq!(
        b.params.get("property").map(String::as_str),
        Some("wal.ckpt")
    );
    assert_eq!(
        b.params.get("property_kind").map(String::as_str),
        Some("always")
    );
    assert_eq!(o.properties.len(), 1);
    assert_eq!(o.properties[0].status, PropertyStatus::Violation);
}

#[test]
fn a_sometimes_that_is_never_true_is_unmet_for_the_run_and_fails_no_trial() {
    let script = emit(&report("cache.cold", "sometimes", 0, 6, true));
    let o = run_script("unmet", &script, 3);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    assert!(o.failure.is_none());
    assert_eq!(o.properties[0].status, PropertyStatus::Unmet);
    let text = to_text("svc", &o);
    assert!(text.contains("UNMET"), "{text}");
    assert!(text.contains("cache.cold"), "{text}");
}

#[test]
fn a_reachable_that_was_hit_is_met() {
    let script = emit(&report("path.rare", "reachable", 1, 0, true));
    let o = run_script("reach", &script, 2);
    assert_eq!(o.status, RunStatus::Passed);
    assert_eq!(o.properties[0].status, PropertyStatus::Pass);
    assert!(!to_text("svc", &o).contains("UNMET"));
}

#[test]
fn degraded_and_perturbing_trials_do_not_count_toward_must_hit() {
    let clean_miss = report("s.id", "sometimes", 0, 3, true);
    let hit = report("s.id", "sometimes", 2, 0, true);
    let perturbing = format!(
        "{hit}{}",
        ev(r#"{"v":1,"kind":"eval","id":"s.id","type":"sometimes","ok":true}"#)
    );
    let degraded = format!("HAIRSPRING_EVENT {{not json\n{hit}");
    let script = by_seed(&[(2, perturbing), (3, degraded)], &clean_miss);
    let o = run_script("unmet-excl", &script, 3);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    assert_eq!(o.properties[0].status, PropertyStatus::Unmet, "{o:?}");
    assert_eq!(o.properties[0].trials_perturbing, 1);
    assert_eq!(o.properties[0].trials_degraded, 1);
    assert_eq!(o.inconclusive, 1, "the degraded trial is no pass");
}

#[test]
fn a_process_with_no_end_line_is_not_a_pass_and_not_rated() {
    let clean = report("a.id", "always", 3, 0, true);
    let cut = report("a.id", "always", 3, 0, false);
    let script = by_seed(&[(2, cut)], &clean);
    let o = run_script("missing-end", &script, 3);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    assert_eq!(o.inconclusive, 1);
    let stat = &o.properties[0];
    assert_eq!(
        (stat.trials, stat.trials_rated, stat.trials_unreported),
        (3, 2, 1)
    );
}

#[test]
fn a_failure_seen_in_a_report_without_end_still_counts() {
    let cut = report("a.id", "always", 3, 1, false);
    let o = run_script("missing-end-false", &emit(&cut), 2);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    assert!(o.failure.unwrap().message.contains("a.id"));
}

#[test]
fn more_than_a_megabyte_of_chatter_before_the_report_still_yields_the_verdict() {
    let script = format!(
        "head -c 2000000 /dev/zero | tr '\\0' x; echo; {}",
        emit(&report("big.id", "always", 1, 1, true))
    );
    let o = run_script("chatter", &script, 1);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    assert!(o.failure.unwrap().message.contains("big.id"));
}

#[test]
fn a_truncated_events_file_degrades_the_trial_but_a_false_that_was_read_still_fails_it() {
    let pad = "head -c 5000000 /dev/zero | tr '\\0' x >> \"$HAIRSPRING_EVENTS_FILE\"";
    let falsy = format!(
        "cat <<'EOF' > \"$HAIRSPRING_EVENTS_FILE\"\n{}EOF\n{pad}",
        report("f.id", "always", 1, 1, true)
    );
    let mut r = rig("trunc-false", &falsy, true, vec![]);
    configure(&mut r, 2, 1);
    let o = run(r);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    assert!(o.failure.unwrap().message.contains("f.id"));
}

#[test]
fn a_truncated_events_file_without_a_false_yields_no_pass_and_no_must_hit_claim() {
    let pad = "head -c 5000000 /dev/zero | tr '\\0' x >> \"$HAIRSPRING_EVENTS_FILE\"";
    let hit = report("s.id", "sometimes", 1, 0, true);
    let degraded = format!("cat <<'EOF' > \"$HAIRSPRING_EVENTS_FILE\"\n{hit}EOF\n{pad}");
    let miss = format!(
        "cat <<'EOF' > \"$HAIRSPRING_EVENTS_FILE\"\n{}EOF\n",
        report("s.id", "sometimes", 0, 2, true)
    );
    let script = format!("if [ \"$HAIRSPRING_SEED\" = 1 ]; then {degraded}\nelse {miss}\nfi");
    let mut r = rig("trunc-ok", &script, true, vec![]);
    configure(&mut r, 3, 1);
    let o = run(r);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    assert_eq!(o.inconclusive, 1, "the degraded trial is not a pass");
    assert_eq!(
        o.properties[0].status,
        PropertyStatus::Unmet,
        "the hit was only seen in a degraded trial"
    );
}

#[test]
fn an_events_file_the_guest_replaced_with_a_symlink_makes_the_trial_inconclusive_not_a_pass() {
    let mut r = rig("planted", "exit 0", true, vec![]);
    std::fs::write(r.dir.join("plant"), "").unwrap();
    configure(&mut r, 3, 1);
    let o = run(r);
    assert_eq!(o.status, RunStatus::Error, "{o:?}");
    assert_eq!(o.inconclusive, 3);
}

#[test]
fn an_infrastructure_fault_is_inconclusive_even_when_the_trial_read_a_false() {
    if unsafe_root() {
        return;
    }
    let script = "cat <<'EOF' > \"$HAIRSPRING_EVENTS_FILE\"\n".to_string()
        + &report("i.id", "always", 1, 1, true)
        + "EOF\n";
    let r = rig("infra", &script, true, vec![]);
    std::fs::write(r.dir.join("lockparent"), "").unwrap();
    let dir = r.dir.clone();
    let o = run(r);
    // Put the permissions back so the directory can be cleaned up.
    for entry in std::fs::read_dir(dir.join("shm")).unwrap().flatten() {
        let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
    }
    assert_ne!(o.status, RunStatus::Passed, "{o:?}");
    assert_ne!(o.status, RunStatus::Failed, "{o:?}");
    assert_eq!(o.inconclusive, 1, "{o:?}");
}

extern "C" {
    fn geteuid() -> u32;
}

fn unsafe_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { geteuid() == 0 }
}

#[test]
fn legacy_violation_lines_and_exit_codes_are_judged_as_before() {
    let clean = emit(&report("ok.id", "always", 2, 0, true));
    let o = run_script(
        "legacy-line",
        &format!("{clean}echo 'HAIRSPRING_VIOLATION: boom'"),
        1,
    );
    assert_eq!(o.status, RunStatus::Failed);
    assert!(o.failure.as_ref().unwrap().message.contains("(boom)"));
    assert!(!bundle_of(&o).params.contains_key("property"));
    let o = run_script("legacy-exit", &format!("{clean}exit 3"), 1);
    assert!(o.failure.unwrap().message.contains("exit code 3"));
}

#[test]
fn a_property_failure_outranks_the_exit_status() {
    let script = format!("{}exit 3", emit(&report("p.id", "always", 0, 2, true)));
    let o = run_script("outranks", &script, 1);
    let msg = o.failure.unwrap().message;
    assert!(msg.contains("p.id"), "{msg}");
}

#[test]
fn the_first_failure_is_the_lowest_failing_seed_and_the_run_stops_there() {
    let bad = report("w.id", "always", 1, 1, true);
    let good = report("w.id", "always", 2, 0, true);
    let script = by_seed(&[(3, bad.clone()), (5, bad)], &good);
    let mut r = rig("lowest", &script, false, vec![]);
    configure(&mut r, 6, 3);
    let o = run(r);
    assert_eq!(o.status, RunStatus::Failed);
    assert_eq!(o.trials, 3);
    let first = o.properties[0].first_failure.as_ref().unwrap();
    assert_eq!(first.seed, 3);
}

#[derive(Default)]
struct Seen(Mutex<Vec<(u64, Option<i32>, usize)>>);

impl TrialObserver for Seen {
    fn on_trial(&self, seed: u64, trial: &Trial, props: &TrialProps) {
        self.0
            .lock()
            .unwrap()
            .push((seed, trial.exit_code, props.properties.len()));
    }
}

#[test]
fn the_on_trial_hook_sees_every_trial_in_seed_order() {
    let good = emit(&report("h.id", "always", 2, 0, true));
    let seen = Arc::new(Seen::default());
    let mut r = rig("hook", &good, false, vec![]);
    configure(&mut r, 5, 2);
    r.backend = r.backend.with_observer(seen.clone());
    let o = run(r);
    assert_eq!(o.status, RunStatus::Passed);
    let got = seen.0.lock().unwrap().clone();
    let seeds: Vec<u64> = got.iter().map(|(s, _, _)| *s).collect();
    assert_eq!(seeds, vec![1, 2, 3, 4, 5]);
    assert!(got.iter().all(|(_, code, n)| *code == Some(0) && *n == 1));
}

#[test]
fn a_run_without_events_has_no_property_section() {
    let o = run_script("plain", "exit 0", 2);
    assert_eq!(o.status, RunStatus::Passed);
    assert!(o.properties.is_empty());
    assert_eq!(
        to_text("svc", &o),
        format!(
            "PASSED  svc  ({} steps, 2 trials, {} ms)\n  hits: 0/2 rated trials, exact one-sided 95% upper bound 77.64% (binomial, 0 of 2)\n",
            o.steps_executed, o.elapsed_ms
        )
    );
}

#[test]
fn a_rerun_that_fails_a_different_property_is_not_a_reproduction() {
    // The first run fails `a.id`; every later run (the confirmations) fails `b.id` instead.
    let a = report("a.id", "always", 0, 1, true);
    let b = report("b.id", "always", 0, 1, true);
    let script = format!(
        "n=$(cat \"$EV_LOG/n\" 2>/dev/null || echo 0); echo $((n+1)) > \"$EV_LOG/n\"\n\
         if [ \"$n\" = 0 ]; then {}\nelse {}\nfi",
        emit(&a),
        emit(&b)
    );
    let o = run_script("confirm-id", &script, 1);
    let msg = o.failure.unwrap().message;
    assert!(msg.contains("reproduced 0/3"), "{msg}");
    assert!(msg.contains("not reproducible"), "{msg}");
}

#[test]
fn replay_confirms_the_recorded_property_and_old_bundles_still_replay() {
    let script = emit(&report("r.id", "always", 0, 1, true));
    let mut r = rig("replay", &script, false, vec![]);
    configure(&mut r, 1, 1);
    let (engine, _) = r.engine();
    let o = engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    let bundle = bundle_of(&o);
    assert!(engine.replay(&bundle).unwrap().reproduced);

    let mut other = bundle.clone();
    other
        .params
        .insert("property".into(), "someone.else".into());
    assert!(!engine.replay(&other).unwrap().reproduced);

    let mut old = bundle;
    old.params.remove("property");
    old.params.remove("property_kind");
    assert!(engine.replay(&old).unwrap().reproduced);
}

#[test]
fn a_clean_events_run_reports_a_rate_for_each_property() {
    let script = emit(&report("rate.id", "always", 3, 0, true));
    let o = run_script("rate", &script, 4);
    let stat = &o.properties[0];
    assert_eq!(stat.trials_rated, 4);
    let rate = stat.rate.expect("a rate");
    assert_eq!((rate.k, rate.n), (0, 4));
}

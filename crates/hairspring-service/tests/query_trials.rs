//! A run's trials: filters, paging, one trial's detail and output, and the coverage series.
mod common;

use common::fixture::*;
use common::*;
use hairspring_props::record::TrialRecord as KeptRecord;
use hairspring_props::AssertKind;
use hairspring_service::{PropertyQuery, ServiceError, TrialQuery};

fn seeds(f: &Fixture, q: &TrialQuery) -> Vec<String> {
    f.svc
        .trial_page(&f.run_id, q)
        .unwrap()
        .unwrap()
        .trials
        .into_iter()
        .map(|t| t.seed)
        .collect()
}

fn idx_of(f: &Fixture, id: &str) -> String {
    f.svc
        .property_page(
            &f.run_id,
            &PropertyQuery {
                q: Some(id.into()),
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .unwrap()
        .properties[0]
        .idx
        .to_string()
}

fn q() -> TrialQuery {
    TrialQuery::default()
}

#[test]
fn trials_are_listed_in_judging_order_with_seeds_as_strings() {
    let f = finished(&local());
    assert_eq!(seeds(&f, &q()), ["1", "2", "3", "4", "5", "6"]);
    let page = f.svc.trial_page(&f.run_id, &q()).unwrap().unwrap();
    assert_eq!(page.trials[2].verdict, "violation");
    assert!(page.trials[4].degraded && !page.trials[4].rated);
    assert!(page.trials[0].rated && page.trials[0].claimable);
}

#[test]
fn a_seed_above_two_to_the_fifty_three_is_an_exact_decimal_string() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 1);
    let report = Recorder::new().pass(BIG_SEED, &props("a.id", AssertKind::Always, 3, 0));
    svc.ingest_trials(&job.job_id, &local(), &[report]).unwrap();
    finish(&svc, &job, &local(), passed(1, Some(1)));
    let page = svc.trial_page(&job.run_id, &q()).unwrap().unwrap();
    assert_eq!(page.trials[0].seed, BIG_SEED.to_string());
    let json = serde_json::to_value(&page).unwrap();
    assert!(json["trials"][0]["seed"].is_string());
}

#[test]
fn filters_select_by_verdict_flags_fingerprint_and_property() {
    let f = finished(&local());
    let only = |mutate: &dyn Fn(&mut TrialQuery)| {
        let mut query = q();
        mutate(&mut query);
        seeds(&f, &query)
    };
    assert_eq!(only(&|t| t.verdict = Some("violation".into())), ["3"]);
    assert_eq!(only(&|t| t.degraded = Some("true".into())), ["5"]);
    assert_eq!(only(&|t| t.degraded = Some("false".into())).len(), 5);
    assert_eq!(
        only(&|t| t.perturbing = Some("true".into())),
        Vec::<String>::new()
    );
    // The recorder gives a trial the fingerprint of its seed modulo three: seeds 3 and 6 share one.
    assert_eq!(
        only(&|t| t.fp = Some("0000000000000000".into())),
        ["3", "6"]
    );

    let broken = idx_of(&f, "b.broken");
    assert_eq!(only(&|t| t.fails = Some(broken.clone())), ["3"]);
    let ok = idx_of(&f, "r.ok");
    assert_eq!(only(&|t| t.hits = Some(ok.clone())).len(), 6);
    assert!(only(&|t| t.misses = Some(ok.clone())).is_empty());
    assert_eq!(
        only(&|t| {
            t.fails = Some(broken.clone());
            t.degraded = Some("false".into());
        }),
        ["3"]
    );
}

#[test]
fn paging_walks_every_trial_once() {
    let f = finished(&local());
    let (mut seen, mut cursor) = (Vec::new(), None);
    loop {
        let page = f
            .svc
            .trial_page(
                &f.run_id,
                &TrialQuery {
                    limit: Some(4),
                    cursor: cursor.clone(),
                    ..q()
                },
            )
            .unwrap()
            .unwrap();
        assert!(page.trials.len() <= 4);
        seen.extend(page.trials.iter().map(|t| t.seed.clone()));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen, ["1", "2", "3", "4", "5", "6"]);
}

#[test]
fn bad_parameters_are_refused_and_an_unknown_run_is_none() {
    let f = finished(&local());
    let bad = |t: TrialQuery| {
        matches!(
            f.svc.trial_page(&f.run_id, &t),
            Err(ServiceError::InvalidRequest(_))
        )
    };
    assert!(bad(TrialQuery {
        verdict: Some("nope".into()),
        ..q()
    }));
    assert!(bad(TrialQuery {
        fails: Some("-1".into()),
        ..q()
    }));
    assert!(bad(TrialQuery {
        hits: Some("x".into()),
        ..q()
    }));
    assert!(bad(TrialQuery {
        degraded: Some("yes".into()),
        ..q()
    }));
    assert!(bad(TrialQuery {
        fp: Some("a b".into()),
        ..q()
    }));
    assert!(bad(TrialQuery {
        cursor: Some("abc".into()),
        ..q()
    }));
    assert!(f
        .svc
        .trial_page("run_0000000000000000", &q())
        .unwrap()
        .is_none());
}

/// A one-trial run whose trial kept a detail record and its output.
fn kept_run(output: &str) -> (hairspring_service::Service, String) {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 1);
    let p = props_multi(&[("b.broken", AssertKind::Always, HOSTILE, 0, 2)]);
    let mut report = Recorder::new().trial(
        9,
        &p,
        hairspring_engine::trial_log::ReportedVerdict::Violation,
        false,
    );
    report.reason = format!("{HOSTILE} failed");
    report.detail = Some(String::from_utf8(KeptRecord::encode(&p).unwrap()).unwrap());
    report.output = Some(serde_json::json!({ "stdout": output, "stderr": "oops" }).to_string());
    svc.ingest_trials(&job.job_id, &local(), &[report]).unwrap();
    finish(&svc, &job, &local(), passed(1, Some(1)));
    (svc, job.run_id)
}

#[test]
fn a_trials_detail_carries_its_kept_record_and_keeps_guest_text_under_the_guest_fence() {
    let (svc, run) = kept_run("out");
    let d = svc.trial_detail(&run, 0).unwrap().unwrap();
    assert_eq!(d.trial.kept, "kept");
    assert_eq!(d.output_streams, ["stdout", "stderr"]);
    assert!(d.guest.detail.is_some());
    let json = serde_json::to_value(&d).unwrap();
    let found = paths_containing(&json, HOSTILE);
    assert!(
        found.len() >= 2,
        "the reason and the kept detail both carry it: {found:?}"
    );
    for path in found {
        assert!(
            path.contains(".guest"),
            "guest text outside the guest fence: {path}"
        );
    }
    assert!(json["guest_note"].as_str().unwrap().contains("untrusted"));
}

#[test]
fn a_trial_without_kept_detail_says_so() {
    let f = finished(&local());
    let d = f.svc.trial_detail(&f.run_id, 0).unwrap().unwrap();
    assert_eq!(d.trial.kept, "none");
    assert!(d.guest.detail.is_none() && d.output_streams.is_empty());
    assert!(f.svc.trial_detail(&f.run_id, 999).unwrap().is_none());
    assert!(f
        .svc
        .trial_detail("run_0000000000000000", 0)
        .unwrap()
        .is_none());
}

#[test]
fn output_comes_back_exactly_as_kept_and_only_for_the_runs_own_trial() {
    let hostile = format!("{HOSTILE}\u{1b}[31m red \u{202e}");
    let (svc, run) = kept_run(&hostile);
    assert_eq!(
        svc.trial_output(&run, 0, "stdout").unwrap().as_deref(),
        Some(hostile.as_str())
    );
    assert_eq!(
        svc.trial_output(&run, 0, "stderr").unwrap().as_deref(),
        Some("oops")
    );
    assert!(matches!(
        svc.trial_output(&run, 0, "../x"),
        Err(ServiceError::InvalidRequest(_))
    ));
    assert!(svc.trial_output(&run, 5, "stdout").unwrap().is_none());

    // The same output text is one shared blob, but another run's trial that kept none cannot reach it.
    let other = finished(&local());
    assert!(other
        .svc
        .trial_output(&other.run_id, 0, "stdout")
        .unwrap()
        .is_none());
    assert!(other.svc.trial_output(&run, 0, "stdout").unwrap().is_none());
}

#[test]
fn coverage_is_a_series_that_never_goes_down_and_ends_at_the_last_trial() {
    let f = finished(&local());
    let c = f.svc.coverage(&f.run_id).unwrap().unwrap();
    assert_eq!(c.trials, 6);
    assert!(!c.downsampled);
    assert_eq!(c.points.first().unwrap().seq, 0);
    assert_eq!(c.points.last().unwrap().seq, 5);
    for w in c.points.windows(2) {
        assert!(w[0].ids_hit <= w[1].ids_hit && w[0].schedules <= w[1].schedules);
    }
    let last = c.points.last().unwrap();
    assert_eq!((last.ids_hit, last.ids_true, last.schedules), (4, 3, 3));
    assert!(f.svc.coverage("run_0000000000000000").unwrap().is_none());
}

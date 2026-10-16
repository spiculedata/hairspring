//! The property table of a run: filters, paging, honest rate cells, UNMET wording, claims and the `guest` fence.
mod common;

use common::fixture::*;
use common::*;
use hairspring_props::claim_text::{BuildRecord, KnowledgeFlag, Label, SeedSelection};
use hairspring_props::honesty::RateCell;
use hairspring_props::AssertKind;
use hairspring_service::{PropertyPage, PropertyQuery, ServiceError};

fn page(f: &Fixture, q: &PropertyQuery) -> PropertyPage {
    f.svc.property_page(&f.run_id, q, None).unwrap().unwrap()
}

fn ids(p: &PropertyPage) -> Vec<&str> {
    p.properties.iter().map(|r| r.guest.id.as_str()).collect()
}

fn declared() -> Label {
    Label {
        build: BuildRecord {
            instrumented: false,
            behaviour_changes: false,
        },
        knowledge: Some(vec![]),
        seed_selection: SeedSelection::Blind,
    }
}

#[test]
fn rows_sort_violation_then_unmet_then_unrated_then_pass_and_carry_honest_rate_cells() {
    let f = finished(&local());
    let p = page(&f, &PropertyQuery::default());
    assert_eq!(ids(&p), ["b.broken", "s.gap", "a.fine", "r.ok"]);
    let statuses: Vec<_> = p.properties.iter().map(|r| r.status).collect();
    assert_eq!(statuses, ["violation", "unmet", "pass", "pass"]);

    let broken = &p.properties[0];
    assert!(
        matches!(broken.rate, RateCell::Rate { k: 1, n: 5, .. }),
        "{:?}",
        broken.rate
    );
    assert_eq!(broken.first_failing_seed.as_deref(), Some("3"));
    let fine = &p.properties[2];
    assert!(
        matches!(fine.rate, RateCell::ZeroEvents { n: 5, .. }),
        "{:?}",
        fine.rate
    );
    assert!(fine.rate_text.contains("exact one-sided 95% upper bound"));
    assert!(!fine.rate_text.contains("at most"), "{}", fine.rate_text);
}

#[test]
fn the_degraded_trial_is_listed_beside_n_and_never_counted_in_it() {
    let f = finished(&local());
    let p = page(&f, &PropertyQuery::default());
    assert_eq!(p.header.trials_judged, 6);
    assert_eq!(p.header.trials_rated, Some(5));
    assert_eq!(p.header.excluded.degraded, 1);
    for row in &p.properties {
        assert_eq!(row.trials, 6);
        assert_eq!(row.trials_rated, 5, "{}", row.guest.id);
    }
}

#[test]
fn the_headline_is_generated_from_counts() {
    let f = finished(&local());
    assert_eq!(
        page(&f, &PropertyQuery::default()).headline,
        "1 property failed; first failing seed 3"
    );
}

#[test]
fn unmet_is_a_coverage_gap_stated_for_the_trials_that_were_run() {
    let f = finished(&local());
    let p = page(&f, &PropertyQuery::default());
    let gap = p.properties.iter().find(|r| r.guest.id == "s.gap").unwrap();
    let text = gap.unmet.as_deref().unwrap();
    assert!(text.contains("coverage gap, not a finding"), "{text}");
    assert!(text.contains("in 5 claimable trials"), "{text}");
    assert!(p
        .properties
        .iter()
        .filter(|r| r.guest.id != "s.gap")
        .all(|r| r.unmet.is_none()));
}

#[test]
fn a_run_that_stopped_at_its_first_failure_shows_no_rate_and_no_unmet() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials(&job.job_id, &local(), &reports[..3])
        .unwrap();
    finish(&svc, &job, &local(), failed_outcome(3));

    let p = svc
        .property_page(&job.run_id, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert!(p.header.stopped_at_first_failure);
    for row in &p.properties {
        assert!(
            matches!(row.rate, RateCell::NotARate { .. }),
            "{}: {:?}",
            row.guest.id,
            row.rate
        );
        assert!(row
            .rate_text
            .starts_with("not a rate (stopped at first failure)"));
        assert!(
            row.unmet.is_none(),
            "no UNMET claim from a stopped run: {}",
            row.guest.id
        );
    }
    assert!(p
        .notes
        .iter()
        .any(|n| n.contains("Stopped at first failure: no rates, no UNMET claim")));
}

#[test]
fn a_run_ended_early_withholds_unmet_and_says_why() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(6, Some(6)));
    let p = svc
        .property_page(&job.run_id, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert!(p.header.ingest_incomplete);
    assert!(p.properties.iter().all(|r| r.unmet.is_none()));
    assert!(
        p.notes.iter().any(|n| n.contains("ended early")),
        "{:?}",
        p.notes
    );
}

#[test]
fn filters_narrow_the_table() {
    let f = finished(&local());
    let by = |status: Option<&str>, kind: Option<&str>, q: Option<&str>| {
        ids(&page(
            &f,
            &PropertyQuery {
                status: status.map(str::to_owned),
                kind: kind.map(str::to_owned),
                q: q.map(str::to_owned),
                ..PropertyQuery::default()
            },
        ))
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    };
    assert_eq!(by(Some("unmet"), None, None), ["s.gap"]);
    assert_eq!(by(None, Some("sometimes"), None), ["s.gap", "r.ok"]);
    assert_eq!(by(None, None, Some("a.")), ["a.fine"]);
    assert_eq!(by(Some("pass"), Some("always"), Some("a")), ["a.fine"]);
    assert!(by(None, None, Some("zzz")).is_empty());
}

#[test]
fn paging_walks_every_row_once_and_a_cursor_is_only_a_position() {
    let f = finished(&local());
    let (mut seen, mut cursor) = (Vec::new(), None);
    loop {
        let p = page(
            &f,
            &PropertyQuery {
                limit: Some(1),
                cursor: cursor.clone(),
                ..PropertyQuery::default()
            },
        );
        assert_eq!(p.total, 4);
        seen.extend(p.properties.iter().map(|r| r.guest.id.clone()));
        match p.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen, ["b.broken", "s.gap", "a.fine", "r.ok"]);
}

#[test]
fn limits_are_capped_and_bad_parameters_are_refused() {
    let f = finished(&local());
    let bad = |q: PropertyQuery| {
        matches!(
            f.svc.property_page(&f.run_id, &q, None),
            Err(ServiceError::InvalidRequest(_))
        )
    };
    assert!(bad(PropertyQuery {
        status: Some("nope".into()),
        ..Default::default()
    }));
    assert!(bad(PropertyQuery {
        kind: Some("nope".into()),
        ..Default::default()
    }));
    assert!(bad(PropertyQuery {
        sort: Some("nope".into()),
        ..Default::default()
    }));
    assert!(bad(PropertyQuery {
        cursor: Some("x".into()),
        ..Default::default()
    }));
    assert!(bad(PropertyQuery {
        cursor: Some("-1".into()),
        ..Default::default()
    }));
    assert!(bad(PropertyQuery {
        q: Some("a".repeat(129)),
        ..Default::default()
    }));
    let huge = page(
        &f,
        &PropertyQuery {
            limit: Some(1_000_000),
            ..Default::default()
        },
    );
    assert_eq!(huge.properties.len(), 4);
    let zero = page(
        &f,
        &PropertyQuery {
            limit: Some(0),
            ..Default::default()
        },
    );
    assert_eq!(zero.properties.len(), 1, "a limit of 0 is raised to 1");
    assert!(f
        .svc
        .property_page("run_0000000000000000", &PropertyQuery::default(), None)
        .unwrap()
        .is_none());
}

#[test]
fn a_property_detail_lists_failing_hit_and_never_hit_seeds_as_strings_in_pages() {
    let f = finished(&local());
    let q = |kind: &str, limit: u32, cursor: Option<String>| hairspring_service::SeedQuery {
        seeds: Some(kind.into()),
        limit: Some(limit),
        cursor,
    };
    let idx_of = |id: &str| {
        let p = page(
            &f,
            &PropertyQuery {
                q: Some(id.into()),
                ..Default::default()
            },
        );
        p.properties[0].idx
    };
    let failing = f
        .svc
        .property_detail(&f.run_id, idx_of("b.broken"), &q("failing", 10, None))
        .unwrap()
        .unwrap();
    assert_eq!(
        failing
            .seeds
            .iter()
            .map(|s| s.seed.as_str())
            .collect::<Vec<_>>(),
        ["3"]
    );

    let r = idx_of("r.ok");
    let hit = f
        .svc
        .property_detail(&f.run_id, r, &q("hit", 10, None))
        .unwrap()
        .unwrap();
    assert_eq!(
        hit.seeds
            .iter()
            .map(|s| s.seed.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5", "6"]
    );
    let first = f
        .svc
        .property_detail(&f.run_id, r, &q("hit", 2, None))
        .unwrap()
        .unwrap();
    assert_eq!(first.seeds.len(), 2);
    let second = f
        .svc
        .property_detail(&f.run_id, r, &q("hit", 2, first.next_cursor.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(
        second
            .seeds
            .iter()
            .map(|s| s.seed.as_str())
            .collect::<Vec<_>>(),
        ["3", "4"]
    );

    let never = f
        .svc
        .property_detail(&f.run_id, idx_of("s.gap"), &q("never", 10, None))
        .unwrap()
        .unwrap();
    assert!(
        never.seeds.is_empty(),
        "s.gap is evaluated in every usable trial"
    );
    assert!(f
        .svc
        .property_detail(&f.run_id, 999, &q("failing", 10, None))
        .unwrap()
        .is_none());
}

#[test]
fn text_from_the_program_appears_only_under_a_guest_object() {
    let f = finished(&local());
    let p = page(&f, &PropertyQuery::default());
    let json = serde_json::to_value(&p).unwrap();
    let found = paths_containing(&json, HOSTILE);
    assert!(
        !found.is_empty(),
        "the fixture's message must reach the page"
    );
    for path in &found {
        assert!(
            path.contains(".guest"),
            "guest text outside the guest fence: {path}"
        );
    }
    for needle in ["b.broken", "s.gap", "a.fine", "r.ok"] {
        for path in paths_containing(&json, needle) {
            assert!(
                path.contains(".guest"),
                "property id outside the guest fence: {path}"
            );
        }
    }
    assert!(json["guest_note"].as_str().unwrap().contains("untrusted"));
    let detail = f
        .svc
        .property_detail(&f.run_id, p.properties[0].idx, &Default::default())
        .unwrap()
        .unwrap();
    for path in paths_containing(&serde_json::to_value(&detail).unwrap(), HOSTILE) {
        assert!(path.contains(".guest"), "{path}");
    }
}

#[test]
fn the_run_detail_carries_no_worker_id_for_a_read_caller() {
    let f = finished(&local());
    let json = serde_json::to_string(&page(&f, &PropertyQuery::default())).unwrap();
    assert!(!json.contains(&local()), "worker ids are lease credentials");
}

#[test]
fn without_a_declared_label_no_claim_is_made_and_the_chip_says_so() {
    let f = finished(&local());
    let p = page(&f, &PropertyQuery::default());
    assert_eq!(p.claim.text, None);
    assert!(p.claim.chips.contains(&"label not declared".to_owned()));
    assert!(p
        .claim
        .withheld
        .iter()
        .any(|w| w.contains("label not declared")));
}

#[test]
fn a_run_with_an_unconfirmed_failure_makes_no_claim_even_with_a_declared_label() {
    let f = finished(&local());
    let p = f
        .svc
        .property_page(&f.run_id, &PropertyQuery::default(), Some(&declared()))
        .unwrap()
        .unwrap();
    assert!(!p.claim.allowed);
    assert_eq!(p.claim.text, None);
    assert!(
        p.claim
            .withheld
            .iter()
            .any(|w| w.contains("no reproduction on a trusted worker")),
        "{:?}",
        p.claim.withheld
    );
}

fn clean_run(worker_trusted: bool) -> (hairspring_service::Service, String) {
    let (svc, _store, engine) = setup();
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    if worker_trusted {
        svc.set_worker_trust(&remote.id, true).unwrap();
    }
    let job = lease_with_budget(&svc, &engine, &remote.id, 3);
    let ok = props_multi(&[("a.fine", AssertKind::Always, "fine", 4, 0)]);
    let mut rec = Recorder::new();
    let reports = vec![rec.pass(1, &ok), rec.pass(2, &ok), rec.pass(3, &ok)];
    svc.ingest_trials_with_secret(&job.job_id, &remote.id, Some(&secret), &reports)
        .unwrap();
    finish(&svc, &job, &remote.id, passed(3, Some(3)));
    (svc, job.run_id)
}

#[test]
fn a_clean_run_from_a_trusted_worker_with_a_declared_label_gets_a_generated_claim() {
    let (svc, run) = clean_run(true);
    let p = svc
        .property_page(&run, &PropertyQuery::default(), Some(&declared()))
        .unwrap()
        .unwrap();
    assert!(p.claim.allowed, "{:?}", p.claim.withheld);
    let text = p.claim.text.as_deref().unwrap();
    assert!(
        text.starts_with("No trial failed a property in 3 rated trials"),
        "{text}"
    );
    assert!(text.contains("unmodified binary, blind seeds"), "{text}");
    assert!(p.claim.chips.contains(&"tier: unmodified".to_owned()));
    assert!(!p.header.unverified_worker_data);
}

#[test]
fn a_targeted_workload_flag_always_travels_with_the_generated_claim() {
    let (svc, run) = clean_run(true);
    let mut label = declared();
    label.knowledge = Some(vec![KnowledgeFlag::TargetedWorkload]);
    let p = svc
        .property_page(&run, &PropertyQuery::default(), Some(&label))
        .unwrap()
        .unwrap();
    assert!(p.claim.text.unwrap().contains("targeted workload"));
}

#[test]
fn unverified_worker_data_is_shown_with_a_chip_and_never_claimed() {
    let (svc, run) = clean_run(false);
    let p = svc
        .property_page(&run, &PropertyQuery::default(), Some(&declared()))
        .unwrap()
        .unwrap();
    assert!(p.header.unverified_worker_data);
    assert!(!p.claim.allowed);
    assert_eq!(p.claim.text, None);
    assert!(p.claim.chips.contains(&"unverified worker data".to_owned()));
    assert_eq!(
        p.properties.len(),
        1,
        "the figures are still shown, labelled"
    );
}

#[test]
fn the_never_hit_list_holds_the_usable_trials_that_did_not_reach_the_property() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    let plain = props_multi(&[("a.fine", AssertKind::Always, "fine", 4, 0)]);
    let rare = props_multi(&[
        ("a.fine", AssertKind::Always, "fine", 4, 0),
        ("x.rare", AssertKind::Sometimes, "rare", 1, 0),
    ]);
    let mut rec = Recorder::new();
    let reports = vec![rec.pass(1, &plain), rec.pass(2, &rare), rec.pass(3, &plain)];
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(3, Some(3)));
    let rows = svc
        .property_page(
            &job.run_id,
            &PropertyQuery {
                q: Some("x.".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .unwrap();
    let q = |kind: &str| hairspring_service::SeedQuery {
        seeds: Some(kind.into()),
        ..Default::default()
    };
    let idx = rows.properties[0].idx;
    let never = svc
        .property_detail(&job.run_id, idx, &q("never"))
        .unwrap()
        .unwrap();
    assert_eq!(
        never
            .seeds
            .iter()
            .map(|s| s.seed.as_str())
            .collect::<Vec<_>>(),
        ["1", "3"]
    );
    let hit = svc
        .property_detail(&job.run_id, idx, &q("hit"))
        .unwrap()
        .unwrap();
    assert_eq!(
        hit.seeds
            .iter()
            .map(|s| s.seed.as_str())
            .collect::<Vec<_>>(),
        ["2"]
    );
}

#[test]
fn the_headline_of_a_clean_run_on_untrusted_data_says_so() {
    let (svc, run) = clean_run(false);
    let p = svc
        .property_page(&run, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert_eq!(
        p.headline,
        "No trial failed a property in 3 rated trials (unverified worker data)"
    );
}

#[test]
fn the_headline_of_a_clean_run_on_trusted_data_has_no_qualifier() {
    let (svc, run) = clean_run(true);
    let p = svc
        .property_page(&run, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert_eq!(p.headline, "No trial failed a property in 3 rated trials");
}

#[test]
fn the_headline_of_a_run_ended_early_carries_its_completion_label() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 10);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials(&job.job_id, &local(), &reports).unwrap();
    finish(&svc, &job, &local(), passed(6, Some(6)));
    let p = svc
        .property_page(&job.run_id, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert!(
        p.headline
            .ends_with("(ended early: trial data is incomplete)"),
        "{}",
        p.headline
    );
}

#[test]
fn the_headline_of_a_run_stopped_at_its_first_failure_says_so() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials(&job.job_id, &local(), &reports[..3])
        .unwrap();
    finish(&svc, &job, &local(), failed_outcome(3));
    let p = svc
        .property_page(&job.run_id, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert!(
        p.headline.ends_with("(stopped at first failure)"),
        "{}",
        p.headline
    );
}

#[test]
fn the_headline_of_a_run_with_no_trial_rows_does_not_call_it_unverified_worker_data() {
    let (svc, _store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, &local(), 3);
    finish(&svc, &job, &local(), passed(3, None));
    let p = svc
        .property_page(&job.run_id, &PropertyQuery::default(), None)
        .unwrap()
        .unwrap();
    assert!(
        p.headline.contains("no worker trial data"),
        "{}",
        p.headline
    );
    assert!(!p.headline.contains("unverified"), "{}", p.headline);
}

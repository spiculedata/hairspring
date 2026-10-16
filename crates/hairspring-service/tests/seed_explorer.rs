//! What the seed explorer reads: a class and failing ids on every trial row, the strip of verdict codes, a property
//! without its seed list, and the checked filters.
mod common;

use common::fixture::*;
use common::*;
use hairspring_service::{PropertyQuery, SeedQuery, ServiceError, StripQuery, TrialQuery};

fn idx_of(f: &Fixture, id: &str) -> u32 {
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
}

fn strip(f: &Fixture, q: &StripQuery) -> hairspring_service::view::StripPage {
    f.svc.strip(&f.run_id, q).unwrap().unwrap()
}

#[test]
fn every_row_has_a_class_and_a_degraded_pass_is_never_a_pass() {
    let f = finished(&local());
    let page = f
        .svc
        .trial_page(&f.run_id, &TrialQuery::default())
        .unwrap()
        .unwrap();
    let classes: Vec<&str> = page.trials.iter().map(|t| t.class).collect();
    // Seed 3 failed; seed 5 was judged a pass by the engine but was degraded, so it is left out of every rate.
    assert_eq!(
        classes,
        ["pass", "pass", "violation", "pass", "excluded", "pass"]
    );
    assert_eq!(page.trials[4].verdict, "pass");
}

#[test]
fn a_failing_trial_names_the_properties_that_failed_in_it_under_the_guest_fence() {
    let f = finished(&local());
    let page = f
        .svc
        .trial_page(&f.run_id, &TrialQuery::default())
        .unwrap()
        .unwrap();
    let broken = idx_of(&f, "b.broken");
    let failed = &page.trials[2];
    assert_eq!(failed.failing_count, 1);
    assert_eq!(failed.failing_idx, [broken]);
    assert_eq!(failed.guest.failing_ids, ["b.broken"]);
    for t in page.trials.iter().filter(|t| t.seq != 2) {
        assert_eq!(t.failing_count, 0, "seq {}", t.seq);
        assert!(t.failing_idx.is_empty() && t.guest.failing_ids.is_empty());
    }
    let json = serde_json::to_value(&page).unwrap();
    assert!(json["trials"][2]["failing_idx"].is_array());
    assert!(json["trials"][2]["guest"]["failing_ids"].is_array());
    assert!(json["trials"][2].get("failing_ids").is_none());
}

#[test]
fn the_failing_ids_on_a_row_agree_with_the_fails_filter() {
    let f = finished(&local());
    let broken = idx_of(&f, "b.broken");
    let q = TrialQuery {
        fails: Some(broken.to_string()),
        ..Default::default()
    };
    let page = f.svc.trial_page(&f.run_id, &q).unwrap().unwrap();
    assert_eq!(page.trials.len(), 1);
    assert_eq!(page.trials[0].failing_idx, [broken]);
}

#[test]
fn the_strip_is_one_code_per_trial_in_judging_order() {
    let f = finished(&local());
    let s = strip(&f, &StripQuery::default());
    assert_eq!((s.first_seq, s.codes.as_str()), (0, "ppvpep"));
    assert_eq!(s.next_cursor, None);
    assert_eq!(s.scanned_trials, 6);
    // The legend is the server's own words for each code.
    let names: Vec<&str> = s.legend.iter().map(|l| l.name).collect();
    assert!(names.contains(&"violation") && names.contains(&"excluded"));
    assert!(s
        .codes
        .chars()
        .all(|c| s.legend.iter().any(|l| l.code == c)));
}

#[test]
fn the_strip_pages_with_a_cursor_and_a_page_starts_where_the_cursor_ends() {
    let f = finished(&local());
    let first = strip(
        &f,
        &StripQuery {
            limit: Some(4),
            cursor: None,
        },
    );
    assert_eq!((first.first_seq, first.codes.as_str()), (0, "ppvp"));
    assert_eq!(first.next_cursor.as_deref(), Some("3"));
    let second = strip(
        &f,
        &StripQuery {
            limit: Some(4),
            cursor: first.next_cursor,
        },
    );
    assert_eq!((second.first_seq, second.codes.as_str()), (4, "ep"));
    assert_eq!(second.next_cursor, None);
}

#[test]
fn the_strip_refuses_bad_parameters_and_an_unknown_run_is_none() {
    let f = finished(&local());
    for cursor in ["", "-1", "x", "99999999999999999999999"] {
        let r = f.svc.strip(
            &f.run_id,
            &StripQuery {
                cursor: Some(cursor.into()),
                limit: None,
            },
        );
        assert!(
            matches!(r, Err(ServiceError::InvalidRequest(_))),
            "{cursor:?}"
        );
    }
    assert!(f
        .svc
        .strip("run_0000000000000000", &StripQuery::default())
        .unwrap()
        .is_none());
}

#[test]
fn a_property_can_be_read_without_scanning_for_its_seeds() {
    let f = finished(&local());
    let broken = idx_of(&f, "b.broken");
    let d = f
        .svc
        .property_detail(
            &f.run_id,
            broken,
            &SeedQuery {
                seeds: Some("none".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(d.seeds_kind, "none");
    assert!(d.seeds.is_empty());
    assert_eq!(d.scanned_trials, 0);
    assert_eq!(d.property.guest.id, "b.broken");
    assert!(d.next_cursor.is_none());
}

#[test]
fn a_fingerprint_filter_is_hex_of_sixteen_to_sixty_four_digits() {
    let f = finished(&local());
    let with = |fp: &str| {
        f.svc.trial_page(
            &f.run_id,
            &TrialQuery {
                fp: Some(fp.into()),
                ..Default::default()
            },
        )
    };
    assert_eq!(with("0000000000000000").unwrap().unwrap().trials.len(), 2);
    assert!(with(&"A".repeat(64)).unwrap().unwrap().trials.is_empty());
    let long = "a".repeat(65);
    for bad in [
        "",
        "a b",
        "4c16a90b",
        "zzzzzzzzzzzzzzzz",
        "000000000000000g",
        "../../etc/passwd",
        long.as_str(),
    ] {
        let e = with(bad).unwrap_err();
        assert!(matches!(e, ServiceError::InvalidRequest(_)), "{bad:?}");
        assert!(
            !e.to_string().contains("passwd") && !e.to_string().contains("zzzz"),
            "an error never echoes the value: {e}"
        );
    }
}

#[test]
fn an_upper_case_fingerprint_matches_the_lower_case_one_stored() {
    let f = finished(&local());
    let page = f
        .svc
        .trial_page(
            &f.run_id,
            &TrialQuery {
                fp: Some("000000000000000A".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert!(page.trials.is_empty());
    let first = f
        .svc
        .trial_page(&f.run_id, &TrialQuery::default())
        .unwrap()
        .unwrap();
    let fp = first.trials[0].schedule_fp.clone().unwrap().to_uppercase();
    let upper = f
        .svc
        .trial_page(
            &f.run_id,
            &TrialQuery {
                fp: Some(fp),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert!(!upper.trials.is_empty());
}

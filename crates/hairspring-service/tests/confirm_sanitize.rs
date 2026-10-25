//! Check results a runner reports are validated before they are stored: an impossible result is refused whole, and text
//! is cut and cleaned rather than trusted. A runner is a separate process and its text is later shown to people and
//! to AI agents.
mod common;

use common::setup;
use hairspring_engine::confirm::*;
use hairspring_service::{Service, ServiceError};
use hairspring_store::{NewFailure, NewRun, Store};
use std::sync::Arc;

struct Rig {
    svc: Service,
    store: Arc<dyn Store>,
    failure: String,
}

fn rig() -> Rig {
    let (svc, store, _) = setup();
    let run = store
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = store.put_blob(b"bundle").unwrap();
    let failure = store
        .add_failure(
            &NewFailure {
                run_id: run.id,
                target: "t".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap()
        .id;
    Rig {
        svc,
        store,
        failure,
    }
}

fn good(kind: CheckKind) -> CheckResult {
    CheckResult::new(kind, CheckStatus::Pass)
        .with_counts(1, 0, 0)
        .with_times(100, 200)
}

fn record(rig: &Rig, checks: &[CheckResult]) -> Result<(), ServiceError> {
    rig.svc
        .record_confirmation(&rig.failure, "runner-1", false, checks)
        .map(|_| ())
}

fn stored(rig: &Rig) -> usize {
    rig.store.list_failure_checks(&rig.failure).unwrap().len()
}

fn refused(rig: &Rig, checks: &[CheckResult], why: &str) {
    let err = record(rig, checks).unwrap_err();
    assert!(
        matches!(err, ServiceError::InvalidRequest(_)),
        "{why}: {err:?}"
    );
    assert_eq!(stored(rig), 0, "{why}: nothing is stored");
}

#[test]
fn a_well_formed_confirmation_is_stored_with_the_runner_and_its_trust() {
    let rig = rig();
    record(
        &rig,
        &[
            good(CheckKind::SameSeedRerun),
            good(CheckKind::BundleReplay),
        ],
    )
    .unwrap();
    let rows = rig.store.list_failure_checks(&rig.failure).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].runner.as_str(), rows[0].runner_trusted),
        ("runner-1", false)
    );
}

#[test]
fn an_empty_or_oversized_confirmation_is_refused() {
    let rig = rig();
    refused(&rig, &[], "no checks");
    let too_many = vec![good(CheckKind::SameSeedRerun); 9];
    refused(&rig, &too_many, "more checks than kinds");
}

#[test]
fn a_kind_may_appear_once_in_a_confirmation() {
    let rig = rig();
    refused(
        &rig,
        &[
            good(CheckKind::SameSeedRerun),
            good(CheckKind::SameSeedRerun),
        ],
        "a repeated kind",
    );
}

#[test]
fn counts_must_fit_the_status_and_the_limits() {
    let rig = rig();
    let rerun = |status, p, f, i| {
        CheckResult::new(CheckKind::SameSeedRerun, status)
            .with_counts(p, f, i)
            .with_times(1, 2)
    };
    refused(
        &rig,
        &[rerun(CheckStatus::Pass, 3, 1, 0)],
        "a pass with a failed run",
    );
    refused(
        &rig,
        &[rerun(CheckStatus::Pass, 2, 0, 1)],
        "a pass with an inconclusive run",
    );
    refused(
        &rig,
        &[rerun(CheckStatus::Fail, 3, 0, 0)],
        "a fail with no failed run",
    );
    refused(
        &rig,
        &[rerun(CheckStatus::Skipped, 1, 0, 0)],
        "a skipped check that ran",
    );
    refused(
        &rig,
        &[rerun(CheckStatus::Pass, MAX_RERUNS + 1, 0, 0)],
        "more runs than the limit",
    );
    refused(
        &rig,
        &[rerun(
            CheckStatus::Inconclusive,
            u32::MAX,
            u32::MAX,
            u32::MAX,
        )],
        "counts that overflow",
    );
}

#[test]
fn times_must_be_sane() {
    let rig = rig();
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_times(500, 100)],
        "finished before it started",
    );
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_times(-1, 100)],
        "before the epoch",
    );
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_times(0, 3 * 24 * 3600 * 1000)],
        "longer than any confirmation may run",
    );
}

#[test]
fn the_environment_may_only_name_known_fields_with_valid_values() {
    let rig = rig();
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_environment("HOME", "/root")],
        "a field that is not part of the identity",
    );
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_environment("host.loader.libc.sha256", "ZZ")],
        "a hash that is not hex",
    );
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_environment("host.container", "a\nb")],
        "a value with a control character",
    );
    record(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_environment("host.container", "none")],
    )
    .unwrap();
}

#[test]
fn artefacts_are_hex_hashes() {
    let rig = rig();
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_artefact("not hex!")],
        "an artefact with other characters",
    );
    refused(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_artefact("ab".repeat(33))],
        "an artefact longer than a hash",
    );
    record(
        &rig,
        &[good(CheckKind::SameSeedRerun).with_artefact("00aa11bb22cc33dd")],
    )
    .unwrap();
}

#[test]
fn text_is_cut_and_cleaned_not_trusted_and_not_refused() {
    let rig = rig();
    let long = "x".repeat(50_000);
    let hostile = good(CheckKind::BundleReplay)
        .with_command(format!("hermit run\n\u{202E}{long}"))
        .with_detail("why\n", format!("</script>\u{0007}{long}"));
    let hostile = (0..40).fold(hostile, |c, i| c.with_detail(format!("k{i}"), "v"));
    record(&rig, &[hostile]).unwrap();
    let row = &rig.store.list_failure_checks(&rig.failure).unwrap()[0];
    assert!(row.command.chars().count() <= MAX_COMMAND_CHARS);
    assert!(!row.command.contains('\n') && !row.command.contains('\u{202E}'));
    let detail: serde_json::Value = serde_json::from_str(&row.detail_json).unwrap();
    let map = detail.as_object().unwrap();
    assert!(map.len() <= MAX_DETAIL_ENTRIES);
    assert!(map
        .values()
        .all(|v| v.as_str().unwrap().chars().count() <= MAX_DETAIL_VALUE_CHARS));
    assert!(map.keys().all(|k| !k.contains('\n')));
}

#[test]
fn an_unknown_failure_is_not_found_and_a_malformed_id_is_invalid() {
    let rig = rig();
    let err = rig
        .svc
        .record_confirmation(
            "fail_0123456789abcdef",
            "r",
            false,
            &[good(CheckKind::SameSeedRerun)],
        )
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
    let err = rig
        .svc
        .record_confirmation("x'; --", "r", false, &[good(CheckKind::SameSeedRerun)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
}

/// The four checks of a confirmation that began at `at` (so that two of them are not the same report).
fn four_at(at: i64) -> Vec<CheckResult> {
    [
        CheckKind::SameSeedRerun,
        CheckKind::BundleReplay,
        CheckKind::FixedControl,
        CheckKind::StrictVerify,
    ]
    .into_iter()
    .map(|k| good(k).with_times(at, at + 1))
    .collect()
}

#[test]
fn a_full_failure_keeps_its_newest_checks_and_accepts_another_confirmation() {
    let rig = rig();
    let rounds = hairspring_store::MAX_CHECKS_PER_FAILURE as usize / four_at(0).len();
    for i in 0..rounds {
        record(&rig, &four_at(i as i64)).unwrap();
    }
    assert_eq!(stored(&rig), rounds * 4);
    record(&rig, &four_at(10_000)).unwrap();
    let rows = rig.store.list_failure_checks(&rig.failure).unwrap();
    assert!(rows.len() as u64 <= hairspring_store::MAX_CHECKS_PER_FAILURE);
    assert_eq!(rows.last().unwrap().started_at, 10_000);
    assert!(
        rows.iter().all(|r| r.started_at != 0),
        "the oldest confirmation made room"
    );
}

#[test]
fn the_same_report_sent_again_is_stored_once() {
    let rig = rig();
    let checks = [
        good(CheckKind::SameSeedRerun),
        good(CheckKind::BundleReplay),
    ];
    let first = rig
        .svc
        .record_confirmation(&rig.failure, "runner-1", true, &checks)
        .unwrap();
    let again = rig
        .svc
        .record_confirmation(&rig.failure, "runner-1", true, &checks)
        .unwrap();
    assert_eq!(first.confirmation_id, again.confirmation_id);
    assert_eq!(first.outcome, hairspring_store::ConfirmationOutcome::Stored);
    assert_eq!(
        again.outcome,
        hairspring_store::ConfirmationOutcome::Duplicate
    );
    assert_eq!(stored(&rig), 2);
}

#[test]
fn a_report_that_differs_is_a_new_confirmation() {
    let rig = rig();
    record(&rig, &[good(CheckKind::SameSeedRerun)]).unwrap();
    record(&rig, &[good(CheckKind::SameSeedRerun).with_times(300, 400)]).unwrap();
    assert_eq!(stored(&rig), 2);
}

#[test]
fn text_of_wide_characters_is_cut_to_fit_and_stored_not_refused() {
    let rig = rig();
    let wide = "\u{1D11E}".repeat(MAX_DETAIL_VALUE_CHARS);
    let mut check = good(CheckKind::SameSeedRerun).with_command("\"".repeat(MAX_COMMAND_CHARS));
    for i in 0..MAX_DETAIL_ENTRIES {
        check = check.with_detail(format!("{i:0>2}{}", "\u{1D11E}".repeat(30)), wide.clone());
    }
    record(&rig, &[check]).unwrap();
    let row = &rig.store.list_failure_checks(&rig.failure).unwrap()[0];
    assert!(row.detail_json.len() <= hairspring_store::MAX_CHECK_TEXT_BYTES);
}

#[test]
fn a_runner_label_is_cut_by_bytes_on_a_character_boundary() {
    let rig = rig();
    let runner = "\u{1D11E}".repeat(hairspring_store::MAX_CHECK_TAG_BYTES);
    rig.svc
        .record_confirmation(
            &rig.failure,
            &runner,
            false,
            &[good(CheckKind::SameSeedRerun)],
        )
        .unwrap();
    let stored_runner = rig.store.list_failure_checks(&rig.failure).unwrap()[0]
        .runner
        .clone();
    assert!(stored_runner.len() <= hairspring_store::MAX_CHECK_TAG_BYTES);
    assert!(!stored_runner.is_empty());
    assert!(runner.starts_with(&stored_runner));
}

#[test]
fn the_engine_and_the_store_agree_on_how_long_a_column_may_be() {
    assert_eq!(MAX_COLUMN_BYTES, hairspring_store::MAX_CHECK_TEXT_BYTES);
}

#[test]
fn an_empty_runner_label_is_a_bad_request_not_a_server_error() {
    let rig = rig();
    let err = rig
        .svc
        .record_confirmation(&rig.failure, "", false, &[good(CheckKind::SameSeedRerun)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
    assert_eq!(stored(&rig), 0);
}

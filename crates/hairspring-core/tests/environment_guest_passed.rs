//! `env.guest.passed`: whether the engine named its variables for the guest. A recording without it reads as `none`.
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{
    compare, known_field_names, EnvError, EnvironmentComparison, EnvironmentIdentity, FieldChange,
};

const PASSED: &str = "env.guest.passed";

fn recorded() -> EnvironmentIdentity {
    [
        ("host.kernel_release", "6.8.0-45-generic"),
        ("env.base_env", "minimal"),
        ("hermit.path.sha256", "bbbbbbbbbbbbbbbbbbbbbbbb"),
    ]
    .iter()
    .fold(EnvironmentIdentity::default(), |acc, (k, v)| {
        acc.with_field(*k, *v).unwrap()
    })
}

fn with_passed(value: &str) -> EnvironmentIdentity {
    recorded().with_field(PASSED, value).unwrap()
}

#[test]
fn the_field_is_known_and_takes_none_or_a_versioned_name() {
    assert!(known_field_names().any(|n| n == PASSED));
    assert_eq!(known_field_names().count(), 18);
    for ok in ["none", "hairspring-1", "hairspring-12"] {
        assert!(recorded().with_field(PASSED, ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "yes",
        "hairspring-",
        "hairspring-x",
        "hairspring-1 ",
        "NONE",
    ] {
        assert_eq!(
            recorded().with_field(PASSED, bad),
            Err(EnvError::BadValue(PASSED.to_string())),
            "{bad:?}"
        );
    }
}

#[test]
fn a_recording_without_the_field_reads_as_none_when_the_current_side_has_it() {
    assert_eq!(
        compare(Some(&recorded()), &with_passed("none")),
        EnvironmentComparison::Same
    );
    let EnvironmentComparison::Differs(changes) =
        compare(Some(&recorded()), &with_passed("hairspring-1"))
    else {
        panic!("an old recording compared with the new default must differ");
    };
    assert_eq!(
        changes,
        [FieldChange {
            name: PASSED.to_string(),
            recorded: "none".to_string(),
            current: Some("hairspring-1".to_string()),
        }]
    );
}

#[test]
fn neither_side_having_the_field_is_no_difference() {
    assert_eq!(
        compare(Some(&recorded()), &recorded()),
        EnvironmentComparison::Same
    );
}

#[test]
fn a_recorded_field_the_current_side_lacks_is_still_a_difference() {
    let EnvironmentComparison::Differs(changes) =
        compare(Some(&with_passed("hairspring-1")), &recorded())
    else {
        panic!("a recorded field the host cannot give is a change");
    };
    assert_eq!(changes[0].current, None);
}

#[test]
fn a_new_recording_differs_from_a_target_that_no_longer_passes() {
    let line = compare(Some(&with_passed("hairspring-1")), &with_passed("none"))
        .warning()
        .unwrap();
    assert!(
        line.contains("env.guest.passed (recorded hairspring-1, now none)"),
        "{line}"
    );
}

#[test]
fn a_bundle_with_no_environment_is_unchanged_by_the_default() {
    assert_eq!(
        compare(None, &with_passed("hairspring-1")),
        EnvironmentComparison::NotRecorded
    );
}

#[test]
fn the_default_is_applied_to_what_a_bundle_recorded() {
    let old = recorded().apply_to(Bundle::new(1, "b", vec![]));
    let parsed = EnvironmentIdentity::from_bundle(&old);
    assert!(parsed.malformed.is_empty());
    assert_eq!(
        compare(parsed.identity.as_ref(), &with_passed("none")),
        EnvironmentComparison::Same
    );
}

#[test]
fn a_tampered_value_in_a_bundle_is_listed_as_malformed() {
    let tampered = Bundle::new(1, "b", vec![])
        .with_param(PASSED, "everything")
        .with_param("env.base_env", "minimal");
    let parsed = EnvironmentIdentity::from_bundle(&tampered);
    assert_eq!(parsed.malformed, [PASSED.to_string()]);
}

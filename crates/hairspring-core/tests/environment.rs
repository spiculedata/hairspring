use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{
    compare, EnvError, EnvironmentComparison, EnvironmentIdentity, MAX_FIELD_NAME_CHARS,
    MAX_FIELD_VALUE_CHARS,
};

fn identity(pairs: &[(&str, &str)]) -> EnvironmentIdentity {
    pairs
        .iter()
        .fold(EnvironmentIdentity::default(), |acc, (k, v)| {
            acc.with_field(*k, *v).unwrap()
        })
}

fn recorded() -> EnvironmentIdentity {
    identity(&[
        ("host.kernel_release", "6.8.0-45-generic"),
        ("host.loader.ld_so_cache.sha256", "aaaaaaaaaaaaaaaaaaaaaaaa"),
        ("env.base_env", "minimal"),
        ("hermit.path.sha256", "bbbbbbbbbbbbbbbbbbbbbbbb"),
    ])
}

#[test]
fn identical_environments_compare_as_same() {
    assert_eq!(
        compare(Some(&recorded()), &recorded()),
        EnvironmentComparison::Same
    );
}

#[test]
fn a_bundle_without_an_environment_says_so() {
    assert_eq!(
        compare(None, &recorded()),
        EnvironmentComparison::NotRecorded
    );
}

#[test]
fn each_changed_field_is_named() {
    let now = identity(&[
        ("host.kernel_release", "6.9.1"),
        ("host.loader.ld_so_cache.sha256", "cccccccccccccccccccccccc"),
        ("env.base_env", "host"),
        ("hermit.path.sha256", "dddddddddddddddddddddddd"),
    ]);
    let EnvironmentComparison::Differs(changes) = compare(Some(&recorded()), &now) else {
        panic!("expected a difference");
    };
    let names: Vec<&str> = changes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "env.base_env",
            "hermit.path.sha256",
            "host.kernel_release",
            "host.loader.ld_so_cache.sha256"
        ]
    );
}

#[test]
fn a_field_missing_now_is_a_difference() {
    let now = identity(&[("host.kernel_release", "6.8.0-45-generic")]);
    let EnvironmentComparison::Differs(changes) = compare(Some(&recorded()), &now) else {
        panic!("expected a difference");
    };
    assert!(changes
        .iter()
        .any(|c| c.name == "env.base_env" && c.current.is_none()));
}

#[test]
fn a_field_only_known_now_is_not_a_difference() {
    let now = recorded().with_field("host.container", "none").unwrap();
    assert_eq!(
        compare(Some(&recorded()), &now),
        EnvironmentComparison::Same
    );
}

#[test]
fn the_warning_lists_fields_with_short_values() {
    let now = recorded()
        .with_field("host.kernel_release", "6.9.1")
        .unwrap()
        .with_field("hermit.path.sha256", "eeeeeeeeeeeeeeeeeeeeeeee")
        .unwrap();
    let text = compare(Some(&recorded()), &now).warning().unwrap();
    assert!(text.starts_with("environment differs: "), "{text}");
    assert!(
        text.contains("host.kernel_release (recorded 6.8.0-45-generic, now 6.9.1)"),
        "{text}"
    );
    assert!(
        text.contains("hermit.path.sha256 (recorded bbbbbbbbbbbb, now eeeeeeeeeeee)"),
        "{text}"
    );
}

#[test]
fn same_has_no_warning_and_not_recorded_has_a_note() {
    assert_eq!(compare(Some(&recorded()), &recorded()).warning(), None);
    let note = compare(None, &recorded()).warning().unwrap();
    assert!(note.contains("no environment recorded"), "{note}");
}

#[test]
fn fields_must_use_a_known_namespace() {
    let err = EnvironmentIdentity::default()
        .with_field("user.name", "tom")
        .unwrap_err();
    assert!(matches!(err, EnvError::UnknownNamespace(_)), "{err}");
}

#[test]
fn field_names_and_values_are_bounded_and_clean() {
    let long_name = format!("host.{}", "a".repeat(MAX_FIELD_NAME_CHARS));
    assert!(matches!(
        EnvironmentIdentity::default().with_field(long_name, "x"),
        Err(EnvError::BadName(_))
    ));
    assert!(matches!(
        EnvironmentIdentity::default().with_field("host.Bad Name", "x"),
        Err(EnvError::BadName(_))
    ));
    assert!(matches!(
        EnvironmentIdentity::default().with_field("host.a", "x".repeat(MAX_FIELD_VALUE_CHARS + 1)),
        Err(EnvError::BadValue(_))
    ));
    assert!(matches!(
        EnvironmentIdentity::default().with_field("host.a", "line\nbreak"),
        Err(EnvError::BadValue(_))
    ));
}

#[test]
fn an_identity_round_trips_through_a_v3_bundle() {
    let bundle = recorded().apply_to(Bundle::new(1, "b", vec![]).with_backend("hermit"));
    let text = bundle.encode();
    assert!(text.starts_with("hairspring-bundle v3\n"), "{text}");
    let back = Bundle::decode(&text).unwrap();
    assert_eq!(
        EnvironmentIdentity::from_bundle(&back).unwrap(),
        Some(recorded())
    );
}

#[test]
fn a_bundle_without_environment_keeps_the_v2_header_and_reads_as_none() {
    let bundle = Bundle::new(1, "b", vec![]).with_param("sched_seed", "1");
    let text = bundle.encode();
    assert!(text.starts_with("hairspring-bundle v2\n"), "{text}");
    let back = Bundle::decode(&text).unwrap();
    assert_eq!(EnvironmentIdentity::from_bundle(&back).unwrap(), None);
}

#[test]
fn a_v2_header_with_environment_params_still_reads() {
    let text = "hairspring-bundle v2\nseed=1\nbuild=x\nparam.host.kernel_release=6.8\npath=\n";
    let back = Bundle::decode(text).unwrap();
    let id = EnvironmentIdentity::from_bundle(&back).unwrap().unwrap();
    assert_eq!(id.get("host.kernel_release"), Some("6.8"));
}

#[test]
fn a_malformed_recorded_field_is_an_error_not_silently_dropped() {
    let text = "hairspring-bundle v3\nseed=1\nbuild=x\nparam.host.Bad Name=1\npath=\n";
    let back = Bundle::decode(text).unwrap();
    assert!(EnvironmentIdentity::from_bundle(&back).is_err());
}

#[test]
fn applying_leaves_the_original_bundle_untouched() {
    let original = Bundle::new(1, "b", vec![]);
    let _ = recorded().apply_to(original.clone());
    assert!(original.params.is_empty());
}

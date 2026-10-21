use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{
    compare, EnvError, EnvironmentComparison, EnvironmentIdentity, FieldChange,
    MAX_FIELD_VALUE_CHARS, MAX_LISTED_CHANGES,
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
fn a_field_the_host_cannot_provide_is_unavailable_not_changed() {
    let now = identity(&[("env.base_env", "minimal")]);
    let text = compare(Some(&recorded()), &now).warning().unwrap();
    assert!(
        text.contains("hermit.path.sha256 (recorded bbbbbbbbbbbb, now unavailable)"),
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
fn a_very_long_list_of_changes_is_cut_with_a_count() {
    let changes: Vec<FieldChange> = (0..5000)
        .map(|i| FieldChange {
            name: format!("host.field_{i}"),
            recorded: "a".repeat(200),
            current: Some("b".repeat(200)),
        })
        .collect();
    let text = EnvironmentComparison::Differs(changes).warning().unwrap();
    assert!(
        text.contains(&format!("and {} more", 5000 - MAX_LISTED_CHANGES)),
        "{text}"
    );
    assert!(text.contains("host.field_0 "), "{text}");
    assert!(!text.contains("host.field_4999"), "{text}");
    assert!(text.len() < 4000, "{}", text.len());
}

#[test]
fn fields_must_have_a_known_name() {
    for name in ["user.name", "host.future_field", "target.anything"] {
        let err = EnvironmentIdentity::default()
            .with_field(name, "tom")
            .unwrap_err();
        assert!(matches!(err, EnvError::UnknownField(_)), "{name}: {err}");
    }
}

#[test]
fn values_are_bounded_and_clean() {
    let bad = |value: String| {
        matches!(
            EnvironmentIdentity::default().with_field("host.container", value),
            Err(EnvError::BadValue(_))
        )
    };
    assert!(bad("x".repeat(MAX_FIELD_VALUE_CHARS + 1)));
    assert!(bad("line\nbreak".to_string()));
    assert!(bad("right\u{202E}to-left".to_string()));
    assert!(bad("zero\u{200B}width".to_string()));
    assert!(bad("mark\u{FEFF}".to_string()));
}

#[test]
fn values_are_printable_ascii_only() {
    let bad = |name: &str, value: &str| {
        matches!(
            EnvironmentIdentity::default().with_field(name, value),
            Err(EnvError::BadValue(_))
        )
    };
    for value in [
        "line\u{2028}sep",
        "para\u{2029}sep",
        "soft\u{00AD}hyphen",
        "tag\u{E0041}char",
        "caf\u{e9}",
        "tab\there",
        "del\u{7f}",
    ] {
        assert!(bad("host.container", value), "{value:?}");
    }
    assert!(!bad("host.cpu_model", "Fake CPU @ 3.0GHz (R)"));
}

#[test]
fn hash_fields_accept_only_lowercase_hex() {
    let bad = |name: &str, value: &str| {
        matches!(
            EnvironmentIdentity::default().with_field(name, value),
            Err(EnvError::BadValue(_))
        )
    };
    assert!(bad("hermit.path.sha256", "not hex"));
    assert!(bad("hermit.path.sha256", "ABCDEF"));
    assert!(bad("host.loader.libc.sha256", ""));
    assert!(!bad("hermit.path.sha256", &"a".repeat(64)));
    // `absent` is how "no /etc/ld.so.preload" is recorded.
    assert!(!bad("host.loader.ld_so_preload.sha256", "absent"));
    assert!(bad("host.loader.libc.sha256", "absent"));
}

#[test]
fn an_identity_round_trips_through_a_v3_bundle() {
    let bundle = recorded().apply_to(Bundle::new(1, "b", vec![]).with_backend("hermit"));
    let text = bundle.encode();
    assert!(text.starts_with("hairspring-bundle v3\n"), "{text}");
    let back = Bundle::decode(&text).unwrap();
    let parsed = EnvironmentIdentity::from_bundle(&back);
    assert_eq!(parsed.identity, Some(recorded()));
    assert!(parsed.malformed.is_empty());
}

#[test]
fn a_bundle_without_environment_keeps_the_v2_header_and_reads_as_none() {
    let bundle = Bundle::new(1, "b", vec![]).with_param("sched_seed", "1");
    let text = bundle.encode();
    assert!(text.starts_with("hairspring-bundle v2\n"), "{text}");
    let back = Bundle::decode(&text).unwrap();
    assert_eq!(EnvironmentIdentity::from_bundle(&back).identity, None);
}

#[test]
fn a_param_with_an_environment_prefix_but_no_known_name_is_not_a_record() {
    let bundle = Bundle::new(1, "b", vec![]).with_param("host.future_field", "1");
    assert!(bundle.encode().starts_with("hairspring-bundle v2\n"));
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(parsed.identity, None);
    assert!(parsed.malformed.is_empty());
}

#[test]
fn a_v2_header_with_environment_params_still_reads() {
    let text = "hairspring-bundle v2\nseed=1\nbuild=x\nparam.host.kernel_release=6.8\npath=\n";
    let back = Bundle::decode(text).unwrap();
    let id = EnvironmentIdentity::from_bundle(&back).identity.unwrap();
    assert_eq!(id.get("host.kernel_release"), Some("6.8"));
}

#[test]
fn one_malformed_field_does_not_switch_off_the_rest() {
    let text = "hairspring-bundle v3\nseed=1\nbuild=x\nparam.host.container=bad%0Avalue\nparam.host.kernel_release=6.8\nparam.env.base_env=minimal\npath=\n";
    let back = Bundle::decode(text).unwrap();
    let parsed = EnvironmentIdentity::from_bundle(&back);
    assert_eq!(parsed.malformed, ["host.container"]);
    let id = parsed.identity.clone().unwrap();
    assert_eq!(id.get("host.kernel_release"), Some("6.8"));
    let now = identity(&[("host.kernel_release", "6.9"), ("env.base_env", "minimal")]);
    assert!(compare(Some(&id), &now).differs());
    assert!(parsed
        .warning()
        .unwrap()
        .contains("not compared: host.container"));
}

#[test]
fn applying_leaves_the_original_bundle_untouched() {
    let original = Bundle::new(1, "b", vec![]);
    let _ = recorded().apply_to(original.clone());
    assert!(original.params.is_empty());
}

//! Characterisation of what replay concludes for a table of recorded and current environments. These rows were written
//! against the behaviour before the field registry and must keep passing unchanged: recorded fields, comparison
//! results, warnings and refusals are not allowed to move.
use crate::replay_environment::check;
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::EnvironmentIdentity;
use hairspring_engine::EngineError;

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

type Rows = Vec<(&'static str, String)>;

/// The fields a current Hermit host reports, as a base the rows then change.
fn base() -> Rows {
    [
        ("host.kernel_release", "6.8.0-45-generic"),
        ("host.container", "none"),
        ("host.loader.ld_so_cache.sha256", HASH_A),
        ("hermit.path.sha256", HASH_A),
        ("env.base_env", "default"),
        ("env.guest.passed", "none"),
        ("target.command.sha256", HASH_B),
        ("target.cwd.sha256", HASH_C),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect()
}

fn with(changes: &[(&'static str, &str)]) -> Rows {
    let mut rows = base();
    for (name, value) in changes {
        rows.retain(|(k, _)| k != name);
        rows.push((name, value.to_string()));
    }
    rows
}

fn without(name: &str) -> Rows {
    let mut rows = base();
    rows.retain(|(k, _)| *k != name);
    rows
}

fn identity_of(rows: &Rows) -> EnvironmentIdentity {
    rows.iter()
        .fold(EnvironmentIdentity::default(), |id, (k, v)| {
            id.with_field(*k, v.as_str()).unwrap()
        })
}

fn verdict(
    recorded: &EnvironmentIdentity,
    current: &EnvironmentIdentity,
    allow: bool,
) -> Result<(bool, bool, Vec<String>), String> {
    let bundle = recorded.apply_to(Bundle::new(1, "b", vec![]));
    match check(&bundle, current, allow) {
        Ok(c) => Ok((c.differs, c.target_differs, c.warnings)),
        Err(EngineError::EnvironmentMismatch(why)) => Err(why),
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

fn now() -> EnvironmentIdentity {
    identity_of(&base())
}

#[test]
fn an_unchanged_environment_is_the_same() {
    assert_eq!(verdict(&now(), &now(), false), Ok((false, false, vec![])));
}

#[test]
fn an_old_bundle_without_the_guest_passed_field_reads_as_none() {
    let old = identity_of(&without("env.guest.passed"));
    assert_eq!(verdict(&old, &now(), false), Ok((false, false, vec![])));
}

#[test]
fn an_old_bundle_meets_the_new_default_and_says_how_to_replay_as_recorded() {
    let old = identity_of(&without("env.guest.passed"));
    let current = identity_of(&with(&[("env.guest.passed", "hairspring-1")]));
    let (differs, target, warnings) = verdict(&old, &current, false).unwrap();
    assert!(differs);
    assert!(!target);
    assert_eq!(
        warnings,
        [
            "environment differs: env.guest.passed (recorded none, now hairspring-1)".to_string(),
            "the bundle was recorded with env.guest.passed=none and this target now uses hairspring-1; set pass_env = false on the target to replay as recorded".to_string(),
        ]
    );
}

#[test]
fn a_changed_host_kernel_is_a_reported_difference() {
    let current = identity_of(&with(&[("host.kernel_release", "6.9.1")]));
    let (differs, target, warnings) = verdict(&now(), &current, false).unwrap();
    assert!(differs);
    assert!(!target);
    assert_eq!(
        warnings,
        ["environment differs: host.kernel_release (recorded 6.8.0-45-generic, now 6.9.1)"]
    );
}

#[test]
fn a_changed_base_env_mode_is_a_reported_difference() {
    let current = identity_of(&with(&[("env.base_env", "minimal")]));
    let (differs, _, warnings) = verdict(&now(), &current, false).unwrap();
    assert!(differs);
    assert_eq!(
        warnings,
        ["environment differs: env.base_env (recorded default, now minimal)"]
    );
}

#[test]
fn a_changed_hermit_binary_is_a_reported_difference() {
    let current = identity_of(&with(&[("hermit.path.sha256", HASH_B)]));
    let (differs, _, warnings) = verdict(&now(), &current, false).unwrap();
    assert!(differs);
    assert_eq!(
        warnings,
        ["environment differs: hermit.path.sha256 (recorded aaaaaaaaaaaa, now bbbbbbbbbbbb)"]
    );
}

#[test]
fn a_changed_loader_is_refused_unless_allowed() {
    let current = identity_of(&with(&[("host.loader.ld_so_cache.sha256", HASH_B)]));
    let why = verdict(&now(), &current, false).unwrap_err();
    assert!(
        why.starts_with("replay refused, the dynamic loader changed"),
        "{why}"
    );
    assert!(why.contains("--allow-environment-change"), "{why}");
    assert!(
        why.contains("host.loader.ld_so_cache.sha256 (recorded aaaaaaaaaaaa, now bbbbbbbbbbbb)"),
        "{why}"
    );
    let (differs, _, warnings) = verdict(&now(), &current, true).unwrap();
    assert!(differs);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn a_changed_command_line_is_a_target_difference() {
    let current = identity_of(&with(&[("target.command.sha256", HASH_A)]));
    let (differs, target, warnings) = verdict(&now(), &current, false).unwrap();
    assert!(differs && target);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[1].starts_with(
        "command line or working directory differs from the recording (target.command.sha256)"
    ));
}

#[test]
fn a_hash_missing_from_the_recording_is_said_aloud() {
    let old = identity_of(&without("hermit.path.sha256"));
    let (differs, _, warnings) = verdict(&old, &now(), false).unwrap();
    assert!(!differs);
    assert_eq!(
        warnings,
        ["hermit.path.sha256 was not recorded: a different Hermit build would not be noticed"]
    );
}

#[test]
fn a_malformed_recorded_field_counts_as_a_change() {
    let host = identity_of(&vec![("host.container", "none".to_string())]);
    let bundle = host
        .apply_to(Bundle::new(1, "b", vec![]))
        .with_param("host.loader.libc.sha256", "not-a-hash");
    let out = check(&bundle, &host, false).unwrap();
    assert!(out.differs);
    assert!(out.warnings.iter().any(
        |w| w == "malformed recorded environment fields, not compared: host.loader.libc.sha256"
    ));
}

#[test]
fn a_bundle_without_an_environment_gets_only_the_no_record_note() {
    let out = check(&Bundle::new(1, "b", vec![]), &now(), false).unwrap();
    assert!(!out.differs);
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].starts_with("no environment recorded"));
}

// Bundles written by a newer build: fields this build does not know.

fn bundle_listing(listed: &str) -> Bundle {
    now()
        .apply_to(Bundle::new(1, "b", vec![]))
        .with_param("env.fields", listed)
}

#[test]
fn a_bundle_with_a_field_this_build_does_not_know_is_a_changed_environment() {
    let listed = "env.base_env,env.guest.passed,hermit.future_flag,hermit.path.sha256,host.container,host.kernel_release,host.loader.ld_so_cache.sha256,target.command.sha256,target.cwd.sha256";
    let out = check(&bundle_listing(listed), &now(), false).unwrap();
    assert!(out.differs);
    assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
    assert!(out.warnings[0].starts_with(
        "environment differs: this bundle records environment fields this build does not know (hermit.future_flag)"
    ));
    assert!(out.warnings[0].contains("upgrade Hairspring"));
}

#[test]
fn an_unknown_field_marked_informational_changes_nothing() {
    let bundle = bundle_listing("env.base_env,~hermit.future_note")
        .with_param("hermit.future_note", "anything");
    let out = check(&bundle, &now(), false).unwrap();
    assert!(!out.differs);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn an_unknown_field_is_not_refused_as_a_loader_change() {
    let bundle =
        bundle_listing("host.loader.future_thing").with_param("host.loader.future_thing", "x");
    let out = check(&bundle, &now(), false).unwrap();
    assert!(out.differs);
}

#[test]
fn a_bundle_that_lists_only_known_fields_behaves_as_before() {
    let bundle = now().apply_to(Bundle::new(1, "b", vec![]));
    assert!(bundle.params.contains_key("env.fields"));
    let out = check(&bundle, &now(), false).unwrap();
    assert_eq!((out.differs, out.warnings), (false, vec![]));
}

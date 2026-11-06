//! `hairspring replay` reports how the host differs from the one a bundle was recorded on.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE_HERMIT: &str = "#!/bin/sh\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n";
const PROJECT: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", \"[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: lost update'; exit 1; }; echo ok\"]\n[target.svc.budget]\nmax_trials = 20\n";
const FAKE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hairspring-renv-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, hermit: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", hermit)
        .output()
        .expect("binary runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A project whose seed 3 fails, with its failing bundle written next to it; returns the bundle's text.
fn recorded(name: &str) -> (PathBuf, PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch(name);
    let hermit = d.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(d.join("hairspring.toml"), PROJECT).unwrap();
    let o = run(&d, &hermit, &["test", "svc", "--seed", "1"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let file = std::fs::read_dir(d.join(".hairspring/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let body = std::fs::read_to_string(file).unwrap();
    (d, hermit, body)
}

fn write_bundle(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path.to_str().unwrap().to_string()
}

fn with_extra_param(bundle: &str, line: &str) -> String {
    bundle.replacen("\npath=", &format!("\n{line}\npath="), 1)
}

#[test]
fn a_new_bundle_records_the_environment_and_replays_without_a_warning() {
    let (d, hermit, body) = recorded("same");
    assert!(body.starts_with("hairspring-bundle v3\n"), "{body}");
    assert!(body.contains("param.env.base_env=default"), "{body}");
    let file = write_bundle(&d, "b.hairspring", &body);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(!text(&o).contains("environment"), "{}", text(&o));
}

#[test]
fn a_changed_field_is_listed_precisely() {
    let (d, hermit, body) = recorded("changed");
    let edited = with_extra_param(&body, "param.host.kernel_release=1.2.3-old");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o)
            .contains("warning: environment differs: host.kernel_release (recorded 1.2.3-old, now"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_changed_loader_is_refused_unless_explicitly_allowed() {
    let (d, hermit, body) = recorded("loader");
    let edited = with_extra_param(
        &body,
        &format!("param.host.loader.ld_so_cache.sha256={FAKE_SHA}"),
    );
    let file = write_bundle(&d, "b.hairspring", &edited);
    let refused = run(&d, &hermit, &["replay", &file]);
    assert_eq!(refused.status.code(), Some(2), "{}", text(&refused));
    assert!(
        text(&refused).contains("host.loader.ld_so_cache.sha256"),
        "{}",
        text(&refused)
    );
    assert!(
        text(&refused).contains("--allow-environment-change"),
        "{}",
        text(&refused)
    );
    let allowed = run(
        &d,
        &hermit,
        &["replay", "--allow-environment-change", &file],
    );
    assert_eq!(allowed.status.code(), Some(1), "{}", text(&allowed));
    assert!(
        text(&allowed).contains("warning: environment differs"),
        "{}",
        text(&allowed)
    );
}

/// The recorded seed is changed to one that passes, so the replay misses.
fn passing_seed(bundle: &str) -> String {
    assert!(bundle.contains("param.sched_seed=3"), "{bundle}");
    bundle.replace("param.sched_seed=3", "param.sched_seed=1")
}

#[test]
fn a_miss_on_a_changed_host_has_its_own_exit_code() {
    let (d, hermit, body) = recorded("inconclusive");
    let edited = with_extra_param(&passing_seed(&body), "param.host.kernel_release=1.2.3-old");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains("inconclusive"), "{}", text(&o));
    assert!(
        text(&o).contains("does not show the bug is fixed"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_tampered_loader_hash_with_a_miss_exits_inconclusive_not_zero() {
    let (d, hermit, body) = recorded("tampered");
    let edited = with_extra_param(
        &passing_seed(&body),
        "param.host.loader.ld_so_cache.sha256=not-a-hash",
    );
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains("malformed"), "{}", text(&o));
}

#[test]
fn a_miss_on_an_unchanged_host_still_exits_zero() {
    let (d, hermit, body) = recorded("fixed");
    let file = write_bundle(&d, "b.hairspring", &passing_seed(&body));
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        text(&o).contains("NOT reproduced: the bundle no longer fails"),
        "{}",
        text(&o)
    );
}

#[test]
fn the_help_lists_the_exit_codes() {
    let (d, hermit, _) = recorded("help");
    let o = run(&d, &hermit, &["replay", "--help"]);
    let help = text(&o);
    assert!(help.contains("Exit codes"), "{help}");
    assert!(
        help.contains("3 not reproduced on a changed environment"),
        "{help}"
    );
}

#[test]
fn a_bundle_without_an_environment_says_so() {
    let (d, hermit, body) = recorded("old");
    let old: String = body
        .lines()
        .filter(|l| {
            ![
                "param.host.",
                "param.hermit.",
                "param.env.",
                "param.target.",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        })
        .map(|l| format!("{}\n", l.replace("bundle v3", "bundle v2")))
        .collect();
    let file = write_bundle(&d, "old.hairspring", &old);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("warning: no environment recorded"),
        "{}",
        text(&o)
    );
}

/// The bundle with the recorded value of `name` replaced, so the recording looks like another directory or command.
fn with_param_replaced(bundle: &str, name: &str) -> String {
    let prefix = format!("param.{name}=");
    assert!(bundle.contains(&prefix), "{bundle}");
    bundle
        .lines()
        .map(|l| match l.starts_with(&prefix) {
            true => format!("{prefix}{FAKE_SHA}\n"),
            false => format!("{l}\n"),
        })
        .collect()
}

#[test]
fn a_changed_working_directory_is_reported_and_a_miss_is_inconclusive() {
    let (d, hermit, body) = recorded("cwd");
    let edited = with_param_replaced(&passing_seed(&body), "target.cwd.sha256");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(
        text(&o).contains(
            "warning: command line or working directory differs from the recording (target.cwd.sha256)"
        ),
        "{}",
        text(&o)
    );
    assert!(
        text(&o).contains("possible cause: the command line or working directory differs"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_changed_command_is_reported() {
    let (d, hermit, body) = recorded("cmd");
    let edited = with_param_replaced(&body, "target.command.sha256");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("(target.command.sha256)"), "{}", text(&o));
}

#[test]
fn a_new_bundle_holds_no_path_text() {
    let (d, _, body) = recorded("nopath");
    assert!(!body.contains(d.to_str().unwrap()), "{body}");
    assert!(body.contains("param.target.cwd.sha256="), "{body}");
}

#[test]
fn a_new_bundle_lists_the_environment_fields_it_recorded() {
    let (_, _, body) = recorded("fieldlist");
    let line = body
        .lines()
        .find_map(|l| l.strip_prefix("param.env.fields="))
        .unwrap_or_else(|| panic!("{body}"));
    for name in ["hermit.path.sha256", "env.guest.passed", "host.container"] {
        assert!(line.split(',').any(|n| n == name), "{line}");
    }
}

/// The bundle as a newer build might write it: one more recorded field, named in the list of recorded fields.
fn from_newer_build(bundle: &str, field: &str) -> String {
    let listed: String = bundle
        .lines()
        .map(|l| match l.strip_prefix("param.env.fields=") {
            Some(names) => format!("param.env.fields={names},{field}\n"),
            None => format!("{l}\n"),
        })
        .collect();
    with_extra_param(&listed, &format!("param.{field}=on"))
}

#[test]
fn a_miss_on_a_bundle_with_a_field_this_build_does_not_know_exits_inconclusive_not_zero() {
    let (d, hermit, body) = recorded("newer-miss");
    let edited = from_newer_build(&passing_seed(&body), "hermit.future_flag");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(
        text(&o).contains("does not know (hermit.future_flag)"),
        "{}",
        text(&o)
    );
    assert!(text(&o).contains("upgrade Hairspring"), "{}", text(&o));
    assert!(text(&o).contains("inconclusive"), "{}", text(&o));
}

#[test]
fn a_reproduction_on_such_a_bundle_still_reproduces_and_still_warns() {
    let (d, hermit, body) = recorded("newer-hit");
    let file = write_bundle(
        &d,
        "b.hairspring",
        &from_newer_build(&body, "hermit.future_flag"),
    );
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("warning: environment differs"),
        "{}",
        text(&o)
    );
}

#[test]
fn an_unknown_field_marked_informational_does_not_stop_a_miss_exiting_zero() {
    let (d, hermit, body) = recorded("newer-info");
    let edited = from_newer_build(&passing_seed(&body), "~hermit.future_note").replace(
        "param.~hermit.future_note=on",
        "param.hermit.future_note=on",
    );
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
}

#[test]
fn an_unknown_field_without_a_list_is_still_caught_by_its_namespace() {
    let (d, hermit, body) = recorded("newer-nolist");
    let edited = with_extra_param(&passing_seed(&body), "param.host.future_fact=1");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
}

#[test]
fn a_hostile_unknown_name_is_never_echoed() {
    let (d, hermit, body) = recorded("newer-hostile");
    let edited = with_extra_param(&passing_seed(&body), "param.host.%0Aforged line=1");
    let file = write_bundle(&d, "b.hairspring", &edited);
    let o = run(&d, &hermit, &["replay", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(!text(&o).contains("forged line"), "{}", text(&o));
}

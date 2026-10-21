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

//! A trial that is killed at its timeout must not leave temporary files in the host temp directory.
//!
//! The fake `hermit` below makes what the real one makes in the host temporary directory (`$TMPDIR`, else `/tmp`): a
//! private guest `/tmp` when it is not given `--tmp`, a copy of a host file, an empty directory, and a strict-verify
//! log (in `$HERMIT_VERIFY_TMPDIR` when that is set). It removes them in a `trap`, which a `SIGKILL` skips. It starts
//! the guest as Hermit does: from an empty environment with `--base-env=minimal`, and otherwise from its own
//! environment without `HERMIT_VERIFY_TMPDIR`, which Hermit's default `host` base environment strips.
mod common;

use common::{scratch, write_fake};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings, Trial};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Whether the tests run as root, which ignores the permissions they use to make a removal fail.
fn is_root() -> bool {
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { geteuid() == 0 }
}

/// `@MARK@` is replaced with the marker directory. The fake records, one file each, the paths it made (`file`, `dir`,
/// `nscd`, `log`), the `--tmp` value (`tmp-arg`), and what it was started with (`tmpdir`, `verify-tmpdir`). The guest
/// command records what it can see (`guest-tmpdir`, `guest-verify-tmpdir`).
const FAKE: &str = r#"#!/bin/sh
MARK='@MARK@'
shift
tmp=
minimal=
while [ "$1" != "--" ]; do
  case "$1" in
    --tmp=*) tmp=${1#--tmp=};;
    --base-env=minimal) minimal=1;;
  esac
  shift
done
shift
base=${TMPDIR:-/tmp}
t=$(mktemp "$base/.tmpXXXXXX")
n=$(mktemp -d "$base/.tmpXXXXXX")
l=$(mktemp "${HERMIT_VERIFY_TMPDIR:-$base}/run1_log_XXXXXX")
made=
if [ -n "$tmp" ]; then d=$tmp; mkdir -p "$d"; else d=$(mktemp -d "$base/.tmpXXXXXX"); made=$d; fi
echo "$t" > "$MARK/file"
echo "$d" > "$MARK/dir"
echo "$n" > "$MARK/nscd"
echo "$l" > "$MARK/log"
echo "$tmp" > "$MARK/tmp-arg"
echo "${TMPDIR-unset}" > "$MARK/tmpdir"
echo "${HERMIT_VERIFY_TMPDIR-unset}" > "$MARK/verify-tmpdir"
if [ -e "$MARK/lockparent" ] && [ -n "${TMPDIR-}" ]; then chmod 500 "$(dirname "$TMPDIR")"; fi
echo guest-owned > "$d/guest-file"
echo hermit-owned > "$n/hermit-file"
trap 'rm -rf "$t" "$n" "$l" "$made"' EXIT TERM
if [ -n "$minimal" ]; then
  env -i MARK="$MARK" PATH=/usr/bin:/bin "$@"
else
  env -u HERMIT_VERIFY_TMPDIR MARK="$MARK" "$@"
fi
"#;

const RECORD: &str = "echo \"${TMPDIR-unset}\" > \"$MARK/guest-tmpdir\"; echo \"${HERMIT_VERIFY_TMPDIR-unset}\" > \"$MARK/guest-verify-tmpdir\";";
const SPIN: &str = "while :; do :; done";
const MINIMAL: &[&str] = &["--base-env=minimal"];
const HOST: &[&str] = &[];

struct Rig {
    markers: PathBuf,
    bin: PathBuf,
    /// The base directory for `summary_dir`.
    summaries: PathBuf,
    /// The base directory for the per-trial directories.
    trials: PathBuf,
    backend: HermitBackend,
    spec: TargetSpec,
}

struct Options {
    hermit_args: &'static [&'static str],
    events_file: bool,
    summaries: bool,
    /// Leave `trial_dir` at its default, the system temporary directory.
    default_trial_dir: bool,
}

impl Options {
    fn new(hermit_args: &'static [&'static str]) -> Options {
        Options {
            hermit_args,
            events_file: false,
            summaries: true,
            default_trial_dir: false,
        }
    }
}

fn rig(name: &str, guest: &str, o: Options) -> Rig {
    let dir = scratch(name);
    let markers = dir.join("markers");
    let summaries = dir.join("shm");
    let trials = dir.join("trials");
    for d in [&markers, &summaries, &trials] {
        std::fs::create_dir_all(d).unwrap();
    }
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), format!("{RECORD} {guest}")],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: o.hermit_args.iter().map(|a| a.to_string()).collect(),
            trial_timeout_secs: Some(1),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 1,
            wall_secs: 60,
        },
    };
    let bin = write_fake(
        &dir,
        &FAKE.replace("@MARK@", &markers.display().to_string()),
    );
    let mut settings = HermitSettings {
        bin: bin.clone(),
        trial_timeout: Duration::from_secs(20),
        summary_dir: o.summaries.then(|| summaries.clone()),
        events_file: o.events_file,
        ..HermitSettings::default()
    };
    if !o.default_trial_dir {
        settings.trial_dir = trials.clone();
    }
    Rig {
        markers,
        bin,
        summaries,
        trials,
        backend: HermitBackend::new(settings),
        spec,
    }
}

impl Rig {
    fn run(&self) -> Trial {
        self.backend.run_one(&self.spec, 1).unwrap()
    }

    fn marker(&self, name: &str) -> String {
        std::fs::read_to_string(self.markers.join(name))
            .unwrap_or_else(|e| panic!("marker {name}: {e}"))
            .trim()
            .to_string()
    }

    /// What is left in the engine's own per-trial base directory: nothing, once a trial has been cleaned up.
    fn engine_leftovers(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for private in std::fs::read_dir(&self.trials).unwrap().flatten() {
            found.extend(
                std::fs::read_dir(private.path())
                    .unwrap()
                    .flatten()
                    .map(|e| e.path()),
            );
        }
        found
    }
}

impl Drop for Rig {
    /// A failing test must not leave the files it is about to complain about.
    fn drop(&mut self) {
        for name in ["file", "dir", "nscd", "log"] {
            if let Ok(p) = std::fs::read_to_string(self.markers.join(name)) {
                let p = PathBuf::from(p.trim());
                let _ = std::fs::remove_dir_all(&p).or_else(|_| std::fs::remove_file(&p));
            }
        }
    }
}

fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

#[test]
fn a_timed_out_trial_under_the_minimal_base_environment_leaves_nothing() {
    let r = rig("minimal-timeout", SPIN, Options::new(MINIMAL));
    let t = r.run();
    assert!(t.timed_out, "{t:?}");
    for what in ["file", "dir", "nscd", "log"] {
        let path = r.marker(what);
        assert!(!exists(&path), "{what} {path} was left behind");
    }
    assert!(
        r.engine_leftovers().is_empty(),
        "{:?}",
        r.engine_leftovers()
    );
}

#[test]
fn a_timed_out_trial_under_the_host_base_environment_leaves_no_verify_log() {
    // Hermit's private guest `/tmp` and its copy of a host file cannot be redirected here without a `TMPDIR` that the
    // guest would inherit, so those two can still be left (documented); the run logs and the engine's directory cannot.
    let r = rig("host-timeout", SPIN, Options::new(HOST));
    let t = r.run();
    assert!(t.timed_out, "{t:?}");
    assert!(!exists(&r.marker("log")), "the verify log was left behind");
    assert!(
        r.engine_leftovers().is_empty(),
        "{:?}",
        r.engine_leftovers()
    );
}

#[test]
fn a_trial_that_exits_normally_leaves_nothing_behind() {
    for args in [MINIMAL, HOST] {
        let r = rig("control", "echo done", Options::new(args));
        let t = r.run();
        assert!(!t.timed_out && t.exit_code == Some(0), "{t:?}");
        for what in ["file", "dir", "nscd", "log"] {
            assert!(!exists(&r.marker(what)), "{what} {args:?}");
        }
        assert!(r.engine_leftovers().is_empty(), "{args:?}");
    }
}

#[test]
fn no_tmp_is_passed_unless_the_events_file_needs_it() {
    for args in [MINIMAL, HOST] {
        let r = rig("no-tmp", "echo done", Options::new(args));
        r.run();
        assert_eq!(r.marker("tmp-arg"), "", "{args:?}");
    }
}

#[test]
fn hermits_temporary_directory_is_set_only_where_it_cannot_reach_the_guest() {
    let r = rig("vars-minimal", "echo done", Options::new(MINIMAL));
    r.run();
    let (tmpdir, verify) = (r.marker("tmpdir"), r.marker("verify-tmpdir"));
    assert!(tmpdir.starts_with(r.trials.to_str().unwrap()), "{tmpdir}");
    assert_eq!(tmpdir, verify);

    let r = rig("vars-host", "echo done", Options::new(HOST));
    r.run();
    assert_eq!(r.marker("tmpdir"), "unset", "a TMPDIR would be inherited");
    assert!(r
        .marker("verify-tmpdir")
        .starts_with(r.trials.to_str().unwrap()));
}

#[test]
fn neither_variable_reaches_the_guest() {
    for args in [MINIMAL, HOST] {
        let r = rig("guest-env", "echo done", Options::new(args));
        r.run();
        assert_eq!(r.marker("guest-tmpdir"), "unset", "{args:?}");
        assert_eq!(r.marker("guest-verify-tmpdir"), "unset", "{args:?}");
    }
}

#[test]
fn trial_directories_default_to_slash_tmp_and_not_the_summary_directory() {
    let mut o = Options::new(MINIMAL);
    o.default_trial_dir = true;
    let r = rig("default-base", "echo done", o);
    r.run();
    let tmpdir = PathBuf::from(r.marker("tmpdir"));
    assert!(tmpdir.starts_with("/tmp"), "{tmpdir:?}");
    assert!(!tmpdir.starts_with(&r.summaries), "{tmpdir:?}");
    assert!(
        tmpdir.components().any(|c| c
            .as_os_str()
            .to_string_lossy()
            .starts_with("hairspring-trials-")),
        "{tmpdir:?}"
    );
    assert_eq!(HermitSettings::default().trial_dir, Path::new("/tmp"));
}

#[test]
fn trial_directories_do_not_need_a_summary_directory() {
    let mut o = Options::new(MINIMAL);
    o.summaries = false;
    let r = rig("no-summaries", SPIN, o);
    let t = r.run();
    assert!(t.timed_out, "{t:?}");
    assert!(!exists(&r.marker("log")));
    assert!(r.engine_leftovers().is_empty());
}

#[test]
fn a_trial_directory_that_cannot_be_made_fails_the_trial_without_starting_hermit() {
    let mut r = rig("cannot-make", "echo done", Options::new(MINIMAL));
    let blocker = r.trials.join("a-file");
    std::fs::write(&blocker, "").unwrap();
    let settings = HermitSettings {
        bin: r.bin.clone(),
        trial_dir: blocker,
        ..HermitSettings::default()
    };
    r.backend = HermitBackend::new(settings);
    let err = r.backend.run_one(&r.spec, 1).unwrap_err().to_string();
    assert!(err.contains("private directory"), "{err}");
    assert!(!r.markers.join("tmpdir").exists(), "hermit was started");
}

#[test]
fn the_events_file_still_uses_its_own_tmp_and_hermit_keeps_its_own_directory() {
    let mut o = Options::new(MINIMAL);
    o.events_file = true;
    let r = rig("events", "echo done", o);
    r.run();
    let (tmp_arg, tmpdir) = (r.marker("tmp-arg"), r.marker("tmpdir"));
    assert!(
        tmp_arg.starts_with(r.summaries.to_str().unwrap()),
        "{tmp_arg}"
    );
    assert!(tmpdir.starts_with(r.trials.to_str().unwrap()), "{tmpdir}");
    assert_ne!(tmp_arg, tmpdir);
    assert!(!exists(&tmp_arg) && r.engine_leftovers().is_empty());
}

/// The engine's per-run base directory for trial directories (`hairspring-trials-...` inside `trials`).
fn trial_base(r: &Rig) -> PathBuf {
    std::fs::read_dir(&r.trials)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .next()
        .expect("the backend made its base directory")
}

fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_failed_cleanup_of_the_trial_directory_does_not_change_the_verdict() {
    if is_root() {
        return;
    }
    for args in [MINIMAL, HOST] {
        let r = rig("cleanup-fails", "echo done", Options::new(args));
        std::fs::write(r.markers.join("lockparent"), "").unwrap();
        let t = r.run();
        // Put the permissions back so the test can clean up after itself.
        set_mode(&trial_base(&r), 0o700);
        assert_eq!(t.exit_code, Some(0), "{args:?} {t:?}");
        assert!(t.events_error.is_none(), "{args:?} {:?}", t.events_error);
        assert_eq!(t.stdout, "done\n", "no message in the guest's output");
    }
}

#[test]
fn a_trial_directory_that_cannot_be_made_for_one_trial_fails_it_without_starting_hermit() {
    if is_root() {
        return;
    }
    let r = rig("cannot-make-one", "echo done", Options::new(MINIMAL));
    let base = trial_base(&r);
    set_mode(&base, 0o500);
    let err = r.backend.run_one(&r.spec, 1);
    set_mode(&base, 0o700);
    let err = err.unwrap_err().to_string();
    assert!(err.contains("cannot start hermit"), "{err}");
    assert!(err.contains("private directory"), "{err}");
    assert!(!r.markers.join("tmpdir").exists(), "hermit was started");
}

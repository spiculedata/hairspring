//! A trial that is killed at its timeout must not leave temporary files in the host temp directory.
//!
//! The fake `hermit` below behaves as the real one does in the two respects that matter: the guest's `/tmp` is the
//! `--tmp` directory when one is given (otherwise a fresh directory in `$TMPDIR`), and a private copy of a host file
//! goes to `mktemp`, which honours `$TMPDIR`. It removes what it made in a `trap`, which a `SIGKILL` skips. With
//! `--base-env=minimal` it also starts the guest from an empty environment, as Hermit does; otherwise it passes its
//! own environment on, which is what Hermit's default `host` base environment does.
mod common;

use common::{scratch, write_fake};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings, Trial};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `@MARK@` is replaced with the marker directory. The fake records, one file each: the `mktemp` file (`file`), the
/// guest's `/tmp` (`dir`), the `--tmp` value (`tmp-arg`), the `$TMPDIR` it was started with (`tmpdir`), and what the
/// guest saw of `$TMPDIR` (`guest-tmpdir`, written by the guest command).
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
t=$(mktemp "${TMPDIR:-/tmp}/.tmpXXXXXX")
made=
if [ -n "$tmp" ]; then d=$tmp; mkdir -p "$d"; else d=$(mktemp -d "${TMPDIR:-/tmp}/.tmpXXXXXX"); made=$d; fi
echo "$t" > "$MARK/file"
echo "$d" > "$MARK/dir"
echo "$tmp" > "$MARK/tmp-arg"
echo "${TMPDIR-unset}" > "$MARK/tmpdir"
echo hermit-owned > "$t"
echo guest-owned > "$d/guest-file"
if [ -n "${TMPDIR-}" ]; then echo hermit-owned > "$TMPDIR/hermit-file"; fi
trap 'rm -rf "$t" "$made"' EXIT TERM
if [ -n "$minimal" ]; then
  MARK="$MARK" env -i MARK="$MARK" PATH=/usr/bin:/bin "$@"
else
  MARK="$MARK" "$@"
fi
"#;

/// The guest: records its `$TMPDIR`, then either spins (a hang) or returns.
const SPIN: &str = "echo \"${TMPDIR-unset}\" > \"$MARK/guest-tmpdir\"; while :; do :; done";
const RETURN: &str = "echo \"${TMPDIR-unset}\" > \"$MARK/guest-tmpdir\"; echo done";

struct Rig {
    markers: PathBuf,
    /// The base directory the engine keeps its private directories in (`summary_dir`).
    base: PathBuf,
    backend: HermitBackend,
    spec: TargetSpec,
}

fn rig(name: &str, guest: &str, hermit_args: &[&str], with_base: bool) -> Rig {
    let dir = scratch(name);
    let markers = dir.join("markers");
    let base = dir.join("shm");
    std::fs::create_dir_all(&markers).unwrap();
    std::fs::create_dir_all(&base).unwrap();
    let fake = FAKE.replace("@MARK@", &markers.display().to_string());
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), guest.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: hermit_args.iter().map(|a| a.to_string()).collect(),
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
    let settings = HermitSettings {
        bin: write_fake(&dir, &fake),
        trial_timeout: Duration::from_secs(20),
        summary_dir: with_base.then(|| base.clone()),
        ..HermitSettings::default()
    };
    Rig {
        markers,
        base,
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

    /// What is left inside the engine's private base directory: nothing, once a trial has been cleaned up.
    fn engine_leftovers(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for private in std::fs::read_dir(&self.base).unwrap().flatten() {
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
        for name in ["file", "dir"] {
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
fn a_timed_out_trial_leaves_no_guest_tmp_directory_behind() {
    let r = rig("dir", SPIN, &[], true);
    let t = r.run();
    assert!(t.timed_out, "{t:?}");
    let dir = r.marker("dir");
    assert!(!exists(&dir), "the guest's /tmp {dir} was left behind");
    assert!(
        r.engine_leftovers().is_empty(),
        "{:?}",
        r.engine_leftovers()
    );
}

#[test]
fn the_guest_tmp_is_always_an_engine_owned_directory() {
    let r = rig("always-tmp", RETURN, &[], true);
    let t = r.run();
    assert!(!t.timed_out, "{t:?}");
    let tmp_arg = r.marker("tmp-arg");
    assert!(
        tmp_arg.starts_with(r.base.to_str().unwrap()),
        "hermit was given --tmp={tmp_arg:?}, not a directory below {}",
        r.base.display()
    );
}

#[test]
fn a_timed_out_trial_under_the_minimal_base_environment_leaves_nothing() {
    let r = rig("minimal-timeout", SPIN, &["--base-env=minimal"], true);
    let t = r.run();
    assert!(t.timed_out, "{t:?}");
    let (file, dir) = (r.marker("file"), r.marker("dir"));
    assert!(
        !exists(&file),
        "hermit's temporary file {file} was left behind"
    );
    assert!(!exists(&dir), "the guest's /tmp {dir} was left behind");
    assert!(
        r.engine_leftovers().is_empty(),
        "{:?}",
        r.engine_leftovers()
    );
}

#[test]
fn hermits_own_temporary_files_do_not_land_in_the_guests_tmp() {
    let r = rig("separate", RETURN, &["--base-env=minimal"], true);
    r.run();
    let (file, tmp_arg, tmpdir) = (r.marker("file"), r.marker("tmp-arg"), r.marker("tmpdir"));
    assert_ne!(tmpdir, "unset", "hermit was given no TMPDIR");
    assert_ne!(
        tmpdir, tmp_arg,
        "TMPDIR and the guest's /tmp are one directory"
    );
    assert!(
        !file.starts_with(&tmp_arg) && !tmpdir.starts_with(&format!("{tmp_arg}/")),
        "hermit's own file {file} is inside the guest's /tmp {tmp_arg}"
    );
}

#[test]
fn a_trial_that_exits_normally_leaves_nothing_behind() {
    for args in [&[][..], &["--base-env=minimal"][..]] {
        let r = rig("control", RETURN, args, true);
        let t = r.run();
        assert!(!t.timed_out && t.exit_code == Some(0), "{t:?}");
        assert!(!exists(&r.marker("dir")), "{args:?}");
        assert!(!exists(&r.marker("file")), "{args:?}");
        assert!(r.engine_leftovers().is_empty(), "{args:?}");
    }
}

#[test]
fn tmpdir_does_not_reach_the_guest_under_the_minimal_base_environment() {
    let r = rig("guest-env-minimal", RETURN, &["--base-env=minimal"], true);
    r.run();
    assert_ne!(
        r.marker("tmpdir"),
        "unset",
        "the test needs hermit to have a TMPDIR"
    );
    assert_eq!(r.marker("guest-tmpdir"), "unset");
}

#[test]
fn tmpdir_is_not_set_where_hermit_would_pass_it_to_the_guest() {
    // Hermit's default base environment passes its own environment through, so a TMPDIR here would be the guest's.
    for args in [
        &[][..],
        &["--base-env=host"][..],
        &["--base-env", "host"][..],
    ] {
        let r = rig("guest-env-host", RETURN, args, true);
        r.run();
        assert_eq!(r.marker("tmpdir"), "unset", "{args:?}");
        assert_eq!(r.marker("guest-tmpdir"), "unset", "{args:?}");
    }
}

#[test]
fn tmpdir_is_not_set_when_the_target_passes_it_through_to_the_guest() {
    for args in [
        &["--base-env=minimal", "--env=TMPDIR"][..],
        &["--base-env=minimal", "--env", "TMPDIR"][..],
    ] {
        let r = rig("passthrough", RETURN, args, true);
        r.run();
        assert_eq!(r.marker("tmpdir"), "unset", "{args:?}");
    }
}

#[test]
fn a_target_that_chooses_the_guests_tmp_itself_is_left_alone() {
    let r = rig("own-tmp", RETURN, &["--bind=/somewhere:/tmp/data"], true);
    r.run();
    assert_eq!(
        r.marker("tmp-arg"),
        "",
        "the engine passed --tmp over the target's own /tmp mount"
    );
}

#[test]
fn without_a_private_directory_the_trial_runs_as_before() {
    let r = rig("no-base", RETURN, &["--base-env=minimal"], false);
    let t = r.run();
    assert_eq!(t.exit_code, Some(0), "{t:?}");
    assert_eq!(r.marker("tmp-arg"), "");
    assert_eq!(r.marker("tmpdir"), "unset");
    let _ = std::fs::remove_file(r.marker("file"));
    let _ = std::fs::remove_dir_all(r.marker("dir"));
}

//! What the guest sees under each `--base-env`, with a fake Hermit that builds the environment as the pinned source
//! does (`common::minimal_fake`).
//!
//! Under `--base-env=minimal` and `empty` Hermit gives the guest nothing of its own environment, so Hairspring names
//! each variable with `--env=NAME` (names only; Hermit copies the values). `pass_env = false` is the way back.
mod common;

use common::minimal_fake::FAKE;
use common::{scratch, write_fake};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings, Trial};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

/// Prints the guest's whole environment. `/usr/bin/env` is named in full: under `empty` there is no PATH.
const ENV_DUMP: &str = "/usr/bin/env";
const EPOCH: &str = "2026-01-01T00:00:00Z";
const HERMIT_MINIMAL_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const PINNED_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
/// Variables the engine gives every guest, and the two Hermit forces in every mode.
const ENGINE_NAMES: [&str; 6] = [
    "HAIRSPRING_PROC",
    "HAIRSPRING_SEED",
    "LANG",
    "LC_ALL",
    "PATH",
    "TZ",
];
const FORCED_BY_HERMIT: [&str; 2] = ["ASAN_OPTIONS", "LSAN_OPTIONS"];

struct Rig {
    dir: PathBuf,
    backend: HermitBackend,
    spec: TargetSpec,
}

#[derive(Clone)]
struct Setup {
    hermit_args: Vec<String>,
    env: Vec<(String, String)>,
    pass_env: bool,
    events_file: bool,
    script: String,
}

impl Setup {
    fn new(hermit_args: &[&str]) -> Setup {
        Setup {
            hermit_args: hermit_args.iter().map(|a| a.to_string()).collect(),
            env: Vec::new(),
            pass_env: true,
            events_file: false,
            script: ENV_DUMP.to_string(),
        }
    }

    fn env(mut self, name: &str, value: &str) -> Setup {
        self.env.push((name.to_string(), value.to_string()));
        self
    }

    fn pass_env(self, pass_env: bool) -> Setup {
        Setup { pass_env, ..self }
    }

    fn script(self, script: &str) -> Setup {
        Setup {
            script: script.to_string(),
            ..self
        }
    }

    fn events_file(self) -> Setup {
        Setup {
            events_file: true,
            ..self
        }
    }

    fn rig(self, name: &str) -> Rig {
        let dir = scratch(name);
        let shm = dir.join("shm");
        std::fs::create_dir_all(&shm).unwrap();
        let report = dir.join("report.events");
        std::fs::write(&report, common::events::REPORT).unwrap();
        let script = self
            .script
            .replace("$REPORT", &report.display().to_string());
        let spec = TargetSpec {
            name: "svc".into(),
            description: String::new(),
            kind: TargetKind::Process {
                command: vec!["/bin/sh".into(), "-c".into(), script],
                env: self.env.into_iter().collect(),
                cwd: None,
                hermit_args: self.hermit_args,
                trial_timeout_secs: Some(20),
                scheduler: SchedulerSpec::default(),
                hang_is_violation: false,
                parallelism: 1,
                pass_env: self.pass_env,
            },
            budget: Budget {
                max_steps: 1,
                max_trials: 1,
                wall_secs: 60,
            },
        };
        let settings = HermitSettings {
            bin: write_fake(&dir, FAKE),
            trial_timeout: Duration::from_secs(20),
            summary_dir: self.events_file.then(|| shm.clone()),
            events_file: self.events_file,
            // Only Hermit's own process gets these; a guest must never see them.
            extra_env: vec![
                ("MF_LOG".to_string(), dir.display().to_string()),
                ("OPERATOR_ONLY".to_string(), "1".to_string()),
            ],
            ..HermitSettings::default()
        };
        Rig {
            dir,
            backend: HermitBackend::new(settings),
            spec,
        }
    }
}

impl Rig {
    fn run(&self, seed: u64) -> Trial {
        self.backend.run_one(&self.spec, seed).unwrap()
    }

    fn logged(&self, what: &str, seed: u64) -> String {
        std::fs::read_to_string(self.dir.join(format!("{what}-{seed}")))
            .unwrap()
            .trim_end_matches('\n')
            .to_string()
    }

    fn argv(&self, seed: u64) -> Vec<String> {
        self.logged("argv", seed)
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The guest's environment, read from `env`'s output. Only lines that start a `NAME=` are taken.
    fn guest_env(&self, seed: u64) -> BTreeMap<String, String> {
        let trial = self.run(seed);
        assert_eq!(trial.exit_code, Some(0), "{}", trial.stderr);
        parse_env(&trial.stdout)
    }
}

fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let ok = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit());
            // The shell that runs the guest's script adds these three itself.
            let by_the_shell = matches!(name, "PWD" | "SHLVL" | "_" | "OLDPWD");
            (ok && !by_the_shell).then(|| (name.to_string(), value.to_string()))
        })
        .collect()
}

fn names(env: &BTreeMap<String, String>) -> BTreeSet<&str> {
    env.keys().map(String::as_str).collect()
}

fn set<'a>(list: &[&'a str]) -> BTreeSet<&'a str> {
    list.iter().copied().collect()
}

const MINIMAL: &[&str] = &["--base-env=minimal"];
const EMPTY: &[&str] = &["--base-env=empty"];

#[test]
fn under_minimal_the_guest_gets_the_engines_variables_and_hermits_own() {
    let rig = Setup::new(MINIMAL).rig("pt-minimal");
    let env = rig.guest_env(7);
    let expected: Vec<&str> = [
        &ENGINE_NAMES[..],
        &FORCED_BY_HERMIT[..],
        &["HOSTNAME", "HOME"],
    ]
    .concat();
    assert_eq!(names(&env), set(&expected), "{env:?}");
    assert_eq!(env["HAIRSPRING_SEED"], "7");
    assert_eq!(env["HAIRSPRING_PROC"], "p1");
    assert_eq!(
        env["PATH"], PINNED_PATH,
        "the engine's PATH replaces Hermit's"
    );
    assert_eq!(
        (
            env["LANG"].as_str(),
            env["LC_ALL"].as_str(),
            env["TZ"].as_str()
        ),
        ("C", "C", "UTC")
    );
    assert_eq!(env["HOSTNAME"], "hermetic-container.local");
    assert_eq!(env["HOME"], "/root");
}

#[test]
fn under_empty_the_guest_gets_only_the_engines_variables() {
    let rig = Setup::new(EMPTY).rig("pt-empty");
    let env = rig.guest_env(3);
    let expected: Vec<&str> = [&ENGINE_NAMES[..], &FORCED_BY_HERMIT[..]].concat();
    assert_eq!(names(&env), set(&expected), "{env:?}");
    assert_eq!(env["HAIRSPRING_SEED"], "3");
}

#[test]
fn an_upper_case_flag_value_still_counts() {
    let rig = Setup::new(&["--base-env=MINIMAL"]).rig("pt-case");
    assert_eq!(rig.guest_env(1)["HAIRSPRING_PROC"], "p1");
}

#[test]
fn nothing_the_engine_keeps_for_hermit_reaches_the_guest() {
    let rig = Setup::new(MINIMAL).rig("pt-private");
    let env = rig.guest_env(1);
    for private in [
        "TMPDIR",
        "HERMIT_VERIFY_TMPDIR",
        "HAIRSPRING_PROC_PID",
        "MF_LOG",
        "OPERATOR_ONLY",
    ] {
        assert!(!env.contains_key(private), "{private}: {env:?}");
    }
    let argv = rig.argv(1).join(" ");
    assert!(
        !argv.contains("OPERATOR_ONLY") && !argv.contains("TMPDIR"),
        "{argv}"
    );
    assert_ne!(
        rig.logged("tmpdir", 1),
        "unset",
        "Hermit itself is still given the per-trial directory"
    );
}

#[test]
fn target_values_arrive_whole_including_empty_equals_newline_and_non_ascii() {
    let script = "printf '[%s][%s][%s][%s][%s]' \"$EMPTY\" \"$EQ\" \"$NL\" \"$UNI\" \"$LANG\"";
    let rig = Setup::new(MINIMAL)
        .env("EMPTY", "")
        .env("EQ", "a=b=c")
        .env("NL", "line one\nline two")
        .env("UNI", "h\u{e9}llo \u{2713}")
        .env("LANG", "en_GB.UTF-8")
        .script(script)
        .rig("pt-values");
    let trial = rig.run(1);
    assert_eq!(trial.exit_code, Some(0), "{}", trial.stderr);
    assert_eq!(
        trial.stdout,
        "[][a=b=c][line one\nline two][h\u{e9}llo \u{2713}][en_GB.UTF-8]"
    );
}

#[test]
fn a_target_that_sets_tmpdir_itself_gets_that_value_and_no_other() {
    let rig = Setup::new(MINIMAL)
        .env("TMPDIR", "/fixed/tmp")
        .rig("pt-tmpdir");
    let env = rig.guest_env(1);
    assert_eq!(env["TMPDIR"], "/fixed/tmp");
    assert_eq!(
        rig.logged("tmpdir", 1),
        "/fixed/tmp",
        "the engine set none of its own"
    );
}

#[test]
fn a_name_the_target_passes_itself_keeps_its_value_and_is_not_added() {
    let rig = Setup::new(&[
        "--base-env=minimal",
        "--env=LANG=en_US",
        "-e",
        "GOTRACEBACK=all",
    ])
    .env("LANG", "fr_FR")
    .env("GOTRACEBACK", "none")
    .rig("pt-own");
    let env = rig.guest_env(1);
    assert_eq!(env["LANG"], "en_US");
    assert_eq!(env["GOTRACEBACK"], "all");
    let argv = rig.argv(1);
    assert!(!argv.contains(&"--env=LANG".to_string()), "{argv:?}");
    assert!(!argv.contains(&"--env=GOTRACEBACK".to_string()), "{argv:?}");
    assert!(argv.contains(&"--env=LANG=en_US".to_string()), "{argv:?}");
}

#[test]
fn the_names_sit_just_before_the_targets_hermit_args_and_carry_no_values() {
    let rig = Setup::new(MINIMAL)
        .env("SECRET_TOKEN", "hunter2")
        .rig("pt-argv");
    let _ = rig.run(5);
    let argv = rig.argv(5);
    let at = |needle: &str| argv.iter().position(|a| a == needle).unwrap();
    let names: Vec<&String> = argv.iter().filter(|a| a.starts_with("--env=")).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "byte order");
    assert!(names.contains(&&"--env=SECRET_TOKEN".to_string()));
    assert!(at("--env=TZ") < at("--base-env=minimal"));
    assert_eq!(at("--base-env=minimal") + 1, at("--"));
    assert!(at("--sched-seed=5") < at("--env=HAIRSPRING_PROC"));
    assert!(argv.iter().all(|a| !a.contains("hunter2")), "{argv:?}");
    assert!(argv
        .iter()
        .filter(|a| a.starts_with("--env="))
        .all(|a| !a.contains('=') || a.matches('=').count() == 1));
}

#[test]
fn a_guest_started_with_a_hostile_rig_still_sees_exactly_one_root_tag_and_no_pid() {
    let mut rig = Setup::new(MINIMAL).rig("pt-hostile");
    if let TargetKind::Process { env, .. } = &mut rig.spec.kind {
        env.insert("HAIRSPRING_PROC".into(), "configured".into());
        env.insert("HAIRSPRING_PROC_PID".into(), "7".into());
    }
    let backend = {
        let mut s = HermitSettings {
            bin: write_fake(&rig.dir, FAKE),
            ..HermitSettings::default()
        };
        s.summary_dir = None;
        s.extra_env = vec![
            ("MF_LOG".into(), rig.dir.display().to_string()),
            ("HAIRSPRING_PROC".into(), "evil".into()),
            ("HAIRSPRING_PROC_PID".into(), "4242".into()),
        ];
        HermitBackend::new(s)
    };
    let rig = Rig { backend, ..rig };
    let env = rig.guest_env(1);
    let procs: Vec<_> = env
        .iter()
        .filter(|(k, _)| k.starts_with("HAIRSPRING_PROC"))
        .collect();
    assert_eq!(
        procs,
        [(&"HAIRSPRING_PROC".to_string(), &"p1".to_string())],
        "{env:?}"
    );
}

#[test]
fn the_guest_environment_is_identical_across_seeds_and_runs_but_for_the_seed() {
    let rig = Setup::new(MINIMAL).env("A", "1").rig("pt-same");
    let strip = |mut env: BTreeMap<String, String>| {
        env.remove("HAIRSPRING_SEED");
        env
    };
    let first = strip(rig.guest_env(1));
    assert_eq!(first, strip(rig.guest_env(2)));
    assert_eq!(first, strip(rig.guest_env(987_654_321)));
    assert_eq!(first, strip(rig.guest_env(1)));
}

#[test]
fn with_pass_env_off_minimal_gives_exactly_what_hermit_gave_before() {
    let rig = Setup::new(MINIMAL)
        .env("A", "1")
        .pass_env(false)
        .rig("pt-off");
    let env = rig.guest_env(7);
    assert_eq!(
        names(&env),
        set(&["HOSTNAME", "PATH", "HOME", "ASAN_OPTIONS", "LSAN_OPTIONS"]),
        "{env:?}"
    );
    assert_eq!(env["PATH"], HERMIT_MINIMAL_PATH);
}

#[test]
fn with_pass_env_off_empty_gives_the_guest_nothing_of_the_engines() {
    let rig = Setup::new(EMPTY).pass_env(false).rig("pt-off-empty");
    let env = rig.guest_env(7);
    assert_eq!(
        names(&env),
        set(&["ASAN_OPTIONS", "LSAN_OPTIONS"]),
        "{env:?}"
    );
}

/// The command line the backend built before `pass_env` existed, for `--base-env` mode `args`, written out.
fn old_argv(args: &[&str], seed: u64, script: &str) -> Vec<String> {
    let mut argv = vec![
        "run".to_string(),
        format!("--epoch={EPOCH}"),
        "--sched-heuristic=random".to_string(),
        format!("--sched-seed={seed}"),
    ];
    argv.extend(args.iter().map(|a| a.to_string()));
    argv.extend(["--", "/bin/sh", "-c", script].map(String::from));
    argv
}

#[test]
fn with_pass_env_off_the_command_line_is_the_one_before_the_setting_existed_in_every_mode() {
    for (i, args) in [
        &[][..],
        &["--base-env=host"][..],
        &["--base-env=minimal"][..],
        &["--base-env=empty"][..],
        &["--base-env=Minimal"][..],
        &["--base-env", "empty"][..],
        &["--base-env=surprise"][..],
        &["--base-env=minimal", "--env=A=1"][..],
        &["--base-env=minimal", "--config=f.yaml"][..],
        &["--image=x"][..],
    ]
    .into_iter()
    .enumerate()
    {
        let rig = Setup::new(args)
            .pass_env(false)
            .rig(&format!("pt-golden-{i}"));
        let _ = rig.run(9);
        assert_eq!(rig.argv(9), old_argv(args, 9, ENV_DUMP), "{args:?}");
    }
}

#[test]
fn where_nothing_is_added_the_command_line_is_the_same_with_the_setting_on() {
    for (i, args) in [
        &[][..],
        &["--base-env=host"][..],
        &["--base-env=surprise"][..],
        &["--base-env=minimal", "--config=f.yaml"][..],
        &["--base-env=minimal", "--from", "m.yaml"][..],
        &["--image=x"][..],
    ]
    .into_iter()
    .enumerate()
    {
        let rig = Setup::new(args).rig(&format!("pt-same-{i}"));
        let _ = rig.run(9);
        assert_eq!(rig.argv(9), old_argv(args, 9, ENV_DUMP), "{args:?}");
    }
}

#[test]
fn the_host_base_environment_gives_the_same_guest_with_the_setting_on_or_off() {
    for args in [&[][..], &["--base-env=host"][..]] {
        let on = Setup::new(args)
            .env("A", "1")
            .rig("pt-host-on")
            .guest_env(4);
        let off = Setup::new(args)
            .env("A", "1")
            .pass_env(false)
            .rig("pt-host-off")
            .guest_env(4);
        assert_eq!(on, off, "{args:?}");
        assert_eq!(on["HAIRSPRING_SEED"], "4");
        assert_eq!(on["A"], "1");
    }
}

#[test]
fn the_events_file_works_under_minimal_and_empty_only_when_the_variable_is_passed() {
    let script = "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"";
    for (i, args) in [MINIMAL, EMPTY].into_iter().enumerate() {
        let on = Setup::new(args)
            .events_file()
            .script(script)
            .rig(&format!("pt-events-on-{i}"))
            .run(1);
        assert_eq!(on.exit_code, Some(0), "{args:?}: {}", on.stderr);
        assert!(
            on.events.is_some(),
            "{args:?}: events arrive through the file"
        );
        assert!(on.events_error.is_none(), "{args:?}: {:?}", on.events_error);

        let off = Setup::new(args)
            .events_file()
            .pass_env(false)
            .script(script)
            .rig(&format!("pt-events-off-{i}"))
            .run(1);
        assert!(
            off.events.is_none(),
            "{args:?}: nothing reaches the file without the variable"
        );
    }
}

#[test]
fn the_events_variable_is_named_on_the_command_line_only_when_the_events_file_is_on() {
    let on = Setup::new(MINIMAL)
        .events_file()
        .script("true")
        .rig("pt-ev-argv-on");
    let _ = on.run(2);
    assert!(on
        .argv(2)
        .contains(&"--env=HAIRSPRING_EVENTS_FILE".to_string()));
    let off = Setup::new(MINIMAL).script("true").rig("pt-ev-argv-off");
    let _ = off.run(2);
    assert!(!off
        .argv(2)
        .contains(&"--env=HAIRSPRING_EVENTS_FILE".to_string()));
}

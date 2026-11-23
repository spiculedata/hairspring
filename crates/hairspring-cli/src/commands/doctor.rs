use crate::cli::DoctorArgs;
use crate::ctx::{CmdResult, Ctx};
use crate::host::{self, HostFacts};
use hairspring_engine::config::TargetKind;
use hairspring_engine::probe::ProbeReport;
use std::io::Write;

pub fn run(ctx: &Ctx, args: &DoctorArgs, out: &mut dyn Write) -> CmdResult<i32> {
    run_with_host(ctx, args, out, &HostFacts::probe())
}

/// `run`, with the host's facts given instead of read, so the host warnings can be tested.
fn run_with_host(
    ctx: &Ctx,
    args: &DoctorArgs,
    out: &mut dyn Write,
    facts: &HostFacts,
) -> CmdResult<i32> {
    // The flags answer for the machine only; no target is looked at.
    if args.process_backend {
        return run_backend_probe(ctx, HERMIT_BACKEND, out);
    }
    if let Some(name) = &args.backend {
        return run_backend_probe(ctx, name, out);
    }
    // First, so a per-target error below cannot hide it.
    if host::involves_process_target(ctx.engine.config(), &args.targets) {
        host::write_report(out, facts)?;
    }
    let names: Vec<String> = if args.targets.is_empty() {
        ctx.engine
            .config()
            .targets
            .values()
            .filter(|t| matches!(t.kind, TargetKind::Sim { .. }))
            .map(|t| t.name.clone())
            .collect()
    } else {
        args.targets.clone()
    };
    let mut code = crate::EXIT_OK;
    for name in names {
        let report = ctx.engine.doctor(&name, args.seeds, args.steps)?;
        match report.divergence {
            None => writeln!(
                out,
                "{}",
                deterministic_line(&name, report.seeds_checked, report.steps_per_seed)
            )?,
            Some(d) => {
                code = crate::EXIT_VIOLATION;
                writeln!(out, "{name}: NON-DETERMINISTIC (seed {}, diverged at step {}). Look for HashMap iteration, time, threads or randomness outside the simulation.", d.seed, d.step)?;
            }
        }
    }
    Ok(code)
}

/// The line for a target that behaved the same on every seed. Wasm and process targets have no step count (their
/// doctor reruns whole trials), so the steps are named only when there are some.
fn deterministic_line(name: &str, seeds: u32, steps: usize) -> String {
    let seeds = match seeds {
        1 => "1 seed".to_string(),
        n => format!("{n} seeds"),
    };
    match steps {
        0 => format!("{name}: deterministic over {seeds}"),
        n => format!("{name}: deterministic over {seeds} x {n} steps"),
    }
}

/// The id of the backend `--process-backend` asks about.
const HERMIT_BACKEND: &str = "hermit";

/// Runs the named backend's probe and prints it; exits `EXIT_UNSUPPORTED_HOST` when any prerequisite is unmet. A name no
/// backend answers to is an error that lists the names there are.
fn run_backend_probe(ctx: &Ctx, name: &str, out: &mut dyn Write) -> CmdResult<i32> {
    let report = ctx.engine.probe_backend(name)?;
    write_probe(out, &report)
}

/// Prints one probe report and gives the exit code it calls for.
fn write_probe(out: &mut dyn Write, report: &ProbeReport) -> CmdResult<i32> {
    report.write(out)?;
    Ok(match report.is_ready() {
        true => crate::EXIT_OK,
        false => crate::EXIT_UNSUPPORTED_HOST,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AutoUpdates, LoaderHashes};
    use hairspring_engine::erased::Registry;
    use hairspring_hermit::host::process_backend::{
        assess, probe_report, HermitFact, PerfFact, ProcessBackendFacts,
    };

    static NEXT_DIR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn ctx_with(config: &str) -> Ctx {
        let n = NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hairspring-doctor-{}-{}-{n}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hairspring.toml");
        std::fs::write(&path, config).unwrap();
        Ctx::load(Registry::new(), &path, dir.join("db.sqlite")).unwrap()
    }

    fn kernel(release: &str) -> HostFacts {
        HostFacts {
            kernel_release: Some(release.into()),
            ..HostFacts::default()
        }
    }

    #[test]
    fn enabled_updates_and_loader_hashes_show_up_in_the_report_without_failing_it() {
        let facts = HostFacts {
            auto_updates: AutoUpdates::Enabled,
            loader: LoaderHashes {
                cache: Some("c0ffee".into()),
                ..LoaderHashes::default()
            },
            ..kernel("6.17.0-42-generic")
        };
        let mut out = Vec::new();
        let code = run_with_host(&ctx_with(PROCESS), &args(), &mut out, &facts).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, crate::EXIT_OK);
        assert!(text.contains("ld.so.cache sha256 c0ffee"), "{text}");
        assert!(
            text.contains("warning: automatic package updates"),
            "{text}"
        );
    }

    #[test]
    fn disabled_missing_or_unreadable_update_state_adds_no_warning() {
        for state in [
            AutoUpdates::Disabled,
            AutoUpdates::NotPresent,
            AutoUpdates::Unreadable,
            AutoUpdates::Unknown,
        ] {
            let facts = HostFacts {
                auto_updates: state,
                ..kernel("6.17.0-42-generic")
            };
            let mut out = Vec::new();
            run_with_host(&ctx_with(PROCESS), &args(), &mut out, &facts).unwrap();
            assert!(out.is_empty(), "{state:?}");
        }
    }

    fn args() -> DoctorArgs {
        DoctorArgs {
            targets: vec![],
            seeds: 1,
            steps: 1,
            process_backend: false,
            backend: None,
        }
    }

    fn probe_for(facts: &ProcessBackendFacts) -> ProbeReport {
        probe_report(&assess(facts))
    }

    fn backend_facts(os: &str) -> ProcessBackendFacts {
        ProcessBackendFacts {
            os: os.into(),
            arch: "x86_64".into(),
            kernel_release: Some("6.17.0-42-generic".into()),
            hermit: HermitFact::Found {
                path: "/h/hermit".into(),
                version: "hermit 0.4.1".into(),
                sha256: Some("ab".into()),
            },
            perf: PerfFact::Usable,
            perf_event_paranoid: Some(2),
            pmu_registered: Some(true),
            ptrace_scope: Some(1),
            is_root: false,
            container: None,
            libc: None,
            loader: Default::default(),
        }
    }

    const GOLDEN_OK: &str = include_str!("../../tests/golden/doctor_process_backend_ok.txt");
    const GOLDEN_BLOCKED: &str =
        include_str!("../../tests/golden/doctor_process_backend_blocked.txt");

    fn blocked_facts() -> ProcessBackendFacts {
        ProcessBackendFacts {
            os: "macos".into(),
            arch: "aarch64".into(),
            kernel_release: Some("6.17.0-42-generic".into()),
            hermit: HermitFact::Missing { installed_at: None },
            libc: Some("ldd (Ubuntu GLIBC 2.39-0ubuntu8.3) 2.39".into()),
            loader: LoaderHashes {
                cache: Some("c0ffee".into()),
                libc: Some("1bc".into()),
                ld_so: Some("d1d".into()),
            },
            ..backend_facts("macos")
        }
    }

    fn ready_facts() -> ProcessBackendFacts {
        ProcessBackendFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            hermit: HermitFact::Found {
                path: "/opt/hermit/bin/hermit".into(),
                version: "hermit 0.4.1 (2026-10-15)".into(),
                sha256: Some(
                    "666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a".into(),
                ),
            },
            ..blocked_facts()
        }
    }

    #[test]
    fn the_process_backend_report_is_byte_identical_to_the_recorded_output() {
        let mut out = Vec::new();
        let code = write_probe(&mut out, &probe_for(&ready_facts())).unwrap();
        assert_eq!(code, crate::EXIT_OK);
        assert_eq!(String::from_utf8(out).unwrap(), GOLDEN_OK);
        let mut out = Vec::new();
        let code = write_probe(&mut out, &probe_for(&blocked_facts())).unwrap();
        assert_eq!(code, crate::EXIT_UNSUPPORTED_HOST);
        assert_eq!(String::from_utf8(out).unwrap(), GOLDEN_BLOCKED);
    }

    #[test]
    fn the_deterministic_line_names_steps_only_when_there_are_some() {
        assert_eq!(
            deterministic_line("lock-lease", 8, 200),
            "lock-lease: deterministic over 8 seeds x 200 steps"
        );
        assert_eq!(
            deterministic_line("wasm-demo", 8, 0),
            "wasm-demo: deterministic over 8 seeds"
        );
        assert_eq!(
            deterministic_line("svc", 1, 0),
            "svc: deterministic over 1 seed"
        );
    }

    #[test]
    fn the_process_backend_flag_prints_the_report_and_exits_zero_when_it_can_run() {
        let mut out = Vec::new();
        let code = write_probe(&mut out, &probe_for(&backend_facts("linux"))).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, crate::EXIT_OK, "{text}");
        assert!(text.contains("result: ok:"), "{text}");
    }

    #[test]
    fn a_machine_that_cannot_run_the_process_backend_gets_its_own_exit_code() {
        let mut out = Vec::new();
        let code = write_probe(&mut out, &probe_for(&backend_facts("macos"))).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, crate::EXIT_UNSUPPORTED_HOST, "{text}");
        assert_ne!(
            code,
            crate::EXIT_VIOLATION,
            "must not look like a found bug"
        );
        assert!(text.contains("result: CANNOT RUN"), "{text}");
    }

    fn backend_args(name: &str) -> DoctorArgs {
        DoctorArgs {
            backend: Some(name.to_string()),
            ..args()
        }
    }

    fn run_named(name: &str) -> (CmdResult<i32>, String) {
        let mut out = Vec::new();
        let result = run_with_host(
            &ctx_with(PROCESS),
            &backend_args(name),
            &mut out,
            &HostFacts::default(),
        );
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_backend_with_nothing_to_check_is_reported_ready() {
        let (code, text) = run_named("sim");
        assert_eq!(code.unwrap(), crate::EXIT_OK, "{text}");
        assert!(
            text.starts_with("Simulation: prerequisites on this machine\n"),
            "{text}"
        );
        assert!(text.contains("result: ok:"), "{text}");
    }

    #[test]
    fn an_unknown_backend_is_an_error_that_lists_the_available_names() {
        let (result, text) = run_named("qemu");
        let message = result.unwrap_err().to_string();
        assert!(message.contains("unknown backend `qemu`"), "{message}");
        assert!(
            message.contains("available: sim, hermit, wasm"),
            "{message}"
        );
        assert!(text.is_empty(), "nothing is printed to stdout: {text}");
    }

    #[test]
    fn backend_hermit_prints_exactly_what_the_process_backend_flag_prints() {
        let (by_name_code, by_name) = run_named("hermit");
        let mut flagged = args();
        flagged.process_backend = true;
        let mut out = Vec::new();
        let by_flag_code = run_with_host(
            &ctx_with(PROCESS),
            &flagged,
            &mut out,
            &HostFacts::default(),
        );
        assert_eq!(by_name_code.unwrap(), by_flag_code.unwrap());
        assert_eq!(by_name, String::from_utf8(out).unwrap());
        assert!(by_name.starts_with("process backend: prerequisites on this machine\n"));
    }

    #[test]
    fn the_backend_flag_skips_the_target_checks_and_the_host_report() {
        let mut named = backend_args("sim");
        named.targets = vec!["no-such-target".to_string()];
        let mut out = Vec::new();
        let facts = kernel("6.8.0-146-generic");
        let result = run_with_host(&ctx_with(PROCESS), &named, &mut out, &facts);
        let text = String::from_utf8(out).unwrap();
        assert!(result.is_ok());
        assert!(
            !text.contains("no-such-target") && !text.contains("warning:"),
            "{text}"
        );
    }

    #[test]
    fn the_process_backend_flag_skips_the_target_checks() {
        let mut flagged = args();
        flagged.process_backend = true;
        flagged.targets = vec!["no-such-target".to_string()];
        let mut out = Vec::new();
        let result = run_with_host(
            &ctx_with(PROCESS),
            &flagged,
            &mut out,
            &HostFacts::default(),
        );
        // The flag answers for the machine only: a target name that would be an error is not looked at.
        assert!(result.is_ok());
        assert!(!String::from_utf8(out).unwrap().contains("no-such-target"));
    }

    const PROCESS: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/true\"]\n";

    #[test]
    fn an_old_kernel_is_warned_about_when_a_process_target_is_configured() {
        let mut out = Vec::new();
        let facts = kernel("6.8.0-146-generic");
        let code = run_with_host(&ctx_with(PROCESS), &args(), &mut out, &facts).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, crate::EXIT_OK, "a warning never fails doctor");
        assert!(text.contains("warning: kernel 6.8.0-146-generic"), "{text}");
    }

    #[test]
    fn a_named_process_target_gets_the_host_report_even_when_its_check_errors() {
        // A hermit that cannot run makes the per-target check fail; the host report was printed before it.
        let config = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/true\"]\n";
        let mut named = args();
        named.targets = vec!["svc".to_string()];
        let mut out = Vec::new();
        let facts = kernel("6.8.0-146-generic");
        let _ = run_with_host(&ctx_with(config), &named, &mut out, &facts);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("warning: kernel 6.8.0-146-generic"), "{text}");
    }

    #[test]
    fn naming_a_target_that_is_not_configured_prints_no_host_report() {
        let mut named = args();
        named.targets = vec!["nonexistent".to_string()];
        let mut out = Vec::new();
        let facts = kernel("6.8.0-146-generic");
        let _ = run_with_host(&ctx_with(PROCESS), &named, &mut out, &facts);
        assert!(out.is_empty());
    }

    #[test]
    fn a_new_kernel_is_not_warned_about() {
        let mut out = Vec::new();
        let facts = kernel("6.17.0-42-generic");
        run_with_host(&ctx_with(PROCESS), &args(), &mut out, &facts).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn without_a_process_target_the_host_is_not_checked() {
        let mut out = Vec::new();
        let facts = kernel("6.8.0-146-generic");
        run_with_host(&ctx_with(""), &args(), &mut out, &facts).unwrap();
        assert!(out.is_empty());
    }
}

use crate::cli::DoctorArgs;
use crate::ctx::{CmdResult, Ctx};
use crate::host::process_backend::{self, ProcessBackendFacts};
use crate::host::{self, HostFacts};
use hairspring_engine::config::TargetKind;
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
    // The flag answers for the machine only; no target is looked at.
    if args.process_backend {
        return run_process_backend(out, &host::process_probe::probe());
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

/// Prints the process backend's prerequisites for `facts`; exits `EXIT_UNSUPPORTED_HOST` when any is unmet.
fn run_process_backend(out: &mut dyn Write, facts: &ProcessBackendFacts) -> CmdResult<i32> {
    let report = process_backend::assess(facts);
    process_backend::write_report(out, &report)?;
    Ok(match report.can_run() {
        true => crate::EXIT_OK,
        false => crate::EXIT_UNSUPPORTED_HOST,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::process_backend::{HermitFact, PerfFact, ProcessBackendFacts};
    use crate::host::{AutoUpdates, LoaderHashes};
    use hairspring_engine::erased::Registry;

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
        }
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
        let code = run_process_backend(&mut out, &backend_facts("linux")).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, crate::EXIT_OK, "{text}");
        assert!(text.contains("result: ok:"), "{text}");
    }

    #[test]
    fn a_machine_that_cannot_run_the_process_backend_gets_its_own_exit_code() {
        let mut out = Vec::new();
        let code = run_process_backend(&mut out, &backend_facts("macos")).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, crate::EXIT_UNSUPPORTED_HOST, "{text}");
        assert_ne!(
            code,
            crate::EXIT_VIOLATION,
            "must not look like a found bug"
        );
        assert!(text.contains("result: CANNOT RUN"), "{text}");
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

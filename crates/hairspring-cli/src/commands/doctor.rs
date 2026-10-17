use crate::cli::DoctorArgs;
use crate::ctx::{CmdResult, Ctx};
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
                "{name}: deterministic over {} seeds x {} steps",
                report.seeds_checked, report.steps_per_seed
            )?,
            Some(d) => {
                code = crate::EXIT_VIOLATION;
                writeln!(out, "{name}: NON-DETERMINISTIC (seed {}, diverged at step {}). Look for HashMap iteration, time, threads or randomness outside the simulation.", d.seed, d.step)?;
            }
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        }
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

use crate::ctx::CmdResult;
use std::io::Write;
use std::path::Path;

const SAMPLE: &str = r#"# Hairspring configuration.
#   hairspring targets    list targets
#   hairspring test       explore all targets (exit 1 if a property is violated)
#   hairspring doctor     check that targets are deterministic

# A simulation target compiled into the binary (see `hairspring targets`).
[target.lock-lease-demo]
kind = "sim"
sim = "lock-lease"
description = "Lease-based lock with a stale-holder split brain"

[target.lock-lease-demo.budget]
max_steps = 200000
wall_secs = 60

# A real program run under Hermit (Linux x86-64 only, experimental).
# [target.my-service]
# kind = "process"
# command = ["/opt/app/bin/service", "--test-mode"]
# env = { MODE = "test" }
# trial_timeout_secs = 120   # Hermit is slow; raise this for heavy programs
# hang_is_violation = true   # a trial that times out is a failure (default: inconclusive, never a pass)
# parallelism = 4            # trials run at once (1-64); keep at or below your core count
# hermit_args = ["--base-env=minimal"]   # with minimal or empty, Hairspring passes env, HAIRSPRING_SEED etc. with --env=NAME
# pass_env = false           # default true; false stops that and gives the program only Hermit's own base environment
#
# How threads are scheduled (all optional; the default is seeded random scheduling).
# Races inside a few instructions need a short timeslice with chaos mode, not the random heuristic.
# chaos = true                    # randomise thread priorities each timeslice; cannot be combined with heuristic = "random"
# max_timeslice_ns = 1000000      # preempt after this much virtual time (smaller finds finer races, costs more)
# skid_margin = 25                # PMU skid margin; smaller is faster. An overshoot aborts the run as an infrastructure error
# heuristic = "sticky-random"     # or "random" (default) or "none"; sticky-random also takes sticky_permille = 900
#
# [target.my-service.budget]
# max_trials = 100
# wall_secs = 600            # the whole run's limit (at most 600); the default of 60 s allows few Hermit trials
"#;

pub fn run(path: &Path, out: &mut dyn Write) -> CmdResult<i32> {
    if path.exists() {
        return Err(format!(
            "{} already exists; refusing to overwrite it",
            path.display()
        )
        .into());
    }
    std::fs::write(path, SAMPLE).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    writeln!(out, "wrote {}", path.display())?;
    Ok(crate::EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::SAMPLE;
    use hairspring_engine::config::{Config, TargetKind};

    /// The process example with its leading `# ` removed, as a user who uncomments it gets it (the prose lines and the
    /// scheduling alternatives stay commented).
    fn uncommented_process_example() -> String {
        let start = SAMPLE
            .find("# [target.my-service]")
            .expect("process example");
        SAMPLE[start..]
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| !line.starts_with("How threads") && !line.starts_with("Races inside"))
            .filter(|line| !line.starts_with("chaos") && !line.starts_with("heuristic"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_uncommented_process_example_parses_with_the_run_limit_it_suggests() {
        let config = Config::parse(&uncommented_process_example()).expect("valid config");
        let target = &config.targets["my-service"];
        assert!(matches!(target.kind, TargetKind::Process { .. }));
        assert_eq!(target.budget.wall_secs, 600);
        assert_eq!(target.budget.max_trials, 100);
    }

    #[test]
    fn the_run_limit_line_says_what_the_default_allows() {
        let line = SAMPLE
            .lines()
            .find(|l| l.starts_with("# wall_secs"))
            .expect("a wall_secs line");
        assert!(line.contains("default of 60 s"), "{line}");
        assert!(line.contains("at most 600"), "{line}");
    }
}

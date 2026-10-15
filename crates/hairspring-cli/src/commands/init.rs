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
#
# How threads are scheduled (all optional; the default is seeded random scheduling).
# Races inside a few instructions need a short timeslice with chaos mode, not the random heuristic.
# chaos = true                    # randomise thread priorities each timeslice; cannot be combined with heuristic = "random"
# max_timeslice_ns = 1000000      # preempt after this much virtual time (smaller finds finer races, costs more)
# skid_margin = 25                # PMU skid margin; smaller is faster, Hermit panics on overshoot if you pass --panic-on-rbc-overshoot
# heuristic = "sticky-random"     # or "random" (default) or "none"; sticky-random also takes sticky_permille = 900
#
# [target.my-service.budget]
# max_trials = 100
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

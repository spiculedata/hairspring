use crate::ctx::CmdResult;
use std::io::Write;
use std::path::Path;

const SAMPLE: &str = r#"# Anthill configuration.
#   anthill targets    list targets
#   anthill test       explore all targets (exit 1 if a property is violated)
#   anthill doctor     check that targets are deterministic

# A simulation target compiled into the binary (see `anthill targets`).
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

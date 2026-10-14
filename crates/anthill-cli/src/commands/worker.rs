use crate::ctx::{CmdResult, Ctx};
use anthill_worker::{run_loop, WorkerConfig};
use clap::Args;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Args)]
pub struct WorkerArgs {
    /// Server base URL, for example http://host:7878
    #[arg(long)]
    pub server: String,
    /// File containing the API token (alternative to the ANTHILL_TOKEN environment variable).
    #[arg(long)]
    pub token_file: Option<PathBuf>,
    /// Name shown in the server's worker list.
    #[arg(long)]
    pub name: Option<String>,
    /// Milliseconds between polls when the queue is empty.
    #[arg(long, default_value_t = 1000)]
    pub poll_ms: u64,
}

const MAX_NAME_LEN: usize = 64;

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "host".to_string())
}

/// Unique per machine and process: containers all have pid 1, so the host name is what tells them apart.
pub(crate) fn worker_name_for(host: &str, pid: u32) -> String {
    let clean: String = host
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let name = format!("worker-{clean}-{pid}");
    name.chars().take(MAX_NAME_LEN).collect()
}

fn default_worker_name() -> String {
    worker_name_for(&host_name(), std::process::id())
}

fn token_from(args: &WorkerArgs) -> CmdResult<Option<String>> {
    if let Some(path) = &args.token_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let token = text.trim().to_string();
        if token.is_empty() {
            return Err(format!("{} is empty", path.display()).into());
        }
        return Ok(Some(token));
    }
    Ok(std::env::var("ANTHILL_TOKEN")
        .ok()
        .filter(|t| !t.is_empty()))
}

pub fn run(ctx: &Ctx, args: &WorkerArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let name = args.name.clone().unwrap_or_else(default_worker_name);
    let mut cfg = WorkerConfig::new(args.server.clone(), name.clone());
    cfg.token = token_from(args)?;
    cfg.poll = Duration::from_millis(args.poll_ms.max(10));
    writeln!(out, "anthill: worker `{name}` serving {}", cfg.server)?;
    out.flush()?;
    // Runs until the process is terminated; an interrupted job's lease expires and it is retried elsewhere.
    run_loop(&ctx.engine, &cfg, &AtomicBool::new(false))?;
    Ok(crate::EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::worker_name_for;

    #[test]
    fn names_differ_across_hosts_even_when_every_pid_is_one() {
        assert_ne!(worker_name_for("3f9a1c", 1), worker_name_for("77be02", 1));
        assert_eq!(worker_name_for("3f9a1c", 1), "worker-3f9a1c-1");
    }

    #[test]
    fn names_are_sanitised_and_bounded() {
        let n = worker_name_for(&"x".repeat(200), 5);
        assert!(n.len() <= 64);
        assert!(!worker_name_for("bad host/\"name\"", 2).contains(['/', ' ', '"']));
    }
}

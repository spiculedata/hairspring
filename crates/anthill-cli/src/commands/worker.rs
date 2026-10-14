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

fn token_from(args: &WorkerArgs) -> CmdResult<Option<String>> {
    if let Some(path) = &args.token_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        return Ok(Some(text.trim().to_string()));
    }
    Ok(std::env::var("ANTHILL_TOKEN")
        .ok()
        .filter(|t| !t.is_empty()))
}

pub fn run(ctx: &Ctx, args: &WorkerArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("worker-{}", std::process::id()));
    let cfg = WorkerConfig {
        server: args.server.clone(),
        token: token_from(args)?,
        name: name.clone(),
        poll: Duration::from_millis(args.poll_ms.max(10)),
    };
    writeln!(out, "anthill: worker `{name}` serving {}", cfg.server)?;
    out.flush()?;
    // Runs until the process is terminated; an interrupted job's lease expires and it is retried elsewhere.
    run_loop(&ctx.engine, &cfg, &AtomicBool::new(false))?;
    Ok(crate::EXIT_OK)
}

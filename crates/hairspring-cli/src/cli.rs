use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "hairspring",
    version,
    about = "Deterministic simulation testing: explore, shrink, replay."
)]
pub struct Cli {
    /// Path to the target configuration.
    #[arg(long, global = true, default_value = "hairspring.toml")]
    pub config: PathBuf,
    /// Path to the local results database.
    #[arg(long, global = true, default_value = ".hairspring/hairspring.db")]
    pub db: PathBuf,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Write a starter hairspring.toml.
    Init,
    /// List the available targets.
    Targets,
    /// Explore targets for property violations; exit 1 if any is found.
    Test(TestArgs),
    /// Replay a bundle (file, `-` for stdin, or stored hash); exit 1 if it still fails.
    Replay(ReplayArgs),
    /// Check that targets are deterministic (same seed, same trace).
    Doctor(DoctorArgs),
    /// List stored runs, newest first.
    Results(ResultsArgs),
    /// Show one stored run.
    Show(ShowArgs),
    /// Inspect or export stored bundles.
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
    /// Start the web server, REST API and MCP endpoint.
    Serve(crate::commands::serve::ServeArgs),
    /// Run a worker that pulls jobs from a server.
    Worker(crate::commands::worker::WorkerArgs),
    /// Run the MCP server over stdio for local agents.
    Mcp,
    /// Create an API token for the server (printed once).
    Token(crate::commands::token::TokenArgs),
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Junit,
    Sarif,
}

#[derive(Args)]
pub struct TestArgs {
    /// Targets to run (default: all).
    pub targets: Vec<String>,
    /// Exploration seed (default: random; always printed).
    #[arg(long)]
    pub seed: Option<u64>,
    /// Override the step budget.
    #[arg(long)]
    pub budget_steps: Option<u64>,
    /// Override the wall-clock budget in seconds.
    #[arg(long)]
    pub budget_secs: Option<u64>,
    #[arg(long, value_enum, default_value = "text")]
    pub format: Format,
    /// Where to write bundle files for failures.
    #[arg(long, default_value = ".hairspring/bundles")]
    pub bundle_dir: PathBuf,
}

#[derive(Args)]
#[command(
    after_help = "Exit codes:\n  0 not reproduced, on the environment it was recorded in\n  1 reproduced: the property is still violated\n  2 error (including a replay refused because the dynamic loader changed)\n  3 not reproduced on a changed environment: inconclusive, not evidence the bug is fixed"
)]
pub struct ReplayArgs {
    /// Bundle file, `-` for stdin, or a stored bundle hash.
    pub input: String,
    /// Replay even when the host's dynamic loader differs from the recording (the differences are still reported).
    #[arg(long)]
    pub allow_environment_change: bool,
}

#[derive(Args)]
pub struct DoctorArgs {
    /// Targets to check (default: all).
    pub targets: Vec<String>,
    #[arg(long, default_value_t = 8)]
    pub seeds: u32,
    #[arg(long, default_value_t = 200)]
    pub steps: usize,
    /// Report whether this machine meets the `process` backend's prerequisites (Hermit, perf counters, ptrace),
    /// instead of checking targets. Exits 4 when it cannot run here.
    #[arg(long)]
    pub process_backend: bool,
}

#[derive(Args)]
pub struct ResultsArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    /// Print NDJSON: one line per run, then `{"_next":null}`.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ShowArgs {
    /// A run id (run_...).
    pub id: String,
    /// Print NDJSON: the run, one line per property, then `{"_next":null}`.
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum BundleCommand {
    /// Print a stored bundle.
    Show { hash: String },
    /// Write a stored bundle to a file.
    Export {
        hash: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}

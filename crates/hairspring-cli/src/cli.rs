use crate::seed_range::SeedRange;
use clap::{Args, Parser, Subcommand, ValueEnum};
use hairspring_engine::config::MAX_TRIALS_CAP;
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
    /// Confirm a failing seed: same-seed reruns, a bundle replay and optional controls, each recorded.
    Confirm(ConfirmArgs),
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
    /// Exploration seed (default: random; always printed). A sweep runs consecutive seeds from this one.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Override the step budget.
    #[arg(long)]
    pub budget_steps: Option<u64>,
    /// Override the wall-clock budget in seconds.
    #[arg(long)]
    pub budget_secs: Option<u64>,
    /// Run the whole trial budget past every failure and store every trial, so the report can state a rate with an
    /// interval. Without it a run stops at its first confirmed failure.
    #[arg(long)]
    pub sweep: bool,
    /// How many trials a sweep runs (default: the target's own budget, at most 10000).
    #[arg(
        long,
        requires = "sweep",
        value_parser = clap::value_parser!(u64).range(1..=MAX_TRIALS_CAP)
    )]
    pub trials: Option<u64>,
    /// The seeds a sweep runs, as FIRST..END with END left out (for example 100..200 is 100 trials).
    #[arg(long, requires = "sweep", conflicts_with_all = ["seed", "trials"])]
    pub seeds: Option<SeedRange>,
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
#[command(
    after_help = "Checks, in order: same-seed reruns (default 3, each compared on behaviour hash, schedule fingerprint and failing property), a replay of the bundle with its environment check, then a fixed-version control (--control) and Hermit's strict determinism verify (--strict-verify) when asked for. Every check is recorded; for a run or failure id the results are stored and `hairspring show` summarises them.\n\nFinding: candidate -> rerun_repeated (reruns repeat it) -> confirmed (and the replay does) -> verified (and the strict verify agrees), or not_reproducible, or control_failed (the fixed build still fails at the seed).\n\nExit codes:\n  1 confirmed or verified: the failure repeats\n  0 not_reproducible: it did not repeat on the environment it was recorded in\n  2 error, including a request over the limits\n  3 inconclusive: rerun_repeated (reruns only), a check could not run or the environment differs from the recording, or the control failed; none of these shows the bug is fixed"
)]
pub struct ConfirmArgs {
    /// A run id (run_...) or failure id (fail_...) whose results are stored, or a bundle file, `-` for stdin or a stored
    /// bundle hash, which are confirmed without storing anything.
    pub input: String,
    /// Same-seed reruns, 1 to 10.
    #[arg(long, default_value_t = hairspring_engine::confirm::DEFAULT_RERUNS)]
    pub reruns: u32,
    /// A target that runs the fixed build: the same seed and flags on it should not fail.
    #[arg(long)]
    pub control: Option<String>,
    /// Also run Hermit's strict determinism verify (two more runs).
    #[arg(long)]
    pub strict_verify: bool,
    /// Go on when the host's dynamic loader differs from the recording (the differences are still recorded).
    #[arg(long)]
    pub allow_environment_change: bool,
    /// Wall-clock budget for the whole confirmation in seconds, 1 to 600; checks that do not fit are skipped.
    #[arg(long, default_value_t = hairspring_engine::confirm::DEFAULT_WALL_SECS)]
    pub wall_secs: u64,
    /// Print the stored answer as one JSON document: an object for a failure id or a bundle, an array with an entry per
    /// failure for a run id.
    #[arg(long)]
    pub json: bool,
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

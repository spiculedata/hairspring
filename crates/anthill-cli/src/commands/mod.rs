pub mod doctor;
pub mod init;
pub mod inspect;
pub mod replay;
pub mod serve;
pub mod test;
pub mod token;
pub mod worker;

use crate::cli::{Cli, Command};
use crate::ctx::{CmdResult, Ctx};
use anthill_engine::config::Config;
use anthill_engine::engine::Engine;
use anthill_engine::erased::Registry;
use std::io::Write;
use std::sync::Arc;

/// Builds the engine with every backend compiled into this binary.
pub fn build_engine(registry: Arc<Registry>, config: Config) -> Engine {
    Engine::new(registry, config)
}

pub fn dispatch(
    cli: Cli,
    registry: Registry,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> CmdResult<i32> {
    if let Command::Init = cli.command {
        return init::run(&cli.config, out);
    }
    let ctx = Ctx::load(registry, &cli.config, cli.db.clone())?;
    match cli.command {
        Command::Init => unreachable!("handled above"),
        Command::Targets => inspect::targets(&ctx, out),
        Command::Test(a) => test::run(&ctx, &a, out, err),
        Command::Replay(a) => replay::run(&ctx, &a, out),
        Command::Doctor(a) => doctor::run(&ctx, &a, out),
        Command::Results(a) => inspect::results(&ctx, &a, out),
        Command::Show(a) => inspect::show(&ctx, &a, out),
        Command::Bundle { command } => inspect::bundle(&ctx, &command, out),
        Command::Serve(a) => serve::run(&ctx, &a, out),
        Command::Worker(a) => worker::run(&ctx, &a, out),
        Command::Mcp => serve::run_mcp_stdio(&ctx),
        Command::Token(a) => token::run(&a, out),
    }
}

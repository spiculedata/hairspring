use crate::ctx::{CmdResult, Ctx};
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct WorkerArgs {}

pub fn run(_ctx: &Ctx, _args: &WorkerArgs, _out: &mut dyn Write) -> CmdResult<i32> {
    Err("`worker` is added in the next phase".into())
}

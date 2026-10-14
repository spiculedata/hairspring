use crate::ctx::{CmdResult, Ctx};
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct ServeArgs {}

pub fn run(_ctx: &Ctx, _args: &ServeArgs, _out: &mut dyn Write) -> CmdResult<i32> {
    Err("`serve` is added in the next phase".into())
}

pub fn run_mcp_stdio(_ctx: &Ctx) -> CmdResult<i32> {
    Err("`mcp` is added in the next phase".into())
}

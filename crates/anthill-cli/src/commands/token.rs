use crate::ctx::CmdResult;
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct TokenArgs {}

pub fn run(_args: &TokenArgs, _out: &mut dyn Write) -> CmdResult<i32> {
    Err("`token` is added in a later phase".into())
}

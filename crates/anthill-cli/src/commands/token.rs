use crate::ctx::CmdResult;
use anthill_service::Role;
use clap::Args;
use std::io::{Read, Write};

#[derive(Args)]
pub struct TokenArgs {
    /// Role for the token: read, run or admin.
    #[arg(long, default_value = "run")]
    pub role: String,
}

const TOKEN_BYTES: usize = 32;

fn random_token() -> CmdResult<String> {
    let mut buf = [0u8; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("cannot read the OS random source (/dev/urandom is required): {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn run(args: &TokenArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let role = Role::parse(&args.role)
        .ok_or_else(|| format!("unknown role `{}` (use read, run or admin)", args.role))?;
    let token = random_token()?;
    writeln!(out, "New API token (shown once, store it safely):")?;
    writeln!(out, "  {token}")?;
    writeln!(
        out,
        "Give it to the server with:  ANTHILL_TOKENS={token}:{}",
        role.as_str()
    )?;
    writeln!(out, "Give it to a worker with:    ANTHILL_TOKEN={token}")?;
    Ok(crate::EXIT_OK)
}

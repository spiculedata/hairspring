use crate::ctx::{CmdResult, Ctx};
use anthill_mcp::McpServer;
use anthill_server::{start, Auth, ServerConfig};
use anthill_service::Role;
use clap::Args;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Args)]
pub struct ServeArgs {
    /// Address to listen on. Non-loopback addresses require tokens.
    #[arg(long, default_value = "127.0.0.1:7878")]
    pub addr: String,
    /// Worker threads inside this process that execute queued runs (0 = remote workers only).
    #[arg(long, default_value_t = 1)]
    pub workers: usize,
    /// Requests per minute per caller (0 disables the limit).
    #[arg(long, default_value_t = 1200)]
    pub rate_limit: u32,
    /// File with one `token:role` per line (alternative to the ANTHILL_TOKENS environment variable).
    #[arg(long)]
    pub tokens_file: Option<PathBuf>,
}

fn auth_from(args: &ServeArgs) -> CmdResult<Auth> {
    let spec = match &args.tokens_file {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join(",")
        }
        None => std::env::var("ANTHILL_TOKENS").unwrap_or_default(),
    };
    if spec.trim().is_empty() {
        return Ok(Auth::open());
    }
    Ok(Auth::from_spec(&spec)?)
}

pub fn run(ctx: &Ctx, args: &ServeArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let addr: SocketAddr = args
        .addr
        .parse()
        .map_err(|e| format!("invalid --addr `{}`: {e}", args.addr))?;
    let auth = auth_from(args)?;
    let open = auth.is_open();
    let mut config = ServerConfig::new(addr, auth);
    config.embedded_workers = args.workers;
    config.rate_limit_per_min = args.rate_limit;
    config.handle_ctrl_c = true;
    let server = start(config, ctx.service()?, ctx.engine.clone())
        .map_err(|e| format!("cannot start the server: {e}"))?;
    writeln!(out, "anthill: listening on http://{}", server.addr())?;
    writeln!(
        out,
        "anthill: REST API at /api, MCP at /mcp, web UI at /{}",
        if open {
            "  (no tokens configured: loopback only)"
        } else {
            ""
        }
    )?;
    out.flush()?;
    server.wait();
    Ok(crate::EXIT_OK)
}

/// MCP over stdio for local agents (for example `claude mcp add anthill -- anthill mcp`).
/// Runs execute inside this process; stdout carries only protocol messages.
pub fn run_mcp_stdio(ctx: &Ctx) -> CmdResult<i32> {
    let server = McpServer::new(ctx.service()?, ctx.engine.clone(), true);
    let stdin = std::io::stdin();
    anthill_mcp::serve_stdio(&server, Role::Admin, stdin.lock(), std::io::stdout().lock())?;
    Ok(crate::EXIT_OK)
}

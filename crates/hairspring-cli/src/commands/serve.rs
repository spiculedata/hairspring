use crate::ctx::{CmdResult, Ctx};
use clap::Args;
use hairspring_mcp::McpServer;
use hairspring_server::{start, Auth, ServerConfig};
use hairspring_service::Role;
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
    /// Workers one token may register per hour (0 disables the limit).
    #[arg(long, default_value_t = 100)]
    pub register_limit: u32,
    /// File with one `token:role` per line (alternative to the HAIRSPRING_TOKENS environment variable).
    #[arg(long)]
    pub tokens_file: Option<PathBuf>,
}

/// A readable-by-others secrets file is a leak waiting to happen; say so (Unix only).
pub(crate) fn loose_permissions_warning(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    (mode & 0o077 != 0).then(|| {
        format!(
            "warning: {} is readable by other users (mode {:o}); run `chmod 600` on it",
            path.display(),
            mode & 0o777
        )
    })
}

fn auth_from(args: &ServeArgs) -> CmdResult<Auth> {
    let spec = match &args.tokens_file {
        Some(path) => {
            if let Some(w) = loose_permissions_warning(path) {
                eprintln!("hairspring: {w}");
            }
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join(",")
        }
        None => std::env::var("HAIRSPRING_TOKENS").unwrap_or_default(),
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
    config.register_limit = args.register_limit;
    config.handle_ctrl_c = true;
    let server = start(config, ctx.service()?, ctx.engine.clone())
        .map_err(|e| format!("cannot start the server: {e}"))?;
    writeln!(out, "hairspring: listening on http://{}", server.addr())?;
    writeln!(
        out,
        "hairspring: REST API at /api, MCP at /mcp, web UI at /{}",
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

/// MCP over stdio for local agents (for example `claude mcp add hairspring -- hairspring mcp`).
/// Runs execute inside this process; stdout carries only protocol messages.
pub fn run_mcp_stdio(ctx: &Ctx) -> CmdResult<i32> {
    let server = McpServer::new(ctx.service()?, ctx.engine.clone(), true);
    let stdin = std::io::stdin();
    hairspring_mcp::serve_stdio(&server, Role::Admin, stdin.lock(), std::io::stdout().lock())?;
    Ok(crate::EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::loose_permissions_warning;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn warns_only_for_files_others_can_read() {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-perm-{}-{}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("tokens");
        std::fs::write(&f, "x").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(loose_permissions_warning(&f).unwrap().contains("chmod 600"));
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(loose_permissions_warning(&f).is_none());
        assert!(loose_permissions_warning(&dir.join("missing")).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}

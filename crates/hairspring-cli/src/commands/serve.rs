use crate::ctx::{CmdResult, Ctx};
use clap::Args;
use hairspring_mcp::McpServer;
use hairspring_server::{start, Auth, ServerConfig, TrustedProxies};
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
    /// Workers one token may register per hour (0 disables the limit). Counted in memory: a restart resets the count.
    #[arg(long, default_value_t = 100)]
    pub register_limit: u32,
    /// Open connections allowed at once (1 to 1000000); a connection beyond it is answered 503 and closed (just closed
    /// while 128 are being refused at once). Keep it below the process's file-descriptor limit.
    #[arg(long, default_value_t = hairspring_server::DEFAULT_MAX_CONNECTIONS)]
    pub max_connections: usize,
    /// Replays allowed to run on the server at once, REST and MCP together (1 to 64). A replay beyond it is
    /// refused with 429 and `Retry-After`, not queued.
    #[arg(long, default_value_t = hairspring_mcp::DEFAULT_MAX_REPLAYS)]
    pub max_replays: usize,
    /// File with one `token:role` per line (alternative to the HAIRSPRING_TOKENS environment variable).
    #[arg(long)]
    pub tokens_file: Option<PathBuf>,
    /// Name of your organisation, shown in the top bar of the web UI (plain text, at most 80 characters; an empty
    /// value shows none). Alternative to the HAIRSPRING_ORGANISATION environment variable.
    #[arg(long)]
    pub organisation: Option<String>,
    /// Reverse proxies (addresses or networks, comma-separated, e.g. `10.0.0.5,2001:db8::/64`) whose
    /// X-Forwarded-For header names the client a request is counted under for the rate limit. Only requests whose
    /// connection comes from one of these use the header, and only its right-most address that is not itself a
    /// listed proxy (anything further left is ignored); the proxy should overwrite any X-Forwarded-For the client sent. Off by default
    /// (forwarding headers are ignored). Networks wider than /8 (IPv4) or /32 (IPv6), unspecified addresses and
    /// ranges starting at them, and IPv4-mapped or IPv4-compatible IPv6 entries are refused. Alternative to the
    /// HAIRSPRING_TRUSTED_PROXY environment variable.
    #[arg(long, value_name = "ADDR[,ADDR...]")]
    pub trusted_proxy: Option<String>,
}

const ORGANISATION_ENV: &str = "HAIRSPRING_ORGANISATION";
const TRUSTED_PROXY_ENV: &str = "HAIRSPRING_TRUSTED_PROXY";

/// The trusted proxies: the flag if given (an empty flag trusts none), else the environment variable. A list that is
/// not valid is an error that names where it came from.
pub(crate) fn trusted_proxies_from(
    flag: Option<&str>,
    env: Option<String>,
) -> CmdResult<TrustedProxies> {
    let (raw, source) = match (flag, env) {
        (Some(flag), _) => (flag.to_owned(), "--trusted-proxy"),
        (None, Some(env)) => (env, TRUSTED_PROXY_ENV),
        (None, None) => return Ok(TrustedProxies::default()),
    };
    TrustedProxies::parse(&raw).map_err(|e| format!("{source}: {e}").into())
}

/// The organisation name to show: the flag if given (an empty flag switches the name off), else the environment
/// variable. A name that is not valid is an error that names where it came from and does not repeat the value.
pub(crate) fn organisation_from(
    flag: Option<&str>,
    env: Option<String>,
) -> CmdResult<Option<String>> {
    let (raw, source) = match (flag, env) {
        (Some(flag), _) => (flag.to_owned(), "--organisation".to_owned()),
        (None, Some(env)) => (env, ORGANISATION_ENV.to_owned()),
        (None, None) => return Ok(None),
    };
    hairspring_server::parse_organisation(&raw).map_err(|e| format!("{source}: {e}").into())
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
    config.max_connections = args.max_connections;
    config.max_replays = args.max_replays;
    config.organisation = organisation_from(
        args.organisation.as_deref(),
        std::env::var(ORGANISATION_ENV).ok(),
    )?;
    config.trusted_proxies = trusted_proxies_from(
        args.trusted_proxy.as_deref(),
        std::env::var(TRUSTED_PROXY_ENV).ok(),
    )?;
    let trusted = config.trusted_proxies.to_string();
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
    if !trusted.is_empty() {
        writeln!(
            out,
            "hairspring: rate limit uses X-Forwarded-For from trusted proxies: {trusted}"
        )?;
    }
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
    use super::{loose_permissions_warning, organisation_from, trusted_proxies_from};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_flag_wins_over_the_environment_and_either_may_be_absent() {
        let name = |flag: Option<&str>, env: Option<&str>| {
            organisation_from(flag, env.map(str::to_owned)).unwrap()
        };
        assert_eq!(name(None, None), None);
        assert_eq!(
            name(Some("Flag Ltd"), Some("Env Ltd")).as_deref(),
            Some("Flag Ltd")
        );
        assert_eq!(name(None, Some("Env Ltd")).as_deref(), Some("Env Ltd"));
        assert_eq!(name(Some("  Acme  "), None).as_deref(), Some("Acme"));
        assert_eq!(
            name(Some(""), Some("Env Ltd")),
            None,
            "an empty flag switches the name off"
        );
        assert_eq!(name(None, Some("  ")), None);
    }

    #[test]
    fn a_name_that_is_not_valid_is_an_error_naming_the_setting() {
        let flag = organisation_from(Some("a\nb"), None)
            .unwrap_err()
            .to_string();
        assert!(flag.contains("--organisation"), "{flag}");
        let env = organisation_from(None, Some("x".repeat(81)))
            .unwrap_err()
            .to_string();
        assert!(env.contains("HAIRSPRING_ORGANISATION"), "{env}");
        assert!(!env.contains("xxxx"), "the value is not repeated: {env}");
    }

    #[test]
    fn trusted_proxies_come_from_the_flag_else_the_environment() {
        let list = |flag: Option<&str>, env: Option<&str>| {
            trusted_proxies_from(flag, env.map(str::to_owned))
                .unwrap()
                .to_string()
        };
        assert_eq!(list(None, None), "");
        assert_eq!(list(Some("10.0.0.5"), Some("10.0.0.6")), "10.0.0.5/32");
        assert_eq!(list(None, Some("10.0.0.0/8")), "10.0.0.0/8");
        assert_eq!(
            list(Some(""), Some("10.0.0.6")),
            "",
            "an empty flag trusts none"
        );
    }

    #[test]
    fn a_trusted_proxy_list_that_covers_everyone_is_an_error_naming_the_setting() {
        let flag = trusted_proxies_from(Some("0.0.0.0/0"), None)
            .unwrap_err()
            .to_string();
        assert!(flag.starts_with("--trusted-proxy: "), "{flag}");
        assert!(flag.contains("every address"), "{flag}");
        let env = trusted_proxies_from(None, Some("::/0".to_owned()))
            .unwrap_err()
            .to_string();
        assert!(env.starts_with("HAIRSPRING_TRUSTED_PROXY: "), "{env}");
    }

    #[test]
    fn a_bad_trusted_proxy_value_is_not_repeated_in_full() {
        let pasted = format!("{}-TAIL-OF-A-TOKEN", "t".repeat(32));
        let flag = trusted_proxies_from(Some(&pasted), None)
            .unwrap_err()
            .to_string();
        assert!(flag.starts_with("--trusted-proxy: "), "{flag}");
        assert!(!flag.contains("TAIL-OF-A-TOKEN"), "{flag}");
        let env = trusted_proxies_from(None, Some(format!("{pasted}\n\u{1b}[2J")))
            .unwrap_err()
            .to_string();
        assert!(
            !env.contains("TAIL-OF-A-TOKEN") && !env.contains('\u{1b}'),
            "{env:?}"
        );
        let broad = trusted_proxies_from(None, Some("10.0.0.0/7".to_owned()))
            .unwrap_err()
            .to_string();
        assert!(broad.contains("too broad"), "{broad}");
    }

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

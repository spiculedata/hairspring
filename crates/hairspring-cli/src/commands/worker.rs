use crate::ctx::{CmdResult, Ctx};
use clap::Args;
use hairspring_worker::{run_loop_notify, WorkerConfig};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Args)]
pub struct WorkerArgs {
    /// Server base URL, for example http://host:7878
    #[arg(long)]
    pub server: String,
    /// File containing the API token (alternative to the HAIRSPRING_TOKEN environment variable).
    #[arg(long)]
    pub token_file: Option<PathBuf>,
    /// Name shown in the server's worker list.
    #[arg(long)]
    pub name: Option<String>,
    /// Milliseconds between polls when the queue is empty.
    #[arg(long, default_value_t = 1000)]
    pub poll_ms: u64,
}

const MAX_NAME_LEN: usize = 64;

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "host".to_string())
}

/// Unique per machine and process: containers all have pid 1, so the host name is what tells them apart.
pub(crate) fn worker_name_for(host: &str, pid: u32) -> String {
    let clean: String = host
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let name = format!("worker-{clean}-{pid}");
    name.chars().take(MAX_NAME_LEN).collect()
}

fn default_worker_name() -> String {
    worker_name_for(&host_name(), std::process::id())
}

#[cfg(unix)]
fn warn_if_readable_by_others(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "hairspring: warning: {} is readable by other users; run `chmod 600` on it",
                path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_path: &std::path::Path) {}

const NOT_A_BARE_TOKEN: &str =
    "the worker token file must contain one bare token (the server's tokens file has token:role lines)";
/// The roles the server's tokens file gives after a token's last `:`.
const ROLES: [&str; 3] = ["read", "run", "admin"];

/// One token and nothing else: no second line or space, and not the server's `token:role` form. Sent as a header, a
/// newline would otherwise fail every request with an HTTP error that names neither the file nor the cause.
fn is_bare_token(token: &str) -> bool {
    let one_word = !token.chars().any(|c| c.is_whitespace() || c.is_control());
    let has_role = token
        .rsplit_once(':')
        .is_some_and(|(_, role)| ROLES.contains(&role.to_ascii_lowercase().as_str()));
    one_word && !has_role
}

fn token_from(args: &WorkerArgs) -> CmdResult<Option<String>> {
    if let Some(path) = &args.token_file {
        warn_if_readable_by_others(path);
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let token = text.trim().to_string();
        if token.is_empty() {
            return Err(format!("{} is empty", path.display()).into());
        }
        if !is_bare_token(&token) {
            return Err(format!("{}: {NOT_A_BARE_TOKEN}", path.display()).into());
        }
        return Ok(Some(token));
    }
    Ok(std::env::var("HAIRSPRING_TOKEN")
        .ok()
        .filter(|t| !t.is_empty()))
}

pub fn run(ctx: &Ctx, args: &WorkerArgs, out: &mut dyn Write) -> CmdResult<i32> {
    let name = args.name.clone().unwrap_or_else(default_worker_name);
    let mut cfg = WorkerConfig::new(args.server.clone(), name.clone());
    cfg.token = token_from(args)?;
    cfg.poll = Duration::from_millis(args.poll_ms.max(10));
    // Runs until the process is terminated; an interrupted job's lease expires and it is retried elsewhere. It says it
    // is serving only once the server has accepted its registration.
    let mut written = Ok(());
    run_loop_notify(&ctx.engine, &cfg, &AtomicBool::new(false), &mut |_| {
        written = writeln!(out, "hairspring: worker `{name}` serving {}", cfg.server)
            .and_then(|()| out.flush());
    })?;
    written?;
    Ok(crate::EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::{is_bare_token, worker_name_for};

    #[test]
    fn a_single_token_is_bare() {
        assert!(is_bare_token("0123456789abcdef0123"));
        // A colon is allowed inside a token; only a trailing role marks the server's form.
        assert!(is_bare_token("abc:def0123456789"));
    }

    #[test]
    fn the_server_token_role_form_and_several_words_or_lines_are_not_bare() {
        for role in ["read", "run", "admin", "RUN"] {
            assert!(
                !is_bare_token(&format!("0123456789abcdef:{role}")),
                "{role}"
            );
        }
        assert!(!is_bare_token("0123456789abcdef\nfedcba9876543210"));
        assert!(!is_bare_token("0123456789abcdef fedcba9876543210"));
    }

    #[test]
    fn names_differ_across_hosts_even_when_every_pid_is_one() {
        assert_ne!(worker_name_for("3f9a1c", 1), worker_name_for("77be02", 1));
        assert_eq!(worker_name_for("3f9a1c", 1), "worker-3f9a1c-1");
    }

    #[test]
    fn names_are_sanitised_and_bounded() {
        let n = worker_name_for(&"x".repeat(200), 5);
        assert!(n.len() <= 64);
        assert!(!worker_name_for("bad host/\"name\"", 2).contains(['/', ' ', '"']));
    }
}

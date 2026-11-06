//! Logic of the Rust loopback experiment server, kept apart from the sockets so that it can be tested alone.
//!
//! Experiment fixture, not product code. Nothing here has been run under Hermit. Fixture rules: the key table is an
//! array (never a hash map, so nothing depends on iteration order), no randomness, no name lookups, no pid, port, path
//! or clock value in any output line.

pub const KEYS: usize = 8;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold `data` into an FNV-1a 64 hash.
pub fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for b in data {
        h = (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME);
    }
    h
}

/// Orphan guard of `dsrs`: exit after this many one-second ticks with no request.
pub const IDLE_TICKS: u32 = 120;

/// Watch an activity counter once per `tick`; call `on_idle` when it has not moved for `limit_ticks` ticks in a row.
/// The program calls it with a 1 s tick and `IDLE_TICKS`; the unit tests call it with a short tick.
pub fn watch_idle(
    activity: &std::sync::atomic::AtomicU64,
    tick: std::time::Duration,
    limit_ticks: u32,
    on_idle: impl FnOnce(),
) {
    use std::sync::atomic::Ordering;
    let (mut last, mut idle) = (activity.load(Ordering::Relaxed), 0);
    loop {
        std::thread::sleep(tick);
        let now = activity.load(Ordering::Relaxed);
        idle = if now == last { idle + 1 } else { 0 };
        last = now;
        if idle >= limit_ticks {
            on_idle();
            return;
        }
    }
}

/// A node name that is safe to use in a path: 1 to 16 ASCII letters and digits.
pub fn valid_name(s: &str) -> bool {
    (1..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `k<d>` with d in 0..7.
pub fn key_index(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() == 2 && b[0] == b'k' && (b'0'..b'0' + KEYS as u8).contains(&b[1]) {
        Some(usize::from(b[1] - b'0'))
    } else {
        None
    }
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Table {
    present: [bool; KEYS],
    value: [u64; KEYS],
}

impl Table {
    pub fn set(&mut self, key: usize, value: u64) {
        self.present[key] = true;
        self.value[key] = value;
    }

    pub fn get(&self, key: usize) -> Option<u64> {
        self.present[key].then_some(self.value[key])
    }

    /// FNV-1a 64 of the text `k<d>=<value>;` for each present key in index order.
    pub fn digest(&self) -> u64 {
        let mut h = FNV_OFFSET;
        for k in 0..KEYS {
            if self.present[k] {
                h = fnv(h, format!("k{}={};", k, self.value[k]).as_bytes());
            }
        }
        h
    }
}

/// What the connection handler does after one request line.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Action {
    pub reply: Option<String>,
    pub replicate: Option<String>,
    pub quit: bool,
}

#[derive(Default)]
pub struct Server {
    pub node: String,
    table: Table,
    applied: u64,
    repl_in: u64,
    repl_out: u64,
    pub refused: u64,
}

impl Server {
    pub fn new(node: &str) -> Self {
        Server {
            node: node.to_string(),
            ..Server::default()
        }
    }

    /// Apply one request line. When `successor` is true a PUT is replicated.
    pub fn handle(&mut self, line: &str, successor: bool) -> Action {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            [] => Action::default(),
            [cmd @ ("PUT" | "REPL"), k, v] => {
                let parsed = key_index(k).zip(v.parse::<u64>().ok());
                let Some((key, value)) = parsed else {
                    return if *cmd == "PUT" {
                        reply("ERR\n")
                    } else {
                        Action::default()
                    };
                };
                self.table.set(key, value);
                if *cmd == "REPL" {
                    self.repl_in += 1;
                    return Action::default();
                }
                self.applied += 1;
                let mut act = reply("OK\n");
                if successor {
                    self.repl_out += 1;
                    act.replicate = Some(format!("REPL {} {}\n", k, v));
                }
                act
            }
            ["GET", k] => match key_index(k) {
                None => reply("ERR\n"),
                Some(key) => match self.table.get(key) {
                    Some(v) => reply(&format!("VAL {}\n", v)),
                    None => reply("NIL\n"),
                },
            },
            ["HELLO", ..] | ["HB"] => Action::default(),
            ["QUIT"] => Action {
                quit: true,
                ..Action::default()
            },
            _ => reply("ERR\n"),
        }
    }

    /// The DSN0 result line of a server, with mode `rust`.
    pub fn result_line(&self) -> String {
        format!(
            "DSN0 v1 role=srv node={} mode=rust applied={} repl_in={} repl_out={} refused={} hb=0 suspect=0 \
             clock=0000000000000000 state={:016x}\n",
            self.node,
            self.applied,
            self.repl_in,
            self.repl_out,
            self.refused,
            self.table.digest()
        )
    }
}

fn reply(text: &str) -> Action {
    Action {
        reply: Some(text.to_string()),
        ..Action::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden values computed by the independent Python oracle (tests/oracle.py), not by this crate.
    #[test]
    fn digest_matches_the_oracle() {
        assert_eq!(
            format!("{:016x}", Table::default().digest()),
            "cbf29ce484222325"
        );
        let mut t = Table::default();
        t.set(0, 5);
        t.set(3, 1003);
        assert_eq!(format!("{:016x}", t.digest()), "975ec6eda8b0a8bf");
        assert_eq!(
            format!("{:016x}", fnv(FNV_OFFSET, b"a")),
            "af63dc4c8601ec8c"
        );
    }

    #[test]
    fn the_idle_guard_fires_only_after_enough_quiet_ticks() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let quiet = AtomicU64::new(0);
        let started = Instant::now();
        let mut fired = false;
        watch_idle(&quiet, Duration::from_millis(5), 4, || fired = true);
        assert!(fired && started.elapsed() >= Duration::from_millis(20));
        // Activity on every tick keeps it quiet for at least twice the limit.
        let busy = Arc::new(AtomicU64::new(0));
        let feeder = Arc::clone(&busy);
        let handle = std::thread::spawn(move || {
            for _ in 0..40 {
                feeder.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let started = Instant::now();
        watch_idle(&busy, Duration::from_millis(5), 4, || {});
        handle.join().unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(80),
            "the guard fired while requests kept arriving"
        );
        assert_eq!(IDLE_TICKS, 120);
    }

    #[test]
    fn node_names_are_validated_before_they_reach_a_path() {
        for ok in ["n1", "N2", "abc123"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "../x", "n1/", "a.b", "n 1", "abcdefghijklmnopq"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn key_index_accepts_only_k0_to_k7() {
        assert_eq!(key_index("k0"), Some(0));
        assert_eq!(key_index("k7"), Some(7));
        for bad in ["k8", "k", "k00", "x1", ""] {
            assert_eq!(key_index(bad), None, "{bad}");
        }
    }

    #[test]
    fn put_get_and_replication() {
        let mut s = Server::new("n1");
        let a = s.handle("PUT k3 7", true);
        assert_eq!(a.reply.as_deref(), Some("OK\n"));
        assert_eq!(a.replicate.as_deref(), Some("REPL k3 7\n"));
        assert_eq!(s.handle("PUT k4 8", false).replicate, None);
        assert_eq!(s.handle("GET k3", true).reply.as_deref(), Some("VAL 7\n"));
        assert_eq!(s.handle("GET k5", true).reply.as_deref(), Some("NIL\n"));
        assert_eq!(s.handle("REPL k5 9", true), Action::default());
    }

    #[test]
    fn bad_requests_get_err_and_quit_is_recognised() {
        let mut s = Server::new("n1");
        for bad in ["BOGUS", "PUT k9 1", "PUT k1 x", "GET k9"] {
            assert_eq!(s.handle(bad, true).reply.as_deref(), Some("ERR\n"), "{bad}");
        }
        assert_eq!(s.handle("HELLO n2", true), Action::default());
        assert!(s.handle("QUIT", true).quit);
    }

    #[test]
    fn result_line_has_the_agreed_shape() {
        let mut s = Server::new("n2");
        s.handle("PUT k3 7", true);
        s.handle("REPL k5 9", true);
        let line = s.result_line();
        let want = "DSN0 v1 role=srv node=n2 mode=rust applied=1 repl_in=1 repl_out=1 refused=0 hb=0 suspect=0 \
                    clock=0000000000000000 state=";
        assert!(line.starts_with(want), "{line}");
        assert_eq!(line.len(), want.len() + 17);
    }
}

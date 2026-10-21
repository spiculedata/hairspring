//! Helpers shared by the integration tests: run a sample program and look at the lines it wrote.
#![allow(dead_code)] // each test file uses a different subset

use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output};

pub const PREFIX: &str = "HAIRSPRING_EVENT ";
pub const LINE_MAX: usize = 4096;

pub struct Run {
    pub stderr: String,
    pub stdout: String,
    pub status: ExitStatus,
}

/// Run `exe` with the harness-owned variables cleared, then the given extra ones.
pub fn run(exe: &str, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .env_remove("HAIRSPRING_PROC")
        .env_remove("HAIRSPRING_PROC_PID")
        .env_remove("HAIRSPRING_EVENTS")
        .env_remove("HAIRSPRING_EVENTS_FILE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out: Output = cmd.output().expect("sample program runs");
    Run {
        stderr: String::from_utf8(out.stderr).expect("utf-8 stderr"),
        stdout: String::from_utf8(out.stdout).expect("utf-8 stdout"),
        status: out.status,
    }
}

/// The protocol lines in `text`, without their prefix.
pub fn events(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|l| l.strip_prefix(PREFIX))
        .collect()
}

/// The lines of one kind.
pub fn of_kind<'a>(events: &[&'a str], kind: &str) -> Vec<&'a str> {
    let needle = format!("\"kind\":\"{kind}\"");
    events
        .iter()
        .copied()
        .filter(|l| l.contains(&needle))
        .collect()
}

/// The unsigned integer after `"<key>":` in `line`, ignoring padding.
pub fn number(line: &str, key: &str) -> u64 {
    let needle = format!("\"{key}\":");
    let at = line
        .find(&needle)
        .unwrap_or_else(|| panic!("no {key} in {line}"))
        + needle.len();
    let digits: String = line[at..]
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no number for {key} in {line}"))
}

/// A scratch path that does not exist yet, unique to this process and `name`.
pub fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("hs_rust_sdk_{}_{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

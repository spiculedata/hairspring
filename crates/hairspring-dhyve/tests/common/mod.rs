//! A rig for the conformance tests: the fake VM programs, started through symlinks named for the programs they stand in
//! for, in a short directory (a Unix socket path must be short), with scripts and records as plain files.
#![allow(dead_code)]
use hairspring_dhyve::host::{HostConfig, Timings};
use hairspring_dhyve::logs::Bounds;
use hairspring_dhyve::session::Session;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static NEXT: AtomicU32 = AtomicU32::new(0);
const FAKE: &str = env!("CARGO_BIN_EXE_dhyve-fake-vmm");

pub struct Rig {
    pub root: PathBuf,
    pub fake: PathBuf,
    pub host: HostConfig,
}

/// A name the first prepare of a fresh rig gets.
pub const FIRST_NAME: &str = "vmvx_00000000T0001Z_XB_r01";
/// The stamp the first prepare of a fresh rig gets.
pub const FIRST_STAMP: &str = "00000000T0001Z";

pub fn tiny_bounds() -> Bounds {
    Bounds {
        run_log_bytes: 256 * 1024,
        uart_log_bytes: 64 * 1024,
        min_free_kib: 1,
    }
}

pub fn quick() -> Timings {
    Timings {
        connect_wait: Duration::from_millis(600),
        prepare_cap: Duration::from_secs(15),
        trial_cap: Duration::from_secs(8),
        ..Timings::fast()
    }
}

impl Rig {
    pub fn new(name: &str) -> Rig {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let root = PathBuf::from(format!("/tmp/hsfk-{}-{n}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fake = root.join("fake");
        let lib = root.join("lib");
        for dir in [&fake, &lib, &root.join("img")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for program in ["bhyve", "bhyvectl", "sudo", "kldstat", "cpuset"] {
            std::os::unix::fs::symlink(FAKE, fake.join(program)).unwrap();
        }
        std::fs::write(fake.join("lib"), lib.display().to_string()).unwrap();
        std::fs::write(root.join("img/kernel"), b"kernel").unwrap();
        std::fs::write(root.join("img/initrd"), b"initrd").unwrap();
        let host = HostConfig {
            sudo: fake.join("sudo"),
            kldstat: fake.join("kldstat"),
            cpuset: fake.join("cpuset"),
            // The stand-ins are links to one program.
            require_regular_files: false,
            ..HostConfig::new(
                fake.join("bhyve"),
                fake.join("bhyvectl"),
                lib,
                root.join("st"),
                root.join("img/kernel"),
                root.join("img/initrd"),
            )
        };
        Rig { root, fake, host }
    }

    pub fn session(&self) -> Session {
        self.session_with(quick())
    }

    pub fn session_with(&self, timings: Timings) -> Session {
        Session::new(self.host.clone(), timings, tiny_bounds())
    }

    fn put(&self, file: &str, lines: &[&str]) {
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(self.fake.join(file), text).unwrap();
    }

    /// One line per start of the fake `bhyve`; the first is the prepare.
    pub fn modes(&self, lines: &[&str]) {
        self.put("modes", lines);
    }

    pub fn ctl(&self, lines: &[&str]) {
        self.put("ctl", lines);
    }

    pub fn events(&self, lines: &[&str]) {
        self.put("events", lines);
    }

    pub fn expect_lib(&self, value: &str) {
        std::fs::write(self.fake.join("lib"), value).unwrap();
    }

    pub fn sudo_mode(&self, mode: &str) {
        self.put("sudo-mode", &[mode]);
    }

    pub fn seed_vm(&self, name: &str) {
        self.put("vms", &[name]);
    }

    fn read_lines(&self, file: &str) -> Vec<String> {
        std::fs::read_to_string(self.fake.join(file))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    pub fn calls(&self) -> Vec<String> {
        self.read_lines("calls.log")
    }

    /// The arguments of each `bhyve` start, with this rig's directory masked.
    pub fn start_args(&self) -> Vec<String> {
        let root = self.root.display().to_string();
        self.read_lines("args.log")
            .into_iter()
            .map(|l| l.replace(&root, "ROOT").replace('\u{1f}', " "))
            .collect()
    }

    /// The arguments of each `bhyve` start as separate words, with this rig's directory masked.
    pub fn start_words(&self) -> Vec<Vec<String>> {
        let root = self.root.display().to_string();
        self.read_lines("args.log")
            .into_iter()
            .map(|l| {
                l.replace(&root, "ROOT")
                    .split('\u{1f}')
                    .map(String::from)
                    .collect()
            })
            .collect()
    }

    /// The six input files each guest read, one line for each.
    pub fn inputs(&self) -> Vec<String> {
        self.read_lines("inputs.log")
    }

    /// Forgets what the fake recorded and what the state directory holds, to run another session in the same rig.
    pub fn reset(&self) {
        for f in [
            "calls.log",
            "args.log",
            "trials.log",
            "inputs.log",
            "vms",
            "bhyve.count",
            "kld.count",
        ] {
            let _ = std::fs::remove_file(self.fake.join(f));
        }
        let _ = std::fs::remove_dir_all(&self.host.state_dir);
    }

    pub fn trials(&self) -> Vec<String> {
        self.read_lines("trials.log")
    }

    pub fn vms(&self) -> Vec<String> {
        self.read_lines("vms")
    }

    /// Files and sockets left under the state directory with the given extension.
    pub fn left(&self, extension: &str) -> Vec<PathBuf> {
        fn walk(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, extension, out);
                } else if path.extension().is_some_and(|e| e == extension) {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.host.state_dir, extension, &mut out);
        out
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

//! The fake's state: a directory next to the symlinks the fake is started through. Tests write scripts into it before a
//! run and read what the fake recorded after.
//!
//! * `modes`      one line per `bhyve` start, a list of words naming what that start does (see `bhyve.rs`); the last line
//!   repeats; no file or an empty line means a normal start.
//! * `lib`        the value `LD_LIBRARY_PATH` must have, and the only variable the environment may hold.
//! * `vms`        the names of the VMs that exist, one per line.
//! * `calls.log`  one line per program started, in order.
//! * `trials.log` one line per guest run: the seed it read.
//! * `events`     lines the guest prints before its exit report (`{seed}` is replaced).
//! * `ctl`        `key=value` lines for the checkpoint and destroy commands.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Fake {
    pub dir: PathBuf,
}

impl Fake {
    /// The fake next to the program named by `argv0` (the symlink it was started through).
    pub fn from_argv0(argv0: &str) -> Fake {
        let dir = Path::new(argv0)
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Fake { dir }
    }

    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.dir.join(file)).unwrap_or_default()
    }

    pub fn append(&self, file: &str, line: &str) {
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(file))
        {
            let _ = writeln!(f, "{line}");
        }
    }

    pub fn log_call(&self, line: &str) {
        self.append("calls.log", line);
    }

    /// The words of the line for this start of `bhyve`, counting starts from zero in the `bhyve.count` file.
    pub fn mode_words(&self) -> Vec<String> {
        let n: usize = self.read("bhyve.count").trim().parse().unwrap_or(0);
        let _ = std::fs::write(self.dir.join("bhyve.count"), (n + 1).to_string());
        let text = self.read("modes");
        let lines: Vec<&str> = text.lines().collect();
        let line = lines.get(n).or(lines.last()).copied().unwrap_or("");
        line.split_whitespace().map(String::from).collect()
    }

    pub fn expected_lib(&self) -> Option<String> {
        let text = self.read("lib");
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.read("ctl").lines().find_map(|l| {
            l.strip_prefix(&format!("{key}="))
                .map(|v| v.trim().to_string())
        })
    }

    pub fn events(&self, seed: u64) -> Vec<String> {
        self.read("events")
            .lines()
            .map(|l| l.replace("{seed}", &seed.to_string()))
            .collect()
    }

    pub fn registry(&self) -> Vec<String> {
        self.read("vms").lines().map(String::from).collect()
    }

    fn write_registry(&self, names: &[String]) {
        let mut text = names.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        let _ = std::fs::write(self.dir.join("vms"), text);
    }

    /// Adds `name`; true when it already existed (a start of a name that exists attaches to it).
    pub fn register(&self, name: &str) -> bool {
        let mut names = self.registry();
        let existed = names.iter().any(|n| n == name);
        if !existed {
            names.push(name.to_string());
            self.write_registry(&names);
        }
        existed
    }

    /// Removes `name`; true when it was there.
    pub fn unregister(&self, name: &str) -> bool {
        let names = self.registry();
        let kept: Vec<String> = names.iter().filter(|n| *n != name).cloned().collect();
        let was_there = kept.len() != names.len();
        if was_there {
            self.write_registry(&kept);
        }
        was_there
    }

    pub fn exists(&self, name: &str) -> bool {
        self.registry().iter().any(|n| n == name)
    }
}

/// Whether the environment is exactly the one variable the root commands are given.
pub fn environment_ok(fake: &Fake) -> Result<(), String> {
    // A build instrumented for coverage adds one variable of its own at start-up; it is not the caller's.
    let vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !k.starts_with("__LLVM_PROFILE"))
        .collect();
    let want = fake.expected_lib();
    match (vars.as_slice(), want) {
        ([(k, v)], Some(want)) if k == "LD_LIBRARY_PATH" && *v == want => Ok(()),
        (_, None) if vars.is_empty() => Ok(()),
        _ => Err(format!(
            "unexpected environment: {}",
            vars.iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )),
    }
}

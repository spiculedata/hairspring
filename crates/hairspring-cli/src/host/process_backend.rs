//! `hairspring doctor --process-backend`: what the `process` backend needs from this machine, in one report.
//!
//! The facts are read once (see `process_probe`) and judged here, so every verdict can be tested without the hardware.
//! A blocker means the backend cannot run here; a note is worth knowing but does not stop it. Nothing here changes the
//! host: when a kernel setting is in the way, the report names it and leaves the decision to the operator.
use super::kernel;
use super::LoaderHashes;
use std::io::Write;

/// The only platform Hermit runs on.
const SUPPORTED_OS: &str = "linux";
const SUPPORTED_ARCH: &str = "x86_64";
/// `kernel.yama.ptrace_scope` values that matter: 2 needs `CAP_SYS_PTRACE`, 3 turns tracing off until reboot.
const PTRACE_ADMIN_ONLY: i32 = 2;
const PTRACE_DISABLED: i32 = 3;

/// What was found for the Hermit executable, resolved the way the backend resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HermitFact {
    /// Found, and `--version` answered.
    Found {
        path: String,
        version: String,
        sha256: Option<String>,
    },
    /// Resolved to `path`, but it did not answer `--version` in time (missing file, not executable, or broken).
    NotRunnable { path: String },
    /// Not named by `HAIRSPRING_HERMIT` and not on `PATH`. `installed_at` is the default install prefix's copy, if any.
    Missing { installed_at: Option<String> },
}

/// Whether a hardware branch counter could be opened for the calling process. Only Linux on x86-64 probes, so the
/// other variants are never built elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PerfFact {
    /// `perf_event_open` for a user-space hardware counter succeeded.
    Usable,
    /// The kernel refused (EACCES or EPERM): `perf_event_paranoid`, a container profile or a security module.
    Denied,
    /// No hardware counters here (no PMU exposed, or the call is not implemented).
    NoCounters,
    /// Some other failure, with its errno.
    Failed(i32),
    /// Not probed: this is not Linux on x86-64.
    NotProbed,
}

/// Everything the report looks at, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessBackendFacts {
    pub os: String,
    pub arch: String,
    pub kernel_release: Option<String>,
    pub hermit: HermitFact,
    pub perf: PerfFact,
    pub perf_event_paranoid: Option<i32>,
    /// `None` when Yama is not present (no restriction from it).
    pub ptrace_scope: Option<i32>,
    pub is_root: bool,
    /// `docker`, `podman`, or `None`.
    pub container: Option<String>,
    /// The first line of `ldd --version`, which names the C library.
    pub libc: Option<String>,
    pub loader: LoaderHashes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Fail,
    Note,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Fail => "FAIL",
            Status::Note => "note",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub status: Status,
    pub detail: String,
}

/// The judged report: one row per prerequisite, and the reasons the backend cannot run (empty when it can).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub rows: Vec<Row>,
    pub blockers: Vec<String>,
}

impl Report {
    pub fn can_run(&self) -> bool {
        self.blockers.is_empty()
    }
}

fn row(label: &'static str, status: Status, detail: impl Into<String>) -> Row {
    Row {
        label,
        status,
        detail: detail.into(),
    }
}

/// One judged prerequisite: its row, and the reason it blocks the backend (when it does).
struct Judged {
    row: Row,
    blocker: Option<String>,
}

fn pass(label: &'static str, detail: impl Into<String>) -> Judged {
    Judged {
        row: row(label, Status::Ok, detail),
        blocker: None,
    }
}

fn note(label: &'static str, detail: impl Into<String>) -> Judged {
    Judged {
        row: row(label, Status::Note, detail),
        blocker: None,
    }
}

fn fail(label: &'static str, detail: impl Into<String>, blocker: impl Into<String>) -> Judged {
    Judged {
        row: row(label, Status::Fail, detail),
        blocker: Some(blocker.into()),
    }
}

fn judge_platform(f: &ProcessBackendFacts) -> Judged {
    let here = format!("{} {}", f.os, f.arch);
    if f.os != SUPPORTED_OS {
        return fail(
            "platform",
            here,
            "the process backend needs Linux: Hermit does not run on this operating system. The sim and wasm backends \
             work here.",
        );
    }
    if f.arch != SUPPORTED_ARCH {
        return fail(
            "platform",
            here,
            format!(
                "the process backend needs Linux on x86-64; this machine is {}. The sim and wasm backends work here.",
                f.arch
            ),
        );
    }
    pass("platform", here)
}

fn judge_kernel(f: &ProcessBackendFacts) -> Judged {
    let Some(release) = f.kernel_release.as_deref() else {
        return note("kernel", "release could not be read");
    };
    match kernel::warning(release) {
        Some(warning) => note("kernel", warning),
        None => pass("kernel", release.trim()),
    }
}

fn judge_hermit(f: &ProcessBackendFacts) -> Judged {
    match &f.hermit {
        HermitFact::Found {
            path,
            version,
            sha256,
        } => {
            let hash = sha256
                .as_deref()
                .map_or("sha256 unreadable".to_string(), |h| format!("sha256 {h}"));
            pass("hermit", format!("{path}: {version}, {hash}"))
        }
        HermitFact::NotRunnable { path } => fail(
            "hermit",
            format!("{path} did not answer --version"),
            format!(
                "Hermit at {path} did not answer `--version`: the file is missing, not executable, or broken. Rebuild it \
                 with scripts/install-hermit.sh."
            ),
        ),
        HermitFact::Missing { installed_at } => {
            let blocker = match installed_at {
                Some(path) => format!(
                    "Hermit was not found: HAIRSPRING_HERMIT is unset and `hermit` is not on PATH. A copy exists at \
                     {path}; use it with: export HAIRSPRING_HERMIT={path}"
                ),
                None => "Hermit was not found: HAIRSPRING_HERMIT is unset and `hermit` is not on PATH. Build it with \
                         scripts/install-hermit.sh and point HAIRSPRING_HERMIT at the result."
                    .to_string(),
            };
            fail("hermit", "not found", blocker)
        }
    }
}

fn judge_perf(f: &ProcessBackendFacts) -> Judged {
    let paranoid = f
        .perf_event_paranoid
        .map_or("unknown".to_string(), |p| p.to_string());
    match f.perf {
        PerfFact::Usable => pass(
            "perf counters",
            format!("a hardware branch counter opened (perf_event_paranoid {paranoid})"),
        ),
        PerfFact::Denied => fail(
            "perf counters",
            format!("opening a hardware counter was refused (perf_event_paranoid {paranoid})"),
            format!(
                "the kernel refused a hardware performance counter (perf_event_paranoid is {paranoid}; a container \
                 profile or security module can also do this). Hairspring never changes kernel settings: if this is \
                 your machine, lowering the setting is your decision, see docs/quickstart.md."
            ),
        ),
        PerfFact::NoCounters => fail(
            "perf counters",
            "no hardware counters available",
            "this machine exposes no hardware performance counters (PMU): common in virtual machines and containers. \
             Hermit needs them; use bare metal or a VM that passes the PMU through.",
        ),
        PerfFact::Failed(errno) => fail(
            "perf counters",
            format!("opening a hardware counter failed (errno {errno})"),
            format!("opening a hardware performance counter failed with errno {errno}"),
        ),
        PerfFact::NotProbed => fail(
            "perf counters",
            "not checked: no hardware counter was opened",
            "no hardware performance counter was opened on this machine (the probe did not run, which happens off \
             Linux x86-64), so the process backend cannot be said to be runnable.",
        ),
    }
}

fn judge_ptrace(f: &ProcessBackendFacts) -> Judged {
    let Some(scope) = f.ptrace_scope else {
        return pass("ptrace", "no Yama restriction");
    };
    let detail = format!("kernel.yama.ptrace_scope = {scope}");
    let admin_only_and_not_root = scope == PTRACE_ADMIN_ONLY && !f.is_root;
    if scope >= PTRACE_DISABLED || admin_only_and_not_root {
        return fail(
            "ptrace",
            detail,
            format!(
                "kernel.yama.ptrace_scope is {scope}, which stops Hermit from tracing the program{}. Hairspring never \
                 changes this setting.",
                if scope >= PTRACE_DISABLED {
                    " (tracing is disabled until reboot)"
                } else {
                    " unless it runs as root"
                }
            ),
        );
    }
    pass("ptrace", detail)
}

fn judge_container(f: &ProcessBackendFacts) -> Option<Judged> {
    f.container.as_deref().map(|kind| {
        note(
            "container",
            format!("running inside {kind}: counters and ptrace are often blocked here"),
        )
    })
}

fn judge_loader(f: &ProcessBackendFacts) -> Judged {
    let libc = f.libc.as_deref().unwrap_or("C library version unknown");
    let hashes = f
        .loader
        .describe()
        .map(|line| line.trim_start_matches("loader state: ").to_string())
        .unwrap_or_else(|| "no loader files could be read".to_string());
    pass("loader / libc", format!("{libc}; {hashes}"))
}

pub fn assess(facts: &ProcessBackendFacts) -> Report {
    let judged = [
        Some(judge_platform(facts)),
        Some(judge_kernel(facts)),
        Some(judge_hermit(facts)),
        Some(judge_perf(facts)),
        Some(judge_ptrace(facts)),
        judge_container(facts),
        Some(judge_loader(facts)),
    ];
    let judged: Vec<Judged> = judged.into_iter().flatten().collect();
    let blockers = judged.iter().filter_map(|j| j.blocker.clone()).collect();
    let rows = judged.into_iter().map(|j| j.row).collect();
    Report { rows, blockers }
}

/// Writes the report. The last line is `result: ok: ...` or `result: CANNOT RUN ...` followed by one line per reason.
pub fn write_report(out: &mut dyn Write, report: &Report) -> std::io::Result<()> {
    writeln!(out, "process backend: prerequisites on this machine")?;
    for r in &report.rows {
        writeln!(
            out,
            "  {:<14} {:<4}  {}",
            r.label,
            r.status.word(),
            r.detail
        )?;
    }
    if report.can_run() {
        return writeln!(
            out,
            "result: ok: a generic hardware branch counter was opened and Hermit answered --version; no program has been \
             run under Hermit, so this does not show that Hermit works here"
        );
    }
    writeln!(
        out,
        "result: CANNOT RUN the process backend on this machine:"
    )?;
    report
        .blockers
        .iter()
        .try_for_each(|b| writeln!(out, "  - {b}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a";

    fn good() -> ProcessBackendFacts {
        ProcessBackendFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            kernel_release: Some("6.17.0-42-generic".into()),
            hermit: HermitFact::Found {
                path: "/opt/hermit/bin/hermit".into(),
                version: "hermit 0.4.1 (2026-10-15)".into(),
                sha256: Some(HASH.into()),
            },
            perf: PerfFact::Usable,
            perf_event_paranoid: Some(2),
            ptrace_scope: Some(1),
            is_root: false,
            container: None,
            libc: Some("ldd (Ubuntu GLIBC 2.39-0ubuntu8.3) 2.39".into()),
            loader: LoaderHashes {
                cache: Some("c0ffee".into()),
                libc: Some("1bc".into()),
                ld_so: Some("d1d".into()),
            },
        }
    }

    fn text(facts: &ProcessBackendFacts) -> String {
        let mut out = Vec::new();
        write_report(&mut out, &assess(facts)).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn blockers(facts: &ProcessBackendFacts) -> Vec<String> {
        assess(facts).blockers
    }

    fn only_blocker(facts: &ProcessBackendFacts) -> String {
        let found = blockers(facts);
        assert_eq!(found.len(), 1, "{found:?}");
        found[0].clone()
    }

    #[test]
    fn a_machine_with_every_prerequisite_is_ok() {
        let report = assess(&good());
        assert!(report.can_run(), "{:?}", report.blockers);
        let text = text(&good());
        assert!(text.contains("result: ok:"), "{text}");
        assert!(!text.contains("CANNOT RUN"), "{text}");
        assert!(!text.contains("FAIL"), "{text}");
    }

    #[test]
    fn the_report_names_hermit_its_version_and_its_hash() {
        let text = text(&good());
        assert!(text.contains("/opt/hermit/bin/hermit"), "{text}");
        assert!(text.contains("hermit 0.4.1 (2026-10-15)"), "{text}");
        assert!(text.contains(&format!("sha256 {HASH}")), "{text}");
    }

    #[test]
    fn the_report_names_the_loader_libc_and_kernel() {
        let text = text(&good());
        assert!(text.contains("6.17.0-42-generic"), "{text}");
        assert!(text.contains("GLIBC 2.39"), "{text}");
        assert!(text.contains("ld.so.cache sha256 c0ffee"), "{text}");
        assert!(text.contains("libc sha256 1bc"), "{text}");
    }

    #[test]
    fn macos_cannot_run_the_process_backend() {
        let facts = ProcessBackendFacts {
            os: "macos".into(),
            arch: "aarch64".into(),
            perf: PerfFact::NotProbed,
            ..good()
        };
        let reasons = blockers(&facts);
        assert!(reasons.iter().any(|r| r.contains("Linux")), "{reasons:?}");
        assert!(text(&facts).contains("result: CANNOT RUN"));
    }

    #[test]
    fn a_counter_that_was_never_probed_blocks_even_when_the_platform_is_fine() {
        let facts = ProcessBackendFacts {
            perf: PerfFact::NotProbed,
            ..good()
        };
        let reason = only_blocker(&facts);
        assert!(reason.contains("performance counter"), "{reason}");
        let text = text(&facts);
        assert!(text.contains("result: CANNOT RUN"), "{text}");
        assert!(!text.contains("result: ok"), "{text}");
    }

    #[test]
    fn the_ok_line_says_what_was_and_was_not_checked() {
        let text = text(&good());
        assert!(
            text.contains("generic hardware branch counter was opened"),
            "{text}"
        );
        assert!(text.contains("--version"), "{text}");
        assert!(
            text.contains("no program has been run under Hermit"),
            "{text}"
        );
    }

    #[test]
    fn linux_on_another_architecture_cannot_run_it() {
        let facts = ProcessBackendFacts {
            arch: "aarch64".into(),
            perf: PerfFact::NotProbed,
            ..good()
        };
        let reasons = blockers(&facts);
        assert!(
            reasons
                .iter()
                .any(|r| r.contains("x86-64") && r.contains("aarch64")),
            "{reasons:?}"
        );
    }

    #[test]
    fn a_missing_hermit_blocks_and_points_at_the_installer() {
        let facts = ProcessBackendFacts {
            hermit: HermitFact::Missing { installed_at: None },
            ..good()
        };
        let reason = only_blocker(&facts);
        assert!(reason.contains("scripts/install-hermit.sh"), "{reason}");
        assert!(reason.contains("HAIRSPRING_HERMIT"), "{reason}");
    }

    #[test]
    fn a_hermit_in_the_install_prefix_that_the_backend_would_not_find_says_how_to_use_it() {
        let facts = ProcessBackendFacts {
            hermit: HermitFact::Missing {
                installed_at: Some("/home/u/.local/share/hairspring/hermit/bin/hermit".into()),
            },
            ..good()
        };
        let reason = only_blocker(&facts);
        assert!(
            reason.contains(
                "export HAIRSPRING_HERMIT=/home/u/.local/share/hairspring/hermit/bin/hermit"
            ),
            "{reason}"
        );
    }

    #[test]
    fn a_hermit_that_does_not_answer_blocks() {
        let facts = ProcessBackendFacts {
            hermit: HermitFact::NotRunnable {
                path: "/nope/hermit".into(),
            },
            ..good()
        };
        let reason = only_blocker(&facts);
        assert!(
            reason.contains("/nope/hermit") && reason.contains("--version"),
            "{reason}"
        );
    }

    #[test]
    fn counters_that_cannot_be_opened_block_and_say_why() {
        let denied = ProcessBackendFacts {
            perf: PerfFact::Denied,
            perf_event_paranoid: Some(3),
            ..good()
        };
        let reason = only_blocker(&denied);
        assert!(
            reason.contains("perf_event_paranoid") && reason.contains('3'),
            "{reason}"
        );
        assert!(reason.contains("never changes"), "{reason}");

        let none = ProcessBackendFacts {
            perf: PerfFact::NoCounters,
            ..good()
        };
        let reason = only_blocker(&none);
        assert!(
            reason.contains("hardware") && reason.contains("PMU"),
            "{reason}"
        );

        let other = ProcessBackendFacts {
            perf: PerfFact::Failed(24),
            ..good()
        };
        assert!(only_blocker(&other).contains("errno 24"));
    }

    #[test]
    fn ptrace_scope_two_needs_root_and_three_is_never_enough() {
        let admin_only = ProcessBackendFacts {
            ptrace_scope: Some(2),
            ..good()
        };
        assert!(only_blocker(&admin_only).contains("ptrace_scope"));
        let as_root = ProcessBackendFacts {
            ptrace_scope: Some(2),
            is_root: true,
            ..good()
        };
        assert!(blockers(&as_root).is_empty());
        let disabled = ProcessBackendFacts {
            ptrace_scope: Some(3),
            is_root: true,
            ..good()
        };
        assert!(only_blocker(&disabled).contains("ptrace_scope"));
    }

    #[test]
    fn ptrace_scope_zero_one_or_absent_is_fine() {
        for scope in [Some(0), Some(1), None] {
            let facts = ProcessBackendFacts {
                ptrace_scope: scope,
                ..good()
            };
            assert!(blockers(&facts).is_empty(), "{scope:?}");
        }
    }

    #[test]
    fn an_old_kernel_is_a_note_and_not_a_blocker() {
        let facts = ProcessBackendFacts {
            kernel_release: Some("6.8.0-146-generic".into()),
            ..good()
        };
        assert!(blockers(&facts).is_empty());
        let text = text(&facts);
        assert!(
            text.contains("note") && text.contains("older than 6.9"),
            "{text}"
        );
        assert!(text.contains("result: ok:"), "{text}");
    }

    #[test]
    fn running_in_a_container_is_a_note_and_not_a_blocker() {
        let facts = ProcessBackendFacts {
            container: Some("docker".into()),
            ..good()
        };
        assert!(blockers(&facts).is_empty());
        assert!(text(&facts).contains("docker"));
    }

    #[test]
    fn every_blocker_is_listed_under_a_cannot_run_result() {
        let facts = ProcessBackendFacts {
            hermit: HermitFact::Missing { installed_at: None },
            perf: PerfFact::NoCounters,
            ..good()
        };
        let text = text(&facts);
        let result = text.find("result: CANNOT RUN").expect("cannot-run line");
        assert!(text[result..].matches("\n  - ").count() == 2, "{text}");
        assert!(!text.contains("result: ok"), "{text}");
    }

    #[test]
    fn a_missing_hash_is_reported_as_unread_not_invented() {
        let facts = ProcessBackendFacts {
            hermit: HermitFact::Found {
                path: "/h".into(),
                version: "hermit 1".into(),
                sha256: None,
            },
            ..good()
        };
        assert!(text(&facts).contains("sha256 unreadable"));
    }
}

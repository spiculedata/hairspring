//! Judges what the machine check read. Pure: facts in, report out, so every row is testable on any operating system.
//!
//! A row that is `Fail` means the backend cannot run here, and says how to fix it. A `Note` is worth knowing and does
//! not block. Nothing here changes the host: the report names a missing kernel module and leaves loading it to an
//! operator.
use crate::facts::{BinaryFact, CpuFact, DhyveFacts, ModuleFact};
use hairspring_engine::probe::{ProbeReport, ProbeRow, ProbeStatus};

/// What the report is about.
pub const SUBJECT: &str = "dhyve backend (experimental)";
/// The only platform the backend is meant for.
const SUPPORTED_OS: &str = "freebsd";
const SUPPORTED_ARCH: &str = "x86_64";
/// Said when every prerequisite is met, including what the probe did not show.
const READY_NOTE: &str =
    "prerequisites met; this backend is experimental and its results are unproven. No virtual \
     machine was started, so this does not show that dhyve runs correctly here";

/// One judged prerequisite: its row, and the reason it blocks the backend (when it does).
struct Judged {
    row: ProbeRow,
    blocker: Option<String>,
}

fn judged(
    label: &str,
    status: ProbeStatus,
    detail: impl Into<String>,
    blocker: Option<String>,
) -> Judged {
    Judged {
        row: ProbeRow {
            label: label.to_string(),
            status,
            detail: detail.into(),
        },
        blocker,
    }
}

fn pass(label: &str, detail: impl Into<String>) -> Judged {
    judged(label, ProbeStatus::Ok, detail, None)
}

fn note(label: &str, detail: impl Into<String>) -> Judged {
    judged(label, ProbeStatus::Note, detail, None)
}

fn fail(label: &str, detail: impl Into<String>, blocker: impl Into<String>) -> Judged {
    judged(label, ProbeStatus::Fail, detail, Some(blocker.into()))
}

/// Judges the facts into the report every backend's probe answers in.
pub fn assess(facts: &DhyveFacts) -> ProbeReport {
    if facts.os != SUPPORTED_OS || facts.arch != SUPPORTED_ARCH {
        return unsupported_host(facts);
    }
    let judged = vec![
        judge_os(facts),
        judge_cpu(facts),
        judge_module(facts),
        note(
            "module kind",
            "not checked: this build has no way to tell dhyve's module from the stock one",
        ),
        judge_binary("bhyve", "HAIRSPRING_DHYVE_BHYVE", &facts.bhyve),
        judge_binary("bhyvectl", "HAIRSPRING_DHYVE_BHYVECTL", &facts.bhyvectl),
        note(
            "access",
            "not checked: whether this user may create virtual machines is not read yet",
        ),
        judge_leftovers(facts),
        note(
            "space",
            "not checked: no checkpoint directory is used by this build",
        ),
        judge_nmi(facts),
    ];
    let missing = judged.iter().filter_map(|j| j.blocker.clone()).collect();
    let rows = judged.into_iter().map(|j| j.row).collect();
    ProbeReport {
        subject: SUBJECT.to_string(),
        rows,
        ready_note: READY_NOTE.to_string(),
        missing,
    }
}

/// Off FreeBSD on x86-64 nothing else is looked at: one row and one missing entry.
fn unsupported_host(facts: &DhyveFacts) -> ProbeReport {
    let here = format!("{}/{}", facts.os, facts.arch);
    let missing = format!(
        "the dhyve backend runs only on FreeBSD 15 on x86-64 with Intel VT-x and EPT, and only in builds with the \
         `dhyve` feature. This machine is {here}."
    );
    ProbeReport {
        subject: SUBJECT.to_string(),
        rows: vec![fail("os", here, "").row],
        ready_note: String::new(),
        missing: vec![missing],
    }
}

fn judge_os(f: &DhyveFacts) -> Judged {
    let here = format!("{}/{}", f.os, f.arch);
    match f.os_release.as_deref() {
        Some(release) => pass("os", format!("{here} {release}")),
        None => note("os", format!("{here}, release could not be read")),
    }
}

fn judge_cpu(f: &DhyveFacts) -> Judged {
    match f.cpu {
        CpuFact::Ept => pass("cpu", "VT-x with EPT"),
        CpuFact::NoEpt => fail(
            "cpu",
            "VT-x feature line has no EPT",
            "VT-x with EPT not found; dhyve needs Intel VT-x with EPT",
        ),
        CpuFact::Unknown => note(
            "cpu",
            "no VT-x feature line could be read from the boot messages",
        ),
    }
}

fn judge_module(f: &DhyveFacts) -> Judged {
    match &f.module {
        ModuleFact::Loaded => pass("module", "a vmm module is loaded"),
        ModuleFact::NotLoaded => fail(
            "module",
            "no vmm module loaded",
            "no vmm module is loaded. An operator loads the pinned dhyve module (`kldload <path>`); Hairspring never \
             loads one.",
        ),
        ModuleFact::Unanswered(why) => fail(
            "module",
            "not answered",
            format!("whether a vmm module is loaded could not be found out: {why}. Hairspring never loads one."),
        ),
    }
}

fn judge_binary(label: &str, variable: &str, fact: &BinaryFact) -> Judged {
    match fact {
        BinaryFact::Found { path, sha256 } => {
            let hash = sha256
                .as_deref()
                .map_or("sha256 unreadable".to_string(), |h| format!("sha256 {h}"));
            pass(label, format!("{path}, {hash}"))
        }
        BinaryFact::Unusable { path } => fail(
            label,
            format!("{path} is not a readable file"),
            format!("{label} not found at {path}: it is not a regular file. Set {variable} to the pinned program."),
        ),
        BinaryFact::NotConfigured => fail(
            label,
            "no path configured",
            format!("no path for {label} is configured: set {variable} to the pinned program."),
        ),
    }
}

fn judge_leftovers(f: &DhyveFacts) -> Judged {
    match f.leftover_vms.as_deref() {
        Some([]) => pass(
            "leftovers",
            "no virtual machine from an earlier run is present",
        ),
        Some(names) => fail(
            "leftovers",
            names.join(", "),
            format!(
                "a virtual machine from an earlier run is still present: {}",
                names
                    .iter()
                    .map(|n| format!("`bhyvectl --destroy --vm={n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        None => note(
            "leftovers",
            "the list of virtual machines could not be read",
        ),
    }
}

fn judge_nmi(f: &DhyveFacts) -> Judged {
    match f.panic_on_nmi {
        Some(0) => pass("nmi", "machdep.panic_on_nmi = 0"),
        Some(v) => note(
            "nmi",
            format!("machdep.panic_on_nmi = {v}: a stray NMI may panic this host"),
        ),
        None => note("nmi", "machdep.panic_on_nmi could not be read"),
    }
}

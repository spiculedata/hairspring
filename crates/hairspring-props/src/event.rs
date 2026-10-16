//! Typed events of protocol v1, as produced by [`crate::parse_line`].

/// The five assertion kinds and their trial/run semantics (see `docs/guest-protocol.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssertKind {
    Always,
    AlwaysOrUnreachable,
    Sometimes,
    Reachable,
    Unreachable,
}

impl AssertKind {
    pub const ALL: [AssertKind; 5] = [
        AssertKind::Always,
        AssertKind::AlwaysOrUnreachable,
        AssertKind::Sometimes,
        AssertKind::Reachable,
        AssertKind::Unreachable,
    ];

    /// The wire name of the kind.
    pub fn as_str(self) -> &'static str {
        match self {
            AssertKind::Always => "always",
            AssertKind::AlwaysOrUnreachable => "always_or_unreachable",
            AssertKind::Sometimes => "sometimes",
            AssertKind::Reachable => "reachable",
            AssertKind::Unreachable => "unreachable",
        }
    }

    /// Inverse of [`AssertKind::as_str`].
    pub fn from_wire(name: &str) -> Option<AssertKind> {
        AssertKind::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// Whether the run fails (UNMET) when the id is never satisfied.
    pub fn must_hit(self) -> bool {
        matches!(
            self,
            AssertKind::Always | AssertKind::Sometimes | AssertKind::Reachable
        )
    }
}

/// Source location of a call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loc {
    pub file: String,
    pub line: u64,
    pub func: Option<String>,
}

/// The first false or first true evaluation of an id in one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireWitness {
    pub thread: String,
    /// The thread's own ordinal for the evaluation (per-thread, not global).
    pub ord: u64,
    pub vals: [i64; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub proc: String,
    pub sdk: String,
    pub sdk_version: String,
    pub catalog_hash: String,
    pub signal_handlers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub proc: String,
    pub id: String,
    pub kind: AssertKind,
    pub must_hit: bool,
    pub msg: String,
    pub loc: Option<Loc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertReport {
    pub proc: String,
    pub id: String,
    pub hits: u64,
    pub trues: u64,
    pub falses: u64,
    pub first_false: Option<WireWitness>,
    pub first_true: Option<WireWitness>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    pub proc: String,
    pub ops: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Exit,
    Signal,
    Explicit,
}

impl EndReason {
    pub fn from_wire(name: &str) -> Option<EndReason> {
        match name {
            "exit" => Some(EndReason::Exit),
            "signal" => Some(EndReason::Signal),
            "explicit" => Some(EndReason::Explicit),
            _ => None,
        }
    }
}

/// Per-thread totals carried by `end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadReport {
    pub thread: String,
    pub evals: u64,
    pub tape_consumed: u64,
    /// False when the thread key was assigned by claim order and so may differ between schedules.
    pub key_stable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct End {
    pub proc: String,
    pub reason: EndReason,
    pub status: i64,
    pub sites: u64,
    pub evals: u64,
    pub dropped: u64,
    pub threads: Vec<ThreadReport>,
}

/// A per-evaluation event from a script or a language without an SDK. Marks the trial perturbing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eval {
    pub proc: String,
    pub id: String,
    pub kind: AssertKind,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Hello(Hello),
    Catalog(CatalogEntry),
    Assert(AssertReport),
    Work(Work),
    End(End),
    Eval(Eval),
}

/// Process tag used for `eval` lines that omit `proc`.
pub const DEFAULT_EVAL_PROC: &str = "p0";

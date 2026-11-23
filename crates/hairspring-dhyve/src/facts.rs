//! What the machine check reads, as plain values. Gathering them is the only part that touches the host (see
//! `gather`); judging them (see `assess`) is pure, so every verdict can be tested on any operating system.

/// Whether the CPU's virtualisation features include extended page tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuFact {
    /// A VT-x feature line was read and it names EPT.
    Ept,
    /// A VT-x feature line was read and it does not name EPT.
    NoEpt,
    /// No such line could be read.
    Unknown,
}

/// Whether a `vmm` kernel module is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleFact {
    Loaded,
    NotLoaded,
    /// The question was not answered in time, or the tool could not be started.
    Unanswered(String),
}

/// What was found for one configured program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryFact {
    /// A regular file. `sha256` is `None` when it could not be read.
    Found {
        path: String,
        sha256: Option<String>,
    },
    /// Configured, but not a regular file that can be read.
    Unusable { path: String },
    /// No path was configured.
    NotConfigured,
}

/// Everything the report looks at, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhyveFacts {
    pub os: String,
    pub arch: String,
    /// `kern.osrelease`, on FreeBSD only.
    pub os_release: Option<String>,
    pub cpu: CpuFact,
    pub module: ModuleFact,
    pub bhyve: BinaryFact,
    pub bhyvectl: BinaryFact,
    /// Names of VMs from an earlier run that are still present; `None` when the list could not be read.
    pub leftover_vms: Option<Vec<String>>,
    /// `machdep.panic_on_nmi`, when it could be read.
    pub panic_on_nmi: Option<i64>,
}

impl DhyveFacts {
    /// The facts of a machine where nothing was looked at beyond what it is. Used off FreeBSD, where no command runs.
    pub fn platform_only(os: &str, arch: &str) -> DhyveFacts {
        DhyveFacts {
            os: os.to_string(),
            arch: arch.to_string(),
            os_release: None,
            cpu: CpuFact::Unknown,
            module: ModuleFact::Unanswered("not checked".to_string()),
            bhyve: BinaryFact::NotConfigured,
            bhyvectl: BinaryFact::NotConfigured,
            leftover_vms: None,
            panic_on_nmi: None,
        }
    }
}

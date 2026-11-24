//! Infrastructure failures of the run path. These are errors of the host or the tooling, never findings about the program
//! under test: a run that meets one stops with an error, and no trial that met one is judged. The texts are fixed and
//! never carry an argument list or an option name.

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfraKind {
    HostBusy,
    Settings,
    Refused,
    StartFailed,
    NoConnect,
    NoReady,
    ControlLost,
    ResumeNotTaken,
    StoppedAgain,
    LogBound,
    LowSpace,
    SerialMissing,
    NoExitStatus,
    InputFiles,
    Checkpoint,
    ShutdownFailed,
    PrepareTimeout,
    FileOperation,
    VmDidNotExit,
    VmmNotLoaded,
    VmmChanged,
    SecondPrepare,
    PinMismatch,
    ModuleMismatch,
    CheckpointChanged,
}

impl InfraKind {
    pub fn text(self) -> &'static str {
        match self {
            InfraKind::HostBusy => "another run holds the host",
            InfraKind::Settings => "a required setting is missing or unusable",
            InfraKind::Refused => "a command was refused",
            InfraKind::StartFailed => "a program could not be started",
            InfraKind::NoConnect => "the VM did not connect to the control socket in time",
            InfraKind::NoReady => "the VM did not reach its first stop in time",
            InfraKind::ControlLost => "the control connection ended unexpectedly",
            InfraKind::ResumeNotTaken => "the resume command was not taken",
            InfraKind::StoppedAgain => "the VM stopped again after it was resumed",
            InfraKind::LogBound => "log size bound reached",
            InfraKind::LowSpace => "not enough free space for the logs",
            InfraKind::SerialMissing => "the serial log is missing or empty",
            InfraKind::NoExitStatus => "the VM ended without reporting an exit status",
            InfraKind::InputFiles => "the input files were not as written",
            InfraKind::Checkpoint => "the checkpoint could not be made or read",
            InfraKind::ShutdownFailed => "the VM did not end cleanly after the shutdown command",
            InfraKind::PrepareTimeout => "the prepare did not finish in time",
            InfraKind::FileOperation => "a file operation failed",
            InfraKind::VmDidNotExit => "the VM program did not exit",
            InfraKind::VmmNotLoaded => "no vmm module is loaded",
            InfraKind::VmmChanged => "the loaded vmm module is not the one seen at the start",
            InfraKind::SecondPrepare => "a second prepare was refused in this session",
            InfraKind::PinMismatch => "a program or a library does not match its pin",
            InfraKind::ModuleMismatch => "the checkpoint was not made by the expected module",
            InfraKind::CheckpointChanged => "the checkpoint changed after it was taken",
        }
    }
}

/// An infrastructure failure with a short detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfraError {
    pub kind: InfraKind,
    pub detail: String,
}

impl InfraError {
    pub fn new(kind: InfraKind, detail: impl Into<String>) -> InfraError {
        InfraError {
            kind,
            detail: detail.into(),
        }
    }

    pub fn bare(kind: InfraKind) -> InfraError {
        InfraError::new(kind, "")
    }

    pub fn io(what: &str, e: &std::io::Error) -> InfraError {
        InfraError::new(InfraKind::FileOperation, format!("{what}: {}", e.kind()))
    }
}

impl std::fmt::Display for InfraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.detail.is_empty() {
            f.write_str(self.kind.text())
        } else {
            write!(f, "{} ({})", self.kind.text(), self.detail)
        }
    }
}

impl std::error::Error for InfraError {}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [InfraKind; 25] = [
        InfraKind::HostBusy,
        InfraKind::Settings,
        InfraKind::Refused,
        InfraKind::StartFailed,
        InfraKind::NoConnect,
        InfraKind::NoReady,
        InfraKind::ControlLost,
        InfraKind::ResumeNotTaken,
        InfraKind::StoppedAgain,
        InfraKind::LogBound,
        InfraKind::LowSpace,
        InfraKind::SerialMissing,
        InfraKind::NoExitStatus,
        InfraKind::InputFiles,
        InfraKind::Checkpoint,
        InfraKind::ShutdownFailed,
        InfraKind::PrepareTimeout,
        InfraKind::FileOperation,
        InfraKind::VmDidNotExit,
        InfraKind::VmmNotLoaded,
        InfraKind::VmmChanged,
        InfraKind::SecondPrepare,
        InfraKind::PinMismatch,
        InfraKind::ModuleMismatch,
        InfraKind::CheckpointChanged,
    ];

    #[test]
    fn every_kind_has_its_own_plain_text() {
        let mut texts: Vec<&str> = ALL.iter().map(|k| k.text()).collect();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), ALL.len());
        for t in texts {
            assert!(!t.contains("--") && !t.contains("deterministic"), "{t}");
        }
    }

    #[test]
    fn the_reasons_the_design_names_read_exactly() {
        assert_eq!(
            InfraKind::StoppedAgain.text(),
            "the VM stopped again after it was resumed"
        );
        assert_eq!(InfraKind::LogBound.text(), "log size bound reached");
        assert_eq!(InfraKind::HostBusy.text(), "another run holds the host");
    }

    #[test]
    fn a_detail_follows_the_text_in_brackets() {
        assert_eq!(
            InfraError::bare(InfraKind::NoReady).to_string(),
            InfraKind::NoReady.text()
        );
        let e = InfraError::new(InfraKind::ResumeNotTaken, "excluded");
        assert_eq!(e.to_string(), "the resume command was not taken (excluded)");
        let io = InfraError::io("read", &std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(io.to_string().contains("not found"));
    }
}

//! Guest events for one trial: the optional events file, and folding every source into [`TrialProps`].
//!
//! The file transport. The guest is told `HAIRSPRING_EVENTS_FILE=/tmp/hairspring-events`, the same bytes on every
//! trial, seed and run, because the launch environment is an input that moves schedules under Hermit. The seed- and
//! trial-specific part lives only on the host: the backend gives Hermit `--tmp=<private per-trial directory>`, which
//! Hermit mounts as the guest's `/tmp`, so the guest's fixed path lands in a directory only this trial can write.
//! Hermit's own command line is not part of the guest's world, so the differing host path does not reach the guest.
use crate::capture::EventLines;
use crate::summary_dir::ScratchDir;
use hairspring_props::limits::MAX_EVENT_BYTES_PER_TRIAL;
use hairspring_props::{EventHealth, TrialEvents, TrialProps};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

/// Name of the variable the guest SDK reads. Mirrors `docs/guest-protocol.md`.
pub const EVENTS_FILE_ENV: &str = "HAIRSPRING_EVENTS_FILE";
/// The value the guest sees: fixed, seed-independent, inside the `/tmp` that Hermit mounts from the scratch directory.
pub const GUEST_EVENTS_FILE: &str = "/tmp/hairspring-events";
/// The same file as the host sees it, relative to the scratch directory.
const HOST_FILE_NAME: &str = "hairspring-events";
const TMP_FLAG: &str = "--tmp";

/// Whether the target's own Hermit arguments already choose the guest's `/tmp`, which the events file needs for itself.
pub fn claims_tmp(hermit_args: &[String]) -> bool {
    hermit_args
        .iter()
        .any(|a| a == TMP_FLAG || a.starts_with("--tmp="))
}

/// The Hermit argument that mounts `dir` as the guest's `/tmp`.
pub fn tmp_arg(dir: &ScratchDir) -> String {
    format!("{TMP_FLAG}={}", dir.path().display())
}

/// What was read back from the events file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileEvents {
    pub bytes: Vec<u8>,
    /// The file held more than the per-trial budget; the excess was not read.
    pub truncated: bool,
}

/// Reads the events file the guest left in `dir`, at most the per-trial budget. `Ok(None)` when the guest never
/// created it. Anything that is not a regular file (a symlink the guest planted, a device) is refused.
pub fn read_events_file(dir: &Path) -> Result<Option<FileEvents>, String> {
    let path = dir.join(HOST_FILE_NAME);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot inspect the events file: {e}")),
    };
    if !meta.file_type().is_file() || meta.file_type().is_fifo() {
        return Err("the events file is not a regular file; ignored".to_string());
    }
    let file = File::open(&path).map_err(|e| format!("cannot open the events file: {e}"))?;
    let mut bytes = Vec::new();
    // One byte past the budget tells "exactly full" from "cut".
    let limit = MAX_EVENT_BYTES_PER_TRIAL as u64 + 1;
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read the events file: {e}"))?;
    let truncated = bytes.len() > MAX_EVENT_BYTES_PER_TRIAL;
    bytes.truncate(MAX_EVENT_BYTES_PER_TRIAL);
    Ok(Some(FileEvents { bytes, truncated }))
}

/// Folds every source of event lines (file first, then stdout, then stderr) into one result, or `None` when the guest
/// sent nothing at all. Lines refused for budget before they got here are added to the health counters.
pub fn fold_sources(
    file: Option<&FileEvents>,
    stdout: &EventLines,
    stderr: &EventLines,
) -> Option<TrialProps> {
    let file_lines = file
        .into_iter()
        .flat_map(|f| f.bytes.split(|b| *b == b'\n'));
    let lines = file_lines.chain(stdout.lines()).chain(stderr.lines());
    let silent = file.is_none_or(|f| f.bytes.is_empty() && !f.truncated)
        && stdout.is_empty()
        && stderr.is_empty();
    if silent {
        return None;
    }
    let props = TrialEvents::fold_stream(lines);
    let refused = stdout.dropped + stderr.dropped + u64::from(file.is_some_and(|f| f.truncated));
    Some(TrialProps {
        health: EventHealth {
            over_limit: props.health.over_limit + refused,
            ..props.health.clone()
        },
        ..props
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, dropped: u64) -> EventLines {
        EventLines {
            bytes: text.as_bytes().to_vec(),
            dropped,
        }
    }

    const END: &str = "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"p1\",\"ops\":5}\n";

    #[test]
    fn nothing_from_anywhere_is_no_events() {
        let none = EventLines::default();
        assert!(fold_sources(None, &none, &none).is_none());
        assert!(fold_sources(Some(&FileEvents::default()), &none, &none).is_none());
    }

    #[test]
    fn stdout_stderr_and_file_are_merged() {
        let file = FileEvents {
            bytes: END.as_bytes().to_vec(),
            truncated: false,
        };
        let props = fold_sources(Some(&file), &lines(END, 0), &lines(END, 0)).unwrap();
        assert_eq!(props.health.accepted, 3);
    }

    #[test]
    fn refused_lines_are_counted_as_over_limit() {
        let file = FileEvents {
            bytes: Vec::new(),
            truncated: true,
        };
        let props = fold_sources(Some(&file), &lines("", 2), &lines("", 3)).unwrap();
        assert_eq!(props.health.over_limit, 6);
        assert!(props.is_degraded());
    }

    #[test]
    fn a_missing_file_is_none_and_a_symlink_is_refused() {
        let dir = std::env::temp_dir().join(format!("hairspring-ev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join(HOST_FILE_NAME));
        assert_eq!(read_events_file(&dir), Ok(None));
        std::os::unix::fs::symlink("/etc/hosts", dir.join(HOST_FILE_NAME)).unwrap();
        assert!(read_events_file(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_over_budget_is_cut_and_flagged() {
        let dir = std::env::temp_dir().join(format!("hairspring-ev-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(HOST_FILE_NAME),
            vec![b'x'; MAX_EVENT_BYTES_PER_TRIAL + 10],
        )
        .unwrap();
        let read = read_events_file(&dir).unwrap().unwrap();
        assert_eq!(read.bytes.len(), MAX_EVENT_BYTES_PER_TRIAL);
        assert!(read.truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tmp_claims_are_detected() {
        assert!(claims_tmp(&["--tmp=/x".to_string()]));
        assert!(claims_tmp(&["--tmp".to_string(), "/x".to_string()]));
        assert!(!claims_tmp(&["--verbose".to_string()]));
    }
}

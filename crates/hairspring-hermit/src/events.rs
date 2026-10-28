//! Guest events for one trial: the optional events file, and folding every source into [`TrialProps`].
//!
//! The file transport. The guest is told `HAIRSPRING_EVENTS_FILE=/tmp/hairspring-events`, the same bytes on every
//! trial, seed and run, because the launch environment is an input that moves schedules under Hermit. The seed- and
//! trial-specific part lives only on the host: the backend gives Hermit `--tmp=<private per-trial directory>`, which
//! Hermit mounts as the guest's `/tmp`, so the guest's fixed path lands in a directory only this trial can write.
//! Hermit's own command line is not part of the guest's world, so the differing host path does not reach the guest.
use crate::capture::EventLines;
use crate::safe_open::open_regular;
use crate::summary_dir::ScratchDir;
use hairspring_props::{EventHealth, TrialEvents, TrialProps};
use std::io::Read;
use std::path::Path;

/// Name of the variable the guest SDK reads. Mirrors `docs/guest-protocol.md`.
pub const EVENTS_FILE_ENV: &str = "HAIRSPRING_EVENTS_FILE";
/// The value the guest sees: fixed, seed-independent, inside the `/tmp` that Hermit mounts from the scratch directory.
pub const GUEST_EVENTS_FILE: &str = "/tmp/hairspring-events";
/// The same file as the host sees it, relative to the scratch directory.
const HOST_FILE_NAME: &str = "hairspring-events";
const TMP_FLAG: &str = "--tmp";

/// Why a trial's guest events could not be fully used. Config faults are the operator's to fix and are refused when a
/// run starts; infrastructure faults are the host's (a full scratch disk); guest faults are what the guest left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventsFault {
    Config(String),
    Infra(String),
    Guest(String),
}

impl EventsFault {
    pub fn message(&self) -> &str {
        match self {
            EventsFault::Config(m) | EventsFault::Infra(m) | EventsFault::Guest(m) => m,
        }
    }
}

impl std::fmt::Display for EventsFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            EventsFault::Config(_) => "configuration",
            EventsFault::Infra(_) => "infrastructure",
            EventsFault::Guest(_) => "guest",
        };
        write!(f, "events {kind} fault: {}", self.message())
    }
}

/// Whether one Hermit argument pair claims the guest's `/tmp`: `--tmp`, or `--bind`/`--mount` onto `/tmp`.
fn is_tmp_path(path: &str) -> bool {
    path == "/tmp" || path.starts_with("/tmp/")
}

fn claims_tmp_value(flag: &str, value: &str) -> bool {
    match flag {
        TMP_FLAG => true,
        // `SRC:DST[:OPTS]`: the destination is any piece after the source.
        "--bind" => value.split(':').skip(1).any(is_tmp_path),
        // `key=value,...`: the destination keys.
        "--mount" => value.split(',').any(|kv| {
            kv.split_once('=').is_some_and(|(k, v)| {
                matches!(k, "target" | "dst" | "destination") && is_tmp_path(v)
            })
        }),
        _ => false,
    }
}

/// Whether the target's own Hermit arguments already choose the guest's `/tmp`, which the events file needs for itself.
pub fn claims_tmp(hermit_args: &[String]) -> bool {
    hermit_args
        .iter()
        .enumerate()
        .any(|(i, arg)| match arg.split_once('=') {
            Some((flag, value)) => claims_tmp_value(flag, value),
            None => {
                let next = hermit_args.get(i + 1).map_or("", String::as_str);
                claims_tmp_value(arg, next)
            }
        })
}

/// What stops the events file from working with these settings, if anything. Checked when a run or replay starts, and
/// again for every trial.
pub fn config_problem(
    events_file: bool,
    hermit_args: &[String],
    has_scratch: bool,
) -> Option<String> {
    if !events_file {
        return None;
    }
    if claims_tmp(hermit_args) {
        return Some(
            "the events file needs the guest's /tmp for itself, but this target's hermit_args already set it (`--tmp`, or `--bind`/`--mount` onto /tmp)"
                .to_string(),
        );
    }
    if !has_scratch {
        return Some(
            "the events file needs a private scratch directory (`summary_dir` or `events_dir`), and none is available"
                .to_string(),
        );
    }
    None
}

/// The Hermit argument that mounts the trial's directory as the guest's `/tmp`.
pub fn tmp_arg(dir: &ScratchDir) -> String {
    format!("{TMP_FLAG}={}", dir.guest_tmp().display())
}

/// What was read back from the events file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileEvents {
    pub bytes: Vec<u8>,
    /// The file held more than the per-trial budget; the excess was not read.
    pub truncated: bool,
}

/// Reads the events file the guest left in `dir`, at most the per-trial budget. `Ok(None)` when the guest never
/// created it. Anything that is not a regular file (a symlink the guest planted, a FIFO, a device) is refused.
pub fn read_events_file(dir: &Path, budget: usize) -> Result<Option<FileEvents>, EventsFault> {
    let path = dir.join(HOST_FILE_NAME);
    let file = match open_regular(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(EventsFault::Guest(format!(
                "the events file was ignored: {e}"
            )))
        }
    };
    let mut bytes = Vec::new();
    // One byte past the budget tells "exactly full" from "cut".
    let limit = (budget as u64).saturating_add(1);
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| EventsFault::Guest(format!("cannot read the events file: {e}")))?;
    let truncated = bytes.len() > budget;
    bytes.truncate(budget);
    Ok(Some(FileEvents { bytes, truncated }))
}

fn is_silent(file: Option<&FileEvents>, stdout: &EventLines, stderr: &EventLines) -> bool {
    file.is_none_or(|f| f.bytes.is_empty() && !f.truncated)
        && stdout.is_empty()
        && stderr.is_empty()
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// A digest of everything the guest reported, in the fixed order stdout, stderr, file, so that `behaviour_hash` can
/// tell two trials apart by their property outcomes. `None` when the guest reported nothing, which keeps the hash of
/// every trial without events exactly what it was before events existed.
pub fn digest(file: Option<&FileEvents>, stdout: &EventLines, stderr: &EventLines) -> Option<u64> {
    if is_silent(file, stdout, stderr) {
        return None;
    }
    let mut h = FNV_OFFSET;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME);
        }
    };
    let empty = FileEvents::default();
    let file = file.unwrap_or(&empty);
    // Each part is length-prefixed so that moving a byte between sources changes the digest.
    for (bytes, refused) in [
        (&stdout.bytes, stdout.dropped),
        (&stderr.bytes, stderr.dropped),
        (&file.bytes, u64::from(file.truncated)),
    ] {
        feed(&(bytes.len() as u64).to_le_bytes());
        feed(bytes);
        feed(&refused.to_le_bytes());
    }
    Some(h)
}

/// Folds every source of event lines (file first, then stdout, then stderr) into one result, or `None` when the guest
/// sent nothing at all. Lines refused for budget before they got here are added to the health counters.
pub fn fold_sources(
    file: Option<&FileEvents>,
    stdout: &EventLines,
    stderr: &EventLines,
    budget: usize,
) -> Option<TrialProps> {
    let file_lines = file
        .into_iter()
        .flat_map(|f| f.bytes.split(|b| *b == b'\n'));
    let lines = file_lines.chain(stdout.lines()).chain(stderr.lines());
    if is_silent(file, stdout, stderr) {
        return None;
    }
    let props = TrialEvents::fold_stream_with_budget(lines, budget);
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
    use hairspring_props::limits::MAX_EVENT_BYTES_PER_TRIAL;

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
        assert!(fold_sources(None, &none, &none, MAX_EVENT_BYTES_PER_TRIAL).is_none());
        assert!(fold_sources(
            Some(&FileEvents::default()),
            &none,
            &none,
            MAX_EVENT_BYTES_PER_TRIAL
        )
        .is_none());
    }

    #[test]
    fn stdout_stderr_and_file_are_merged() {
        let file = FileEvents {
            bytes: END.as_bytes().to_vec(),
            truncated: false,
        };
        let props = fold_sources(
            Some(&file),
            &lines(END, 0),
            &lines(END, 0),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!(props.health.accepted, 3);
    }

    #[test]
    fn refused_lines_are_counted_as_over_limit() {
        let file = FileEvents {
            bytes: Vec::new(),
            truncated: true,
        };
        let props = fold_sources(
            Some(&file),
            &lines("", 2),
            &lines("", 3),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!(props.health.over_limit, 6);
        assert!(props.is_degraded());
    }

    #[test]
    fn a_missing_file_is_none_and_a_symlink_is_refused() {
        let dir = std::env::temp_dir().join(format!("hairspring-ev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join(HOST_FILE_NAME));
        assert_eq!(read_events_file(&dir, MAX_EVENT_BYTES_PER_TRIAL), Ok(None));
        std::os::unix::fs::symlink("/etc/hosts", dir.join(HOST_FILE_NAME)).unwrap();
        assert!(matches!(
            read_events_file(&dir, MAX_EVENT_BYTES_PER_TRIAL),
            Err(EventsFault::Guest(_))
        ));
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
        let read = read_events_file(&dir, MAX_EVENT_BYTES_PER_TRIAL)
            .unwrap()
            .unwrap();
        assert_eq!(read.bytes.len(), MAX_EVENT_BYTES_PER_TRIAL);
        assert!(read.truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unbounded_budget_reads_the_whole_file_without_overflowing() {
        let dir = std::env::temp_dir().join(format!("hairspring-ev-max-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(HOST_FILE_NAME), b"HAIRSPRING_EVENT {}\n").unwrap();
        let read = read_events_file(&dir, usize::MAX).unwrap().unwrap();
        assert_eq!(read.bytes, b"HAIRSPRING_EVENT {}\n");
        assert!(!read.truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tmp_claims_are_detected() {
        assert!(claims_tmp(&["--tmp=/x".to_string()]));
        assert!(claims_tmp(&["--tmp".to_string(), "/x".to_string()]));
        assert!(!claims_tmp(&["--verbose".to_string()]));
    }

    #[test]
    fn bind_and_mount_onto_tmp_count_as_claiming_it() {
        let claim =
            |args: &[&str]| claims_tmp(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert!(claim(&["--bind=/h:/tmp"]));
        assert!(claim(&["--bind", "/h:/tmp"]));
        assert!(claim(&["--bind=/h:/tmp/sub:ro"]));
        assert!(claim(&["--mount=type=tmpfs,target=/tmp"]));
        assert!(claim(&["--mount", "type=bind,src=/h,dst=/tmp"]));
        assert!(!claim(&["--bind=/tmp:/data"]));
        assert!(!claim(&["--bind=/h:/var/lib"]));
    }

    fn through(bytes: &[u8]) -> EventLines {
        let mut s = crate::capture::Splitter::new(0);
        s.feed(bytes);
        s.finish().events
    }

    /// A valid `work` event whose whole line (prefix included, newline excluded) is exactly `total` bytes.
    fn line_of(total: usize) -> Vec<u8> {
        let base = r#"HAIRSPRING_EVENT {"v":1,"kind":"work","proc":"p1","ops":1,"x":""}"#;
        let mut line = base.as_bytes()[..base.len() - 2].to_vec();
        line.extend(std::iter::repeat_n(b'a', total - base.len()));
        line.extend_from_slice(b"\"}");
        assert_eq!(line.len(), total);
        line.push(b'\n');
        line
    }

    #[test]
    fn a_line_of_exactly_4096_bytes_is_accepted_and_4097_is_too_long() {
        let ok = fold_sources(
            None,
            &through(&line_of(4096)),
            &EventLines::default(),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!((ok.health.accepted, ok.health.too_long), (1, 0));
        let long = fold_sources(
            None,
            &through(&line_of(4097)),
            &EventLines::default(),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!((long.health.accepted, long.health.too_long), (0, 1));
        let huge = fold_sources(
            None,
            &through(&line_of(300_000)),
            &EventLines::default(),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!((huge.health.accepted, huge.health.too_long), (0, 1));
    }

    #[test]
    fn nul_bytes_and_invalid_utf8_are_counted_not_fatal() {
        let bad_utf8 = through(b"HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"\xff\"}\n");
        let p = fold_sources(
            None,
            &bad_utf8,
            &EventLines::default(),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!(p.health.invalid_utf8, 1);
        let nul = through(b"HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"a\0b\"}\n");
        let p = fold_sources(
            None,
            &nul,
            &EventLines::default(),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!(p.health.malformed + p.health.over_limit, 1);
        assert_eq!(p.health.accepted, 0);
    }

    /// The splitter's per-stream budget uses the fold's own cost, so the fold never refuses a line the splitter kept.
    #[test]
    fn the_splitter_and_the_fold_agree_on_the_budget() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut input = Vec::new();
        let mut sent = 0u64;
        while input.len() < 3 * MAX_EVENT_BYTES_PER_TRIAL {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let total = 100 + (state >> 33) as usize % 5900;
            input.extend(line_of(total.max(80)));
            sent += 1;
        }
        let ev = through(&input);
        let stored = ev.lines().count() as u64;
        assert_eq!(stored + ev.dropped, sent);
        assert!(ev.dropped > 0);
        let p = fold_sources(None, &ev, &EventLines::default(), MAX_EVENT_BYTES_PER_TRIAL).unwrap();
        assert_eq!(
            p.health.over_limit, ev.dropped,
            "the fold refused nothing more"
        );
        assert_eq!(p.health.accepted + p.health.too_long, stored);
    }

    #[test]
    fn a_full_file_starves_the_stdout_events_and_they_are_counted() {
        let mut bytes = Vec::new();
        while bytes.len() + END.len() <= MAX_EVENT_BYTES_PER_TRIAL {
            bytes.extend_from_slice(END.as_bytes());
        }
        let file = FileEvents {
            bytes,
            truncated: false,
        };
        let p = fold_sources(
            Some(&file),
            &lines(&END.repeat(3), 0),
            &EventLines::default(),
            MAX_EVENT_BYTES_PER_TRIAL,
        )
        .unwrap();
        assert_eq!(p.health.over_limit, 3, "{:?}", p.health);
        assert!(p.is_degraded());
    }

    #[test]
    fn the_digest_tells_sources_and_content_apart_and_is_absent_without_events() {
        let none = EventLines::default();
        assert_eq!(digest(None, &none, &none), None);
        let a = digest(None, &lines(END, 0), &none);
        let b = digest(None, &none, &lines(END, 0));
        let c = digest(
            Some(&FileEvents {
                bytes: END.as_bytes().to_vec(),
                truncated: false,
            }),
            &none,
            &none,
        );
        assert!(a.is_some() && a != b && b != c && a != c);
        assert_ne!(
            digest(None, &lines(END, 0), &none),
            digest(None, &lines(END, 1), &none)
        );
    }
}

//! Reading the guest's serial log after a trial: the exit report the guest's agent prints last, and every line, for the
//! event parser. The log is the guest's output and is read as untrusted text, up to a fixed size.
use std::io::Read;
use std::path::Path;

/// The line the guest's agent prints when the program has ended: `HS_EXIT <code> <signal>`, a signal of 0 meaning none.
const EXIT_PREFIX: &str = "HS_EXIT ";

/// How the guest program ended, as the guest reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// What a serial log held.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Serial {
    /// The last well-formed exit line.
    pub exit: Option<GuestExit>,
    /// Every line, without its line end.
    pub lines: Vec<Vec<u8>>,
    /// The log was longer than the size it was read to.
    pub truncated: bool,
}

/// A whole number written with digits only: no sign, no space, no exponent.
fn whole(text: &str) -> Option<i32> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn parse_exit(line: &str) -> Option<GuestExit> {
    let rest = line.strip_prefix(EXIT_PREFIX)?;
    let mut parts = rest.split(' ');
    let code = whole(parts.next()?)?;
    let signal = whole(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(if signal > 0 {
        GuestExit {
            code: None,
            signal: Some(signal),
        }
    } else {
        GuestExit {
            code: Some(code),
            signal: None,
        }
    })
}

/// Splits `bytes` into lines and finds the exit report.
pub fn parse(bytes: &[u8], truncated: bool) -> Serial {
    let mut lines: Vec<Vec<u8>> = bytes
        .split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
        .collect();
    if lines.last().is_some_and(Vec::is_empty) {
        lines.pop();
    }
    let exit = lines
        .iter()
        .rev()
        .filter_map(|l| std::str::from_utf8(l).ok())
        .find_map(parse_exit);
    Serial {
        exit,
        lines,
        truncated,
    }
}

/// Reads the log at `path`, at most `max_bytes`. `Ok(None)` when it is not there.
pub fn read(path: &Path, max_bytes: u64) -> std::io::Result<Option<Serial>> {
    let file = match crate::safe_open::open_regular(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > max_bytes;
    bytes.truncate(usize::try_from(max_bytes).unwrap_or(usize::MAX));
    Ok(Some(parse(&bytes, truncated)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_exit_line_decides_and_a_signal_hides_the_code() {
        let s = parse(b"boot\nHS_EXIT 1 0\nHS_EXIT 0 0\n", false);
        assert_eq!(
            s.exit,
            Some(GuestExit {
                code: Some(0),
                signal: None
            })
        );
        let s = parse(b"HS_EXIT 0 9\n", false);
        assert_eq!(
            s.exit,
            Some(GuestExit {
                code: None,
                signal: Some(9)
            })
        );
        assert_eq!(s.lines, vec![b"HS_EXIT 0 9".to_vec()]);
    }

    #[test]
    fn a_malformed_exit_line_is_not_an_exit_report() {
        for bad in [
            "HS_EXIT",
            "HS_EXIT 0",
            "HS_EXIT x 0",
            "HS_EXIT 0 0 0",
            " HS_EXIT 0 0",
            "HS_EXIT 99999999999 0",
            "HS_EXIT +0 0",
            "HS_EXIT 0 +9",
            "HS_EXIT -1 0",
            "hs_exit 0 0",
        ] {
            assert_eq!(parse(bad.as_bytes(), false).exit, None, "{bad}");
        }
        assert_eq!(
            parse(b"HS_EXIT 0 0\nHS_EXIT zz\n", false)
                .exit
                .unwrap()
                .code,
            Some(0)
        );
    }

    #[test]
    fn a_fifo_or_a_link_at_the_log_is_refused_and_is_not_read() {
        let dir = std::env::temp_dir().join(format!("hs-dhyve-serial-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("real"), b"HS_EXIT 0 0\n").unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("uart.log")).unwrap();
        assert!(read(&dir.join("uart.log"), 100).is_err());
    }

    #[test]
    fn carriage_returns_and_a_missing_final_newline_are_handled() {
        let s = parse(b"a\r\nb\r\nHS_EXIT 3 0", false);
        assert_eq!(s.lines.len(), 3);
        assert_eq!(s.lines[0], b"a");
        assert_eq!(s.exit.unwrap().code, Some(3));
        assert!(parse(b"", false).lines.is_empty());
    }

    #[test]
    fn non_text_lines_are_kept_and_never_an_exit() {
        let s = parse(b"\xff\xfe\nHS_EXIT 0 0\n", false);
        assert_eq!(s.lines.len(), 2);
        assert!(s.exit.is_some());
    }

    #[test]
    fn a_log_is_read_to_a_size_and_marked_truncated_past_it() {
        let dir = std::env::temp_dir().join(format!("hs-dhyve-serial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("uart.log");
        assert_eq!(read(&path.with_extension("none"), 100).unwrap(), None);
        std::fs::write(&path, b"HS_EXIT 0 0\nmore lines here\n").unwrap();
        let whole = read(&path, 1000).unwrap().unwrap();
        assert!(!whole.truncated && whole.exit.is_some());
        let cut = read(&path, 12).unwrap().unwrap();
        assert!(cut.truncated);
        assert_eq!(cut.lines.len(), 1);
        let exact = read(&path, 28).unwrap().unwrap();
        assert!(!exact.truncated);
    }
}

//! Bounds on what a VM may write. `run.log` and the serial log have no bound the VM itself keeps, so the sizes are looked
//! at while it runs and a trial that passes a bound is ended and counted as an infrastructure failure. Free space is
//! checked before a trial starts.
use std::path::Path;

const MIB: u64 = 1024 * 1024;
const GIB_KIB: u64 = 1024 * 1024;

/// The largest each log may grow, and the least free space a trial starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub run_log_bytes: u64,
    pub uart_log_bytes: u64,
    /// The floor of the free-space check, in KiB.
    pub min_free_kib: u64,
}

/// Which log passed its bound, and how large it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breach {
    RunLog(u64),
    UartLog(u64),
}

impl Bounds {
    /// Until a write rate has been measured on a host: 256 MiB for `run.log` (with the trial cut at 100 seconds, which
    /// keeps the two in step) and 16 MiB for the serial log, and at least 2 GiB free.
    pub fn interim() -> Bounds {
        Bounds {
            run_log_bytes: 256 * MIB,
            uart_log_bytes: 16 * MIB,
            min_free_kib: 2 * GIB_KIB,
        }
    }

    /// The first log past its bound, if any.
    pub fn breach(&self, run_log_len: u64, uart_log_len: u64) -> Option<Breach> {
        if run_log_len > self.run_log_bytes {
            Some(Breach::RunLog(run_log_len))
        } else if uart_log_len > self.uart_log_bytes {
            Some(Breach::UartLog(uart_log_len))
        } else {
            None
        }
    }

    /// The free space a trial needs: both bounds plus the checkpoint it restores, but never under the floor.
    pub fn required_free_kib(&self, checkpoint_bytes: u64) -> u64 {
        let want = self
            .run_log_bytes
            .saturating_add(self.uart_log_bytes)
            .saturating_add(checkpoint_bytes);
        (want / 1024).max(self.min_free_kib)
    }
}

/// The size of a file now; a file that is not there yet is empty.
pub fn len_now(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// The available space in KiB from the output of `df -Pk <dir>`.
pub fn parse_free_kib(text: &str) -> Option<u64> {
    let line = text.lines().rfind(|l| !l.trim().is_empty())?;
    let columns: Vec<&str> = line.split_whitespace().collect();
    columns.get(3)?.parse().ok()
}

/// How much of a log is kept when a trial or a prepare ends in a finding or an error.
pub const KEPT_TAIL_BYTES: u64 = 1024 * 1024;

/// Cuts the file at `path` down to its last [`KEPT_TAIL_BYTES`]. A file that is not there is left alone.
pub fn keep_tail(path: &Path) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = crate::safe_open::open_regular(path) else {
        return;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return;
    };
    if len <= KEPT_TAIL_BYTES {
        return;
    }
    let mut tail = Vec::new();
    let read = file
        .seek(SeekFrom::Start(len - KEPT_TAIL_BYTES))
        .and_then(|_| file.take(KEPT_TAIL_BYTES).read_to_end(&mut tail));
    if read.is_ok() {
        let _ = std::fs::write(path, tail);
    }
}

/// The size of the files directly in `dir`.
pub fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .fold(0u64, |sum, m| sum.saturating_add(m.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interim_bounds_are_256_mib_and_16_mib_and_a_2_gib_floor() {
        let b = Bounds::interim();
        assert_eq!(b.run_log_bytes, 268_435_456);
        assert_eq!(b.uart_log_bytes, 16 * 1024 * 1024);
        assert_eq!(b.min_free_kib, 2_097_152);
    }

    #[test]
    fn a_log_past_its_bound_is_a_breach_and_one_at_it_is_not() {
        let b = Bounds {
            run_log_bytes: 100,
            uart_log_bytes: 10,
            min_free_kib: 1,
        };
        assert_eq!(b.breach(100, 10), None);
        assert_eq!(b.breach(101, 0), Some(Breach::RunLog(101)));
        assert_eq!(b.breach(0, 11), Some(Breach::UartLog(11)));
        assert_eq!(b.breach(200, 20), Some(Breach::RunLog(200)));
    }

    #[test]
    fn the_free_space_needed_is_the_larger_of_the_floor_and_the_bounds_plus_the_checkpoint() {
        let b = Bounds::interim();
        assert_eq!(b.required_free_kib(0), 2_097_152);
        let big = 4 * 1024 * MIB;
        assert_eq!(
            b.required_free_kib(big),
            (256 * MIB + 16 * MIB + big) / 1024
        );
        assert_eq!(b.required_free_kib(u64::MAX), u64::MAX / 1024);
    }

    #[test]
    fn free_space_is_read_from_the_available_column_of_the_last_line() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/ada0p2 100000 40000 60000 40% /var\n";
        assert_eq!(parse_free_kib(text), Some(60_000));
        assert_eq!(parse_free_kib(""), None);
        assert_eq!(parse_free_kib("Filesystem\n"), None);
        assert_eq!(parse_free_kib("a b c x d e\n"), None);
    }

    #[test]
    fn a_long_log_is_cut_to_its_last_mebibyte_and_a_short_one_is_left() {
        let dir = std::env::temp_dir().join(format!("hs-dhyve-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let long = dir.join("long");
        let mut bytes = vec![b'a'; KEPT_TAIL_BYTES as usize];
        bytes.extend_from_slice(b"END");
        std::fs::write(&long, &bytes).unwrap();
        keep_tail(&long);
        let kept = std::fs::read(&long).unwrap();
        assert_eq!(kept.len() as u64, KEPT_TAIL_BYTES);
        assert!(kept.ends_with(b"END"));
        let short = dir.join("short");
        std::fs::write(&short, b"abc").unwrap();
        keep_tail(&short);
        assert_eq!(std::fs::read(&short).unwrap(), b"abc");
        keep_tail(&dir.join("none"));
    }

    #[test]
    fn sizes_of_missing_files_and_directories_are_zero_and_files_are_summed() {
        let dir = std::env::temp_dir().join(format!("hs-dhyve-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(len_now(&dir.join("none")), 0);
        assert_eq!(dir_bytes(&dir), 0);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), [0u8; 10]).unwrap();
        std::fs::write(dir.join("b"), [0u8; 5]).unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert_eq!(len_now(&dir.join("a")), 10);
        assert_eq!(dir_bytes(&dir), 15);
    }
}

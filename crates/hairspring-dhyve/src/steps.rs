//! The session steps log, in the form the session's evaluator reads: one row for each step and each root command, seven TAB
//! separated fields, `UTC WRITER KIND STATE REF RC DETAIL`. The driver writes its own rows and, through [`SpawnLog`], the
//! rows of every start, destroy and checkpoint the backend runs. A row for a root command ends in `argv=` and the
//! command's argument list, with the state directory shown as `<W>`.
use crate::spawn::SpawnLog;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The longest an error text may be in a row.
const MAX_ERROR_BYTES: usize = 120;

/// `YYYY-MM-DDTHH:MM:SSZ` for a time in whole seconds since the epoch.
pub fn utc(epoch_secs: u64) -> String {
    let days = i64::try_from(epoch_secs / 86_400).unwrap_or(0);
    let rest = epoch_secs % 86_400;
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn now() -> String {
    utc(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()))
}

/// A field with no TAB, no line end and only printable ASCII; an empty one is `-`.
fn field(text: &str) -> String {
    let plain: String = text
        .chars()
        .map(|c| {
            if (' '..='~').contains(&c) && c != '\t' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if plain.is_empty() {
        "-".to_string()
    } else {
        plain
    }
}

/// An error text as a row carries it: runs of white space as one `_`, at most 120 bytes, printable ASCII.
pub fn error_text(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join("_");
    field(&joined.chars().take(MAX_ERROR_BYTES).collect::<String>())
}

/// The steps log file.
#[derive(Debug)]
pub struct Steps {
    path: PathBuf,
    state_dir: String,
}

impl Steps {
    pub fn new(path: &Path, state_dir: &Path) -> Steps {
        Steps {
            path: path.to_path_buf(),
            state_dir: state_dir.display().to_string(),
        }
    }

    fn append(&self, row: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            // One write, so rows from two writers never interleave inside a line.
            let _ = f.write_all(format!("{row}\n").as_bytes());
        }
    }

    /// One row of the driver's own: `kind`, `state`, a reference, a result and a detail.
    pub fn driver(&self, kind: &str, state: &str, reference: &str, rc: &str, detail: &str) {
        self.append(&format!(
            "{}\tdriver\t{kind}\t{state}\t{}\t{}\t{}",
            now(),
            field(reference),
            field(rc),
            field(detail)
        ));
    }
}

impl SpawnLog for Steps {
    fn row(&self, kind: &str, state: &str, reference: &str, rc: &str, argv: &[String]) {
        let detail = if argv.is_empty() {
            "-".to_string()
        } else {
            let masked = argv.join(" ").replace(&self.state_dir, "<W>");
            format!("argv={}", field(&masked))
        };
        self.append(&format!(
            "{}\tdriver\t{kind}\t{state}\t{}\t{}\t{detail}",
            now(),
            field(reference),
            field(rc)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_written_in_utc_in_the_fixed_form() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(
            utc(1_796_083_200 + 4 * 3_600 + 40 * 60 + 45),
            "2026-12-01T04:40:45Z"
        );
        assert_eq!(utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn a_field_is_one_printable_word_and_an_empty_one_is_a_dash() {
        assert_eq!(field(""), "-");
        assert_eq!(field("a\tb\nc\u{e9}"), "a_b_c_");
        assert_eq!(error_text("one  two\tthree"), "one_two_three");
        assert!(error_text(&"x".repeat(500)).len() <= MAX_ERROR_BYTES);
    }

    #[test]
    fn rows_have_seven_fields_and_a_root_command_ends_in_its_argv_with_the_state_directory_masked()
    {
        let dir = std::env::temp_dir().join(format!("hs-dhyve-steps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("steps.tsv");
        let _ = std::fs::remove_file(&path);
        let steps = Steps::new(&path, &dir);
        steps.driver("PREPARE", "start", "", "", "");
        steps.row(
            "R5",
            "done",
            "vmvx_x",
            "0",
            &[
                "sudo".to_string(),
                format!("--checkpoint={}/ckpt/boot", dir.display()),
            ],
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let rows: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        assert!(
            rows.iter().all(|r| r.len() == 7 && r[1] == "driver"),
            "{text}"
        );
        assert_eq!(&rows[0][2..], ["PREPARE", "start", "-", "-", "-"]);
        assert_eq!(rows[1][6], "argv=sudo --checkpoint=<W>/ckpt/boot");
        assert_eq!(rows[0][0].len(), 20);
    }
}

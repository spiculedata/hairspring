//! The six small files a VM reads after its stop: seed, job, plan, override, trigger and runner. Each file holds exactly
//! its field's width in ASCII, with no newline and no NUL. They are written before a VM starts and read back and hashed
//! (the gate) so a VM never starts on a file that is not what was meant.
use hairspring_core::point::encoding::sha256_hex;
use std::path::{Path, PathBuf};

/// The fields in the order they are given to the VM, with their widths.
pub const FIELDS: [(&str, usize); 6] = [
    ("seed", 20),
    ("job", 20),
    ("plan", 64),
    ("override", 34),
    ("trigger", 20),
    ("runner", 20),
];
/// The value of the `job` input every session gives: the guest image's fixture needs it to run.
pub const GUEST_JOB: u64 = 1;
/// The value of `runner` that says the guest runs under this kind of VM.
const RUNNER: u64 = 2;
/// The override record that overrides nothing.
const NO_OVERRIDE: &str = "k=0000000000v=00000000000000000000";

/// Why the files could not be made or did not read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FwcfgError {
    Io(String),
    /// A file read back different from what was written.
    Mismatch(&'static str),
}

impl std::fmt::Display for FwcfgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FwcfgError::Io(m) => write!(f, "the input files could not be written: {m}"),
            FwcfgError::Mismatch(field) => {
                write!(f, "the input file `{field}` does not read back as written")
            }
        }
    }
}

impl std::error::Error for FwcfgError {}

/// The values one VM run is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub seed: u64,
    pub job: u64,
}

/// The files of one run, with the SHA-256 of each as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    pub entries: Vec<(&'static str, PathBuf, String)>,
}

impl Inputs {
    fn value(&self, field: &str) -> String {
        match field {
            "seed" => format!("{:020}", self.seed),
            "job" => format!("{:020}", self.job),
            "plan" => "0".repeat(64),
            "override" => NO_OVERRIDE.to_string(),
            "trigger" => "0".repeat(20),
            _ => format!("{RUNNER:020}"),
        }
    }

    /// The same inputs with another seed.
    pub fn with_seed(self, seed: u64) -> Inputs {
        Inputs { seed, ..self }
    }

    /// Writes the six files under `dir` and reads them back.
    pub fn write(&self, dir: &Path) -> Result<Files, FwcfgError> {
        let io = |e: std::io::Error| FwcfgError::Io(e.kind().to_string());
        std::fs::create_dir_all(dir).map_err(io)?;
        let mut entries = Vec::new();
        for (field, width) in FIELDS {
            let value = self.value(field);
            debug_assert_eq!(value.len(), width);
            let path = dir.join(field);
            std::fs::write(&path, value.as_bytes()).map_err(io)?;
            entries.push((field, path, sha256_hex(value.as_bytes())));
        }
        let files = Files { entries };
        self.gate(&files)?;
        Ok(files)
    }

    /// Reads every file back and compares it with what these inputs say it must hold.
    pub fn gate(&self, files: &Files) -> Result<(), FwcfgError> {
        for (field, path, sha) in &files.entries {
            let expected = self.value(field);
            let read = std::fs::read(path).map_err(|e| FwcfgError::Io(e.kind().to_string()))?;
            if read != expected.as_bytes() || sha256_hex(&read) != *sha {
                return Err(FwcfgError::Mismatch(field));
            }
        }
        Ok(())
    }
}

impl Files {
    /// `-f` items for the VM's argument list, in field order.
    pub fn items(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(field, path, _)| format!("name=opt/hs/{field},file={}", path.display()))
            .collect()
    }

    /// `field=sha256` pairs, for the record.
    pub fn hashes(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .map(|(f, _, h)| (f.to_string(), h.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hs-dhyve-fw-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn each_file_holds_exactly_its_width_with_no_newline() {
        let dir = scratch("widths");
        let files = Inputs { seed: 1007, job: 3 }.write(&dir).unwrap();
        for ((field, width), (name, path, _)) in FIELDS.iter().zip(&files.entries) {
            assert_eq!(field, name);
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(bytes.len(), *width, "{field}");
            assert!(bytes.is_ascii() && !bytes.contains(&b'\n') && !bytes.contains(&0));
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("seed")).unwrap(),
            "00000000000000001007"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("override")).unwrap(),
            "k=0000000000v=00000000000000000000"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("runner")).unwrap(),
            "00000000000000000002"
        );
    }

    #[test]
    fn another_seed_changes_only_the_seed_file() {
        let a = Inputs { seed: 1, job: 0 }
            .write(&scratch("seed-a"))
            .unwrap();
        let b = Inputs { seed: 1, job: 0 }
            .with_seed(2)
            .write(&scratch("seed-b"))
            .unwrap();
        let differing: Vec<_> = a
            .hashes()
            .into_iter()
            .zip(b.hashes())
            .filter(|(x, y)| x != y)
            .map(|(x, _)| x.0)
            .collect();
        assert_eq!(differing, vec!["seed".to_string()]);
    }

    #[test]
    fn the_inputs_of_a_session_hash_to_the_constants_of_the_run_table() {
        let c = Inputs {
            seed: 1007,
            job: GUEST_JOB,
        }
        .write(&scratch("consts-c"))
        .unwrap();
        let s = Inputs {
            seed: 1008,
            job: GUEST_JOB,
        }
        .write(&scratch("consts-s"))
        .unwrap();
        let want = |files: &Files, field: &str| {
            files
                .hashes()
                .into_iter()
                .find(|(f, _)| f == field)
                .unwrap()
                .1
        };
        assert_eq!(
            want(&c, "seed"),
            "292b1fcd6c00d6ff28d8ca5cb6d02f9470e36712a39fb0135b9af7114df4ed7a"
        );
        assert_eq!(
            want(&s, "seed"),
            "0c7bbc4cb64b328d64391be1eeddf10c1af36887a32d2c47d44b10b897026e3b"
        );
        assert_eq!(
            want(&c, "job"),
            "ad8324689b78cd6e2177b4f02dec255566ae4a430a820312ee7ca567e32db079"
        );
        assert_eq!(
            want(&c, "plan"),
            "60e05bd1b195af2f94112fa7197a5c88289058840ce7c6df9693756bc6250f55"
        );
        assert_eq!(
            want(&c, "override"),
            "7875ad0b8d0084cfc3cfffc0df542140bc6bc31ad0db5b618e063040eb5cf654"
        );
        assert_eq!(
            want(&c, "trigger"),
            "a7d351f3de399c746064a53c69959ab10312a003ec63985bcd86fb6dc1c402d5"
        );
        assert_eq!(
            want(&c, "runner"),
            "720c1731450d7d8c443026e27685a7ea775e9c679783d9868bc135d393ef9e81"
        );
    }

    #[test]
    fn the_gate_refuses_a_file_changed_after_it_was_written() {
        let dir = scratch("gate");
        let inputs = Inputs { seed: 5, job: 0 };
        let files = inputs.write(&dir).unwrap();
        assert_eq!(inputs.gate(&files), Ok(()));
        std::fs::write(dir.join("seed"), b"00000000000000000006").unwrap();
        assert_eq!(inputs.gate(&files), Err(FwcfgError::Mismatch("seed")));
        std::fs::remove_file(dir.join("job")).unwrap();
        std::fs::write(dir.join("seed"), b"00000000000000000005").unwrap();
        assert!(matches!(inputs.gate(&files), Err(FwcfgError::Io(_))));
    }

    #[test]
    fn items_name_the_fields_in_order() {
        let files = Inputs { seed: 1, job: 0 }.write(&scratch("items")).unwrap();
        let items = files.items();
        assert_eq!(items.len(), 6);
        assert!(items[0].starts_with("name=opt/hs/seed,file="));
        assert!(items[5].starts_with("name=opt/hs/runner,file="));
    }

    #[test]
    fn an_unwritable_directory_is_an_error() {
        let dir = scratch("unwritable");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
        let err = Inputs { seed: 1, job: 0 }
            .write(&dir.join("f").join("inside"))
            .unwrap_err();
        assert!(err.to_string().contains("could not be written"));
        assert!(FwcfgError::Mismatch("seed").to_string().contains("seed"));
    }
}

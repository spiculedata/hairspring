//! Hashes of the files that decide how the dynamic loader behaves, so an operator can compare them across sessions:
//! a change to any of them changes schedules and stops earlier recordings replaying.
use std::path::Path;

const LD_SO_CACHE: &str = "/etc/ld.so.cache";
const LIBC_CANDIDATES: [&str; 3] = [
    "/lib/x86_64-linux-gnu/libc.so.6",
    "/usr/lib/x86_64-linux-gnu/libc.so.6",
    "/lib64/libc.so.6",
];
const LD_SO_CANDIDATES: [&str; 3] = [
    "/lib64/ld-linux-x86-64.so.2",
    "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
    "/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
];

/// Largest file hashed. The loader cache and the C library are a few megabytes.
const MAX_HASHED_BYTES: u64 = 64 << 20;

/// SHA-256 (hex) of each file, `None` for one that is missing or unreadable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoaderHashes {
    pub cache: Option<String>,
    pub libc: Option<String>,
    pub ld_so: Option<String>,
}

impl LoaderHashes {
    pub fn probe() -> LoaderHashes {
        LoaderHashes {
            cache: hash_first(&[Path::new(LD_SO_CACHE)]),
            libc: hash_first(&paths(&LIBC_CANDIDATES)),
            ld_so: hash_first(&paths(&LD_SO_CANDIDATES)),
        }
    }

    /// One line for the doctor report, or `None` when nothing could be read (another operating system).
    pub fn describe(&self) -> Option<String> {
        let parts: Vec<String> = [
            ("ld.so.cache", &self.cache),
            ("libc", &self.libc),
            ("ld.so", &self.ld_so),
        ]
        .iter()
        .filter_map(|(name, hash)| hash.as_ref().map(|h| format!("{name} sha256 {h}")))
        .collect();
        (!parts.is_empty()).then(|| format!("loader state: {}", parts.join(", ")))
    }
}

fn paths(candidates: &[&'static str]) -> Vec<&'static Path> {
    candidates.iter().map(|s| Path::new(*s)).collect()
}

/// The hash of the first of `candidates` that can be read.
fn hash_first(candidates: &[&Path]) -> Option<String> {
    candidates.iter().find_map(|path| hash_file(path))
}

fn hash_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_HASHED_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_HASHED_BYTES).read_to_end(&mut bytes).ok()?;
    Some(hairspring_store::sha256_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-loader-{name}-{}-{}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_first_readable_candidate_is_hashed() {
        let dir = scratch("first");
        let present = dir.join("libc.so.6");
        std::fs::write(&present, b"abc").unwrap();
        let missing = dir.join("absent");
        let hash = hash_first(&[missing.as_path(), present.as_path()]);
        assert_eq!(
            hash.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn nothing_readable_gives_no_hash() {
        let dir = scratch("none");
        let missing = dir.join("absent");
        assert_eq!(hash_first(&[missing.as_path(), dir.as_path()]), None);
        assert_eq!(hash_first(&[]), None);
    }

    #[test]
    fn the_description_lists_only_what_was_read() {
        let hashes = LoaderHashes {
            cache: Some("aa".into()),
            libc: None,
            ld_so: Some("cc".into()),
        };
        assert_eq!(
            hashes.describe().as_deref(),
            Some("loader state: ld.so.cache sha256 aa, ld.so sha256 cc")
        );
        assert_eq!(LoaderHashes::default().describe(), None);
    }
}

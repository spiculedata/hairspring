//! The bundle files `hairspring test` writes next to the database, and how `gc` removes the ones whose bundle it has
//! just deleted from the store.
//!
//! A bundle file is `<target>-<first 8 hex digits of the bundle's hash>.hairspring` and holds the bundle text. `gc`
//! touches a file only when all of these hold: it is a direct child of the bundle directory (never a path from the
//! database), it is a regular file and not a symbolic link, its name ends in `-<8 digits>.hairspring` with the digits
//! of a bundle that was just deleted, and its contents hash to that bundle's full hash. A file that merely looks
//! like a bundle file but holds something else is never removed.
use hairspring_store::sha256_hex;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const EXTENSION: &str = ".hairspring";
const HASH_PREFIX_LEN: usize = 8;
/// Bundles are small text files; a larger file is not one and is left alone.
const MAX_BUNDLE_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    pub path: PathBuf,
    pub bytes: u64,
}

/// What the bundle directory holds for the deleted bundles.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BundleFiles {
    /// Files that match a deleted bundle exactly, in name order.
    pub files: Vec<BundleFile>,
    pub bytes: u64,
    /// Things looked at and left alone, with the reason.
    pub skipped: Vec<String>,
}

/// The files in `dir` that are copies of the bundles with these hashes. A missing directory holds none; a directory
/// that is a symbolic link is not entered.
pub fn find(dir: &Path, hashes: &[String]) -> io::Result<BundleFiles> {
    let mut found = BundleFiles::default();
    if hashes.is_empty() {
        return Ok(found);
    }
    match fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => {
            found.skipped.push(format!(
                "{} is a symbolic link, so no bundle files in it were looked at",
                dir.display()
            ));
            return Ok(found);
        }
        Ok(m) if !m.is_dir() => {
            found
                .skipped
                .push(format!("{} is not a directory", dir.display()));
            return Ok(found);
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(e),
    }
    let by_prefix: BTreeMap<&str, &str> = hashes
        .iter()
        .filter(|h| h.len() >= HASH_PREFIX_LEN && h.is_char_boundary(HASH_PREFIX_LEN))
        .map(|h| (&h[..HASH_PREFIX_LEN], h.as_str()))
        .collect();
    let mut names: Vec<_> = fs::read_dir(dir)?
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.file_name())
        .collect();
    names.sort();
    for name in names {
        let Some(name) = name.to_str() else { continue };
        let Some(prefix) = prefix_of(name) else {
            continue;
        };
        let Some(full) = by_prefix.get(prefix) else {
            continue;
        };
        let path = dir.join(name);
        match inspect(&path, full) {
            Inspected::Match(bytes) => {
                found.bytes += bytes;
                found.files.push(BundleFile { path, bytes });
            }
            Inspected::Skip(why) => found.skipped.push(format!("{}: {why}", path.display())),
        }
    }
    Ok(found)
}

/// The eight hex digits a bundle file's name ends with, if it is named like one.
fn prefix_of(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(EXTENSION)?;
    let (_, digits) = stem.rsplit_once('-')?;
    (digits.len() == HASH_PREFIX_LEN && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(digits)
}

enum Inspected {
    Match(u64),
    Skip(&'static str),
}

fn inspect(path: &Path, full_hash: &str) -> Inspected {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Inspected::Skip("could not be read");
    };
    if meta.file_type().is_symlink() {
        return Inspected::Skip("is a symbolic link, left alone");
    }
    if !meta.is_file() {
        return Inspected::Skip("is not a regular file, left alone");
    }
    if meta.len() > MAX_BUNDLE_FILE_BYTES {
        return Inspected::Skip("is too large to be a bundle, left alone");
    }
    match fs::read(path) {
        Ok(bytes) if sha256_hex(&bytes) == full_hash => Inspected::Match(meta.len()),
        Ok(_) => Inspected::Skip("does not hold the deleted bundle, left alone"),
        Err(_) => Inspected::Skip("could not be read"),
    }
}

/// What `remove` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removal {
    pub removed: u64,
    pub bytes: u64,
    /// One line for each file that could not be removed.
    pub failed: Vec<String>,
}

/// Removes the files `find` returned. It looks at each once more first (a path that has since become a symbolic link
/// or a directory is left alone) and goes on after a failure; the result lists what could not be removed. A file
/// already gone is not a failure, so removal can be repeated.
pub fn remove(files: &[BundleFile]) -> Removal {
    let mut done = Removal::default();
    for BundleFile { path, bytes } in files {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                done.failed.push(format!("{}: {e}", path.display()));
                continue;
            }
            Ok(m) if !m.is_file() => {
                done.failed.push(format!(
                    "{}: no longer a regular file, left alone",
                    path.display()
                ));
                continue;
            }
            Ok(_) => {}
        }
        match fs::remove_file(path) {
            Ok(()) => {
                done.removed += 1;
                done.bytes += bytes;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => done.failed.push(format!("{}: {e}", path.display())),
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-gc-files-{name}-{}-{}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_bundle(dir: &Path, target: &str, text: &str) -> (PathBuf, String) {
        let hash = sha256_hex(text.as_bytes());
        let path = dir.join(format!("{target}-{}{EXTENSION}", &hash[..HASH_PREFIX_LEN]));
        fs::write(&path, text).unwrap();
        (path, hash)
    }

    #[test]
    fn a_missing_directory_holds_nothing() {
        let found = find(Path::new("/nonexistent/hairspring-gc"), &["a".repeat(64)]).unwrap();
        assert_eq!(found, BundleFiles::default());
    }

    #[test]
    fn only_files_that_hold_a_deleted_bundle_are_found() {
        let d = scratch("find");
        let (gone, gone_hash) = write_bundle(&d, "svc", "seed 1\n");
        let (kept, _) = write_bundle(&d, "svc", "seed 2\n");
        // Named like the deleted bundle's file but holding something else.
        let imposter = d.join(format!(
            "other-{}{EXTENSION}",
            &gone_hash[..HASH_PREFIX_LEN]
        ));
        fs::write(&imposter, "not the bundle").unwrap();
        fs::write(d.join("notes.txt"), "seed 1\n").unwrap();
        let found = find(&d, &[gone_hash]).unwrap();
        assert_eq!(found.files.len(), 1);
        assert_eq!(found.files[0].path, gone);
        assert_eq!(found.bytes, 7);
        assert_eq!(found.skipped.len(), 1, "{:?}", found.skipped);
        assert!(found.skipped[0].contains("does not hold the deleted bundle"));
        assert!(kept.exists() && imposter.exists());
        fs::remove_dir_all(d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_never_followed_or_removed_by_find() {
        let d = scratch("link");
        let outside = scratch("link-outside");
        let (real, hash) = write_bundle(&outside, "svc", "seed 3\n");
        let link = d.join(real.file_name().unwrap());
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let found = find(&d, &[hash]).unwrap();
        assert!(found.files.is_empty());
        assert!(found.skipped[0].contains("symbolic link"));
        assert!(real.exists() && link.exists());
        fs::remove_dir_all(d).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_bundle_directory_that_is_a_symbolic_link_is_not_entered() {
        let outside = scratch("dirlink-outside");
        let (real, hash) = write_bundle(&outside, "svc", "seed 4\n");
        let parent = scratch("dirlink");
        let link = parent.join("bundles");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let found = find(&link, &[hash]).unwrap();
        assert!(found.files.is_empty());
        assert!(found.skipped[0].contains("symbolic link"));
        assert!(real.exists());
        fs::remove_dir_all(parent).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn removal_leaves_a_path_that_became_a_symbolic_link() {
        let d = scratch("swap");
        let outside = scratch("swap-outside");
        let victim = outside.join("precious.txt");
        fs::write(&victim, "keep me").unwrap();
        let (path, hash) = write_bundle(&d, "svc", "seed 5\n");
        let found = find(&d, &[hash]).unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        let done = remove(&found.files);
        assert_eq!(done.removed, 0);
        assert_eq!(done.failed.len(), 1);
        assert!(victim.exists(), "the target of the link is untouched");
        fs::remove_dir_all(d).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn removal_counts_what_it_removed_and_can_be_repeated() {
        let d = scratch("remove");
        let (a, ha) = write_bundle(&d, "svc", "seed 6\n");
        let (b, hb) = write_bundle(&d, "svc", "seed 7\n");
        let found = find(&d, &[ha, hb]).unwrap();
        assert_eq!(found.files.len(), 2);
        let done = remove(&found.files);
        assert_eq!((done.removed, done.bytes, done.failed.len()), (2, 14, 0));
        assert!(!a.exists() && !b.exists());
        assert_eq!(
            remove(&found.files),
            Removal::default(),
            "already gone is not a failure"
        );
        fs::remove_dir_all(d).ok();
    }

    #[test]
    fn a_failed_removal_is_reported_and_the_rest_still_go() {
        let d = scratch("fail");
        let (a, ha) = write_bundle(&d, "svc", "seed 8\n");
        let (b, hb) = write_bundle(&d, "svc", "seed 9\n");
        let found = find(&d, &[ha, hb]).unwrap();
        // A directory where a file was expected cannot be removed as a file.
        fs::remove_file(&a).unwrap();
        fs::create_dir(&a).unwrap();
        let done = remove(&found.files);
        assert_eq!(done.removed, 1);
        assert_eq!(done.failed.len(), 1, "{:?}", done.failed);
        assert!(!b.exists() && a.exists());
        fs::remove_dir_all(d).ok();
    }

    #[test]
    fn names_that_are_not_bundle_files_are_ignored() {
        assert_eq!(prefix_of("svc-0123abcd.hairspring"), Some("0123abcd"));
        assert_eq!(prefix_of("svc-0123abc.hairspring"), None);
        assert_eq!(prefix_of("svc-0123abcg.hairspring"), None);
        assert_eq!(prefix_of("svc0123abcd.hairspring"), None);
        assert_eq!(prefix_of("svc-0123abcd.txt"), None);
    }
}

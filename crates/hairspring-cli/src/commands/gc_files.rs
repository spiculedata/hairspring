//! The bundle files `hairspring test` writes next to the database, and how `gc` removes the ones whose bundle it has
//! just deleted from the store.
//!
//! A bundle file is `<target>-<first 8 hex digits of the bundle's hash>.hairspring` and holds the bundle text. `gc`
//! touches a file only when all of these hold: it is a direct child of the bundle directory (never a path from the
//! database), it is a regular file and not a symbolic link, its name ends in `-<8 digits>.hairspring` with the digits
//! of a bundle that was just deleted, and the contents read from the opened file hash to that bundle's full hash. A
//! file that merely looks like a bundle file but holds something else is never removed.
//!
//! Between finding a file and removing it, the directory or the file could be swapped. Just before each removal the
//! bundle directory must still be the same directory (device and inode) and not a symbolic link, and the file must
//! still be the same file, opened and hashed again. The standard library has no way to remove a file relative to an
//! open directory, so a short window remains between that last check and the removal: someone able to rename the
//! bundle directory (write access to the directory above it) could swap it in that window. See "Retention" in
//! `docs/architecture.md`.
use hairspring_store::sha256_hex;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const EXTENSION: &str = ".hairspring";
const HASH_PREFIX_LEN: usize = 8;
/// Bundles are small text files; a larger file is not one and is left alone.
const MAX_BUNDLE_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Which file (or directory) a path named when it was looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    fn of(m: &fs::Metadata) -> FileId {
        FileId {
            dev: m.dev(),
            ino: m.ino(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    pub path: PathBuf,
    pub bytes: u64,
    hash: String,
    id: FileId,
}

/// What the bundle directory holds for the deleted bundles.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BundleFiles {
    /// Files that match a deleted bundle exactly, in name order.
    pub files: Vec<BundleFile>,
    pub bytes: u64,
    /// Things looked at and left alone, with the reason.
    pub skipped: Vec<String>,
    /// The bundle directory the files were found in.
    dir: Option<FileId>,
}

/// The bundle directory as it is now: `Ok(None)` when it is missing, a note when it is not a plain directory.
fn dir_id(dir: &Path) -> io::Result<Result<Option<FileId>, String>> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => Ok(Err(format!(
            "{} is a symbolic link, so no bundle files in it were looked at",
            dir.display()
        ))),
        Ok(m) if !m.is_dir() => Ok(Err(format!("{} is not a directory", dir.display()))),
        Ok(m) => Ok(Ok(Some(FileId::of(&m)))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Ok(None)),
        Err(e) => Err(e),
    }
}

/// The files in `dir` that are copies of the bundles with these hashes. A missing directory holds none; a directory
/// that is a symbolic link is not entered.
pub fn find(dir: &Path, hashes: &[String]) -> io::Result<BundleFiles> {
    let mut found = BundleFiles::default();
    if hashes.is_empty() {
        return Ok(found);
    }
    let id = match dir_id(dir)? {
        Ok(Some(id)) => id,
        Ok(None) => return Ok(found),
        Err(note) => {
            found.skipped.push(note);
            return Ok(found);
        }
    };
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
            Ok((bytes, file_id)) => {
                found.bytes += bytes;
                found.files.push(BundleFile {
                    path,
                    bytes,
                    hash: (*full).to_owned(),
                    id: file_id,
                });
            }
            Err(why) => found.skipped.push(format!("{}: {why}", path.display())),
        }
    }
    // What was read must have come from the directory that was checked.
    if dir_id(dir)? != Ok(Some(id)) {
        return Ok(BundleFiles {
            skipped: vec![format!(
                "{} changed while it was being read, so no bundle files in it were looked at",
                dir.display()
            )],
            ..BundleFiles::default()
        });
    }
    found.dir = Some(id);
    Ok(found)
}

/// The eight hex digits a bundle file's name ends with, if it is named like one.
fn prefix_of(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(EXTENSION)?;
    let (_, digits) = stem.rsplit_once('-')?;
    (digits.len() == HASH_PREFIX_LEN && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(digits)
}

/// Opens the file at `path` without trusting the path: it must not be a symbolic link, the opened file must be the
/// regular file the path named a moment before, and the bytes read from the opened file must hash to `full_hash`.
/// Returns its size and identity, or why it is left alone.
fn inspect(path: &Path, full_hash: &str) -> Result<(u64, FileId), &'static str> {
    let named = fs::symlink_metadata(path).map_err(|_| "could not be read")?;
    if named.file_type().is_symlink() {
        return Err("is a symbolic link, left alone");
    }
    if !named.is_file() {
        return Err("is not a regular file, left alone");
    }
    if named.len() > MAX_BUNDLE_FILE_BYTES {
        return Err("is too large to be a bundle, left alone");
    }
    let file = File::open(path).map_err(|_| "could not be read")?;
    let opened = file.metadata().map_err(|_| "could not be read")?;
    if !opened.is_file() || FileId::of(&opened) != FileId::of(&named) {
        return Err("changed while it was being read, left alone");
    }
    let mut bytes = Vec::new();
    file.take(MAX_BUNDLE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not be read")?;
    if sha256_hex(&bytes) != full_hash {
        return Err("does not hold the deleted bundle, left alone");
    }
    Ok((bytes.len() as u64, FileId::of(&opened)))
}

/// What `remove` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removal {
    pub removed: u64,
    pub bytes: u64,
    /// One line for each file that could not be removed.
    pub failed: Vec<String>,
}

/// Removes the files `find` returned from `dir`. Just before each removal it checks again that `dir` is the directory
/// `find` read (not a symbolic link, same device and inode) and that the path still names the same file holding the
/// same bundle; anything else is left alone and listed. It goes on after a failure. A file already gone is not a
/// failure, so removal can be repeated.
pub fn remove(dir: &Path, found: &BundleFiles) -> Removal {
    let mut done = Removal::default();
    for BundleFile {
        path,
        bytes,
        hash,
        id,
    } in &found.files
    {
        match dir_id(dir) {
            Ok(Ok(now)) if now.is_some() && now == found.dir => {}
            _ => {
                done.failed.push(format!(
                    "{}: the bundle directory is no longer the one that was read, left alone",
                    path.display()
                ));
                continue;
            }
        }
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                done.failed.push(format!("{}: {e}", path.display()));
                continue;
            }
            Ok(_) => {}
        }
        match inspect(path, hash) {
            Ok((_, now)) if now == *id => {}
            Ok(_) => {
                done.failed.push(format!(
                    "{}: no longer the file that was read, left alone",
                    path.display()
                ));
                continue;
            }
            Err(why) => {
                done.failed.push(format!("{}: {why}", path.display()));
                continue;
            }
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
#[path = "gc_files_tests.rs"]
mod tests;

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

#[test]
fn removal_leaves_a_path_that_became_a_symbolic_link() {
    let d = scratch("swap");
    let outside = scratch("swap-outside");
    let victim = outside.join("precious.txt");
    fs::write(&victim, "seed 5\n").unwrap();
    let (path, hash) = write_bundle(&d, "svc", "seed 5\n");
    let found = find(&d, &[hash]).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    let done = remove(&d, &found);
    assert_eq!(done.removed, 0);
    assert_eq!(done.failed.len(), 1);
    assert!(victim.exists(), "the target of the link is untouched");
    fs::remove_dir_all(d).ok();
    fs::remove_dir_all(outside).ok();
}

/// The bundle directory swapped for a symbolic link to another directory that holds a file of the same name (here
/// even with the same contents) after `find` and before `remove`: the other directory is not touched.
#[test]
fn removal_leaves_alone_a_bundle_directory_swapped_for_a_symbolic_link() {
    let parent = scratch("dirswap");
    let d = parent.join("bundles");
    fs::create_dir(&d).unwrap();
    let elsewhere = scratch("dirswap-elsewhere");
    let (path, hash) = write_bundle(&d, "svc", "seed 10\n");
    let (same_name, _) = write_bundle(&elsewhere, "svc", "seed 10\n");
    let found = find(&d, &[hash]).unwrap();
    assert_eq!(found.files.len(), 1);
    fs::rename(&d, parent.join("bundles.old")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &d).unwrap();
    let done = remove(&d, &found);
    assert_eq!(done.removed, 0);
    assert_eq!(done.failed.len(), 1, "{:?}", done.failed);
    assert!(done.failed[0].contains("no longer the one that was read"));
    assert!(
        same_name.exists(),
        "the file outside the bundle directory stays"
    );
    assert!(parent
        .join("bundles.old")
        .join(path.file_name().unwrap())
        .exists());
    fs::remove_dir_all(parent).ok();
    fs::remove_dir_all(elsewhere).ok();
}

/// The same, with a real directory put in its place.
#[test]
fn removal_leaves_alone_a_bundle_directory_replaced_by_another_directory() {
    let parent = scratch("dirreplace");
    let d = parent.join("bundles");
    fs::create_dir(&d).unwrap();
    let (_, hash) = write_bundle(&d, "svc", "seed 11\n");
    let found = find(&d, &[hash]).unwrap();
    fs::rename(&d, parent.join("bundles.old")).unwrap();
    fs::create_dir(&d).unwrap();
    let (same_name, _) = write_bundle(&d, "svc", "seed 11\n");
    let done = remove(&d, &found);
    assert_eq!(
        (done.removed, done.failed.len()),
        (0, 1),
        "{:?}",
        done.failed
    );
    assert!(done.failed[0].contains("no longer the one that was read"));
    assert!(same_name.exists());
    fs::remove_dir_all(parent).ok();
}

/// A different file of the same name and contents is not the file that was read.
#[test]
fn removal_leaves_alone_a_file_replaced_since_it_was_read() {
    let d = scratch("filereplace");
    let (path, hash) = write_bundle(&d, "svc", "seed 12\n");
    let found = find(&d, &[hash]).unwrap();
    // Keep the old inode alive so the new file cannot reuse its number.
    let old = d.join("old-copy");
    fs::hard_link(&path, &old).unwrap();
    fs::remove_file(&path).unwrap();
    write_bundle(&d, "svc", "seed 12\n");
    let done = remove(&d, &found);
    assert_eq!(
        (done.removed, done.failed.len()),
        (0, 1),
        "{:?}",
        done.failed
    );
    assert!(done.failed[0].contains("no longer the file that was read"));
    assert!(path.exists());
    fs::remove_dir_all(d).ok();
}

/// The same file rewritten in place to hold something else is no longer a copy of the bundle.
#[test]
fn removal_leaves_alone_a_file_whose_contents_changed() {
    let d = scratch("rewrite");
    let (path, hash) = write_bundle(&d, "svc", "seed 13\n");
    let found = find(&d, &[hash]).unwrap();
    // fs::write truncates and rewrites the same inode.
    fs::write(&path, "something else\n").unwrap();
    let done = remove(&d, &found);
    assert_eq!(
        (done.removed, done.failed.len()),
        (0, 1),
        "{:?}",
        done.failed
    );
    assert!(done.failed[0].contains("does not hold the deleted bundle"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "something else\n");
    fs::remove_dir_all(d).ok();
}

#[test]
fn removal_counts_what_it_removed_and_can_be_repeated() {
    let d = scratch("remove");
    let (a, ha) = write_bundle(&d, "svc", "seed 6\n");
    let (b, hb) = write_bundle(&d, "svc", "seed 7\n");
    let found = find(&d, &[ha, hb]).unwrap();
    assert_eq!(found.files.len(), 2);
    let done = remove(&d, &found);
    assert_eq!((done.removed, done.bytes, done.failed.len()), (2, 14, 0));
    assert!(!a.exists() && !b.exists());
    assert_eq!(
        remove(&d, &found),
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
    let done = remove(&d, &found);
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

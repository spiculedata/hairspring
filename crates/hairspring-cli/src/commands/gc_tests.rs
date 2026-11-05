use super::gc_report::human_bytes;
use super::*;
use hairspring_store::{NewFailure, NewRun, SqliteStore};

const DAY_MS: i64 = 24 * MS_PER_HOUR;

#[test]
fn ages_are_a_whole_number_and_a_unit() {
    assert_eq!(parse_age("1h"), Ok(3_600_000));
    assert_eq!(parse_age("30d"), Ok(30 * 24 * 3_600_000));
    assert_eq!(parse_age("2w"), Ok(14 * 24 * 3_600_000));
}

#[test]
fn anything_else_is_refused() {
    for bad in [
        "",
        "d",
        "0d",
        "30",
        "30m",
        "-5d",
        "1.5d",
        "+3d",
        "3 d",
        "d3",
        "٣d",
        "99999999999999999999d",
    ] {
        assert!(parse_age(bad).is_err(), "{bad:?} should be refused");
    }
    assert!(
        parse_age(&format!("{}w", i64::MAX / 1000)).is_err(),
        "overflow is refused, not wrapped"
    );
}

#[test]
fn sizes_read_plainly() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(1023), "1023 B");
    assert_eq!(human_bytes(1536), "1.5 KiB");
    assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
}

#[test]
fn a_given_cutoff_is_used_but_never_later_than_the_age_allows() {
    let now = 100 * DAY_MS;
    assert_eq!(cutoff_for(None, "30d", now), Ok(70 * DAY_MS));
    assert_eq!(cutoff_for(Some(60 * DAY_MS), "30d", now), Ok(60 * DAY_MS));
    assert_eq!(cutoff_for(Some(70 * DAY_MS), "30d", now), Ok(70 * DAY_MS));
    let later = cutoff_for(Some(70 * DAY_MS + 1), "30d", now).unwrap_err();
    assert!(
        later.contains("later than --older-than 30d allows"),
        "{later}"
    );
    assert!(cutoff_for(Some(-1), "30d", now).is_err());
    assert!(cutoff_for(Some(0), "bad", now).is_err());
}

#[test]
fn the_digest_is_of_the_set_of_runs() {
    let a = vec!["run_b".to_owned(), "run_a".to_owned()];
    let b = vec!["run_a".to_owned(), "run_b".to_owned()];
    assert_eq!(runs_digest(&a), runs_digest(&b), "order does not matter");
    assert_eq!(runs_digest(&a).len(), DIGEST_LEN);
    assert_ne!(runs_digest(&a), runs_digest(&["run_a".to_owned()]));
    assert_ne!(
        runs_digest(&["run_a".to_owned(), "run_c".to_owned()]),
        runs_digest(&a),
        "same count, other runs"
    );
}

#[test]
fn counts_read_as_english() {
    assert_eq!(runs_text(0), "0 runs");
    assert_eq!(runs_text(1), "1 run");
    assert_eq!(runs_text(2), "2 runs");
}

fn section(runs: &[&str], kept: &[(&'static str, u64)]) -> Section {
    let mut report = RetentionReport {
        run_ids: runs.iter().map(|r| (*r).to_owned()).collect(),
        ..RetentionReport::default()
    };
    for (k, v) in kept {
        report.kept.insert(k, *v);
    }
    Section {
        report,
        files: BundleFiles::default(),
        removal: Some(Removal::default()),
        file_error: None,
    }
}

#[test]
fn an_apply_that_removed_fewer_runs_than_the_report_is_a_problem() {
    let planned = section(&["run_a", "run_b"], &[]);
    let applied = section(&["run_a"], &[("active_job", 1)]);
    let problems = problems_of(&planned, Some(&applied));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("removed 1 of the 2 runs in the report"),
        "{problems:?}"
    );
    assert!(problems_of(&planned, Some(&section(&["run_a", "run_b"], &[]))).is_empty());
    assert!(problems_of(&planned, None).is_empty());
}

fn header<'a>(skipped: &'a BTreeMap<String, u64>) -> Header<'a> {
    Header {
        older_than: "1h",
        cutoff: 5_000,
        digest: "0123456789abcdef",
        states: &[RunState::Passed],
        skipped,
    }
}

#[test]
fn the_reasons_the_apply_kept_runs_are_reported() {
    let skipped = BTreeMap::new();
    let h = header(&skipped);
    let planned = section(&["run_a", "run_b"], &[]);
    let applied = section(&["run_a"], &[("active_job", 1)]);
    let mut text = Vec::new();
    gc_report::write_text(&mut text, &h, &planned, Some(&applied), true).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.contains("kept when removing:\n  1 run with a queued or running job"),
        "{text}"
    );
    let mut doc = Vec::new();
    gc_report::write_json(&mut doc, &h, &planned, Some(&applied), None, &[]).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&doc).unwrap();
    assert_eq!(v["kept_when_removing"]["active_job"], 1);
    assert_eq!(v["removed"]["runs"], 1);
    assert_eq!(v["would_remove"]["runs"], 2);
}

#[test]
fn a_dry_run_names_the_exact_arguments_that_remove_what_it_shows() {
    let skipped = BTreeMap::new();
    let h = header(&skipped);
    let mut text = Vec::new();
    gc_report::write_text(&mut text, &h, &section(&["run_a"], &[]), None, false).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(text.contains("cutoff 5000"), "{text}");
    assert!(
        text.contains(
            "To remove exactly this run, run again with --apply --cutoff 5000 --expect-runs 1 --expect-digest 0123456789abcdef."
        ),
        "{text}"
    );
    let mut text = Vec::new();
    gc_report::write_text(&mut text, &h, &section(&["a", "b"], &[]), None, false).unwrap();
    assert!(String::from_utf8(text)
        .unwrap()
        .contains("To remove exactly these 2 runs"));
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-gc-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Once the store has committed, a bundle directory that cannot be read must not hide what was removed.
#[test]
fn what_was_removed_is_reported_when_the_bundle_directory_cannot_be_read_afterwards() {
    let dir = scratch("unreadable");
    let store = SqliteStore::open(&dir.join("db.sqlite")).unwrap();
    let run = store
        .create_run(
            &NewRun {
                target: "svc".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    store
        .finish_run(&run.id, RunState::Passed, 1, 1, None, 2)
        .unwrap();
    let hash = store.put_blob(b"bundle").unwrap();
    store
        .add_failure(
            &NewFailure {
                run_id: run.id.clone(),
                target: "svc".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: hash,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap();
    let plan = store
        .retention_delete(std::slice::from_ref(&run.id), 10, true)
        .unwrap();
    let planned = Section {
        report: plan,
        files: BundleFiles::default(),
        removal: None,
        file_error: None,
    };
    // A path below a regular file cannot be read as a directory.
    let not_a_dir = dir.join("a-file");
    std::fs::write(&not_a_dir, "x").unwrap();
    let applied = apply(&store, &planned, 10, &not_a_dir.join("bundles")).unwrap();
    assert_eq!(applied.report.run_ids, vec![run.id.clone()]);
    assert!(store.get_run(&run.id).unwrap().is_none());
    let problems = problems_of(&planned, Some(&applied));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("could not read the bundle directory"),
        "{problems:?}"
    );
    std::fs::remove_dir_all(dir).ok();
}

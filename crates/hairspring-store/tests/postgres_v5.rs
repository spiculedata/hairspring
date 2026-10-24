//! Schema version 5 on PostgreSQL: the stored results of confirming a failure. Skipped unless
//! `HAIRSPRING_TEST_PG_URL` is set (see `postgres.rs`). The migration only adds a table, so a version 4 schema keeps
//! every row it has.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use hairspring_store::*;
use postgres::Client;

const CURRENT_VERSION: i32 = 5;

fn recorded_version(c: &mut Client, schema: &str) -> i32 {
    c.query_one(
        &format!("SELECT version FROM \"{schema}\".schema_version"),
        &[],
    )
    .unwrap()
    .get(0)
}

fn table_exists(c: &mut Client, schema: &str, table: &str) -> bool {
    c.query_one(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2)",
        &[&schema, &table],
    )
    .unwrap()
    .get(0)
}

fn columns_of(c: &mut Client, schema: &str, table: &str) -> Vec<String> {
    c.query(
        "SELECT column_name || ' ' || data_type FROM information_schema.columns
          WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
        &[&schema, &table],
    )
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// A genuine version 4 schema holding one run and one failure, and nothing of version 5.
fn make_v4_schema(url: &str, schema: &str) -> (String, String) {
    let s = PgStore::connect_in_schema(url, schema, 1).unwrap();
    let run = s
        .create_run(
            &NewRun {
                target: "legacy".into(),
                backend: "hermit".into(),
                seed: 9,
                config_json: "{}".into(),
            },
            1000,
        )
        .unwrap();
    let bundle = s.put_blob(b"bundle").unwrap();
    let failure = s
        .add_failure(
            &NewFailure {
                run_id: run.id.clone(),
                target: "legacy".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            1001,
        )
        .unwrap();
    drop(s);
    admin(url)
        .batch_execute(&format!(
            "DROP TABLE \"{schema}\".failure_checks;
             UPDATE \"{schema}\".schema_version SET version = 4"
        ))
        .unwrap();
    (run.id, failure.id)
}

fn one_confirmation(failure_id: &str) -> NewConfirmation {
    NewConfirmation {
        id: "conf_a".into(),
        failure_id: failure_id.into(),
        runner: "local-1".into(),
        runner_trusted: true,
        checks: vec![NewCheck {
            kind: "same_seed_rerun".into(),
            status: "pass".into(),
            passed: 3,
            failed: 0,
            inconclusive: 0,
            command: "hermit run".into(),
            detail_json: "{}".into(),
            artefacts_json: "[]".into(),
            environment_json: "{}".into(),
            started_at: 1,
            finished_at: 2,
        }],
    }
}

#[test]
fn a_fresh_schema_is_current_with_the_checks_table() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5f");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    assert!(table_exists(&mut c, &schema, "failure_checks"));
}

#[test]
fn a_version_four_schema_migrates_in_place_and_keeps_its_rows() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5m");
    let (run_id, failure_id) = make_v4_schema(&url, &schema);
    let mut c = admin(&url);
    let (runs, failures, trials) = (
        columns_of(&mut c, &schema, "runs"),
        columns_of(&mut c, &schema, "failures"),
        columns_of(&mut c, &schema, "trials"),
    );

    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    assert_eq!(s.get_run(&run_id).unwrap().unwrap().target, "legacy");
    assert!(s.get_failure(&failure_id).unwrap().is_some());
    assert!(s.list_failure_checks(&failure_id).unwrap().is_empty());
    assert_eq!(
        s.add_confirmation(&one_confirmation(&failure_id), 5)
            .unwrap(),
        ConfirmationOutcome::Stored
    );
    drop(s);

    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    assert!(table_exists(&mut c, &schema, "failure_checks"));
    assert_eq!(columns_of(&mut c, &schema, "runs"), runs);
    assert_eq!(columns_of(&mut c, &schema, "failures"), failures);
    assert_eq!(columns_of(&mut c, &schema, "trials"), trials);
}

#[test]
fn opening_a_current_schema_again_changes_nothing() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5i");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    let before = columns_of(&mut c, &schema, "failure_checks");
    for _ in 0..3 {
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    }
    assert_eq!(columns_of(&mut c, &schema, "failure_checks"), before);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
}

#[test]
fn a_schema_newer_than_this_build_is_refused_untouched() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5n");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    c.batch_execute(&format!(
        "UPDATE \"{schema}\".schema_version SET version = {}",
        CURRENT_VERSION + 1
    ))
    .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains('6'), "{err}");
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION + 1);
    assert!(table_exists(&mut c, &schema, "failure_checks"));
}

#[test]
fn a_current_schema_that_lost_the_table_or_its_index_is_reported_not_repaired() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5l");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    c.batch_execute(&format!("DROP TABLE \"{schema}\".failure_checks"))
        .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("failure_checks"), "{err}");
    assert!(!table_exists(&mut c, &schema, "failure_checks"));

    let schema = new_schema("v5x");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    c.batch_execute(&format!("DROP INDEX \"{schema}\".failure_checks_failure"))
        .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("failure_checks_failure"), "{err}");
}

#[test]
fn a_version_four_schema_missing_a_version_four_table_is_refused_not_repaired() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5l4");
    make_v4_schema(&url, &schema);
    let mut c = admin(&url);
    c.batch_execute(&format!("DROP TABLE \"{schema}\".run_coverage"))
        .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("run_coverage"), "{err}");
    assert_eq!(recorded_version(&mut c, &schema), 4, "left at version 4");
    assert!(!table_exists(&mut c, &schema, "failure_checks"));
}

#[test]
fn downgrading_a_version_five_schema_drops_the_checks_and_the_version_four_tables() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5d5");
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    let run = s
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = s.put_blob(b"b").unwrap();
    let failure = s
        .add_failure(
            &NewFailure {
                run_id: run.id,
                target: "t".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap();
    s.add_confirmation(&one_confirmation(&failure.id), 3)
        .unwrap();
    drop(s);

    let dry = downgrade_postgres(
        &url,
        &schema,
        &DowngradeOptions {
            dry_run: true,
            ..DowngradeOptions::default()
        },
    )
    .unwrap();
    assert_eq!((dry.from_version, dry.applied), (5, false));
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    assert!(table_exists(&mut c, &schema, "failure_checks"));

    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (5, 2));
    assert_eq!(recorded_version(&mut c, &schema), 2);
    for t in ["failure_checks", "run_digest_use", "run_coverage"] {
        assert!(!table_exists(&mut c, &schema, t), "{t} is dropped");
    }

    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    assert!(s.list_failure_checks(&failure.id).unwrap().is_empty());
}

#[test]
fn a_version_four_schema_still_downgrades_to_two() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5d4");
    make_v4_schema(&url, &schema);
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (4, 2));
    assert_eq!(recorded_version(&mut admin(&url), &schema), 2);
}

#[test]
fn a_current_schema_whose_checks_table_predates_the_seq_column_is_refused_with_the_fix() {
    let Some(url) = url() else { return };
    let schema = new_schema("v5q");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    c.batch_execute(&format!(
        "ALTER TABLE \"{schema}\".failure_checks DROP COLUMN seq"
    ))
    .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    let text = err.to_string();
    assert!(
        text.contains("seq") && text.contains("failure_checks"),
        "{text}"
    );
    assert!(
        text.contains("drop") || text.contains("DROP"),
        "says how to recreate it: {text}"
    );
}

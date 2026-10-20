//! Schema version 4 on PostgreSQL: the per-run digest allowance and the stored coverage series. Skipped unless
//! `HAIRSPRING_TEST_PG_URL` is set (see `postgres.rs`). The migration only adds tables, so a version 3 schema keeps
//! every row it has.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use hairspring_store::*;
use postgres::Client;

const NEW_TABLES: [&str; 2] = ["run_digest_use", "run_coverage"];

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

/// A genuine version 3 schema: one run, and none of the version 4 tables.
fn make_v3_schema(url: &str, schema: &str) -> String {
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
    drop(s);
    admin(url)
        .batch_execute(&format!(
            "DROP TABLE \"{schema}\".run_digest_use; DROP TABLE \"{schema}\".run_coverage;
             UPDATE \"{schema}\".schema_version SET version = 3"
        ))
        .unwrap();
    run.id
}

#[test]
fn a_fresh_schema_is_at_version_four_with_the_new_tables() {
    let Some(url) = url() else { return };
    let schema = new_schema("v4f");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), 4);
    for t in NEW_TABLES {
        assert!(table_exists(&mut c, &schema, t), "{t}");
    }
}

#[test]
fn a_version_three_schema_migrates_in_place_and_keeps_its_rows() {
    let Some(url) = url() else { return };
    let schema = new_schema("v4m");
    let run_id = make_v3_schema(&url, &schema);
    let mut c = admin(&url);
    let runs_before = columns_of(&mut c, &schema, "runs");
    let trials_before = columns_of(&mut c, &schema, "trials");

    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    assert_eq!(s.get_run(&run_id).unwrap().unwrap().target, "legacy");
    assert_eq!(s.run_digest_use(&run_id).unwrap(), DigestUse::default());
    assert_eq!(s.run_coverage_json(&run_id).unwrap(), None);
    drop(s);

    assert_eq!(recorded_version(&mut c, &schema), 4);
    for t in NEW_TABLES {
        assert!(table_exists(&mut c, &schema, t), "{t}");
    }
    assert_eq!(columns_of(&mut c, &schema, "runs"), runs_before);
    assert_eq!(columns_of(&mut c, &schema, "trials"), trials_before);
}

#[test]
fn a_migrated_schema_takes_ingest_and_keeps_the_allowance() {
    let Some(url) = url() else { return };
    let schema = new_schema("v4i");
    let run_id = make_v3_schema(&url, &schema);
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    s.enqueue_job(&run_id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w1", 600, 10).unwrap().unwrap();
    s.mark_running(&run_id, 10).unwrap();
    let batch = common::explorer::batch_of_digests(&[4, 8], 10);
    s.ingest_trials(&job.id, "w1", true, &batch, 20).unwrap();
    assert_eq!(
        s.run_digest_use(&run_id).unwrap(),
        DigestUse {
            bytes: 6,
            capped_trials: 1
        }
    );
}

#[test]
fn a_version_four_schema_that_lost_a_table_is_reported_not_repaired() {
    let Some(url) = url() else { return };
    for lost in NEW_TABLES {
        let schema = new_schema("v4l");
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
        let mut c = admin(&url);
        c.batch_execute(&format!("DROP TABLE \"{schema}\".{lost}"))
            .unwrap();
        let err = PgStore::connect_in_schema(&url, &schema, 1)
            .err()
            .expect("must refuse");
        assert!(err.to_string().contains(lost), "{err}");
        assert!(!table_exists(&mut c, &schema, lost), "not recreated");
    }
}

#[test]
fn a_version_three_schema_still_downgrades_to_two() {
    let Some(url) = url() else { return };
    let schema = new_schema("v4d3");
    make_v3_schema(&url, &schema);
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (3, 2));
    assert!(report.applied);
    assert_eq!(recorded_version(&mut admin(&url), &schema), 2);
}

#[test]
fn downgrading_a_version_four_schema_drops_the_new_tables() {
    let Some(url) = url() else { return };
    let schema = new_schema("v4d4");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (4, 2));
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), 2);
    for t in NEW_TABLES {
        assert!(!table_exists(&mut c, &schema, t), "{t} is dropped");
    }
}

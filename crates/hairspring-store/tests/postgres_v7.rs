//! Schema version 7 on PostgreSQL: two indexes and nothing else. Skipped unless `HAIRSPRING_TEST_PG_URL` is set (see
//! `postgres.rs`). The migration rewrites no row, every list reads the same with or without the indexes, and the plans
//! of the failures list and of its bounded property filter use them.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use common::v7_lists::{every_list, fill};
use hairspring_store::*;
use postgres::Client;

const CURRENT_VERSION: i32 = 7;
const V7_INDEXES: [&str; 2] = ["failures_created", "run_properties_violation"];

fn recorded_version(c: &mut Client, schema: &str) -> i32 {
    c.query_one(
        &format!("SELECT version FROM \"{schema}\".schema_version"),
        &[],
    )
    .unwrap()
    .get(0)
}

fn index_def(c: &mut Client, schema: &str, index: &str) -> Option<String> {
    c.query_opt(
        "SELECT indexdef FROM pg_indexes WHERE schemaname = $1 AND indexname = $2",
        &[&schema, &index],
    )
    .unwrap()
    .map(|r| r.get(0))
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

fn count(c: &mut Client, schema: &str, table: &str) -> i64 {
    c.query_one(&format!("SELECT COUNT(*) FROM \"{schema}\".{table}"), &[])
        .unwrap()
        .get(0)
}

/// Turns a current schema into what a version 6 build leaves behind: the same tables, without the version 7 indexes.
fn make_v6(url: &str, schema: &str) {
    admin(url)
        .batch_execute(&format!(
            "DROP INDEX \"{schema}\".failures_created;
             DROP INDEX \"{schema}\".run_properties_violation;
             UPDATE \"{schema}\".schema_version SET version = 6"
        ))
        .unwrap();
}

/// The URL with its user replaced by `user` (no password), for the trust-authenticated test server.
fn url_as(url: &str, user: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("a postgres URL");
    let (_, host) = rest.rsplit_once('@').expect("a URL with a user");
    format!("{scheme}://{user}@{host}")
}

fn property(prefix: &str) -> FailureFilter {
    FailureFilter {
        property_prefix: Some(prefix.into()),
        ..FailureFilter::default()
    }
}

#[test]
fn a_fresh_schema_is_current_with_both_indexes() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7f");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    let created = index_def(&mut c, &schema, "failures_created").unwrap();
    assert!(
        created.ends_with("USING btree (created_at, seq)"),
        "{created}"
    );
    let violations = index_def(&mut c, &schema, "run_properties_violation").unwrap();
    assert!(
        violations.ends_with("USING btree (run_id, idx) WHERE (status = 'violation'::text)"),
        "{violations}"
    );
}

#[test]
fn a_version_six_schema_with_data_migrates_in_place_and_every_list_reads_the_same() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7m");
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let fx = fill(&s);
    let with_indexes = every_list(&s, &fx);
    make_v6(&url, &schema);
    let without_indexes = every_list(&s, &fx);
    drop(s);
    let mut c = admin(&url);
    let tables = [
        "runs",
        "failures",
        "run_properties",
        "run_catalog",
        "trials",
    ];
    let columns: Vec<Vec<String>> = tables
        .iter()
        .map(|t| columns_of(&mut c, &schema, t))
        .collect();
    let rows: Vec<i64> = tables.iter().map(|t| count(&mut c, &schema, t)).collect();

    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let migrated = every_list(&s, &fx);
    drop(s);

    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    for index in V7_INDEXES {
        assert!(index_def(&mut c, &schema, index).is_some(), "{index}");
    }
    assert_eq!(
        tables
            .iter()
            .map(|t| columns_of(&mut c, &schema, t))
            .collect::<Vec<_>>(),
        columns
    );
    assert_eq!(
        tables
            .iter()
            .map(|t| count(&mut c, &schema, t))
            .collect::<Vec<_>>(),
        rows
    );
    assert!(with_indexes.iter().any(|(_, ids)| !ids.is_empty()));
    assert_eq!(without_indexes, with_indexes);
    assert_eq!(migrated, with_indexes);
}

#[test]
fn opening_a_migrated_schema_again_changes_nothing() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7i");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v6(&url, &schema);
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    let defs: Vec<Option<String>> = V7_INDEXES
        .iter()
        .map(|i| index_def(&mut c, &schema, i))
        .collect();
    for _ in 0..3 {
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    }
    assert_eq!(
        V7_INDEXES
            .iter()
            .map(|i| index_def(&mut c, &schema, i))
            .collect::<Vec<_>>(),
        defs
    );
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
}

/// An operator may build the indexes beforehand with `CREATE INDEX CONCURRENTLY` (which cannot run inside the
/// migration's transaction) to avoid blocking writes; the migration then finds them and only sets the version.
#[test]
fn indexes_built_beforehand_concurrently_are_kept_and_the_schema_becomes_current() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7c");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v6(&url, &schema);
    let mut c = admin(&url);
    c.batch_execute(&format!(
        "CREATE INDEX CONCURRENTLY failures_created ON \"{schema}\".failures(created_at, seq)"
    ))
    .unwrap();
    c.batch_execute(&format!(
        "CREATE INDEX CONCURRENTLY run_properties_violation ON \"{schema}\".run_properties(run_id, idx) \
         WHERE status = 'violation'"
    ))
    .unwrap();
    let oids: Vec<u32> = V7_INDEXES
        .iter()
        .map(|i| {
            c.query_one(&format!("SELECT '\"{schema}\".{i}'::regclass::oid"), &[])
                .unwrap()
                .get(0)
        })
        .collect();
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    let after: Vec<u32> = V7_INDEXES
        .iter()
        .map(|i| {
            c.query_one(&format!("SELECT '\"{schema}\".{i}'::regclass::oid"), &[])
                .unwrap()
                .get(0)
        })
        .collect();
    assert_eq!(
        after, oids,
        "the indexes built beforehand are the ones kept"
    );
}

#[test]
fn a_current_schema_that_lost_a_version_seven_index_is_reported_not_repaired() {
    let Some(url) = url() else { return };
    for index in V7_INDEXES {
        let schema = new_schema("v7l");
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
        let mut c = admin(&url);
        c.batch_execute(&format!("DROP INDEX \"{schema}\".{index}"))
            .unwrap();
        let err = PgStore::connect_in_schema(&url, &schema, 1)
            .err()
            .expect("must refuse");
        assert!(err.to_string().contains(index), "{err}");
        assert!(index_def(&mut c, &schema, index).is_none(), "not recreated");
    }
}

#[test]
fn a_schema_newer_than_this_build_is_refused_untouched() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7n");
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
    assert!(
        err.to_string().contains(&(CURRENT_VERSION + 1).to_string()),
        "{err}"
    );
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION + 1);
}

/// Creating an index needs ownership of the table. A role that only reads and writes rows can open a current schema
/// (no DDL is issued), but cannot be the one that migrates a version 6 schema: the open fails, the schema stays at
/// version 6, and the owner's open then migrates it.
#[test]
fn only_the_owner_can_migrate_a_version_six_schema_and_any_role_then_opens_it() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7o");
    let role = format!("hs_v7_{}", &new_id("u").replace('-', "_")[..12]);
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v6(&url, &schema);
    let mut a = admin(&url);
    a.batch_execute(&format!(
        "CREATE ROLE \"{role}\" LOGIN;
         GRANT USAGE, CREATE ON SCHEMA \"{schema}\" TO \"{role}\";
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA \"{schema}\" TO \"{role}\";
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA \"{schema}\" TO \"{role}\";"
    ))
    .unwrap();
    let as_role = url_as(&url, &role);
    let refused = PgStore::connect_in_schema(&as_role, &schema, 1)
        .err()
        .map(|e| e.to_string());
    let version_after_refusal = recorded_version(&mut a, &schema);
    let index_after_refusal = index_def(&mut a, &schema, "failures_created");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let opened = PgStore::connect_in_schema(&as_role, &schema, 1)
        .map(|s| s.list_failures_page(&property("db."), 0, 1).map(|_| ()))
        .map_err(|e| e.to_string());
    a.batch_execute(&format!("DROP OWNED BY \"{role}\"; DROP ROLE \"{role}\""))
        .unwrap();
    let refused = refused.expect("a non-owner cannot create the indexes");
    assert!(refused.contains("owner"), "{refused}");
    assert_eq!(version_after_refusal, 6, "nothing half done");
    assert!(index_after_refusal.is_none());
    assert!(matches!(opened, Ok(Ok(()))), "{opened:?}");
    assert_eq!(recorded_version(&mut a, &schema), CURRENT_VERSION);
}

/// Enough rows that the planner's choice is the one a real store gets: 3000 runs, a fifth of them with a violation, and
/// three failures each, then `ANALYZE`.
fn bulk(url: &str, schema: &str) {
    raw(url, schema)
        .batch_execute(
            "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at)
               SELECT 'run_' || lpad(to_hex(n), 16, '0'), 't', 'sim', 'failed', n, '{}', n
               FROM generate_series(1, 3000) n;
             INSERT INTO run_catalog(run_id,idx,prop_id,kind,must_hit,msg,decided)
               SELECT 'run_' || lpad(to_hex(n), 16, '0'), i, 'db.p' || i, 'always', TRUE, 'm', TRUE
               FROM generate_series(1, 3000) n, generate_series(0, 7) i;
             INSERT INTO run_properties(run_id,idx,status,trials,trials_rated,trials_unreported,trials_degraded,
                 trials_perturbing,trials_conflicted,trials_inconclusive_with_failures,trials_claimable,trials_hit,
                 trials_true,trials_false,evals,evals_true,evals_false)
               SELECT 'run_' || lpad(to_hex(n), 16, '0'), i,
                      CASE WHEN i = 0 AND n % 5 = 0 THEN 'violation' ELSE 'pass' END,
                      1,1,0,0,0,0,0,1,1,1,0,1,1,0
               FROM generate_series(1, 3000) n, generate_series(0, 7) i;
             INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at)
               SELECT 'fail_' || lpad(to_hex(f), 16, '0'), 'run_' || lpad(to_hex((f + 2) / 3), 16, '0'), 't', 'm',
                      2, 1, repeat('0', 64), '[]', (f + 2) / 3
               FROM generate_series(1, 9000) f;
             ANALYZE runs; ANALYZE run_catalog; ANALYZE run_properties; ANALYZE failures;",
        )
        .unwrap();
}

#[test]
fn the_failures_list_reads_the_created_index_and_the_property_filter_the_violations() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7p");
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    bulk(&url, &schema);

    let plan = s
        .explain_failures_page(&FailureFilter::default(), 0, 51)
        .unwrap();
    assert!(
        plan.iter()
            .any(|p| p.contains("Index Scan Backward using failures_created on failures")),
        "{plan:?}"
    );
    assert!(!plan.iter().any(|p| p.contains("Sort")), "{plan:?}");

    for prefix in ["db.p0", "zz"] {
        let plan = s.explain_failures_page(&property(prefix), 0, 51).unwrap();
        for wanted in [
            "using runs_created on runs",
            "using run_properties_violation on run_properties",
            "using failures_run on failures",
        ] {
            assert!(
                plan.iter().any(|p| p.contains(wanted)),
                "{prefix} {wanted}: {plan:?}"
            );
        }
        assert!(
            !plan.iter().any(|p| p.contains("Seq Scan")),
            "{prefix}: {plan:?}"
        );
    }
    let named = FailureFilter {
        run_id: Some("run_0000000000000005".into()),
        ..property("db.p0")
    };
    let plan = s.explain_failures_page(&named, 0, 51).unwrap();
    assert!(
        plan.iter()
            .any(|p| p.contains("using run_properties_violation on run_properties")),
        "{plan:?}"
    );
    assert!(!plan.iter().any(|p| p.contains("Seq Scan")), "{plan:?}");
    assert_eq!(
        s.list_failures_page(&named, 0, 10).unwrap().len(),
        3,
        "the named run's three failures"
    );
    assert_eq!(
        s.list_failures_page(&property("db.p0"), 0, 1000)
            .unwrap()
            .len(),
        600,
        "every fifth of the 1000 newest runs has a violation of db.p0, three failures each"
    );

    // Without the indexes the same list is sorted whole.
    make_v6(&url, &schema);
    let plan = s
        .explain_failures_page(&FailureFilter::default(), 0, 51)
        .unwrap();
    assert!(plan.iter().any(|p| p.contains("Sort")), "{plan:?}");
}

#[test]
fn downgrading_a_version_seven_schema_drops_its_indexes() {
    let Some(url) = url() else { return };
    let schema = new_schema("v7d");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!(
        (report.from_version, report.to_version),
        (i64::from(CURRENT_VERSION), 2)
    );
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), 2);
    for index in V7_INDEXES {
        assert!(index_def(&mut c, &schema, index).is_none(), "{index}");
    }
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    for index in V7_INDEXES {
        assert!(index_def(&mut c, &schema, index).is_some(), "{index}");
    }
}

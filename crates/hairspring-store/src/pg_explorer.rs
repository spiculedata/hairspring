//! PostgreSQL side of the explorer's storage needs (schema version 4): the digest allowance counters, the stored
//! coverage series and the reporter lookup. Every statement binds its inputs as parameters.
use crate::digest_allowance::{DigestUse, MAX_COVERAGE_JSON_BYTES, MAX_REPORTER_SEEDS};
use crate::reporters::{FirstReporters, Reported};
use crate::trials::to_db;
use crate::types::{Result, StoreError};
use postgres::GenericClient;
use std::collections::BTreeMap;

pub(crate) fn load_digest_use(c: &mut impl GenericClient, run_id: &str) -> Result<DigestUse> {
    let row = c.query_opt(
        "SELECT bytes, capped_trials FROM run_digest_use WHERE run_id=$1",
        &[&run_id],
    )?;
    Ok(row.map_or_else(DigestUse::default, |r| DigestUse {
        bytes: r.get::<_, i64>(0) as u64,
        capped_trials: r.get::<_, i64>(1) as u64,
    }))
}

pub(crate) fn save_digest_use(
    c: &mut impl GenericClient,
    run_id: &str,
    used: DigestUse,
) -> Result<()> {
    c.execute(
        "INSERT INTO run_digest_use(run_id,bytes,capped_trials) VALUES ($1,$2,$3) \
         ON CONFLICT(run_id) DO UPDATE SET bytes=EXCLUDED.bytes, capped_trials=EXCLUDED.capped_trials",
        &[&run_id, &to_db(used.bytes), &to_db(used.capped_trials)],
    )?;
    Ok(())
}

/// Keeps `json` as the run's coverage series, or removes the run's series when there is none.
pub(crate) fn write_coverage(
    c: &mut impl GenericClient,
    run_id: &str,
    json: Option<&str>,
) -> Result<()> {
    let Some(json) = json else {
        c.execute("DELETE FROM run_coverage WHERE run_id=$1", &[&run_id])?;
        return Ok(());
    };
    if json.len() > MAX_COVERAGE_JSON_BYTES {
        return Err(StoreError::LimitExceeded {
            what: "coverage series bytes",
            limit: MAX_COVERAGE_JSON_BYTES as u64,
        });
    }
    c.execute(
        "INSERT INTO run_coverage(run_id,series_json) VALUES ($1,$2) \
         ON CONFLICT(run_id) DO UPDATE SET series_json=EXCLUDED.series_json",
        &[&run_id, &json],
    )?;
    Ok(())
}

pub(crate) fn run_coverage_json(
    c: &mut impl GenericClient,
    run_id: &str,
) -> Result<Option<String>> {
    Ok(c.query_opt(
        "SELECT series_json FROM run_coverage WHERE run_id=$1",
        &[&run_id],
    )?
    .map(|r| r.get(0)))
}

/// The first reporter (lowest sequence number) of each seed. Reads two columns of the rows `trials_seed` finds.
pub(crate) fn reporters_of(
    c: &mut impl GenericClient,
    run_id: &str,
    seeds: &[u64],
) -> Result<BTreeMap<u64, String>> {
    if seeds.len() > MAX_REPORTER_SEEDS {
        return Err(StoreError::LimitExceeded {
            what: "seeds in one reporter lookup",
            limit: MAX_REPORTER_SEEDS as u64,
        });
    }
    let wanted: Vec<i64> = seeds.iter().map(|s| *s as i64).collect();
    let rows = c.query(
        "SELECT seed, seq, worker_id FROM trials WHERE run_id=$1 AND seed = ANY($2)",
        &[&run_id, &wanted],
    )?;
    let mut first_reporters = FirstReporters::default();
    for r in &rows {
        first_reporters.offer(Reported {
            seed: r.get::<_, i64>(0) as u64,
            seq: r.get::<_, i64>(1) as u64,
            worker: r.get(2),
        });
    }
    Ok(first_reporters.into_workers())
}

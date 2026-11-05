//! Filtered, paged lists of runs and failures (`GET /api/runs`, `GET /api/failures`).
//!
//! Every parameter is checked here, in one place, so REST and any other front end accept the same inputs, and an error
//! names the parameter and never echoes the value. Paging uses the explorer's convention: `cursor` is a position (a
//! plain number) and `next_cursor` is the position of the next page, or absent on the last one. The position counts
//! rows of the filtered list, newest first, so a run or failure stored while a client pages shifts later pages down by
//! one: a client that loads further pages should drop ids it already holds.
//!
//! A failure's confirmation state is derived from its stored checks, not stored on its row, so a `status` filter reads
//! the checks of the failures it passes over. One request reads at most [`STATUS_SCAN_CAP`] failures; if that is not
//! enough to fill a page, the page comes back short with a `next_cursor` that carries on from where it stopped.
use crate::query::{page_size, parse_cursor};
use crate::{summary, FailureSummary, Result, Service, ServiceError, MAX_LIST, MAX_TARGET_LEN};
use hairspring_engine::confirm::FindingStatus;
use hairspring_store::{FailureFilter, RunFilter, RunRecord, RunState};
use serde::Serialize;

/// Rows in a page when the caller gives no limit.
pub const DEFAULT_LIST_PAGE: u32 = 50;
/// Most rows in a page; a larger limit is lowered to this, and a limit of 0 is raised to 1.
pub const MAX_LIST_PAGE: u32 = MAX_LIST;
/// Largest position a cursor may name. Reaching further down means narrowing the list with a filter.
pub const MAX_LIST_OFFSET: u64 = 100_000;
/// Longest property prefix, in bytes (the longest property id).
const MAX_PROPERTY_PREFIX_BYTES: usize = crate::MAX_PREFIX_BYTES;
/// Failures whose checks one status-filtered request reads before it stops and hands back a cursor.
pub const STATUS_SCAN_CAP: u64 = 1000;
/// Failures fetched from the store at a time while scanning for a status.
const STATUS_SCAN_BATCH: u32 = 100;
const RUN_ID_PREFIX: &str = "run_";
const RUN_ID_HEX: usize = 16;
const MODES: [&str; 2] = ["search", "sweep"];

/// The query of `GET /api/runs`, as the raw text received.
#[derive(Debug, Clone, Default)]
pub struct RunListQuery {
    pub state: Option<String>,
    pub target: Option<String>,
    /// The mode the run was stored with: `search` or `sweep`.
    pub mode: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// The query of `GET /api/failures`, as the raw text received.
#[derive(Debug, Clone, Default)]
pub struct FailureListQuery {
    /// A run id: only that run's failures.
    pub run: Option<String>,
    /// A property id prefix: failures of runs in which a property with such an id is stored as a violation.
    pub property: Option<String>,
    /// A confirmation state (`candidate`, `rerun_repeated`, `confirmed`, `verified`, `not_reproducible`,
    /// `control_failed`).
    pub status: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Where a status-filtered scan starts and how far it may go.
struct Scan {
    offset: u64,
    limit: usize,
    /// Failures read before the scan stops.
    cap: u64,
    /// Largest cursor the server accepts; no `next_cursor` is handed out beyond it.
    max_offset: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunPage {
    pub runs: Vec<RunRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailurePage {
    pub failures: Vec<FailureSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

fn invalid(what: &str) -> ServiceError {
    ServiceError::InvalidRequest(format!("`{what}` is not valid"))
}

fn offset_of(cursor: Option<&str>, max_offset: u64) -> Result<u64> {
    match parse_cursor(cursor)? {
        Some(n) if n > max_offset => Err(invalid("cursor")),
        other => Ok(other.unwrap_or(0)),
    }
}

/// Text of 1 to `max` bytes with no control character (a NUL would also be refused by PostgreSQL).
fn plain_text(value: Option<&str>, what: &str, max: usize) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(v) if !v.is_empty() && v.len() <= max && !v.chars().any(char::is_control) => {
            Ok(Some(v.to_owned()))
        }
        Some(_) => Err(invalid(what)),
    }
}

fn run_id(value: Option<&str>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(v)
            if v.strip_prefix(RUN_ID_PREFIX).is_some_and(|hex| {
                hex.len() == RUN_ID_HEX && hex.bytes().all(|b| b.is_ascii_hexdigit())
            }) =>
        {
            Ok(Some(v.to_owned()))
        }
        Some(_) => Err(invalid("run")),
    }
}

fn state(value: Option<&str>) -> Result<Option<RunState>> {
    value
        .map(|v| RunState::parse(v).ok_or_else(|| invalid("state")))
        .transpose()
}

fn mode(value: Option<&str>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(v) if MODES.contains(&v) => Ok(Some(v.to_owned())),
        Some(_) => Err(invalid("mode")),
    }
}

fn finding(value: Option<&str>) -> Result<Option<FindingStatus>> {
    value
        .map(|v| FindingStatus::parse(v).ok_or_else(|| invalid("status")))
        .transpose()
}

/// The cursor for the page that starts at `position`, or none when the server would refuse it: a list is never
/// handed out with a `next_cursor` that the next request cannot send back.
fn next_cursor(position: u64, max_offset: u64) -> Option<String> {
    (position <= max_offset).then(|| position.to_string())
}

impl Service {
    /// One page of runs, newest first, narrowed by the query's filters.
    pub fn run_page(&self, q: &RunListQuery) -> Result<RunPage> {
        self.run_page_capped(q, MAX_LIST_OFFSET)
    }

    fn run_page_capped(&self, q: &RunListQuery, max_offset: u64) -> Result<RunPage> {
        let filter = RunFilter {
            state: state(q.state.as_deref())?,
            target: plain_text(q.target.as_deref(), "target", MAX_TARGET_LEN)?,
            mode: mode(q.mode.as_deref())?,
        };
        let offset = offset_of(q.cursor.as_deref(), max_offset)?;
        let limit = page_size(q.limit, DEFAULT_LIST_PAGE, MAX_LIST_PAGE);
        // One row more than asked for tells whether another page exists.
        let mut runs = self
            .store
            .list_runs_page(&filter, offset, (limit + 1) as u32)?;
        let more = runs.len() > limit;
        runs.truncate(limit);
        Ok(RunPage {
            runs,
            next_cursor: more
                .then(|| next_cursor(offset + limit as u64, max_offset))
                .flatten(),
        })
    }

    /// One page of failures, newest first, narrowed by the query's filters.
    pub fn failure_page(&self, q: &FailureListQuery) -> Result<FailurePage> {
        self.failure_page_scanning(q, STATUS_SCAN_CAP, MAX_LIST_OFFSET)
    }

    fn failure_page_scanning(
        &self,
        q: &FailureListQuery,
        scan_cap: u64,
        max_offset: u64,
    ) -> Result<FailurePage> {
        let filter = FailureFilter {
            run_id: run_id(q.run.as_deref())?,
            property_prefix: plain_text(
                q.property.as_deref(),
                "property",
                MAX_PROPERTY_PREFIX_BYTES,
            )?,
        };
        let want = finding(q.status.as_deref())?;
        let offset = offset_of(q.cursor.as_deref(), max_offset)?;
        let limit = page_size(q.limit, DEFAULT_LIST_PAGE, MAX_LIST_PAGE);
        match want {
            Some(want) => {
                let scan = Scan {
                    offset,
                    limit,
                    cap: scan_cap,
                    max_offset,
                };
                self.failures_in_state(&filter, want, scan)
            }
            None => {
                let mut records =
                    self.store
                        .list_failures_page(&filter, offset, (limit + 1) as u32)?;
                let more = records.len() > limit;
                records.truncate(limit);
                Ok(FailurePage {
                    failures: records.iter().map(summary).collect(),
                    next_cursor: more
                        .then(|| next_cursor(offset + limit as u64, max_offset))
                        .flatten(),
                })
            }
        }
    }

    /// The failures from position `offset` whose state is `want`, reading at most `scan_cap` failures. Each one comes
    /// with its finding, which was worked out to decide.
    fn failures_in_state(
        &self,
        filter: &FailureFilter,
        want: FindingStatus,
        scan: Scan,
    ) -> Result<FailurePage> {
        let Scan {
            offset,
            limit,
            cap: scan_cap,
            max_offset,
        } = scan;
        let mut failures: Vec<FailureSummary> = Vec::new();
        let mut position = offset;
        let mut scanned = 0u64;
        loop {
            let batch = self
                .store
                .list_failures_page(filter, position, STATUS_SCAN_BATCH)?;
            let last_batch = batch.len() < STATUS_SCAN_BATCH as usize;
            for record in &batch {
                let found = self.finding_summary(&record.id)?;
                if found.status == want.as_str() {
                    if failures.len() == limit {
                        // One more match than the page holds: the next page starts at it.
                        return Ok(FailurePage {
                            failures,
                            next_cursor: next_cursor(position, max_offset),
                        });
                    }
                    failures.push(FailureSummary {
                        finding: Some(found),
                        ..summary(record)
                    });
                }
                position += 1;
                scanned += 1;
                if scanned >= scan_cap {
                    return Ok(FailurePage {
                        failures,
                        next_cursor: next_cursor(position, max_offset),
                    });
                }
            }
            if last_batch {
                return Ok(FailurePage {
                    failures,
                    next_cursor: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_store::{NewFailure, NewRun, SqliteStore, Store};
    use std::sync::Arc;

    fn service_with_failures(n: usize) -> Service {
        let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
        let run = store
            .create_run(
                &NewRun {
                    target: "t".into(),
                    backend: "sim".into(),
                    seed: 1,
                    config_json: "{}".into(),
                },
                1,
            )
            .unwrap();
        for i in 0..n {
            store
                .add_failure(
                    &NewFailure {
                        run_id: run.id.clone(),
                        target: "t".into(),
                        message: "m".into(),
                        original_len: 1,
                        shrunk_len: 1,
                        bundle_hash: "0".repeat(64),
                        steps_json: "[]".into(),
                    },
                    10 + i as i64,
                )
                .unwrap();
        }
        Service::new(store)
    }

    fn query(status: &str, cursor: Option<&str>) -> FailureListQuery {
        FailureListQuery {
            status: Some(status.into()),
            cursor: cursor.map(str::to_owned),
            ..FailureListQuery::default()
        }
    }

    #[test]
    fn a_scan_that_reaches_its_cap_stops_short_and_says_where_to_carry_on() {
        let svc = service_with_failures(5);
        let first = svc
            .failure_page_scanning(&query("confirmed", None), 2, u64::MAX)
            .unwrap();
        assert_eq!(
            (first.failures.len(), first.next_cursor.as_deref()),
            (0, Some("2"))
        );
        let second = svc
            .failure_page_scanning(&query("confirmed", Some("2")), 2, u64::MAX)
            .unwrap();
        assert_eq!(second.next_cursor.as_deref(), Some("4"));
        let last = svc
            .failure_page_scanning(&query("confirmed", Some("4")), 2, u64::MAX)
            .unwrap();
        assert_eq!((last.failures.len(), last.next_cursor), (0, None));
    }

    #[test]
    fn a_scan_gathers_matches_across_the_store_batches() {
        let svc = service_with_failures(STATUS_SCAN_BATCH as usize * 2 + 7);
        let page = svc
            .failure_page_scanning(&query("candidate", None), u64::MAX, u64::MAX)
            .unwrap();
        assert_eq!(page.failures.len(), DEFAULT_LIST_PAGE as usize);
        assert_eq!(page.next_cursor.as_deref(), Some("50"));
        let all = svc
            .failure_page_scanning(
                &FailureListQuery {
                    limit: Some(MAX_LIST_PAGE),
                    ..query("candidate", None)
                },
                u64::MAX,
                u64::MAX,
            )
            .unwrap();
        assert_eq!((all.failures.len(), all.next_cursor), (207, None));
    }

    fn service_with_runs(n: usize) -> Service {
        let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
        for i in 0..n {
            store
                .create_run(
                    &NewRun {
                        target: "t".into(),
                        backend: "sim".into(),
                        seed: i as u64,
                        config_json: "{}".into(),
                    },
                    i as i64,
                )
                .unwrap();
        }
        Service::new(store)
    }

    fn run_query(cursor: Option<&str>) -> RunListQuery {
        RunListQuery {
            cursor: cursor.map(str::to_owned),
            limit: Some(2),
            ..RunListQuery::default()
        }
    }

    fn failure_query(cursor: Option<&str>) -> FailureListQuery {
        FailureListQuery {
            cursor: cursor.map(str::to_owned),
            limit: Some(2),
            ..FailureListQuery::default()
        }
    }

    fn status_query_with_limit(cursor: &str, limit: u32) -> FailureListQuery {
        FailureListQuery {
            limit: Some(limit),
            ..query("candidate", Some(cursor))
        }
    }

    #[test]
    fn a_run_page_never_hands_out_a_cursor_the_server_refuses() {
        let svc = service_with_runs(6);
        let first = svc.run_page_capped(&run_query(None), 3).unwrap();
        assert_eq!(first.next_cursor.as_deref(), Some("2"));
        // The page after cursor 2 would start at 4, one past the cap.
        let edge = svc.run_page_capped(&run_query(Some("2")), 3).unwrap();
        assert_eq!((edge.runs.len(), edge.next_cursor), (2, None));
        // A cursor at the cap is accepted, but the page after it would start at 5.
        let at_cap = svc.run_page_capped(&run_query(Some("3")), 3).unwrap();
        assert_eq!((at_cap.runs.len(), at_cap.next_cursor), (2, None));
        assert!(svc.run_page_capped(&run_query(Some("4")), 3).is_err());
    }

    #[test]
    fn a_failure_page_never_hands_out_a_cursor_the_server_refuses() {
        let svc = service_with_failures(6);
        let first = svc
            .failure_page_scanning(&failure_query(None), u64::MAX, 3)
            .unwrap();
        assert_eq!(first.next_cursor.as_deref(), Some("2"));
        let edge = svc
            .failure_page_scanning(&failure_query(Some("2")), u64::MAX, 3)
            .unwrap();
        assert_eq!((edge.failures.len(), edge.next_cursor), (2, None));
        let at_cap = svc
            .failure_page_scanning(&failure_query(Some("3")), u64::MAX, 3)
            .unwrap();
        assert_eq!((at_cap.failures.len(), at_cap.next_cursor), (2, None));
        assert!(svc
            .failure_page_scanning(&failure_query(Some("4")), u64::MAX, 3)
            .is_err());
    }

    #[test]
    fn a_status_scan_cut_short_past_the_cap_hands_out_no_cursor() {
        let svc = service_with_failures(8);
        // Scanning 2 from position 1 stops at 3, which the server still accepts.
        let within = svc
            .failure_page_scanning(&query("confirmed", Some("1")), 2, 3)
            .unwrap();
        assert_eq!(within.next_cursor.as_deref(), Some("3"));
        // Scanning 2 from position 2 would stop at 4, past the cap.
        let beyond = svc
            .failure_page_scanning(&query("confirmed", Some("2")), 2, 3)
            .unwrap();
        assert_eq!(beyond.next_cursor, None);
    }

    #[test]
    fn a_status_page_that_finds_one_match_too_many_past_the_cap_hands_out_no_cursor() {
        let svc = service_with_failures(8);
        // Every failure is a candidate. From 3 the page takes position 3 and the next match is at 4.
        let page = svc
            .failure_page_scanning(&status_query_with_limit("3", 1), u64::MAX, 3)
            .unwrap();
        assert_eq!(page.failures.len(), 1);
        assert_eq!(page.next_cursor, None);
        // With room below the cap the same request does name the next match.
        let roomy = svc
            .failure_page_scanning(&status_query_with_limit("3", 1), u64::MAX, 4)
            .unwrap();
        assert_eq!(roomy.next_cursor.as_deref(), Some("4"));
    }
}

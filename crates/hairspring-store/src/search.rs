//! Bounded search over ids, seeds and fingerprints, shared by both backends so that they cannot disagree.
//!
//! Nothing here reads a table end to end. Every probe is one of two things:
//!
//! * an equality on an indexed column (a run or failure id is a primary key), or
//! * a look at the *recent window*: the newest `recent_runs` runs, taken from the `runs_created` index, joined to the
//!   rows of those runs through an index that starts with the run id (`failures_run`, `trials_seed`,
//!   `trials_schedule`, and the primary key of `run_catalog`).
//!
//! So the cost of a probe is a number the caller chooses, not a function of how much has been stored. A prefix is
//! compared with `substr(column, 1, n) = ?` on the rows the window reaches, never with `LIKE`, so `%` and `_` are
//! themselves on both backends. A value is only ever bound as a parameter; the text of every statement is fixed apart
//! from integers made here.
//!
//! The price is honest and visible: a prefix of a run id, a failure id, a property id, and an exact seed or schedule
//! fingerprint are looked for among the recent runs only. An exact run id or failure id is also looked up directly, so
//! a run of any age is found by its whole id.
use crate::types::Result;

/// Runs in the recent window of the probes on run ids, failure ids, seeds and schedule fingerprints.
pub const SEARCH_RECENT_RUNS: u32 = 1000;
/// Runs in the recent window of the property probe. A run may hold up to 16384 catalog rows, so this window is
/// smaller: at most 100 x 16384 catalog rows are compared, and a typical run holds a few dozen.
pub const SEARCH_PROPERTY_RUNS: u32 = 100;

/// What to look for. The service has already checked the text; the store binds it and never builds SQL from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// An id or id prefix: a run id, a failure id, or a property id.
    pub text: String,
    /// Look for this seed, exactly, as a run's seed and as a trial's seed.
    pub seed: Option<u64>,
    /// Look for this schedule fingerprint, exactly.
    pub fingerprint: Option<String>,
    /// Most rows in any one group.
    pub limit: u32,
    /// Runs in the recent window ([`SEARCH_RECENT_RUNS`] in the server).
    pub recent_runs: u32,
    /// Runs in the property window ([`SEARCH_PROPERTY_RUNS`] in the server).
    pub property_runs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHit {
    pub id: String,
    pub target: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureHit {
    pub id: String,
    pub run_id: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyHit {
    pub prop_id: String,
    /// The newest run in the window whose catalog holds this property id.
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedHit {
    pub run_id: String,
    pub seed: u64,
    /// The trial that ran the seed, or none when the seed is the run's own.
    pub trial: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleHit {
    pub fingerprint: String,
    pub run_id: String,
    pub trial: u64,
}

/// Each group newest first, at most `limit` rows each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchHits {
    pub runs: Vec<RunHit>,
    pub failures: Vec<FailureHit>,
    pub properties: Vec<PropertyHit>,
    pub seeds: Vec<SeedHit>,
    pub schedules: Vec<ScheduleHit>,
}

/// One value of a row, as the backends read it.
#[derive(Debug, Clone)]
pub(crate) enum Cell {
    Text(String),
    Int(i64),
}

pub(crate) type Row = Vec<Cell>;

/// What a statement binds: its one parameter.
#[derive(Debug, Clone)]
pub(crate) enum Arg {
    Text(String),
    Int(i64),
}

/// The columns of the rows a statement returns, one letter each: `t` text, `n` integer.
pub(crate) type Shape = &'static str;

/// What differs between the two backends.
pub(crate) struct Dialect {
    /// The placeholder of the one parameter.
    pub marker: &'static str,
    /// The column that orders rows stored in the same millisecond: the insertion order.
    pub order: &'static str,
    /// Whether `CROSS JOIN LATERAL (...)` is how to say "for each run of the window". SQLite has no `LATERAL`, but a
    /// `CROSS JOIN` always keeps its left table as the outer loop.
    pub lateral: bool,
}

impl Dialect {
    /// Joins `table` (as `alias`) to the window row `r` so that, for each run of the window, `cond` is looked up
    /// through an index on the run id. The join order is fixed: a planner that is free to choose may read the whole
    /// table and hash it against the window, which is the unbounded scan this module exists to avoid. On PostgreSQL
    /// `OFFSET 0` keeps the planner from flattening the subquery back into a free join.
    fn per_run(&self, table: &str, alias: &str, cond: &str) -> String {
        if self.lateral {
            format!(
                "CROSS JOIN LATERAL (SELECT * FROM {table} {alias} WHERE {cond} OFFSET 0) {alias}"
            )
        } else {
            format!("CROSS JOIN {table} {alias} ON {cond}")
        }
    }
}

/// Runs a statement and reads its rows.
pub(crate) trait Execute {
    fn rows(&mut self, sql: &str, arg: &Arg, shape: Shape) -> Result<Vec<Row>>;
}

fn recent(d: &Dialect, window: u32) -> String {
    let order = d.order;
    format!(
        "(SELECT id, target, state, seed, created_at, {order} AS ord FROM runs \
         ORDER BY created_at DESC, {order} DESC LIMIT {window})"
    )
}

fn newest_first(alias: &str) -> String {
    format!("{alias}.created_at DESC, {alias}.ord DESC")
}

fn text(row: &Row, i: usize) -> String {
    match &row[i] {
        Cell::Text(s) => s.clone(),
        Cell::Int(n) => n.to_string(),
    }
}

fn int(row: &Row, i: usize) -> u64 {
    match &row[i] {
        Cell::Int(n) => u64::try_from(*n).unwrap_or(0),
        Cell::Text(_) => 0,
    }
}

/// Whether an id that starts with `fixed` can start with `text`, or `text` can start with `fixed`: a text that does
/// neither cannot match any id of that kind, so no statement is run for it.
fn could_be_id(text: &str, fixed: &str) -> bool {
    text.starts_with(fixed) || fixed.starts_with(text)
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

struct Search<'a, E: Execute> {
    q: &'a SearchQuery,
    d: &'a Dialect,
    exec: &'a mut E,
}

impl<E: Execute> Search<'_, E> {
    fn text_arg(&self) -> Arg {
        Arg::Text(self.q.text.clone())
    }

    fn runs(&mut self) -> Result<Vec<RunHit>> {
        if !could_be_id(&self.q.text, "run_") {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT id, target, state FROM {} r WHERE substr(r.id, 1, {}) = {} ORDER BY {} LIMIT {}",
            recent(self.d, self.q.recent_runs),
            chars(&self.q.text),
            self.d.marker,
            newest_first("r"),
            self.q.limit
        );
        let mut hits = self.run_rows(&sql)?;
        let exact = format!(
            "SELECT id, target, state FROM runs WHERE id = {}",
            self.d.marker
        );
        for hit in self.run_rows(&exact)? {
            if !hits.iter().any(|h| h.id == hit.id) {
                hits.push(hit);
            }
        }
        hits.truncate(self.q.limit as usize);
        Ok(hits)
    }

    fn run_rows(&mut self, sql: &str) -> Result<Vec<RunHit>> {
        let arg = self.text_arg();
        Ok(self
            .exec
            .rows(sql, &arg, "ttt")?
            .iter()
            .map(|r| RunHit {
                id: text(r, 0),
                target: text(r, 1),
                state: text(r, 2),
            })
            .collect())
    }

    fn failures(&mut self) -> Result<Vec<FailureHit>> {
        if !could_be_id(&self.q.text, "fail_") {
            return Ok(Vec::new());
        }
        let order = self.d.order;
        let sql = format!(
            "SELECT f.id, f.run_id, f.target FROM {} r {} ORDER BY f.created_at DESC, f.{order} DESC LIMIT {}",
            recent(self.d, self.q.recent_runs),
            self.d.per_run(
                "failures",
                "f",
                &format!(
                    "f.run_id = r.id AND substr(f.id, 1, {}) = {}",
                    chars(&self.q.text),
                    self.d.marker
                )
            ),
            self.q.limit
        );
        let mut hits = self.failure_rows(&sql)?;
        let exact = format!(
            "SELECT id, run_id, target FROM failures WHERE id = {}",
            self.d.marker
        );
        for hit in self.failure_rows(&exact)? {
            if !hits.iter().any(|h| h.id == hit.id) {
                hits.push(hit);
            }
        }
        hits.truncate(self.q.limit as usize);
        Ok(hits)
    }

    fn failure_rows(&mut self, sql: &str) -> Result<Vec<FailureHit>> {
        let arg = self.text_arg();
        Ok(self
            .exec
            .rows(sql, &arg, "ttt")?
            .iter()
            .map(|r| FailureHit {
                id: text(r, 0),
                run_id: text(r, 1),
                target: text(r, 2),
            })
            .collect())
    }

    /// Each property id once, with the newest run of the window that holds it.
    fn properties(&mut self) -> Result<Vec<PropertyHit>> {
        let sql = format!(
            "SELECT prop_id, run_id FROM (\
               SELECT c.prop_id AS prop_id, r.id AS run_id, r.created_at AS created_at, r.ord AS ord, c.idx AS idx, \
                      ROW_NUMBER() OVER (PARTITION BY c.prop_id ORDER BY {r_newest}) AS n \
               FROM {recent} r {catalog}) x \
             WHERE n = 1 ORDER BY {x_newest}, idx LIMIT {limit}",
            r_newest = newest_first("r"),
            x_newest = newest_first("x"),
            recent = recent(self.d, self.q.property_runs),
            catalog = self.d.per_run(
                "run_catalog",
                "c",
                &format!(
                    "c.run_id = r.id AND substr(c.prop_id, 1, {}) = {}",
                    chars(&self.q.text),
                    self.d.marker
                )
            ),
            limit = self.q.limit
        );
        let arg = self.text_arg();
        Ok(self
            .exec
            .rows(&sql, &arg, "tt")?
            .iter()
            .map(|r| PropertyHit {
                prop_id: text(r, 0),
                run_id: text(r, 1),
            })
            .collect())
    }

    fn seeds(&mut self) -> Result<Vec<SeedHit>> {
        let Some(seed) = self.q.seed else {
            return Ok(Vec::new());
        };
        let Ok(bound) = i64::try_from(seed) else {
            return Ok(Vec::new());
        };
        let arg = Arg::Int(bound);
        let recent = recent(self.d, self.q.recent_runs);
        let own = format!(
            "SELECT r.id, r.seed FROM {recent} r WHERE r.seed = {} ORDER BY {} LIMIT {}",
            self.d.marker,
            newest_first("r"),
            self.q.limit
        );
        let trials = format!(
            "SELECT t.run_id, t.seed, t.seq FROM {recent} r {} ORDER BY {}, t.seq LIMIT {}",
            self.d.per_run(
                "trials",
                "t",
                &format!("t.run_id = r.id AND t.seed = {}", self.d.marker)
            ),
            newest_first("r"),
            self.q.limit
        );
        let mut hits: Vec<SeedHit> = self
            .exec
            .rows(&own, &arg, "tn")?
            .iter()
            .map(|r| SeedHit {
                run_id: text(r, 0),
                seed: int(r, 1),
                trial: None,
            })
            .collect();
        hits.extend(
            self.exec
                .rows(&trials, &arg, "tnn")?
                .iter()
                .map(|r| SeedHit {
                    run_id: text(r, 0),
                    seed: int(r, 1),
                    trial: Some(int(r, 2)),
                }),
        );
        hits.truncate(self.q.limit as usize);
        Ok(hits)
    }

    fn schedules(&mut self) -> Result<Vec<ScheduleHit>> {
        let Some(fingerprint) = &self.q.fingerprint else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT t.run_id, t.seq FROM {} r {} ORDER BY {}, t.seq LIMIT {}",
            recent(self.d, self.q.recent_runs),
            self.d.per_run(
                "trials",
                "t",
                &format!("t.run_id = r.id AND t.schedule_fp = {}", self.d.marker)
            ),
            newest_first("r"),
            self.q.limit
        );
        let arg = Arg::Text(fingerprint.clone());
        Ok(self
            .exec
            .rows(&sql, &arg, "tn")?
            .iter()
            .map(|r| ScheduleHit {
                fingerprint: fingerprint.clone(),
                run_id: text(r, 0),
                trial: int(r, 1),
            })
            .collect())
    }
}

/// Runs every probe of `q` through `exec`.
pub(crate) fn search<E: Execute>(q: &SearchQuery, d: &Dialect, exec: &mut E) -> Result<SearchHits> {
    let mut s = Search { q, d, exec };
    Ok(SearchHits {
        runs: s.runs()?,
        failures: s.failures()?,
        properties: s.properties()?,
        seeds: s.seeds()?,
        schedules: s.schedules()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records each statement and its argument, and answers nothing.
    #[derive(Default)]
    struct Recorder {
        seen: Vec<(String, Arg)>,
    }

    impl Execute for Recorder {
        fn rows(&mut self, sql: &str, arg: &Arg, _shape: Shape) -> Result<Vec<Row>> {
            self.seen.push((sql.to_owned(), arg.clone()));
            Ok(Vec::new())
        }
    }

    const D: Dialect = Dialect {
        marker: "?1",
        order: "rowid",
        lateral: false,
    };

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            seed: Some(42),
            fingerprint: Some("a".repeat(16)),
            limit: 6,
            recent_runs: 1000,
            property_runs: 100,
        }
    }

    fn statements(q: &SearchQuery) -> Vec<(String, Arg)> {
        let mut r = Recorder::default();
        search(q, &D, &mut r).unwrap();
        r.seen
    }

    #[test]
    fn hostile_text_is_only_ever_bound_never_spliced_and_never_a_pattern() {
        let hostile = "run_x' OR 1=1 --%_";
        for (sql, arg) in statements(&query(hostile)) {
            assert!(!sql.contains("OR 1=1") && !sql.contains(hostile), "{sql}");
            assert!(!sql.to_uppercase().contains("LIKE"), "{sql}");
            assert!(!sql.contains(';'), "{sql}");
            if let Arg::Text(t) = arg {
                assert!(t == hostile || t == "a".repeat(16), "{t}");
            }
        }
    }

    #[test]
    fn every_scan_goes_through_a_limited_window_and_every_group_is_limited() {
        for (sql, _) in statements(&query("run_")) {
            let windowed = sql.contains("FROM runs ORDER BY created_at DESC, rowid DESC LIMIT");
            let direct = sql.contains("WHERE id = ?1");
            assert!(windowed || direct, "an unbounded read: {sql}");
            assert!(direct || sql.matches("LIMIT").count() >= 2, "{sql}");
        }
    }

    #[test]
    fn a_table_is_only_ever_reached_for_one_run_of_the_window_at_a_time() {
        for lateral in [false, true] {
            let d = Dialect { lateral, ..D };
            let mut r = Recorder::default();
            search(&query("run_"), &d, &mut r).unwrap();
            for (sql, _) in &r.seen {
                let free = sql.replace("CROSS JOIN", "").contains(" JOIN ");
                assert!(!free, "a join the planner may reorder: {sql}");
                if lateral && sql.contains("CROSS JOIN") {
                    assert!(sql.contains("LATERAL") && sql.contains("OFFSET 0"), "{sql}");
                }
            }
        }
    }

    #[test]
    fn a_text_that_cannot_start_an_id_of_a_kind_runs_no_statement_for_it() {
        let seen = statements(&SearchQuery {
            seed: None,
            fingerprint: None,
            ..query("db.commit")
        });
        assert_eq!(seen.len(), 1, "only the property probe: {seen:?}");
        assert!(seen[0].0.contains("run_catalog"));
        assert!(
            could_be_id("r", "run_")
                && could_be_id("run_ab", "run_")
                && !could_be_id("fail_", "run_")
        );
    }

    #[test]
    fn a_seed_too_big_for_the_database_is_a_miss_not_an_error() {
        let seen = statements(&SearchQuery {
            seed: Some(u64::MAX),
            fingerprint: None,
            ..query("zzz")
        });
        assert!(
            seen.iter().all(|(sql, _)| !sql.contains("seed =")),
            "{seen:?}"
        );
    }
}

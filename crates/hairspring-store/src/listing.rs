//! Filters and paging for the lists of runs and failures, shared by both backends so that they cannot disagree.
//!
//! The SQL is built here from fixed text; a filter value is only ever bound as a parameter, and compared with `=` or
//! `substr`, never `LIKE`, so `%` and `_` in a value are themselves on SQLite and on PostgreSQL alike. Paging is by
//! offset over a total order (the stored time, then the order of insertion), so the same filter pages the same way on
//! either backend.
//!
//! The property filter is bounded: it looks for violations in the newest [`FAILURE_PROPERTY_RUNS`] runs only (or in the
//! one run named by the run filter), never in every run ever stored. Each run of that window is one seek into the
//! partial index of violations (`run_properties_violation`, schema version 7), so its cost depends on the window and on
//! the violations those runs hold, not on how many failures or runs the database keeps.
use crate::types::RunState;

/// Runs whose stored violations the property filter of the failures list looks at: the newest this many runs. A
/// failure of an older run is not matched by a property prefix, and the service says so in its answer.
pub const FAILURE_PROPERTY_RUNS: u32 = 1000;

/// Which runs to list. Every field that is set must match; a target is an exact, case-sensitive name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunFilter {
    pub state: Option<RunState>,
    pub target: Option<String>,
    /// The stored mode, `search` or `sweep`.
    pub mode: Option<String>,
}

/// Which failures to list. Every field that is set must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureFilter {
    pub run_id: Option<String>,
    /// A failure holds no property id of its own. This matches the failures of runs in which a property whose id starts
    /// with this text is stored as a violation, among the newest `property_runs` runs (or the run of `run_id`, of any
    /// age, when that is set).
    pub property_prefix: Option<String>,
    /// How many of the newest runs the property filter looks at ([`FAILURE_PROPERTY_RUNS`] unless a test lowers it).
    pub property_runs: u32,
}

impl Default for FailureFilter {
    fn default() -> FailureFilter {
        FailureFilter {
            run_id: None,
            property_prefix: None,
            property_runs: FAILURE_PROPERTY_RUNS,
        }
    }
}

/// A `WHERE` clause (empty, or starting with a space) and the text values its placeholders stand for, in order.
pub(crate) struct Clause {
    pub sql: String,
    pub args: Vec<String>,
}

/// Collects conditions, numbering placeholders with `marker` (`?1` on SQLite, `$1` on PostgreSQL).
struct Builder {
    marker: fn(usize) -> String,
    conditions: Vec<String>,
    args: Vec<String>,
}

impl Builder {
    fn new(marker: fn(usize) -> String) -> Builder {
        Builder {
            marker,
            conditions: Vec::new(),
            args: Vec::new(),
        }
    }

    /// Binds `value` and returns its placeholder.
    fn bind(&mut self, value: &str) -> String {
        self.args.push(value.to_owned());
        (self.marker)(self.args.len())
    }

    fn equals(&mut self, column: &str, value: &str) {
        let marker = self.bind(value);
        self.conditions.push(format!("{column} = {marker}"));
    }

    fn finish(self) -> Clause {
        let sql = if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        };
        Clause {
            sql,
            args: self.args,
        }
    }
}

pub(crate) fn run_clause(filter: &RunFilter, marker: fn(usize) -> String) -> Clause {
    let mut b = Builder::new(marker);
    if let Some(state) = filter.state {
        b.equals("state", state.as_str());
    }
    if let Some(target) = &filter.target {
        b.equals("target", target);
    }
    if let Some(mode) = &filter.mode {
        b.equals("mode", mode);
    }
    b.finish()
}

/// What differs between the backends in the failures list.
pub(crate) struct FailureDialect {
    /// Numbers placeholders: `?1` on SQLite, `$1` on PostgreSQL.
    pub marker: fn(usize) -> String,
    /// The column that orders rows stored in the same millisecond: `rowid` on SQLite, `seq` on PostgreSQL. The property
    /// window uses it, so it is the same runs on both backends.
    pub order: &'static str,
    /// Whether to pin the plan of the property filter the PostgreSQL way. Its planner is free to flatten subqueries, and
    /// on small or freshly loaded tables it may hash the window against a scan of every catalogue row, or the matched
    /// runs against a scan of every failure: the unbounded reads the window exists to avoid. So each run of the window
    /// is looked at in a `LATERAL` subquery with a `LIMIT` (which cannot be flattened), and the matched runs are handed
    /// to the failures as an array (`= ANY(ARRAY(...))`, worked out once, then looked up through `failures_run`).
    /// SQLite runs a correlated `EXISTS` once per row of its outer query and an `IN (...)` subquery once, then looks the
    /// result up through `failures_run`, so it needs neither.
    pub postgres: bool,
}

/// The `WHERE` clause of the failures list.
pub(crate) fn failure_clause(filter: &FailureFilter, dialect: &FailureDialect) -> Clause {
    let (marker, order) = (dialect.marker, dialect.order);
    let mut b = Builder::new(marker);
    if let Some(run_id) = &filter.run_id {
        b.equals("failures.run_id", run_id);
    }
    if let Some(prefix) = &filter.property_prefix {
        let window = match &filter.run_id {
            Some(run_id) => format!("SELECT id FROM runs WHERE id = {}", b.bind(run_id)),
            None => format!(
                "SELECT id FROM runs ORDER BY created_at DESC, {order} DESC LIMIT {}",
                filter.property_runs
            ),
        };
        let chars = prefix.chars().count();
        let marker = b.bind(prefix);
        // The window is a separate, uncorrelated subquery, read once: each of its runs is one range of the partial
        // index of violations, and the failures are then read through `failures_run` for the runs that matched.
        let violated = format!(
            "SELECT 1 FROM run_properties p JOIN run_catalog c ON c.run_id = p.run_id AND c.idx = p.idx \
             WHERE p.run_id = w.id AND p.status = 'violation' AND substr(c.prop_id, 1, {chars}) = {marker}"
        );
        b.conditions.push(if dialect.postgres {
            format!(
                "failures.run_id = ANY(ARRAY(SELECT w.id FROM ({window}) w \
                 CROSS JOIN LATERAL ({violated} LIMIT 1) v))"
            )
        } else {
            format!("failures.run_id IN (SELECT w.id FROM ({window}) w WHERE EXISTS ({violated}))")
        });
    }
    b.finish()
}

/// `LIMIT` and `OFFSET` as integer literals: both are numbers made here, never text from a caller.
pub(crate) fn window(offset: u64, limit: u32) -> String {
    format!(" LIMIT {limit} OFFSET {}", offset.min(i64::MAX as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(n: usize) -> String {
        format!("?{n}")
    }

    const SQLITE_LIKE: FailureDialect = FailureDialect {
        marker: question,
        order: "rowid",
        postgres: false,
    };

    #[test]
    fn no_filter_is_no_where_clause() {
        let c = run_clause(&RunFilter::default(), question);
        assert_eq!((c.sql.as_str(), c.args.len()), ("", 0));
    }

    #[test]
    fn values_are_bound_in_order_and_never_spliced_into_the_text() {
        let c = run_clause(
            &RunFilter {
                state: Some(RunState::Failed),
                target: Some("x' OR 1=1 --".into()),
                mode: Some("sweep".into()),
            },
            question,
        );
        assert_eq!(c.sql, " WHERE state = ?1 AND target = ?2 AND mode = ?3");
        assert_eq!(c.args, vec!["failed", "x' OR 1=1 --", "sweep"]);
    }

    #[test]
    fn a_prefix_is_compared_by_characters_not_matched_as_a_pattern() {
        let c = failure_clause(
            &FailureFilter {
                property_prefix: Some("ünï_%".into()),
                ..FailureFilter::default()
            },
            &SQLITE_LIKE,
        );
        assert!(c.sql.contains("substr(c.prop_id, 1, 5) = ?1"), "{}", c.sql);
        assert!(!c.sql.contains("LIKE"));
        assert_eq!(c.args, vec!["ünï_%"]);
    }

    #[test]
    fn the_window_is_clamped_to_what_a_database_accepts() {
        assert_eq!(window(3, 7), " LIMIT 7 OFFSET 3");
        assert_eq!(window(u64::MAX, 1), format!(" LIMIT 1 OFFSET {}", i64::MAX));
    }
}

//! Filters and paging for the lists of runs and failures, shared by both backends so that they cannot disagree.
//!
//! The SQL is built here from fixed text; a filter value is only ever bound as a parameter, and compared with `=` or
//! `substr`, never `LIKE`, so `%` and `_` in a value are themselves on SQLite and on PostgreSQL alike. Paging is by
//! offset over a total order (the stored time, then the order of insertion), so the same filter pages the same way on
//! either backend.
use crate::types::RunState;

/// Which runs to list. Every field that is set must match; a target is an exact, case-sensitive name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunFilter {
    pub state: Option<RunState>,
    pub target: Option<String>,
    /// The stored mode, `search` or `sweep`.
    pub mode: Option<String>,
}

/// Which failures to list. Every field that is set must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureFilter {
    pub run_id: Option<String>,
    /// A failure holds no property id of its own. This matches the failures of runs in which a property whose id starts
    /// with this text is stored as a violation.
    pub property_prefix: Option<String>,
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

pub(crate) fn failure_clause(filter: &FailureFilter, marker: fn(usize) -> String) -> Clause {
    let mut b = Builder::new(marker);
    if let Some(run_id) = &filter.run_id {
        b.equals("failures.run_id", run_id);
    }
    if let Some(prefix) = &filter.property_prefix {
        let chars = prefix.chars().count();
        let marker = b.bind(prefix);
        b.conditions.push(format!(
            "EXISTS (SELECT 1 FROM run_catalog c JOIN run_properties p ON p.run_id = c.run_id AND p.idx = c.idx \
             WHERE c.run_id = failures.run_id AND p.status = 'violation' AND substr(c.prop_id, 1, {chars}) = {marker})"
        ));
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
                run_id: None,
                property_prefix: Some("ünï_%".into()),
            },
            question,
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

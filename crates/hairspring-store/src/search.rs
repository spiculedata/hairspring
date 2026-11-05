//! Bounded search over ids, seeds and fingerprints, shared by both backends so that they cannot disagree.
//!
//! Nothing here reads a table end to end. Every probe is one of two things:
//!
//! * an equality on an indexed column (a run or failure id is a primary key), or
//! * a look at the *recent window*: the newest `recent_runs` runs, taken from the `runs_created` index, joined to the
//!   rows of those runs through an index that starts with the run id (`failures_run`, `trials_seed`,
//!   `trials_schedule`), or
//! * for property ids, the newest `property_runs` runs, each read on its own as one range of the
//!   `(run_id, prop_id)` index of `run_catalog`, with its own `LIMIT`.
//!
//! So the cost of a probe is a number the caller chooses, not a function of how much has been stored. A prefix is
//! compared with `substr(column, 1, n) = ?` on the rows the window reaches, never with `LIKE`, so `%` and `_` are
//! themselves on both backends. A value is only ever bound as a parameter; the text of every statement is fixed apart
//! from integers made here.
//!
//! The price is honest and visible: a prefix of a run id, a failure id, a property id, and an exact seed or schedule
//! fingerprint are looked for among the recent runs only. An exact run id or failure id is also looked up directly, so
//! a run of any age is found by its whole id. Text that is plainly another kind of id (a whole run or failure id,
//! digits, or a fingerprint) is not looked for as a property id.
use crate::types::Result;

/// Runs in the recent window of the probes on run ids, failure ids, seeds and schedule fingerprints.
pub const SEARCH_RECENT_RUNS: u32 = 1000;
/// Runs in the recent window of the property probe. A guest decides how many catalog rows a run holds (up to 16384),
/// so this window is small and each of its runs is read on its own with a `LIMIT`: SQLite seeks one range of the
/// `(run_id, prop_id)` index per run; PostgreSQL, whose index follows the database collation, reads at most the
/// run's own index entries (so at most 10 x 16384 in all) and keeps the first few.
pub const SEARCH_PROPERTY_RUNS: u32 = 10;

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunHit {
    pub id: String,
    pub target: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SeedHit {
    pub run_id: String,
    pub seed: u64,
    /// The trial that ran the seed, or none when the seed is the run's own.
    pub trial: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
    /// How many runs' catalogues the property probe read: the runs of its window that exist, or 0 when the text is
    /// shaped like another kind of id and the probe did not run.
    pub property_runs_read: u32,
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
    /// What a numbered placeholder starts with: `?` or `$`.
    pub marker: &'static str,
    /// The column that orders rows stored in the same millisecond: the insertion order.
    pub order: &'static str,
    /// Whether `CROSS JOIN LATERAL (...)` is how to say "for each run of the window". SQLite has no `LATERAL`, but a
    /// `CROSS JOIN` always keeps its left table as the outer loop.
    pub lateral: bool,
    /// What makes a comparison or an order of text bytewise: nothing on SQLite (TEXT compares bytewise unless told
    /// otherwise), ` COLLATE "C"` on PostgreSQL, whose default collation may follow a locale.
    pub bytewise: &'static str,
}

impl Dialect {
    /// The placeholder of parameter `n`, counted from 1.
    fn mark(&self, n: usize) -> String {
        format!("{}{n}", self.marker)
    }

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
    fn rows(&mut self, sql: &str, args: &[Arg], shape: Shape) -> Result<Vec<Row>>;
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

/// The least text that sorts, bytewise, after every text that starts with `text`, or none when there is no such
/// text. UTF-8 sorts bytewise in code point order, so this is `text` with its last code point that can grow grown to
/// the next code point.
fn prefix_upper(text: &str) -> Option<String> {
    let mut chars: Vec<char> = text.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = (u32::from(last) + 1..=u32::from(char::MAX)).find_map(char::from_u32) {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

fn is_hex_id(text: &str, prefix: &str) -> bool {
    const ID_HEX_DIGITS: usize = 16;
    text.strip_prefix(prefix)
        .is_some_and(|hex| hex.len() == ID_HEX_DIGITS && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether the text is plainly another kind of id: a whole run id or failure id, digits (a seed), or 16 to 64 lower
/// case hex digits (a schedule fingerprint). The property probe does not run for it, so those searches stay cheap.
fn names_another_kind(text: &str) -> bool {
    const FINGERPRINT_DIGITS: std::ops::RangeInclusive<usize> = 16..=64;
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let fingerprint = FINGERPRINT_DIGITS.contains(&text.len())
        && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    digits || fingerprint || is_hex_id(text, "run_") || is_hex_id(text, "fail_")
}

/// The property ids of one run (parameter 1) that start with the text (parameter 2), found as one range of the
/// `(run_id, prop_id)` index: from the text up to, but not including, its [`prefix_upper`] (parameter 3, when
/// `bounded`). `substr` decides; the range only says where to look. At most `limit` rows, in bytewise order.
pub(crate) fn property_statement(d: &Dialect, text: &str, bounded: bool, limit: u32) -> String {
    let col = format!("prop_id{}", d.bytewise);
    let upper = if bounded {
        format!(" AND {col} < {}", d.mark(3))
    } else {
        String::new()
    };
    format!(
        "SELECT prop_id FROM run_catalog WHERE run_id = {run} AND {col} >= {text}{upper} \
         AND substr(prop_id, 1, {n}) = {text} ORDER BY {col} LIMIT {limit}",
        run = d.mark(1),
        text = d.mark(2),
        n = chars(text),
    )
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
            self.d.mark(1),
            newest_first("r"),
            self.q.limit
        );
        let mut hits = self.run_rows(&sql)?;
        let exact = format!(
            "SELECT id, target, state FROM runs WHERE id = {}",
            self.d.mark(1)
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
            .rows(sql, &[arg], "ttt")?
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
                    self.d.mark(1)
                )
            ),
            self.q.limit
        );
        let mut hits = self.failure_rows(&sql)?;
        let exact = format!(
            "SELECT id, run_id, target FROM failures WHERE id = {}",
            self.d.mark(1)
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
            .rows(sql, &[arg], "ttt")?
            .iter()
            .map(|r| FailureHit {
                id: text(r, 0),
                run_id: text(r, 1),
                target: text(r, 2),
            })
            .collect())
    }

    /// Each property id once, with the newest run of the window that holds it: newest run first, and within a run in
    /// bytewise id order. Each run of the window is read on its own, as one range of the `(run_id, prop_id)` index
    /// with its own `LIMIT`, so no statement sorts or numbers the rows of the whole window. A guest decides how many
    /// catalog rows a run holds (up to 16384), and this keeps that number out of the cost.
    fn properties(&mut self) -> Result<(Vec<PropertyHit>, u32)> {
        if names_another_kind(&self.q.text) {
            return Ok((Vec::new(), 0));
        }
        let window = format!(
            "SELECT id FROM runs ORDER BY created_at DESC, {order} DESC LIMIT {n}",
            order = self.d.order,
            n = self.q.property_runs
        );
        let runs = self.exec.rows(&window, &[], "t")?;
        let upper = prefix_upper(&self.q.text);
        let sql = property_statement(self.d, &self.q.text, upper.is_some(), self.q.limit);
        let mut hits: Vec<PropertyHit> = Vec::new();
        for run in &runs {
            let run_id = text(run, 0);
            let mut args = vec![Arg::Text(run_id.clone()), self.text_arg()];
            args.extend(upper.clone().map(Arg::Text));
            for row in self.exec.rows(&sql, &args, "t")? {
                let prop_id = text(&row, 0);
                if !hits.iter().any(|h| h.prop_id == prop_id) {
                    hits.push(PropertyHit {
                        prop_id,
                        run_id: run_id.clone(),
                    });
                }
            }
        }
        hits.truncate(self.q.limit as usize);
        let read = u32::try_from(runs.len()).unwrap_or(u32::MAX);
        Ok((hits, read))
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
            self.d.mark(1),
            newest_first("r"),
            self.q.limit
        );
        let trials = format!(
            "SELECT t.run_id, t.seed, t.seq FROM {recent} r {} ORDER BY {}, t.seq LIMIT {}",
            self.d.per_run(
                "trials",
                "t",
                &format!("t.run_id = r.id AND t.seed = {}", self.d.mark(1))
            ),
            newest_first("r"),
            self.q.limit
        );
        let mut hits: Vec<SeedHit> = self
            .exec
            .rows(&own, std::slice::from_ref(&arg), "tn")?
            .iter()
            .map(|r| SeedHit {
                run_id: text(r, 0),
                seed: int(r, 1),
                trial: None,
            })
            .collect();
        hits.extend(
            self.exec
                .rows(&trials, &[arg], "tnn")?
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
                &format!("t.run_id = r.id AND t.schedule_fp = {}", self.d.mark(1))
            ),
            newest_first("r"),
            self.q.limit
        );
        let arg = Arg::Text(fingerprint.clone());
        Ok(self
            .exec
            .rows(&sql, &[arg], "tn")?
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
    let runs = s.runs()?;
    let failures = s.failures()?;
    let (properties, property_runs_read) = s.properties()?;
    Ok(SearchHits {
        runs,
        failures,
        properties,
        seeds: s.seeds()?,
        schedules: s.schedules()?,
        property_runs_read,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records each statement and its argument, and answers nothing.
    #[derive(Default)]
    struct Recorder {
        seen: Vec<(String, Vec<Arg>)>,
    }

    impl Execute for Recorder {
        fn rows(&mut self, sql: &str, args: &[Arg], _shape: Shape) -> Result<Vec<Row>> {
            self.seen.push((sql.to_owned(), args.to_vec()));
            Ok(Vec::new())
        }
    }

    const D: Dialect = Dialect {
        marker: "?",
        order: "rowid",
        lateral: false,
        bytewise: "",
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

    fn statements(q: &SearchQuery) -> Vec<(String, Vec<Arg>)> {
        let mut r = Recorder::default();
        search(q, &D, &mut r).unwrap();
        r.seen
    }

    #[test]
    fn hostile_text_is_only_ever_bound_never_spliced_and_never_a_pattern() {
        let hostile = "run_x' OR 1=1 --%_";
        for (sql, args) in statements(&query(hostile)) {
            assert!(!sql.contains("OR 1=1") && !sql.contains(hostile), "{sql}");
            assert!(!sql.to_uppercase().contains("LIKE"), "{sql}");
            assert!(!sql.contains(';'), "{sql}");
            for arg in args {
                if let Arg::Text(t) = arg {
                    assert!(t == hostile || t == "a".repeat(16), "{t}");
                }
            }
        }
    }

    #[test]
    fn every_scan_goes_through_a_limited_window_and_every_group_is_limited() {
        for (sql, _) in statements(&query("run_")) {
            let windowed = sql.contains("FROM runs ORDER BY created_at DESC, rowid DESC LIMIT");
            let direct = sql.contains("WHERE id = ?1");
            assert!(windowed || direct, "an unbounded read: {sql}");
            let window_only = sql.starts_with("SELECT id FROM runs ORDER BY");
            assert!(
                direct || window_only || sql.matches("LIMIT").count() >= 2,
                "{sql}"
            );
        }
        let (_, seen) = windowed("run_", 2);
        for (sql, _) in catalogue_reads(&seen) {
            assert!(
                sql.contains("WHERE run_id = ?1") && sql.contains("LIMIT 6"),
                "one run at a time, limited: {sql}"
            );
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
        assert_eq!(seen.len(), 1, "only the property window: {seen:?}");
        assert!(seen[0].0.starts_with("SELECT id FROM runs ORDER BY"));
        assert!(
            could_be_id("r", "run_")
                && could_be_id("run_ab", "run_")
                && !could_be_id("fail_", "run_")
        );
    }

    /// Answers the property window with `runs` run ids and every other statement with nothing.
    struct Window {
        runs: usize,
        seen: Vec<(String, Vec<Arg>)>,
    }

    impl Execute for Window {
        fn rows(&mut self, sql: &str, args: &[Arg], _shape: Shape) -> Result<Vec<Row>> {
            self.seen.push((sql.to_owned(), args.to_vec()));
            if sql.starts_with("SELECT id FROM runs ORDER BY") {
                return Ok((0..self.runs)
                    .map(|i| vec![Cell::Text(format!("run_{i}"))])
                    .collect());
            }
            Ok(Vec::new())
        }
    }

    fn windowed(text: &str, runs: usize) -> (SearchHits, Vec<(String, Vec<Arg>)>) {
        let mut w = Window {
            runs,
            seen: Vec::new(),
        };
        let hits = search(
            &SearchQuery {
                seed: None,
                fingerprint: None,
                ..query(text)
            },
            &D,
            &mut w,
        )
        .unwrap();
        (hits, w.seen)
    }

    fn catalogue_reads(seen: &[(String, Vec<Arg>)]) -> Vec<&(String, Vec<Arg>)> {
        seen.iter()
            .filter(|(s, _)| s.contains("run_catalog"))
            .collect()
    }

    #[test]
    fn the_property_probe_reads_each_run_through_one_bounded_range_and_never_sorts_the_window() {
        let (hits, seen) = windowed("db.", 3);
        assert!(
            seen.iter()
                .all(|(s, _)| !s.contains("OVER") && !s.contains("ROW_NUMBER")),
            "no window function: {seen:?}"
        );
        let reads = catalogue_reads(&seen);
        assert_eq!(reads.len(), 3, "one read per run of the window: {seen:?}");
        for (i, (sql, args)) in reads.iter().enumerate() {
            assert!(sql.contains("WHERE run_id = ?1"), "{sql}");
            assert!(
                sql.contains("prop_id >= ?2 AND prop_id < ?3"),
                "a range on (run_id, prop_id): {sql}"
            );
            assert!(sql.ends_with("ORDER BY prop_id LIMIT 6"), "{sql}");
            let texts: Vec<String> = args
                .iter()
                .map(|a| match a {
                    Arg::Text(t) => t.clone(),
                    Arg::Int(n) => n.to_string(),
                })
                .collect();
            assert_eq!(texts, vec![format!("run_{i}"), "db.".into(), "db/".into()]);
        }
        assert_eq!(hits.property_runs_read, 3);
    }

    #[test]
    fn text_shaped_like_another_kind_of_id_skips_the_property_probe() {
        for other in [
            "run_0123456789abcdef",
            "fail_0123456789abcdef",
            "42",
            "0123456789abcdef",
            &"f".repeat(64),
        ] {
            let (hits, seen) = windowed(other, 3);
            assert!(catalogue_reads(&seen).is_empty(), "{other}: {seen:?}");
            assert!(
                !seen
                    .iter()
                    .any(|(s, _)| s.starts_with("SELECT id FROM runs ORDER BY")),
                "{other}: not even the window is read"
            );
            assert_eq!(hits.property_runs_read, 0, "{other}");
        }
        for property in ["run_0123", "fail_x", "db.commit", "4x", "0123456789abcdeF"] {
            let (hits, seen) = windowed(property, 2);
            assert_eq!(catalogue_reads(&seen).len(), 2, "{property}");
            assert_eq!(hits.property_runs_read, 2, "{property}");
        }
    }

    #[test]
    fn the_property_window_reads_only_as_many_runs_as_exist() {
        let (hits, seen) = windowed("p", 0);
        assert!(catalogue_reads(&seen).is_empty());
        assert_eq!(hits.property_runs_read, 0);
    }

    #[test]
    fn the_upper_bound_of_a_prefix_is_the_least_text_after_every_text_that_starts_with_it() {
        assert_eq!(prefix_upper("db.").as_deref(), Some("db/"));
        assert_eq!(prefix_upper("z").as_deref(), Some("{"));
        assert_eq!(prefix_upper("a\u{10FFFF}").as_deref(), Some("b"));
        assert_eq!(prefix_upper("a\u{D7FF}").as_deref(), Some("a\u{E000}"));
        assert_eq!(prefix_upper("\u{10FFFF}"), None);
        assert_eq!(prefix_upper(""), None);
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

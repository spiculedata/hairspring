//! Types of the offline downgrade from schema version 3 back to version 2 (a release note tells operators to back up
//! first; nothing is downgraded automatically). Branch runs, which only exist because of version 3, are kept by
//! default as ordinary runs that no tree refers to; `drop_branch_runs` deletes them in the same transaction.

/// The version a downgrade leads to.
pub const DOWNGRADE_TARGET_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DowngradeOptions {
    /// Delete the branch runs (with their trials, jobs, events and failures) instead of keeping them.
    pub drop_branch_runs: bool,
    /// Report what would happen and change nothing.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DowngradeReport {
    pub from_version: i64,
    pub to_version: i64,
    /// Every branch run the database holds (`mode = 'branch'` or recorded in `tree_branches`), listed before anything
    /// is changed, whether they are kept or dropped.
    pub branch_runs: Vec<String>,
    /// The branch runs that were deleted (empty unless `drop_branch_runs`).
    pub dropped_runs: Vec<String>,
    /// Blobs no remaining row refers to, removed.
    pub dropped_blobs: u64,
    /// False for a dry run and for a database that was already at the target version.
    pub applied: bool,
}

/// The tables version 3 added, in an order that respects their foreign keys when dropping.
pub(crate) const V3_TABLES_DROP_ORDER: [&str; 4] =
    ["tree_branches", "trial_lineage", "trees", "run_identity"];

/// The tables holding a run's rows (each has a `run_id` column), to be emptied before the run row itself.
pub(crate) const RUN_OWNED_TABLES: [&str; 6] = [
    "events",
    "run_properties",
    "run_catalog",
    "trials",
    "failures",
    "jobs",
];

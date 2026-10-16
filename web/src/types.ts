export type RunState = 'queued' | 'running' | 'passed' | 'failed' | 'error' | 'cancelled';

export interface Budget {
  max_steps: number;
  max_trials: number;
  wall_secs: number;
}

export interface Target {
  name: string;
  kind: string;
  description: string;
  budget: Budget;
}

export interface Run {
  id: string;
  target: string;
  backend: string;
  state: RunState;
  seed: number;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
  steps: number;
  trials: number;
  error: string | null;
}

export interface FailureSummary {
  id: string;
  run_id: string;
  target: string;
  message: string;
  original_len: number;
  shrunk_len: number;
  bundle_hash: string;
  created_at: number;
}

export interface Step {
  index: number;
  action_id: string;
  label: string;
  state: string;
}

export interface RunDetail {
  run: Run;
  failures: FailureSummary[];
}

export interface FailureDetail {
  record: FailureSummary;
  steps: Step[];
  bundle: string;
}

export interface ReplayResult {
  reproduced: boolean;
  steps: Step[];
}

// ---- Results explorer (GET /api/runs/{id}/properties and /properties/{idx}) ----
// Mirrors crates/hairspring-service/src/view.rs and crates/hairspring-props/src/honesty.rs. Every string that comes
// from the program under test sits under a `guest` object; every other string is the server's own wording.

export type PropertyStatus = 'violation' | 'unmet' | 'unrated' | 'pass';
export type AssertKind = 'always' | 'always_or_unreachable' | 'sometimes' | 'reachable' | 'unreachable';
/**
 * Which count a rate is of. The server's `RateBasis` enum has no serde rename, so `/properties` sends it as
 * `TrialsFalse`; the snake_case spelling is accepted too, so renaming it on the server cannot break the page.
 */
export type RateBasis = 'TrialsFalse' | 'TrialsTrue' | 'TrialsHit' | 'trials_false' | 'trials_true' | 'trials_hit';

/** The server's rate cell, tagged by `kind`. Fractions are in 0..1. */
export type RateCellData =
  | { kind: 'rate'; basis: RateBasis; k: number; n: number; point: number; lower: number; upper: number }
  | { kind: 'zero_events'; basis: RateBasis; n: number; upper_bound: number }
  | { kind: 'unrated' }
  | { kind: 'not_a_rate'; basis: RateBasis; k: number; n: number };

export interface GuestLoc {
  file: string;
  line: number;
  func: string | null;
}

export interface GuestProperty {
  id: string;
  msg: string;
  loc: GuestLoc | null;
}

export interface PropertyRow {
  idx: number;
  status: PropertyStatus;
  kind: AssertKind;
  must_hit: boolean;
  rate: RateCellData;
  /** The server's own wording of `rate`; the UI renders from `rate` and wording.ts instead. */
  rate_text: string;
  unmet: string | null;
  trials: number;
  trials_rated: number;
  trials_claimable: number;
  trials_unreported: number;
  trials_degraded: number;
  trials_perturbing: number;
  trials_conflicted: number;
  failures_seen_in_inconclusive_trials: number;
  trials_hit: number;
  trials_true: number;
  trials_false: number;
  evals: number;
  /** A decimal string: a seed can exceed 2^53. */
  first_failing_seed: string | null;
  guest: GuestProperty;
}

export interface Excluded {
  degraded: number;
  perturbing: number;
  inconclusive: number;
  report_missing: number;
}

export type RunCompletion =
  | { kind: 'running' | 'complete' | 'stopped_at_first_failure' | 'ended_early' | 'aborted' }
  | { kind: 'time_or_step_limited'; trials_run: number; trials_budget: number };

export interface RunHeader {
  trials_judged: number;
  trials_rated: number | null;
  trials_claimable: number | null;
  distinct_schedules: number;
  excluded: Excluded;
  completion: RunCompletion;
  completion_label: string;
  stopped_at_first_failure: boolean;
  ingest_incomplete: boolean;
  workers_contributing: number;
  workers_untrusted: number;
  unverified_worker_data: boolean;
}

export interface ClaimView {
  allowed: boolean;
  text: string | null;
  chips: string[];
  caveats: string[];
  withheld: string[];
}

export interface PropertyPage {
  run_id: string;
  headline: string;
  header: RunHeader;
  claim: ClaimView;
  notes: string[];
  counts_by_status: Partial<Record<PropertyStatus, number>>;
  total: number;
  next_cursor: string | null;
  properties: PropertyRow[];
  guest_note: string;
}

export interface SeedRef {
  seed: string;
  seq: number;
}

export type SeedsKind = 'failing' | 'hit' | 'never';

export interface PropertyDetail {
  run_id: string;
  property: PropertyRow;
  seeds_kind: SeedsKind;
  seeds: SeedRef[];
  next_cursor: string | null;
  scanned_trials: number;
  completion_label: string;
  guest_note: string;
}

export type PropertySort = 'status' | 'id' | 'rate' | 'evals';

export interface PropertyQuery {
  status?: PropertyStatus;
  kind?: AssertKind;
  q?: string;
  sort?: PropertySort;
  cursor?: string;
  limit?: number;
}

export interface Worker {
  /** Hidden from read-only tokens (it doubles as a lease credential). */
  id?: string;
  name: string;
  last_seen: number;
  registered_at: number;
}

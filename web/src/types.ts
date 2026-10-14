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

export interface Worker {
  /** Hidden from read-only tokens (it doubles as a lease credential). */
  id?: string;
  name: string;
  last_seen: number;
  registered_at: number;
}

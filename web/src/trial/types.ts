/**
 * The shapes GET /api/runs/{id}/trials/{seq} sends. They mirror crates/hairspring-service/src/view.rs and the kept
 * record of crates/hairspring-props/src/{record,trial,event}.rs. Everything under `guest` comes from the program under
 * test and is untrusted text; the numbers and flags outside it are the server's own.
 */
import type { AssertKind, GuestLoc } from '../types';

export type TrialVerdict = 'pass' | 'violation' | 'inconclusive' | 'unusable' | 'infra';

/**
 * How the server classes a trial for display. `pass` is only a rated, clean pass: a pass the engine judged but that was
 * degraded or reported nothing is `excluded`, and is never shown as a pass.
 */
export type TrialClass = 'pass' | 'violation' | 'inconclusive' | 'unusable' | 'infra' | 'excluded';

export interface TrialRow {
  seq: number;
  /** A decimal string: a seed can exceed 2^53. */
  seed: string;
  verdict: TrialVerdict;
  class: TrialClass;
  /** How many properties failed in this trial; `failing_idx` and `guest.failing_ids` hold the first few. */
  failing_count: number;
  failing_idx: readonly number[];
  timed_out: boolean;
  degraded: boolean;
  perturbing: boolean;
  report_missing: boolean;
  rated: boolean;
  claimable: boolean;
  exit_code: number | null;
  signal: number | null;
  behaviour_hash: string | null;
  schedule_fp: string | null;
  syscalls: number;
  work_ops: number;
  elapsed_ms: number;
  worker_trusted: boolean;
  attempt: number;
  kept: string;
  has_detail: boolean;
  has_output: boolean;
  guest: { reason: string; failing_ids: readonly string[] };
}

export interface Witness {
  proc: string;
  thread: string;
  /** The thread's own evaluation ordinal. It says nothing about order across threads. */
  ord: number;
  vals: readonly number[];
}

export interface KeptProperty {
  id: string;
  kind: AssertKind;
  must_hit: boolean;
  msg: string;
  loc: GuestLoc | null;
  hits: number;
  trues: number;
  falses: number;
  first_false: Witness | null;
  first_true: Witness | null;
}

export interface KeptThread {
  thread: string;
  evals: number;
  tape_consumed: number;
  /** False when the key was assigned by claim order and so may differ between schedules. */
  key_stable: boolean;
}

export interface KeptEnd {
  reason: string;
  status: number;
  sites: number;
  evals: number;
  dropped: number;
  threads: number;
}

export interface KeptProc {
  tag: string;
  sdk: string | null;
  sdk_version: string | null;
  catalog_hash: string | null;
  signal_handlers: boolean | null;
  work_ops: number | null;
  end: KeptEnd | null;
  threads: readonly KeptThread[];
}

/** The health counters, all non-negative integers. */
export type KeptHealth = Readonly<Record<string, number>>;

export interface KeptConflict {
  id: string;
  proc: string;
  kept: string;
  rejected: string;
}

export interface KeptRecord {
  v: number;
  /** The property list was left out because the whole record would not fit. */
  truncated: boolean;
  props: {
    procs: readonly KeptProc[];
    properties: readonly KeptProperty[];
    conflicts: readonly KeptConflict[];
    perturbing: boolean;
    health: KeptHealth;
  };
}

export interface ScheduleData {
  sched_turns: number;
  syscalls: number;
  virttime_elapsed: string | null;
  timeslice_stats: string | null;
  per_thread_timeslice: string | null;
}

export type StreamName = 'stdout' | 'stderr';

export interface TrialDetailResponse {
  run_id: string;
  trial: TrialRow;
  guest: { detail: KeptRecord | null; schedule: ScheduleData | null };
  output_streams: readonly StreamName[];
  output_omitted_bytes: { stdout: number; stderr: number };
  guest_note: string;
}

export interface TrialPageResponse {
  run_id: string;
  trials: readonly TrialRow[];
  next_cursor: string | null;
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

function isKeptRecord(v: unknown): boolean {
  if (!isObject(v) || !isObject(v.props)) return false;
  const p = v.props;
  return Array.isArray(p.procs) && Array.isArray(p.properties) && Array.isArray(p.conflicts) && isObject(p.health) && p.procs.every(isProc);
}

function isProc(v: unknown): boolean {
  return isObject(v) && typeof v.tag === 'string' && Array.isArray(v.threads) && v.threads.every((t) => isObject(t) && typeof t.thread === 'string');
}

/** Checks the parts of a trial response the page reads, so a wrong-shaped body is an error and not a crash. */
export function isTrialDetail(v: unknown): v is TrialDetailResponse {
  if (!isObject(v) || !isObject(v.trial) || !isObject(v.guest) || !isObject(v.output_omitted_bytes)) return false;
  if (typeof v.trial.seed !== 'string' || typeof v.trial.verdict !== 'string' || !isObject(v.trial.guest)) return false;
  if (!Array.isArray(v.output_streams)) return false;
  return v.guest.detail === null || v.guest.detail === undefined ? true : isKeptRecord(v.guest.detail);
}

export function isTrialPage(v: unknown): v is TrialPageResponse {
  return isObject(v) && Array.isArray(v.trials) && v.trials.every((t) => isObject(t) && typeof t.seed === 'string' && typeof t.seq === 'number');
}

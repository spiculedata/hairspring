import { trialRow } from '../trial/fixtures';
import type { TrialRow } from '../trial/types';
import type { StripPage } from './api';

/** Builders for tests: a passing trial row (the default is a clean pass, unlike `trialRow`) and a strip page. */
export function seedRow(over: Partial<TrialRow> = {}): TrialRow {
  return trialRow({
    seq: 17,
    seed: '1017',
    verdict: 'pass',
    class: 'pass',
    failing_count: 0,
    failing_idx: [],
    schedule_fp: 'a90b00112233445f',
    work_ops: 3000,
    elapsed_ms: 887,
    kept: 'none',
    has_detail: false,
    has_output: false,
    guest: { reason: '', failing_ids: [] },
    ...over,
  });
}

export function failingRow(over: Partial<TrialRow> = {}): TrialRow {
  return seedRow({
    seq: 16,
    seed: '1016',
    verdict: 'violation',
    class: 'violation',
    failing_count: 1,
    failing_idx: [0],
    schedule_fp: '4c16f00dcafe0001',
    elapsed_ms: 912,
    kept: 'kept',
    has_detail: true,
    guest: { reason: 'always wal.no-lost-commit was false', failing_ids: ['wal.no-lost-commit'] },
    ...over,
  });
}

export function manyRows(n: number): TrialRow[] {
  return Array.from({ length: n }, (_, i) => seedRow({ seq: i, seed: String(1000 + i) }));
}

export function stripPage(over: Partial<StripPage> = {}): StripPage {
  return {
    run_id: 'run_0123456789abcdef',
    first_seq: 0,
    codes: 'ppvpep',
    legend: [
      { code: 'v', name: 'violation' },
      { code: 'p', name: 'pass' },
    ],
    next_cursor: null,
    scanned_trials: 6,
    ...over,
  };
}

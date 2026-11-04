import { describe, expect, it } from 'vitest';
import { trialRow } from '../trial/fixtures';
import type { FailureSummary } from '../types';
import { failureSummary, trialSummary, type TrialSummaryInput } from './summary';

const LINK = 'https://hairspring.example/runs/run_1/trials/16';

const input = (over: Partial<TrialSummaryInput> = {}): TrialSummaryInput => ({
  runId: 'run_0123456789abcdef',
  trial: trialRow(),
  link: LINK,
  completionLabel: 'complete',
  stoppedAtFirstFailure: false,
  ...over,
});

const failure = (over: Partial<FailureSummary> = {}): FailureSummary => ({
  id: 'f_abc',
  run_id: 'run_0123456789abcdef',
  target: 'seats',
  message: 'GUEST-MESSAGE oversold seat 12',
  original_len: 12,
  shrunk_len: 3,
  bundle_hash: 'ab'.repeat(32),
  created_at: 1,
  ...over,
});

describe('trialSummary', () => {
  it('gives the seed, the run, the trial, the failing property and the link, as plain lines', () => {
    const s = trialSummary(input());
    expect(s).toContain('Seed 1016 failed');
    expect(s).toContain('Run: run_0123456789abcdef');
    expect(s).toContain('Trial: 16');
    expect(s).toContain('First failing property: wal.no-lost-commit');
    expect(s).toContain(`Link: ${LINK}`);
    expect(s.split('\n').length).toBeLessThan(12);
  });

  it('says how many more properties failed, and says so when the count is not known', () => {
    expect(trialSummary(input({ trial: trialRow({ failing_count: 3 }) }))).toContain('Failing properties: 3');
    expect(trialSummary(input({ trial: trialRow({ digest_unreadable: true, failing_count: 0, guest: { reason: '', failing_ids: [] } }) }))).toContain('Failing properties: not known');
  });

  it('says a run that stopped at its first failure gives no rate', () => {
    expect(trialSummary(input({ stoppedAtFirstFailure: true }))).toContain('not a rate');
    expect(trialSummary(input())).not.toContain('not a rate');
  });

  it('carries no guest text but the escaped property id: not the reason, and nothing else the program said', () => {
    const t = trialRow({ guest: { reason: 'GUEST-REASON secret detail', failing_ids: ['wal.no-lost-commit'] } });
    const s = trialSummary(input({ trial: t }));
    expect(s).not.toContain('GUEST-REASON');
    expect(s).not.toContain('secret');
  });

  it('escapes a hostile property id and cuts a long one', () => {
    const hostile = `a\u001b[31m<script>‮\nforged: line${'x'.repeat(1000)}`;
    const s = trialSummary(input({ trial: trialRow({ guest: { reason: '', failing_ids: [hostile] } }) }));
    expect(s).toContain('\\u{1B}');
    expect(s).toContain('\\u{202E}');
    expect(s).toContain('\\u{A}');
    expect(s).not.toMatch(/[\u0000-\u0009\u000b-\u001f\u007f‮]/);
    expect(s.split('\n').filter((l) => l.startsWith('forged')).length).toBe(0);
    expect(s.length).toBeLessThan(800);
  });

  it('never says reproduced or confirmed', () => {
    expect(trialSummary(input())).not.toMatch(/reproduced|confirmed/i);
  });
});

describe('failureSummary', () => {
  it('gives the target, run, failure, step counts, bundle hash and the link', () => {
    const s = failureSummary({ record: failure(), link: 'https://hairspring.example/failures/f_abc' });
    expect(s).toContain('Target: seats');
    expect(s).toContain('Run: run_0123456789abcdef');
    expect(s).toContain('Failure: f_abc');
    expect(s).toContain('Shortest failing steps: 3 (from 12)');
    expect(s).toContain(`Bundle hash: ${'ab'.repeat(32)}`);
    expect(s).toContain('Link: https://hairspring.example/failures/f_abc');
  });

  it('leaves out the failure message and every step: they are the program under test', () => {
    const s = failureSummary({ record: failure(), link: 'x' });
    expect(s).not.toContain('GUEST-MESSAGE');
    expect(s).not.toContain('oversold');
  });

  it('says where the failure stands only when the server said, in the server wording', () => {
    expect(failureSummary({ record: failure(), link: 'x' })).not.toMatch(/Confirmation/);
    const withFinding = failure({ finding: { status: 'candidate', checks: 0, environment_pinned: false } });
    expect(failureSummary({ record: withFinding, link: 'x' })).toContain('Confirmation: Candidate');
    expect(failureSummary({ record: failure(), link: 'x' })).not.toMatch(/confirmed|reproduced/i);
  });

  it('escapes a hostile target name', () => {
    const s = failureSummary({ record: failure({ target: 'a\u001bb\nforged: x' }), link: 'x' });
    expect(s).toContain('\\u{1B}');
    expect(s.split('\n').filter((l) => l.startsWith('forged')).length).toBe(0);
  });
});

import { describe, expect, it } from 'vitest';
import type { FindingStatus, RateCellData, RunCompletion, SweepSummary } from '../types';
import { runPanelModel, type RunPanelInput } from './runPanelModel';

const complete: RunCompletion = { kind: 'complete' };
const sweepSummary = (over: Partial<SweepSummary> = {}): SweepSummary => ({
  trials: 100,
  passes: 97,
  violations: 3,
  inconclusive: 0,
  unusable: 0,
  infra: 0,
  first_failing_seeds: ['1016'],
  ...over,
});
const rateCell: RateCellData = { kind: 'rate', basis: 'TrialsFalse', k: 3, n: 100, point: 0.03, lower: 0.0103, upper: 0.0851 };

function input(over: Partial<RunPanelInput> = {}): RunPanelInput {
  return {
    state: 'passed',
    trials: 30,
    completion: complete,
    sweepPartial: false,
    failureCount: 0,
    failedRates: [],
    findings: [],
    ...over,
  };
}

const all = (m: { shows: readonly string[]; doesNotShow: readonly string[] }): string => [...m.shows, ...m.doesNotShow].join('\n');

describe('a run with no sweep summary (the server reports mode "search" for sweeps too, so the panel never asserts a mode)', () => {
  it('that has a failure shows the failure and states no rate, without saying it was a search', () => {
    const m = runPanelModel(input({ state: 'failed', trials: 4, completion: { kind: 'stopped_at_first_failure' }, failureCount: 1, firstFailingSeed: '1016' }));
    expect(m.shows).toContain('A failure: seed 1016 failed.');
    expect(m.doesNotShow).toContain('A sweep rate: the server gave no sweep summary for this run, so no rate is stated here.');
    expect(all(m)).not.toMatch(/\bsearch\b/i);
  });
  it('that has a failure without a seed still says a failure was recorded', () => {
    const m = runPanelModel(input({ state: 'failed', completion: { kind: 'stopped_at_first_failure' }, failureCount: 2 }));
    expect(m.shows).toContain('2 failure records were stored for this run.');
  });
  it('with no failure gives the trial count and denies that the target cannot fail', () => {
    const m = runPanelModel(input());
    expect(m.shows).toEqual(['30 trials ran.']);
    expect(m.doesNotShow).toContain('That the target cannot fail: only those trials were run.');
    expect(all(m)).not.toMatch(/\bsearch\b/i);
  });
  it('that stopped early gives counts only, as the server marks, and never says none failed', () => {
    const m = runPanelModel(
      input({ trials: 2, completion: { kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 }, completionLabel: 'time- or step-limited (2 of 30 trials)' }),
    );
    expect(m.shows).toContain('2 trials ran.');
    expect(m.doesNotShow).toContain('How the run ended, as the server marks it: time- or step-limited (2 of 30 trials). The figures are counts of the trials that ran, not a rate.');
    expect(all(m)).not.toMatch(/none of them failed|did not fail/);
  });
});

describe('a sweep', () => {
  const base = { trials: 100 } as const;
  it('with failures gives the rate for the trials run with the server interval, and no other rate', () => {
    const m = runPanelModel(input({ ...base, state: 'failed', sweep: sweepSummary(), failureCount: 1, failedRates: [rateCell] }));
    expect(m.shows).toContain('100 trials ran: 97 did not fail and 3 failed.');
    expect(m.shows).toContain('3 of 100 rated trials failed.');
    expect(m.shows).toContain('Rate for the trials run: 3/100 · 3.00% · [1.03%–8.51%]');
    expect(m.doesNotShow).toContain('How often it fails outside these trials: the rate is for the trials run.');
  });
  it('with failures but no rate cell loaded states the counts and no rate', () => {
    const m = runPanelModel(input({ ...base, state: 'failed', sweep: sweepSummary(), failureCount: 1 }));
    expect(m.shows).toContain('3 of 100 rated trials failed.');
    expect(all(m)).not.toMatch(/Rate for the trials run/);
  });
  it('with several failed properties points to the table instead of choosing one rate', () => {
    const m = runPanelModel(input({ ...base, state: 'failed', sweep: sweepSummary(), failureCount: 2, failedPropertyCount: 2 }));
    expect(m.shows).toContain('2 properties failed; any rate is in the property table.');
    expect(all(m)).not.toMatch(/Rate for the trials run: /);
  });
  it('with no failure says only that those trials did not fail', () => {
    const m = runPanelModel(input({ ...base, sweep: sweepSummary({ passes: 100, violations: 0, first_failing_seeds: [] }) }));
    expect(m.shows).toContain('Those 100 rated trials did not fail.');
    expect(m.doesNotShow).toContain('That the target cannot fail. A sweep with no failure says only that those trials did not fail.');
  });
  it('lists timeouts, unusable trials and host failures apart from every rate', () => {
    const m = runPanelModel(input({ ...base, sweep: sweepSummary({ trials: 110, passes: 100, violations: 0, inconclusive: 4, unusable: 2, infra: 4, first_failing_seeds: [] }) }));
    expect(m.doesNotShow).toContain('Not in any rate: 4 timed out or were cut short, 2 could not be used, 4 failed in the host.');
  });
  it('that ended in an error or was cancelled gives counts only and no rate', () => {
    const m = runPanelModel(
      input({ ...base, state: 'cancelled', completion: { kind: 'aborted' }, completionLabel: 'aborted', sweep: sweepSummary(), sweepPartial: true, failedRates: [rateCell] }),
    );
    expect(m.shows).toContain('100 trials ran, 3 of them failed.');
    expect(m.doesNotShow).toContain('A rate: the sweep stopped when something happened, so the figures are counts only and stopping then can bias them.');
    expect(all(m)).not.toMatch(/Rate for the trials run/);
  });
  it('without a server summary gives the trial count and no rate', () => {
    const m = runPanelModel(input({ ...base, completion: { kind: 'ended_early' }, completionLabel: 'ended early: trial data is incomplete', failedRates: [rateCell] }));
    expect(m.shows).toEqual(['100 trials ran.']);
    expect(m.doesNotShow).toContain('A sweep rate: the server gave no sweep summary for this run, so no rate is stated here.');
    expect(all(m)).not.toMatch(/Rate for the trials run/);
  });
  it('with a full sweep summary but a stopped-early completion still states counts only', () => {
    const m = runPanelModel(
      input({ ...base, trials: 60, completion: { kind: 'time_or_step_limited', trials_run: 60, trials_budget: 100 }, completionLabel: 'time- or step-limited (60 of 100 trials)', sweep: sweepSummary({ trials: 60, passes: 60, violations: 0, first_failing_seeds: [] }) }),
    );
    expect(all(m)).not.toMatch(/did not fail/);
  });
});

describe('a sweep as the real server reports it (run mode "search", with a sweep summary)', () => {
  it('is treated as a sweep: rate, not the search sentence', () => {
    const m = runPanelModel(input({ state: 'failed', trials: 100, sweep: sweepSummary(), failureCount: 1, failedRates: [rateCell] }));
    expect(m.shows).toContain('Rate for the trials run: 3/100 · 3.00% · [1.03%–8.51%]');
    expect(all(m)).not.toMatch(/a search ends/);
  });
  it('with none failing says only that those trials did not fail', () => {
    const m = runPanelModel(input({ trials: 100, sweep: sweepSummary({ passes: 100, violations: 0, first_failing_seeds: [] }) }));
    expect(m.shows).toContain('Those 100 rated trials did not fail.');
  });
  it('that errored gives counts only', () => {
    const m = runPanelModel(input({ state: 'error', trials: 100, completion: { kind: 'aborted' }, sweep: sweepSummary(), sweepPartial: true }));
    expect(m.shows).toContain('100 trials ran, 3 of them failed.');
    expect(all(m)).not.toMatch(/Rate for the trials run|a search ends/);
  });
  it('does not say none failed when the run is marked failed or holds a failure record', () => {
    const clean = sweepSummary({ passes: 100, violations: 0, first_failing_seeds: [] });
    for (const over of [{ state: 'failed' }, { failureCount: 1 }]) {
      const m = runPanelModel(input({ trials: 100, sweep: clean, ...over }));
      expect(m.shows.join('\n')).not.toMatch(/did not fail/);
      expect(m.shows.length).toBeGreaterThan(0);
    }
  });
  it('states counts only until the property page has loaded', () => {
    const m = runPanelModel(input({ trials: 100, completion: undefined, sweep: sweepSummary(), failedRates: [rateCell] }));
    expect(m.shows).toEqual(['100 trials ran, 3 of them failed.']);
    expect(m.doesNotShow).toContain('How the run ended: its completion has not loaded.');
  });
});

describe('whatever the mode', () => {
  it('says nothing was run for zero trials', () => {
    const m = runPanelModel(input({ trials: 0 }));
    expect(m.shows).toEqual(['No trials were run.']);
    expect(m.doesNotShow).toEqual(['Anything about the target: nothing was run.']);
  });
  it('makes no result claim while the run is going', () => {
    for (const state of ['queued', 'running']) {
      const m = runPanelModel(input({ state, trials: 7 }));
      expect(m.shows).toEqual(['The run has not finished, so nothing here is a result yet.']);
    }
    expect(runPanelModel(input({ completion: { kind: 'running' } })).shows).toEqual(['The run has not finished, so nothing here is a result yet.']);
  });
  it('says an errored or cancelled run has no result', () => {
    for (const state of ['error', 'cancelled']) {
      const m = runPanelModel(input({ state, trials: 5, completion: { kind: 'aborted' }, completionLabel: 'aborted' }));
      expect(m.shows).toContain('The run ended in an error or was cancelled.');
      expect(m.shows).toContain('5 trials ran before it stopped. The figures are counts only.');
      expect(m.doesNotShow).toContain('Whether the target fails: the run did not finish.');
    }
  });
  it('makes no claim for a state it does not know', () => {
    expect(runPanelModel(input({ state: 'mystery' })).shows).toEqual(['The server sent a run state this page does not know, so it makes no claim.']);
  });
  it('makes no claim from a passed run whose completion has not loaded', () => {
    const m = runPanelModel(input({ completion: undefined }));
    expect(m.shows).toEqual(['30 trials ran.']);
    expect(m.doesNotShow).toContain('How the run ended: its completion has not loaded.');
  });
  it('treats incomplete trial data as stopped early', () => {
    const m = runPanelModel(input({ ingestIncomplete: true }));
    expect(all(m)).not.toMatch(/none of them failed/);
  });
  it('lists what was left out of every rate, using the existing wording', () => {
    const m = runPanelModel(input({ excluded: { degraded: 9, perturbing: 0, inconclusive: 13, report_missing: 0 } }));
    expect(m.doesNotShow).toContain('Excluded from every rate: 9 degraded, 13 inconclusive.');
  });
  it('says unverified worker data is included', () => {
    const m = runPanelModel(input({ unverifiedWorkerData: true }));
    expect(m.doesNotShow).toContain('That the trial data is trustworthy: it includes unverified worker data.');
  });
  it('says no failure is confirmed when every failure has a status and none is confirmed or verified', () => {
    const open = runPanelModel(input({ state: 'failed', failureCount: 2, firstFailingSeed: '3', findings: ['candidate', 'rerun_repeated'] }));
    expect(open.doesNotShow).toContain('That a failure is confirmed: no failure of this run has a confirmed or verified status.');
    const future = runPanelModel(input({ state: 'failed', failureCount: 1, firstFailingSeed: '3', findings: ['from_the_future' as FindingStatus] }));
    expect(all(future)).not.toMatch(/That a failure is confirmed/);
    const done = runPanelModel(input({ state: 'failed', failureCount: 2, firstFailingSeed: '3', findings: ['candidate', 'confirmed'] }));
    expect(all(done)).not.toMatch(/That a failure is confirmed/);
    const unknown = runPanelModel(input({ state: 'failed', failureCount: 1, firstFailingSeed: '3', findings: [undefined] }));
    expect(all(unknown)).not.toMatch(/That a failure is confirmed/);
  });
});

describe('no case reads as a pass', () => {
  const PASS_LOOKING = /\b(pass|passed|passes|passing|safe|correct|healthy|no bugs?|always holds|all good|ok)\b/i;
  const NONE_FAILED = /none of them failed|did not fail/;
  const cases: ReadonlyArray<readonly [string, RunPanelInput, boolean]> = [
    ['zero trials', input({ trials: 0 }), false],
    ['zero trials, failed state', input({ state: 'failed', trials: 0 }), false],
    ['unknown state', input({ state: 'mystery' }), false],
    ['unknown completion', input({ completion: undefined }), false],
    ['running', input({ state: 'running' }), false],
    ['queued', input({ state: 'queued' }), false],
    ['error', input({ state: 'error', completion: { kind: 'aborted' } }), false],
    ['cancelled', input({ state: 'cancelled', completion: { kind: 'aborted' } }), false],
    ['ended early', input({ completion: { kind: 'ended_early' } }), false],
    ['digest allowance used up', input({ completion: { kind: 'digest_allowance_used_up' } }), false],
    ['step limited', input({ completion: { kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 } }), false],
    ['ingest incomplete', input({ ingestIncomplete: true }), false],
    ['sweep partial', input({ sweep: sweepSummary({ passes: 100, violations: 0 }), sweepPartial: true }), false],
    ['no summary, failure', input({ state: 'failed', failureCount: 1, firstFailingSeed: '3' }), false],
    ['sweep that rated nothing', input({ sweep: sweepSummary({ trials: 3, passes: 0, violations: 0, infra: 3, first_failing_seeds: [] }) }), false],
    ['no summary, no failure', input(), true],
    ['sweep with no failure', input({ sweep: sweepSummary({ passes: 100, violations: 0, first_failing_seeds: [] }) }), true],
  ];
  it.each(cases)('%s', (_name, i, mayDescribeNoFailure) => {
    const m = runPanelModel(i);
    expect(all(m)).not.toMatch(PASS_LOOKING);
    // Only the does-not-show side may say "cannot fail", and only to deny it.
    expect(m.shows.join('\n')).not.toMatch(/cannot fail/);
    expect(all(m)).not.toMatch(/reproduced/i);
    if (!mayDescribeNoFailure) expect(m.shows.join('\n')).not.toMatch(NONE_FAILED);
    expect(m.shows.length).toBeGreaterThan(0);
    expect(m.doesNotShow.length).toBeGreaterThan(0);
  });
});

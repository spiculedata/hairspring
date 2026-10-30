import { describe, expect, it } from 'vitest';
import type { RunCompletion } from './types';
import { runOutcome } from './runOutcome';

const passed = (trials: number, completion?: RunCompletion) => runOutcome({ state: 'passed', trials, completion });

describe('runOutcome for a passed run whose completion is known', () => {
  it('shows "No failure seen" with the trial count, never "passed" and never a planned count it was not given', () => {
    const o = passed(30, { kind: 'complete' });
    expect(o).toMatchObject({ label: 'No failure seen', scope: '30 trials', tone: 'good' });
    expect(o.scope).not.toMatch(/planned/);
    expect(passed(1, { kind: 'complete' }).scope).toBe('1 trial');
    expect(passed(4790, { kind: 'complete' }).scope).toBe('4,790 trials');
  });

  it('shows a neutral "No trials run" when no trial ran, never a green all-clear', () => {
    for (const completion of [undefined, { kind: 'complete' } as const]) {
      expect(passed(0, completion)).toMatchObject({ label: 'No trials run', tone: 'idle' });
      expect(passed(0, completion).scope).toBeUndefined();
    }
  });

  it('says "Stopped early" with trials run of planned when a limit cut the run short', () => {
    expect(passed(2, { kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 })).toEqual({
      label: 'Stopped early',
      scope: '2 of 30 planned trials',
      tone: 'idle',
      title: 'Server state: passed',
    });
    expect(passed(0, { kind: 'time_or_step_limited', trials_run: 0, trials_budget: 30 }).scope).toBe('0 of 30 planned trials');
  });

  it('does not say "Stopped early" or "40 of 30" when the limit was reached after every planned trial ran', () => {
    for (const [run, budget] of [[30, 30], [40, 30], [1, 1]] as const) {
      const o = passed(run, { kind: 'time_or_step_limited', trials_run: run, trials_budget: budget });
      expect(o.label).toBe('No failure seen');
      expect(o.scope).toBe(`${run} ${run === 1 ? 'trial' : 'trials'}`);
    }
    expect(passed(5, { kind: 'time_or_step_limited', trials_run: 5, trials_budget: 0 }).label).toBe('No failure seen');
  });

  it('says "Stopped early" for ended early, digest allowance used up and aborted', () => {
    for (const kind of ['ended_early', 'digest_allowance_used_up', 'aborted'] as const) {
      expect(passed(5, { kind })).toMatchObject({ label: 'Stopped early', scope: '5 trials', tone: 'idle' });
    }
  });

  it('never calls a run that stopped at its first failure "No failure seen"', () => {
    const o = passed(3, { kind: 'stopped_at_first_failure' });
    expect(o.label).toBe('Stopped at first failure');
    expect(o.tone).toBe('bad');
  });

  it('shows a run still going as running, not as an outcome', () => {
    expect(passed(3, { kind: 'running' })).toMatchObject({ label: 'running', tone: 'busy' });
  });
});

describe('runOutcome for a passed run whose completion is not known', () => {
  it('is neutral: no pass tone and no "No failure seen" until the completion has loaded', () => {
    for (const trials of [1, 30]) {
      const o = passed(trials);
      expect(o.label).toBe('Finished');
      expect(o.tone).toBe('idle');
      expect(o.scope).toBe(`${trials} ${trials === 1 ? 'trial' : 'trials'}`);
    }
  });
});

describe('runOutcome for the other states', () => {
  it('leaves them as the server sent them, without a scope', () => {
    for (const state of ['queued', 'running', 'failed', 'error', 'cancelled']) {
      const o = runOutcome({ state, trials: 3, completion: { kind: 'complete' } });
      expect(o.label).toBe(state);
      expect(o.scope).toBeUndefined();
    }
    expect(runOutcome({ state: 'failed', trials: 3 }).tone).toBe('bad');
    expect(runOutcome({ state: 'running', trials: 3 }).tone).toBe('busy');
  });

  it('keeps the server word for a tooltip', () => {
    expect(runOutcome({ state: 'passed', trials: 3, completion: { kind: 'complete' } }).title).toBe('Server state: passed');
    expect(runOutcome({ state: 'failed', trials: 3 }).title).toBe('Server state: failed');
  });
});

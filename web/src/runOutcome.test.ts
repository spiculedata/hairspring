import { describe, expect, it } from 'vitest';
import { runOutcome } from './runOutcome';

describe('runOutcome', () => {
  it('shows a passed run as "No failure seen", never as passed', () => {
    const o = runOutcome({ state: 'passed', trials: 30 });
    expect(o.label).toBe('No failure seen');
    expect(o.label).not.toMatch(/pass/i);
    expect(o.tone).toBe('good');
  });

  it('gives the scope as "n of n planned trials" when the run is known to have finished its plan', () => {
    const o = runOutcome({ state: 'passed', trials: 30, completion: { kind: 'complete' } });
    expect(o).toMatchObject({ label: 'No failure seen', scope: '30 of 30 planned trials' });
  });

  it('gives only the trial count when completion is not known', () => {
    expect(runOutcome({ state: 'passed', trials: 30 }).scope).toBe('30 trials');
    expect(runOutcome({ state: 'passed', trials: 1 }).scope).toBe('1 trial');
  });

  it('says "Stopped early" with its scope when fewer trials ran than planned', () => {
    const o = runOutcome({ state: 'passed', trials: 2, completion: { kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 } });
    expect(o).toEqual({ label: 'Stopped early', scope: '2 of 30 planned trials', tone: 'idle' });
  });

  it('says "Stopped early" when the run ended early or used up its digest allowance', () => {
    for (const kind of ['ended_early', 'digest_allowance_used_up'] as const) {
      const o = runOutcome({ state: 'passed', trials: 5, completion: { kind } });
      expect(o.label).toBe('Stopped early');
      expect(o.scope).toBe('5 trials');
    }
  });

  it('formats large counts with separators', () => {
    expect(runOutcome({ state: 'passed', trials: 4790, completion: { kind: 'complete' } }).scope).toBe('4,790 of 4,790 planned trials');
  });

  it('leaves every other state as the server sent it, without a scope', () => {
    for (const state of ['queued', 'running', 'failed', 'error', 'cancelled']) {
      const o = runOutcome({ state, trials: 3, completion: { kind: 'complete' } });
      expect(o.label).toBe(state);
      expect(o.scope).toBeUndefined();
    }
    expect(runOutcome({ state: 'failed', trials: 3 }).tone).toBe('bad');
    expect(runOutcome({ state: 'running', trials: 3 }).tone).toBe('busy');
  });
});

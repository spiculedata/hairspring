import { describe, expect, it } from 'vitest';
import { ApiError } from './api';
import { MAX_STEPS, MAX_TRIALS, MAX_WALL_SECS, buildStartRun, enforcedBudget, startErrorText } from './startRun';
import type { Target } from './types';

const target = (over: Partial<Target> = {}): Target => ({
  name: 'pubsub-buggy',
  kind: 'sim',
  description: 'd',
  budget: { max_steps: 200_000, max_trials: 200, wall_secs: 60 },
  ...over,
});
const input = (over: Partial<Parameters<typeof buildStartRun>[0]> = {}) => ({ target: target(), mode: 'search' as const, seedText: '', trialsText: '', ...over });

describe('the server limits the form follows', () => {
  it('are the ones the server clamps to', () => {
    expect([MAX_TRIALS, MAX_STEPS, MAX_WALL_SECS]).toEqual([10_000, 50_000_000, 600]);
  });
});

describe('buildStartRun for a search', () => {
  it('sends the target and the mode, and no budget, so the target keeps its own', () => {
    expect(buildStartRun(input())).toEqual({ ok: true, body: { target: 'pubsub-buggy', mode: 'search' } });
  });
  it('sends the seed when one is given', () => {
    expect(buildStartRun(input({ seedText: ' 42 ' }))).toEqual({ ok: true, body: { target: 'pubsub-buggy', seed: 42, mode: 'search' } });
  });
  it('ignores the trials field, which a search does not show', () => {
    expect(buildStartRun(input({ trialsText: 'abc' }))).toMatchObject({ ok: true });
  });
});

describe('buildStartRun for a sweep', () => {
  it('sends the whole budget with the chosen trials, because a partial budget would replace the target steps and seconds with defaults', () => {
    expect(buildStartRun(input({ mode: 'sweep', trialsText: '40' }))).toEqual({
      ok: true,
      body: { target: 'pubsub-buggy', mode: 'sweep', budget: { max_steps: 200_000, max_trials: 40, wall_secs: 60 } },
    });
  });
  it('sends no budget when trials is empty, so the target budget applies', () => {
    expect(buildStartRun(input({ mode: 'sweep' }))).toEqual({ ok: true, body: { target: 'pubsub-buggy', mode: 'sweep' } });
  });
  it('accepts the smallest and largest number of trials', () => {
    for (const t of ['1', '10000', '10,000'.replace(',', '')]) expect(buildStartRun(input({ mode: 'sweep', trialsText: t }))).toMatchObject({ ok: true });
  });
  it.each([['0'], ['10001'], ['99999999999999999999']])('refuses %s trials with a message that gives the range', (t) => {
    const r = buildStartRun(input({ mode: 'sweep', trialsText: t }));
    expect(r).toEqual({ ok: false, errors: { trials: 'Trials must be between 1 and 10,000' } });
  });
  it.each([['abc'], ['1.5'], ['-3'], ['1e3'], ['+5'], ['4 0']])('refuses %s as not a whole number', (t) => {
    expect(buildStartRun(input({ mode: 'sweep', trialsText: t }))).toEqual({ ok: false, errors: { trials: 'Trials must be a whole number' } });
  });
  it('reports a bad seed and bad trials together, each by field', () => {
    const r = buildStartRun(input({ mode: 'sweep', seedText: 'x', trialsText: '0' }));
    expect(r).toEqual({
      ok: false,
      errors: { seed: 'Seed must be a non-negative whole number', trials: 'Trials must be between 1 and 10,000' },
    });
  });
  it('does not change the target it was given', () => {
    const t = target();
    const copy = structuredClone(t);
    buildStartRun(input({ target: t, mode: 'sweep', trialsText: '5' }));
    expect(t).toEqual(copy);
  });
});

describe('enforcedBudget', () => {
  const b = { max_steps: 200_000, max_trials: 200, wall_secs: 60 };
  it('is the target budget for a search and for a sweep with no trials given', () => {
    expect(enforcedBudget(b, 'search')).toEqual(b);
    expect(enforcedBudget(b, 'sweep')).toEqual(b);
  });
  it('takes the chosen trials for a sweep', () => {
    expect(enforcedBudget(b, 'sweep', 40)).toEqual({ ...b, max_trials: 40 });
  });
  it('is capped at the limits the server clamps to', () => {
    expect(enforcedBudget({ max_steps: 9e9, max_trials: 9e9, wall_secs: 9e9 }, 'sweep')).toEqual({ max_steps: MAX_STEPS, max_trials: MAX_TRIALS, wall_secs: MAX_WALL_SECS });
  });
});

describe('startErrorText', () => {
  it('says plainly that a token the server refuses cannot start runs', () => {
    expect(startErrorText(new ApiError(401, 'unauthorized'))).toBe('The server did not accept the token, so this browser cannot start runs. Enter a token that can.');
    expect(startErrorText(new ApiError(403, 'this endpoint requires the run role'))).toBe(
      'This token cannot start runs. The server says: this endpoint requires the run role.',
    );
  });
  it('gives the server reason for a refused request, such as a full queue', () => {
    expect(startErrorText(new ApiError(400, 'invalid request: the queue is full (5 runs waiting); try again later'))).toBe(
      'The server did not accept this run: invalid request: the queue is full (5 runs waiting); try again later.',
    );
  });
  it('tells a rate limit and a server failure apart', () => {
    expect(startErrorText(new ApiError(429, 'slow down'))).toBe('The server is limiting requests: slow down.');
    expect(startErrorText(new ApiError(500, 'internal error'))).toBe('The server failed to start the run: internal error.');
  });
  it('says the server could not be reached for a network failure', () => {
    expect(startErrorText(new TypeError('Failed to fetch'))).toBe('Could not reach the server: Failed to fetch.');
  });
});

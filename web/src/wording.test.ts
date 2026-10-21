import { describe, expect, it } from 'vitest';
import type { RateBasis, RateCellData } from './types';
import {
  caveatLabel,
  formatCount,
  formatPct,
  noFailuresText,
  rateParts,
  rateText,
  statusLabel,
  truncationMarker,
  unmetSuppressedReason,
} from './wording';

describe('formatCount / formatPct', () => {
  it('groups thousands deterministically', () => {
    expect(formatCount(4790)).toBe('4,790');
    expect(formatCount(1_200_000)).toBe('1,200,000');
    expect(formatCount(0)).toBe('0');
  });
  it('shows two decimals and clamps to 0..100%', () => {
    expect(formatPct(0.0006263)).toBe('0.06%');
    expect(formatPct(1.2)).toBe('100.00%');
    expect(formatPct(-1)).toBe('0.00%');
    expect(formatPct(Number.NaN)).toBe('0.00%');
  });
});

describe('rateText truth table', () => {
  const rate = (k: number, n: number, lo: number, hi: number): RateCellData => ({
    kind: 'rate',
    basis: 'trials_false',
    k,
    n,
    point: k / n,
    lower: lo,
    upper: hi,
  });

  it('k of n: always k/n with its interval', () => {
    expect(rateText(rate(3, 4790, 0.0002, 0.0018))).toBe('3/4,790 · 0.06% · [0.02%–0.18%]');
  });
  it('k = n: still k/n with an interval', () => {
    expect(rateText(rate(16, 16, 0.806, 1))).toBe('16/16 · 100.00% · [80.60%–100.00%]');
  });
  it('k = 0: the exact bound the server sent, worded as the server words it, with 3/n only as a note', () => {
    const cell: RateCellData = { kind: 'zero_events', basis: 'trials_false', n: 4790, upper_bound: 1 - 0.05 ** (1 / 4790) };
    expect(rateText(cell)).toBe('0/4,790, exact one-sided 95% upper bound 0.06% (binomial, 0 of 4,790); about 3/4,790');
  });
  it('k = 0 with few trials: no 3/n note below 30 trials, and never "at most"', () => {
    const cell: RateCellData = { kind: 'zero_events', basis: 'trials_false', n: 10, upper_bound: 1 - 0.05 ** (1 / 10) };
    const text = rateText(cell);
    expect(text).toBe('0/10, exact one-sided 95% upper bound 25.89% (binomial, 0 of 10)');
    expect(text).not.toMatch(/at most|3\/|approx/);
    expect(rateText({ ...cell, n: 29 })).not.toContain('about 3/');
    expect(rateText({ ...cell, n: 30 })).toContain('; about 3/30');
  });
  it('unrated is never 0%', () => {
    const text = rateText({ kind: 'unrated' });
    expect(text).toBe('unrated (0 rated trials)');
    expect(text).not.toMatch(/%/);
  });
  it('a rate cell with n = 0 is treated as unrated, never as a percentage', () => {
    expect(rateText(rate(0, 0, 0, 1))).toBe('unrated (0 rated trials)');
  });
  it('stopped at first failure: counts only, no interval and no 3/N', () => {
    const cell: RateCellData = { kind: 'not_a_rate', basis: 'trials_false', k: 1, n: 16 };
    const text = rateText(cell);
    expect(text).toBe('not a rate (stopped at first failure) · 1 failure in 16 rated trials');
    expect(text).not.toMatch(/%|\[|3\//);
  });
  it('reads the basis as the server sends it (TrialsFalse) and as snake_case', () => {
    const text = (basis: RateBasis) => rateText({ kind: 'not_a_rate', basis, k: 1, n: 16 });
    expect(text('TrialsFalse')).toBe(text('trials_false'));
    expect(text('TrialsFalse')).toContain('1 failure in 16 rated trials');
    expect(rateText({ kind: 'not_a_rate', basis: 'TrialsHit', k: 2, n: 5 })).toContain('2 trials that reached it');
  });
  it('names the event by its basis and pluralises', () => {
    expect(rateText({ kind: 'not_a_rate', basis: 'trials_true', k: 2, n: 1 })).toBe(
      'not a rate (stopped at first failure) · 2 trials where it held in 1 rated trial',
    );
    expect(rateText({ kind: 'not_a_rate', basis: 'trials_hit', k: 1, n: 5 })).toBe(
      'not a rate (stopped at first failure) · 1 trial that reached it in 5 rated trials',
    );
  });
});

describe('fixed phrases', () => {
  it('labels every status, and UNMET names itself as a gap', () => {
    expect(statusLabel('violation')).toBe('VIOLATION');
    expect(statusLabel('pass')).toBe('PASS');
    expect(statusLabel('unrated')).toBe('UNRATED');
    expect(statusLabel('unmet')).toBe('UNMET (coverage gap)');
  });
  it('turns a caveat code into words and never drops an unknown one', () => {
    expect(caveatLabel('single_run')).toBe('single run');
    expect(caveatLabel('some_new_code')).toBe('some new code');
  });
  it('states why UNMET is suppressed', () => {
    expect(unmetSuppressedReason({ kind: 'stopped_at_first_failure' }, false)).toMatch(/stopped at its first failure/);
    expect(unmetSuppressedReason({ kind: 'complete' }, true)).toMatch(/incomplete/);
    expect(unmetSuppressedReason({ kind: 'digest_allowance_used_up' }, false)).toMatch(/incomplete/);
    expect(unmetSuppressedReason({ kind: 'running' }, false)).toMatch(/not finished/);
    expect(
      unmetSuppressedReason({ kind: 'time_or_step_limited', trials_run: 3, trials_budget: 10 }, false, true),
    ).toMatch(/unverified worker data/);
    expect(unmetSuppressedReason({ kind: 'complete' }, false, true)).toBe('no trial could claim it');
  });
  it('marks truncation visibly with the count', () => {
    expect(truncationMarker(88)).toBe('… (88 more characters)');
    expect(truncationMarker(1)).toBe('… (1 more character)');
  });
});

describe('F16: the empty failures text is generated, never a fixed all-clear', () => {
  it('uses the server headline for a finished run', () => {
    expect(noFailuresText({ live: false, headline: 'No trial failed a property in 4,790 rated trials' })).toBe(
      'No trial failed a property in 4,790 rated trials',
    );
    expect(noFailuresText({ live: false, headline: 'Nothing was rated: 0 rated trials' })).toBe('Nothing was rated: 0 rated trials');
  });
  it('does not claim anything while the run is live', () => {
    expect(noFailuresText({ live: true, headline: undefined })).toBe('No failures reported yet.');
  });
  it('does not claim anything when the counts are unavailable', () => {
    const text = noFailuresText({ live: false, headline: undefined });
    expect(text).toBe('No failure records for this run, and its property counts are not available.');
    expect(text).not.toMatch(/No property violations/i);
  });
});

describe('rateParts: the figure is one short line, the qualifier is the rest', () => {
  it('puts k/n (or the counts) in the figure for every kind of cell', () => {
    const rate: RateCellData = { kind: 'rate', basis: 'TrialsFalse', k: 3, n: 4790, point: 0.000626, lower: 0.0002, upper: 0.0018 };
    expect(rateParts(rate)).toEqual({ figure: '3/4,790 · 0.06%', qualifier: '[0.02%–0.18%]' });
    const zero: RateCellData = { kind: 'zero_events', basis: 'TrialsFalse', n: 3, upper_bound: 1 - 0.05 ** (1 / 3) };
    expect(rateParts(zero).figure).toBe('0/3');
    expect(rateParts(zero).qualifier).toBe('exact one-sided 95% upper bound 63.16% (binomial, 0 of 3)');
    expect(rateParts({ kind: 'unrated' })).toEqual({ figure: 'unrated (0 rated trials)', qualifier: undefined });
    expect(rateParts({ kind: 'not_a_rate', basis: 'TrialsFalse', k: 1, n: 3 })).toEqual({
      figure: '1 failure in 3 rated trials',
      qualifier: 'not a rate (stopped at first failure)',
    });
  });

  it('says the same thing as rateText', () => {
    const cell: RateCellData = { kind: 'not_a_rate', basis: 'TrialsTrue', k: 2, n: 9 };
    const { figure, qualifier } = rateParts(cell);
    expect(rateText(cell)).toContain(figure);
    expect(rateText(cell)).toContain(qualifier ?? '');
  });
});

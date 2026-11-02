import { describe, expect, it } from 'vitest';
import type { Run } from '../types';
import { attentionRuns, firstSafeTargetName, recentRuns } from './overviewModel';

const run = (id: string, state: string, created_at: number): Run => ({
  id,
  target: 't',
  backend: 'sim',
  state: state as Run['state'],
  seed: 1,
  created_at,
  started_at: null,
  finished_at: null,
  steps: 0,
  trials: 0,
  error: null,
});

describe('attentionRuns', () => {
  it('keeps only failed and errored runs, newest first, without changing the input', () => {
    const runs = [run('a', 'passed', 5), run('b', 'failed', 1), run('c', 'error', 3), run('d', 'running', 9), run('e', 'cancelled', 8)];
    const copy = [...runs];
    expect(attentionRuns(runs, 10).map((r) => r.id)).toEqual(['c', 'b']);
    expect(runs).toEqual(copy);
  });

  it('does not treat an unknown state as needing attention, and cuts the list at the limit', () => {
    const runs = [run('a', 'weird', 5), run('b', 'failed', 1), run('c', 'failed', 2), run('d', 'failed', 3)];
    expect(attentionRuns(runs, 2).map((r) => r.id)).toEqual(['d', 'c']);
  });
});

describe('recentRuns', () => {
  it('takes the newest few of any state, newest first, as a new array', () => {
    const runs = [run('a', 'passed', 1), run('b', 'failed', 3), run('c', 'running', 2)];
    expect(recentRuns(runs, 2).map((r) => r.id)).toEqual(['b', 'c']);
    expect(runs.map((r) => r.id)).toEqual(['a', 'b', 'c']);
  });

  it('keeps the server order for runs created at the same time', () => {
    const runs = [run('a', 'passed', 1), run('b', 'passed', 1)];
    expect(recentRuns(runs, 5).map((r) => r.id)).toEqual(['a', 'b']);
  });
});

describe('firstSafeTargetName', () => {
  it('is the first name made only of letters, digits, dot, underscore and hyphen', () => {
    expect(firstSafeTargetName([{ name: 'has space' }, { name: 'lock-lease' }])).toBe('lock-lease');
  });
  it('needs a letter or digit first, so a name cannot read as an option or a path', () => {
    expect(firstSafeTargetName([{ name: '--sweep' }, { name: '..' }, { name: '-x' }, { name: '.hidden' }, { name: '_x' }])).toBeUndefined();
    expect(firstSafeTargetName([{ name: '--sweep' }, { name: 'a.b-c_d' }])).toBe('a.b-c_d');
  });
  it('is undefined for no usable name', () => {
    expect(firstSafeTargetName([])).toBeUndefined();
    expect(firstSafeTargetName([{ name: '$(x)' }, { name: '' }])).toBeUndefined();
  });
});

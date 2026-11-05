import { describe, expect, it } from 'vitest';
import {
  MAX_PROPERTY_PREFIX_BYTES,
  MAX_TARGET_BYTES,
  checkPropertyPrefix,
  checkRunId,
  checkTarget,
  failureFiltersFromSearch,
  failureFiltersToSearch,
  hasFailureFilters,
  hasRunFilters,
  runFiltersFromSearch,
  runFiltersToSearch,
} from './filters';

describe('run filters in the address', () => {
  it('read the state, the target and the mode', () => {
    expect(runFiltersFromSearch('?state=failed&target=lock-lease&mode=sweep')).toEqual({ state: 'failed', target: 'lock-lease', mode: 'sweep' });
    expect(runFiltersFromSearch('')).toEqual({});
  });

  it('leave out anything the server would refuse, so a pasted link cannot cause a 400', () => {
    expect(runFiltersFromSearch('?state=done&mode=sweeps&target=')).toEqual({});
    expect(runFiltersFromSearch('?state=FAILED')).toEqual({});
    expect(runFiltersFromSearch(`?target=${'x'.repeat(MAX_TARGET_BYTES + 1)}`)).toEqual({});
    expect(runFiltersFromSearch('?target=a%00b')).toEqual({});
    expect(runFiltersFromSearch('?target=a%0Ab')).toEqual({});
    expect(runFiltersFromSearch('?cursor=5&limit=3&unknown=1')).toEqual({});
  });

  it('keep the longest allowed target and a name with spaces and symbols', () => {
    const longest = 'x'.repeat(MAX_TARGET_BYTES);
    expect(runFiltersFromSearch(`?target=${longest}`)).toEqual({ target: longest });
    expect(runFiltersFromSearch(`?target=${encodeURIComponent('a b&c=d')}`)).toEqual({ target: 'a b&c=d' });
  });

  it('write only the filters that are set, encoded, and read back the same', () => {
    expect(runFiltersToSearch({})).toBe('');
    const filters = { state: 'running', target: 'a b&c=d', mode: 'search' } as const;
    expect(runFiltersToSearch(filters)).toBe('?state=running&target=a+b%26c%3Dd&mode=search');
    expect(runFiltersFromSearch(runFiltersToSearch(filters))).toEqual(filters);
  });

  it('say whether any filter is set', () => {
    expect(hasRunFilters({})).toBe(false);
    expect(hasRunFilters({ mode: 'sweep' })).toBe(true);
    expect(hasRunFilters({ target: 'x' })).toBe(true);
    expect(hasRunFilters({ state: 'queued' })).toBe(true);
  });
});

describe('failure filters in the address', () => {
  const run = 'run_0123456789abcdef';

  it('read the run, the property prefix and the status', () => {
    expect(failureFiltersFromSearch(`?run=${run}&property=db.&status=confirmed`)).toEqual({ run, property: 'db.', status: 'confirmed' });
    expect(failureFiltersFromSearch('')).toEqual({});
  });

  it('leave out anything the server would refuse', () => {
    expect(failureFiltersFromSearch('?run=nope&status=reproduced&property=')).toEqual({});
    expect(failureFiltersFromSearch(`?run=${run}0`)).toEqual({});
    expect(failureFiltersFromSearch('?status=Confirmed')).toEqual({});
    expect(failureFiltersFromSearch(`?property=${'p'.repeat(MAX_PROPERTY_PREFIX_BYTES + 1)}`)).toEqual({});
    expect(failureFiltersFromSearch('?property=a%00')).toEqual({});
    expect(failureFiltersFromSearch('?property=a%09b')).toEqual({});
  });

  it('count the property prefix in bytes, as the server does', () => {
    const fits = 'é'.repeat(MAX_PROPERTY_PREFIX_BYTES / 2);
    expect(checkPropertyPrefix(fits)).toEqual({ value: fits });
    expect(checkPropertyPrefix(`${fits}é`)).toEqual({ problem: 'property' });
  });

  it('write only the filters that are set, and read back the same', () => {
    expect(failureFiltersToSearch({})).toBe('');
    const filters = { run, property: 'a%b_c', status: 'not_reproducible' } as const;
    expect(failureFiltersFromSearch(failureFiltersToSearch(filters))).toEqual(filters);
    expect(failureFiltersToSearch({ status: 'verified' })).toBe('?status=verified');
  });

  it('say whether any filter is set', () => {
    expect(hasFailureFilters({})).toBe(false);
    expect(hasFailureFilters({ run })).toBe(true);
    expect(hasFailureFilters({ property: 'a' })).toBe(true);
    expect(hasFailureFilters({ status: 'candidate' })).toBe(true);
  });
});

describe('checking typed text', () => {
  it('treats empty text as no filter', () => {
    expect(checkTarget('')).toEqual({ value: undefined });
    expect(checkPropertyPrefix('')).toEqual({ value: undefined });
    expect(checkRunId('')).toEqual({ value: undefined });
  });

  it('names the problem for text that cannot be used', () => {
    expect(checkTarget('a\nb')).toEqual({ problem: 'target' });
    expect(checkTarget('x'.repeat(MAX_TARGET_BYTES + 1))).toEqual({ problem: 'target' });
    expect(checkRunId('run_12')).toEqual({ problem: 'run' });
    expect(checkRunId('run_0123456789abcdeg')).toEqual({ problem: 'run' });
    expect(checkPropertyPrefix('a\u0000')).toEqual({ problem: 'property' });
  });

  it('counts a target in bytes, as the server does', () => {
    const fits = '研'.repeat(Math.floor(MAX_TARGET_BYTES / 3));
    expect(checkTarget(fits)).toEqual({ value: fits });
    const tooLong = '研'.repeat(Math.floor(MAX_TARGET_BYTES / 3) + 1);
    expect(checkTarget(tooLong)).toEqual({ problem: 'target' });
    expect(runFiltersFromSearch(`?target=${encodeURIComponent(tooLong)}`)).toEqual({});
  });
});

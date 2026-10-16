import { describe, expect, it } from 'vitest';
import { filtersFromSearch, filtersToSearch, hasFilters, seedsPath, trialsUrl, withProperty } from './filters';

describe('filtersFromSearch', () => {
  it('reads every filter the server accepts', () => {
    const f = filtersFromSearch('?verdict=violation&fails=3&fp=4C16F00DCAFE0001&from=40');
    expect(f).toEqual({ verdict: 'violation', fails: 3, fp: '4c16f00dcafe0001', from: 40 });
  });

  it('reads hits and misses', () => {
    expect(filtersFromSearch('?hits=7')).toEqual({ hits: 7 });
    expect(filtersFromSearch('?misses=0')).toEqual({ misses: 0 });
  });

  it('keeps only one property filter: fails wins over hits wins over misses', () => {
    expect(filtersFromSearch('?misses=1&hits=2&fails=3')).toEqual({ fails: 3 });
    expect(filtersFromSearch('?misses=1&hits=2')).toEqual({ hits: 2 });
  });

  it('drops anything the server would refuse, so a pasted link cannot send it', () => {
    const bad = [
      '?verdict=<script>',
      '?verdict=excluded',
      '?fails=-1',
      '?fails=4294967296',
      '?fails=1.5',
      '?fails=%00',
      '?fp=4c16',
      '?fp=zzzzzzzzzzzzzzzz',
      `?fp=${'a'.repeat(65)}`,
      '?from=-3',
      '?from=99999999999999999999',
      '?from=x',
    ];
    for (const q of bad) expect(filtersFromSearch(q), q).toEqual({});
  });

  it('ignores unknown parameters and takes the first of a repeated one', () => {
    expect(filtersFromSearch('?x=1&verdict=pass&verdict=violation')).toEqual({ verdict: 'pass' });
  });
});

describe('filtersToSearch and seedsPath', () => {
  it('writes only what is set, in a fixed order, and round-trips', () => {
    const f = { verdict: 'violation', fails: 3, fp: '4c16f00dcafe0001', from: 40 } as const;
    const q = filtersToSearch(f);
    expect(q).toBe('?verdict=violation&fails=3&fp=4c16f00dcafe0001&from=40');
    expect(filtersFromSearch(q)).toEqual(f);
    expect(filtersToSearch({})).toBe('');
  });

  it('builds the explorer path for a run, and nothing for an invalid run id', () => {
    expect(seedsPath('run_0123', {})).toBe('/runs/run_0123/seeds');
    expect(seedsPath('run_0123', { fails: 12 })).toBe('/runs/run_0123/seeds?fails=12');
    expect(seedsPath('../x', {})).toBeUndefined();
  });
});

describe('withProperty', () => {
  it('replaces the property filter and keeps the others', () => {
    const f = { verdict: 'violation', fails: 3, from: 4 } as const;
    expect(withProperty(f, 'hits', 9)).toEqual({ verdict: 'violation', hits: 9, from: 4 });
    expect(withProperty(f, 'fails', undefined)).toEqual({ verdict: 'violation', from: 4 });
    expect(f).toEqual({ verdict: 'violation', fails: 3, from: 4 });
  });
});

describe('hasFilters', () => {
  it('is true only when something narrows the list', () => {
    expect(hasFilters({})).toBe(false);
    expect(hasFilters({ verdict: 'pass' })).toBe(true);
    expect(hasFilters({ from: 4 })).toBe(true);
  });
});

describe('trialsUrl', () => {
  it('encodes each filter, the cursor and the limit', () => {
    const url = trialsUrl('run_0123', { verdict: 'violation', fails: 3, fp: '4c16f00dcafe0001' }, { cursor: '199', limit: 200 });
    expect(url).toBe('/api/runs/run_0123/trials?verdict=violation&fails=3&fp=4c16f00dcafe0001&cursor=199&limit=200');
  });

  it('starts a list at a sequence number by asking for the trials after the one before it', () => {
    expect(trialsUrl('run_0123', { from: 40 }, { limit: 200 })).toBe('/api/runs/run_0123/trials?cursor=39&limit=200');
    expect(trialsUrl('run_0123', { from: 0 }, { limit: 200 })).toBe('/api/runs/run_0123/trials?limit=200');
    // A continuation cursor beats the starting point.
    expect(trialsUrl('run_0123', { from: 40 }, { cursor: '99', limit: 200 })).toBe('/api/runs/run_0123/trials?cursor=99&limit=200');
  });
});

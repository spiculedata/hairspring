import { describe, expect, it } from 'vitest';
import { isPropertyPage, parseRetryAfter, propertiesUrl } from './api';
import { page } from './explorer/fixtures';

describe('parseRetryAfter', () => {
  it('reads delta-seconds', () => {
    expect(parseRetryAfter('7', 0)).toBe(7000);
  });
  it('reads an HTTP date relative to now', () => {
    const now = Date.parse('2026-01-01T00:00:00Z');
    expect(parseRetryAfter('Thu, 01 Jan 2026 00:00:30 GMT', now)).toBe(30_000);
  });
  it('ignores a missing, negative or unparseable value', () => {
    expect(parseRetryAfter(null, 0)).toBeUndefined();
    expect(parseRetryAfter('-3', 0)).toBeUndefined();
    expect(parseRetryAfter('soon', 0)).toBeUndefined();
  });
  it('caps an absurd value', () => {
    expect(parseRetryAfter('999999', 0)).toBe(300_000);
  });
});

describe('isPropertyPage', () => {
  it('accepts a well-formed page', () => {
    expect(isPropertyPage(page())).toBe(true);
  });
  it('rejects valid JSON of the wrong shape', () => {
    for (const bad of [null, 5, 'x', [], {}, { ...page(), properties: 'no' }, { ...page(), header: null }, { ...page(), claim: undefined }, { ...page(), counts_by_status: [] }]) {
      expect(isPropertyPage(bad)).toBe(false);
    }
  });
  it('rejects a row without a guest object', () => {
    expect(isPropertyPage({ ...page(), properties: [{ idx: 1 }] })).toBe(false);
  });
});

describe('propertiesUrl', () => {
  it('has no query string when there are no filters', () => {
    expect(propertiesUrl('run_1', {})).toBe('/api/runs/run_1/properties');
  });

  it('encodes every filter the server accepts', () => {
    expect(propertiesUrl('run_1', { status: 'unmet', kind: 'sometimes', q: 'wal.', sort: 'rate', cursor: '100', limit: 50 })).toBe(
      '/api/runs/run_1/properties?status=unmet&kind=sometimes&q=wal.&sort=rate&cursor=100&limit=50',
    );
  });

  it('encodes the run id and the prefix, so neither can add parameters', () => {
    expect(propertiesUrl('a/b?c', { q: 'x&sort=id' })).toBe('/api/runs/a%2Fb%3Fc/properties?q=x%26sort%3Did');
  });

  it('leaves out an empty prefix', () => {
    expect(propertiesUrl('run_1', { q: '' })).toBe('/api/runs/run_1/properties');
  });
});

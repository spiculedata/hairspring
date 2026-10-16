import { describe, expect, it } from 'vitest';
import { propertiesUrl } from './api';

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

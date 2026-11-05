import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api';
import { isSearchResults, probeSearchAvailable, searchRequest, searchUrl } from './searchApi';

const EMPTY = {
  runs: [],
  failures: [],
  targets: [],
  properties: [],
  seeds: [],
  schedules: [],
  more: [],
  scope: { per_group: 5, recent_runs: 1000, property_runs: 100 },
};

const reply = (status: number, body: unknown) =>
  vi.fn().mockResolvedValue(new Response(typeof body === 'string' ? body : JSON.stringify(body), { status }));

afterEach(() => vi.unstubAllGlobals());

describe('searchUrl', () => {
  it('encodes the text, so it cannot add or override a parameter', () => {
    expect(searchUrl('run_8f3a')).toBe('/api/search?q=run_8f3a');
    expect(searchUrl('a&q=b#c')).toBe('/api/search?q=a%26q%3Db%23c');
  });
});

describe('isSearchResults', () => {
  it('accepts the documented shape', () => {
    expect(isSearchResults(EMPTY)).toBe(true);
    expect(
      isSearchResults({
        ...EMPTY,
        runs: [{ id: 'run_1', target: 't', state: 'failed' }],
        failures: [{ id: 'fail_1', run_id: 'run_1', target: 't' }],
        targets: [{ name: 'lock-lease' }],
        properties: [{ run_id: 'run_1', guest: { note: 'n', id: 'db.commit' } }],
        seeds: [{ run_id: 'run_1', seed: 7, trial: null }, { run_id: 'run_1', seed: 7, trial: 2 }],
        schedules: [{ fingerprint: '0123456789abcdef', run_id: 'run_1', trial: 0 }],
        more: ['runs'],
      }),
    ).toBe(true);
  });

  it('refuses a body that is not the shape the box reads, so the box shows an error and never crashes', () => {
    for (const bad of [null, [], 'x', {}, { ...EMPTY, runs: 'x' }, { ...EMPTY, runs: [{ id: 1 }] }, { ...EMPTY, more: [1] }, { ...EMPTY, scope: {} }, { ...EMPTY, seeds: [{ run_id: 'r', seed: '7', trial: null }] }, { ...EMPTY, properties: [{ run_id: 'r' }] }]) {
      expect(isSearchResults(bad), JSON.stringify(bad)).toBe(false);
    }
  });
});

describe('searchRequest', () => {
  it('asks the server and returns the answer', async () => {
    const fetchMock = reply(200, EMPTY);
    vi.stubGlobal('fetch', fetchMock);
    await expect(searchRequest('run_', undefined)).resolves.toEqual(EMPTY);
    expect(fetchMock.mock.calls[0][0]).toBe('/api/search?q=run_');
  });

  it('turns a body of the wrong shape into an error', async () => {
    vi.stubGlobal('fetch', reply(200, { runs: 3 }));
    await expect(searchRequest('run_', undefined)).rejects.toMatchObject({ status: 502 });
  });

  it('keeps the status of a refusal, and the retry time of a rate limit', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{"error":"rate limit exceeded"}', { status: 429, headers: { 'Retry-After': '30' } })));
    const e = await searchRequest('run_', undefined).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(ApiError);
    expect(e).toMatchObject({ status: 429, retryAfterMs: 30_000 });
  });
});

describe('probeSearchAvailable', () => {
  it('says the endpoint is there when the server answers anything but 404, using a text the server refuses before it queries', async () => {
    for (const status of [400, 200, 401, 403, 429]) {
      const fetchMock = reply(status, { error: 'x' });
      vi.stubGlobal('fetch', fetchMock);
      await expect(probeSearchAvailable(), String(status)).resolves.toBe(true);
      expect(fetchMock.mock.calls[0][0]).toBe('/api/search?q=%20');
    }
  });

  it('says it is absent on a 404, which is what an older server answers', async () => {
    vi.stubGlobal('fetch', reply(404, { error: 'not found' }));
    await expect(probeSearchAvailable()).resolves.toBe(false);
  });

  it('keeps the box when the network fails or the server errors: absence is only ever a 404', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('offline')));
    await expect(probeSearchAvailable()).resolves.toBe(true);
    vi.stubGlobal('fetch', reply(500, { error: 'internal error' }));
    await expect(probeSearchAvailable()).resolves.toBe(true);
  });
});

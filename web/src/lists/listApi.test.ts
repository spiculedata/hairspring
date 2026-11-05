import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api';
import type { FailureSummary, Run } from '../types';
import { failuresPage, failuresUrl, runsPage, runsUrl } from './listApi';

const run = (id: string): Run => ({
  id,
  target: 't',
  backend: 'sim',
  state: 'queued',
  seed: 1,
  created_at: 1,
  started_at: null,
  finished_at: null,
  steps: 0,
  trials: 0,
  error: null,
});
const failure = (id: string): FailureSummary => ({
  id,
  run_id: 'run_0123456789abcdef',
  target: 't',
  message: 'm',
  original_len: 2,
  shrunk_len: 1,
  bundle_hash: 'ab'.repeat(32),
  created_at: 1,
});

const stub = (body: unknown, status = 200) => {
  const fetchMock = vi.fn(async () => new Response(JSON.stringify(body), { status }));
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
};
const calledWith = (fetchMock: ReturnType<typeof stub>): string => (fetchMock.mock.calls[0] as unknown as [string])[0];

describe('the runs URL', () => {
  it('has only the limit when nothing else is set', () => {
    expect(runsUrl({ limit: 50 })).toBe('/api/runs?limit=50');
    expect(runsUrl({})).toBe('/api/runs');
  });

  it('carries each filter, the cursor and the limit, encoded so a value cannot add or override a parameter', () => {
    expect(runsUrl({ state: 'failed', target: 'a&limit=1', mode: 'sweep', cursor: '50', limit: 50 })).toBe(
      '/api/runs?state=failed&target=a%26limit%3D1&mode=sweep&cursor=50&limit=50',
    );
  });
});

describe('the failures URL', () => {
  it('carries each filter, the cursor and the limit, encoded', () => {
    expect(failuresUrl({ limit: 50 })).toBe('/api/failures?limit=50');
    expect(failuresUrl({ run: 'run_0123456789abcdef', property: 'db.%&x', status: 'confirmed', cursor: '100', limit: 25 })).toBe(
      '/api/failures?run=run_0123456789abcdef&property=db.%25%26x&status=confirmed&cursor=100&limit=25',
    );
  });
});

describe('runsPage', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('gives the runs and the cursor of the next page', async () => {
    const fetchMock = stub({ runs: [run('run_a')], next_cursor: '1' });
    await expect(runsPage({ state: 'queued', limit: 1 })).resolves.toEqual({ items: [run('run_a')], nextCursor: '1' });
    expect(calledWith(fetchMock)).toBe('/api/runs?state=queued&limit=1');
  });

  it('has no cursor on the last page', async () => {
    stub({ runs: [run('run_a')] });
    await expect(runsPage({})).resolves.toEqual({ items: [run('run_a')], nextCursor: undefined });
  });

  it('refuses a body of the wrong shape as an error, not a crash', async () => {
    for (const body of [{ runs: 'x' }, { runs: [{ id: 3 }] }, { runs: [], next_cursor: 5 }, [], { failures: [] }]) {
      stub(body);
      await expect(runsPage({})).rejects.toThrow(/unexpected shape/);
    }
  });

  it('passes the server error on, with its status', async () => {
    stub({ error: '`state` is not valid' }, 400);
    await expect(runsPage({})).rejects.toMatchObject({ status: 400, message: '`state` is not valid' });
    await expect(runsPage({})).rejects.toBeInstanceOf(ApiError);
  });
});

describe('failuresPage', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('gives the failures and the cursor of the next page', async () => {
    const fetchMock = stub({ failures: [failure('fail_a')], next_cursor: '7' });
    await expect(failuresPage({ status: 'verified', limit: 3 })).resolves.toEqual({ items: [failure('fail_a')], nextCursor: '7' });
    expect(calledWith(fetchMock)).toBe('/api/failures?status=verified&limit=3');
  });

  it('gives the scope of a property filter, and ignores one of the wrong shape', async () => {
    stub({ failures: [], scope: { recent_runs: 1000, cut_short: true } });
    await expect(failuresPage({ property: 'db.' })).resolves.toEqual({ items: [], nextCursor: undefined, scope: { recentRuns: 1000, cutShort: true } });
    for (const scope of [{ recent_runs: '1000', cut_short: true }, { recent_runs: 1000 }, 'x', null]) {
      stub({ failures: [], scope });
      const page = await failuresPage({ property: 'db.' });
      expect(page.scope).toBeUndefined();
    }
  });

  it('refuses a body of the wrong shape as an error', async () => {
    for (const body of [{ failures: {} }, { failures: [{ id: 1 }] }, { failures: [], next_cursor: [] }, null]) {
      stub(body);
      await expect(failuresPage({})).rejects.toThrow(/unexpected shape/);
    }
  });
});

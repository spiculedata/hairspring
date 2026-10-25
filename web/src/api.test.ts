import { describe, expect, it, vi } from 'vitest';
import { failureChecksUrl, getTokenVersion, isPropertyPage, onTokenChange, parseRetryAfter, propertiesUrl, setToken } from './api';
import { page } from './explorer/fixtures';
import type { FailureChecks, FindingStatus } from './types';

describe('token version', () => {
  it('advances each time a token is entered, so a hook that stopped on a 401 can load again', () => {
    const before = getTokenVersion();
    setToken('first');
    setToken('second');
    expect(getTokenVersion()).toBe(before + 2);
    setToken(null);
  });

  it('tells listeners when the token changes, until they unsubscribe', () => {
    const listener = vi.fn();
    const off = onTokenChange(listener);
    setToken('a');
    expect(listener).toHaveBeenCalledTimes(1);
    off();
    setToken('b');
    expect(listener).toHaveBeenCalledTimes(1);
    setToken(null);
  });
});

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

describe('failureChecksUrl', () => {
  it('names the failure and the checks', () => {
    expect(failureChecksUrl('fail_0123456789abcdef')).toBe('/api/failures/fail_0123456789abcdef/checks');
  });
  it('encodes an id that is not shaped like one, so it cannot change the path', () => {
    expect(failureChecksUrl('../runs?x=1')).toBe('/api/failures/..%2Fruns%3Fx%3D1/checks');
  });
});

describe('finding status', () => {
  it('calls the rung where only reruns repeated the failure rerun_repeated, never reproduced', () => {
    const rung: FindingStatus = 'rerun_repeated';
    // @ts-expect-error `reproduced` is not a state: that word is kept for confirmed and verified findings.
    const stale: FindingStatus = 'reproduced';
    expect([rung, stale]).toContain('rerun_repeated');
  });
  it('carries the confirmation the finding comes from', () => {
    const checks: FailureChecks = {
      failure_id: 'fail_0123456789abcdef',
      finding: 'confirmed',
      confirmation_id: 'conf_0123456789abcdef0123456789abcdef',
      environment_pinned: true,
      checks: [],
    };
    expect(checks.confirmation_id).toMatch(/^conf_/);
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

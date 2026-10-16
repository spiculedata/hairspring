import { describe, expect, it } from 'vitest';
import { outputUrl, trialBySeedUrl, trialUrl } from './api';
import { detail } from './fixtures';
import { isTrialDetail, isTrialPage } from './types';

describe('trial urls', () => {
  it('builds the detail, seed lookup and output urls from encoded pieces', () => {
    expect(trialUrl('run_0123', 16)).toBe('/api/runs/run_0123/trials/16');
    expect(trialBySeedUrl('run_0123', '18446744073709551615')).toBe('/api/runs/run_0123/trials?seed=18446744073709551615&limit=1');
    expect(outputUrl('run_0123', 16, 'stderr')).toBe('/api/runs/run_0123/trials/16/output/stderr');
  });

  it('encodes a run id so it cannot add a path segment or a parameter', () => {
    expect(trialUrl('a/../b?x=1', 1)).toBe('/api/runs/a%2F..%2Fb%3Fx%3D1/trials/1');
    expect(trialBySeedUrl('a&b', '1')).toBe('/api/runs/a%26b/trials?seed=1&limit=1');
  });
});

describe('response guards', () => {
  it('accepts a well-formed detail, with or without kept detail', () => {
    expect(isTrialDetail(detail())).toBe(true);
    expect(isTrialDetail(detail({ guest: { detail: null, schedule: null } }))).toBe(true);
  });

  it('rejects valid JSON of the wrong shape', () => {
    const d = detail();
    const bad = [null, 5, [], {}, { ...d, trial: null }, { ...d, guest: null }, { ...d, output_streams: 'x' }, { ...d, trial: { ...d.trial, seed: 5 } }, { ...d, guest: { detail: { props: {} }, schedule: null } }];
    for (const b of bad) expect(isTrialDetail(b)).toBe(false);
  });

  it('checks a trial page by its rows', () => {
    expect(isTrialPage({ run_id: 'r', trials: [detail().trial], next_cursor: null })).toBe(true);
    expect(isTrialPage({ trials: [{ seed: 5, seq: 1 }] })).toBe(false);
    expect(isTrialPage({})).toBe(false);
  });
});

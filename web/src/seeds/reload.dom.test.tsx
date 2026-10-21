// @vitest-environment happy-dom
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, setToken } from '../api';
import { seedsApi, type SeedTrialPage, type StripPage } from './api';
import { seedRow, stripPage } from './fixtures';
import { useSeedTrials } from './useSeedTrials';
import { useStrip } from './useStrip';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const trialPage = (seed: string): SeedTrialPage => ({ run_id: 'run_1', trials: [seedRow({ seed })], next_cursor: null, scanned_trials: 1 });

describe('useSeedTrials in a DOM', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    setToken(null);
  });

  it('reload drops the rows it holds at once and loads the trials again', async () => {
    const second = deferred<SeedTrialPage>();
    const spy = vi.spyOn(seedsApi, 'trials').mockResolvedValueOnce(trialPage('1111')).mockImplementationOnce(() => second.promise);
    const { result } = renderHook(() => useSeedTrials('run_1', {}, false));
    await waitFor(() => expect(result.current.rows).toHaveLength(1));
    expect(result.current.rows[0]?.seed).toBe('1111');

    act(() => result.current.reload());
    expect(result.current.rows).toHaveLength(0);
    expect(result.current.loaded).toBe(false);

    await act(async () => second.resolve(trialPage('2222')));
    await waitFor(() => expect(result.current.rows[0]?.seed).toBe('2222'));
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('stops on a 401, and loads again when a token is entered', async () => {
    const spy = vi.spyOn(seedsApi, 'trials').mockRejectedValueOnce(new ApiError(401, 'unauthorized')).mockResolvedValue(trialPage('3333'));
    const { result } = renderHook(() => useSeedTrials('run_1', {}, true));
    await waitFor(() => expect(result.current.error).toBe('unauthorized'));
    expect(spy).toHaveBeenCalledTimes(1);
    act(() => setToken('a token'));
    await waitFor(() => expect(result.current.rows[0]?.seed).toBe('3333'));
  });
});

describe('useStrip in a DOM', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    setToken(null);
  });

  it('reload drops the strip it holds at once and loads it again', async () => {
    const second = deferred<StripPage>();
    const spy = vi.spyOn(seedsApi, 'strip').mockResolvedValueOnce(stripPage({ codes: 'ppvpep' })).mockImplementationOnce(() => second.promise);
    const { result } = renderHook(() => useStrip('run_1', false));
    await waitFor(() => expect(result.current.strip.codes).toBe('ppvpep'));
    expect(result.current.loading).toBe(false);

    act(() => result.current.reload());
    expect(result.current.strip.codes).toBe('');
    expect(result.current.loading).toBe(true);

    await act(async () => second.resolve(stripPage({ codes: 'pp' })));
    await waitFor(() => expect(result.current.strip.codes).toBe('pp'));
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('stops on a 401, and loads again when a token is entered', async () => {
    const spy = vi.spyOn(seedsApi, 'strip').mockRejectedValueOnce(new ApiError(401, 'unauthorized')).mockResolvedValue(stripPage({ codes: 'pv' }));
    const { result } = renderHook(() => useStrip('run_1', true));
    await waitFor(() => expect(result.current.error).toBe('unauthorized'));
    expect(spy).toHaveBeenCalledTimes(1);
    act(() => setToken('a token'));
    await waitFor(() => expect(result.current.strip.codes).toBe('pv'));
  });
});

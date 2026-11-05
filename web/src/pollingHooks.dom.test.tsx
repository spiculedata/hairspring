// @vitest-environment happy-dom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { api, ApiError, setToken } from './api';
import { getConnection, OFFLINE_AFTER_FAILURES, resetConnection } from './connectionStatus';
import { page, row } from './explorer/fixtures';
import { usePropertyOverview } from './explorer/usePropertyOverview';
import { seedsApi } from './seeds/api';
import { seedRow, stripPage } from './seeds/fixtures';
import { useSeedTrials } from './seeds/useSeedTrials';
import { useStrip } from './seeds/useStrip';

const LIVE_MS = 3000;
const LONG_MS = LIVE_MS * 20;

const setVisibility = (state: 'visible' | 'hidden') => {
  vi.spyOn(document, 'visibilityState', 'get').mockReturnValue(state);
  document.dispatchEvent(new Event('visibilitychange'));
};
const advance = (ms: number) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)));

type Request = MockInstance<(...args: never[]) => Promise<unknown>>;

interface Case {
  name: string;
  /** Replaces the request this hook makes, which answers with `ok` until told otherwise. */
  install: () => Request;
  ok: () => unknown;
  mount: (live: boolean) => { unmount: () => void };
}

const CASES: Case[] = [
  {
    name: 'usePropertyOverview',
    install: () => vi.spyOn(api, 'properties') as unknown as Request,
    ok: () => page({ properties: [row()] }),
    mount: (live) => renderHook(() => usePropertyOverview('run_1', live)),
  },
  {
    name: 'useSeedTrials',
    install: () => vi.spyOn(seedsApi, 'trials') as unknown as Request,
    ok: () => ({ run_id: 'run_1', trials: [seedRow({ seed: '1' })], next_cursor: null, scanned_trials: 1 }),
    mount: (live) => renderHook(() => useSeedTrials('run_1', {}, live)),
  },
  {
    name: 'useStrip',
    install: () => vi.spyOn(seedsApi, 'strip') as unknown as Request,
    ok: () => stripPage({ codes: 'pv' }),
    mount: (live) => renderHook(() => useStrip('run_1', live)),
  },
];

describe.each(CASES)('$name polling', ({ install, ok, mount }) => {
  const request = (): Request => {
    const spy = install();
    spy.mockImplementation(async () => ok());
    return spy;
  };

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.useRealTimers();
    resetConnection();
    setToken(null);
  });

  it('makes no request while the tab is hidden, and refreshes at once when it is shown again', async () => {
    const spy = request();
    mount(true);
    await advance(0);
    expect(spy).toHaveBeenCalledTimes(1);

    setVisibility('hidden');
    await advance(LONG_MS);
    expect(spy).toHaveBeenCalledTimes(1);

    setVisibility('visible');
    await advance(0);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('keeps asking at its interval while the run is live', async () => {
    const spy = request();
    mount(true);
    await advance(LIVE_MS * 3);
    expect(spy.mock.calls.length).toBeGreaterThanOrEqual(3);
  });

  it('asks once and stays quiet for a run that has ended', async () => {
    const spy = request();
    mount(false);
    await advance(LONG_MS);
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('reports failed requests to the connection banner, and clears them when one succeeds', async () => {
    const spy = request();
    spy.mockImplementation(async () => {
      throw new TypeError('network down');
    });
    mount(true);
    await advance(LONG_MS);
    expect(getConnection().failures).toBeGreaterThanOrEqual(OFFLINE_AFTER_FAILURES);

    spy.mockImplementation(async () => ok());
    await advance(LONG_MS * 10);
    expect(getConnection().failures).toBe(0);
  });

  it('does not count a refusal from the server as the server being out of reach', async () => {
    const spy = request();
    spy.mockImplementation(async () => {
      throw new ApiError(404, 'gone');
    });
    mount(true);
    await advance(LONG_MS);
    expect(getConnection().failures).toBe(0);
  });

  it('removes its entry from the banner when it is unmounted', async () => {
    const spy = request();
    spy.mockImplementation(async () => {
      throw new TypeError('network down');
    });
    const view = mount(true);
    await advance(LONG_MS);
    expect(getConnection().failures).toBeGreaterThan(0);
    view.unmount();
    expect(getConnection().failures).toBe(0);
  });
});

describe('a live table once the reader has loaded further pages', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.useRealTimers();
    resetConnection();
  });

  it('keeps the property rows and makes no refresh request', async () => {
    const spy = vi.spyOn(api, 'properties');
    spy.mockImplementationOnce(async () => ({ ...page({ properties: [row({ idx: 0 })] }), next_cursor: 'c' }));
    spy.mockImplementation(async () => page({ properties: [row({ idx: 1 })] }));
    const { result } = renderHook(() => usePropertyOverview('run_1', true));
    await advance(0);
    act(() => result.current.loadMore?.());
    await advance(0);
    expect(result.current.rows).toHaveLength(2);
    await advance(LIVE_MS * 5);
    expect(spy).toHaveBeenCalledTimes(2);
    expect(result.current.rows).toHaveLength(2);
  });

  it('keeps the trial rows and makes no refresh request', async () => {
    const trials = (seed: string, next: string | null) => ({ run_id: 'run_1', trials: [seedRow({ seed })], next_cursor: next, scanned_trials: 1 });
    const spy = vi.spyOn(seedsApi, 'trials');
    spy.mockImplementationOnce(async () => trials('1', 'c'));
    spy.mockImplementation(async () => trials('2', null));
    const { result } = renderHook(() => useSeedTrials('run_1', {}, true));
    await advance(0);
    act(() => result.current.loadMore());
    await advance(0);
    expect(result.current.rows).toHaveLength(2);
    await advance(LIVE_MS * 5);
    expect(spy).toHaveBeenCalledTimes(2);
    expect(result.current.rows).toHaveLength(2);
  });
});

describe('useStrip first pass', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.useRealTimers();
    resetConnection();
  });

  it('pauses between pages while the tab is hidden and carries on from the same cursor when it is shown', async () => {
    const spy = vi.spyOn(seedsApi, 'strip');
    spy.mockImplementationOnce(async () => {
      setVisibility('hidden');
      return { ...stripPage({ codes: 'pv' }), next_cursor: '1' };
    });
    spy.mockImplementation(async () => ({ ...stripPage({ codes: 'pv' }), first_seq: 2, next_cursor: null }));
    const { result } = renderHook(() => useStrip('run_1', false));
    await advance(LONG_MS);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(result.current.loading).toBe(true);

    setVisibility('visible');
    await advance(0);
    expect(spy).toHaveBeenCalledTimes(2);
    expect(spy.mock.calls[1][1]).toMatchObject({ cursor: '1' });
    expect(result.current.loading).toBe(false);
  });
});

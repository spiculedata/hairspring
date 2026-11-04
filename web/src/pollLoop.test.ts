// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import { getConnection, registerPoller, resetConnection } from './connectionStatus';
import { startPolling } from './pollLoop';

const INTERVAL = 1000;

/** Lets the promise chain started by the loop settle without moving the fake clock. */
const settle = () => vi.advanceTimersByTimeAsync(0);

describe('startPolling', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('loads at once and then every interval, handing each result to the sink', async () => {
    const load = vi.fn().mockResolvedValueOnce(1).mockResolvedValue(2);
    const onData = vi.fn();
    const cancel = startPolling({ load, intervalMs: INTERVAL, onData, onError: vi.fn() });
    await settle();
    expect(onData).toHaveBeenLastCalledWith(1);
    await vi.advanceTimersByTimeAsync(INTERVAL);
    expect(onData).toHaveBeenLastCalledWith(2);
    expect(load).toHaveBeenCalledTimes(2);
    cancel();
  });

  it('stops once `stop` says the data is final', async () => {
    const load = vi.fn().mockResolvedValue('done');
    startPolling({ load, intervalMs: INTERVAL, stop: () => true, onData: vi.fn(), onError: vi.fn() });
    await vi.advanceTimersByTimeAsync(INTERVAL * 5);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('asks for the token once on a 401 and does not poll again until the page is remounted', async () => {
    const load = vi.fn().mockRejectedValue(new ApiError(401, 'unauthorized'));
    const onError = vi.fn();
    startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError });
    await vi.advanceTimersByTimeAsync(INTERVAL * 10);
    expect(load).toHaveBeenCalledTimes(1);
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it('stops for a run that no longer exists', async () => {
    const load = vi.fn().mockRejectedValue(new ApiError(404, 'gone'));
    startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
    await vi.advanceTimersByTimeAsync(INTERVAL * 5);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('keeps polling after an ordinary failure, at the usual interval', async () => {
    const load = vi.fn().mockRejectedValueOnce(new ApiError(500, 'boom')).mockResolvedValue('ok');
    const onData = vi.fn();
    const onError = vi.fn();
    startPolling({ load, intervalMs: INTERVAL, onData, onError });
    await settle();
    expect(onError).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(INTERVAL);
    expect(onData).toHaveBeenCalledWith('ok');
  });

  describe('when the tab is hidden', () => {
    const setVisibility = (state: 'visible' | 'hidden') => {
      vi.spyOn(document, 'visibilityState', 'get').mockReturnValue(state);
      document.dispatchEvent(new Event('visibilitychange'));
    };
    beforeEach(() => {
      resetConnection();
      vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible');
    });
    afterEach(() => vi.restoreAllMocks());

    it('makes no requests while hidden, and refreshes at once when it is shown again', async () => {
      const load = vi.fn().mockResolvedValue('x');
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      expect(load).toHaveBeenCalledTimes(1);
      setVisibility('hidden');
      await vi.advanceTimersByTimeAsync(INTERVAL * 10);
      expect(load).toHaveBeenCalledTimes(1);
      setVisibility('visible');
      await settle();
      expect(load).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(load).toHaveBeenCalledTimes(3);
      cancel();
    });

    it('does not start the next request when the one in flight finishes after the tab was hidden', async () => {
      let finish: (v: string) => void = () => undefined;
      const load = vi.fn(() => new Promise<string>((resolve) => (finish = resolve)));
      const onData = vi.fn();
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData, onError: vi.fn() });
      setVisibility('hidden');
      finish('late');
      await vi.advanceTimersByTimeAsync(INTERVAL * 5);
      expect(onData).toHaveBeenCalledWith('late');
      expect(load).toHaveBeenCalledTimes(1);
      cancel();
    });

    it('does not refresh on return when the data was final', async () => {
      const load = vi.fn().mockResolvedValue('done');
      startPolling({ load, intervalMs: INTERVAL, stop: () => true, onData: vi.fn(), onError: vi.fn() });
      await settle();
      setVisibility('hidden');
      setVisibility('visible');
      await settle();
      expect(load).toHaveBeenCalledTimes(1);
    });

    it('does not refresh on return after a 401 stopped it', async () => {
      const load = vi.fn().mockRejectedValue(new ApiError(401, 'no'));
      startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      setVisibility('hidden');
      setVisibility('visible');
      await settle();
      expect(load).toHaveBeenCalledTimes(1);
    });

    it('does not refresh on return once cancelled', async () => {
      const load = vi.fn().mockResolvedValue('x');
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      cancel();
      setVisibility('hidden');
      setVisibility('visible');
      await settle();
      expect(load).toHaveBeenCalledTimes(1);
    });

    it('does not start a second request when it is shown while one is still in flight', async () => {
      const load = vi.fn(() => new Promise<string>(() => undefined));
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      setVisibility('hidden');
      setVisibility('visible');
      await settle();
      expect(load).toHaveBeenCalledTimes(1);
      cancel();
    });
  });

  describe('after consecutive failures', () => {
    beforeEach(() => resetConnection());

    it('waits longer after each failure, up to a ceiling, and goes back to the interval after a success', async () => {
      const load = vi
        .fn()
        .mockRejectedValueOnce(new ApiError(500, 'a'))
        .mockRejectedValueOnce(new ApiError(500, 'b'))
        .mockRejectedValueOnce(new ApiError(500, 'c'))
        .mockResolvedValue('ok');
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      expect(load).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(INTERVAL - 1);
      expect(load).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(load).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(INTERVAL * 2 - 1);
      expect(load).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(1);
      expect(load).toHaveBeenCalledTimes(3);
      await vi.advanceTimersByTimeAsync(INTERVAL * 4);
      expect(load).toHaveBeenCalledTimes(4);
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(load).toHaveBeenCalledTimes(5);
      cancel();
    });

    it('reports connectivity failures and successes to the connection status', async () => {
      const load = vi.fn().mockRejectedValueOnce(new TypeError('Failed to fetch')).mockResolvedValue('ok');
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      expect(getConnection().failures).toBe(1);
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(getConnection().failures).toBe(0);
      cancel();
    });

    it('does not count a refusal as the connection being down, but does count it as having answered', async () => {
      const load = vi.fn().mockRejectedValue(new ApiError(429, 'slow'));
      const other = registerPoller();
      other.failure();
      other.unregister();
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      expect(getConnection().failures).toBe(0);
      cancel();
    });

    it('refreshes at once when the browser comes back online, and starts the waits again', async () => {
      const load = vi.fn().mockRejectedValue(new TypeError('x'));
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      window.dispatchEvent(new Event('online'));
      await settle();
      expect(load).toHaveBeenCalledTimes(2);
      // Without the reset this would be the second failure in a row and wait twice the interval.
      await vi.advanceTimersByTimeAsync(INTERVAL - 1);
      expect(load).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(1);
      expect(load).toHaveBeenCalledTimes(3);
      cancel();
    });

    it('ignores the browser coming back online while the tab is hidden', async () => {
      const load = vi.fn().mockRejectedValue(new TypeError('x'));
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
      document.dispatchEvent(new Event('visibilitychange'));
      window.dispatchEvent(new Event('online'));
      await vi.advanceTimersByTimeAsync(INTERVAL * 10);
      expect(load).toHaveBeenCalledTimes(1);
      cancel();
      vi.restoreAllMocks();
    });

    it('does not let one poll\'s success hide another poll\'s failures', async () => {
      const failing = startPolling({ load: vi.fn().mockRejectedValue(new TypeError('x')), intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      const working = startPolling({ load: vi.fn().mockResolvedValue('ok'), intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(getConnection().failures).toBe(2);
      failing();
      working();
    });

    it('clears its failures when it is cancelled', async () => {
      const load = vi.fn().mockRejectedValue(new TypeError('x'));
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(getConnection().failures).toBe(2);
      cancel();
      expect(getConnection().failures).toBe(0);
    });

    it('clears its failures when it ends on a 404 or a 401', async () => {
      for (const status of [404, 401]) {
        const load = vi.fn().mockRejectedValueOnce(new TypeError('x')).mockRejectedValueOnce(new TypeError('x')).mockRejectedValue(new ApiError(status, 'x'));
        startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
        await vi.advanceTimersByTimeAsync(INTERVAL * 4);
        expect(load).toHaveBeenCalledTimes(3);
        expect(getConnection().failures).toBe(0);
      }
    });

    it('clears its failures when it ends because the data is final', async () => {
      const load = vi.fn().mockRejectedValueOnce(new TypeError('x')).mockRejectedValueOnce(new TypeError('x')).mockResolvedValue('done');
      startPolling({ load, intervalMs: INTERVAL, stop: () => true, onData: vi.fn(), onError: vi.fn() });
      await vi.advanceTimersByTimeAsync(INTERVAL * 4);
      expect(getConnection().failures).toBe(0);
    });
  });

  describe('when the server refuses with 429', () => {
    beforeEach(() => resetConnection());

    it('waits at least as long as Retry-After', async () => {
      const load = vi.fn().mockRejectedValueOnce(new ApiError(429, 'slow', INTERVAL * 5)).mockResolvedValue('ok');
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      await vi.advanceTimersByTimeAsync(INTERVAL * 5 - 1);
      expect(load).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(load).toHaveBeenCalledTimes(2);
      cancel();
    });

    it('backs off on repeated 429s that carry no Retry-After', async () => {
      const load = vi.fn().mockRejectedValue(new ApiError(429, 'slow'));
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await settle();
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(load).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(INTERVAL * 2 - 1);
      expect(load).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(1);
      expect(load).toHaveBeenCalledTimes(3);
      cancel();
    });

    it('returns to the usual interval after a success', async () => {
      const load = vi.fn().mockRejectedValueOnce(new ApiError(429, 'x')).mockRejectedValueOnce(new ApiError(429, 'x')).mockResolvedValue('ok');
      const cancel = startPolling({ load, intervalMs: INTERVAL, onData: vi.fn(), onError: vi.fn() });
      await vi.advanceTimersByTimeAsync(INTERVAL * 3);
      expect(load).toHaveBeenCalledTimes(3);
      await vi.advanceTimersByTimeAsync(INTERVAL);
      expect(load).toHaveBeenCalledTimes(4);
      cancel();
    });
  });

  it('delivers nothing after it is cancelled', async () => {
    const load = vi.fn().mockResolvedValue('late');
    const onData = vi.fn();
    const cancel = startPolling({ load, intervalMs: INTERVAL, onData, onError: vi.fn() });
    cancel();
    await vi.advanceTimersByTimeAsync(INTERVAL * 3);
    expect(onData).not.toHaveBeenCalled();
    expect(load).toHaveBeenCalledTimes(1);
  });
});

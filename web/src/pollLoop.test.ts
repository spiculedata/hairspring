import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
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

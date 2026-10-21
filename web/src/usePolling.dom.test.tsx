// @vitest-environment happy-dom
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, setToken } from './api';
import { usePolling } from './usePolling';

const FAST_MS = 20;
const SETTLE_MS = 120;
const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe('usePolling in a DOM', () => {
  afterEach(() => {
    cleanup();
    setToken(null);
  });

  it('stops on a 401 and does not ask again by itself', async () => {
    const load = vi.fn().mockRejectedValue(new ApiError(401, 'unauthorized'));
    const { result } = renderHook(() => usePolling(load, FAST_MS));
    await waitFor(() => expect(result.current.error).toBe('unauthorized'));
    await pause(SETTLE_MS);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('starts again when a token is entered, and shows the data that comes back', async () => {
    const load = vi.fn().mockRejectedValueOnce(new ApiError(401, 'unauthorized')).mockResolvedValue('data');
    const { result } = renderHook(() => usePolling(load, FAST_MS, () => true));
    await waitFor(() => expect(result.current.error).toBe('unauthorized'));
    act(() => setToken('a token'));
    await waitFor(() => expect(result.current.data).toBe('data'));
    expect(result.current.error).toBeUndefined();
    expect(load).toHaveBeenCalledTimes(2);
  });

  it('keeps polling at its interval until the data is final', async () => {
    let n = 0;
    const load = vi.fn(async () => ++n);
    const { result } = renderHook(() => usePolling(load, FAST_MS, (data) => data >= 3));
    await waitFor(() => expect(result.current.data).toBe(3));
    await pause(SETTLE_MS);
    expect(load).toHaveBeenCalledTimes(3);
  });

  it('loads again when reload is called', async () => {
    const load = vi.fn().mockResolvedValueOnce('first').mockResolvedValue('second');
    const { result } = renderHook(() => usePolling(load, FAST_MS, () => true));
    await waitFor(() => expect(result.current.data).toBe('first'));
    act(() => result.current.reload());
    await waitFor(() => expect(result.current.data).toBe('second'));
  });
});

// @vitest-environment happy-dom
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { ApiError } from './api';
import { resetConnection } from './connectionStatus';
import { usePolling } from './usePolling';

const ONE_HOUR_MS = 3_600_000;

afterEach(() => {
  cleanup();
  resetConnection();
});

describe('usePolling networkError', () => {
  it('is true when the request got no answer at all', async () => {
    const { result } = renderHook(() =>
      usePolling(() => Promise.reject(new TypeError('Failed to fetch')), ONE_HOUR_MS),
    );
    await waitFor(() => expect(result.current.error).toBe('Failed to fetch'));
    expect(result.current.networkError).toBe(true);
  });

  it('is false when the server answered with an error status', async () => {
    const { result } = renderHook(() => usePolling(() => Promise.reject(new ApiError(500, 'HTTP 500')), ONE_HOUR_MS));
    await waitFor(() => expect(result.current.error).toBe('HTTP 500'));
    expect(result.current.networkError).toBe(false);
  });
});

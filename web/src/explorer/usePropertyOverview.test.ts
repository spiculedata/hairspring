import { describe, expect, it } from 'vitest';
import { ApiError } from '../api';
import { LIVE_REFRESH_MS, nextDelay } from './usePropertyOverview';

describe('nextDelay', () => {
  it('uses the usual interval after an ordinary error', () => {
    expect(nextDelay(new Error('x'))).toBe(LIVE_REFRESH_MS);
    expect(nextDelay(new ApiError(500, 'x'))).toBe(LIVE_REFRESH_MS);
  });
  it('never retries a 429 sooner than Retry-After', () => {
    expect(nextDelay(new ApiError(429, 'slow down', 30_000))).toBe(30_000);
  });
  it('does not go below the usual interval for a short Retry-After', () => {
    expect(nextDelay(new ApiError(429, 'slow down', 500))).toBe(LIVE_REFRESH_MS);
  });
});

import { describe, expect, it } from 'vitest';
import { ApiError } from './api';
import { retryDelay, STOP_ON_UNAUTHORIZED } from './pollPolicy';

const BASE = 2000;

describe('retryDelay', () => {
  it('retries an ordinary failure at the usual interval', () => {
    expect(retryDelay(new Error('network'), BASE)).toBe(BASE);
    expect(retryDelay(new ApiError(500, 'x'), BASE)).toBe(BASE);
  });

  it('stops for a run that no longer exists', () => {
    expect(retryDelay(new ApiError(404, 'not found'), BASE)).toBeUndefined();
  });

  it('never retries a 429 sooner than Retry-After', () => {
    expect(retryDelay(new ApiError(429, 'slow', 30_000), BASE)).toBe(30_000);
    expect(retryDelay(new ApiError(429, 'slow', 500), BASE)).toBe(BASE);
  });

  it('stops on 401 only when asked to', () => {
    expect(retryDelay(new ApiError(401, 'no'), BASE)).toBe(BASE);
    expect(retryDelay(new ApiError(401, 'no'), BASE, [401])).toBeUndefined();
  });

  it('offers one list of statuses that stop a poll because the token prompt has been raised', () => {
    expect(retryDelay(new ApiError(401, 'no'), BASE, STOP_ON_UNAUTHORIZED)).toBeUndefined();
    expect(retryDelay(new ApiError(403, 'no'), BASE, STOP_ON_UNAUTHORIZED)).toBe(BASE);
  });
});

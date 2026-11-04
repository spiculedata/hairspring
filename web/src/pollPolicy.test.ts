import { describe, expect, it } from 'vitest';
import { ApiError } from './api';
import { backoffDelay, isConnectivityFailure, MAX_BACKOFF_MS, retryDelay, STOP_ON_UNAUTHORIZED } from './pollPolicy';

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

describe('backoffDelay', () => {
  it('keeps the usual interval after the first failure and doubles it for each further one', () => {
    expect(backoffDelay(BASE, 1)).toBe(BASE);
    expect(backoffDelay(BASE, 2)).toBe(BASE * 2);
    expect(backoffDelay(BASE, 3)).toBe(BASE * 4);
  });

  it('never goes past the ceiling, and never below the usual interval', () => {
    expect(backoffDelay(BASE, 50)).toBe(MAX_BACKOFF_MS);
    expect(backoffDelay(MAX_BACKOFF_MS * 2, 3)).toBe(MAX_BACKOFF_MS * 2);
    expect(backoffDelay(BASE, 0)).toBe(BASE);
  });
});

describe('isConnectivityFailure', () => {
  it('counts a network error and a server error, not a refusal that shows the server answered', () => {
    expect(isConnectivityFailure(new TypeError('Failed to fetch'))).toBe(true);
    expect(isConnectivityFailure(new ApiError(502, 'bad gateway'))).toBe(true);
    for (const status of [400, 401, 403, 404, 429]) expect(isConnectivityFailure(new ApiError(status, 'x'))).toBe(false);
  });
});

import { ApiError } from './api';

const NOT_FOUND = 404;
const UNAUTHORIZED = 401;

/**
 * Statuses that stop a poll for good: on a 401 the token prompt has already been raised once, so asking again every
 * interval would only raise it again. Entering a token reloads the page's data (see `useTokenVersion`).
 */
export const STOP_ON_UNAUTHORIZED: readonly number[] = [UNAUTHORIZED];

/**
 * How long to wait before polling again after a failure, or `undefined` to stop.
 *
 * A 404 means the run is gone, so polling it again can only fail. A 429 waits at least as long as the server's
 * Retry-After. Any status in `stopOn` (the property table passes 401, because the token prompt has already been raised)
 * also stops; other failures retry at the usual interval.
 */
export function retryDelay(error: unknown, baseMs: number, stopOn: readonly number[] = []): number | undefined {
  if (!(error instanceof ApiError)) return baseMs;
  if (error.status === NOT_FOUND || stopOn.includes(error.status)) return undefined;
  return Math.max(baseMs, error.retryAfterMs ?? 0);
}

export const isNotFound = (error: unknown): boolean => error instanceof ApiError && error.status === NOT_FOUND;

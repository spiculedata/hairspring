import { ApiError } from './api';

const NOT_FOUND = 404;
const UNAUTHORIZED = 401;
const TOO_MANY_REQUESTS = 429;

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

export const isRateLimited = (error: unknown): boolean => error instanceof ApiError && error.status === TOO_MANY_REQUESTS;

export const isNotFound = (error: unknown): boolean => error instanceof ApiError && error.status === NOT_FOUND;

/** The longest a poll waits between attempts while requests keep failing. */
export const MAX_BACKOFF_MS = 30_000;

/**
 * How long to wait after the `failures`-th failure (or 429) in a row: the usual interval after the first, doubling for each
 * further one up to MAX_BACKOFF_MS. An interval already longer than that ceiling is kept as it is.
 */
export function backoffDelay(baseMs: number, failures: number): number {
  const doublings = Math.max(0, failures - 1);
  return Math.max(baseMs, Math.min(MAX_BACKOFF_MS, baseMs * 2 ** doublings));
}

/**
 * Whether a failure says the server could not be reached or could not answer (no response, or a 5xx), as opposed to
 * answering with a refusal such as a 401, 404 or 429.
 */
export const isConnectivityFailure = (error: unknown): boolean => !(error instanceof ApiError) || error.status >= 500;

/**
 * Whether a failure is the browser saying no response arrived at all (fetch rejects with a TypeError), as opposed to the
 * server answering with an error status. Only this kind may be put down to the connection; a 500 is the server's own answer.
 */
export const isNetworkFailure = (error: unknown): boolean => error instanceof TypeError;

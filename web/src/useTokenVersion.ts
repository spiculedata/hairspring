import { useSyncExternalStore } from 'react';
import { getTokenVersion, onTokenChange } from './api';

/**
 * A number that changes each time a token is entered.
 *
 * This is a fallback. In the app as it is, a 401 makes App replace the page with the token prompt and mount the page
 * again once a token is entered, which reloads everything. A loader that stops on a 401 still lists this among its
 * effect dependencies, so that it starts again with the new token even where the page is not remounted (a prompt shown
 * beside the page, or a loader mounted elsewhere), and does not wait for its filters or its run to change.
 */
export function useTokenVersion(): number {
  return useSyncExternalStore(onTokenChange, getTokenVersion, getTokenVersion);
}

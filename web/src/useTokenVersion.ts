import { useSyncExternalStore } from 'react';
import { getTokenVersion, onTokenChange } from './api';

/**
 * A number that changes each time a token is entered. A loader that stops on a 401 lists it among its effect
 * dependencies, so it starts again with the new token and does not wait for its filters or its run to change.
 */
export function useTokenVersion(): number {
  return useSyncExternalStore(onTokenChange, getTokenVersion, getTokenVersion);
}

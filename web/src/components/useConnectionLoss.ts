import { useSyncExternalStore } from 'react';
import { getConnection, OFFLINE_AFTER_FAILURES, subscribeConnection } from '../connectionStatus';

function subscribeOnline(listener: () => void): () => void {
  window.addEventListener('online', listener);
  window.addEventListener('offline', listener);
  return () => {
    window.removeEventListener('online', listener);
    window.removeEventListener('offline', listener);
  };
}

const getOnline = (): boolean => navigator.onLine;

export type ConnectionLoss = 'offline' | 'unreachable' | undefined;

/**
 * Why the page cannot get fresh data right now, if it cannot: the browser says it is offline, or requests have failed
 * several times in a row. The connection banner and the per-section load errors both read this, so they agree on when
 * the connection is the cause.
 */
export function useConnectionLoss(): ConnectionLoss {
  const online = useSyncExternalStore(subscribeOnline, getOnline, () => true);
  const { failures } = useSyncExternalStore(subscribeConnection, getConnection, getConnection);
  if (!online) return 'offline';
  return failures >= OFFLINE_AFTER_FAILURES ? 'unreachable' : undefined;
}

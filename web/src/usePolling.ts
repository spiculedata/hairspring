import { useCallback, useEffect, useRef, useState } from 'react';
import { startPolling } from './pollLoop';
import { isNotFound } from './pollPolicy';
import { useTokenVersion } from './useTokenVersion';

export interface Polled<T> {
  data: T | undefined;
  error: string | undefined;
  /** The last request was a 404: the thing polled no longer exists, and polling has stopped. */
  notFound: boolean;
  reload: () => void;
}

/**
 * Loads data now and then every `intervalMs` until `stop(data)` returns true. A 401 stops it (the token prompt has been
 * raised once); it starts again when a token is entered.
 */
export function usePolling<T>(load: () => Promise<T>, intervalMs: number, stop?: (data: T) => boolean): Polled<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<string>();
  const [notFound, setNotFound] = useState(false);
  const loadRef = useRef(load);
  const stopRef = useRef(stop);
  loadRef.current = load;
  stopRef.current = stop;
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  const tokenVersion = useTokenVersion();

  useEffect(
    () =>
      startPolling({
        load: () => loadRef.current(),
        intervalMs,
        stop: (next) => stopRef.current?.(next) ?? false,
        onData: (next) => {
          setData(next);
          setError(undefined);
          setNotFound(false);
        },
        onError: (e) => {
          setError(e instanceof Error ? e.message : String(e));
          setNotFound(isNotFound(e));
        },
      }),
    [intervalMs, tick, tokenVersion],
  );

  return { data, error, notFound, reload };
}

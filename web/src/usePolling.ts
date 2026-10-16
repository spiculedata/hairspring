import { useCallback, useEffect, useRef, useState } from 'react';
import { isNotFound, retryDelay } from './pollPolicy';

export interface Polled<T> {
  data: T | undefined;
  error: string | undefined;
  /** The last request was a 404: the thing polled no longer exists, and polling has stopped. */
  notFound: boolean;
  reload: () => void;
}

/** Loads data now and then every `intervalMs` until `stop(data)` returns true. */
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

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      let delay = intervalMs;
      try {
        const next = await loadRef.current();
        if (cancelled) return;
        setData(next);
        setError(undefined);
        setNotFound(false);
        if (stopRef.current?.(next)) return;
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        setNotFound(isNotFound(e));
        const wait = retryDelay(e, intervalMs);
        if (wait === undefined) return;
        delay = wait;
      }
      timer = setTimeout(run, delay);
    };
    void run();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [intervalMs, tick]);

  return { data, error, notFound, reload };
}

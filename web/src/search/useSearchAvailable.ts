import { useCallback, useEffect, useState } from 'react';
import { probeSearchAvailable } from './searchApi';

export type SearchAvailability = 'checking' | 'available' | 'absent';

let probe: Promise<boolean> | undefined;

/** Forgets what the server said about search, so the next box asks again. For tests. */
export function resetSearchProbe(): void {
  probe = undefined;
}

/**
 * Whether to show the search box: it is shown once the server has answered that it has a search endpoint, and never
 * shown for a server that answers 404 (an older release). The question is asked once per page load, however often the
 * layout is drawn again. `markAbsent` is for a search that later gets a 404 after all.
 */
export function useSearchAvailable(): readonly [SearchAvailability, () => void] {
  const [state, setState] = useState<SearchAvailability>('checking');
  useEffect(() => {
    let live = true;
    probe ??= probeSearchAvailable();
    void probe.then((ok) => {
      if (live) setState(ok ? 'available' : 'absent');
    });
    return () => {
      live = false;
    };
  }, []);
  const markAbsent = useCallback(() => {
    probe = Promise.resolve(false);
    setState('absent');
  }, []);
  return [state, markAbsent];
}

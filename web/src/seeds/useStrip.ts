import { useCallback, useEffect, useState } from 'react';
import { startPolling } from '../pollLoop';
import { isNotFound, retryDelay, STOP_ON_UNAUTHORIZED } from '../pollPolicy';
import { useTokenVersion } from '../useTokenVersion';
import { seedsApi } from './api';
import { appendStripPage, EMPTY_STRIP, type StripState } from './strip';

/** Codes asked for per request; the server caps this at the most trials it reads in one request. */
export const STRIP_PAGE = 20_000;
export const STRIP_LIVE_MS = 3000;

export interface StripModel {
  strip: StripState;
  /** The first pass over the run has not finished. */
  loading: boolean;
  error: string | undefined;
  /** Loads the strip again from the start (the error boundary's Retry). */
  reload: () => void;
}

interface PassResult {
  /** The pass reached the end of the run as it stood; false when it was paused or superseded. */
  complete: boolean;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Loads the strip a page at a time until the run's end. While the run is live it then asks again, from the last trial
 * it holds, so new trials are added without reloading what is already there. It polls through the shared loop
 * (pollLoop.ts): no request while the tab is hidden, a refresh when it is shown again, failures reported to the
 * connection banner. A failed request keeps what was loaded and retries; a 404 (the run is gone) and a 401 (the token
 * prompt is up) stop it.
 */
export function useStrip(runId: string, live: boolean): StripModel {
  const [strip, setStrip] = useState<StripState>(EMPTY_STRIP);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const tokenVersion = useTokenVersion();
  const [reloads, setReloads] = useState(0);
  // What is held is dropped, so a retried part does not render the data that made it fail.
  const reload = useCallback(() => {
    setStrip(EMPTY_STRIP);
    setLoading(true);
    setReloads((n) => n + 1);
  }, []);

  useEffect(() => {
    const ctl = new AbortController();
    let state = EMPTY_STRIP;
    let cursor: string | undefined;
    const isHidden = () => typeof document !== 'undefined' && document.visibilityState === 'hidden';
    // One pass: pages until the end of the run as it stands. A tab that is hidden part way leaves the pass unfinished,
    // and the loop carries on from the same cursor when the tab is shown again.
    const pass = async (): Promise<PassResult> => {
      for (;;) {
        const page = await seedsApi.strip(runId, { cursor, limit: STRIP_PAGE }, ctl.signal);
        if (ctl.signal.aborted) return { complete: false };
        state = appendStripPage(state, page);
        setStrip(state);
        setError(undefined);
        if (page.next_cursor === null || state.truncated) {
          cursor = state.nextSeq > 0 ? String(state.nextSeq - 1) : undefined;
          return { complete: true };
        }
        cursor = page.next_cursor;
        if (isHidden()) return { complete: false };
      }
    };
    const cancel = startPolling<PassResult>({
      load: pass,
      intervalMs: STRIP_LIVE_MS,
      stop: (result) => result.complete && !live,
      onData: (result) => {
        if (result.complete) setLoading(false);
      },
      onError: (e) => {
        setError(isNotFound(e) ? 'this run does not exist' : messageOf(e));
        if (retryDelay(e, STRIP_LIVE_MS, STOP_ON_UNAUTHORIZED) === undefined) setLoading(false);
      },
    });
    return () => {
      cancel();
      ctl.abort();
    };
  }, [runId, live, tokenVersion, reloads]);

  return { strip, loading, error, reload };
}

import { useEffect, useState } from 'react';
import { isNotFound, retryDelay } from '../pollPolicy';
import { seedsApi } from './api';
import { appendStripPage, EMPTY_STRIP, type StripState } from './strip';

/** Codes asked for per request; the server caps this at the most trials it reads in one request. */
export const STRIP_PAGE = 20_000;
export const STRIP_LIVE_MS = 3000;
const UNAUTHORIZED = 401;

export interface StripModel {
  strip: StripState;
  /** The first pass over the run has not finished. */
  loading: boolean;
  error: string | undefined;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Loads the strip a page at a time until the run's end. While the run is live it then asks again, from the last trial
 * it holds, so new trials are added without reloading what is already there. A failed request keeps what was loaded
 * and retries; a 404 (the run is gone) and a 401 (the token prompt is up) stop it.
 */
export function useStrip(runId: string, live: boolean): StripModel {
  const [strip, setStrip] = useState<StripState>(EMPTY_STRIP);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();

  useEffect(() => {
    const ctl = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let state = EMPTY_STRIP;
    let cursor: string | undefined;
    const step = async () => {
      let delay: number | undefined = live ? STRIP_LIVE_MS : undefined;
      try {
        const page = await seedsApi.strip(runId, { cursor, limit: STRIP_PAGE }, ctl.signal);
        if (ctl.signal.aborted) return;
        state = appendStripPage(state, page);
        setStrip(state);
        setError(undefined);
        if (page.next_cursor !== null && !state.truncated) {
          cursor = page.next_cursor;
          delay = 0;
        } else {
          cursor = state.nextSeq > 0 ? String(state.nextSeq - 1) : undefined;
          setLoading(false);
        }
      } catch (e) {
        if (ctl.signal.aborted) return;
        setError(isNotFound(e) ? 'this run does not exist' : messageOf(e));
        const wait = retryDelay(e, STRIP_LIVE_MS, [UNAUTHORIZED]);
        if (wait === undefined) {
          setLoading(false);
          return;
        }
        delay = wait;
      }
      if (delay !== undefined && !ctl.signal.aborted) timer = setTimeout(step, delay);
    };
    void step();
    return () => {
      ctl.abort();
      if (timer) clearTimeout(timer);
    };
  }, [runId, live]);

  return { strip, loading, error };
}

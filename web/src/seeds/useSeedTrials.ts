import { useCallback, useEffect, useRef, useState } from 'react';
import { isNotFound, retryDelay } from '../pollPolicy';
import type { TrialRow } from '../trial/types';
import { seedsApi } from './api';
import type { SeedFilters } from './filters';

/** Rows asked for per request; the server caps this, and the table windows what is loaded. */
export const TRIALS_PAGE = 200;
export const TRIALS_LIVE_MS = 3000;
const UNAUTHORIZED = 401;

export interface SeedTrialsModel {
  rows: readonly TrialRow[];
  /** The last trial the server read for the newest page; where "read further" continues. `null` when the run is exhausted. */
  cursor: string | null;
  loaded: boolean;
  loadingMore: boolean;
  error: string | undefined;
  loadMore: () => void;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Loads a run's trials for a set of filters, a server page at a time.
 *
 * - While the run is live the first page is refreshed, unless the reader has loaded further pages (their rows are never
 *   dropped silently).
 * - A response for an earlier filter, or one that arrives after a refresh replaced the rows it would extend, is dropped,
 *   and a superseded request is aborted.
 * - 404 stops refreshing for good, 401 stops it (the token prompt is up), 429 waits for Retry-After (pollPolicy.ts).
 */
export function useSeedTrials(runId: string, filters: SeedFilters, live: boolean): SeedTrialsModel {
  const [first, setFirst] = useState<readonly TrialRow[]>([]);
  const [more, setMore] = useState<readonly TrialRow[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string>();
  const epoch = useRef(0);
  const moreCount = useRef(0);
  const moreRequest = useRef<AbortController | undefined>(undefined);
  const filterKey = JSON.stringify(filters);

  useEffect(() => {
    const ctl = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let firstLoad = true;
    setLoaded(false);
    const load = async () => {
      let delay = TRIALS_LIVE_MS;
      if (firstLoad || moreCount.current === 0) {
        try {
          const page = await seedsApi.trials(runId, filters, { limit: TRIALS_PAGE }, ctl.signal);
          if (ctl.signal.aborted) return;
          epoch.current++;
          moreCount.current = 0;
          setFirst(page.trials);
          setMore([]);
          setCursor(page.next_cursor);
          setLoaded(true);
          setError(undefined);
        } catch (e) {
          if (ctl.signal.aborted) return;
          setError(isNotFound(e) ? 'this run does not exist' : messageOf(e));
          const wait = retryDelay(e, TRIALS_LIVE_MS, [UNAUTHORIZED]);
          if (wait === undefined) return;
          delay = wait;
        }
      }
      firstLoad = false;
      if (live && !ctl.signal.aborted) timer = setTimeout(load, delay);
    };
    void load();
    return () => {
      ctl.abort();
      moreRequest.current?.abort();
      if (timer) clearTimeout(timer);
    };
    // `filterKey` stands in for `filters`, which is a new object on every change.
  }, [runId, filterKey, live]);

  const loadMore = useCallback(() => {
    if (cursor === null) return;
    moreRequest.current?.abort();
    const ctl = new AbortController();
    moreRequest.current = ctl;
    const startedAt = epoch.current;
    setLoadingMore(true);
    seedsApi
      .trials(runId, filters, { cursor, limit: TRIALS_PAGE }, ctl.signal)
      .then((page) => {
        if (ctl.signal.aborted || epoch.current !== startedAt) return;
        moreCount.current += page.trials.length;
        setMore((prev) => [...prev, ...page.trials]);
        setCursor(page.next_cursor);
      })
      .catch((e: unknown) => {
        if (!ctl.signal.aborted && epoch.current === startedAt) setError(messageOf(e));
      })
      .finally(() => {
        if (moreRequest.current === ctl) setLoadingMore(false);
      });
    // `filterKey` stands in for `filters`.
  }, [runId, cursor, filterKey]);

  return { rows: [...first, ...more], cursor, loaded, loadingMore, error, loadMore };
}

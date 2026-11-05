import { useCallback, useEffect, useRef, useState } from 'react';
import { startPolling } from '../pollLoop';
import { isNotFound } from '../pollPolicy';
import type { TrialRow } from '../trial/types';
import { useTokenVersion } from '../useTokenVersion';
import { seedsApi, type SeedTrialPage } from './api';
import type { SeedFilters } from './filters';

/** Rows asked for per request; the server caps this, and the table windows what is loaded. */
export const TRIALS_PAGE = 200;
export const TRIALS_LIVE_MS = 3000;

export interface SeedTrialsModel {
  rows: readonly TrialRow[];
  /** The last trial the server read for the newest page; where "read further" continues. `null` when the run is exhausted. */
  cursor: string | null;
  loaded: boolean;
  loadingMore: boolean;
  error: string | undefined;
  loadMore: () => void;
  /** Loads the trials again from the start (the error boundary's Retry). */
  reload: () => void;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Loads a run's trials for a set of filters, a server page at a time.
 *
 * - While the run is live the first page is refreshed, unless the reader has loaded further pages (their rows are never
 *   dropped silently).
 * - A response for an earlier filter, or one that arrives after a refresh replaced the rows it would extend, is dropped,
 *   and a superseded request is aborted.
 * - It polls through the shared loop (pollLoop.ts): no request while the tab is hidden, a refresh when it is shown again,
 *   failures reported to the connection banner. 404 stops refreshing for good, 401 stops it (the token prompt is up;
 *   entering a token loads again), 429 waits for Retry-After (pollPolicy.ts).
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
  const tokenVersion = useTokenVersion();
  const [reloads, setReloads] = useState(0);
  // What is held is dropped, so a retried part does not render the data that made it fail.
  const reload = useCallback(() => {
    setFirst([]);
    setMore([]);
    setCursor(null);
    setLoaded(false);
    setReloads((n) => n + 1);
  }, []);

  useEffect(() => {
    const ctl = new AbortController();
    let firstLoad = true;
    setLoaded(false);
    const cancel = startPolling<SeedTrialPage | undefined>({
      // Once the reader has loaded further pages a refresh would replace their rows, so the request is skipped.
      load: async () => (firstLoad || moreCount.current === 0 ? seedsApi.trials(runId, filters, { limit: TRIALS_PAGE }, ctl.signal) : undefined),
      intervalMs: TRIALS_LIVE_MS,
      stop: () => !live,
      onData: (page) => {
        firstLoad = false;
        if (!page) return;
        epoch.current++;
        moreCount.current = 0;
        setFirst(page.trials);
        setMore([]);
        setCursor(page.next_cursor);
        setLoaded(true);
        setError(undefined);
      },
      onError: (e) => setError(isNotFound(e) ? 'this run does not exist' : messageOf(e)),
    });
    return () => {
      cancel();
      ctl.abort();
      moreRequest.current?.abort();
    };
    // `filterKey` stands in for `filters`, which is a new object on every change.
  }, [runId, filterKey, live, tokenVersion, reloads]);

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

  return { rows: [...first, ...more], cursor, loaded, loadingMore, error, loadMore, reload };
}

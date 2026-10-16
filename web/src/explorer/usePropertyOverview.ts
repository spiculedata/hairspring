import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../api';
import { isNotFound, retryDelay } from '../pollPolicy';
import type { PropertyPage, PropertyQuery, PropertyRow } from '../types';

/** Rows asked for per request; the server caps this, and the table windows what is loaded. */
export const PAGE_SIZE = 200;
export const LIVE_REFRESH_MS = 2000;
const UNAUTHORIZED = 401;

export interface PropertyOverviewModel {
  /** First page for the current filters: carries the header, claim and counts. */
  page: PropertyPage | undefined;
  /** Every row loaded so far. */
  rows: readonly PropertyRow[];
  query: PropertyQuery;
  setQuery: (next: PropertyQuery) => void;
  loadMore: (() => void) | undefined;
  loadingMore: boolean;
  error: string | undefined;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Loads a run's property table.
 *
 * - While the run is live the first page is refreshed, because figures are written when it ends. A refresh replaces the
 *   rows, so it is skipped once the reader has loaded further pages (their rows are never dropped silently); when the
 *   run ends the table is reloaded from the start with the final figures.
 * - A response that belongs to an earlier filter, or that arrives after a refresh replaced the rows it would extend,
 *   is dropped, and a superseded request is aborted.
 * - A 429 delays the next refresh by Retry-After, a 401 stops refreshing (the token prompt has been raised once) and a
 *   404 stops it for good (the run is gone); see pollPolicy.ts.
 */
export function usePropertyOverview(runId: string, live: boolean): PropertyOverviewModel {
  const [query, setQuery] = useState<PropertyQuery>({});
  const [page, setPage] = useState<PropertyPage>();
  const [more, setMore] = useState<readonly PropertyRow[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string>();
  const epoch = useRef(0);
  const moreCount = useRef(0);
  const moreRequest = useRef<AbortController | undefined>(undefined);
  const queryKey = JSON.stringify(query);

  useEffect(() => {
    const ctl = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let first = true;
    const load = async () => {
      let delay = LIVE_REFRESH_MS;
      if (first || moreCount.current === 0) {
        try {
          const next = await api.properties(runId, { ...query, limit: PAGE_SIZE }, ctl.signal);
          if (ctl.signal.aborted) return;
          epoch.current++;
          moreCount.current = 0;
          setPage(next);
          setMore([]);
          setCursor(next.next_cursor);
          setError(undefined);
        } catch (e) {
          if (ctl.signal.aborted) return;
          setError(isNotFound(e) ? 'this run does not exist' : messageOf(e));
          const wait = retryDelay(e, LIVE_REFRESH_MS, [UNAUTHORIZED]);
          if (wait === undefined) return;
          delay = wait;
        }
      }
      first = false;
      if (live && !ctl.signal.aborted) timer = setTimeout(load, delay);
    };
    void load();
    return () => {
      ctl.abort();
      moreRequest.current?.abort();
      if (timer) clearTimeout(timer);
    };
    // `queryKey` stands in for `query`, which is a new object on every change.
  }, [runId, queryKey, live]);

  const loadMore = useCallback(() => {
    if (cursor === null) return;
    moreRequest.current?.abort();
    const ctl = new AbortController();
    moreRequest.current = ctl;
    const startedAt = epoch.current;
    setLoadingMore(true);
    api
      .properties(runId, { ...query, cursor, limit: PAGE_SIZE }, ctl.signal)
      .then((next) => {
        if (ctl.signal.aborted || epoch.current !== startedAt) return;
        moreCount.current += next.properties.length;
        setMore((prev) => [...prev, ...next.properties]);
        setCursor(next.next_cursor);
      })
      .catch((e: unknown) => {
        if (!ctl.signal.aborted && epoch.current === startedAt) setError(messageOf(e));
      })
      .finally(() => {
        if (moreRequest.current === ctl) setLoadingMore(false);
      });
    // `queryKey` stands in for `query`.
  }, [runId, cursor, queryKey]);

  const rows = page ? [...page.properties, ...more] : [];
  return { page, rows, query, setQuery, loadMore: cursor === null ? undefined : loadMore, loadingMore, error };
}

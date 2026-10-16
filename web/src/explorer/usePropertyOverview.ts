import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../api';
import type { PropertyPage, PropertyQuery, PropertyRow } from '../types';

/** Rows asked for per request; the server caps this, and the table windows what is loaded. */
export const PAGE_SIZE = 200;
const LIVE_REFRESH_MS = 2000;

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
 * Loads a run's property table. The first page is refreshed while the run is live (figures are written when it ends);
 * "load more" appends the next page for the same filters. Changing a filter starts over, and a response that belongs
 * to an earlier filter is dropped.
 */
export function usePropertyOverview(runId: string, live: boolean): PropertyOverviewModel {
  const [query, setQuery] = useState<PropertyQuery>({});
  const [page, setPage] = useState<PropertyPage>();
  const [more, setMore] = useState<readonly PropertyRow[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string>();
  const generation = useRef(0);
  const queryKey = JSON.stringify(query);

  useEffect(() => {
    const mine = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      try {
        const next = await api.properties(runId, { ...query, limit: PAGE_SIZE });
        if (generation.current !== mine) return;
        setPage(next);
        setMore([]);
        setCursor(next.next_cursor);
        setError(undefined);
      } catch (e) {
        if (generation.current !== mine) return;
        setError(messageOf(e));
      }
      if (live && generation.current === mine) timer = setTimeout(load, LIVE_REFRESH_MS);
    };
    void load();
    return () => {
      generation.current++;
      if (timer) clearTimeout(timer);
    };
    // `queryKey` stands in for `query`, which is a new object on every change.
  }, [runId, queryKey, live]);

  const loadMore = useCallback(() => {
    if (cursor === null) return;
    const mine = generation.current;
    setLoadingMore(true);
    api
      .properties(runId, { ...query, cursor, limit: PAGE_SIZE })
      .then((next) => {
        if (generation.current !== mine) return;
        setMore((prev) => [...prev, ...next.properties]);
        setCursor(next.next_cursor);
      })
      .catch((e: unknown) => {
        if (generation.current === mine) setError(messageOf(e));
      })
      .finally(() => setLoadingMore(false));
    // `queryKey` stands in for `query`.
  }, [runId, cursor, queryKey]);

  const rows = page ? [...page.properties, ...more] : [];
  return { page, rows, query, setQuery, loadMore: cursor === null ? undefined : loadMore, loadingMore, error };
}

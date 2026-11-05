import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ApiError } from '../api';
import { startPolling } from '../pollLoop';
import { useTokenVersion } from '../useTokenVersion';
import { applyHead, applyMore, rowsOf, type Held, type ListPage } from './listPaging';

export interface ListPagesOptions<T> {
  /** Loads a page: the first when `cursor` is undefined, otherwise the page that starts there. */
  loadPage: (cursor: string | undefined, signal: AbortSignal) => Promise<ListPage<T>>;
  /** Tells rows apart; two rows with one id are one row. */
  id: (row: T) => string;
  /** How often the first page is loaded again. */
  intervalMs: number;
  /** Rows can leave the list between loads (a filter on a state that changes); see `HeadOptions` in listPaging.ts. */
  rowsLeave?: boolean;
}

export interface ListPagesModel<T> {
  /** Every row held, newest first; undefined until the first page has arrived. */
  rows: readonly T[] | undefined;
  /** An older page exists and `loadMore` will fetch it. */
  hasMore: boolean;
  loadMore: () => void;
  loadingMore: boolean;
  /** Why the last attempt at an older page failed. The rows already held are kept; asking again tries again. */
  moreError: string | undefined;
  /** The server said the list cannot be read any further back (the cursor is too deep). That is an end, not a failure. */
  endReached: boolean;
  /** Pages added by asking for more so far; zero until the reader asks. */
  moreLoads: number;
  /** Why the last load of the first page failed. The rows already held are kept, and the page tries again by itself. */
  error: string | undefined;
  /** When the first page last arrived (ms since the epoch); kept through later errors. */
  updatedAt: number | undefined;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** The server's refusal of a cursor beyond the deepest position it reads (a 400 that names `cursor`). */
const isCursorTooDeep = (e: unknown): boolean => e instanceof ApiError && e.status === 400 && e.message.includes('`cursor`');

/**
 * A list that is refreshed from its first page and extended by asking for older pages.
 *
 * - The first page is loaded again every `intervalMs` by the polling rules of pollLoop.ts (paused while the tab is
 *   hidden, backed off on failures, stopped by a 401 until a token is entered). The rows held stay on screen through a
 *   failed load, with the reason in `error`.
 * - `loadMore` fetches the page after the last one held. One request is made at a time. A failure keeps the rows and
 *   is reported in `moreError`. A page that arrives after the list started again is ignored, as is one that arrives after
 *   the page was left.
 * - A change of filters is a new list: mount this hook again for it (give its component a new `key`).
 * The rules for keeping rows once and leaving none out are in listPaging.ts.
 */
export function useListPages<T>({ loadPage, id, intervalMs, rowsLeave = false }: ListPagesOptions<T>): ListPagesModel<T> {
  const [held, setHeld] = useState<Held<T>>();
  const [error, setError] = useState<string>();
  const [updatedAt, setUpdatedAt] = useState<number>();
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState<string>();
  const [moreLoads, setMoreLoads] = useState(0);
  /** The epoch in which the server said the list cannot be read further; a restart of the list clears it. */
  const [endEpoch, setEndEpoch] = useState<number>();
  const endReached = held !== undefined && endEpoch === held.epoch;
  const endReachedRef = useRef(endReached);
  endReachedRef.current = endReached;
  const heldRef = useRef(held);
  heldRef.current = held;
  const loadRef = useRef(loadPage);
  loadRef.current = loadPage;
  const idRef = useRef(id);
  idRef.current = id;
  const rowsLeaveRef = useRef(rowsLeave);
  rowsLeaveRef.current = rowsLeave;
  const request = useRef<AbortController>(undefined);
  const tokenVersion = useTokenVersion();

  useEffect(() => {
    const controller = new AbortController();
    const stop = startPolling({
      load: () => loadRef.current(undefined, controller.signal),
      intervalMs,
      onData: (page) => {
        setHeld((prev) => applyHead(prev, page, idRef.current, { rowsLeave: rowsLeaveRef.current }));
        setUpdatedAt(Date.now());
        setError(undefined);
      },
      onError: (e) => setError(messageOf(e)),
    });
    return () => {
      controller.abort();
      stop();
    };
  }, [intervalMs, tokenVersion]);

  // Leaving the page cancels a request for an older page.
  useEffect(() => () => request.current?.abort(), []);

  const loadMore = useCallback(() => {
    const current = heldRef.current;
    if (current === undefined || current.cursor === undefined || endReachedRef.current || request.current !== undefined) return;
    const controller = new AbortController();
    request.current = controller;
    setLoadingMore(true);
    setMoreError(undefined);
    loadRef
      .current(current.cursor, controller.signal)
      .then((page) => {
        if (controller.signal.aborted) return;
        setHeld((prev) => applyMore(prev, page, current.epoch, idRef.current));
        setMoreLoads((n) => n + 1);
      })
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        if (isCursorTooDeep(e)) setEndEpoch(current.epoch);
        else setMoreError(messageOf(e));
      })
      .finally(() => {
        if (request.current !== controller) return;
        request.current = undefined;
        if (!controller.signal.aborted) setLoadingMore(false);
      });
  }, []);

  const rows = useMemo(() => (held === undefined ? undefined : rowsOf(held, idRef.current)), [held]);
  const hasMore = held?.cursor !== undefined && !endReached;
  return { rows, hasMore, endReached, loadMore, loadingMore, moreError, moreLoads, error, updatedAt };
}

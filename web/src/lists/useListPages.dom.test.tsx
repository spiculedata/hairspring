// @vitest-environment happy-dom
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, setToken } from '../api';
import type { ListPage } from './listPaging';
import { useListPages } from './useListPages';

interface Row {
  id: string;
  state: string;
}
const FAST_MS = 20;
const SETTLE_MS = 100;
const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const row = (n: number, state = 'queued'): Row => ({ id: `r${n}`, state });
const page = (ns: number[], nextCursor?: string): ListPage<Row> => ({ items: ns.map((n) => row(n)), nextCursor });
const ids = (rows: readonly Row[] | undefined): string[] => (rows ?? []).map((r) => r.id);
const id = (r: Row): string => r.id;

type Load = (cursor: string | undefined, signal: AbortSignal) => Promise<ListPage<Row>>;
const setup = (loadPage: Load, intervalMs = FAST_MS) => renderHook(() => useListPages<Row>({ loadPage, id, intervalMs }));

describe('useListPages', () => {
  afterEach(() => {
    cleanup();
    setToken(null);
  });

  it('has no rows until the first page arrives, then the rows and when they arrived', async () => {
    const before = Date.now();
    const { result } = setup(vi.fn(async () => page([3, 2, 1])));
    expect(result.current.rows).toBeUndefined();
    expect(result.current.updatedAt).toBeUndefined();
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r3', 'r2', 'r1']));
    expect(result.current.updatedAt).toBeGreaterThanOrEqual(before);
    expect(result.current.hasMore).toBe(false);
  });

  it('asks for the first page without a cursor, and says more is there when the server sent a cursor', async () => {
    const load = vi.fn<Load>(async () => page([3, 2], '2'));
    const { result } = setup(load);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    expect(load.mock.calls[0][0]).toBeUndefined();
  });

  it('loads the next page with the cursor and appends it', async () => {
    const load = vi.fn<Load>(async (cursor: string | undefined, _signal: AbortSignal) => (cursor === undefined ? page([5, 4], '2') : page([3, 2], '4')));
    const { result } = setup(load, 60_000);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r5', 'r4', 'r3', 'r2']));
    expect(load).toHaveBeenLastCalledWith('2', expect.any(AbortSignal));
    expect(result.current.moreLoads).toBe(1);
    expect(result.current.loadingMore).toBe(false);
    expect(result.current.hasMore).toBe(true);
    act(() => result.current.loadMore());
    await waitFor(() => expect(load).toHaveBeenLastCalledWith('4', expect.any(AbortSignal)));
  });

  it('makes one request for older rows at a time', async () => {
    let release: (p: ListPage<Row>) => void = () => undefined;
    const load = vi.fn<Load>((cursor) => (cursor === undefined ? Promise.resolve(page([2, 1], '2')) : new Promise((r) => (release = r))));
    const { result } = setup(load, 60_000);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => {
      result.current.loadMore();
      result.current.loadMore();
    });
    expect(load).toHaveBeenCalledTimes(2);
    expect(result.current.loadingMore).toBe(true);
    await act(async () => release(page([0])));
    expect(result.current.loadingMore).toBe(false);
  });

  it('keeps the rows and says why when a further page fails, and tries again when asked again', async () => {
    const load = vi
      .fn<Load>()
      .mockResolvedValueOnce(page([2, 1], '2'))
      .mockRejectedValueOnce(new ApiError(503, 'database is locked'))
      .mockResolvedValue(page([0]));
    const { result } = setup(load, 60_000);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.moreError).toBe('database is locked'));
    expect(ids(result.current.rows)).toEqual(['r2', 'r1']);
    expect(result.current.hasMore).toBe(true);
    expect(result.current.error).toBeUndefined();
    act(() => result.current.loadMore());
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r2', 'r1', 'r0']));
    expect(result.current.moreError).toBeUndefined();
  });

  it('keeps showing the rows it has when a refresh fails, with the reason, and the time of the last good load', async () => {
    const load = vi.fn<Load>().mockResolvedValueOnce(page([2, 1])).mockRejectedValue(new Error('down'));
    const { result } = setup(load);
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r2', 'r1']));
    const first = result.current.updatedAt;
    await waitFor(() => expect(result.current.error).toBe('down'));
    expect(ids(result.current.rows)).toEqual(['r2', 'r1']);
    expect(result.current.updatedAt).toBe(first);
  });

  it('shows a refreshed first page and keeps the older pages already loaded', async () => {
    let refreshed = false;
    const load = vi.fn<Load>(async (cursor) => {
      if (cursor !== undefined) return page([2, 1]);
      return refreshed ? page([4, 3], '2') : page([3, 2], '2');
    });
    const { result } = setup(load);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r3', 'r2', 'r1']));
    refreshed = true;
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r4', 'r3', 'r2', 'r1']));
  });

  it('does not ask for older rows when there are none', async () => {
    const load = vi.fn<Load>(async () => page([1]));
    const { result } = setup(load, 60_000);
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r1']));
    act(() => result.current.loadMore());
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('ignores a page that arrives after the page was left', async () => {
    let release: (p: ListPage<Row>) => void = () => undefined;
    const load = vi.fn<Load>((cursor) => (cursor === undefined ? Promise.resolve(page([2, 1], '2')) : new Promise((r) => (release = r))));
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const { result, unmount } = setup(load, 60_000);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    const signal = load.mock.calls[1][1];
    unmount();
    expect(signal.aborted).toBe(true);
    await act(async () => release(page([0])));
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });

  it('stops on a 401 and loads again when a token is entered', async () => {
    const load = vi.fn<Load>().mockRejectedValueOnce(new ApiError(401, 'unauthorized')).mockResolvedValue(page([1]));
    const { result } = setup(load);
    await waitFor(() => expect(result.current.error).toBe('unauthorized'));
    await pause(SETTLE_MS);
    expect(load).toHaveBeenCalledTimes(1);
    act(() => setToken('a-token-0123456789abcdef'));
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r1']));
    expect(result.current.error).toBeUndefined();
  });

  it('starts the list again from the first page when newer rows arrived than a page holds', async () => {
    let phase = 0;
    const load = vi.fn<Load>(async (cursor) => {
      if (cursor !== undefined) return page([2, 1]);
      return phase === 0 ? page([4, 3], '2') : page([90, 89], '2');
    });
    const { result } = setup(load);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r4', 'r3', 'r2', 'r1']));
    phase = 1;
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r90', 'r89']));
    expect(result.current.hasMore).toBe(true);
  });

  it('says the list cannot be read further, and shows no failure, when the server refuses the cursor as too deep', async () => {
    const load = vi
      .fn<Load>()
      .mockResolvedValueOnce(page([2, 1], '100000'))
      .mockRejectedValueOnce(new ApiError(400, '`cursor` is not valid'));
    const { result } = setup(load, 60_000);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.endReached).toBe(true));
    expect(result.current.hasMore).toBe(false);
    expect(result.current.moreError).toBeUndefined();
    expect(ids(result.current.rows)).toEqual(['r2', 'r1']);
  });

  it('keeps showing an ordinary 400 for a further page as a failure', async () => {
    const load = vi.fn<Load>().mockResolvedValueOnce(page([2, 1], '2')).mockRejectedValueOnce(new ApiError(400, '`limit` is not valid'));
    const { result } = setup(load, 60_000);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.moreError).toBe('`limit` is not valid'));
    expect(result.current.endReached).toBe(false);
  });

  it('carries on from the cursor an empty further page gave, through a refresh of the first page', async () => {
    const load = vi.fn<Load>(async (cursor) => {
      if (cursor === undefined) return page([], '1000');
      return cursor === '1000' ? page([], '2000') : page([5], undefined);
    });
    const { result } = setup(load);
    await waitFor(() => expect(result.current.hasMore).toBe(true));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.moreLoads).toBe(1));
    await pause(SETTLE_MS);
    act(() => result.current.loadMore());
    await waitFor(() => expect(load).toHaveBeenLastCalledWith('2000', expect.any(AbortSignal)));
    await waitFor(() => expect(ids(result.current.rows)).toEqual(['r5']));
  });
});

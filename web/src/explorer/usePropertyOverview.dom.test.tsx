// @vitest-environment happy-dom
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError, setToken } from '../api';
import type { PropertyPage } from '../types';
import { page, row } from './fixtures';
import { usePropertyOverview } from './usePropertyOverview';

const pageOf = (id: string): PropertyPage => page({ properties: [row({ idx: 0, guest: { id, msg: '', loc: null } })] });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('usePropertyOverview in a DOM', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    setToken(null);
  });

  it('reload drops the rows it holds at once and loads the table again from the start', async () => {
    const second = deferred<PropertyPage>();
    const spy = vi.spyOn(api, 'properties').mockResolvedValueOnce(pageOf('old.row')).mockImplementationOnce(() => second.promise);
    const { result } = renderHook(() => usePropertyOverview('run_1', false));
    await waitFor(() => expect(result.current.rows).toHaveLength(1));
    expect(result.current.rows[0]?.guest.id).toBe('old.row');

    act(() => result.current.reload());
    expect(result.current.rows).toHaveLength(0);
    expect(result.current.page).toBeUndefined();

    await act(async () => second.resolve(pageOf('new.row')));
    await waitFor(() => expect(result.current.rows[0]?.guest.id).toBe('new.row'));
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('stops on a 401, and loads the table when a token is entered', async () => {
    const spy = vi.spyOn(api, 'properties').mockRejectedValueOnce(new ApiError(401, 'unauthorized')).mockResolvedValue(pageOf('after.token'));
    const { result } = renderHook(() => usePropertyOverview('run_1', true));
    await waitFor(() => expect(result.current.error).toBe('unauthorized'));
    expect(spy).toHaveBeenCalledTimes(1);

    act(() => setToken('a token'));
    await waitFor(() => expect(result.current.rows[0]?.guest.id).toBe('after.token'));
    expect(result.current.error).toBeUndefined();
  });
});

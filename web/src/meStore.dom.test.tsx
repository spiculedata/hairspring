// @vitest-environment happy-dom
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api, setToken } from './api';
import type { Me } from './me';
import { resetMeForTests, useMe } from './meStore';

const read: Me = { role: 'read', open_mode: false, can: { read: true, run: false, admin: false }, roles: ['read', 'run', 'admin'] };
const admin: Me = { role: 'admin', open_mode: false, can: { read: true, run: true, admin: true }, roles: ['read', 'run', 'admin'] };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

function Show({ id }: { id: string }) {
  const s = useMe();
  return <p data-testid={id}>{s.status === 'ready' ? s.me.role : s.status}</p>;
}
const text = (c: HTMLElement, id: string) => c.querySelector(`[data-testid="${id}"]`)?.textContent;

describe('useMe', () => {
  beforeEach(() => {
    resetMeForTests();
    setToken(null);
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    setToken(null);
    resetMeForTests();
  });

  it('asks the server once for every component that wants the role, and shares the answer', async () => {
    const fetchMock = vi.fn(async (_url: string) => json(read));
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(
      <>
        <Show id="a" />
        <Show id="b" />
      </>,
    );
    expect(text(container, 'a')).toBe('loading');
    await waitFor(() => expect(text(container, 'a')).toBe('read'));
    expect(text(container, 'b')).toBe('read');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toBe('/api/me');
  });

  it('keeps the answer across a page change and does not ask again', async () => {
    const fetchMock = vi.fn(async (_url: string) => json(read));
    vi.stubGlobal('fetch', fetchMock);
    const first = render(<Show id="a" />);
    await waitFor(() => expect(text(first.container, 'a')).toBe('read'));
    first.unmount();
    const second = render(<Show id="a" />);
    expect(text(second.container, 'a')).toBe('read');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('asks again with the new token after the token changes', async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      const auth = (init?.headers as Record<string, string> | undefined)?.Authorization;
      return json(auth === 'Bearer admin-token-0123456789' ? admin : read);
    });
    vi.stubGlobal('fetch', fetchMock);
    setToken('read-token-0123456789');
    const { container } = render(<Show id="a" />);
    await waitFor(() => expect(text(container, 'a')).toBe('read'));
    act(() => setToken('admin-token-0123456789'));
    await waitFor(() => expect(text(container, 'a')).toBe('admin'));
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('does not let a slow answer for the old token overwrite the answer for the new one', async () => {
    let releaseOld: (r: Response) => void = () => undefined;
    const fetchMock = vi
      .fn()
      .mockImplementationOnce(() => new Promise<Response>((resolve) => (releaseOld = resolve)))
      .mockImplementationOnce(async () => json(admin));
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<Show id="a" />);
    act(() => setToken('admin-token-0123456789'));
    await waitFor(() => expect(text(container, 'a')).toBe('admin'));
    await act(async () => releaseOld(json(read)));
    expect(text(container, 'a')).toBe('admin');
  });

  it('is unavailable, not a guessed role, when the server has no /api/me, fails, or sends a wrong shape', async () => {
    const replies: Array<() => Promise<Response>> = [
      async () => json({ error: 'not found' }, 404),
      async () => json({ role: 'root' }),
      async () => json({}, 500),
      () => Promise.reject(new TypeError('offline')),
    ];
    for (const reply of replies) {
      resetMeForTests();
      vi.stubGlobal('fetch', vi.fn(reply));
      const { container, unmount } = render(<Show id="a" />);
      await waitFor(() => expect(text(container, 'a')).toBe('unavailable'));
      unmount();
    }
  });
});

describe('api.me', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('rejects a body of the wrong shape with a 502-style error', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ role: 'admin' })));
    await expect(api.me()).rejects.toMatchObject({ status: 502 });
  });

  it('returns the caller for a valid body', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(admin)));
    await expect(api.me()).resolves.toEqual(admin);
  });
});

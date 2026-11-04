// @vitest-environment happy-dom
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setToken } from '../api';
import type { Me } from '../me';
import { resetMeForTests } from '../meStore';
import { Runs } from './Runs';

const me = (role: Me['role'], open = false): Me => ({
  role,
  open_mode: open,
  can: { read: true, run: role !== 'read', admin: role === 'admin' },
  roles: ['read', 'run', 'admin'],
});
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const target = { name: 'pubsub-buggy', kind: 'sim', description: 'd', budget: { max_steps: 10, max_trials: 5, wall_secs: 6 } };

function serve(meReply: () => Response) {
  const fetchMock = vi.fn(async (url: string) => {
    if (url === '/api/me') return meReply();
    if (url === '/api/targets') return json({ targets: [target] });
    if (url.startsWith('/api/runs')) return json({ runs: [] });
    return json({ error: 'not found' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}
const button = (c: HTMLElement) => c.querySelector('button[type="submit"]') as HTMLButtonElement;
const note = (c: HTMLElement) => c.querySelector('.start-access-note');

describe("Runs page: the start form follows the caller's role", () => {
  beforeEach(() => {
    resetMeForTests();
    setToken(null);
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    resetMeForTests();
  });

  it('disables Start run for a read caller and explains why in one line', async () => {
    serve(() => json(me('read')));
    const { container } = render(<Runs />);
    await waitFor(() =>
      expect(note(container)?.textContent).toBe('Your role (read) cannot start runs or sweeps. Ask an administrator for the run role.'),
    );
    expect(button(container).disabled).toBe(true);
    expect(button(container).getAttribute('aria-describedby')).toBe(note(container)?.id);
  });

  it.each([
    ['run', me('run')],
    ['admin', me('admin')],
    ['open mode', me('admin', true)],
  ])('lets a %s caller start a run, with no note', async (_name, who) => {
    const fetchMock = serve(() => json(who));
    const { container } = render(<Runs />);
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => u === '/api/me')).toBe(true));
    await waitFor(() => expect(button(container).disabled).toBe(false));
    expect(note(container)).toBeNull();
  });

  it('leaves the button to the server when /api/me is not available (an older server)', async () => {
    serve(() => json({ error: 'not found' }, 404));
    const { container } = render(<Runs />);
    await waitFor(() => expect(button(container).disabled).toBe(false));
    expect(note(container)).toBeNull();
  });

  it('asks /api/me once for the page', async () => {
    const fetchMock = serve(() => json(me('run')));
    const { container } = render(<Runs />);
    await waitFor(() => expect(button(container).disabled).toBe(false));
    expect(fetchMock.mock.calls.filter(([u]) => u === '/api/me')).toHaveLength(1);
  });
});

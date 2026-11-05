// @vitest-environment happy-dom
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setToken } from '../api';
import type { Me } from '../me';
import { resetMeForTests } from '../meStore';
import type { Worker, WorkersOverview } from '../types';
import { WorkersPage } from './WorkersPage';
import { CAPS_NOT_STATED, HOLDING_NOTHING, ID_HIDDEN_NOTE, NO_WORKERS, NO_WORKERS_HINT, STATE_NOT_REPORTED, TRUST_NOT_REPORTED } from './workersWording';

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const me = (role: Me['role'], open = false): Me => ({
  role,
  open_mode: open,
  can: { read: true, run: role !== 'read', admin: role === 'admin' },
  roles: ['read', 'run', 'admin'],
});

const NOW = Date.now();
const busy: Worker = {
  id: 'wrk-busy',
  name: 'remote-1',
  registered_at: NOW - 10_000,
  last_seen: NOW - 1000,
  state: 'busy',
  trusted: false,
  capabilities: { backends: ['sim', 'embedded'] },
  caps_json: '{"backends":["sim","embedded"]}',
  current_jobs: [{ run_id: 'run_abc', kind: 'run', lease_until: NOW + 60_000, attempts: 1 }],
  leased_jobs: 1,
};
const idle: Worker = {
  id: 'wrk-idle',
  name: 'embedded-0',
  registered_at: NOW - 10_000,
  last_seen: NOW - 2000,
  state: 'idle',
  trusted: true,
  capabilities: {},
  caps_json: '{}',
  current_jobs: [],
  leased_jobs: 0,
};
const quiet: Worker = { ...idle, id: 'wrk-quiet', name: 'gone-quiet', state: 'not_seen_recently', trusted: false };

const withoutId = (w: Worker): Worker => {
  const { id: _id, ...rest } = w;
  return rest;
};

function serve(role: Me['role'], overview: WorkersOverview | number) {
  const fetchMock = vi.fn(async (url: string) => {
    if (url === '/api/me') return json(me(role));
    if (url === '/api/workers') return typeof overview === 'number' ? json({ error: 'boom' }, overview) : json(overview);
    return json({ error: 'not found' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

const headers = (c: HTMLElement) => [...c.querySelectorAll('th')].map((th) => th.textContent);
const rows = (c: HTMLElement) => [...c.querySelectorAll('tbody tr')] as HTMLElement[];
const text = (c: HTMLElement) => c.textContent ?? '';

describe('Workers page', () => {
  beforeEach(() => {
    resetMeForTests();
    setToken(null);
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    resetMeForTests();
  });

  it('shows state, trust, capabilities and what each worker holds, and the queue', async () => {
    serve('run', { workers: [busy, idle, quiet], queue: { queued: 2, running: 1 } });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(rows(container)).toHaveLength(3));
    expect(headers(container)).toEqual(['Name', 'State', 'Trust', 'Capabilities', 'Holding', 'Last seen', 'Id']);
    const [first, second, third] = rows(container);
    expect(first.textContent).toContain('remote-1');
    expect(first.querySelector('.badge-busy')?.textContent).toBe('Busy');
    expect(first.querySelector('.badge-gap')?.textContent).toBe('Untrusted');
    expect(first.textContent).toContain('backends: sim, embedded');
    const link = first.querySelector('a');
    expect(link?.getAttribute('href')).toBe('/runs/run_abc');
    expect(link?.textContent).toBe('run_abc');
    expect(first.querySelector('code')?.textContent).toBe('wrk-busy');
    expect(second.querySelector('.badge-idle')?.textContent).toBe('Idle');
    expect(second.querySelector('.badge-good')?.textContent).toBe('Trusted');
    expect(second.textContent).toContain(CAPS_NOT_STATED);
    expect(second.textContent).toContain(HOLDING_NOTHING);
    expect(third.querySelector('.badge-gap')?.textContent).toBe('Not seen recently');
    expect(text(container)).toContain('Queue: 2 jobs waiting for a worker. 1 job held by workers.');
  });

  it('says how many more jobs a worker holds than are named', async () => {
    serve('run', { workers: [{ ...busy, leased_jobs: 5 }], queue: { queued: 0, running: 5 } });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(text(container)).toContain('and 4 more'));
  });

  it('leaves out the id column for a read caller and says why', async () => {
    serve('read', { workers: [withoutId(busy), withoutId(idle)], queue: { queued: 0, running: 1 } });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(text(container)).toContain(ID_HIDDEN_NOTE));
    expect(headers(container)).not.toContain('Id');
    expect(container.querySelector('code')).toBeNull();
    expect(text(container)).not.toContain('wrk-');
  });

  it('shows the id column to a run caller and gives no id note', async () => {
    serve('run', { workers: [busy] });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(headers(container)).toContain('Id'));
    expect(text(container)).not.toContain(ID_HIDDEN_NOTE);
  });

  it('does not guess the role while it is being fetched or when it cannot be fetched', async () => {
    const fetchMock = vi.fn(async (url: string) => (url === '/api/workers' ? json({ workers: [withoutId(busy)] }) : json({ error: 'x' }, 500)));
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(rows(container)).toHaveLength(1));
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => u === '/api/me')).toBe(true));
    expect(text(container)).not.toContain(ID_HIDDEN_NOTE);
  });

  it('says there are no workers and how one starts', async () => {
    serve('read', { workers: [], queue: { queued: 0, running: 0 } });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(text(container)).toContain(NO_WORKERS));
    expect(text(container)).toContain(NO_WORKERS_HINT);
    expect(container.querySelector('table')).toBeNull();
  });

  it('says what an older server leaves out, and shows no queue line', async () => {
    serve('run', { workers: [{ id: 'wrk-old', name: 'old', registered_at: 1, last_seen: NOW }] });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(rows(container)).toHaveLength(1));
    expect(text(container)).toContain(STATE_NOT_REPORTED);
    expect(text(container)).toContain(TRUST_NOT_REPORTED);
    expect(text(container)).not.toContain('Queue:');
  });

  it('shows a word it does not know as Unknown, never as busy', async () => {
    serve('run', { workers: [{ ...busy, state: 'hibernating' }] });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(rows(container)).toHaveLength(1));
    expect(container.querySelector('.badge-busy')).toBeNull();
    expect(rows(container)[0].querySelector('.badge-idle')?.textContent).toBe('Unknown');
  });

  it('shows text a worker sent as text, never as markup', async () => {
    const hostile = '<img src=x onerror=alert(1)>';
    serve('run', { workers: [{ ...busy, name: hostile, capabilities: { note: hostile } }] });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(rows(container)).toHaveLength(1));
    expect(container.querySelector('img')).toBeNull();
    expect(text(container)).toContain(hostile);
  });

  it('reports a failed load by name and keeps the table it already had', async () => {
    let calls = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        if (url === '/api/me') return json(me('read'));
        calls += 1;
        return calls === 1 ? json({ workers: [withoutId(busy)] }) : json({ error: 'down' }, 500);
      }),
    );
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { container } = render(<WorkersPage />);
      await waitFor(() => expect(rows(container)).toHaveLength(1));
      await vi.advanceTimersByTimeAsync(4000);
      await waitFor(() => expect(text(container)).toContain('Could not load workers'));
      expect(rows(container)).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('fits a narrow screen: the table sits in a scroll region', async () => {
    serve('run', { workers: [busy] });
    const { container } = render(<WorkersPage />);
    await waitFor(() => expect(rows(container)).toHaveLength(1));
    const region = container.querySelector('[role="region"]');
    expect(region?.getAttribute('aria-label')).toBe('Workers');
    expect(region?.querySelector('table.workers')).not.toBeNull();
  });
});

// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetMeForTests } from '../meStore';
import type { Run } from '../types';
import { Runs } from './Runs';

const run = (n: number, over: Partial<Run> = {}): Run => ({
  id: `run_${String(n).padStart(16, '0')}`,
  target: 'lock-lease',
  backend: 'sim',
  state: 'queued',
  seed: 1,
  created_at: 1_000 + n,
  started_at: null,
  finished_at: null,
  steps: 0,
  trials: 0,
  error: null,
  ...over,
});
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const many = (from: number, n: number): Run[] => Array.from({ length: n }, (_, i) => run(from - i));
const runUrls = (fetchMock: ReturnType<typeof vi.fn>): string[] =>
  fetchMock.mock.calls.map((c) => String((c as unknown[])[0])).filter((u) => u.startsWith('/api/runs'));

/** Answers the runs list with `list`, and the targets the start form asks for with none. */
const serve = (list: (url: string) => Response) =>
  vi.fn(async (url: string) => {
    if (url.startsWith('/api/targets')) return json({ targets: [] });
    if (url === '/api/me') return json({ error: 'not found' }, 404);
    return list(url);
  });

/** The filter bar, apart from the start form above it, which has a target of its own. */
const bar = () => within(screen.getByRole('search', { name: 'Filter runs' }));

const rows = (container: HTMLElement): number => container.querySelectorAll('table.runs tbody tr').length;

describe('Runs page', () => {
  beforeEach(() => {
    resetMeForTests();
    window.history.replaceState({}, '', '/runs');
  });
  afterEach(() => {
    cleanup();
    resetMeForTests();
    vi.unstubAllGlobals();
    window.history.replaceState({}, '', '/');
  });

  it('says what is loading, then lists the runs newest first as the server sent them', async () => {
    vi.stubGlobal('fetch', serve(() => json({ runs: [run(3), run(2), run(1)] })));
    const { container } = render(<Runs />);
    expect(container.textContent).toContain('Loading runs…');
    await waitFor(() => expect(rows(container)).toBe(3));
    expect(container.querySelector('table.runs tbody tr')?.textContent).toContain('run_0000000000000003');
    expect(container.textContent).toContain('Updated');
  });

  it('asks for the newest page with the page size and no filters', async () => {
    const fetchMock = serve(() => json({ runs: [] }));
    vi.stubGlobal('fetch', fetchMock);
    render(<Runs />);
    await waitFor(() => expect(runUrls(fetchMock)).toHaveLength(1));
    expect(runUrls(fetchMock)[0]).toBe('/api/runs?limit=50');
  });

  it('says what to do when there are no runs at all', async () => {
    vi.stubGlobal('fetch', serve(() => json({ runs: [] })));
    const { container } = render(<Runs />);
    await waitFor(() => expect(container.textContent).toContain('No runs yet.'));
  });

  it('says it could not load, with the reason, and keeps the form and the filters', async () => {
    vi.stubGlobal('fetch', serve(() => json({ error: 'database is locked' }, 503)));
    const { container } = render(<Runs />);
    await waitFor(() => expect(container.textContent).toContain('Could not load runs: database is locked'));
    expect(screen.getByRole('search', { name: 'Filter runs' })).toBeTruthy();
    expect(container.textContent).not.toContain('No runs yet.');
  });

  it('loads the next page with the cursor, adds its rows, and stops offering more at the end', async () => {
    const fetchMock = serve((url) => json(url.includes('cursor=50') ? { runs: many(10, 4) } : { runs: many(60, 50), next_cursor: '50' }));
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<Runs />);
    await waitFor(() => expect(rows(container)).toBe(50));
    fireEvent.click(screen.getByRole('button', { name: 'Load more runs (50 shown)' }));
    await waitFor(() => expect(rows(container)).toBe(54));
    expect(runUrls(fetchMock)).toContain('/api/runs?cursor=50&limit=50');
    expect(screen.queryByRole('button', { name: /load more/i })).toBeNull();
    expect(screen.getByRole('status', { name: '' }).textContent).toBe('54 runs shown.');
  });

  it('keeps the rows and says why when a further page fails', async () => {
    vi.stubGlobal(
      'fetch',
      serve((url) => (url.includes('cursor=') ? json({ error: 'database is locked' }, 503) : json({ runs: many(5, 2), next_cursor: '2' }))),
    );
    const { container } = render(<Runs />);
    await waitFor(() => expect(rows(container)).toBe(2));
    fireEvent.click(screen.getByRole('button', { name: /load more/i }));
    await waitFor(() => expect(container.textContent).toContain('Could not load more runs: database is locked'));
    expect(rows(container)).toBe(2);
    expect(screen.getByRole('button', { name: /load more/i })).toBeTruthy();
  });

  it('filters by state, run type and target: a new list each time, the address kept in step', async () => {
    const fetchMock = serve((url) => json({ runs: url.includes('state=failed') ? [run(9, { state: 'failed' })] : [run(2), run(1)] }));
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<Runs />);
    await waitFor(() => expect(rows(container)).toBe(2));
    fireEvent.change(bar().getByLabelText('State'), { target: { value: 'failed' } });
    await waitFor(() => expect(rows(container)).toBe(1));
    expect(runUrls(fetchMock)).toContain('/api/runs?state=failed&limit=50');
    fireEvent.change(bar().getByLabelText('Run type'), { target: { value: 'sweep' } });
    await waitFor(() => expect(runUrls(fetchMock)).toContain('/api/runs?state=failed&mode=sweep&limit=50'));
    const target = bar().getByLabelText('Target');
    fireEvent.change(target, { target: { value: 'pubsub-buggy' } });
    fireEvent.keyDown(target, { key: 'Enter' });
    await waitFor(() => expect(runUrls(fetchMock)).toContain('/api/runs?state=failed&target=pubsub-buggy&mode=sweep&limit=50'));
    expect(window.location.pathname + window.location.search).toBe('/runs?state=failed&target=pubsub-buggy&mode=sweep');
  });

  it('starts from the filters in the address and leaves out any the server would refuse', async () => {
    window.history.replaceState({}, '', '/runs?state=cancelled&mode=nope&target=x');
    const fetchMock = serve(() => json({ runs: [] }));
    vi.stubGlobal('fetch', fetchMock);
    render(<Runs />);
    await waitFor(() => expect(runUrls(fetchMock)).toHaveLength(1));
    expect(runUrls(fetchMock)[0]).toBe('/api/runs?state=cancelled&target=x&limit=50');
    expect((bar().getByLabelText('State') as HTMLSelectElement).value).toBe('cancelled');
    expect((bar().getByLabelText('Run type') as HTMLSelectElement).value).toBe('');
  });

  it('says no run matches when a filtered list is empty, and does not tell the reader to start one', async () => {
    window.history.replaceState({}, '', '/runs?state=error');
    vi.stubGlobal('fetch', serve(() => json({ runs: [] })));
    const { container } = render(<Runs />);
    await waitFor(() => expect(container.textContent).toContain('No runs match these filters.'));
    expect(container.textContent).not.toContain('No runs yet.');
  });

  it('refuses a target the server would refuse, with the reason, and sends no request for it', async () => {
    const fetchMock = serve(() => json({ runs: [] }));
    vi.stubGlobal('fetch', fetchMock);
    render(<Runs />);
    await waitFor(() => expect(runUrls(fetchMock)).toHaveLength(1));
    const target = bar().getByLabelText('Target');
    fireEvent.change(target, { target: { value: 'x'.repeat(65) } });
    fireEvent.blur(target);
    expect(screen.getByRole('alert').textContent).toContain('64 bytes');
    expect(runUrls(fetchMock)).toHaveLength(1);
  });

  it('shows a run whose state changed after the first load, from the next refresh', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      let state: Run['state'] = 'running';
      vi.stubGlobal('fetch', serve(() => json({ runs: [run(1, { state })] })));
      const { container } = render(<Runs />);
      await waitFor(() => expect(container.querySelector('table.runs tbody')?.textContent).toContain('Running'));
      state = 'failed';
      await vi.advanceTimersByTimeAsync(2100);
      await waitFor(() => expect(container.querySelector('table.runs tbody')?.textContent).toContain('Failed'));
    } finally {
      vi.useRealTimers();
    }
  });

  describe.each([
    ['with no state filter', '/runs', 4],
    ['with a state filter', '/runs?state=running', 3],
  ])('a run pushed off the newest page by a newer one, %s', (_name, address, expectedRows) => {
    it('is kept only when no filter can have taken it out of the list', async () => {
      window.history.replaceState({}, '', address);
      vi.useFakeTimers({ shouldAdvanceTime: true });
      try {
        let head: Run[] = [run(5), run(4)];
        vi.stubGlobal('fetch', serve((url) => json(url.includes('cursor=') ? { runs: [run(1)] } : { runs: head, next_cursor: '2' })));
        const { container } = render(<Runs />);
        await waitFor(() => expect(rows(container)).toBe(2));
        fireEvent.click(screen.getByRole('button', { name: /load more/i }));
        await waitFor(() => expect(rows(container)).toBe(3));
        head = [run(6), run(4)];
        await vi.advanceTimersByTimeAsync(2100);
        await waitFor(() => expect(container.querySelector('table.runs tbody')?.textContent).toContain('run_0000000000000006'));
        expect(rows(container)).toBe(expectedRows);
      } finally {
        vi.useRealTimers();
      }
    });
  });
});

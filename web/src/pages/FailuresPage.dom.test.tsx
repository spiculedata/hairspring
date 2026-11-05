// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FailureSummary } from '../types';
import { FailuresPage } from './FailuresPage';

const row = (id: string, created_at: number, over: Partial<FailureSummary> = {}): FailureSummary => ({
  id,
  run_id: 'run_1',
  target: 't',
  message: 'm',
  original_len: 3,
  shrunk_len: 1,
  bundle_hash: 'cd'.repeat(32),
  created_at,
  ...over,
});
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const many = (from: number, n: number): FailureSummary[] => Array.from({ length: n }, (_, i) => row(`f_${from - i}`, from - i));
const urls = (fetchMock: ReturnType<typeof vi.fn>): string[] => fetchMock.mock.calls.map((c) => String((c as unknown[])[0]));
const RUN = 'run_0123456789abcdef';

describe('FailuresPage', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn());
    window.history.replaceState({}, '', '/failures');
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.history.replaceState({}, '', '/');
  });

  it('says what is loading, then lists the failures', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [row('f_a', 1), row('f_b', 2)] })));
    const { container } = render(<FailuresPage />);
    expect(container.textContent).toContain('Loading failures…');
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(2));
    expect(container.querySelector('h1')?.textContent).toBe('Failures');
    expect(container.querySelector('tbody tr')?.textContent).toContain('f_b');
    expect(container.textContent).toContain('Updated');
  });

  it('asks for the newest page with the page size and no filters', async () => {
    const fetchMock = vi.fn(async () => json({ failures: [] }));
    vi.stubGlobal('fetch', fetchMock);
    render(<FailuresPage />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(urls(fetchMock)[0]).toBe('/api/failures?limit=50');
  });

  it('says there are none yet when the list is empty, and shows no table', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [] })));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.textContent).toContain('No failures reported yet.'));
    expect(container.querySelector('table')).toBeNull();
  });

  it('says it could not load, with the reason, and does not claim the list is empty', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ error: 'database is locked' }, 503)));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelector('[role="alert"]')).not.toBeNull());
    expect(container.textContent).toContain('Could not load failures: database is locked');
    expect(container.textContent).not.toContain('No failures reported yet.');
  });

  it('offers to load more only when the server sent a cursor, and no longer says the list is cut off', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: many(50, 50) })));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(50));
    expect(screen.queryByRole('button', { name: /load more/i })).toBeNull();
    expect(container.textContent).not.toContain('There may be older ones');
  });

  it('loads the next page with the cursor and adds its rows below, then stops offering more at the end', async () => {
    const fetchMock = vi.fn(async (url: string) =>
      json(url.includes('cursor=50') ? { failures: many(10, 10) } : { failures: many(60, 50), next_cursor: '50' }),
    );
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(50));
    fireEvent.click(screen.getByRole('button', { name: 'Load more failures (50 shown)' }));
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(60));
    expect(urls(fetchMock)).toContain('/api/failures?cursor=50&limit=50');
    expect(screen.queryByRole('button', { name: /load more/i })).toBeNull();
    expect(screen.getByRole('status').textContent).toBe('60 failures shown.');
  });

  it('keeps the rows and says why when a further page fails, and asking again tries again', async () => {
    let failNext = true;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        if (!url.includes('cursor=')) return json({ failures: many(5, 2), next_cursor: '2' });
        if (failNext) {
          failNext = false;
          return json({ error: 'database is locked' }, 503);
        }
        return json({ failures: many(3, 1) });
      }),
    );
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(2));
    fireEvent.click(screen.getByRole('button', { name: /load more/i }));
    await waitFor(() => expect(container.textContent).toContain('Could not load more failures: database is locked'));
    expect(container.querySelectorAll('tbody tr')).toHaveLength(2);
    fireEvent.click(screen.getByRole('button', { name: /load more/i }));
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(3));
    expect(container.textContent).not.toContain('Could not load more');
  });

  it('filters by state when one is chosen: a new list, a request with the state, the address kept in step, and a State column', async () => {
    const fetchMock = vi.fn(async (url: string) =>
      json(
        url.includes('status=confirmed')
          ? { failures: [row('f_c', 3, { finding: { status: 'confirmed', checks: 2, environment_pinned: false } })] }
          : { failures: [row('f_a', 1), row('f_b', 2)] },
      ),
    );
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(2));
    fireEvent.change(screen.getByLabelText('State'), { target: { value: 'confirmed' } });
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(1));
    expect(urls(fetchMock)).toContain('/api/failures?status=confirmed&limit=50');
    expect(window.location.pathname + window.location.search).toBe('/failures?status=confirmed');
    expect(container.querySelector('tbody tr')?.textContent).toContain('f_c');
    expect(container.querySelector('thead')?.textContent).toContain('State');
    expect(container.querySelector('tbody tr')?.textContent).toContain('Confirmed');
  });

  it('says when a property filter left older runs out, and not otherwise', async () => {
    window.history.replaceState({}, '', '/failures?property=db.');
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [row('f_a', 1)], scope: { recent_runs: 1000, cut_short: true } })));
    const { container } = render(<FailuresPage />);
    await waitFor(() =>
      expect(container.textContent).toContain('Results are limited to the newest 1,000 runs; add a run id to search an older run.'),
    );
    cleanup();
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [row('f_a', 1)], scope: { recent_runs: 1000, cut_short: false } })));
    const second = render(<FailuresPage />);
    await waitFor(() => expect(second.container.querySelectorAll('tbody tr')).toHaveLength(1));
    expect(second.container.textContent).not.toContain('Results are limited');
  });

  it('starts from the filters in the address, and leaves out any the server would refuse', async () => {
    window.history.replaceState({}, '', `/failures?run=${RUN}&status=verified&property=db.&junk=1&status2=x`);
    const fetchMock = vi.fn(async () => json({ failures: [] }));
    vi.stubGlobal('fetch', fetchMock);
    render(<FailuresPage />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(urls(fetchMock)[0]).toBe(`/api/failures?run=${RUN}&property=db.&status=verified&limit=50`);
    expect((screen.getByLabelText('Run id') as HTMLInputElement).value).toBe(RUN);
    expect((screen.getByLabelText('State') as HTMLSelectElement).value).toBe('verified');
    cleanup();
    window.history.replaceState({}, '', '/failures?status=reproduced&run=nope');
    fetchMock.mockClear();
    render(<FailuresPage />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(urls(fetchMock)[0]).toBe('/api/failures?limit=50');
  });

  it('says no failure matches when a filtered list is empty, not that none were reported', async () => {
    window.history.replaceState({}, '', '/failures?status=verified');
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [] })));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.textContent).toContain('No failures match these filters.'));
    expect(container.textContent).not.toContain('No failures reported yet.');
  });

  it('offers to read further when a state filter found nothing in the part it read', async () => {
    window.history.replaceState({}, '', '/failures?status=verified');
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [], next_cursor: '1000' })));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.textContent).toContain('None of the failures read so far match.'));
    expect(container.textContent).not.toContain('No failures match these filters.');
    expect(screen.getByRole('button', { name: /load more/i })).toBeTruthy();
  });

  it('clears the filters with Clear filters and loads the whole list again', async () => {
    window.history.replaceState({}, '', '/failures?status=candidate');
    const fetchMock = vi.fn(async () => json({ failures: [row('f_a', 1)] }));
    vi.stubGlobal('fetch', fetchMock);
    render(<FailuresPage />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    await waitFor(() => expect(urls(fetchMock)).toContain('/api/failures?limit=50'));
    expect(window.location.search).toBe('');
  });

  it('shows what a refused value looks like to the reader, from the server, when a request is refused', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ error: '`status` is not valid' }, 400)));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.textContent).toContain('Could not load failures: `status` is not valid'));
  });

  describe.each([
    ['with no state filter', '/failures', 4],
    ['with a state filter', '/failures?status=confirmed', 3],
  ])('a failure pushed off the newest page by a newer one, %s', (_name, address, expectedRows) => {
    it('is kept only when no filter can have taken it out of the list', async () => {
      window.history.replaceState({}, '', address);
      vi.useFakeTimers({ shouldAdvanceTime: true });
      try {
        let head: FailureSummary[] = [row('f_5', 5), row('f_4', 4)];
        vi.stubGlobal(
          'fetch',
          vi.fn(async (url: string) => json(url.includes('cursor=') ? { failures: [row('f_1', 1)] } : { failures: head, next_cursor: '2' })),
        );
        const { container } = render(<FailuresPage />);
        await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(2));
        fireEvent.click(screen.getByRole('button', { name: /load more/i }));
        await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(3));
        head = [row('f_6', 6), row('f_4', 4)];
        await vi.advanceTimersByTimeAsync(5100);
        await waitFor(() => expect(container.querySelector('tbody')?.textContent).toContain('f_6'));
        expect(container.querySelectorAll('tbody tr')).toHaveLength(expectedRows);
      } finally {
        vi.useRealTimers();
      }
    });
  });
});

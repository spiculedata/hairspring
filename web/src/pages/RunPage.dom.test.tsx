// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { page } from '../explorer/fixtures';
import { NO_FAILURES_YET } from '../wording';
import type { FailureSummary, Run, RunDetail } from '../types';
import { RunPage } from './RunPage';

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

const run: Run = {
  id: 'run_1',
  target: 'lock-lease',
  backend: 'sim',
  state: 'running',
  seed: 7,
  created_at: 1,
  started_at: null,
  finished_at: null,
  steps: 10,
  trials: 4,
  error: null,
};
const failure: FailureSummary = {
  id: 'fail_1',
  run_id: 'run_1',
  target: 'lock-lease',
  message: 'lease handed out twice',
  original_len: 23,
  shrunk_len: 8,
  bundle_hash: 'cd'.repeat(32),
  created_at: 2,
};

function serve(detail: RunDetail) {
  return vi.fn(async (url: string) => {
    const u = String(url);
    if (u.includes('/events')) return new Promise<Response>(() => undefined);
    if (u.includes('/properties')) return json(page());
    if (u.includes('/coverage')) return json({}, 404);
    if (u.startsWith('/api/runs/run_1')) return json(detail);
    return json({}, 404);
  });
}

const mount = async (search: string, detail: RunDetail = { run, failures: [failure] }) => {
  window.history.replaceState({}, '', `/runs/run_1${search}`);
  vi.stubGlobal('fetch', serve(detail));
  const view = render(<RunPage id="run_1" />);
  await waitFor(() => expect(screen.queryByText(/Loading run/)).toBeNull());
  return view;
};

const tab = (name: RegExp) => screen.getByRole('tab', { name });

describe('RunPage tabs', () => {
  beforeEach(() => vi.useRealTimers());
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.history.replaceState({}, '', '/');
  });

  it('offers the four sections in order and starts on Properties', async () => {
    await mount('');
    expect(screen.getAllByRole('tab').map((t) => t.textContent?.replace(/\s*[\d,]+$/, ''))).toEqual([
      'Properties',
      'Seeds and trials',
      'Failures',
      'Activity',
    ]);
    expect(tab(/^Properties/).getAttribute('aria-selected')).toBe('true');
    await waitFor(() => expect(screen.getByRole('tabpanel').querySelector('table')).not.toBeNull());
    expect(screen.queryByText('lease handed out twice')).toBeNull();
    expect(screen.queryByRole('link', { name: /Browse every judged seed/ })).toBeNull();
  });

  it('opens the tab the address names', async () => {
    await mount('?tab=failures');
    expect(tab(/^Failures/).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByText('lease handed out twice')).toBeTruthy();
  });

  it('shows the first tab for an unknown tab name', async () => {
    await mount('?tab=nonsense');
    expect(tab(/^Properties/).getAttribute('aria-selected')).toBe('true');
  });

  it('writes the chosen tab to the address, and the plain address for the first tab', async () => {
    await mount('');
    fireEvent.click(tab(/^Activity/));
    expect(window.location.search).toBe('?tab=activity');
    expect(tab(/^Activity/).getAttribute('aria-selected')).toBe('true');
    fireEvent.click(tab(/^Properties/));
    expect(window.location.search).toBe('');
  });

  it('follows the address when the reader goes back', async () => {
    await mount('');
    fireEvent.click(tab(/^Failures/));
    act(() => {
      window.history.replaceState({}, '', '/runs/run_1');
      window.dispatchEvent(new PopStateEvent('popstate'));
    });
    await waitFor(() => expect(tab(/^Properties/).getAttribute('aria-selected')).toBe('true'));
  });

  it('keeps the facts, the run panel and the actions above the tabs on every tab', async () => {
    await mount('?tab=activity');
    expect(screen.getByText('lock-lease')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Cancel run' })).toBeTruthy();
    expect(screen.getByText('Seed')).toBeTruthy();
  });

  it('shows the activity log only on the Activity tab', async () => {
    await mount('?tab=activity');
    expect(screen.getByRole('heading', { level: 2, name: 'Activity' })).toBeTruthy();
    cleanup();
    await mount('');
    expect(screen.queryByRole('heading', { level: 2, name: 'Activity' })).toBeNull();
  });

  it('counts the failures in the Failures tab', async () => {
    await mount('');
    expect(screen.getByRole('tab', { name: 'Failures 1' })).toBeTruthy();
  });

  it('says there are none yet on a live run with no failures, and lists the first failing seed link only with one', async () => {
    await mount('?tab=failures', { run, failures: [] });
    expect(screen.getByText(NO_FAILURES_YET)).toBeTruthy();
  });

  it('links to the seed explorer from Seeds and trials, which keeps its own address', async () => {
    await mount('?tab=seeds');
    const link = screen.getByRole('link', { name: /Browse every judged seed/ });
    expect(link.getAttribute('href')).toBe('/runs/run_1/seeds');
  });

  it('moves between tabs with the arrow keys', async () => {
    await mount('');
    fireEvent.keyDown(tab(/^Properties/), { key: 'End' });
    expect(window.location.search).toBe('?tab=activity');
    expect(document.activeElement).toBe(tab(/^Activity/));
  });
});

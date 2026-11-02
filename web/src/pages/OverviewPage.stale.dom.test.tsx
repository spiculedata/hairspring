// @vitest-environment happy-dom
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { FailureSummary, Run, Worker } from '../types';
import { OverviewPage } from './OverviewPage';

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const run = (id: string, state: string): Run => ({
  id,
  target: 't',
  backend: 'sim',
  state: state as Run['state'],
  seed: 1,
  created_at: 1,
  started_at: null,
  finished_at: null,
  steps: 1,
  trials: 4,
  error: null,
});
const failure = (id: string): FailureSummary => ({
  id,
  run_id: 'r',
  target: 't',
  message: 'm',
  original_len: 2,
  shrunk_len: 1,
  bundle_hash: 'cd'.repeat(32),
  created_at: 2,
});
const worker = (name: string): Worker => ({ name, last_seen: 3, registered_at: 1 });

type Key = 'runs' | 'failures' | 'workers';
interface Store {
  runs?: Run[];
  failures?: FailureSummary[];
  workers?: Worker[];
}

/** Serves the store, then answers 500 for the `broken` endpoints once the returned function is called. */
function flaky(store: Store, broken: Key[]) {
  let down = false;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      const u = String(url);
      const key = (['runs', 'failures', 'workers', 'targets'] as const).find((k) => u.startsWith(`/api/${k}`));
      if (key === undefined) return json({}, 404);
      if (down && (broken as string[]).includes(key)) return json({ error: 'down' }, 500);
      return json({ [key]: store[key as Key] ?? [] });
    }),
  );
  return async (container: HTMLElement, text: string) => {
    down = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_500);
    });
    await waitFor(() => expect(container.textContent).toContain(text));
  };
}

describe('OverviewPage after a poll fails', () => {
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('does not say the server has no runs when the last good load was empty and the next poll fails', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const breakIt = flaky({}, ['runs']);
    const { container } = render(<OverviewPage />);
    await waitFor(() => expect(container.textContent).toContain('This server has no stored runs'));
    await breakIt(container, 'Could not load runs');
    expect(container.textContent).not.toContain('This server has no stored runs');
    expect(container.querySelector('.guide')).toBeNull();
  });

  for (const broken of ['runs', 'failures'] as const) {
    it(`drops the "None of the latest" sentence once the ${broken} poll fails`, async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const breakIt = flaky({ runs: [run('run_ok', 'passed')] }, [broken]);
      const { container } = render(<OverviewPage />);
      await waitFor(() => expect(container.textContent).toContain('None of the latest 1 run failed or errored'));
      await breakIt(container, `Could not load ${broken}`);
      expect(container.textContent).not.toContain('None of the latest');
    });
  }

  it('says the figures on screen are from the last successful load, for runs, failures and workers', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const breakIt = flaky({ runs: [run('run_ok', 'passed')], failures: [failure('f_1')], workers: [worker('w1')] }, ['runs', 'failures', 'workers']);
    const { container } = render(<OverviewPage />);
    await waitFor(() => expect(container.textContent).toContain('1 worker registered'));
    await breakIt(container, 'Could not load workers');
    for (const what of ['runs', 'failures', 'workers']) expect(container.textContent).toContain(`Showing the last successful load of ${what}.`);
  });

  it('does not claim a last successful load when there never was one', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => (String(url).startsWith('/api/workers') ? json({ error: 'x' }, 500) : json({ runs: [], failures: [], targets: [] }))));
    const { container } = render(<OverviewPage />);
    await waitFor(() => expect(container.textContent).toContain('Could not load workers'));
    expect(container.textContent).not.toContain('last successful load');
  });
});

// @vitest-environment happy-dom
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FailureSummary, Run, Target, Worker } from '../types';
import { OverviewPage } from './OverviewPage';

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

const run = (id: string, state: string, created_at: number, extra: Partial<Run> = {}): Run => ({
  id,
  target: 'lock-lease',
  backend: 'sim',
  state: state as Run['state'],
  seed: 1,
  created_at,
  started_at: null,
  finished_at: null,
  steps: 10,
  trials: 4,
  error: null,
  ...extra,
});
const failure = (id: string, created_at: number): FailureSummary => ({
  id,
  run_id: 'run_a',
  target: 'lock-lease',
  message: 'invariant violated',
  original_len: 23,
  shrunk_len: 8,
  bundle_hash: 'cd'.repeat(32),
  created_at,
});
const target = (name: string): Target => ({ name, kind: 'sim', description: '' });
const worker = (name: string, last_seen: number): Worker => ({ name, last_seen, registered_at: 1 });

interface Store {
  runs?: Run[] | number;
  failures?: FailureSummary[] | number;
  targets?: Target[] | number;
  workers?: Worker[] | number;
}

/** Answers each endpoint with its list, or with the status code given instead of a list. */
function serve(store: Store) {
  const answer = (value: unknown[] | number | undefined, key: string) =>
    typeof value === 'number' ? json({ error: 'boom' }, value) : json({ [key]: value ?? [] });
  return vi.fn(async (url: string) => {
    const u = String(url);
    if (u.startsWith('/api/runs')) return answer(store.runs, 'runs');
    if (u.startsWith('/api/failures')) return answer(store.failures, 'failures');
    if (u.startsWith('/api/targets')) return answer(store.targets, 'targets');
    if (u.startsWith('/api/workers')) return answer(store.workers, 'workers');
    return json({}, 404);
  });
}

const mount = async (store: Store) => {
  vi.stubGlobal('fetch', serve(store));
  const view = render(<OverviewPage />);
  await waitFor(() => expect(view.container.textContent).not.toContain('Loading'));
  return view;
};

describe('OverviewPage', () => {
  beforeEach(() => vi.stubGlobal('fetch', vi.fn()));
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('has an Overview heading and says what is loading before any data arrives', () => {
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => undefined)));
    const { container } = render(<OverviewPage />);
    expect(container.querySelector('h1')?.textContent).toBe('Overview');
    expect(container.textContent).toContain('Loading');
    expect(container.querySelector('.guide')).toBeNull();
  });

  describe('an empty store', () => {
    it('says so plainly and shows the guide with the documented commands, and no run table', async () => {
      const { container } = await mount({});
      expect(container.textContent).toContain('No runs yet');
      expect(container.textContent).toContain('This server lists no targets.');
      const commands = [...container.querySelectorAll('.guide code')].map((c) => c.textContent);
      expect(commands).toEqual(
        expect.arrayContaining([
          'hairspring targets',
          'hairspring init',
          'hairspring test <target> --seed 1',
          'hairspring replay .hairspring/bundles/<bundle>.hairspring',
        ]),
      );
      expect(container.querySelector('table')).toBeNull();
      expect(container.querySelector('a[href="/runs"]')).not.toBeNull();
    });

    it('makes no health claim, and no pass-looking badge', async () => {
      const { container } = await mount({});
      expect(container.querySelector('.badge-good')).toBeNull();
      expect(container.textContent?.toLowerCase()).not.toMatch(/healthy|all clear|all good|passing/);
    });

    it('says workers are none when the server lists none', async () => {
      const { container } = await mount({});
      expect(container.textContent).toContain('No workers registered.');
    });
  });

  describe('targets but no runs', () => {
    it('lists the targets from the server and uses the first in the test command', async () => {
      const { container } = await mount({ targets: [target('lock-lease'), target('pubsub-buggy')] });
      expect(container.textContent).toContain('This server lists 2 targets: lock-lease, pubsub-buggy.');
      const commands = [...container.querySelectorAll('.guide code')].map((c) => c.textContent);
      expect(commands).toContain('hairspring test lock-lease --seed 1');
      expect(container.querySelector('table')).toBeNull();
    });

    it('does not put a name with shell characters into a command', async () => {
      const { container } = await mount({ targets: [target('a b; rm')] });
      const commands = [...container.querySelectorAll('.guide code')].map((c) => c.textContent);
      expect(commands).toContain('hairspring test <target> --seed 1');
      expect(commands.join(' ')).not.toContain('rm');
    });
  });

  describe('runs with failures', () => {
    const store: Store = {
      runs: [run('run_ok', 'passed', 1000), run('run_bad', 'failed', 3000), run('run_new', 'running', 4000, { trials: 0 })],
      failures: [failure('f_old', 10), failure('f_new', 20)],
      targets: [target('lock-lease')],
      workers: [worker('w1', 5000), worker('w2', 9000)],
    };

    it('shows failed runs and the newest failures under Needs attention, linked', async () => {
      const { container } = await mount(store);
      const attention = container.querySelector('section[aria-labelledby="overview-attention"]');
      expect(attention?.querySelector('h2')?.textContent).toBe('Needs attention');
      const hrefs = [...(attention?.querySelectorAll('a') ?? [])].map((a) => a.getAttribute('href'));
      expect(hrefs).toContain('/runs/run_bad');
      expect(hrefs).not.toContain('/runs/run_ok');
      expect(hrefs).toEqual(expect.arrayContaining(['/failures/f_new', '/failures/f_old']));
      expect(hrefs.indexOf('/failures/f_new')).toBeLessThan(hrefs.indexOf('/failures/f_old'));
      expect(attention?.textContent).toContain('Failed');
      expect(attention?.textContent).not.toContain('None of the latest');
    });

    it('lists recent runs with a status badge each, newest first, and links to all runs', async () => {
      const { container } = await mount(store);
      const recent = container.querySelector('section[aria-labelledby="overview-recent"]');
      const rows = [...(recent?.querySelectorAll('tbody tr') ?? [])];
      expect(rows.map((r) => r.querySelector('a')?.getAttribute('href'))).toEqual(['/runs/run_new', '/runs/run_bad', '/runs/run_ok']);
      for (const r of rows) expect(r.querySelector('.badge')).not.toBeNull();
      expect(recent?.querySelector('a[href="/runs"]')).not.toBeNull();
    });

    it('never shows a pass-looking badge for a run with no trials', async () => {
      const { container } = await mount({ runs: [run('run_zero', 'passed', 1, { trials: 0 })] });
      expect(container.querySelector('.badge-good')).toBeNull();
      expect(container.textContent).toContain('No trials run');
    });

    it('does not show the guide when runs exist', async () => {
      const { container } = await mount(store);
      expect(container.querySelector('.guide')).toBeNull();
    });

    it('counts workers from the list and gives the newest contact, with no claim about them working', async () => {
      const { container } = await mount(store);
      const workers = container.querySelector('section[aria-labelledby="overview-workers"]');
      expect(workers?.textContent).toContain('2 workers registered');
      expect(workers?.querySelector('time')?.getAttribute('datetime')).toBe(new Date(9000).toISOString());
      expect(workers?.textContent).toContain('does not say whether a worker is working');
      expect(workers?.querySelector('a[href="/workers"]')).not.toBeNull();
    });

    it('says what was looked at when nothing needs attention, without calling anything fine', async () => {
      const { container } = await mount({ runs: [run('run_ok', 'passed', 1), run('run_two', 'cancelled', 2)] });
      expect(container.textContent).toContain('None of the latest 2 runs failed or errored, and no failures are stored.');
      expect(container.textContent?.toLowerCase()).not.toMatch(/healthy|all clear|all good/);
    });
  });

  describe('errored runs', () => {
    it('shows an errored run under Needs attention with the server error as escaped text', async () => {
      const { container } = await mount({ runs: [run('run_e', 'error', 5, { error: '<b>host lost</b>‮' })] });
      const attention = container.querySelector('section[aria-labelledby="overview-attention"]');
      expect(attention?.querySelector('a[href="/runs/run_e"]')).not.toBeNull();
      expect(attention?.textContent).toContain('Error');
      expect(attention?.textContent).toContain('<b>host lost</b>');
      expect(attention?.querySelector('b')).toBeNull();
      expect(attention?.textContent).not.toContain('‮');
    });
  });

  describe('load errors, by cause', () => {
    it('names the runs when runs cannot load, and does not call the store empty or show the guide', async () => {
      const { container } = await mount({ runs: 500 });
      expect(container.textContent).toContain('Could not load runs');
      expect(container.querySelector('.guide')).toBeNull();
      expect(container.textContent).not.toContain('No runs yet');
      expect(container.textContent).not.toContain('None of the latest');
    });

    it('names the failures when only failures cannot load, and still shows the failed runs', async () => {
      const { container } = await mount({ runs: [run('run_bad', 'failed', 3)], failures: 500 });
      expect(container.textContent).toContain('Could not load failures');
      expect(container.querySelector('a[href="/runs/run_bad"]')).not.toBeNull();
      expect(container.textContent).not.toContain('None of the latest');
    });

    it('names the workers when only workers cannot load, and gives no count', async () => {
      const { container } = await mount({ runs: [run('run_ok', 'passed', 1)], workers: 500 });
      expect(container.textContent).toContain('Could not load workers');
      expect(container.textContent).not.toMatch(/\d+ workers? registered/);
      expect(container.textContent).not.toContain('No workers registered.');
    });

    it('shows the guide with a placeholder target, and a targets error, when only targets cannot load', async () => {
      const { container } = await mount({ targets: 500 });
      expect(container.textContent).toContain('Could not load targets');
      expect(container.textContent).not.toContain('This server lists no targets.');
      const commands = [...container.querySelectorAll('.guide code')].map((c) => c.textContent);
      expect(commands).toContain('hairspring test <target> --seed 1');
    });
  });

  describe('polling', () => {
    it('loads again on its interval, and a new run replaces the guide', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      let runs: Run[] = [];
      vi.stubGlobal(
        'fetch',
        vi.fn(async (url: string) => {
          const u = String(url);
          if (u.startsWith('/api/runs')) return json({ runs });
          if (u.startsWith('/api/failures')) return json({ failures: [] });
          if (u.startsWith('/api/targets')) return json({ targets: [] });
          return json({ workers: [] });
        }),
      );
      const { container } = render(<OverviewPage />);
      await waitFor(() => expect(container.querySelector('.guide')).not.toBeNull());
      runs = [run('run_x', 'running', 1)];
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5500);
      });
      await waitFor(() => expect(container.querySelector('.guide')).toBeNull());
      expect(container.querySelector('a[href="/runs/run_x"]')).not.toBeNull();
    });

    it('keeps the last good data on screen when a later poll fails, and says it could not load', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      let fail = false;
      vi.stubGlobal(
        'fetch',
        vi.fn(async (url: string) => {
          const u = String(url);
          if (u.startsWith('/api/runs')) return fail ? json({ error: 'down' }, 500) : json({ runs: [run('run_k', 'failed', 1)] });
          if (u.startsWith('/api/failures')) return json({ failures: [] });
          if (u.startsWith('/api/targets')) return json({ targets: [] });
          return json({ workers: [] });
        }),
      );
      const { container } = render(<OverviewPage />);
      await waitFor(() => expect(container.querySelector('a[href="/runs/run_k"]')).not.toBeNull());
      fail = true;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5500);
      });
      await waitFor(() => expect(container.textContent).toContain('Could not load runs'));
      expect(container.querySelector('a[href="/runs/run_k"]')).not.toBeNull();
    });
  });
});

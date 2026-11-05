// @vitest-environment happy-dom
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CheckView, FailureChecks } from '../types';
import { resetMeForTests } from '../meStore';
import { FailureConfirmation } from './FailureConfirmation';

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const body = (over: Partial<FailureChecks> = {}): FailureChecks => ({
  failure_id: 'fail_0123456789abcdef',
  finding: 'candidate',
  confirmation_id: null,
  environment_pinned: false,
  checks: [],
  ...over,
});

describe('FailureConfirmation', () => {
  beforeEach(() => {
    resetMeForTests();
    vi.stubGlobal('fetch', vi.fn());
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it('says what is loading, then asks for the checks of this failure only', async () => {
    const fetchMock = vi.fn(async (_url: string) => json(body()));
    vi.stubGlobal('fetch', fetchMock);
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    expect(container.textContent).toContain('Loading confirmation checks…');
    await waitFor(() => expect(container.textContent).toContain('Candidate'));
    expect(fetchMock.mock.calls[0][0]).toBe('/api/failures/fail_0123456789abcdef/checks');
  });

  it('shows a plain empty state, derived from the empty list, with the command to run, and no table', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(body())));
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.textContent).toContain('No checks recorded. This failure has not been confirmed.'));
    expect(container.querySelector('table')).toBeNull();
    expect(container.textContent).toContain('hairspring confirm fail_0123456789abcdef');
  });

  it('shows the ladder and one row per check', async () => {
    const c = {
      id: 'c1', confirmation_id: 'conf_a', ordinal: 1, kind: 'same_seed_rerun', status: 'pass',
      counts: { passed: 3, failed: 0, inconclusive: 0 }, artefacts: [], environment: {}, started_at: 1, finished_at: 2,
      runner_trusted: true, created_at: 2, guest: { command: 'hermit run', detail: {} },
    } as unknown as CheckView;
    vi.stubGlobal('fetch', vi.fn(async () => json(body({ finding: 'rerun_repeated', confirmation_id: 'conf_a', checks: [c, { ...c, id: 'c2', kind: 'bundle_replay', status: 'inconclusive' }] }))));
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(2));
    expect(container.querySelector('[aria-current="step"]')?.textContent).toContain('Rerun repeated');
    expect(container.textContent).toContain('3 passed, 0 failed, 0 inconclusive');
  });

  it('passes the confirmation the server names through, so the table marks which checks count', async () => {
    const mk = (id: string, confirmation_id: string) =>
      ({
        id, confirmation_id, ordinal: 1, kind: 'same_seed_rerun', status: 'pass', counts: { passed: 3, failed: 0, inconclusive: 0 },
        artefacts: [], environment: {}, started_at: 1, finished_at: 2, runner_trusted: true, created_at: 2, guest: { command: 'x', detail: {} },
      }) as unknown as CheckView;
    const checks = [mk('a', 'conf_old'), mk('b', 'conf_new'), mk('c', 'conf_old')];
    vi.stubGlobal('fetch', vi.fn(async () => json(body({ finding: 'confirmed', confirmation_id: 'conf_new', checks }))));
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(3));
    const heads = [...container.querySelectorAll('th')].map((h) => h.textContent);
    const col = heads.indexOf('Counts towards the finding');
    const cells = [...container.querySelectorAll('tbody tr')].map((r) => r.querySelectorAll('td')[col].textContent);
    expect(cells).toEqual(['No', 'Yes', 'No']);
  });

  it('shows a placeholder, not the text, when the failure id is not in the server format', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(body())));
    const { container } = render(<FailureConfirmation failureId="<b>x</b>" />);
    await waitFor(() => expect(container.textContent).toContain('No checks recorded.'));
    expect(container.textContent).toContain('hairspring confirm <failure id>');
    expect(container.textContent).toContain('Linux on x86-64');
    expect(container.querySelector('b')).toBeNull();
  });

  it('says it could not load, with the reason, and does not claim there are no checks', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ error: 'database is locked' }, 503)));
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.querySelector('[role="alert"]')).not.toBeNull());
    expect(container.textContent).toContain('Could not load the confirmation checks: database is locked');
    expect(container.textContent).not.toContain('No checks recorded');
  });

  it('tells a missing failure apart from a failing server and does not promise a retry', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ error: 'not found' }, 404)));
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.querySelector('[role="alert"]')).not.toBeNull());
    expect(container.textContent).toContain('Reload the page to try again.');
    expect(container.textContent).not.toContain('tries again by itself');
  });

  it('escapes a hostile finding word from the server', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json(body({ finding: '<img src=x onerror=alert(1)>' as FailureChecks['finding'] }))));
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.textContent).toContain('no wording for'));
    expect(container.querySelector('img')).toBeNull();
  });

  const me = (confirm: boolean) => ({
    role: 'run',
    open_mode: false,
    can: { read: true, run: true, admin: false, confirm },
    ...(confirm ? { confirm: { wall_secs: 120, max_running: 2 } } : {}),
    roles: ['read', 'run', 'admin'],
  });
  const serve = (canConfirm: boolean) =>
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        if (url === '/api/me') return json(me(canConfirm));
        if (url.endsWith('/confirm')) return json({ enabled: canConfirm, state: 'idle' });
        return json(body());
      }),
    );

  it('adds the run control, and keeps the command to run, when the server says this caller may confirm', async () => {
    serve(true);
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.textContent).toContain('Run confirmation'));
    expect(container.textContent).toContain('hairspring confirm fail_0123456789abcdef');
  });

  it('shows nothing new when the server says this caller may not', async () => {
    serve(false);
    const { container } = render(<FailureConfirmation failureId="fail_0123456789abcdef" />);
    await waitFor(() => expect(container.textContent).toContain('No checks recorded.'));
    await waitFor(() => expect(vi.mocked(fetch).mock.calls.some(([u]) => u === '/api/me')).toBe(true));
    expect(container.textContent).not.toContain('Run confirmation');
  });
});

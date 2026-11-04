// @vitest-environment happy-dom
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { FailureSummary } from '../types';
import { detail } from './fixtures';
import { TrialActions } from './TrialActions';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const failure = (id: string): FailureSummary => ({ id, run_id: 'run_0123456789abcdef', target: 't', message: 'm', original_len: 3, shrunk_len: 1, bundle_hash: 'cd'.repeat(32), created_at: 1 });

function stubRun(failures: FailureSummary[], status = 200) {
  vi.stubGlobal('fetch', vi.fn(async () => (status === 200 ? json({ run: { id: 'run_0123456789abcdef' }, failures }) : json({ error: 'nope' }, status))));
}

const mount = () => render(<TrialActions runId="run_0123456789abcdef" trial={detail().trial} context={undefined} />);

describe('TrialActions', () => {
  it('offers copy link and copy summary at once', () => {
    stubRun([]);
    const { container } = mount();
    const labels = [...container.querySelectorAll('button')].map((b) => b.textContent);
    expect(labels).toEqual(['Copy link', 'Copy summary']);
  });

  it('links to the run failure page when the run has a bundle failure', async () => {
    stubRun([failure('f_abc')]);
    const { container } = mount();
    await waitFor(() => expect(container.querySelector('a[href="/failures/f_abc"]')).not.toBeNull());
    expect(container.querySelector('a[href="/failures/f_abc"]')?.textContent).toBe('Shortest failing steps for this run');
  });

  it('shows no link when the run has no bundle failure, and none when the lookup fails', async () => {
    stubRun([]);
    const first = mount();
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(first.container.querySelector('a')).toBeNull();
    cleanup();
    stubRun([], 500);
    const second = mount();
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(second.container.querySelector('a')).toBeNull();
    expect(second.container.querySelector('[role="alert"]')).toBeNull();
  });

  it('does not link to a failure id that is not a plain id', async () => {
    stubRun([failure('../x')]);
    const { container } = mount();
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(container.querySelector('a')).toBeNull();
  });
});

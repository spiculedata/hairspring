// @vitest-environment happy-dom
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { detail } from '../trial/fixtures';
import type { FailureSummary } from '../types';
import { FailureActions } from './FailureActions';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const record: FailureSummary = { id: 'f_abc', run_id: 'run_0123456789abcdef', target: 'seats', message: 'm', original_len: 3, shrunk_len: 1, bundle_hash: 'cd'.repeat(32), created_at: 1 };

function stubTrials(trials: unknown[], status = 200) {
  vi.stubGlobal('fetch', vi.fn(async () => (status === 200 ? json({ run_id: record.run_id, trials, next_cursor: null }) : json({ error: 'x' }, status))));
}

const mount = () => render(<FailureActions record={record} />);

describe('FailureActions', () => {
  it('asks for the first failing trial of the run, by verdict, and nothing else', async () => {
    stubTrials([]);
    mount();
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(vi.mocked(fetch).mock.calls[0][0]).toBe('/api/runs/run_0123456789abcdef/trials?verdict=violation&limit=1');
  });

  it('links to the trial page for that trial', async () => {
    stubTrials([detail().trial]);
    const { container } = mount();
    await waitFor(() => expect(container.querySelector('a[href="/runs/run_0123456789abcdef/trials/16"]')).not.toBeNull());
    expect(container.querySelector('a')?.textContent).toBe('Open trial 16');
  });

  it('shows no link when the run kept no failing trial, and no error when the lookup fails', async () => {
    stubTrials([]);
    const first = mount();
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(first.container.querySelector('a')).toBeNull();
    cleanup();
    stubTrials([], 503);
    const second = mount();
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(second.container.querySelector('a')).toBeNull();
    expect(second.container.querySelector('[role="alert"]')).toBeNull();
  });

  it('offers copy link and copy summary', () => {
    stubTrials([]);
    const { container } = mount();
    expect([...container.querySelectorAll('button')].map((b) => b.textContent)).toEqual(['Copy link', 'Copy summary']);
  });
});

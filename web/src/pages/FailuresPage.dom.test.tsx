// @vitest-environment happy-dom
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FailureSummary } from '../types';
import { FailuresPage } from './FailuresPage';

const row = (id: string, created_at: number): FailureSummary => ({
  id,
  run_id: 'run_1',
  target: 't',
  message: 'm',
  original_len: 3,
  shrunk_len: 1,
  bundle_hash: 'cd'.repeat(32),
  created_at,
});
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

describe('FailuresPage', () => {
  beforeEach(() => vi.stubGlobal('fetch', vi.fn()));
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
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

  it('says the list is cut off when the server returned as many as were asked for', async () => {
    const many = Array.from({ length: 50 }, (_, i) => row(`f_${i}`, i));
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: many })));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(50));
    expect(container.textContent).toContain('Showing the newest 50 failures. There may be older ones.');
  });

  it('does not say the list is cut off when it is shorter than the limit', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ failures: [row('f_a', 1)] })));
    const { container } = render(<FailuresPage />);
    await waitFor(() => expect(container.querySelectorAll('tbody tr')).toHaveLength(1));
    expect(container.textContent).not.toContain('There may be older ones');
  });
});

// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { FailureDetail } from '../types';
import { FailurePage } from './FailurePage';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const failure: FailureDetail = {
  record: { id: 'f_abc', run_id: 'run_1', target: 'seats', message: 'm', original_len: 3, shrunk_len: 1, bundle_hash: 'cd'.repeat(32), created_at: 1 },
  steps: [{ index: 0, action_id: 'a', label: 'the first step', state: 's' }],
  bundle: 'the bundle text',
};

describe('FailurePage replay that ran nothing', () => {
  it('says nothing was run, does not say the failure did not repeat, and keeps the recorded steps', async () => {
    vi.spyOn(api, 'failure').mockResolvedValue(failure);
    vi.spyOn(api, 'replay').mockResolvedValue({
      reproduced: false,
      steps: [],
      ran: false,
      environment_changed: true,
      warnings: ['the environment changed'],
    });
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })));
    render(<FailurePage id="f_abc" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Replay' }));
    const line = await screen.findByText(/Nothing was run: the bundle was recorded on another backend/);
    expect(line.textContent).toContain('does not show the bug is fixed');
    expect(screen.queryByText(/Did not fail on replay/)).toBeNull();
    expect(screen.getByText('the first step')).toBeTruthy();
  });

  it('still shows the replayed steps when the replay ran', async () => {
    vi.spyOn(api, 'failure').mockResolvedValue(failure);
    vi.spyOn(api, 'replay').mockResolvedValue({
      reproduced: true,
      steps: [{ index: 0, action_id: 'b', label: 'the replayed step', state: 's' }],
    });
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })));
    render(<FailurePage id="f_abc" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Replay' }));
    expect(await screen.findByText('the replayed step')).toBeTruthy();
    expect(screen.queryByText(/Nothing was run/)).toBeNull();
  });
});

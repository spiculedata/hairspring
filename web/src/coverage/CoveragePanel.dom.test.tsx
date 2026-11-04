// @vitest-environment happy-dom
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError, setToken } from '../api';
import type { Coverage } from '../types';
import { CoveragePanel } from './CoveragePanel';

const series = (n: number): Coverage => ({
  run_id: 'run_1',
  trials: n,
  downsampled: false,
  points: Array.from({ length: n }, (_, i) => ({ seq: i + 1, ids_hit: i, ids_true: 0, schedules: i })),
});

describe('CoveragePanel', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    setToken(null);
  });

  it('shows the heading at once and the chart when the series arrives', async () => {
    vi.spyOn(api, 'coverage').mockResolvedValue(series(10));
    render(<CoveragePanel runId="run_1" live={false} />);
    expect(screen.getByRole('heading', { name: 'Coverage over trials' })).toBeTruthy();
    await waitFor(() => expect(screen.getAllByRole('img')).toHaveLength(3));
  });

  it('says there is nothing to draw yet when no trial has been judged', async () => {
    vi.spyOn(api, 'coverage').mockResolvedValue(series(0));
    render(<CoveragePanel runId="run_1" live={false} />);
    await waitFor(() => expect(screen.getByText(/no trial has been judged yet/i)).toBeTruthy());
    expect(screen.queryAllByRole('img')).toHaveLength(0);
  });

  it('says what failed to load, in an alert', async () => {
    vi.spyOn(api, 'coverage').mockRejectedValue(new ApiError(500, 'store unavailable'));
    render(<CoveragePanel runId="run_1" live={false} />);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Could not load coverage: store unavailable'));
  });

  it('loads once for a finished run and loads again when a live run ends', async () => {
    const spy = vi.spyOn(api, 'coverage').mockResolvedValue(series(3));
    const { rerender } = render(<CoveragePanel runId="run_1" live={true} />);
    await waitFor(() => expect(screen.getAllByRole('img')).toHaveLength(3));
    const before = spy.mock.calls.length;
    rerender(<CoveragePanel runId="run_1" live={false} />);
    await waitFor(() => expect(spy.mock.calls.length).toBeGreaterThan(before));
  });
});

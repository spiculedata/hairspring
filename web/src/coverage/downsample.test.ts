import { describe, expect, it } from 'vitest';
import { downsamplePoints, MAX_CHART_POINTS } from './downsample';
import type { CoveragePoint } from '../types';

const series = (n: number): CoveragePoint[] =>
  Array.from({ length: n }, (_, i) => ({ seq: i * 3, ids_hit: i, ids_true: i, schedules: i }));

describe('downsamplePoints', () => {
  it('returns the points unchanged, not flagged, when they fit', () => {
    const points = series(MAX_CHART_POINTS);
    const out = downsamplePoints(points);
    expect(out.points).toEqual(points);
    expect(out.reduced).toBe(false);
  });

  it('keeps at most the cap, always the first and last point, in order', () => {
    const points = series(10_000);
    const out = downsamplePoints(points);
    expect(out.reduced).toBe(true);
    expect(out.points.length).toBeLessThanOrEqual(MAX_CHART_POINTS);
    expect(out.points[0]).toEqual(points[0]);
    expect(out.points[out.points.length - 1]).toEqual(points[9_999]);
    const seqs = out.points.map((p) => p.seq);
    expect([...seqs].sort((a, b) => a - b)).toEqual(seqs);
    expect(new Set(seqs).size).toBe(seqs.length);
  });

  it('honours a smaller cap and does not change its input', () => {
    const points = series(100);
    const copy = points.map((p) => ({ ...p }));
    expect(downsamplePoints(points, 10).points).toHaveLength(10);
    expect(points).toEqual(copy);
  });

  it('handles empty and single-point series', () => {
    expect(downsamplePoints([])).toEqual({ points: [], reduced: false });
    expect(downsamplePoints(series(1)).points).toHaveLength(1);
    expect(downsamplePoints(series(5), 1).points).toHaveLength(1);
  });
});

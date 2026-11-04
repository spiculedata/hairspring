import type { CoveragePoint } from '../types';

/** The most points the chart draws. Beyond this a line is no more readable and the page only gets slower. */
export const MAX_CHART_POINTS = 2000;

export interface Downsampled {
  points: readonly CoveragePoint[];
  /** True when points were left out. */
  reduced: boolean;
}

/**
 * At most `cap` points, evenly spaced through the series and always including the first and the last. The counts only
 * ever grow, so the points kept still lie on the true curve; what is lost is the detail between them. The input is not
 * changed.
 */
export function downsamplePoints(points: readonly CoveragePoint[], cap: number = MAX_CHART_POINTS): Downsampled {
  if (points.length <= cap) return { points, reduced: false };
  if (cap < 2) return { points: points.slice(0, Math.max(cap, 0)), reduced: true };
  const last = points.length - 1;
  const kept = Array.from({ length: cap }, (_, i) => points[Math.round((i * last) / (cap - 1))] as CoveragePoint);
  return { points: kept, reduced: true };
}

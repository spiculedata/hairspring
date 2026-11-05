import type { CoveragePoint } from '../types';

export type PanelKey = 'ids_hit' | 'ids_true' | 'schedules';

/** The series, in the order they are drawn from top to bottom. */
export const PANEL_KEYS: readonly PanelKey[] = ['ids_hit', 'ids_true', 'schedules'];

export interface PanelSize {
  width: number;
  height: number;
}

export interface Panel {
  key: PanelKey;
  /** Largest value of the series. */
  max: number;
  /** The value at the top edge of the plot and on the y-axis label: the last round tick at or above `max`. Every scale starts at zero. */
  top: number;
  /** One x and one y per point, in drawing units. y grows downwards, so the largest value is 0. */
  xs: readonly number[];
  ys: readonly number[];
}

/** The x position of every point on one shared trial axis, from 0 to `width`. A lone point sits at the left. */
function trialAxis(points: readonly CoveragePoint[], width: number): number[] {
  const first = points[0]?.seq ?? 0;
  const span = (points[points.length - 1]?.seq ?? 0) - first;
  return points.map((p) => (span > 0 ? ((p.seq - first) / span) * width : 0));
}

/** One panel per series. Each has its own scale (the units differ), but all share the trial axis. Empty input gives none. */
export function buildPanels(points: readonly CoveragePoint[], size: PanelSize): Panel[] {
  if (points.length === 0) return [];
  const xs = trialAxis(points, size.width);
  return PANEL_KEYS.map((key) => {
    const max = points.reduce((m, p) => Math.max(m, p[key]), 0);
    const top = niceTicks(max).at(-1) ?? 0;
    const ys = points.map((p) => (top > 0 ? size.height * (1 - p[key] / top) : size.height));
    return { key, max, top, xs, ys };
  });
}

/** Round tick values from zero up to at least `max`: two or three of them, so the scale is readable and uncluttered. */
export function niceTicks(max: number): number[] {
  if (max <= 0) return [0];
  const magnitude = 10 ** Math.floor(Math.log10(max));
  const step = [1, 2, 5, 10].map((m) => m * magnitude).find((s) => max / s <= 2) ?? 10 * magnitude;
  const top = Math.ceil(max / step) * step;
  return Array.from({ length: Math.round(top / step) + 1 }, (_, i) => i * step);
}

const COUNT_FORMAT = new Intl.NumberFormat('en-GB');
export const formatCount = (n: number): string => COUNT_FORMAT.format(n);

/** Where a series ended and the trial at which it last rose. States what was seen; says nothing about what is left. */
export function panelSummary(key: PanelKey, points: readonly CoveragePoint[]): string {
  const final = points[points.length - 1]?.[key] ?? 0;
  if (final === 0) return 'none seen in any trial';
  const lastRise = points.find((p) => p[key] === final);
  return `${formatCount(final)} by trial ${formatCount(points[points.length - 1]?.seq ?? 0)}, last rising at trial ${formatCount(lastRise?.seq ?? 0)}`;
}

/** True when every series stayed at zero to the end. Counts never fall, so the last point says it. */
export function isAllZero(points: readonly CoveragePoint[]): boolean {
  const last = points[points.length - 1];
  return last !== undefined && PANEL_KEYS.every((key) => last[key] === 0);
}

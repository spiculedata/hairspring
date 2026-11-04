import { describe, expect, it } from 'vitest';
import { buildPanels, niceTicks, panelSummary, PANEL_KEYS } from './coverageModel';
import type { Coverage } from '../types';

const coverage = (over: Partial<Coverage> = {}): Coverage => ({
  run_id: 'run_1',
  trials: 100,
  downsampled: false,
  points: [
    { seq: 1, ids_hit: 1, ids_true: 0, schedules: 1 },
    { seq: 10, ids_hit: 4, ids_true: 2, schedules: 7 },
    { seq: 100, ids_hit: 5, ids_true: 3, schedules: 40 },
  ],
  ...over,
});

describe('buildPanels', () => {
  it('makes one panel per series, each on its own scale starting at zero, sharing the trial axis', () => {
    const panels = buildPanels(coverage().points, { width: 600, height: 100 });
    expect(panels.map((p) => p.key)).toEqual([...PANEL_KEYS]);
    const [hit, , schedules] = panels;
    expect(hit?.max).toBe(5);
    expect(schedules?.max).toBe(40);
    expect(hit?.xs).toEqual(schedules?.xs);
    expect(hit?.xs[0]).toBe(0);
    expect(hit?.xs[2]).toBe(600);
  });

  it('puts the largest value at the top and a fifth of it at four fifths down', () => {
    const [hit] = buildPanels(coverage().points, { width: 600, height: 100 });
    expect(hit?.ys[2]).toBe(0);
    expect(hit?.ys[0]).toBeCloseTo(80);
  });

  it('draws a single point at the left and an empty series as nothing', () => {
    const one = buildPanels(coverage().points.slice(0, 1), { width: 600, height: 100 });
    expect(one[0]?.xs).toEqual([0]);
    expect(buildPanels([], { width: 600, height: 100 })).toEqual([]);
  });

  it('does not divide by zero when a series never leaves zero', () => {
    const flat = buildPanels(
      [
        { seq: 1, ids_hit: 0, ids_true: 0, schedules: 0 },
        { seq: 9, ids_hit: 0, ids_true: 0, schedules: 0 },
      ],
      { width: 100, height: 50 },
    );
    for (const p of flat) expect(p.ys.every(Number.isFinite)).toBe(true);
  });
});

describe('buildPanels axis', () => {
  it('scales the line to the y-axis top label, so a maximum below a round tick sits below the top edge', () => {
    const pts = [
      { seq: 1, ids_hit: 0, ids_true: 0, schedules: 1 },
      { seq: 2, ids_hit: 7, ids_true: 0, schedules: 1234 },
    ];
    const [hit, , schedules] = buildPanels(pts, { width: 100, height: 100 });
    expect(hit?.top).toBe(10);
    expect(hit?.ys[1]).toBeCloseTo(30);
    expect(schedules?.top).toBe(2000);
    expect(schedules?.ys[1]).toBeCloseTo(100 * (1 - 1234 / 2000));
  });
});

describe('niceTicks', () => {
  it('gives round ticks from zero up to at least the maximum', () => {
    expect(niceTicks(5)).toEqual([0, 5]);
    expect(niceTicks(40)).toEqual([0, 20, 40]);
    expect(niceTicks(0)).toEqual([0]);
    expect(niceTicks(1234).at(-1)).toBeGreaterThanOrEqual(1234);
  });
});

describe('panelSummary', () => {
  it('says where the series ended and when it last rose, with no claim about what is left', () => {
    const text = panelSummary('ids_hit', coverage().points);
    expect(text).toContain('5');
    expect(text).toContain('trial 100');
    expect(text).not.toMatch(/reproduced|found|complete/i);
  });
  it('says so when a series never rose above zero', () => {
    const text = panelSummary('ids_true', [{ seq: 1, ids_hit: 0, ids_true: 0, schedules: 0 }]);
    expect(text).toMatch(/none/i);
  });
});

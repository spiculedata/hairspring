import { describe, expect, it } from 'vitest';
import { binStrip } from './strip';
import { BAR_HEIGHTS, layoutBars, STRIP_HEIGHT } from './stripLayout';

describe('BAR_HEIGHTS', () => {
  it('gives a failure the tallest mark and a pass the shortest, so class does not rest on colour', () => {
    expect(BAR_HEIGHTS.v).toBe(1);
    expect(BAR_HEIGHTS.p).toBeLessThan(BAR_HEIGHTS.e);
    expect(BAR_HEIGHTS.e).toBeLessThan(BAR_HEIGHTS.i);
    expect(BAR_HEIGHTS.i).toBeLessThan(BAR_HEIGHTS.v);
    for (const h of Object.values(BAR_HEIGHTS)) expect(h).toBeGreaterThan(0);
  });

  it('never draws a pass as tall as anything that is not one', () => {
    for (const code of ['v', 'x', 'u', 'i', 'e', '-'] as const) expect(BAR_HEIGHTS[code]).toBeGreaterThan(BAR_HEIGHTS.p);
  });
});

describe('layoutBars', () => {
  it('draws one bar per bin, left to right, inside the width and without overlap', () => {
    const bins = binStrip('ppvpepp'.repeat(100), 300);
    const bars = layoutBars(bins, 300, STRIP_HEIGHT);
    expect(bars).toHaveLength(300);
    let edge = 0;
    for (const b of bars) {
      expect(b.x).toBeGreaterThanOrEqual(edge);
      expect(b.w).toBeGreaterThanOrEqual(1);
      edge = b.x + b.w;
    }
    expect(edge).toBeLessThanOrEqual(300);
  });

  it('gives each bar the height of its class, measured up from the baseline', () => {
    const [p, v] = layoutBars(binStrip('pv', 100), 100, STRIP_HEIGHT);
    expect(p && v && p.h < v.h).toBe(true);
    expect(v?.h).toBe(STRIP_HEIGHT);
  });

  it('is empty when there is nothing to draw or no room', () => {
    expect(layoutBars([], 100, STRIP_HEIGHT)).toEqual([]);
    expect(layoutBars(binStrip('pp', 10), 0, STRIP_HEIGHT)).toEqual([]);
  });

  it('keeps every bar one or more pixels wide when there are as many bins as pixels', () => {
    const bars = layoutBars(binStrip('p'.repeat(100_000), 937), 937, STRIP_HEIGHT);
    expect(bars).toHaveLength(937);
    for (const b of bars) expect(b.w).toBeGreaterThanOrEqual(1);
  });
});

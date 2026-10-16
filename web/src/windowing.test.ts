import { describe, expect, it } from 'vitest';
import { computeWindow } from './windowing';

const base = { rowHeight: 32, viewportHeight: 320, overscan: 4 };

describe('computeWindow', () => {
  it('shows the first rows plus overscan at the top', () => {
    expect(computeWindow({ ...base, total: 10_000, scrollTop: 0 })).toEqual({
      start: 0,
      end: 14,
      padTop: 0,
      padBottom: (10_000 - 14) * 32,
    });
  });

  it('keeps the whole scroll height: padding plus rendered rows equals total rows', () => {
    const w = computeWindow({ ...base, total: 10_000, scrollTop: 3200 });
    expect(w.start).toBe(96);
    expect(w.end).toBe(114);
    expect(w.padTop + (w.end - w.start) * 32 + w.padBottom).toBe(10_000 * 32);
  });

  it('clamps at the end of the list', () => {
    const w = computeWindow({ ...base, total: 50, scrollTop: 999_999 });
    expect(w.end).toBe(50);
    expect(w.start).toBeLessThan(50);
    expect(w.padBottom).toBe(0);
  });

  it('handles an empty list', () => {
    expect(computeWindow({ ...base, total: 0, scrollTop: 0 })).toEqual({ start: 0, end: 0, padTop: 0, padBottom: 0 });
  });

  it('treats negative or NaN scroll as the top', () => {
    expect(computeWindow({ ...base, total: 100, scrollTop: -50 }).start).toBe(0);
    expect(computeWindow({ ...base, total: 100, scrollTop: Number.NaN }).start).toBe(0);
  });

  it('renders every row of a short list', () => {
    const w = computeWindow({ ...base, total: 5, scrollTop: 0 });
    expect([w.start, w.end]).toEqual([0, 5]);
  });

  it('renders a bounded number of rows for a huge list', () => {
    const w = computeWindow({ ...base, total: 1_000_000, scrollTop: 16_000_000 });
    expect(w.end - w.start).toBeLessThanOrEqual(20);
  });
});

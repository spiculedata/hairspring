// @vitest-environment happy-dom
import { act, cleanup, render, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { PropertyTable } from './PropertyTable';
import { row } from './fixtures';
import { clipHeightPx } from './PropertyRowView';
import { rowHeightPx } from './rowHeight';
import { useRootFontPx } from './useRootFontPx';

describe('useRootFontPx in a DOM', () => {
  afterEach(() => {
    cleanup();
    document.documentElement.style.fontSize = '';
  });

  it('reads the root font size, and reads it again when the window is resized', () => {
    document.documentElement.style.fontSize = '20px';
    const { result } = renderHook(() => useRootFontPx());
    expect(result.current).toBe(20);
    document.documentElement.style.fontSize = '24px';
    act(() => {
      window.dispatchEvent(new Event('resize'));
    });
    expect(result.current).toBe(24);
  });

  it('makes the property table rows as tall as that size needs', () => {
    document.documentElement.style.fontSize = '20px';
    const worst = row({ status: 'unmet', failures_seen_in_inconclusive_trials: 3, unmet: 'x', rate: { kind: 'zero_events', basis: 'TrialsTrue', n: 4790, upper_bound: 0.0006 } });
    const { container } = render(<PropertyTable properties={[worst]} />);
    const tr = container.querySelector('tbody tr[aria-rowindex]') as HTMLElement;
    expect(tr.style.height).toBe(`${rowHeightPx(20)}px`);
    const clip = container.querySelector('.cell-clip') as HTMLElement;
    expect(clip.style.height).toBe(`${clipHeightPx(rowHeightPx(20))}px`);
  });
});

import type { StripBin, StripCode } from './strip';

/** Height of the strip in CSS pixels. */
export const STRIP_HEIGHT = 56;

/**
 * How tall each class is drawn, as a fraction of the strip. Height carries the class as well as colour: a failure is the
 * full height, a pass the shortest mark, and the classes in between are in order of how much they need a reader's attention.
 */
export const BAR_HEIGHTS: Readonly<Record<StripCode, number>> = { v: 1, x: 0.8, u: 0.7, i: 0.6, e: 0.45, '-': 0.35, p: 0.2 };

export interface Bar {
  readonly x: number;
  readonly w: number;
  readonly h: number;
  readonly code: StripCode;
}

/** The marks to draw: one per bin, left to right, at least one pixel wide, standing on the strip's baseline. */
export function layoutBars(bins: readonly StripBin[], width: number, height: number): Bar[] {
  if (bins.length === 0 || !(width >= 1)) return [];
  return bins.map((bin, i) => {
    const x0 = Math.floor((i * width) / bins.length);
    const x1 = Math.floor(((i + 1) * width) / bins.length);
    return { x: x0, w: Math.max(1, x1 - x0), h: Math.max(1, Math.round(height * BAR_HEIGHTS[bin.worst])), code: bin.worst };
  });
}

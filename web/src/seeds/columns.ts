import { CONTENT_WIDTH_PX } from '../explorer/columns';

/**
 * Column widths of the trial table, in one place so the stylesheet, the markup and a test agree. One column has no
 * fixed width and takes whatever the others leave; the budget counts it at its minimum, so the table fits inside the
 * scroll box (the page content width less its border and, in a window just over 1000px wide, a classic scrollbar) and
 * scrolls sideways only in a narrower window.
 */
export const COLUMN_KEYS = ['seq', 'seed', 'verdict', 'failing', 'fp', 'ops', 'ms', 'kept'] as const;
export type ColumnKey = (typeof COLUMN_KEYS)[number];

/** The failing ids are clipped with an ellipsis, so this column can give way. */
export const FLEXIBLE_COLUMN: ColumnKey = 'failing';
export const FLEXIBLE_MIN_PX = 200;

/** Fixed widths; the flexible column has none. */
export const COLUMN_WIDTHS_PX: Readonly<Partial<Record<ColumnKey, number>>> = {
  seq: 72,
  seed: 168,
  verdict: 176,
  fp: 112,
  ops: 64,
  ms: 64,
  kept: 88,
};

/** The width at which nothing is squeezed: the fixed columns plus the flexible one at its minimum. */
export const columnBudgetPx = (): number => COLUMN_KEYS.reduce((sum, key) => sum + (COLUMN_WIDTHS_PX[key] ?? 0), 0) + FLEXIBLE_MIN_PX;

export { CONTENT_WIDTH_PX };

import { CONTENT_WIDTH_PX } from '../explorer/columns';

/**
 * Column widths of the trial table, in one place so the stylesheet, the markup and a test agree. They add up to the room
 * inside the scroll box (the 968px page content width less its 2px border) so that the table does not scroll at desktop
 * width; below it the table scrolls sideways.
 */
export const COLUMN_KEYS = ['seq', 'seed', 'verdict', 'failing', 'fp', 'ops', 'ms', 'kept'] as const;
export type ColumnKey = (typeof COLUMN_KEYS)[number];

export const COLUMN_WIDTHS_PX: Readonly<Record<ColumnKey, number>> = {
  seq: 72,
  seed: 168,
  verdict: 176,
  failing: 222,
  fp: 112,
  ops: 64,
  ms: 64,
  kept: 88,
};

export const columnBudgetPx = (): number => COLUMN_KEYS.reduce((sum, key) => sum + COLUMN_WIDTHS_PX[key], 0);

export { CONTENT_WIDTH_PX };

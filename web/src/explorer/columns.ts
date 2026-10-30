/**
 * Column widths of the property table, in one place so the stylesheet, the markup and a test agree.
 *
 * `main` in styles.css is at most 1000px wide with 16px gutters, so the scroll box gets 968px, 966px inside its border,
 * and a classic page scrollbar (about 15px) can narrow it further in a window just over 1000px wide. So one column has
 * no fixed width: it takes whatever the others leave, and the budget counts it at its minimum. The budget then fits the
 * box even with a scrollbar, and the table scrolls sideways, with a visible hint, only in a window narrower than that.
 */
export const MAIN_MAX_WIDTH_PX = 1000;
export const MAIN_GUTTER_PX = 16;
export const CONTENT_WIDTH_PX = MAIN_MAX_WIDTH_PX - 2 * MAIN_GUTTER_PX;
/** The scroll box has a 1px border each side (explorer.css), so the table inside it has this much less than the content width. */
export const TABLE_BOX_BORDER_PX = 2;
/** A classic (non-overlay) page scrollbar on Windows and Linux; overlay scrollbars take nothing. */
export const SCROLLBAR_PX = 15;

export const COLUMN_KEYS = ['status', 'kind', 'property', 'rate', 'seed', 'evals'] as const;
export type ColumnKey = (typeof COLUMN_KEYS)[number];

/** The column with no fixed width: property ids and messages are clipped with an ellipsis, so it can give way. */
export const FLEXIBLE_COLUMN: ColumnKey = 'property';
export const FLEXIBLE_MIN_PX = 200;

/** Fixed widths; the flexible column has none. */
export const COLUMN_WIDTHS_PX: Readonly<Partial<Record<ColumnKey, number>>> = {
  status: 160,
  kind: 92,
  rate: 272,
  seed: 112,
  evals: 112,
};

/** The width at which nothing is squeezed: the fixed columns plus the flexible one at its minimum. */
export function columnBudgetPx(): number {
  const fixed = COLUMN_KEYS.reduce((sum, key) => sum + (COLUMN_WIDTHS_PX[key] ?? 0), 0);
  return fixed + FLEXIBLE_MIN_PX;
}

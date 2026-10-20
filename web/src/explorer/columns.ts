/**
 * Column widths of the property table, in one place so the stylesheet, the markup and a test agree.
 *
 * `main` in styles.css is at most 1000px wide with 16px gutters, so the scroll box gets 968px, 966px inside its border. The
 * columns must add up to no more than that, or at desktop width the table scrolls by a pixel or two. Below it the table keeps these widths and scrolls
 * sideways, with a visible hint.
 */
export const MAIN_MAX_WIDTH_PX = 1000;
export const MAIN_GUTTER_PX = 16;
export const CONTENT_WIDTH_PX = MAIN_MAX_WIDTH_PX - 2 * MAIN_GUTTER_PX;
/** The scroll box has a 1px border each side (explorer.css), so the table inside it has this much less than the content width. */
export const TABLE_BOX_BORDER_PX = 2;

export const COLUMN_KEYS = ['status', 'kind', 'property', 'rate', 'seed', 'evals'] as const;
export type ColumnKey = (typeof COLUMN_KEYS)[number];

export const COLUMN_WIDTHS_PX: Readonly<Record<ColumnKey, number>> = {
  status: 128,
  kind: 92,
  property: 232,
  rate: 300,
  seed: 112,
  evals: 80,
};

export function columnBudgetPx(): number {
  return COLUMN_KEYS.reduce((sum, key) => sum + COLUMN_WIDTHS_PX[key], 0);
}

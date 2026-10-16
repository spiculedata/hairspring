/**
 * Column widths of the property table, in one place so the stylesheet, the markup and a test agree.
 *
 * `main` in styles.css is at most 1000px wide with 16px gutters, so tables get 968px. The columns must add up to no more
 * than that, or at desktop width the last columns are cut off. Below it the table keeps these widths and scrolls
 * sideways, with a visible hint.
 */
export const MAIN_MAX_WIDTH_PX = 1000;
export const MAIN_GUTTER_PX = 16;
export const CONTENT_WIDTH_PX = MAIN_MAX_WIDTH_PX - 2 * MAIN_GUTTER_PX;

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

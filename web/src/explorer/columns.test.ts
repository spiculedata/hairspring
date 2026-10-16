import { describe, expect, it } from 'vitest';
import { COLUMN_KEYS, COLUMN_WIDTHS_PX, CONTENT_WIDTH_PX, columnBudgetPx } from './columns';

describe('property table column budget', () => {
  it('fits the page content width, so all six columns are visible without scrolling at desktop width', () => {
    expect(COLUMN_KEYS).toHaveLength(6);
    expect(columnBudgetPx()).toBeLessThanOrEqual(CONTENT_WIDTH_PX);
  });

  it('gives every column a usable width, the rate column the most room after the property column', () => {
    for (const key of COLUMN_KEYS) expect(COLUMN_WIDTHS_PX[key]).toBeGreaterThanOrEqual(72);
    expect(COLUMN_WIDTHS_PX.rate).toBeGreaterThanOrEqual(260);
  });

  it('uses the content width the stylesheet gives main: 1000px less 16px gutters each side', () => {
    expect(CONTENT_WIDTH_PX).toBe(1000 - 2 * 16);
  });
});

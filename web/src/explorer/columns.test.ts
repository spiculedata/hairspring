import { describe, expect, it } from 'vitest';
import {
  columnBudgetPx as trialBudgetPx,
  COLUMN_WIDTHS_PX as TRIAL_WIDTHS,
  FLEXIBLE_COLUMN as TRIAL_FLEXIBLE,
  FLEXIBLE_MIN_PX as TRIAL_FLEXIBLE_MIN_PX,
} from '../seeds/columns';
import { readSource } from '../readSource';
import { statusLabel, statusMark } from '../wording';
import {
  COLUMN_KEYS,
  COLUMN_WIDTHS_PX,
  CONTENT_WIDTH_PX,
  FLEXIBLE_COLUMN,
  FLEXIBLE_MIN_PX,
  MAIN_GUTTER_PX,
  MAIN_MAX_WIDTH_PX,
  SCROLLBAR_PX,
  TABLE_BOX_BORDER_PX,
  columnBudgetPx,
} from './columns';

const stylesCss = readSource('styles.css');
const explorerCss = readSource('explorer/explorer.css');

describe('the page widths the column budgets rest on, read from the stylesheets', () => {
  it('main is at most 1000px wide with 16px gutters', () => {
    expect(/main\s*\{[^}]*max-width:\s*1000px/.test(stylesCss)).toBe(true);
    expect(/main\s*\{[^}]*padding:\s*1rem 16px/.test(stylesCss)).toBe(true);
    expect(MAIN_MAX_WIDTH_PX).toBe(1000);
    expect(MAIN_GUTTER_PX).toBe(16);
    expect(CONTENT_WIDTH_PX).toBe(1000 - 2 * 16);
  });

  it('the scroll box has a 1px border on each side', () => {
    expect(/\.table-scroll\s*\{[^}]*border:\s*1px solid/.test(explorerCss)).toBe(true);
    expect(TABLE_BOX_BORDER_PX).toBe(2);
  });
});

describe('property table column budget', () => {
  it('has six columns, one of which takes the width that is left', () => {
    expect(COLUMN_KEYS).toHaveLength(6);
    expect(COLUMN_KEYS).toContain(FLEXIBLE_COLUMN);
    expect(COLUMN_WIDTHS_PX[FLEXIBLE_COLUMN]).toBeUndefined();
  });

  it('fits inside the scroll box even when a classic page scrollbar narrows it, so the table scrolls only in a narrower window', () => {
    expect(SCROLLBAR_PX).toBeGreaterThanOrEqual(15);
    expect(columnBudgetPx()).toBeLessThanOrEqual(CONTENT_WIDTH_PX - TABLE_BOX_BORDER_PX - SCROLLBAR_PX);
  });

  it('gives every fixed column a usable width, and the rate column the most room', () => {
    for (const key of COLUMN_KEYS) {
      if (key !== FLEXIBLE_COLUMN) expect(COLUMN_WIDTHS_PX[key]).toBeGreaterThanOrEqual(72);
    }
    expect(COLUMN_WIDTHS_PX.rate).toBeGreaterThanOrEqual(260);
    expect(FLEXIBLE_MIN_PX).toBeGreaterThanOrEqual(160);
  });
});

describe('the status column shows every status label whole', () => {
  // A conservative width for text at the 15px body size: 0.6em per character, plus the cell's 8px padding each side.
  const BODY_PX = 15;
  const EM_PER_CHAR = 0.6;
  const PAD_PX = 16;
  const longest = (['violation', 'unmet', 'unrated', 'pass'] as const).map((s) => `${statusMark(s)} ${statusLabel(s)}`).sort((a, b) => b.length - a.length)[0];

  it('is wide enough for the longest label on a desktop', () => {
    expect(longest.length * EM_PER_CHAR * BODY_PX + PAD_PX).toBeLessThanOrEqual(COLUMN_WIDTHS_PX.status ?? 0);
  });

  it('is wide enough on a phone, where the stylesheet sets the first column in rem', () => {
    const m = /table\.props col:nth-child\(1\) \{ width: ([\d.]+)rem/.exec(explorerCss);
    expect(m).not.toBeNull();
    expect(longest.length * EM_PER_CHAR * BODY_PX + PAD_PX).toBeLessThanOrEqual(Number(m![1]) * 16);
  });

  it('keeps the phone table as wide as its three columns', () => {
    const rem = (re: RegExp) => Number(re.exec(explorerCss)![1]);
    const sum = rem(/col:nth-child\(1\) \{ width: ([\d.]+)rem/) + rem(/col:nth-child\(3\) \{ width: ([\d.]+)rem/) + rem(/col:nth-child\(4\) \{ width: ([\d.]+)rem/);
    expect(rem(/width: ([\d.]+)rem; \}\n\s*table\.props col:nth-child\(1\)/)).toBeCloseTo(sum, 5);
  });
});

describe('trial table column budget', () => {
  it('has one column that takes the width that is left, and fits inside the box with a classic scrollbar', () => {
    expect(TRIAL_WIDTHS[TRIAL_FLEXIBLE]).toBeUndefined();
    expect(TRIAL_FLEXIBLE_MIN_PX).toBeGreaterThanOrEqual(160);
    expect(trialBudgetPx()).toBeLessThanOrEqual(CONTENT_WIDTH_PX - TABLE_BOX_BORDER_PX - SCROLLBAR_PX);
  });
});

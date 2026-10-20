import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import { DEFAULT_ROOT_FONT_PX, parseFontPx, ROW_CHROME_PX, rowHeightPx } from './rowHeight';

const stylesCss = readSource('styles.css');
const explorerCss = readSource('explorer/explorer.css');

/** The declaration block of the first rule whose selector list contains `selector`. */
function block(css: string, selector: string): string {
  const bare = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const rule = [...bare.matchAll(/([^{}]+)\{([^}]*)\}/g)].find((m) => m[1].split(',').some((s) => s.trim() === selector));
  if (!rule) throw new Error(`no rule for ${selector}`);
  return rule[2];
}
const decl = (css: string, selector: string, prop: string): string => {
  const m = new RegExp(`(?:^|[;\\s])${prop}:\\s*([^;]+)`).exec(block(css, selector));
  if (!m) throw new Error(`no ${prop} in ${selector}`);
  return m[1].trim();
};
const rem = (v: string) => Number(v.replace('rem', ''));
const px = (v: string) => Number(v.replace('px', ''));

describe('property row height, against the stylesheet', () => {
  it('takes the type sizes and line height the stylesheet gives the stacked rate cell', () => {
    // The cell stacks: the figure at the body size, a two-line qualifier, an UNMET note and an inconclusive note.
    expect(px(decl(stylesCss, 'body', 'font').split('/')[0])).toBe(15);
    expect(decl(explorerCss, 'table.props tbody td', 'line-height')).toBe('1.25');
    expect(rem(decl(explorerCss, '.rate-qualifier', 'font-size'))).toBe(0.85);
    expect(rem(decl(explorerCss, '.unmet-note', 'font-size'))).toBe(0.8);
    expect(rem(decl(explorerCss, '.inconclusive-note', 'font-size'))).toBe(0.8);
    expect(decl(explorerCss, '.rate-qualifier', '-webkit-line-clamp')).toBe('2');
  });

  it('takes its chrome from the cell padding and the row border', () => {
    expect(decl(explorerCss, 'table.props tbody td', 'padding').startsWith('8px')).toBe(true);
    expect(decl(explorerCss, 'table.props tbody td', 'border-bottom')).toMatch(/^1px /);
    expect(ROW_CHROME_PX).toBe(2 * 8 + 1);
  });

  it('is 104px at the default 16px root size, where the cell was measured in a browser', () => {
    expect(DEFAULT_ROOT_FONT_PX).toBe(16);
    expect(rowHeightPx(16)).toBe(104);
  });

  it('grows with the root font size, because the qualifier and the notes are sized in rem', () => {
    expect(rowHeightPx(20)).toBeGreaterThan(rowHeightPx(16));
    expect(rowHeightPx(24)).toBeGreaterThan(rowHeightPx(20));
    expect(rowHeightPx(12)).toBeLessThan(rowHeightPx(16));
  });
});

describe('parseFontPx', () => {
  it('reads a computed pixel size', () => {
    expect(parseFontPx('20px')).toBe(20);
    expect(parseFontPx('17.5px')).toBe(17.5);
  });

  it('falls back to the default for anything that is not a usable size', () => {
    expect(parseFontPx('')).toBe(16);
    expect(parseFontPx('large')).toBe(16);
    expect(parseFontPx('0px')).toBe(16);
    expect(parseFontPx('-3px')).toBe(16);
  });
});

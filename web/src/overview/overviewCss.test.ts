import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';

const css = readSource('overview/overview.css');
const rule = (selector: string): string => {
  const start = css.indexOf(`\n${selector} {`);
  if (start < 0) throw new Error(`no rule for ${selector}`);
  return css.slice(start, css.indexOf('}', start));
};

describe('Overview rows', () => {
  it('draws the recent runs and failures as a plain list, without bullets or indent', () => {
    expect(rule('.overview-list')).toMatch(/list-style:\s*none/);
    expect(rule('.overview-list')).toMatch(/padding:\s*0/);
  });

  it('spaces each row with padding above and below and a line between rows', () => {
    const row = rule('.overview-list li');
    expect(row).toMatch(/padding:\s*\.5rem 0/);
    expect(row).toMatch(/border-bottom:\s*1px solid var\(--line\)/);
  });

  it('lets a long id or message wrap inside its row instead of widening the page', () => {
    expect(rule('.overview-list li')).toMatch(/overflow-wrap:\s*anywhere/);
  });

  it('puts the detail line a little below the row heading', () => {
    expect(rule('.overview-detail')).toMatch(/margin-top:\s*\.25rem/);
  });

  it('keeps the relative time beside the row on a wide screen and under it on a narrow one', () => {
    expect(rule('.overview-list .time-relative')).toMatch(/display:\s*inline/);
    expect(rule('.overview-list .time-relative')).toMatch(/margin-left:\s*\.5rem/);
    expect(css).toMatch(/@media \(max-width: 40rem\)\s*\{\s*\.overview-list \.time-relative \{ display: block; margin-left: 0; \}/);
  });
});

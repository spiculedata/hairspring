import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { TimeStamp } from './TimeStamp';
import { UpdatedAt } from './UpdatedAt';

const THEN = Date.UTC(2026, 9, 30, 15, 44, 50);

describe('TimeStamp', () => {
  it('shows the absolute time first, with the relative time as the second, quieter line', () => {
    const html = renderToString(<TimeStamp ms={THEN} now={THEN + 90_000} timeZone="UTC" />);
    expect(html).toContain('30 Oct 2026, 15:44:50 UTC');
    expect(html).toContain('1m ago');
    expect(html.indexOf('30 Oct')).toBeLessThan(html.indexOf('1m ago'));
    expect(html).toMatch(/<time dateTime="2026-10-30T15:44:50.000Z"/);
    expect(html).toContain('class="muted time-relative"');
  });
});

describe('UpdatedAt', () => {
  it('says when the data on the page was last loaded, in absolute time', () => {
    const html = renderToString(<UpdatedAt at={THEN} timeZone="UTC" />);
    expect(html).toContain('Updated 15:44:50 UTC');
  });
  it('says nothing until there is data', () => {
    expect(renderToString(<UpdatedAt at={undefined} />)).toBe('');
  });
});

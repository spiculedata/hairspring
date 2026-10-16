import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { assertTextOnly, CORPUS } from '../guest/hostile';
import { row } from '../explorer/fixtures';
import { SeedFilterBar } from './SeedFilterBar';

const noop = () => undefined;
type Props = Parameters<typeof SeedFilterBar>[0];
const base: Props = {
  filters: {},
  onChange: noop,
  propertyLabel: undefined,
  search: { text: '', matches: [], state: 'idle' },
  onSearch: noop,
  fpText: '',
  onFpText: noop,
};
const render = (over: Partial<Props> = {}) => renderToString(<SeedFilterBar {...base} {...over} />);
const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('SeedFilterBar', () => {
  it('offers every verdict the server filters on, and all', () => {
    const html = render();
    for (const v of ['All', 'violation', 'inconclusive', 'unusable', 'infrastructure error', 'pass']) expect(textOf(html)).toContain(v);
    expect(html).toContain('<select');
  });

  it('selects the current verdict', () => {
    expect(render({ filters: { verdict: 'violation' } })).toMatch(/<option value="violation" selected/);
  });

  it('names the property filter in words and shows the property id as escaped text', () => {
    const html = render({ filters: { fails: 3 }, propertyLabel: 'wal.no-lost-commit' });
    expect(textOf(html)).toContain('failed in');
    expect(textOf(html)).toContain('wal.no-lost-commit');
  });

  it('lists matching properties as buttons that pick one', () => {
    const html = render({
      search: { text: 'wal', state: 'idle', matches: [row({ idx: 4, guest: { id: 'wal.header', msg: '', loc: null } })] },
    });
    expect(html).toContain('wal.header');
    expect(html).toMatch(/<button type="button"[^>]*>/);
  });

  it('says when no property matches', () => {
    expect(textOf(render({ search: { text: 'zzz', matches: [], state: 'idle' } }))).toContain('No property id starts with that.');
  });

  it('rejects a fingerprint that is not 16 to 64 hex digits, and says what one is', () => {
    expect(textOf(render({ fpText: '4c16' }))).toContain('A fingerprint is 16 to 64 hex digits.');
    expect(textOf(render({ fpText: '4c16f00dcafe0001' }))).not.toContain('A fingerprint is');
    expect(textOf(render({ fpText: '' }))).not.toContain('A fingerprint is');
  });

  it('shows where the list starts and how to go back, when it was started at a trial', () => {
    const text = textOf(render({ filters: { from: 40 } }));
    expect(text).toContain('starts at trial 40');
    expect(text).toContain('Show from the first trial');
  });

  it('offers to clear filters only when some are set', () => {
    expect(textOf(render())).not.toContain('Clear filters');
    expect(textOf(render({ filters: { verdict: 'pass' } }))).toContain('Clear filters');
  });
});

describe.each(CORPUS)('XSS corpus: %s', (_name, hostile) => {
  it('shows a hostile property id as escaped text only, in the chip and in the matches', () => {
    const html = render({
      filters: { fails: 1 },
      propertyLabel: hostile,
      search: { text: 'x', state: 'idle', matches: [row({ guest: { id: hostile, msg: hostile, loc: null } })] },
    });
    assertTextOnly(html);
  });

  it('keeps typed text only in the value of its own input, escaped, and never repeats it elsewhere', () => {
    const typed = hostile.slice(0, 200);
    const html = render({ fpText: typed, search: { text: typed, matches: [], state: 'idle' } });
    expect(html).not.toMatch(/<(script|img|svg|iframe|object)/i);
    const outsideInputs = html.replace(/<input[^>]*>/g, '');
    // Printable ASCII only, so the check is about the markup characters and not about invisible ones.
    const needle = typed.slice(0, 20).replace(/[^\x20-\x7e]/g, '');
    // A needle shorter than this would match the page's own words by chance.
    if (needle.length >= 8) expect(outsideInputs).not.toContain(needle);
  });
});

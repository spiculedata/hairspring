import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { GuestText } from '../components/GuestText';
import { CH } from '../guest/chars';
import { page, row } from './fixtures';
import { PropertyOverviewView } from './PropertyOverview';
import { PropertyTable } from './PropertyTable';
import type { PropertyRow } from '../types';

/** Hostile strings from the plan's corpus (RESULTS-EXPLORER-PLAN 7.1), plus a 10 MB string. */
const CORPUS: ReadonlyArray<readonly [string, string]> = [
  ['script tag', '<script>alert(1)</script>'],
  ['img onerror', '<img src=x onerror=alert(1)>'],
  ['svg onload', '<svg/onload=alert(1)>'],
  ['attribute break-out', '" onmouseover="alert(1)" x="'],
  ['javascript url', 'javascript:alert(document.cookie)'],
  ['data url', 'data:text/html,<script>alert(1)</script>'],
  ['template syntax', '{{constructor.constructor("alert(1)")()}}'],
  ['bidi override', `safe${CH.RLO}evil.exe`],
  ['bidi isolate', `${CH.LRI}hidden${CH.PDI}`],
  ['line separator', `a${CH.LS}b${CH.PS}c`],
  ['ansi escape', `${CH.ESC}[31mred${CH.ESC}[2J${CH.ESC}]0;title${CH.BEL}`],
  ['nul and del', `a${CH.NUL}b${CH.DEL}c`],
  ['512 bytes', '<b>'.repeat(171)],
  ['10 MB', '<script>'.repeat((10 * 1024 * 1024) / 8)],
];

/** Control, format and separator characters that must never reach the page raw. */
const UNSAFE_IN_MARKUP = new RegExp(
  '[\\u0000-\\u0008\\u000b-\\u001f\\u007f\\u0080-\\u009f\\u200b-\\u200f\\u2028-\\u202e\\u2060-\\u206f\\ufeff]',
);

function hostileRow(text: string, status: PropertyRow['status'] = 'violation'): PropertyRow {
  return row({
    status,
    unmet: status === 'unmet' ? 'coverage gap, not a finding: never satisfied in 5 claimable trials' : null,
    guest: { id: text, msg: text, loc: { file: text, line: 1, func: text } },
  });
}

/** Every tag in the output, so guest text can be checked for never having reached markup. */
const tagsOf = (html: string) => html.match(/<[^>]*>/g) ?? [];

function assertTextOnly(html: string) {
  const tags = tagsOf(html);
  for (const tag of tags) {
    expect(tag, 'guest text reached markup').not.toMatch(/alert|javascript:|onerror|onload|onmouseover|constructor|data:text/i);
    expect(tag).not.toMatch(UNSAFE_IN_MARKUP);
  }
  expect(html).not.toMatch(UNSAFE_IN_MARKUP);
  const names = new Set(tags.map((t) => /^<\/?([a-zA-Z0-9]+)/.exec(t)?.[1]?.toLowerCase()));
  for (const forbidden of ['script', 'img', 'svg', 'iframe', 'object', 'embed', 'style', 'link', 'a', 'form']) {
    expect(names.has(forbidden), `unexpected <${forbidden}>`).toBe(false);
  }
}

describe.each(CORPUS)('XSS corpus: %s', (_name, hostile) => {
  it('GuestText shows it as escaped text only', () => {
    assertTextOnly(renderToString(<GuestText text={hostile} />));
  });

  it('PropertyTable shows it as escaped text only, in every guest field', () => {
    assertTextOnly(renderToString(<PropertyTable properties={[hostileRow(hostile)]} />));
  });

  it('the whole overview shows it as escaped text only, for violation and UNMET rows', () => {
    const rows = [hostileRow(hostile), hostileRow(hostile, 'unmet')];
    const p = page({ properties: rows, counts_by_status: { violation: 1, unmet: 1 } });
    const html = renderToString(
      <PropertyOverviewView page={p} rows={rows} query={{}} onQuery={() => undefined} onLoadMore={undefined} loadingMore={false} />,
    );
    assertTextOnly(html);
    // Display is bounded however large the input was.
    expect(html.length).toBeLessThan(80_000);
  });
});

describe('XSS corpus: visible, escaped output', () => {
  it('turns a bidi override into a visible marker and shows the markup characters as text', () => {
    const html = renderToString(<GuestText text={`safe${CH.RLO}evil <b>`} />);
    expect(html).toContain('\\u{202E}');
    expect(html).toContain('evil &lt;b&gt;');
  });

  it('turns ANSI escapes into visible markers', () => {
    expect(renderToString(<GuestText text={`${CH.ESC}[31mred`} />)).toContain('\\u{1B}</span>[31mred');
  });
});

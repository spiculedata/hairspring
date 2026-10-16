import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { GuestText } from '../components/GuestText';
import { CH } from '../guest/chars';
import { assertTextOnly, CORPUS } from '../guest/hostile';
import { page, row } from './fixtures';
import { PropertyOverviewView } from './PropertyOverview';
import { PropertyTable } from './PropertyTable';
import type { PropertyRow } from '../types';

function hostileRow(text: string, status: PropertyRow['status'] = 'violation'): PropertyRow {
  return row({
    status,
    unmet: status === 'unmet' ? 'coverage gap, not a finding: never satisfied in 5 claimable trials' : null,
    guest: { id: text, msg: text, loc: { file: text, line: 1, func: text } },
  });
}

describe.each(CORPUS)('XSS corpus: %s', (_name, hostile) => {
  it('GuestText shows it as escaped text only', () => {
    assertTextOnly(renderToString(<GuestText text={hostile} />));
  });

  it('PropertyTable shows it as escaped text only, in every guest field', () => {
    assertTextOnly(renderToString(<PropertyTable runId="run_1" properties={[hostileRow(hostile)]} />));
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

import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { row } from './fixtures';
import { PropertyTable } from './PropertyTable';

const many = (n: number) =>
  Array.from({ length: n }, (_, i) => row({ idx: i, status: 'pass', first_failing_seed: null, guest: { id: `prop.${i}`, msg: '', loc: null } }));

describe('PropertyTable', () => {
  it('uses table semantics: caption, column headers and a row header per property', () => {
    const html = renderToString(<PropertyTable properties={[row()]} />);
    expect(html).toContain('<table');
    expect(html).toContain('<caption');
    expect(html).toContain('<th scope="col"');
    expect(html).toContain('<th scope="row"');
    expect(html).toContain('role="region"');
    expect(html).toContain('tabindex="0"');
  });

  it('shows a violation row with status text, kind, id, rate with interval, first failing seed and evals', () => {
    const html = renderToString(<PropertyTable properties={[row()]} />);
    expect(html).toContain('VIOLATION');
    expect(html).toContain('always');
    expect(html).toContain('wal.no-lost-commit');
    expect(html).toContain('3/4,790');
    expect(html).toContain('[0.02%–0.18%]');
    expect(html).toContain('seed 1016');
    expect(html).toContain('1,200,000');
  });

  it('shows a seed above 2^53 exactly, as the string the server sent', () => {
    const html = renderToString(<PropertyTable properties={[row({ first_failing_seed: '18446744073709551615' })]} />);
    expect(html).toContain('seed 18446744073709551615');
  });

  it('shows UNMET as a coverage gap in its own tone, never as a failure', () => {
    const unmet = row({
      status: 'unmet',
      kind: 'sometimes',
      rate: { kind: 'zero_events', basis: 'trials_true', n: 4790, upper_bound: 3 / 4790 },
      first_failing_seed: null,
      unmet: 'coverage gap, not a finding: never satisfied in 4,790 claimable trials',
      guest: { id: 'ckpt.passive', msg: '', loc: null },
    });
    const html = renderToString(<PropertyTable properties={[unmet]} />);
    expect(html).toContain('UNMET (coverage gap)');
    expect(html).toContain('status-unmet');
    expect(html).not.toContain('status-violation');
    const body = html.slice(html.indexOf('<tbody'));
    expect(body).not.toMatch(/fail|violation/i);
    expect(body).toContain('coverage gap, not a finding');
  });

  it('shows an unrated property as unrated, with no percentage', () => {
    const unrated = row({ status: 'unrated', kind: 'reachable', rate: { kind: 'unrated' }, first_failing_seed: null, evals: 0 });
    const text = renderToString(<PropertyTable properties={[unrated]} />).replace(/<[^>]+>/g, ' ');
    expect(text).toContain('unrated (0 rated trials)');
    expect(text).not.toContain('%');
  });

  it('shows failures seen only in inconclusive trials as exactly that', () => {
    const html = renderToString(<PropertyTable properties={[row({ status: 'pass', failures_seen_in_inconclusive_trials: 2, first_failing_seed: null })]} />);
    expect(html).toContain('seen in 2 inconclusive trials');
  });

  it('windows a long list: renders a bounded number of rows and keeps the full row count for assistive tech', () => {
    const html = renderToString(<PropertyTable properties={many(5000)} />);
    const rendered = html.match(/<tr[^>]*aria-rowindex/g)?.length ?? 0;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(40);
    expect(html).toContain('aria-rowcount="5001"');
    expect(html).toContain('prop.0');
    expect(html).not.toContain('prop.4999');
  });

  it('renders an empty list as a message, not an empty table', () => {
    const html = renderToString(<PropertyTable properties={[]} />);
    expect(html).toContain('No properties match.');
    expect(html).not.toContain('<table');
  });

  it('tells the reader where the text came from', () => {
    const html = renderToString(<PropertyTable properties={[row()]} />);
    expect(html).toContain('from the program under test');
  });
});

import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { claim, header, page, row } from './fixtures';
import { PropertyOverviewView } from './PropertyOverview';
import type { PropertyPage, PropertyRow } from '../types';

const noop = () => undefined;
const render = (p: PropertyPage, rows: PropertyRow[] = p.properties) =>
  renderToString(
    <PropertyOverviewView page={p} rows={rows} query={{}} onQuery={noop} onLoadMore={p.next_cursor ? noop : undefined} loadingMore={false} />,
  );
const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

const unmetRow = (over: Partial<PropertyRow> = {}) =>
  row({
    idx: 1,
    status: 'unmet',
    kind: 'sometimes',
    rate: { kind: 'zero_events', basis: 'trials_true', n: 4790, upper_bound: 3 / 4790 },
    first_failing_seed: null,
    unmet: 'coverage gap, not a finding: never satisfied in 4,790 claimable trials',
    guest: { id: 'ckpt.passive-during-write', msg: '', loc: null },
    ...over,
  });

describe('PropertyOverviewView header', () => {
  it('shows the generated headline, completion, counts and tier chips', () => {
    const html = render(page());
    const text = textOf(html);
    expect(text).toContain('1 property failed; first failing seed 1016');
    expect(text).toContain('complete');
    expect(text).toContain('4,790 rated');
    expect(text).toContain('211 schedules');
    expect(html).toContain('label not declared');
  });

  it('shows the claim sentence only when the server sent one, otherwise why not', () => {
    expect(textOf(render(page({ claim: claim({ text: 'The claim.', withheld: [] }) })))).toContain('The claim.');
    const withheld = textOf(render(page({ claim: claim({ withheld: ['no label was declared'] }) })));
    expect(withheld).toContain('No claim sentence: no label was declared');
  });

  it('shows "unverified worker data" when the header says so', () => {
    const html = render(page({ header: header({ unverified_worker_data: true, workers_untrusted: 1 }), claim: claim({ chips: ['label not declared'] }) }));
    expect(html).toContain('unverified worker data');
  });

  it('shows the server notes, such as the stopped-at-first-failure line', () => {
    const note = 'Stopped at first failure: no rates, no UNMET claim. The stop decided how many trials ran.';
    expect(textOf(render(page({ notes: [note] })))).toContain(note);
  });

  it('shows excluded trials next to n, named, never as part of a denominator', () => {
    const text = textOf(render(page()));
    expect(text).toContain('Excluded from every rate: 9 degraded, 13 inconclusive');
  });

  it('says nothing about exclusions when nothing was excluded', () => {
    const none = header({ excluded: { degraded: 0, perturbing: 0, inconclusive: 0, report_missing: 0 } });
    expect(textOf(render(page({ header: none })))).not.toContain('Excluded from every rate');
  });

  it('keeps perturbing trials labelled as excluded from tier claims', () => {
    const h = header({ excluded: { degraded: 0, perturbing: 2, inconclusive: 0, report_missing: 0 } });
    expect(textOf(render(page({ header: h })))).toContain('2 perturbing (eval lines perturb the schedule; excluded from tier claims)');
  });
});

describe('PropertyOverviewView UNMET section', () => {
  it('lists UNMET as a coverage gap in its own section, not among failures', () => {
    const p = page({ properties: [row(), unmetRow()], counts_by_status: { violation: 1, unmet: 1 } });
    const html = render(p);
    expect(html).toContain('Coverage gaps (UNMET)');
    expect(textOf(html)).toContain('coverage gaps, not findings');
    expect(textOf(html)).toContain('coverage gap, not a finding: never satisfied in 4,790 claimable trials');
  });

  it('is suppressed with the reason when the run stopped at its first failure', () => {
    const stopped = header({ stopped_at_first_failure: true, completion: { kind: 'stopped_at_first_failure' } });
    const p = page({
      header: stopped,
      properties: [unmetRow({ unmet: null, rate: { kind: 'not_a_rate', basis: 'trials_true', k: 0, n: 16 } })],
      counts_by_status: { unmet: 1 },
    });
    const text = textOf(render(p));
    expect(text).toContain('Coverage gaps (UNMET)');
    expect(text).toContain('not stated: the run stopped at its first failure');
    expect(text).not.toContain('never satisfied');
  });

  it('is absent when no property is UNMET', () => {
    expect(render(page())).not.toContain('Coverage gaps (UNMET)');
  });

  it('says how many more UNMET properties there are than are loaded', () => {
    const p = page({ properties: [unmetRow()], counts_by_status: { unmet: 3 } });
    expect(textOf(render(p))).toContain('2 more UNMET properties not loaded');
  });
});

describe('PropertyOverviewView controls', () => {
  it('has labelled filters and a load-more button only when there is another page', () => {
    const html = render(page());
    expect(html).toContain('<label');
    expect(html).toContain('Filter by id prefix');
    expect(html).not.toContain('Load more');
    const more = render(page({ next_cursor: '100', total: 250 }));
    expect(textOf(more)).toContain('Load more (1 of 250 shown)');
  });

  it('shows status counts', () => {
    const p = page({ counts_by_status: { violation: 1, pass: 30, unmet: 2, unrated: 4 }, total: 37 });
    expect(textOf(render(p))).toContain('Properties (37)');
  });
});

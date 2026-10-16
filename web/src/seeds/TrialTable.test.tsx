import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { COLUMN_KEYS, COLUMN_WIDTHS_PX, columnBudgetPx } from './columns';
import { CONTENT_WIDTH_PX } from '../explorer/columns';
import { failingRow, manyRows, seedRow } from './fixtures';
import { CLIP_HEIGHT, ROW_HEIGHT } from './rowMetrics';
import { TrialTable } from './TrialTable';

const noop = () => undefined;
const render = (rows: ReturnType<typeof manyRows>, extra: Partial<Parameters<typeof TrialTable>[0]> = {}) =>
  renderToString(<TrialTable runId="run_0123" rows={rows} total={undefined} highlightSeq={undefined} onFilter={noop} onNearEnd={noop} {...extra} />);
const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('TrialTable', () => {
  it('uses table semantics: caption, column headers and a row header per trial', () => {
    const html = render([failingRow()]);
    expect(html).toContain('<table');
    expect(html).toContain('<caption');
    expect(html).toContain('<th scope="col"');
    expect(html).toContain('<th scope="row"');
    expect(html).toContain('role="region"');
    expect(html).toContain('tabindex="0"');
  });

  it('shows a failing trial with its seq, seed, verdict, failing id, fingerprint, ops, ms and detail', () => {
    const text = textOf(render([failingRow()]));
    expect(text).toContain('16');
    expect(text).toContain('1016');
    expect(text).toContain('✕ violation');
    expect(text).toContain('wal.no-lost-commit');
    expect(text).toContain('4c16f00d');
    expect(text).toContain('3,000');
    expect(text).toContain('912');
    expect(text).toContain('yes');
  });

  it('links the sequence number to the failure page of that trial', () => {
    expect(render([failingRow()])).toContain('<a href="/runs/run_0123/trials/16">16</a>');
    expect(render([failingRow()], { runId: '../x' })).not.toContain('<a ');
  });

  it('shows a seed above 2^53 exactly, as the string the server sent', () => {
    expect(render([seedRow({ seed: '18446744073709551615' })])).toContain('18446744073709551615');
  });

  it('shows a degraded pass as excluded and never as a pass', () => {
    const html = render([seedRow({ verdict: 'pass', class: 'excluded', degraded: true, rated: false })]);
    const body = html.slice(html.indexOf('<tbody'));
    expect(textOf(body)).toContain('◌ excluded (degraded)');
    expect(textOf(body)).not.toMatch(/✓|\bpass\b/);
    expect(body).toContain('class-excluded');
  });

  it('shows an inconclusive trial as inconclusive, with its own mark', () => {
    const body = render([seedRow({ verdict: 'inconclusive', class: 'inconclusive', timed_out: true })]);
    expect(textOf(body)).toContain('? inconclusive');
    expect(textOf(body.slice(body.indexOf('<tbody')))).not.toMatch(/\bpass\b/);
  });

  it('shows a clean pass with a mark and words', () => {
    expect(textOf(render([seedRow()]))).toContain('✓ pass');
  });

  it('names the first failing ids and counts the rest', () => {
    const row = failingRow({ failing_count: 5, failing_idx: [0, 1, 2], guest: { reason: '', failing_ids: ['a.one', 'b.two', 'c.three'] } });
    const text = textOf(render([row]));
    expect(text).toContain('a.one');
    expect(text).toContain('b.two');
    expect(text).not.toContain('c.three');
    expect(text).toContain('+3 more');
  });

  it('makes a failing id a button that filters to the trials where it failed', () => {
    const html = render([failingRow()]);
    expect(html).toMatch(/<button type="button" class="linklike[^"]*">/);
  });

  it('makes a fingerprint a button only when it is hex, and shows anything else as escaped text', () => {
    expect(render([failingRow()])).toContain('fp-filter');
    const html = render([seedRow({ schedule_fp: '<img src=x onerror=alert(1)>' })]);
    const body = html.slice(html.indexOf('<tbody'));
    expect(body).not.toContain('<img');
    expect(body).not.toContain('fp-filter');
  });

  it('shows no fingerprint, failing ids or detail as a dash and a plain no', () => {
    const text = textOf(render([seedRow({ schedule_fp: null })]));
    expect(text).toContain('–');
    expect(text).toContain('no');
  });

  it('windows a long list: renders a bounded number of rows and keeps the loaded row count for assistive tech', () => {
    const html = render(manyRows(5000), { total: 5000 });
    const rendered = html.match(/<tr[^>]*aria-rowindex/g)?.length ?? 0;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(40);
    expect(html).toContain('aria-rowcount="5001"');
    expect(html).toContain('1000');
    expect(html).not.toContain('5999');
  });

  it('says the row count is unknown when the filtered total is not known', () => {
    expect(render(manyRows(10))).toContain('aria-rowcount="-1"');
  });

  it('keeps every row exactly one height: every cell clips its content to a fixed-height block', () => {
    const worst = failingRow({
      failing_count: 9,
      failing_idx: [0, 1],
      guest: { reason: '', failing_ids: ['i'.repeat(128), 'j'.repeat(128)] },
    });
    const html = render([worst]);
    const body = html.slice(html.indexOf('<tbody'));
    const clips = body.match(/<div class="cell-clip" style="height:(\d+)px">/g) ?? [];
    expect(clips).toHaveLength(COLUMN_KEYS.length);
    for (const c of clips) expect(Number(/height:(\d+)px/.exec(c)?.[1])).toBe(CLIP_HEIGHT);
    expect(body).toContain(`<tr aria-rowindex="2" style="height:${ROW_HEIGHT}px"`);
    expect(CLIP_HEIGHT).toBeLessThan(ROW_HEIGHT);
  });

  it('sets each column to its budgeted width, the widths to the page content width, and tells a narrow screen to scroll', () => {
    const html = render([seedRow()]);
    const widths = [...html.matchAll(/<col style="width:(\d+)px"/g)].map((m) => Number(m[1]));
    expect(widths).toEqual(COLUMN_KEYS.map((k) => COLUMN_WIDTHS_PX[k]));
    expect(columnBudgetPx()).toBeLessThanOrEqual(CONTENT_WIDTH_PX);
    expect(html).toContain(`min-width:${columnBudgetPx()}px`);
    expect(html).toContain('class="scroll-hint"');
  });

  it('marks the row a jump landed on', () => {
    const html = render([seedRow({ seq: 4 }), seedRow({ seq: 5 })], { highlightSeq: 5 });
    expect(html.match(/start-row/g)).toHaveLength(1);
  });

  it('renders an empty list as nothing, leaving the message to the page', () => {
    expect(render([])).not.toContain('<table');
  });

  it('tells the reader where the ids came from', () => {
    expect(render([failingRow()])).toContain('from the program under test');
  });
});

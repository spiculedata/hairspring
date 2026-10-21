import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { row } from './fixtures';
import { COLUMN_KEYS, COLUMN_WIDTHS_PX, columnBudgetPx } from './columns';
import { CLIP_HEIGHT, PropertyRowView, ROW_HEIGHT } from './PropertyRowView';
import { ROW_CHROME_PX, rowHeightPx } from './rowHeight';
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

  it('links the first failing seed to its failure page when the run is known, and shows plain text when it is not', () => {
    const linked = renderToString(<PropertyTable runId="run_0123" properties={[row()]} />);
    expect(linked).toContain('<a href="/runs/run_0123/seeds/1016">seed 1016</a>');
    expect(renderToString(<PropertyTable properties={[row()]} />)).not.toContain('<a ');
    expect(renderToString(<PropertyTable runId="run_0123" properties={[row({ first_failing_seed: null })]} />)).not.toContain('/seeds/');
  });

  it('links a failing property to the list of seeds that failed it, and any other to the seeds that reached it', () => {
    const failing = renderToString(<PropertyTable runId="run_0123" properties={[row({ idx: 7 })]} />);
    expect(failing).toContain('<a href="/runs/run_0123/seeds?fails=7">failing seeds</a>');
    const pass = renderToString(<PropertyTable runId="run_0123" properties={[row({ idx: 8, status: 'pass', first_failing_seed: null })]} />);
    expect(pass).toContain('<a href="/runs/run_0123/seeds?hits=8">seeds that reached it</a>');
    const unmet = renderToString(<PropertyTable runId="run_0123" properties={[row({ idx: 9, status: 'unmet', first_failing_seed: null })]} />);
    expect(unmet).toContain('seeds?misses=9');
    expect(renderToString(<PropertyTable properties={[row()]} />)).not.toContain('/seeds?');
  });

  it('only links a seed that is plain digits, so nothing else can reach a path', () => {
    const html = renderToString(<PropertyTable runId="run_0123" properties={[row({ first_failing_seed: '../x?y' })]} />);
    expect(html).not.toContain('/seeds/');
    expect(html).not.toContain('href="../');
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

  it('reports the size of the whole result, not of what is loaded, in aria-rowcount', () => {
    const html = renderToString(<PropertyTable properties={many(10)} total={250} />);
    expect(html).toContain('aria-rowcount="251"');
  });

  it('keeps every row exactly one height: each cell clips its content to a fixed-height block', () => {
    const worst = row({
      status: 'unmet',
      unmet: 'coverage gap, not a finding: never satisfied in 4,790 claimable trials',
      failures_seen_in_inconclusive_trials: 3,
      rate: { kind: 'zero_events', basis: 'TrialsTrue', n: 4790, upper_bound: 0.000626 },
      guest: { id: 'i'.repeat(96), msg: 'm'.repeat(140), loc: { file: 'f'.repeat(80), line: 1, func: 'g'.repeat(80) } },
    });
    const html = renderToString(<PropertyTable properties={[worst]} />);
    const body = html.slice(html.indexOf('<tbody'));
    const clips = body.match(/<div class="cell-clip" style="height:(\d+)px">/g) ?? [];
    expect(clips).toHaveLength(6);
    for (const c of clips) expect(Number(/height:(\d+)px/.exec(c)?.[1])).toBe(CLIP_HEIGHT);
    expect(body).toContain(`<tr aria-rowindex="2" style="height:${ROW_HEIGHT}px">`);
    expect(CLIP_HEIGHT).toBeLessThan(ROW_HEIGHT);
    expect(body).toContain('clip-line');
  });

  it('sets each fixed column to its width, leaves one to take the rest, and the table minimum to the budget so it scrolls instead of squeezing', () => {
    const html = renderToString(<PropertyTable properties={[row()]} />);
    const cols = [...html.matchAll(/<col(?: style="width:(\d+)px")?\/?>/g)].map((m) => (m[1] === undefined ? undefined : Number(m[1])));
    expect(cols).toEqual(COLUMN_KEYS.map((k) => COLUMN_WIDTHS_PX[k]));
    expect(cols.filter((w) => w === undefined)).toHaveLength(1);
    expect(html).toContain(`min-width:${columnBudgetPx()}px`);
  });

  it('sizes its rows for the reader\'s root font size, so a larger default does not clip the stacked rate cell', () => {
    const worst = row({ status: 'unmet', failures_seen_in_inconclusive_trials: 3, unmet: 'x', rate: { kind: 'zero_events', basis: 'TrialsTrue', n: 4790, upper_bound: 0.0006 } });
    const clipAt = (rootPx: number) => {
      const html = renderToString(
        <table>
          <tbody>
            <PropertyRowView row={worst} rowIndex={2} rowHeight={rowHeightPx(rootPx)} />
          </tbody>
        </table>,
      );
      return Number(/class="cell-clip" style="height:(\d+)px"/.exec(html)?.[1]);
    };
    expect(clipAt(16)).toBe(CLIP_HEIGHT);
    expect(clipAt(20)).toBe(rowHeightPx(20) - ROW_CHROME_PX);
    expect(clipAt(20)).toBeGreaterThan(clipAt(16));
  });

  it('tells a narrow screen that the table scrolls sideways', () => {
    expect(renderToString(<PropertyTable properties={[row()]} />)).toContain('class="scroll-hint"');
  });

  it('on a run stopped at its first failure, every row keeps its counts in the unclamped figure', () => {
    const rows = [
      row({ rate: { kind: 'not_a_rate', basis: 'TrialsFalse', k: 1, n: 3 } }),
      row({ idx: 1, status: 'unrated', rate: { kind: 'unrated' }, guest: { id: 'b', msg: '', loc: null } }),
      row({ idx: 2, status: 'pass', rate: { kind: 'not_a_rate', basis: 'TrialsFalse', k: 0, n: 3 }, guest: { id: 'c', msg: '', loc: null } }),
    ];
    const html = renderToString(<PropertyTable properties={rows} />);
    expect(html).toContain('<span class="rate-figure">1 failure in 3 rated trials</span>');
    expect(html).toContain('<span class="rate-figure">0 failures in 3 rated trials</span>');
    expect(html).toContain('<span class="rate-figure">unrated (0 rated trials)</span>');
  });

  it('says how many characters of a long id were cut', () => {
    const html = renderToString(<PropertyTable properties={[row({ guest: { id: 'i'.repeat(200), msg: '', loc: null } })]} />);
    expect(html).toContain('… (104 more characters)');
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

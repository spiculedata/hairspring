import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { header, page, row } from '../explorer/fixtures';
import type { RunCompletion, RunDetail } from '../types';
import { RunPanel } from './RunPanel';

const text = (html: string): string => html.replace(/<[^>]+>/g, '');

function detail(over: Partial<RunDetail['run']> = {}, rest: Partial<RunDetail> = {}): RunDetail {
  return {
    run: {
      id: 'r1',
      target: 't',
      backend: 'process',
      state: 'passed',
      seed: 1,
      created_at: 0,
      started_at: 1,
      finished_at: 2,
      steps: 10,
      trials: 30,
      error: null,
      mode: 'search',
      ...over,
    },
    failures: [],
    ...rest,
  };
}
const withCompletion = (completion: RunCompletion, label = 'x') => page({ header: header({ completion, completion_label: label }) });

describe('RunPanel', () => {
  it('has the two headings and the outcome badge', () => {
    const html = renderToString(<RunPanel detail={detail()} page={withCompletion({ kind: 'complete' })} />);
    expect(text(html)).toContain('What this run shows');
    expect(text(html)).toContain('What it does not show');
    expect(html).toContain('badge');
    expect(text(html)).toContain('No failure seen');
  });

  it.each<[string, RunCompletion]>([
    ['running', { kind: 'running' }],
    ['complete', { kind: 'complete' }],
    ['stopped_at_first_failure', { kind: 'stopped_at_first_failure' }],
    ['time_or_step_limited', { kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 }],
    ['ended_early', { kind: 'ended_early' }],
    ['digest_allowance_used_up', { kind: 'digest_allowance_used_up' }],
    ['aborted', { kind: 'aborted' }],
  ])('renders a non-empty list on each side for completion %s', (_n, completion) => {
    const html = renderToString(<RunPanel detail={detail()} page={withCompletion(completion)} />);
    const lists = html.match(/<li/g) ?? [];
    expect(lists.length).toBeGreaterThanOrEqual(2);
  });

  it('renders a failure with no sweep summary', () => {
    const d = detail({ state: 'failed', trials: 4 }, { properties: { first_failing_seed: '1016', stopped_at_first_failure: true }, failures: [] });
    const html = renderToString(<RunPanel detail={d} page={withCompletion({ kind: 'stopped_at_first_failure' })} />);
    expect(text(html)).toContain('A failure: seed 1016 failed.');
    expect(text(html)).toContain('no rate is stated here');
  });

  it('renders the sweep rate with the interval from the failed property row', () => {
    const d = detail(
      { state: 'failed', trials: 100, mode: 'search' },
      { sweep: { trials: 100, passes: 97, violations: 3, inconclusive: 0, unusable: 0, infra: 0, first_failing_seeds: ['1016'] } },
    );
    const html = renderToString(<RunPanel detail={d} page={page({ properties: [row()] })} />);
    expect(text(html)).toContain('Rate for the trials run: 3/4790 · 0.06% · [0.02%–0.18%]');
  });

  it('prints the single rate line only when the page holds the one failed property in full', () => {
    const d = detail(
      { state: 'failed', trials: 100, mode: 'search' },
      { sweep: { trials: 100, passes: 97, violations: 3, inconclusive: 0, unusable: 0, infra: 0, first_failing_seeds: ['1016'] } },
    );
    const more = renderToString(<RunPanel detail={d} page={page({ next_cursor: 'c2' })} />);
    expect(text(more)).not.toContain('Rate for the trials run');
    const filtered = renderToString(<RunPanel detail={d} page={page({ counts_by_status: { pass: 3 }, properties: [row({ status: 'pass' })] })} />);
    expect(text(filtered)).not.toContain('Rate for the trials run');
    const two = renderToString(<RunPanel detail={d} page={page({ counts_by_status: { violation: 2 }, properties: [row(), row({ idx: 1 })] })} />);
    expect(text(two)).toContain('2 properties failed; any rate is in the property table.');
  });

  it('renders zero trials as nothing run, with a neutral badge', () => {
    const html = renderToString(<RunPanel detail={detail({ trials: 0 })} page={withCompletion({ kind: 'complete' })} />);
    expect(text(html)).toContain('No trials were run.');
    expect(html).not.toContain('badge-good');
  });

  it('renders before the property page has loaded without a pass-looking claim', () => {
    const html = renderToString(<RunPanel detail={detail()} page={undefined} />);
    expect(html).not.toContain('badge-good');
    expect(text(html)).not.toContain('none of them failed');
    expect(text(html)).toContain('How the run ended: its completion has not loaded.');
  });

  it('escapes nothing from the program under test: no property ids or messages are shown', () => {
    const r = row({ guest: { id: 'secret.id', msg: '<b>boom</b>', loc: null } });
    const d = detail({ state: 'failed', mode: 'search', trials: 100 }, { sweep: { trials: 100, passes: 97, violations: 3, inconclusive: 0, unusable: 0, infra: 0, first_failing_seeds: [] } });
    const html = renderToString(<RunPanel detail={d} page={page({ properties: [r] })} />);
    expect(html).not.toContain('secret.id');
    expect(html).not.toContain('boom');
  });
});

import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { assertTextOnly, CORPUS } from '../guest/hostile';
import { row } from '../explorer/fixtures';
import { failingRow, manyRows, seedRow } from './fixtures';
import { SeedExplorerView, type SeedExplorerViewProps } from './SeedExplorerView';
import { appendStripPage, EMPTY_STRIP } from './strip';

const noop = () => undefined;
const stripOf = (codes: string) => appendStripPage(EMPTY_STRIP, { first_seq: 0, codes });

/** What a test overrides: the data of the strip and the trials, whose `reload` the helper supplies. */
type Over = Omit<Partial<SeedExplorerViewProps>, 'strip' | 'trials'> & {
  strip?: Omit<SeedExplorerViewProps['strip'], 'reload'>;
  trials?: Omit<SeedExplorerViewProps['trials'], 'reload'>;
};

function props({ strip, trials, ...over }: Over = {}): SeedExplorerViewProps {
  return {
    runId: 'run_0123',
    target: 'wal-reset',
    context: {
      stoppedAtFirstFailure: false,
      completionLabel: 'complete',
      excluded: { degraded: 1, perturbing: 0, inconclusive: 0, report_missing: 0 },
    },
    firstFailingSeed: '1016',
    live: false,
    strip: { strip: stripOf('ppvpep'), loading: false, error: undefined, reload: noop, ...strip },
    filters: {},
    trials: {
      rows: [seedRow({ seq: 0, seed: '1000' }), failingRow({ seq: 2 })],
      cursor: null,
      loaded: true,
      loadingMore: false,
      error: undefined,
      reload: noop,
      ...trials,
    },
    filterBar: {
      propertyLabel: undefined,
      search: { text: '', matches: [], state: 'idle' },
      fpText: '',
      onSearch: noop,
      onFpText: noop,
    },
    onFilters: noop,
    onOpen: noop,
    onLoadMore: noop,
    ...over,
  };
}
const render = (over: Over = {}) => renderToString(<SeedExplorerView {...props(over)} />);
const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('SeedExplorerView', () => {
  it('shows the strip, the filters and the table', () => {
    const html = render();
    expect(html).toContain('role="slider"');
    expect(html).toContain('class="seed-filters"');
    expect(html).toContain('<table class="seeds"');
    expect(textOf(html)).toContain('wal-reset');
  });

  it('links the lowest failing seed to its failure page', () => {
    expect(render()).toContain('<a href="/runs/run_0123/seeds/1016">1016</a>');
  });

  it('shows the stopped-at-first-failure note, naming the last trial judged', () => {
    const text = textOf(render({ context: { stoppedAtFirstFailure: true, completionLabel: 'stopped at first failure', excluded: props().context!.excluded } }));
    expect(text).toContain('This run stopped at its first failure');
    expect(text).toContain('after trial 5');
    expect(text).toContain('not a rate');
  });

  it('shows no stopped note for a run that did not stop at a failure', () => {
    expect(textOf(render())).not.toContain('stopped at its first failure');
  });

  it('lists what was excluded from every rate, so excluded trials are visible next to the list', () => {
    expect(textOf(render())).toContain('Excluded from every rate: 1 degraded');
  });

  it('says the run is live and that the list grows', () => {
    expect(textOf(render({ live: true }))).toContain('The run is still going: 6 trials judged so far');
  });

  it('says so when no trial matches the filters', () => {
    const text = textOf(render({ filters: { verdict: 'violation' }, trials: { rows: [], cursor: null, loaded: true, loadingMore: false, error: undefined } }));
    expect(text).toContain('No judged trial matches these filters.');
    expect(text).not.toContain('<table');
  });

  it('says a run with no stored rows has none, rather than that nothing matched', () => {
    const text = textOf(render({ strip: { strip: EMPTY_STRIP, loading: false, error: undefined }, trials: { rows: [], cursor: null, loaded: true, loadingMore: false, error: undefined } }));
    expect(text).toContain('This run has no stored trial rows.');
  });

  it('keeps reading further when a bounded read found nothing yet, and says how far it got', () => {
    const html = render({ filters: { fails: 3 }, trials: { rows: [], cursor: '19999', loaded: true, loadingMore: false, error: undefined } });
    const text = textOf(html);
    expect(text).toContain('No match yet in the trials read so far (up to trial 19,999)');
    expect(text).toContain('Read further (after trial 19,999)');
    expect(text).not.toContain('No judged trial matches');
  });

  it('offers to load more rows when the server has more, with the shown and total counts when the total is known', () => {
    const strip = { strip: stripOf('p'.repeat(300)), loading: false, error: undefined };
    const text = textOf(render({ strip, trials: { rows: manyRows(200), cursor: '199', loaded: true, loadingMore: false, error: undefined } }));
    expect(text).toContain('Load more (200 of 300 shown)');
  });

  it('knows the size of an unfiltered list and of one that starts at a trial, and not of a filtered one', () => {
    const strip = { strip: stripOf('p'.repeat(300)), loading: false, error: undefined };
    const rows = { rows: manyRows(3), cursor: null, loaded: true, loadingMore: false, error: undefined };
    expect(render({ strip, trials: rows })).toContain('aria-rowcount="301"');
    expect(render({ strip, trials: rows, filters: { from: 100 } })).toContain('aria-rowcount="201"');
    expect(render({ strip, trials: rows, filters: { verdict: 'pass' } })).toContain('aria-rowcount="-1"');
    expect(render({ strip: { ...strip, loading: true }, trials: rows })).toContain('aria-rowcount="-1"');
  });

  it('shows loading and error states without hiding the strip', () => {
    expect(textOf(render({ trials: { rows: [], cursor: null, loaded: false, loadingMore: false, error: undefined } }))).toContain('Loading trials…');
    const html = render({ trials: { rows: [], cursor: null, loaded: false, loadingMore: false, error: 'the server is down' } });
    expect(html).toContain('role="alert"');
    expect(html).toContain('role="slider"');
  });

  it('never shows a degraded pass or an inconclusive trial as a pass, in the table or the counts', () => {
    const rows = [
      seedRow({ seq: 0, class: 'excluded', degraded: true, rated: false }),
      seedRow({ seq: 1, verdict: 'inconclusive', class: 'inconclusive' }),
    ];
    const html = render({ strip: { strip: stripOf('ei'), loading: false, error: undefined }, trials: { rows, cursor: null, loaded: true, loadingMore: false, error: undefined } });
    const tableBody = html.slice(html.indexOf('<tbody', html.indexOf('<table class="seeds"')));
    const counts = html.slice(html.indexOf('strip-counts'), html.indexOf('</table>', html.indexOf('strip-counts')));
    for (const part of [tableBody, counts]) expect(textOf(part)).not.toMatch(/✓|\bpass\b/);
    expect(html).not.toMatch(/class-p\b|class-pass/);
  });
});

describe.each(CORPUS)('XSS corpus: %s', (_name, hostile) => {
  it('shows hostile guest text in every field of the whole page as escaped text only', () => {
    const rows = [
      failingRow({ seq: 0, guest: { reason: hostile, failing_ids: [hostile, hostile, hostile] }, failing_count: 5, failing_idx: [0, 1, 2] }),
      seedRow({ seq: 1, schedule_fp: hostile, seed: hostile }),
    ];
    const matches = [row({ idx: 1, guest: { id: hostile, msg: hostile, loc: { file: hostile, line: 1, func: hostile } } })];
    const html = render({
      filters: { fails: 1, verdict: 'violation' },
      target: hostile,
      firstFailingSeed: hostile,
      trials: { rows, cursor: '9', loaded: true, loadingMore: false, error: hostile },
      strip: { strip: stripOf('pv'), loading: false, error: hostile },
      filterBar: { propertyLabel: hostile, search: { text: 'x', matches, state: 'idle' }, fpText: '', onSearch: noop, onFpText: noop },
    });
    assertTextOnly(html);
    expect(html.length).toBeLessThan(120_000);
  });
});

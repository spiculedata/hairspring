// @vitest-environment happy-dom
import axe from 'axe-core';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { App } from './App';
import { ActivityLog } from './components/ActivityLog';
import { page } from './explorer/fixtures';
import { Term } from './glossary/Term';
import { navigate } from './router';
import { failingRow, seedRow } from './seeds/fixtures';
import { SeedExplorerView } from './seeds/SeedExplorerView';
import { appendStripPage, EMPTY_STRIP } from './seeds/strip';
import { OutputViewer } from './trial/OutputViewer';
import type { FailureDetail, FailureSummary, Run } from './types';

/**
 * An automated axe-core pass over the main pages as the app renders them. happy-dom has no layout or painting, so only
 * the rules that read the markup are run here (names, roles, headings, landmarks, tables, ids). Colour contrast and
 * target size are not checked by this suite; `contrast.test.ts` checks the colour pairs against their tokens.
 */
const MARKUP_RULES = [
  'aria-allowed-attr',
  'aria-allowed-role',
  'aria-hidden-focus',
  'aria-prohibited-attr',
  'aria-required-attr',
  'aria-required-children',
  'aria-required-parent',
  'aria-roles',
  'aria-valid-attr',
  'aria-valid-attr-value',
  'button-name',
  'duplicate-id-aria',
  'empty-heading',
  'heading-order',
  'label',
  'landmark-no-duplicate-banner',
  'landmark-no-duplicate-main',
  'landmark-one-main',
  'landmark-unique',
  'link-name',
  'list',
  'listitem',
  'nested-interactive',
  'page-has-heading-one',
  'scope-attr-valid',
  'select-name',
  'table-duplicate-name',
  'td-headers-attr',
  'th-has-data-cells',
];

/**
 * A name on an element with no role is ignored by assistive technology, and axe only reports it as certain when the
 * element has no text, so a box with text (a log, a bundle) is checked here: a div, span, pre or p that has an
 * `aria-label` needs a `role`.
 */
function nameWithoutRole(): string[] {
  return [...document.querySelectorAll('div[aria-label], span[aria-label], pre[aria-label], p[aria-label]')]
    .filter((el) => !el.hasAttribute('role'))
    .map((el) => `name-without-role: ${el.tagName.toLowerCase()}[aria-label="${el.getAttribute('aria-label')}"]`);
}

async function violations(): Promise<string[]> {
  const result = await axe.run(document, { runOnly: { type: 'rule', values: MARKUP_RULES } });
  return [...result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`), ...nameWithoutRole()];
}

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const pending = () => new Promise<Response>(() => undefined);

const run: Run = {
  id: 'run_1',
  target: 'lock-lease',
  backend: 'sim',
  state: 'failed',
  seed: 7,
  created_at: 1,
  started_at: 1,
  finished_at: 2,
  steps: 10,
  trials: 4,
  error: null,
};
const failure: FailureSummary = {
  id: 'f_1',
  run_id: 'run_1',
  target: 'lock-lease',
  message: 'lease handed out twice',
  original_len: 23,
  shrunk_len: 8,
  bundle_hash: 'cd'.repeat(32),
  created_at: 2,
};
const failureDetail: FailureDetail = { record: failure, steps: [], bundle: 'bundle text' };

function serve() {
  return vi.fn(async (url: string) => {
    const u = String(url);
    if (u.includes('/events')) return pending();
    if (u.includes('/runs/run_missing')) return json({ error: 'not found' }, 404);
    if (u.startsWith('/api/failures/f_missing')) return json({ error: 'not found' }, 404);
    if (u.startsWith('/api/runs/run_1/trials?seed=')) return json({ trials: [] });
    if (u.includes('/properties')) return json(page());
    if (u.includes('/coverage') || u.includes('/checks')) return json({}, 404);
    if (u.startsWith('/api/runs/run_1')) return json({ run, failures: [failure] });
    if (u.startsWith('/api/runs')) return json({ runs: [run] });
    if (u.startsWith('/api/failures/f_1')) return json(failureDetail);
    if (u.startsWith('/api/failures')) return json({ failures: [failure] });
    if (u.startsWith('/api/targets')) return json({ targets: [{ name: 'lock-lease', kind: 'sim', description: 'A lock service' }] });
    if (u.startsWith('/api/workers')) {
      return json({ workers: [{ name: 'remote-1', last_seen: 2, registered_at: 1, state: 'idle', trusted: false }], queue: { queued: 0, running: 0 } });
    }
    return json({}, 404);
  });
}

const mount = (path: string) => {
  window.history.replaceState({}, '', path);
  return render(<App />);
};

describe('axe-core over the main pages', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', serve());
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    document.title = '';
  });

  const settled = async (heading: string | RegExp) => {
    await screen.findByRole('heading', { level: 1, name: heading });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30));
    });
  };

  it.each([
    ['/', 'Overview'],
    ['/runs', 'Runs'],
    ['/failures', 'Failures'],
    ['/targets', 'Targets'],
    ['/workers', 'Workers'],
    ['/glossary', 'Glossary'],
  ])('%s has no markup violations', async (path, heading) => {
    mount(path);
    await settled(heading);
    expect(await violations()).toEqual([]);
  });

  it.each(['', '?tab=seeds', '?tab=failures', '?tab=activity'])('a run page %s has no markup violations', async (search) => {
    mount(`/runs/run_1${search}`);
    await settled(/lock-lease/);
    expect(await violations()).toEqual([]);
  });

  it('a failure page has no markup violations', async () => {
    mount('/failures/f_1');
    await settled(/shortest failing steps/);
    expect(await violations()).toEqual([]);
  });

  it('a term definition, open, adds no markup violation to its page', async () => {
    document.body.innerHTML = '';
    render(
      <main>
        <h1>Failure</h1>
        <Term term="confirmation" heading={{ id: 'confirmation-heading' }} />
        <p>
          <Term term="bundle" />
        </p>
      </main>,
    );
    for (const button of screen.getAllByRole('button')) fireEvent.click(button);
    expect(screen.getAllByRole('group')).toHaveLength(2);
    expect(await violations()).toEqual([]);
  });

  /**
   * Every page a reader can reach for something that is not there. Each row is one branch in the app: the route that
   * matches nothing, a run id the router refuses, a trial number too large to be one, and each page whose server
   * answer is 404 (run, seeds of a run, failure, trial, and the run behind a seed). A row for a branch added later
   * belongs here too; `NotFound` and the no-stored-trial note in `SeedPage` are the only places such a page is built.
   */
  it.each([
    ['/nowhere', 'No such page'],
    ['/runs/bad.id/trials/1', 'No such page'],
    ['/runs/run_1/trials/99999999999999999999', 'No such page'],
    ['/runs/run_missing', 'No such run'],
    ['/runs/run_missing/seeds', 'No such run'],
    ['/runs/run_missing/seeds/5', 'No such run'],
    ['/runs/run_missing/trials/1', 'No such trial'],
    ['/failures/f_missing', 'No such failure'],
    ['/runs/run_1/seeds/999999', 'No stored trial'],
  ])('%s has a level one heading and no other markup violations', async (path, heading) => {
    mount(path);
    await settled(heading);
    expect(await violations()).toEqual([]);
  });

  it('keeps the page free of violations after moving to another route', async () => {
    mount('/runs');
    await settled('Runs');
    act(() => navigate('/nowhere'));
    await settled('No such page');
    expect(await violations()).toEqual([]);
    await waitFor(() => expect(document.title).toBe('No such page · Hairspring'));
  });
});

describe('axe-core over parts of pages that need data a route does not give here', () => {
  afterEach(cleanup);

  it('the activity log with a raw event open', async () => {
    document.body.innerHTML = '';
    const { container } = render(
      <main>
        <h1>Run</h1>
        <ActivityLog lines={[{ id: '1', event: 'started', data: '{"worker":"w1"}' }]} />
      </main>,
    );
    container.querySelectorAll('details').forEach((d) => d.setAttribute('open', ''));
    expect(await violations()).toEqual([]);
  });

  it('the output viewer', async () => {
    document.body.innerHTML = '';
    render(
      <main>
        <h1>Trial</h1>
        <OutputViewer
          streams={['stdout', 'stderr']}
          tails={{ stdout: { status: 'ready', text: 'one\ntwo\n' }, stderr: { status: 'ready', text: 'warning\n' } }}
          omittedBytes={{ stdout: 0, stderr: 0 }}
          onDownload={() => undefined}
        />
      </main>,
    );
    expect(await violations()).toEqual([]);
  });

  it('the seed explorer, whose strip and table are both named landmarks that must not share a name', async () => {
    const noop = () => undefined;
    const html = renderToString(
      <SeedExplorerView
        runId="run_0123"
        target="wal-reset"
        context={{ stoppedAtFirstFailure: false, completionLabel: 'complete', excluded: { degraded: 0, perturbing: 0, inconclusive: 0, report_missing: 0 } }}
        firstFailingSeed="1016"
        live={false}
        strip={{ strip: appendStripPage(EMPTY_STRIP, { first_seq: 0, codes: 'ppvpep' }), loading: false, error: undefined, reload: noop }}
        filters={{}}
        trials={{ rows: [seedRow({ seq: 0, seed: '1000' }), failingRow({ seq: 2 })], cursor: null, loaded: true, loadingMore: false, error: undefined, reload: noop }}
        filterBar={{ propertyLabel: undefined, search: { text: '', matches: [], state: 'idle' }, fpText: '', onSearch: noop, onFpText: noop }}
        onFilters={noop}
        onOpen={noop}
        onLoadMore={noop}
      />,
    );
    document.body.innerHTML = `<main><h1>Seeds</h1>${html}</main>`;
    expect(await violations()).toEqual([]);
  });
});

describe('the markup rules do catch the defects this suite was added for', () => {
  afterEach(() => {
    cleanup();
  });

  it('reports a skipped heading level and a name on a generic element', async () => {
    document.body.innerHTML = '<main><h1>Page</h1><h3>Skipped</h3><pre tabindex="0" aria-label="Raw"></pre></main>';
    const found = (await violations()).map((v) => v.split(':')[0]);
    expect(found).toEqual(expect.arrayContaining(['heading-order', 'aria-prohibited-attr']));
  });

  it('reports a page with no level one heading', async () => {
    document.body.innerHTML = '<main><h2>Section</h2></main>';
    expect((await violations()).map((v) => v.split(':')[0])).toContain('page-has-heading-one');
  });
});

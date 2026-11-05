// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SearchBox } from './SearchBox';
import type { SearchResults } from './searchApi';
import { SEARCH_DEBOUNCE_MS } from './useSearchResults';
import { resetSearchProbe } from './useSearchAvailable';

const RUN = 'run_8f3a9c0d11b2e4a7';
const NEWER = 'run_0000000000000002';
const EMPTY: SearchResults = {
  runs: [],
  failures: [],
  targets: [],
  properties: [],
  seeds: [],
  schedules: [],
  more: [],
  scope: { per_group: 5, recent_runs: 1000, property_runs: 100 },
};
const WITH_HITS: SearchResults = {
  ...EMPTY,
  runs: [
    { id: RUN, target: 'lock-lease', state: 'failed' },
    { id: NEWER, target: 'lock-lease', state: 'passed' },
  ],
  targets: [{ name: 'lock-lease' }],
};

type Handler = (url: string) => Response | Promise<Response>;
const json = (status: number, body: unknown, headers?: Record<string, string>) => new Response(JSON.stringify(body), { status, headers });

/** Answers the probe (the lone-space text) with a 400 like the real server, and every other search with `search`. */
function server(search: Handler = () => json(200, EMPTY)) {
  const calls: string[] = [];
  const fetchMock = vi.fn((url: string) => {
    calls.push(url);
    if (url === '/api/search?q=%20') return Promise.resolve(json(400, { error: '`q` is not valid' }));
    return Promise.resolve(search(url));
  });
  vi.stubGlobal('fetch', fetchMock);
  return { calls, searches: () => calls.filter((u) => u !== '/api/search?q=%20') };
}

const settle = () => act(async () => { await vi.advanceTimersByTimeAsync(0); });
const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

async function open() {
  const view = render(<SearchBox />);
  await settle();
  return view;
}

const box = () => screen.getByRole('combobox', { name: 'Search ids and names' }) as HTMLInputElement;
const type = (text: string) => fireEvent.change(box(), { target: { value: text } });

beforeEach(() => {
  vi.useFakeTimers();
  resetSearchProbe();
  window.history.replaceState({}, '', '/');
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('whether the box is shown', () => {
  it('shows nothing until the server has said it has a search endpoint', async () => {
    server();
    const { container } = render(<SearchBox />);
    expect(container.innerHTML).toBe('');
    await settle();
    expect(box()).toBeTruthy();
  });

  it('is hidden for a server that answers 404, which is an older release, and asks only once however often it is drawn', async () => {
    const fetchMock = vi.fn().mockResolvedValue(json(404, { error: 'not found' }));
    vi.stubGlobal('fetch', fetchMock);
    const first = render(<SearchBox />);
    await settle();
    expect(first.container.innerHTML).toBe('');
    first.unmount();
    const second = render(<SearchBox />);
    await settle();
    expect(second.container.innerHTML).toBe('');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('is shown when the server needs a token or is busy: only a 404 means the endpoint is absent', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(json(401, { error: 'a valid bearer token is required' })));
    await open();
    expect(box()).toBeTruthy();
  });

  it('disappears when a search gets a 404 after all', async () => {
    server(() => json(404, { error: 'not found' }));
    const { container } = await open();
    type('run_');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(container.innerHTML).toBe('');
  });
});

describe('the control', () => {
  it('is a labelled search region with an editable combobox, closed until there is something to show', async () => {
    server();
    const { container } = await open();
    expect(container.querySelector('[role="search"]')).toBeTruthy();
    expect(box().getAttribute('aria-expanded')).toBe('false');
    expect(box().getAttribute('aria-autocomplete')).toBe('list');
    expect(box().autocomplete).toBe('off');
    expect(screen.queryByRole('listbox')).toBeNull();
  });
});

describe('asking', () => {
  it('waits for typing to pause, then sends one search for the last text', async () => {
    const s = server();
    await open();
    type('ru');
    await wait(100);
    type('run');
    await wait(100);
    type('run_8');
    await wait(SEARCH_DEBOUNCE_MS - 1);
    expect(s.searches()).toEqual([]);
    await wait(1);
    expect(s.searches()).toEqual(['/api/search?q=run_8']);
  });

  it('sends nothing for text it can say is no use, and says why', async () => {
    const s = server();
    await open();
    type('r');
    await wait(1000);
    expect(screen.getAllByText('Type at least 2 characters.').length).toBeGreaterThan(0);
    type('a b');
    await wait(1000);
    expect(screen.getAllByText('Use letters, digits and these marks only: _ - . : /').length).toBeGreaterThan(0);
    type('a'.repeat(65));
    await wait(1000);
    expect(screen.getAllByText('Use at most 64 characters.').length).toBeGreaterThan(0);
    type("x' OR 1=1 --");
    await wait(1000);
    expect(s.searches()).toEqual([]);
  });

  it('trims the spaces round a pasted id', async () => {
    const s = server();
    await open();
    type(`  ${RUN} `);
    await wait(SEARCH_DEBOUNCE_MS);
    expect(s.searches()).toEqual([`/api/search?q=${RUN}`]);
  });

  it('drops the answer to text that is no longer typed, however late it arrives', async () => {
    let releaseFirst: (r: Response) => void = () => {};
    const s = server((url) => (url.endsWith('q=ru') ? new Promise<Response>((r) => (releaseFirst = r)) : json(200, { ...EMPTY, targets: [{ name: 'run-target' }] })));
    await open();
    type('ru');
    await wait(SEARCH_DEBOUNCE_MS);
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    releaseFirst(json(200, { ...EMPTY, targets: [{ name: 'ru-old' }] }));
    await settle();
    expect(s.searches()).toEqual(['/api/search?q=ru', '/api/search?q=run']);
    expect(screen.queryByText('ru-old')).toBeNull();
    expect(screen.getByText('run-target')).toBeTruthy();
  });

  it('says "Searching" while it waits, and keeps the last answer on screen meanwhile', async () => {
    server((url) => (url.endsWith('q=ru') ? json(200, WITH_HITS) : new Promise<Response>(() => {})));
    await open();
    type('ru');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByRole('option')).toHaveLength(3);
    type('run');
    expect(screen.getAllByRole('option')).toHaveLength(3);
    expect(screen.getByRole('listbox').getAttribute('aria-busy')).toBe('true');
    expect(screen.getByRole('status').textContent).toBe('Searching…');
  });
});

describe('the answer', () => {
  it('lists hits in labelled groups as options of one listbox, with how many in a status line', async () => {
    server(() => json(200, WITH_HITS));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    const list = screen.getByRole('listbox', { name: 'Search results' });
    expect(box().getAttribute('aria-expanded')).toBe('true');
    expect(box().getAttribute('aria-controls')).toBe(list.id);
    expect(screen.getByRole('group', { name: 'Runs' })).toBeTruthy();
    expect(screen.getByRole('group', { name: 'Targets' })).toBeTruthy();
    expect(screen.getAllByRole('group')).toHaveLength(2);
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual([`${RUN}lock-lease`, `${NEWER}lock-lease`, 'lock-leaseRuns of this target']);
    expect(screen.getByRole('status').textContent).toBe('3 matches.');
  });

  it('says "No matches." plainly, and how far it looked, so it is not read as "never stored"', async () => {
    server(() => json(200, EMPTY));
    await open();
    type('zzzz');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByText('No matches.').length).toBeGreaterThan(0);
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(box().getAttribute('aria-expanded')).toBe('false');
    const text = document.body.textContent ?? '';
    expect(text).toContain('1000 newest runs');
    expect(text).toContain('whole run or failure id of any age');
  });

  it('marks a group with more hits than were listed, and says how to narrow it', async () => {
    server(() => json(200, { ...WITH_HITS, more: ['runs'] }));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getByText('(more)')).toBeTruthy();
    expect(document.body.textContent).toContain('Type more of the id to narrow it.');
  });

  it('shows what the server says went wrong, and a rate limit with when to try again', async () => {
    server(() => json(500, { error: 'internal error' }));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByText('Search failed: internal error').length).toBeGreaterThan(0);
  });

  it('stops sending after a 429 until the time the server gave has passed', async () => {
    const s = server(() => json(429, { error: 'rate limit exceeded' }, { 'Retry-After': '30' }));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByText('Too many searches just now. Try again in 30 seconds.').length).toBeGreaterThan(0);
    type('run_');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(s.searches()).toHaveLength(1);
    expect(screen.getAllByText(/Too many searches just now/).length).toBeGreaterThan(0);
    await wait(30_000);
    expect(s.searches()).toHaveLength(2);
  });

  it('clears the rate-limit message once the wait has passed, and searches the same text again', async () => {
    let calls = 0;
    const s = server(() => (++calls === 1 ? json(429, { error: 'rate limit exceeded' }, { 'Retry-After': '30' }) : json(200, WITH_HITS)));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByText(/Too many searches just now/).length).toBeGreaterThan(0);
    await wait(29_000);
    expect(screen.getAllByText(/Too many searches just now/).length).toBeGreaterThan(0);
    await wait(1_000);
    expect(screen.queryAllByText(/Too many searches just now/)).toHaveLength(0);
    await wait(2 * SEARCH_DEBOUNCE_MS);
    expect(screen.queryAllByText(/Too many searches just now/)).toHaveLength(0);
    expect(s.searches()).toHaveLength(2);
    expect(screen.getAllByRole('option')).toHaveLength(3);
  });

  it('sends text typed during a 429 once when the wait ends, and does not send it again', async () => {
    let calls = 0;
    const s = server(() => (++calls === 1 ? json(429, { error: 'rate limit exceeded' }, { 'Retry-After': '30' }) : json(200, WITH_HITS)));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    type('run_');
    await wait(30_000);
    await wait(1_000);
    expect(s.searches()).toHaveLength(2);
    expect(screen.getAllByRole('option')).toHaveLength(3);
  });

  it('does not search again after a 429 if the text was cleared before the wait ended', async () => {
    const s = server(() => json(429, { error: 'rate limit exceeded' }, { 'Retry-After': '30' }));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    type('');
    await wait(60_000);
    expect(s.searches()).toHaveLength(1);
  });

  it('says the search is unavailable until the connection returns when the request never reached the server', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string) => (url === '/api/search?q=%20' ? Promise.resolve(json(400, { error: 'bad' })) : Promise.reject(new TypeError('Failed to fetch')))),
    );
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByText('Search is unavailable until the connection returns.').length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toContain('Failed to fetch');
  });

  it('says "Some results could not be shown", not "No matches.", when every hit has an id the page cannot link to', async () => {
    server(() => json(200, { ...EMPTY, runs: [{ id: 'run.1:a', target: 't', state: 'failed' }] }));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByText('Some results could not be shown.').length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toContain('No matches.');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('keeps the hits it can show and says that some could not be', async () => {
    server(() => json(200, { ...EMPTY, runs: [{ id: 'run.1:a', target: 't', state: 'failed' }, { id: RUN, target: 't', state: 'failed' }] }));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(screen.getAllByRole('option')).toHaveLength(1);
    expect(document.body.textContent).toContain('Some results could not be shown.');
    expect(screen.getByRole('status').textContent).toBe('1 match. Some results could not be shown.');
  });
});

describe('the keyboard', () => {
  async function withHits() {
    server(() => json(200, WITH_HITS));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
  }

  it('moves through the hits with Down and Up, naming the active one for a screen reader without moving focus', async () => {
    await withHits();
    const options = screen.getAllByRole('option');
    expect(options.map((o) => o.getAttribute('aria-selected'))).toEqual(['false', 'false', 'false']);
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    expect(options[0].getAttribute('aria-selected')).toBe('true');
    expect(box().getAttribute('aria-activedescendant')).toBe(options[0].id);
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    expect(options[2].getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    expect(options[0].getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(box(), { key: 'ArrowUp' });
    expect(options[2].getAttribute('aria-selected')).toBe('true');
  });

  it('opens the active hit on Enter, or the first when none is active, and clears the box', async () => {
    await withHits();
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(window.location.pathname).toBe(`/runs/${NEWER}`);
    expect(box().value).toBe('');
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('opens the first hit when Enter is pressed with none active', async () => {
    await withHits();
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(window.location.pathname).toBe(`/runs/${RUN}`);
  });

  it('closes the list on Escape, keeping the text, and clears the text on a second Escape', async () => {
    await withHits();
    fireEvent.keyDown(box(), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(box().getAttribute('aria-expanded')).toBe('false');
    expect(box().value).toBe('run');
    fireEvent.keyDown(box(), { key: 'Escape' });
    expect(box().value).toBe('');
  });

  it('opens the list again on Down, and when the box is focused again', async () => {
    await withHits();
    fireEvent.keyDown(box(), { key: 'Escape' });
    fireEvent.keyDown(box(), { key: 'ArrowDown' });
    expect(screen.getByRole('listbox')).toBeTruthy();
    fireEvent.keyDown(box(), { key: 'Escape' });
    fireEvent.focus(box());
    expect(screen.getByRole('listbox')).toBeTruthy();
  });

  it('closes the list when focus leaves the box', async () => {
    await withHits();
    fireEvent.blur(box());
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('does nothing on Enter when there is nothing to open', async () => {
    server(() => json(200, EMPTY));
    await open();
    type('zzzz');
    await wait(SEARCH_DEBOUNCE_MS);
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(window.location.pathname).toBe('/');
  });
});

describe('the mouse', () => {
  it('opens a hit on click, and does not take focus from the box when the button goes down', async () => {
    server(() => json(200, WITH_HITS));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    const [first, , target] = screen.getAllByRole('option');
    const down = fireEvent.mouseDown(first);
    expect(down).toBe(false);
    fireEvent.mouseMove(target);
    expect(target.getAttribute('aria-selected')).toBe('true');
    fireEvent.click(target);
    expect(window.location.pathname + window.location.search).toBe('/runs?target=lock-lease');
  });

  it('does not take focus from the box when the button goes down on the popup itself, such as on its scrollbar', async () => {
    server(() => json(200, WITH_HITS));
    await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    const popup = document.querySelector('.search-popup') as HTMLElement;
    expect(fireEvent.mouseDown(popup)).toBe(false);
  });
});

describe('hostile text', () => {
  const HOSTILE = '<img src=x onerror=alert(1)>';

  it('shows a property id as text through GuestText: no element is made from it, and control characters are visible escapes', async () => {
    server(() => json(200, { ...EMPTY, properties: [{ run_id: RUN, guest: { note: 'n', id: `${HOSTILE}‮\u001b[31m` } }] }));
    const { container } = await open();
    type('db');
    await wait(SEARCH_DEBOUNCE_MS);
    const popup = container.querySelector('.search-popup') as HTMLElement;
    expect(popup.querySelector('img, script, a, iframe, svg')).toBeNull();
    expect(popup.textContent).toContain(HOSTILE);
    expect(popup.textContent).not.toContain('‮');
    expect(popup.textContent).not.toContain('\u001b');
    expect(popup.querySelector('.guest-text')).toBeTruthy();
  });

  it('shows ids, targets and fingerprints from the server as text, never as markup', async () => {
    server(() => json(200, { ...EMPTY, targets: [{ name: HOSTILE }], seeds: [{ run_id: RUN, seed: 1, trial: null }] }));
    const { container } = await open();
    type('run');
    await wait(SEARCH_DEBOUNCE_MS);
    const popup = container.querySelector('.search-popup') as HTMLElement;
    expect(popup.querySelector('img')).toBeNull();
    expect(popup.textContent).toContain(HOSTILE);
    expect(popup.innerHTML).toContain('&lt;img');
  });

  it('puts no link, image or script in the list, and builds no address from guest text', async () => {
    server(() => json(200, { ...EMPTY, properties: [{ run_id: RUN, guest: { note: 'n', id: 'javascript:alert(1)' } }] }));
    const { container } = await open();
    type('jav');
    await wait(SEARCH_DEBOUNCE_MS);
    fireEvent.click(screen.getByRole('option'));
    expect(window.location.pathname).toBe(`/runs/${RUN}`);
    expect(container.querySelector('a, img, script')).toBeNull();
  });

  it('encodes the typed text in the request so it cannot add a parameter', async () => {
    const s = server();
    await open();
    type('a&q=b');
    await wait(1000);
    expect(s.searches()).toEqual([]);
    type('a:b/c');
    await wait(SEARCH_DEBOUNCE_MS);
    expect(s.searches()).toEqual(['/api/search?q=a%3Ab%2Fc']);
  });
});

// @vitest-environment happy-dom
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetConnection } from './connectionStatus';
import { CoveragePanel } from './coverage/CoveragePanel';
import { resetMeForTests } from './meStore';
import { FailuresPage } from './pages/FailuresPage';
import { Runs } from './pages/Runs';
import { RunPage } from './pages/RunPage';
import { SeedPage, TrialPage } from './trial/TrialPage';

/**
 * Each page that shows a load error passes the poller's `networkError` flag to LoadFailed. While the browser is offline
 * a request that got no answer is put down to the connection (the banner says so); a request the server answered with
 * a 500 is not, and keeps its own text.
 */
const noAnswer = () => Promise.reject(new TypeError('Failed to fetch'));
const serverError = () => Promise.resolve(new Response('', { status: 500 }));

/** Answers `/api/me` and `/api/targets` quietly, and every other request with `answer`. */
const serve = (answer: () => Promise<Response>) =>
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      if (String(url).startsWith('/api/targets')) return new Response(JSON.stringify({ targets: [] }));
      if (String(url) === '/api/me') return new Response('{}', { status: 404 });
      if (String(url).includes('/events')) return new Promise<Response>(() => undefined);
      return answer();
    }),
  );

const pages: ReadonlyArray<{ name: string; what: string; show: () => unknown }> = [
  { name: 'Runs list', what: 'runs', show: () => render(<Runs />) },
  { name: 'Failures list', what: 'failures', show: () => render(<FailuresPage />) },
  { name: 'coverage panel', what: 'coverage', show: () => render(<CoveragePanel runId="run_1" live />) },
  { name: 'trial page', what: 'this trial', show: () => render(<TrialPage runId="run_1" seq={1} />) },
  { name: 'seed page', what: 'this seed', show: () => render(<SeedPage runId="run_1" seed="7" />) },
  { name: 'run page', what: 'this run', show: () => render(<RunPage id="run_1" />) },
];

describe('load errors on the page level', () => {
  beforeEach(() => {
    resetMeForTests();
    window.history.replaceState({}, '', '/');
    vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
  });
  afterEach(() => {
    cleanup();
    resetConnection();
    resetMeForTests();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  for (const { name, what, show } of pages) {
    it(`${name}: a request with no answer is put down to the connection, not shown as the browser's text`, async () => {
      serve(noAnswer);
      show();
      await waitFor(() => expect(document.body.textContent).toContain(`Could not load ${what} until the connection returns.`));
      expect(document.body.textContent).not.toContain('Failed to fetch');
    });

    it(`${name}: a 500 keeps its own text while the browser is offline`, async () => {
      serve(serverError);
      show();
      await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(`Could not load ${what}: HTTP 500`));
      expect(document.body.textContent).not.toContain('until the connection returns');
    });
  }
});

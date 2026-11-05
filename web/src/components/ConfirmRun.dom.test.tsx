// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setToken } from '../api';
import { confirmStepText, FINISHED_TEXT, runningText } from '../confirmationWording';
import type { Me } from '../me';
import { resetMeForTests } from '../meStore';
import { ConfirmRun } from './ConfirmRun';

const ID = 'fail_0123456789abcdef';
const STATUS_URL = `/api/failures/${ID}/confirm`;
const json = (body: unknown, status = 200, headers: Record<string, string> = {}) => new Response(JSON.stringify(body), { status, headers });

const base: Me = { role: 'run', open_mode: false, can: { read: true, run: true, admin: false }, roles: ['read', 'run', 'admin'] };
const allowed: Me = { ...base, can: { ...base.can, confirm: true }, confirm: { wall_secs: 120, max_running: 2 } };
const refused: Me = { ...base, can: { ...base.can, confirm: false } };

interface Server {
  me: Me;
  states: string[];
  post: () => Response;
}

/** A fetch that answers `/api/me`, the status route and the start route from the given script. */
function serve(over: Partial<Server> = {}) {
  const server: Server = { me: allowed, states: ['idle'], post: () => json({ accepted: true }, 202), ...over };
  const calls: { url: string; method: string }[] = [];
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET';
    calls.push({ url, method });
    if (url === '/api/me') return json(server.me);
    if (url === STATUS_URL && method === 'POST') return server.post();
    if (url === STATUS_URL) {
      const state = server.states.length > 1 ? server.states.shift() : server.states[0];
      return json({ enabled: true, state, ...(state === 'failed' ? { reason: 'the confirmation could not run on this server; the server\'s log says why' } : {}) });
    }
    return json({ error: 'unexpected' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  return { calls, server };
}

describe('ConfirmRun', () => {
  beforeEach(() => {
    resetMeForTests();
    setToken(null);
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    resetMeForTests();
  });

  it('shows nothing new when the server does not let this caller confirm', async () => {
    const { calls } = serve({ me: refused });
    const { container } = render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    await waitFor(() => expect(calls.some((c) => c.url === '/api/me')).toBe(true));
    await act(async () => undefined);
    expect(container.innerHTML).toBe('');
    expect(calls.some((c) => c.url === STATUS_URL)).toBe(false);
  });

  it('shows nothing, and assumes nothing, when the role cannot be fetched or a server predates the key', async () => {
    serve({ me: base });
    const { container } = render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    await act(async () => undefined);
    await act(async () => undefined);
    expect(container.innerHTML).toBe('');
  });

  it('offers the control, and asks first, saying where it runs, for how long and what it can change, before it starts anything', async () => {
    const { calls } = serve();
    render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Run confirmation' }));
    const step = screen.getByRole('group', { name: 'Run a confirmation on the server?' });
    expect(step.textContent).toContain('This runs on the server for up to 120 seconds.');
    expect(step.textContent).toContain(confirmStepText(120));
    expect(step.textContent).toContain('trusted runner');
    expect(step.textContent).toContain('in either direction');
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(0);
  });

  it('goes back to the control, with focus on it, when the reader says not now', async () => {
    const { calls } = serve();
    render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Run confirmation' }));
    fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
    const control = screen.getByRole('button', { name: 'Run confirmation' });
    expect(document.activeElement).toBe(control);
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(0);
  });

  it('starts once however many times the button is pressed, says it is running, then loads the checks when it has finished', async () => {
    const onFinished = vi.fn();
    const { calls } = serve({ states: ['idle', 'running', 'running', 'idle'] });
    render(<ConfirmRun failureId={ID} onFinished={onFinished} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Run confirmation' }));
    const start = screen.getByRole('button', { name: 'Start confirmation' });
    fireEvent.click(start);
    fireEvent.click(start);
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain(runningText(120)));
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
    expect((screen.getByRole('button', { name: 'Run confirmation' }) as HTMLButtonElement).disabled).toBe(true);
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain(FINISHED_TEXT), { timeout: 15000 });
    expect(onFinished).toHaveBeenCalledTimes(1);
    expect((screen.getByRole('button', { name: 'Run confirmation' }) as HTMLButtonElement).disabled).toBe(false);
  }, 20000);

  it('says what the server said when it refused, and does not run anything', async () => {
    serve({ post: () => json({ error: 'a confirmation of this failure is already running', code: 'confirm_running' }, 409) });
    render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Run confirmation' }));
    fireEvent.click(screen.getByRole('button', { name: 'Start confirmation' }));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('a confirmation of this failure is already running');
    expect(screen.queryByText(FINISHED_TEXT)).toBeNull();
  });

  it('says the server is full, and when to try again, for a 429', async () => {
    serve({ post: () => json({ error: 'too many confirmations are running; try again shortly' }, 429, { 'Retry-After': '30' }) });
    render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Run confirmation' }));
    fireEvent.click(screen.getByRole('button', { name: 'Start confirmation' }));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toBe('The server is already running as many confirmations as it allows. Try again in about 30 seconds.');
  });

  it('shows the control disabled and says it is running when it opens on a confirmation that is already running', async () => {
    serve({ states: ['running'] });
    render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain(runningText(120)));
    expect((screen.getByRole('button', { name: 'Run confirmation' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows the server\'s words, as text, when a confirmation ended in an error', async () => {
    serve({ states: ['failed'] });
    render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('the confirmation could not run on this server'));
    expect(screen.queryByText(FINISHED_TEXT)).toBeNull();
  });

  it('never says a failure is confirmed: the ladder says that, from what is stored', async () => {
    serve({ states: ['running', 'idle'] });
    const { container } = render(<ConfirmRun failureId={ID} onFinished={() => undefined} />);
    await waitFor(() => expect(container.textContent).toContain(runningText(120)));
    await waitFor(() => expect(container.textContent).toContain(FINISHED_TEXT), { timeout: 15000 });
    expect(container.textContent?.toLowerCase()).not.toMatch(/\b(confirmed|verified|fixed|reproduced)\b/);
  }, 20000);
});

// @vitest-environment happy-dom
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { navigate } from './router';

const pending = () => new Promise<Response>(() => undefined);

describe('App: the document title follows the route', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(pending));
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    document.title = '';
  });

  it('sets the title for the first route and changes it on a route change', () => {
    window.history.pushState({}, '', '/targets');
    render(<App />);
    expect(document.title).toBe('Targets · Hairspring');
    act(() => navigate('/workers'));
    expect(document.title).toBe('Workers · Hairspring');
    act(() => navigate('/runs/run_12'));
    expect(document.title).toBe('Run run_12 · Hairspring');
    act(() => navigate('/nowhere'));
    expect(document.title).toBe('No such page · Hairspring');
  });

  it('announces the new page in a polite live region, so a screen reader hears the route change', () => {
    window.history.pushState({}, '', '/targets');
    const { container } = render(<App />);
    const region = container.querySelector('[role="status"].sr-only');
    expect(region?.getAttribute('aria-live')).toBe('polite');
    expect(region?.textContent).toBe('Targets · Hairspring');
    act(() => navigate('/workers'));
    expect(container.querySelector('[role="status"].sr-only')?.textContent).toBe('Workers · Hairspring');
  });

  it('gives a run that does not exist the not-found title, not "Run <id>"', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => (String(url).includes('/api/runs/run_zzzz') ? new Response('{"error":"not found"}', { status: 404 }) : pending())),
    );
    window.history.pushState({}, '', '/runs/run_zzzz');
    render(<App />);
    await waitFor(() => expect(document.title).toBe('No such page · Hairspring'));
  });

  it('gives a failure that does not exist the not-found title', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => (String(url).includes('/api/failures/f_zzzz') ? new Response('{"error":"not found"}', { status: 404 }) : pending())),
    );
    window.history.pushState({}, '', '/failures/f_zzzz');
    render(<App />);
    expect(document.title).toBe('Failure f_zzzz · Hairspring');
    await waitFor(() => expect(document.title).toBe('No such page · Hairspring'));
  });
});

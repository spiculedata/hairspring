// @vitest-environment happy-dom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetSearchProbe } from '../search/useSearchAvailable';
import { Layout } from './Layout';

const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

beforeEach(() => resetSearchProbe());
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('Layout with search', () => {
  it('puts the search box in the top bar between the brand and the theme switch once the server has search', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{"error":"`q` is not valid"}', { status: 400 })));
    const { container } = render(<Layout path="/">body</Layout>);
    await settle();
    const header = container.querySelector('header.topbar') as HTMLElement;
    const parts = [...header.children].map((c) => c.className || c.getAttribute('role'));
    expect(parts).toEqual(['topbar-id', 'search', 'theme-switch']);
    expect(screen.getByRole('combobox', { name: 'Search ids and names' })).toBeTruthy();
    expect(container.querySelectorAll('nav')).toHaveLength(1);
  });

  it('leaves the top bar as it was for a server without search', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{"error":"not found"}', { status: 404 })));
    const { container } = render(<Layout path="/">body</Layout>);
    await settle();
    const header = container.querySelector('header.topbar') as HTMLElement;
    expect([...header.children].map((c) => c.className || c.getAttribute('role'))).toEqual(['topbar-id', 'theme-switch']);
    expect(screen.queryByRole('combobox')).toBeNull();
  });
});

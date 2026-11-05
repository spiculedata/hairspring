// @vitest-environment happy-dom
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { FailureDetail } from '../types';
import { FailurePage } from './FailurePage';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const failure: FailureDetail = {
  record: { id: 'f_abc', run_id: 'run_1', target: 'seats', message: 'm', original_len: 3, shrunk_len: 1, bundle_hash: 'cd'.repeat(32), created_at: 1 },
  steps: [],
  bundle: 'the bundle text',
};

describe('FailurePage bundle', () => {
  it('shows the bundle in a box a keyboard reader can scroll, with a name', async () => {
    vi.spyOn(api, 'failure').mockResolvedValue(failure);
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })));
    render(<FailurePage id="f_abc" />);
    const bundle = await screen.findByLabelText('Bundle text');
    expect(bundle.tagName).toBe('PRE');
    expect(bundle.getAttribute('tabindex')).toBe('0');
    expect(bundle.textContent).toBe('the bundle text');
  });

  it('offers Copy bundle next to the other copy buttons', async () => {
    vi.spyOn(api, 'failure').mockResolvedValue(failure);
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })));
    render(<FailurePage id="f_abc" />);
    expect(await screen.findByRole('button', { name: 'Copy bundle' })).toBeTruthy();
  });

  it('links back to the run it came from, worded as the other run-level pages word it', async () => {
    vi.spyOn(api, 'failure').mockResolvedValue(failure);
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })));
    render(<FailurePage id="f_abc" />);
    const back = await screen.findByRole('link', { name: '← Run' });
    expect(back.getAttribute('href')).toBe('/runs/run_1');
  });
});

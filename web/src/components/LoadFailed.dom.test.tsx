// @vitest-environment happy-dom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { registerPoller, resetConnection } from '../connectionStatus';
import { LoadFailed } from './Notices';

const setOnline = (online: boolean) => {
  vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(online);
  window.dispatchEvent(new Event(online ? 'online' : 'offline'));
};

describe('LoadFailed while the connection banner is showing', () => {
  afterEach(() => {
    cleanup();
    resetConnection();
    vi.restoreAllMocks();
  });

  it('shows the reason and an alert while the connection is fine', () => {
    render(<LoadFailed what="runs" message="HTTP 500" />);
    expect(screen.getByRole('alert').textContent).toContain('Could not load runs: HTTP 500');
  });

  it('does not show the raw browser error when the browser is offline, and leaves the announcing to the banner', () => {
    setOnline(false);
    render(<LoadFailed what="runs" message="Failed to fetch" />);
    expect(document.body.textContent).not.toContain('Failed to fetch');
    expect(screen.queryByRole('alert')).toBeNull();
    expect(document.body.textContent).toContain('Could not load runs until the connection returns.');
  });

  it('does the same once requests have failed several times in a row', () => {
    render(<LoadFailed what="workers" message="Failed to fetch" />);
    expect(document.body.textContent).toContain('Failed to fetch');
    const p = registerPoller();
    act(() => {
      p.failure();
      p.failure();
    });
    expect(document.body.textContent).not.toContain('Failed to fetch');
    expect(document.body.textContent).toContain('Could not load workers until the connection returns.');
  });
});

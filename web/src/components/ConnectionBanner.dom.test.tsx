// @vitest-environment happy-dom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { reportPollFailure, reportPollSuccess, resetConnection } from '../connectionStatus';
import { ConnectionBanner } from './ConnectionBanner';

const setOnline = (online: boolean) => {
  vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(online);
  window.dispatchEvent(new Event(online ? 'online' : 'offline'));
};

describe('ConnectionBanner', () => {
  afterEach(() => {
    cleanup();
    resetConnection();
    vi.restoreAllMocks();
  });

  it('shows nothing while requests are working', () => {
    render(<ConnectionBanner />);
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('stays quiet after a single failed request', () => {
    render(<ConnectionBanner />);
    act(() => reportPollFailure());
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('says the server cannot be reached after failures in a row, and goes when a request works again', () => {
    render(<ConnectionBanner />);
    act(() => {
      reportPollFailure();
      reportPollFailure();
    });
    expect(screen.getByRole('status').textContent).toMatch(/cannot reach the server/i);
    expect(screen.getByRole('status').textContent).toMatch(/last data/i);
    act(() => reportPollSuccess());
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('says the browser is offline as soon as it is, and goes when it is back', () => {
    render(<ConnectionBanner />);
    act(() => setOnline(false));
    expect(screen.getByRole('status').textContent).toMatch(/offline/i);
    act(() => setOnline(true));
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('prefers the offline wording when both apply', () => {
    render(<ConnectionBanner />);
    act(() => {
      setOnline(false);
      reportPollFailure();
      reportPollFailure();
    });
    expect(screen.getByRole('status').textContent).toMatch(/offline/i);
  });
});

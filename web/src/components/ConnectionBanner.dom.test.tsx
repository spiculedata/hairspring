// @vitest-environment happy-dom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { registerPoller, resetConnection } from '../connectionStatus';
import { ConnectionBanner } from './ConnectionBanner';

const setOnline = (online: boolean) => {
  vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(online);
  window.dispatchEvent(new Event(online ? 'online' : 'offline'));
};

const poller = () => registerPoller();
const banner = () => screen.getByRole('status').textContent;

describe('ConnectionBanner', () => {
  afterEach(() => {
    cleanup();
    resetConnection();
    vi.restoreAllMocks();
  });

  it('keeps an empty live region in the page while requests are working, so a screen reader registers it first', () => {
    render(<ConnectionBanner />);
    expect(screen.getByRole('status').textContent).toBe('');
  });

  it('puts the message inside the same live region element that was there before it appeared', () => {
    render(<ConnectionBanner />);
    const region = screen.getByRole('status');
    const p = poller();
    act(() => {
      p.failure();
      p.failure();
    });
    expect(screen.getByRole('status')).toBe(region);
    expect(region.textContent).toMatch(/cannot reach the server/i);
  });

  it('stays quiet after a single failed request', () => {
    render(<ConnectionBanner />);
    act(() => poller().failure());
    expect(banner()).toBe('');
  });

  it('says the server cannot be reached after failures in a row, and goes when a request works again', () => {
    render(<ConnectionBanner />);
    const p = poller();
    act(() => {
      p.failure();
      p.failure();
    });
    expect(banner()).toMatch(/cannot reach the server/i);
    expect(banner()).toMatch(/last data/i);
    act(() => p.success());
    expect(banner()).toBe('');
  });

  it('goes when the failing poll ends', () => {
    render(<ConnectionBanner />);
    const p = poller();
    act(() => {
      p.failure();
      p.failure();
    });
    act(() => p.unregister());
    expect(banner()).toBe('');
  });

  it('says the browser is offline as soon as it is, and goes when it is back', () => {
    render(<ConnectionBanner />);
    act(() => setOnline(false));
    expect(banner()).toMatch(/offline/i);
    act(() => setOnline(true));
    expect(banner()).toBe('');
  });

  it('prefers the offline wording when both apply', () => {
    render(<ConnectionBanner />);
    const p = poller();
    act(() => {
      setOnline(false);
      p.failure();
      p.failure();
    });
    expect(banner()).toMatch(/offline/i);
  });
});

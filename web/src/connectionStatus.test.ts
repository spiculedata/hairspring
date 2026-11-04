import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getConnection,
  OFFLINE_AFTER_FAILURES,
  registerPoller,
  resetConnection,
  subscribeConnection,
} from './connectionStatus';

describe('connection status', () => {
  afterEach(resetConnection);

  it('starts with no failures', () => {
    expect(getConnection().failures).toBe(0);
  });

  it('counts a poller\'s consecutive failures and clears them on its success', () => {
    const poller = registerPoller();
    poller.failure();
    poller.failure();
    expect(getConnection().failures).toBe(2);
    poller.success();
    expect(getConnection().failures).toBe(0);
  });

  it('replaces the snapshot instead of changing it, and tells subscribers only when it changes', () => {
    const poller = registerPoller();
    const listener = vi.fn();
    const stop = subscribeConnection(listener);
    const before = getConnection();
    poller.success();
    expect(listener).not.toHaveBeenCalled();
    poller.failure();
    expect(listener).toHaveBeenCalledTimes(1);
    expect(getConnection()).not.toBe(before);
    expect(before.failures).toBe(0);
    stop();
    poller.failure();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('does not let one poller\'s success hide another poller\'s failures', () => {
    const failing = registerPoller();
    const working = registerPoller();
    failing.failure();
    failing.failure();
    working.success();
    expect(getConnection().failures).toBe(2);
    failing.success();
    expect(getConnection().failures).toBe(0);
  });

  it('drops a poller\'s failures when it unregisters, and tells subscribers', () => {
    const poller = registerPoller();
    poller.failure();
    poller.failure();
    const listener = vi.fn();
    const stop = subscribeConnection(listener);
    poller.unregister();
    expect(getConnection().failures).toBe(0);
    expect(listener).toHaveBeenCalledTimes(1);
    stop();
  });

  it('keeps the other pollers\' failures when one unregisters', () => {
    const a = registerPoller();
    const b = registerPoller();
    a.failure();
    b.failure();
    b.failure();
    a.unregister();
    expect(getConnection().failures).toBe(2);
  });

  it('ignores reports from a poller after it unregistered', () => {
    const poller = registerPoller();
    poller.unregister();
    poller.failure();
    poller.failure();
    expect(getConnection().failures).toBe(0);
  });

  it('treats a single failure as a blip and two in a row as the server being out of reach', () => {
    expect(OFFLINE_AFTER_FAILURES).toBe(2);
  });
});

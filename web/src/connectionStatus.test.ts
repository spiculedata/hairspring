import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getConnection,
  OFFLINE_AFTER_FAILURES,
  reportPollFailure,
  reportPollSuccess,
  resetConnection,
  subscribeConnection,
} from './connectionStatus';

describe('connection status', () => {
  afterEach(resetConnection);

  it('starts with no failures', () => {
    expect(getConnection().failures).toBe(0);
  });

  it('counts consecutive failures and clears them on a success', () => {
    reportPollFailure();
    reportPollFailure();
    expect(getConnection().failures).toBe(2);
    reportPollSuccess();
    expect(getConnection().failures).toBe(0);
  });

  it('replaces the snapshot instead of changing it, and tells subscribers only when it changes', () => {
    const listener = vi.fn();
    const stop = subscribeConnection(listener);
    const before = getConnection();
    reportPollSuccess();
    expect(listener).not.toHaveBeenCalled();
    reportPollFailure();
    expect(listener).toHaveBeenCalledTimes(1);
    expect(getConnection()).not.toBe(before);
    expect(before.failures).toBe(0);
    stop();
    reportPollFailure();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('treats a single failure as a blip and two in a row as the server being out of reach', () => {
    expect(OFFLINE_AFTER_FAILURES).toBe(2);
  });
});

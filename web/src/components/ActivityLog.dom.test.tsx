// @vitest-environment happy-dom
import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ActivityLog } from './ActivityLog';

describe('ActivityLog in a DOM', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('gives every fact its own key: two keys that read the same once underscores are spaces raise no duplicate-key warning', () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const { container } = render(<ActivityLog lines={[{ id: '1', event: 'x', data: '{"a_b":1,"a b":2}' }]} />);
    expect(container.querySelectorAll('.activity-fact')).toHaveLength(2);
    const warnings = errors.mock.calls.map((c) => String(c[0]) + String(c[1] ?? '')).filter((m) => /same key|unique "key"/i.test(m));
    expect(warnings).toEqual([]);
  });

  it('shows a bidi override in an event name or a key as a visible escape, never as the raw character', () => {
    const { container } = render(<ActivityLog lines={[{ id: '1', event: 'a\u202eb', data: '{"k\u202e":"v"}' }]} />);
    expect(container.textContent).not.toContain('\u202e');
    expect(container.querySelectorAll('.guest-esc').length).toBeGreaterThanOrEqual(2);
  });
});

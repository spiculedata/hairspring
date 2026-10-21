// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useEffect } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ErrorBoundary } from './ErrorBoundary';

const failing = { current: true };
const mounts = { count: 0 };

function Part() {
  useEffect(() => {
    mounts.count += 1;
  }, []);
  if (failing.current) throw new Error('boom');
  return <p>part is fine</p>;
}

describe('ErrorBoundary in a DOM', () => {
  beforeEach(() => {
    failing.current = true;
    mounts.count = 0;
    // React reports a caught render error on the console; it is expected here.
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shows the failure with a Retry button named for the part', () => {
    render(
      <ErrorBoundary label="Property table">
        <Part />
      </ErrorBoundary>,
    );
    expect(screen.getByRole('alert').textContent).toBe('Property table could not be shown: boom');
    expect(screen.getByRole('button', { name: 'Retry Property table' })).toBeTruthy();
  });

  it('mounts the children again on Retry, and shows them once what failed has passed', () => {
    render(
      <ErrorBoundary label="Property table">
        <Part />
      </ErrorBoundary>,
    );
    failing.current = false;
    fireEvent.click(screen.getByRole('button', { name: 'Retry Property table' }));
    expect(screen.getByText('part is fine')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(mounts.count).toBe(1);
  });

  it('shows the failure again, and Retry again, when it has not passed', () => {
    render(
      <ErrorBoundary label="Property table">
        <Part />
      </ErrorBoundary>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Retry Property table' }));
    expect(screen.getByRole('alert')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Retry Property table' })).toBeTruthy();
  });

  it('asks its owner to load the data again before it mounts the children', () => {
    const order: string[] = [];
    const onRetry = vi.fn(() => {
      order.push('reload');
      failing.current = false;
    });
    function Recording() {
      order.push('render');
      return <Part />;
    }
    render(
      <ErrorBoundary label="Strip" onRetry={onRetry}>
        <Recording />
      </ErrorBoundary>,
    );
    order.length = 0;
    fireEvent.click(screen.getByRole('button', { name: 'Retry Strip' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(order[0]).toBe('reload');
    expect(screen.getByText('part is fine')).toBeTruthy();
  });

  it('moves focus to the part when Retry is used, because the button is gone', () => {
    render(
      <ErrorBoundary label="Property table">
        <Part />
      </ErrorBoundary>,
    );
    failing.current = false;
    const button = screen.getByRole('button', { name: 'Retry Property table' });
    button.focus();
    fireEvent.click(button);
    const part = screen.getByRole('group', { name: 'Property table' });
    expect(document.activeElement).toBe(part);
  });

  it('does not take focus when it first renders', () => {
    failing.current = false;
    render(
      <ErrorBoundary label="Property table">
        <Part />
      </ErrorBoundary>,
    );
    expect(document.activeElement).toBe(document.body);
  });
});

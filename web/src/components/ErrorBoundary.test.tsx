import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { ErrorBoundary, ErrorNotice, afterRetry } from './ErrorBoundary';

/** The boundary's render, run against a given state; the node test environment has no DOM to throw inside. */
function renderFailed(message: string): string {
  const self = { state: { message, attempt: 0 }, props: { label: 'Property table', children: <p>fine</p> }, retry: () => undefined };
  return renderToString(ErrorBoundary.prototype.render.call(self as unknown as ErrorBoundary));
}

describe('ErrorBoundary', () => {
  it('turns a render error into state', () => {
    expect(ErrorBoundary.getDerivedStateFromError(new Error('boom'))).toEqual({ message: 'boom' });
  });

  it('renders its children when nothing went wrong', () => {
    expect(renderToString(<ErrorBoundary label="Property table"><p>fine</p></ErrorBoundary>)).toContain('fine');
  });

  it('shows the failure as an alert, escaped, without the component stack', () => {
    const html = renderToString(<ErrorNotice label="Property table" message={'<img src=x onerror=1>'} />);
    expect(html).toContain('role="alert"');
    expect(html).toContain('Property table could not be shown');
    expect(html).not.toContain('<img');
  });

  it('offers a Retry button when it can retry, and none when it cannot', () => {
    const withRetry = renderToString(<ErrorNotice label="Strip" message="boom" onRetry={() => undefined} />);
    expect(withRetry).toMatch(/<button type="button"[^>]*>Retry<\/button>/);
    expect(renderToString(<ErrorNotice label="Strip" message="boom" />)).not.toContain('<button');
  });

  it('shows the Retry button when a part of the page failed', () => {
    const html = renderFailed('boom');
    expect(html).toContain('Property table could not be shown: boom');
    expect(html).toContain('>Retry</button>');
  });

  it('asks its owner to load the data again on retry, because the data it failed on lives outside it', () => {
    const onRetry = vi.fn();
    const boundary = new ErrorBoundary({ label: 'Property table', children: null, onRetry });
    boundary.setState = vi.fn();
    boundary.retry();
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(boundary.setState).toHaveBeenCalledTimes(1);
  });

  it('retries without an owner callback too', () => {
    const boundary = new ErrorBoundary({ label: 'Strip', children: null });
    boundary.setState = vi.fn();
    expect(() => boundary.retry()).not.toThrow();
    expect(boundary.setState).toHaveBeenCalledTimes(1);
  });

  it('clears the failure on retry and remounts the children, so what failed loads again', () => {
    const before = { message: 'boom', attempt: 2 };
    const after = afterRetry(before);
    expect(after).toEqual({ message: undefined, attempt: 3 });
    expect(before).toEqual({ message: 'boom', attempt: 2 });
  });
});

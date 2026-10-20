import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { ErrorBoundary, ErrorNotice, afterRetry } from './ErrorBoundary';

describe('ErrorBoundary markup', () => {
  it('turns a render error into state', () => {
    expect(ErrorBoundary.getDerivedStateFromError(new Error('boom'))).toEqual({ message: 'boom' });
  });

  it('renders its children when nothing went wrong, in a labelled group', () => {
    const html = renderToString(<ErrorBoundary label="Property table"><p>fine</p></ErrorBoundary>);
    expect(html).toContain('fine');
    expect(html).toContain('role="group"');
    expect(html).toContain('aria-label="Property table"');
  });

  it('shows the failure as an alert, escaped, without the component stack', () => {
    const html = renderToString(<ErrorNotice label="Property table" message={'<img src=x onerror=1>'} />);
    expect(html).toContain('role="alert"');
    expect(html).toContain('Property table could not be shown');
    expect(html).not.toContain('<img');
  });

  it('offers a Retry button named for the part it retries, and none when it cannot retry', () => {
    const withRetry = renderToString(<ErrorNotice label="Strip" message="boom" onRetry={() => undefined} />);
    expect(withRetry).toMatch(/<button type="button" aria-label="Retry Strip">Retry<\/button>/);
    expect(renderToString(<ErrorNotice label="Strip" message="boom" />)).not.toContain('<button');
  });

  it('clears the failure on retry and remounts the children, so what failed loads again', () => {
    const before = { message: 'boom', attempt: 2 };
    const after = afterRetry(before);
    expect(after).toEqual({ message: undefined, attempt: 3 });
    expect(before).toEqual({ message: 'boom', attempt: 2 });
  });
});

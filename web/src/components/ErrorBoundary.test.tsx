import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { ErrorBoundary, ErrorNotice } from './ErrorBoundary';

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
});

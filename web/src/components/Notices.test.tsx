import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { LoadFailed, Loading, NotFound } from './Notices';

describe('Loading', () => {
  it('says what is loading', () => {
    expect(renderToString(<Loading what="run" />)).toContain('Loading run…');
    expect(renderToString(<Loading what="failure" />)).toContain('Loading failure…');
  });
});

describe('NotFound', () => {
  it('says what was not found, why that may be, and where to go next', () => {
    const html = renderToString(<NotFound what="run" />);
    expect(html).toContain('This run does not exist');
    expect(html).toContain('may have been removed');
    expect(html).toMatch(/<a href="\/"[^>]*>[^<]*Runs/);
    expect(html).toContain('role="alert"');
  });
  it('says so for a page that does not exist, and to check the address', () => {
    const html = renderToString(<NotFound what="page" />);
    expect(html).toContain('There is no page at this address');
    expect(html).toContain('Check the address');
  });
});

describe('LoadFailed', () => {
  it('names what could not be loaded, shows the reason and says what to do, all inside the one announced alert', () => {
    const html = renderToString(<LoadFailed what="runs" message="HTTP 500" retries={false} />);
    expect(html.match(/role="alert"/g)).toHaveLength(1);
    const alert = html.slice(html.indexOf('role="alert"'));
    for (const part of ['Could not load runs', 'HTTP 500', 'Reload the page to try again']) expect(alert).toContain(part);
  });
  it('does not tell the reader to reload a page that tries again by itself', () => {
    const html = renderToString(<LoadFailed what="runs" message="HTTP 500" retries />);
    expect(html).toContain('This page tries again by itself');
    expect(html).not.toContain('Reload');
  });
});

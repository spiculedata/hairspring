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
  it('names what could not be loaded, shows the reason and says what to do', () => {
    const html = renderToString(<LoadFailed what="runs" message="HTTP 500" />);
    expect(html).toContain('Could not load runs');
    expect(html).toContain('HTTP 500');
    expect(html).toContain('Reload the page to try again');
    expect(html).toContain('role="alert"');
  });
});

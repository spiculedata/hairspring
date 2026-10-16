import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { appendStripPage, EMPTY_STRIP, MAX_STRIP_TRIALS } from './strip';
import { StripSection } from './StripSection';

const noop = () => undefined;
const strip = (codes: string) => appendStripPage(EMPTY_STRIP, { first_seq: 0, codes });
const render = (codes: string, extra: Partial<Parameters<typeof StripSection>[0]> = {}) =>
  renderToString(<StripSection strip={strip(codes)} loading={false} error={undefined} onOpen={noop} {...extra} />);
const textOf = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('StripSection', () => {
  it('is a keyboard slider with a text value, and its canvas is hidden from assistive technology', () => {
    const html = render('ppvpep');
    expect(html).toContain('role="slider"');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain('aria-valuetext');
    expect(html).toMatch(/<canvas[^>]*aria-hidden="true"/);
  });

  it('has a text alternative: the counts of judged trials by class, in a table', () => {
    const html = render('p'.repeat(90) + 'vvv' + 'iiii' + 'eee');
    expect(html).toContain('<table');
    expect(html).toContain('<caption');
    const text = textOf(html);
    expect(text).toContain('100 judged: 3 violation, 4 inconclusive, 3 excluded, 90 pass');
    expect(text).toMatch(/✕ violation\s+3/);
    expect(text).toMatch(/◌ excluded\s+3/);
    expect(text).toMatch(/\? inconclusive\s+4/);
    expect(text).toMatch(/✓ pass\s+90/);
  });

  it('lists only classes that occur in the table, and never shows an excluded or inconclusive trial under pass', () => {
    const text = textOf(render('ppe'));
    expect(text).toMatch(/◌ excluded\s+1/);
    expect(text).toMatch(/✓ pass\s+2/);
    expect(text).not.toContain('violation');
  });

  it('explains the keys and how a group of trials is drawn', () => {
    const text = textOf(render('pp'));
    expect(text).toContain('Arrow keys');
    expect(text).toContain('worst trial');
  });

  it('says so when the strip was cut at its cap', () => {
    const cut = appendStripPage(EMPTY_STRIP, { first_seq: 0, codes: 'p'.repeat(MAX_STRIP_TRIALS) });
    const html = renderToString(<StripSection strip={{ ...cut, truncated: true }} loading={false} error={undefined} onOpen={noop} />);
    expect(textOf(html)).toContain('first 250,000 trials');
  });

  it('shows loading and error states without hiding what is loaded', () => {
    expect(textOf(render('pp', { loading: true }))).toContain('Loading');
    const html = render('pp', { error: 'the server is down' });
    expect(html).toContain('role="alert"');
    expect(textOf(html)).toContain('the server is down');
    expect(textOf(html)).toContain('2 judged');
  });

  it('shows an empty run as such', () => {
    expect(textOf(render(''))).toContain('0 judged');
  });
});

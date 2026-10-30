import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import { ActivityLog } from './ActivityLog';

const events = [
  { id: '7', event: 'started', data: '{"worker":"local-55774"}' },
  { id: '8', event: 'progress', data: '{"message":"1 of 40 trials run","steps":79,"trials":1}' },
  { id: '9', event: 'finished', data: '{"distinct_schedules":0,"inconclusive":0,"state":"failed","work":0}' },
];

describe('ActivityLog', () => {
  it('shows each event as a titled line with labelled facts, not as raw JSON', () => {
    const html = renderToString(<ActivityLog lines={events} />);
    for (const t of ['Started', 'Progress', 'Finished', 'Worker', 'local-55774', '1 of 40 trials run', 'Distinct schedules']) expect(html).toContain(t);
    expect(html).toMatch(/<ol[^>]*aria-live="polite"/);
    const outsideDisclosures = html.replace(/<details[\s\S]*?<\/details>/g, '');
    expect(outsideDisclosures).not.toMatch(/&quot;|\{/);
  });

  it('keeps the raw event behind a disclosure for anyone who wants it', () => {
    const html = renderToString(<ActivityLog lines={events} />);
    expect(html.match(/<details/g)).toHaveLength(3);
    expect(html).toContain('Raw event');
  });

  it('says what an empty log means', () => {
    const html = renderToString(<ActivityLog lines={[]} />);
    expect(html).toContain('No activity yet');
  });

  it('shows text from a worker as escaped text only', () => {
    const html = renderToString(<ActivityLog lines={[{ id: '1', event: 'started', data: '{"worker":"<img src=x onerror=alert(1)>"}' }]} />);
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img');
  });

  it('shows an event name, a JSON key and a value from a worker as escaped text only', () => {
    const data = JSON.stringify({ '<b>key</b>': 'a\u202eb\u0007c', worker: '<i>w</i>' });
    const html = renderToString(<ActivityLog lines={[{ id: '1', event: '<img src=x onerror=alert(1)>', data }]} />);
    for (const raw of ['<img', '<b>', '<i>', '\u202e', '\u0007']) expect(html, raw).not.toContain(raw);
    expect(html).toContain('&lt;img');
    expect(html).toContain('&lt;b&gt;key');
  });

  it('keeps two keys that differ only by underscore or space apart', () => {
    const html = renderToString(<ActivityLog lines={[{ id: '1', event: 'x', data: '{"a_b":1,"a b":2}' }]} />);
    expect(html.match(/activity-fact/g)).toHaveLength(2);
  });

  it('cuts a very long value once, with one marker', () => {
    const html = renderToString(<ActivityLog lines={[{ id: '1', event: 'progress', data: JSON.stringify({ message: 'x'.repeat(5000) }) }]} />);
    const facts = html.slice(0, html.indexOf('<details'));
    expect(facts.match(/more characters/g)).toHaveLength(1);
    expect(facts).not.toMatch(/…\s*…/);
  });

  it('renders no title, label or value except through the escaping text component (the source cannot regress)', () => {
    const src = readSource('components/ActivityLog.tsx').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(src).not.toMatch(/dangerouslySetInnerHTML|innerHTML/);
    expect(src).not.toMatch(/(?<![=\w$])\{\s*(title|label|value|line\.event|line\.data)\s*\}/);
    expect(src.match(/<GuestText/g)?.length).toBeGreaterThanOrEqual(4);
  });
});

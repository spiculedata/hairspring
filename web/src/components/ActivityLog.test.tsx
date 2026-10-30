import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
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
});

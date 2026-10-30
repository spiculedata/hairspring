import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { StateBadge } from './Badge';

describe('StateBadge', () => {
  it('shows a passed run as "No failure seen" with its scope and the server word in the tooltip', () => {
    const html = renderToString(<StateBadge state="passed" trials={30} completion={{ kind: 'complete' }} />);
    expect(html).toContain('No failure seen');
    expect(html).toContain('30 trials');
    expect(html).toContain('title="Server state: passed"');
    expect(html).not.toMatch(/>passed</);
  });
  it('shows "Stopped early" with trials run of planned', () => {
    const html = renderToString(
      <StateBadge state="passed" trials={2} completion={{ kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 }} />,
    );
    expect(html).toContain('Stopped early');
    expect(html).toContain('2 of 30 planned trials');
  });
  it('is neutral, with no pass tone, while the completion is not known', () => {
    const html = renderToString(<StateBadge state="passed" trials={30} />);
    expect(html).toContain('badge-idle');
    expect(html).not.toContain('badge-good');
    expect(html).not.toContain('No failure seen');
  });
  it('has a space in the text between the label and its scope, so it reads right without the stylesheet', () => {
    const html = renderToString(<StateBadge state="passed" trials={30} completion={{ kind: 'complete' }} />);
    expect(html.replace(/<[^>]+>/g, '')).toBe('No failure seen 30 trials');
  });
  it('puts the scope in its own small element, so it does not take the heading size', () => {
    expect(renderToString(<StateBadge state="passed" trials={30} completion={{ kind: 'complete' }} />)).toContain('badge-scope');
  });
  it('shows other states as sent, without a scope', () => {
    const html = renderToString(<StateBadge state="failed" trials={3} />);
    expect(html).toContain('>failed<');
    expect(html).not.toContain('trials');
  });
});

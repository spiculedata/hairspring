import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { StateBadge } from './Badge';

describe('StateBadge', () => {
  it('shows a passed run as "No failure seen" with its scope', () => {
    const html = renderToString(<StateBadge state="passed" trials={30} completion={{ kind: 'complete' }} />);
    expect(html).toContain('No failure seen');
    expect(html).toContain('30 of 30 planned trials');
    expect(html).not.toMatch(/>passed</);
  });
  it('shows "Stopped early" with trials run of planned', () => {
    const html = renderToString(
      <StateBadge state="passed" trials={2} completion={{ kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 }} />,
    );
    expect(html).toContain('Stopped early');
    expect(html).toContain('2 of 30 planned trials');
  });
  it('shows other states as sent, without a scope', () => {
    const html = renderToString(<StateBadge state="failed" trials={3} />);
    expect(html).toContain('>failed<');
    expect(html).not.toContain('trials');
  });
});

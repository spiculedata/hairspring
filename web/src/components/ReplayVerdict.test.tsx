import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { ReplayVerdict } from './ReplayVerdict';

describe('ReplayVerdict', () => {
  it('says "Failed again on replay" and what one replay does and does not show', () => {
    const html = renderToString(<ReplayVerdict failedAgain />);
    expect(html).toContain('Failed again on replay');
    expect(html).toContain('One replay is not a confirmation');
    expect(html).toContain('class="error"');
  });

  it('says "Did not fail on replay" and does not claim the bug is fixed', () => {
    const html = renderToString(<ReplayVerdict failedAgain={false} />);
    expect(html).toContain('Did not fail on replay');
    expect(html).toContain('One replay is not a confirmation');
    expect(html).not.toMatch(/no longer fails|has been fixed|now passes/i);
    expect(html).toContain('class="ok"');
  });

  it('never says "reproduced" or "still violated"', () => {
    for (const failedAgain of [true, false]) {
      expect(renderToString(<ReplayVerdict failedAgain={failedAgain} />)).not.toMatch(/reproduc|still violated/i);
    }
  });
});

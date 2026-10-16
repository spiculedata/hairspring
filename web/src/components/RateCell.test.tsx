import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { RateCellData } from '../types';
import { RateCell } from './RateCell';

const rate: RateCellData = { kind: 'rate', basis: 'trials_false', k: 3, n: 4790, point: 0.000626, lower: 0.0002, upper: 0.0018 };

describe('RateCell', () => {
  it('shows k/n with its interval and an interval bar with a text alternative', () => {
    const html = renderToString(<RateCell cell={rate} />);
    expect(html).toContain('3/4,790');
    expect(html).toContain('[0.02%–0.18%]');
    expect(html).toContain('role="img"');
    expect(html).toContain('aria-label="interval 0.02% to 0.18%"');
  });

  it('never renders a bare percentage', () => {
    for (const cell of [
      rate,
      { kind: 'zero_events', basis: 'trials_false', n: 100, upper_bound: 0.03 } as RateCellData,
      { kind: 'unrated' } as RateCellData,
      { kind: 'not_a_rate', basis: 'trials_false', k: 1, n: 16 } as RateCellData,
    ]) {
      const text = renderToString(<RateCell cell={cell} />).replace(/<[^>]+>/g, '');
      if (text.includes('%')) expect(text).toMatch(/\d+\/[\d,]+/);
    }
  });

  it('zero failures: 0/n and the 3/n bound, no interval bar', () => {
    const html = renderToString(<RateCell cell={{ kind: 'zero_events', basis: 'trials_false', n: 4790, upper_bound: 3 / 4790 }} />);
    expect(html).toContain('0/4,790');
    expect(html).toContain('at most 3/4,790');
    expect(html).not.toContain('role="img"');
  });

  it('unrated says so and shows no percentage or bar', () => {
    const html = renderToString(<RateCell cell={{ kind: 'unrated' }} />);
    expect(html).toContain('unrated (0 rated trials)');
    expect(html).not.toContain('%');
    expect(html).not.toContain('role="img"');
  });

  it('a run stopped at its first failure shows counts, never an interval or a bar', () => {
    const html = renderToString(<RateCell cell={{ kind: 'not_a_rate', basis: 'trials_false', k: 1, n: 16 }} />);
    expect(html).toContain('not a rate (stopped at first failure)');
    expect(html).toContain('1 failure in 16 rated trials');
    expect(html).not.toContain('%');
    expect(html).not.toContain('role="img"');
  });
});

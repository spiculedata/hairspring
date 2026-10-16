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
    const html = renderToString(<RateCell cell={{ kind: 'zero_events', basis: 'trials_false', n: 4790, upper_bound: 1 - 0.05 ** (1 / 4790) }} />);
    expect(html).toContain('0/4,790');
    expect(html).toContain('exact one-sided 95% upper bound 0.06% (binomial, 0 of 4,790)');
    expect(html).not.toContain('at most');
    expect(html).not.toContain('role="img"');
  });

  it('keeps the counts in the unclamped figure and the qualifier in its own clamped line', () => {
    const cells: RateCellData[] = [
      { kind: 'not_a_rate', basis: 'TrialsFalse', k: 1, n: 3 },
      { kind: 'zero_events', basis: 'TrialsFalse', n: 3, upper_bound: 0.63 },
      { kind: 'rate', basis: 'TrialsFalse', k: 3, n: 4790, point: 0.000626, lower: 0.0002, upper: 0.0018 },
    ];
    for (const cell of cells) {
      const html = renderToString(<RateCell cell={cell} />);
      const figure = /<span class="rate-figure">(.*?)<\/span>/.exec(html)?.[1] ?? '';
      expect(figure).toMatch(/\d/);
      expect(html).toContain('rate-qualifier');
      expect(html.indexOf('rate-figure')).toBeLessThan(html.indexOf('rate-qualifier'));
    }
    const stopped = renderToString(<RateCell cell={cells[0]} />);
    expect(stopped).toContain('<span class="rate-figure">1 failure in 3 rated trials</span>');
    expect(stopped).toContain('<span class="rate-qualifier">not a rate (stopped at first failure)</span>');
  });

  it('carries the full text in a title so a clipped qualifier can still be read', () => {
    const html = renderToString(<RateCell cell={{ kind: 'not_a_rate', basis: 'TrialsFalse', k: 1, n: 3 }} />);
    expect(html).toContain('title="not a rate (stopped at first failure) · 1 failure in 3 rated trials"');
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

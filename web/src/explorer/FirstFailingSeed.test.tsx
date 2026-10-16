import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { FirstFailingSeed } from './FirstFailingSeed';

describe('FirstFailingSeed', () => {
  it('links the lowest failing seed to its failure page, in words that make no claim about rate', () => {
    const html = renderToString(<FirstFailingSeed runId="run_0123" seed="1016" />);
    expect(html).toContain('<a href="/runs/run_0123/seeds/1016">');
    expect(html.replace(/<[^>]+>/g, '')).toContain('Lowest failing seed: 1016');
  });

  it('renders nothing when the run has no failing seed or the seed is not plain digits', () => {
    expect(renderToString(<FirstFailingSeed runId="run_0123" seed={null} />)).toBe('');
    expect(renderToString(<FirstFailingSeed runId="run_0123" seed={undefined} />)).toBe('');
    expect(renderToString(<FirstFailingSeed runId="run_0123" seed="../x" />)).toBe('');
  });
});

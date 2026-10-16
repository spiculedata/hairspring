import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { TierChips } from './TierChips';

const text = (html: string) => html.replace(/<[^>]+>/g, '|').replace(/\|+/g, '|');

describe('TierChips', () => {
  it('shows the tier and seed selection chips the server sent', () => {
    const html = renderToString(<TierChips chips={['tier: assisted', 'seed selection: blind', 'targeted workload']} caveats={[]} unverified={false} />);
    expect(html).toContain('tier: assisted');
    expect(html).toContain('seed selection: blind');
    expect(html).toContain('targeted workload');
  });

  it('shows the grey "label not declared" chip when the server sent none', () => {
    const html = renderToString(<TierChips chips={[]} caveats={[]} unverified={false} />);
    expect(html).toContain('label not declared');
    expect(html).toContain('chip-muted');
  });

  it('adds "unverified worker data" from the header flag and does not duplicate the server chip', () => {
    const added = renderToString(<TierChips chips={['label not declared']} caveats={[]} unverified />);
    expect(added).toContain('unverified worker data');
    const dup = renderToString(<TierChips chips={['label not declared', 'unverified worker data']} caveats={[]} unverified />);
    expect(dup.match(/unverified worker data/g)).toHaveLength(1);
  });

  it('shows the single_run caveat as a chip in words', () => {
    const html = renderToString(<TierChips chips={['label not declared']} caveats={['single_run']} unverified={false} />);
    expect(text(html)).toContain('single run');
    expect(html).not.toContain('single_run');
  });

  it('carries meaning in text, not colour alone: every chip has a text label and a tone class', () => {
    const html = renderToString(<TierChips chips={['tier: assisted']} caveats={['single_run']} unverified />);
    expect(html.match(/<li class="chip chip-(muted|warn|info)"/g)?.length).toBe(3);
    expect(html).toContain('<ul class="chips" aria-label="Run label and caveats">');
  });

  it('never generates the word "unassisted"', () => {
    const html = renderToString(<TierChips chips={[]} caveats={['single_run']} unverified />);
    expect(html).not.toMatch(/unassisted/i);
  });
});

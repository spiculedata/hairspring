import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';

const css = readSource('trial/trial.css');

describe('trial.css source order', () => {
  it('puts the phone navigation rules after the base rules they override', () => {
    const mediaStart = css.indexOf('@media (max-width: 40rem)');
    const mediaEnd = css.indexOf('\n}', mediaStart);
    expect(mediaStart).toBeGreaterThanOrEqual(0);
    const media = css.slice(mediaStart, mediaEnd);
    const outside = css.slice(0, mediaStart) + css.slice(mediaEnd);
    for (const selector of ['.trial-nav-list', '.trial-nav-select']) {
      expect(media, selector).toContain(`${selector} {`);
      // The base rule must be declared before the media block; a later one would win at equal specificity.
      const base = outside.indexOf(`\n${selector} {`);
      expect(base, `${selector} base rule`).toBeGreaterThanOrEqual(0);
      const baseAt = css.indexOf(`\n${selector} {`);
      expect(baseAt, `${selector} base rule before the media block`).toBeLessThan(mediaStart);
      expect(css.lastIndexOf(`\n${selector} {`), `${selector} has no later base rule`).toBeLessThan(mediaStart);
    }
  });
});

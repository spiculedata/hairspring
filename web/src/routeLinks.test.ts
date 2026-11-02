import { readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { readSource } from './readSource';

const sourceFiles = (readdirSync(new URL('./', import.meta.url), { recursive: true }) as string[]).filter(
  (n) => n.endsWith('.tsx') && !n.endsWith('.test.tsx'),
);

describe('links to the run list', () => {
  it('no page links to "/" as if it were the run list: "/" is the Overview page and the run list is /runs', () => {
    const offenders = sourceFiles.filter((f) => /<Link to="\/">/.test(readSource(f)));
    expect(offenders).toEqual([]);
  });

  it('the run page links back to /runs', () => {
    expect(readSource('pages/RunPage.tsx')).toContain('<Link to="/runs">← Runs</Link>');
  });
});

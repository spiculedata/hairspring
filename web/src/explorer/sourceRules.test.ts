import { describe, expect, it } from 'vitest';

/** Every non-test source file under src, as raw text (resolved by Vite at test time, so no Node typings are needed). */
const RAW = import.meta.glob<string>('../**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });

const ALL: ReadonlyArray<readonly [string, string]> = Object.entries(RAW).filter(
  ([path]) => !/\.test\.tsx?$/.test(path) && !/fixtures\.ts$/.test(path),
);

/** Source with comments removed, so a rule is about code and strings, not prose. */
function code(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1');
}

describe('source rules (plan 7.2 item 1 and 5.7 rule 5)', () => {
  it('finds the sources it is meant to check', () => {
    expect(ALL.length).toBeGreaterThan(15);
    expect(ALL.some(([p]) => p.endsWith('components/GuestText.tsx'))).toBe(true);
  });

  it('never uses dangerouslySetInnerHTML, innerHTML or document.write', () => {
    for (const [path, src] of ALL) {
      expect(code(src), path).not.toMatch(/dangerouslySetInnerHTML|\.innerHTML|outerHTML|insertAdjacentHTML|document\.write|\beval\(|new Function\(/);
    }
  });

  it('never builds an href, src or style from guest data', () => {
    for (const [path, src] of ALL) expect(code(src), path).not.toMatch(/(href|src|style)=\{[^}]*\bguest\b/);
  });

  // "not found" is allowed: it is the wording of a missing page, not a claim about a result.
  it('never generates the words found, discovered, unassisted or flaky anywhere in the UI source', () => {
    const checked = ALL.filter(([p]) => !p.endsWith('/types.ts'));
    expect(checked.length).toBeGreaterThan(15);
    expect(checked.some(([p]) => p.includes('/seeds/'))).toBe(true);
    expect(checked.some(([p]) => p.includes('/pages/'))).toBe(true);
    for (const [path, src] of checked) expect(code(src).replace(/Page not found/g, ''), path).not.toMatch(/\bfound\b|\b(discovered|unassisted|flaky)\b/i);
  });

  it('never generates "reproduced", as a replay verdict or for counts, anywhere in the UI source', () => {
    // `types.ts` mirrors the server's own field and status names (`reproduced`, `not_reproducible`); reading the
    // server's `.reproduced` field is allowed, writing the word for a person to read is not.
    const checked = ALL.filter(([p]) => !p.endsWith('/types.ts'));
    expect(checked.some(([p]) => p.includes('/pages/'))).toBe(true);
    expect(checked.some(([p]) => p.includes('/trial/'))).toBe(true);
    for (const [path, src] of checked) expect(code(src).replace(/\.reproduced\b/g, ''), path).not.toMatch(/reproduc/i);
  });

  it('never says a replay "still violated" the property: a replay is one more run, not a confirmation', () => {
    for (const [path, src] of ALL) expect(code(src), path).not.toMatch(/still violated|is still violated/i);
  });

  it('never shows the engine terms PASS, VIOLATION or UNMET as a label in the UI source', () => {
    for (const [path, src] of ALL.filter(([p]) => !p.endsWith('/types.ts'))) expect(code(src), path).not.toMatch(/['"`](PASS|VIOLATION|UNMET)\b/);
  });

  it('never maps a status or class key to its own engine word as a label, in any case ("pass: \'pass\'", "violation: \'violation\'")', () => {
    for (const [path, src] of ALL.filter(([p]) => !p.endsWith('/types.ts'))) {
      expect(code(src), path).not.toMatch(/\b(pass|violation|unmet)\s*:\s*['"`](pass|passed|violation|unmet)\b/i);
    }
  });

  it('renders the failure page guest fields only through GuestText: none is a bare JSX expression', () => {
    const trialFiles = ALL.filter(([p]) => /\/trial\/[A-Za-z]+\.tsx$/.test(p));
    for (const [path, src] of trialFiles) {
      expect(code(src), path).not.toMatch(
        /(?<![=$])\{\s*[\w.?]+\.(msg|tag|sdk|sdk_version|catalog_hash|thread|proc|reason|virttime_elapsed|timeslice_stats|per_thread_timeslice|propertyIds|error)\s*\}/,
      );
    }
  });

  it('never reads a guest string outside GuestText in the seed explorer: failing ids are only ever passed to it', () => {
    const seedFiles = ALL.filter(([p]) => /\/seeds\/[A-Za-z]+\.tsx$/.test(p));
    expect(seedFiles.length).toBeGreaterThan(5);
    for (const [path, src] of seedFiles) {
      expect(code(src), path).not.toMatch(/(?<![=$])\{\s*[\w.?]*(failing_ids|propertyLabel|guest\.id)[\w.\[\]?]*\s*\}/);
      expect(code(src), path).not.toMatch(/(title|aria-label|placeholder|alt)=\{[^}]*(guest|propertyLabel|failing_ids)/);
    }
  });

  it('renders guest strings only through GuestText: a guest field is never a bare JSX child', () => {
    for (const [path, src] of ALL) {
      expect(code(src), path).not.toMatch(/>\s*\{[\w.?]*guest\.(id|msg|file|func|reason)\}/);
    }
  });
});

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

  it('never generates the words found, discovered, unassisted or flaky in the explorer or the run page', () => {
    const checked = ALL.filter(([p]) => /\/(explorer|guest|trial)\/|\/wording\.ts$|GuestText|RateCell|TierChips|RunPage/.test(p));
    expect(checked.length).toBeGreaterThan(6);
    for (const [path, src] of checked) expect(code(src), path).not.toMatch(/\b(found|discovered|unassisted|flaky)\b/i);
  });

  it('never generates "reproduced" on the failure page: nothing there re-runs a trial or reads a confirmation', () => {
    const trialFiles = ALL.filter(([p]) => /\/trial\//.test(p));
    expect(trialFiles.length).toBeGreaterThan(8);
    for (const [path, src] of trialFiles) expect(code(src), path).not.toMatch(/reproduc/i);
  });

  it('renders the failure page guest fields only through GuestText: none is a bare JSX expression', () => {
    const trialFiles = ALL.filter(([p]) => /\/trial\/[A-Za-z]+\.tsx$/.test(p));
    for (const [path, src] of trialFiles) {
      expect(code(src), path).not.toMatch(
        /(?<![=$])\{\s*[\w.?]+\.(msg|tag|sdk|sdk_version|catalog_hash|thread|proc|reason|virttime_elapsed|timeslice_stats|per_thread_timeslice|propertyIds|error)\s*\}/,
      );
    }
  });

  it('renders guest strings only through GuestText: a guest field is never a bare JSX child', () => {
    for (const [path, src] of ALL) {
      expect(code(src), path).not.toMatch(/>\s*\{[\w.?]*guest\.(id|msg|file|func|reason)\}/);
    }
  });
});

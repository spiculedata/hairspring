/// <reference types="node" />
import { readdirSync } from 'node:fs';
import { sep } from 'node:path';
import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import { GLOSSARY, GLOSSARY_KEYS, glossaryEntries } from './glossaryData';

function sourceFiles(): string[] {
  const names = (readdirSync(new URL('../', import.meta.url), { recursive: true }) as string[]).map((n) => n.split(sep).join('/'));
  return names.filter((n) => /\.tsx$/.test(n) && !/\.test\.tsx$/.test(n)).sort();
}

describe('glossary data', () => {
  it('gives every entry a term, one definition and a wiki source', () => {
    for (const e of glossaryEntries()) {
      expect(e.term, e.key).not.toBe('');
      expect(e.definition, e.key).toMatch(/\.$/);
      expect(e.wiki, e.key).not.toBe('');
      expect(e.definition, e.key).not.toMatch(/\n/);
    }
  });

  it('keeps each key plain, so it can be an address fragment', () => {
    for (const key of GLOSSARY_KEYS) expect(key).toMatch(/^[a-z][a-z-]*$/);
  });

  it('lists the terms alphabetically for the page', () => {
    const terms = glossaryEntries().map((e) => e.term.toLowerCase());
    expect(terms).toEqual([...terms].sort());
  });

  it('follows the wording rules: no hype, no internal names, never "reproduced" or "found"', () => {
    for (const e of glossaryEntries()) {
      // The server's finding state `not_reproducible` is its own word, as in the source rules.
      expect(e.definition.replaceAll('not_reproducible', ''), e.key).not.toMatch(/reproduc|\bfound\b|\bflaky\b|\bdiscovered\b|powerful|seamless|revolutionary|[\u{1F300}-\u{1FAFF}]/iu);
      expect(e.definition, e.key).not.toMatch(/box hour|experiment|reviewer|\bS\d+\b|\bC\d\b/i);
    }
  });
});

describe('Term keys', () => {
  it('every <Term term="..."> in the source names a glossary entry, and none is built at run time', () => {
    const used: string[] = [];
    for (const file of sourceFiles()) {
      const text = readSource(file);
      expect(text, file).not.toMatch(/<Term\s+term=\{/);
      for (const m of text.matchAll(/<Term\s+term="([^"]*)"/g)) used.push(m[1]);
    }
    expect(used.length).toBeGreaterThanOrEqual(2);
    for (const key of used) expect(Object.hasOwn(GLOSSARY, key), key).toBe(true);
  });
});

/**
 * The manual wiki cross-check. This list is the glossary as (term, where the wiki says the same). When an entry is
 * added, renamed or re-pointed this test fails; update the list, then compare each definition with the wiki page named
 * (https://github.com/spiculedata/hairspring/wiki/Glossary unless another page is named) and say so in the PR.
 */
describe('terms to check against the wiki by hand', () => {
  it('lists every term and its wiki source', () => {
    expect(glossaryEntries().map((e) => `${e.term} -> ${e.wiki}`)).toEqual([
      'Bundle -> Glossary: Bundle',
      'Confirmation -> Glossary: Confirmation',
      'Control -> Glossary: Control',
      'Deterministic -> Glossary: Deterministic',
      'Finding -> Glossary: Finding',
      'Hit -> Glossary: Hit',
      'Inconclusive -> Glossary: Inconclusive',
      'Infra -> Glossary: Infra',
      'Property -> Glossary: Property',
      'Rated trial -> Glossary: Rated trial',
      'Replay -> Glossary: Replay',
      'Run -> Glossary: Run',
      'Schedule -> Glossary: Schedule',
      'Search -> Glossary: Search',
      'Seed -> Glossary: Seed',
      'Strict verify -> Glossary: Confirmation, and docs/determinism.md',
      'Sweep -> Glossary: Sweep',
      'Target -> Glossary: Target',
      'Trial -> Glossary: Trial',
      'Unusable -> Glossary: Unusable',
      'Verdict -> Glossary: Verdict',
      'Witness -> Guest-SDKs-by-Language: witness values',
    ]);
  });
});

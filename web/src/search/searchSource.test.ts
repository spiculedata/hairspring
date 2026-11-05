/// <reference types="node" />
import { readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';

const FILES = (readdirSync(new URL('./', import.meta.url)) as string[]).filter((n) => /\.(tsx?|css)$/.test(n) && !/\.test\.tsx?$/.test(n)).sort();
const CODE_FILES = FILES.filter((n) => /\.tsx?$/.test(n));
const css = readSource('search/search.css');
const phone = css.slice(css.indexOf('@media (max-width: 640px)'));

/** Source with comments removed, so a rule is about code and strings, not prose. */
const code = (source: string): string => source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1');

describe('the search sources', () => {
  it('are all scanned', () => {
    expect(FILES).toEqual(expect.arrayContaining(['SearchBox.tsx', 'search.css', 'searchApi.ts', 'searchHits.ts', 'searchText.ts', 'searchWording.ts', 'useSearchResults.ts']));
  });

  it('never use the words the product does not say, and no hype', () => {
    for (const name of CODE_FILES) {
      const text = code(readSource(`search/${name}`));
      expect(text, name).not.toMatch(/\bfound\b|\b(discovered|unassisted|flaky)\b/i);
      expect(text, name).not.toMatch(/reproduc/i);
      expect(text, name).not.toMatch(/\b(powerful|seamless|revolutionary|blazing|effortless|magic|smart|instant|supercharge)/i);
    }
  });

  it('spell in British English: no -ize endings, "color", "behavior", "gray", "center" or "catalog" in the words', () => {
    for (const name of CODE_FILES) {
      const text = code(readSource(`search/${name}`));
      expect(text, name).not.toMatch(/\b(color|behavior|center|gray|catalog|organize|normalize|initialize)\b/i);
    }
  });

  it('write no emoji', () => {
    for (const name of FILES) expect(readSource(`search/${name}`), name).not.toMatch(/[\u{1F300}-\u{1FAFF}\u{2600}-\u{27BF}]/u);
  });

  it('never set HTML, an inline style or an address from what the server sent', () => {
    for (const name of CODE_FILES) {
      const text = code(readSource(`search/${name}`));
      expect(text, name).not.toMatch(/dangerouslySetInnerHTML|innerHTML|outerHTML|insertAdjacentHTML|document\.write|\beval\(|new Function\(/);
      expect(text, name).not.toMatch(/\bstyle=\{/);
      expect(text, name).not.toMatch(/\bhref=/);
      expect(text, name).not.toMatch(/window\.location\.(href|assign|replace)\s*=|location\.(assign|replace)\(/);
    }
  });

  it('show guest text only through GuestText', () => {
    const box = code(readSource('search/SearchBox.tsx'));
    expect(box).toMatch(/<GuestText text=\{row\.guestText\}/);
    expect(box).not.toMatch(/(?<!text=)\{\s*[\w.?]*guestText\s*\}/);
    expect(box).not.toMatch(/(title|aria-label|placeholder|alt)=\{[^}]*(guest|guestText)/);
  });
});

describe('search.css', () => {
  it('uses the colour tokens only, so both themes and the contrast checks cover it', () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\brgba?\(/);
    expect(css).not.toMatch(/\bhsla?\(/);
    expect(css).not.toMatch(/\b(white|black|red|green|blue|grey|gray)\b\s*[;}]/);
  });

  it('lets the box and its list shrink and wrap instead of widening the page, with no fixed widths', () => {
    expect(css).toMatch(/\.search \{[^}]*min-width: 0/);
    expect(css).toMatch(/\.search-input \{[^}]*max-width: 100%/);
    expect(css).toMatch(/\.search-popup \{[^}]*left: 0; right: 0/);
    expect(css).toMatch(/\.search-hit-main \{[^}]*overflow-wrap: anywhere/);
    expect(css).toMatch(/\.search-hit-detail \{[^}]*overflow-wrap: anywhere/);
    expect(css).toMatch(/\.search-note, \.search-scope \{[^}]*overflow-wrap: anywhere/);
    expect(css).not.toMatch(/(?<!min-|max-)width:\s*\d+(px|rem)/);
  });

  it('takes the whole row under the brand on a phone, so it fits 320 px', () => {
    expect(phone).toMatch(/\.search \{[^}]*flex-basis: 100%/);
    expect(phone).toMatch(/\.search \{[^}]*max-width: none/);
  });

  it('keeps the list inside the screen and scrolls it', () => {
    expect(css).toMatch(/\.search-popup \{[^}]*max-height: min\(24rem, 70vh\)/);
    expect(css).toMatch(/\.search-popup \{[^}]*overflow-y: auto/);
  });

  it('shows focus, and marks the active hit by background and a bar, not by colour alone', () => {
    expect(css).toMatch(/\.search-input:focus-visible \{ outline: 2px solid var\(--focus\)/);
    expect(css).toMatch(/\.search-option-active[^{]*\{[^}]*border-left-color: var\(--accent\)/);
  });
});

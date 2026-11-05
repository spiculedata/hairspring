import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';

const css = readSource('lists/lists.css');
const phone = css.slice(css.indexOf('@media (max-width: 640px)'));

describe('lists.css', () => {
  it('uses the colour tokens only, so both themes and the contrast checks cover it', () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\brgba?\(/);
  });

  it('puts each filter on its own line on a phone, each as wide as the screen allows', () => {
    expect(phone).toMatch(/\.list-filters \{[^}]*grid-template-columns: minmax\(0, 1fr\)/);
    expect(phone).toMatch(/\.filter-actions button, \.load-more button \{[^}]*width: 100%/);
  });

  it('lets controls and their notes shrink and wrap instead of widening the page', () => {
    expect(css).toMatch(/\.filter-field \{[^}]*min-width: 0/);
    expect(css).toMatch(/\.filter-field select, \.filter-field input \{[^}]*max-width: 100%/);
    expect(css).toMatch(/\.filter-hint, \.filter-field \.filter-problem \{[^}]*overflow-wrap: anywhere/);
    expect(css).not.toMatch(/(?<!min-|max-)width:\s*\d+(px|rem)/);
  });

  it('marks an invalid box by border and outline, not by colour alone, and keeps a visible focus ring', () => {
    expect(css).toMatch(/aria-invalid="true"\][^}]*box-shadow/);
    expect(css).toMatch(/:focus-visible \{ outline: 2px solid var\(--focus\)/);
  });

  it('lets a badge in the Failures table wrap at word level, and leaves a narrow screen to the scrolling table region', () => {
    expect(css).toMatch(/table\.failures \.badge \{[^}]*white-space: normal/);
    expect(css).toMatch(/table\.failures \.badge \{[^}]*overflow-wrap: normal/);
    expect(css).not.toMatch(/table\.failures \.badge \{[^}]*overflow-wrap: anywhere/);
  });

  it('shows a busy Load more button as unavailable by style, since it is aria-disabled and not disabled', () => {
    expect(css).toMatch(/\.load-more button\[aria-disabled="true"\] \{[^}]*cursor: default/);
  });
});

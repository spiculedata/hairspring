import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';

const css = readSource('components/tabs.css');
const MEDIA = '@media (max-width: 40rem)';
const mediaStart = css.indexOf(MEDIA);
const base = mediaStart < 0 ? css : css.slice(0, mediaStart);
const media = mediaStart < 0 ? '' : css.slice(mediaStart);

/** The declarations of the first rule whose selector is exactly `selector`, searched in `text`. */
function rule(text: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const found = new RegExp(`(?:^|\\n)\\s*${escaped}\\s*\\{([^}]*)\\}`).exec(text);
  expect(found, `a rule for ${selector}`).not.toBeNull();
  return found![1];
}

describe('tabs.css', () => {
  it('is loaded by the component that renders the tabs', () => {
    expect(readSource('components/Tabs.tsx')).toContain("import './tabs.css';");
  });

  it('marks the selected tab by weight and a thicker underline as well as by colour', () => {
    const selected = rule(base, '.tabs [role="tab"][aria-selected="true"]');
    expect(selected).toMatch(/font-weight:\s*600/);
    expect(selected).toMatch(/border-bottom-color:\s*var\(--accent\)/);
    const tab = rule(base, '.tabs [role="tab"]');
    expect(tab).toMatch(/border-bottom:\s*3px solid transparent/);
  });

  it('gives every tab a touch target of at least 44px', () => {
    expect(rule(base, '.tabs [role="tab"]')).toMatch(/min-height:\s*2\.75rem/);
  });

  it('styles the count after the label', () => {
    expect(rule(base, '.tabs .count')).toMatch(/font-variant-numeric:\s*tabular-nums/);
  });

  it('wraps the tab row on a wide screen and scrolls it sideways on a phone, never overflowing the page', () => {
    expect(rule(base, '.tabs')).toMatch(/flex-wrap:\s*wrap/);
    const phone = rule(media, '.tabs');
    expect(phone).toMatch(/flex-wrap:\s*nowrap/);
    expect(phone).toMatch(/overflow-x:\s*auto/);
    expect(rule(media, '.tabs [role="tab"]')).toMatch(/flex:\s*none/);
  });

  it('keeps the phone block last, so its rules win over the base rules at equal specificity', () => {
    expect(mediaStart).toBeGreaterThanOrEqual(0);
    expect(css.indexOf(MEDIA, mediaStart + 1), 'only one media block').toBe(-1);
    // From the media query to the end of the file there is exactly one block, closed by the last brace.
    expect(media.trim()).toMatch(/^@media[^{]*\{(?:[^{}]*\{[^{}]*\})*\s*\}$/);
    expect(base.trim().length).toBeGreaterThan(0);
  });

  it('uses the colour tokens only', () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/);
    expect(css).toContain('var(--');
  });
});

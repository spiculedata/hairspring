import { describe, expect, it } from 'vitest';
import { CH } from './chars';
import { escapeGuest, GUEST_MAX_EXPANDED, GUEST_MAX_INLINE } from './escape';

const joined = (s: string, max = 512) =>
  escapeGuest(s, max)
    .segments.map((x) => x.text)
    .join('');

describe('escapeGuest', () => {
  it('passes ordinary text through unchanged', () => {
    const out = escapeGuest('committed write missing', 512);
    expect(out.segments).toEqual([{ text: 'committed write missing', escaped: false }]);
    expect(out.truncated).toBe(false);
    expect(out.omitted).toBe(0);
  });

  it('does not touch markup characters: React escapes them as text nodes', () => {
    expect(joined('<script>alert(1)</script>')).toBe('<script>alert(1)</script>');
  });

  it('shows a bidi override as a visible escape and marks it', () => {
    const out = escapeGuest(`abc${CH.RLO}def`, 512);
    expect(out.segments).toEqual([
      { text: 'abc', escaped: false },
      { text: '\\u{202E}', escaped: true },
      { text: 'def', escaped: false },
    ]);
  });

  it('shows ESC and other control characters, including newlines, as visible escapes', () => {
    expect(joined(`${CH.ESC}[31mred${CH.ESC}[0m`)).toBe('\\u{1B}[31mred\\u{1B}[0m');
    expect(joined(`a${CH.LF}b${CH.TAB}c${CH.DEL}${CH.NEL}`)).toBe('a\\u{A}b\\u{9}c\\u{7F}\\u{85}');
    expect(joined(`x${CH.NUL}y${CH.BEL}`)).toBe('x\\u{0}y\\u{7}');
  });

  it('shows line and paragraph separators, zero-width and BOM characters', () => {
    expect(joined(`${CH.LS}${CH.PS}${CH.ZWSP}${CH.ZWJ}${CH.BOM}${CH.LRM}`)).toBe(
      '\\u{2028}\\u{2029}\\u{200B}\\u{200D}\\u{FEFF}\\u{200E}',
    );
  });

  it('shows bidi isolates and a lone surrogate', () => {
    expect(joined(`${CH.LRI}x${CH.PDI}`)).toBe('\\u{2066}x\\u{2069}');
    expect(joined(`a${CH.LONE_SURROGATE}b`)).toBe('a\\u{D800}b');
  });

  it('shows Arabic letter mark, noncharacters and tag characters', () => {
    expect(joined(`${CH.ALM}${CH.NONCHAR}${CH.TAG_BEGIN}`)).toBe('\\u{61C}\\u{FFFE}\\u{E0001}');
  });

  it('shows invisible letters (Hangul fillers, braille blank) that no category rule catches', () => {
    expect(
      joined(`${CH.HANGUL_CHOSEONG_FILLER}${CH.HANGUL_JUNGSEONG_FILLER}${CH.HANGUL_FILLER}${CH.HALFWIDTH_HANGUL_FILLER}${CH.BRAILLE_BLANK}`),
    ).toBe('\\u{115F}\\u{1160}\\u{3164}\\u{FFA0}\\u{2800}');
  });

  it('keeps astral characters whole', () => {
    expect(joined(`ok ${CH.GRIN} ok`)).toBe(`ok ${CH.GRIN} ok`);
  });

  it('truncates with an exact count of what was left out', () => {
    const out = escapeGuest('x'.repeat(600), 512);
    expect(out.truncated).toBe(true);
    expect(out.omitted).toBe(88);
    expect(out.segments.map((s) => s.text).join('')).toBe('x'.repeat(512));
  });

  it('does not split a surrogate pair at the cut', () => {
    const out = escapeGuest(`a${CH.GRIN}${CH.GRIN}`, 2);
    expect(out.segments.map((s) => s.text).join('')).toBe('a');
    expect(out.omitted).toBe(4);
  });

  it('bounds the work by what it shows, not by the input: a 10 MB string is cut at the limit', () => {
    const big = 'A'.repeat(10 * 1024 * 1024);
    const out = escapeGuest(big, GUEST_MAX_INLINE);
    expect(out.segments.map((s) => s.text).join('').length).toBe(GUEST_MAX_INLINE);
    expect(out.omitted).toBe(big.length - GUEST_MAX_INLINE);
  });

  it('counts an escape as the source characters it replaced', () => {
    const out = escapeGuest(CH.RLO.repeat(10), 4);
    expect(out.segments.filter((s) => s.escaped)).toHaveLength(4);
    expect(out.omitted).toBe(6);
  });

  it('exposes limits that match the plan: 512 inline, 64 KiB expanded', () => {
    expect(GUEST_MAX_INLINE).toBe(512);
    expect(GUEST_MAX_EXPANDED).toBe(64 * 1024);
  });
});

/**
 * Display escaping for text that comes from the program under test.
 *
 * Guest text is untrusted, and "plain text" is not safe to show as it is: a bidi override can reorder what a reader
 * sees, an ESC sequence can mean something to a terminal the text is copied into, and a line separator can break a
 * layout. So control, format and separator characters are shown as visible `\u{XXXX}` escapes. Markup characters are
 * left alone here because the only way this output is rendered is as React text nodes, which escape them.
 *
 * The work is bounded by what is shown, never by the size of the input, so a 10 MB string costs the same as a short one.
 */

/** Characters shown inline before the rest is cut (plan 7.2 item 3). */
export const GUEST_MAX_INLINE = 512;
/** Characters shown after "show all"; anything longer is cut with a marker. */
export const GUEST_MAX_EXPANDED = 64 * 1024;

/** One run of output: literal text, or an escape the UI generated. */
export interface Segment {
  readonly text: string;
  readonly escaped: boolean;
}

export interface Escaped {
  readonly segments: readonly Segment[];
  readonly truncated: boolean;
  /** UTF-16 units of the input that were not shown. */
  readonly omitted: number;
}

/** Cc controls, Cf format (bidi, zero-width, BOM), Zl and Zp separators, surrogates, private use and unassigned. */
const MUST_ESCAPE = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\p{Cs}\p{Co}\p{Cn}]/u;

const HEX_RADIX = 16;
const ASTRAL_START = 0x10000;

function escapeOf(codePoint: number): string {
  return `\\u{${codePoint.toString(HEX_RADIX).toUpperCase()}}`;
}

/** Escape `input` for display, showing at most `maxUnits` UTF-16 units of it. */
export function escapeGuest(input: string, maxUnits: number): Escaped {
  const segments: Segment[] = [];
  let literal = '';
  let pos = 0;
  while (pos < input.length) {
    const codePoint = input.codePointAt(pos) ?? 0;
    const width = codePoint >= ASTRAL_START ? 2 : 1;
    if (pos + width > maxUnits) break;
    const char = String.fromCodePoint(codePoint);
    if (MUST_ESCAPE.test(char)) {
      if (literal) segments.push({ text: literal, escaped: false });
      literal = '';
      segments.push({ text: escapeOf(codePoint), escaped: true });
    } else {
      literal += char;
    }
    pos += width;
  }
  if (literal) segments.push({ text: literal, escaped: false });
  const omitted = input.length - pos;
  return { segments, truncated: omitted > 0, omitted };
}

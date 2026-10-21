/**
 * The height of a property row as a function of the browser's root font size.
 *
 * A fixed row height is what makes windowing a sum, and every cell clips its content to the row, so the row has to hold
 * the tallest cell. That is the rate cell of a zero-event UNMET property that also saw failures in inconclusive trials:
 * the figure, a two-line qualifier, an UNMET note and an inconclusive note. Its sizes come from explorer.css and
 * styles.css (a test reads both files and fails if they drift): the figure is at the body size in pixels, the qualifier
 * and the notes are sized in `rem`, so they grow with the browser's default font size and the row must grow with them.
 */
export const DEFAULT_ROOT_FONT_PX = 16;
/** Vertical padding (2 x 8px) plus the 1px row border. */
export const ROW_CHROME_PX = 17;

const BODY_FONT_PX = 15;
const LINE_HEIGHT = 1.25;
const QUALIFIER_REM = 0.85;
const QUALIFIER_LINES = 2;
const NOTE_REM = 0.8;
const NOTES = 2;
/** Room for fonts that round a little taller than the one this was measured with. */
const SLACK_PX = 2;

/** The row height in pixels for a root font size in pixels. */
export function rowHeightPx(rootFontPx: number): number {
  const figure = BODY_FONT_PX * LINE_HEIGHT;
  const qualifier = QUALIFIER_LINES * QUALIFIER_REM * LINE_HEIGHT * rootFontPx;
  const notes = NOTES * NOTE_REM * LINE_HEIGHT * rootFontPx;
  return Math.ceil(ROW_CHROME_PX + figure + qualifier + notes) + SLACK_PX;
}

/** A computed `font-size` such as `20px` as a number; the default for anything that is not a usable size. */
export function parseFontPx(computed: string): number {
  const value = Number.parseFloat(computed);
  return Number.isFinite(value) && value > 0 ? value : DEFAULT_ROOT_FONT_PX;
}

/** Fixed-row-height windowing: which rows of a long list to render for a scroll position. No dependency. */

export interface WindowInput {
  readonly total: number;
  readonly rowHeight: number;
  readonly viewportHeight: number;
  readonly scrollTop: number;
  /** Extra rows rendered above and below the viewport so scrolling does not show blanks. */
  readonly overscan: number;
}

export interface RowWindow {
  /** First rendered row (inclusive). */
  readonly start: number;
  /** One past the last rendered row. */
  readonly end: number;
  /** Height of the blank space standing in for the rows before `start`. */
  readonly padTop: number;
  /** Height of the blank space standing in for the rows from `end` on. */
  readonly padBottom: number;
}

const EMPTY: RowWindow = { start: 0, end: 0, padTop: 0, padBottom: 0 };

export function computeWindow(input: WindowInput): RowWindow {
  const { total, rowHeight, viewportHeight, overscan } = input;
  if (total <= 0 || rowHeight <= 0) return EMPTY;
  const scrollTop = Number.isFinite(input.scrollTop) ? Math.max(0, input.scrollTop) : 0;
  const first = Math.min(total, Math.floor(scrollTop / rowHeight));
  const visible = Math.ceil(Math.max(0, viewportHeight) / rowHeight);
  const start = Math.max(0, first - overscan);
  const end = Math.min(total, first + visible + overscan);
  return { start, end, padTop: start * rowHeight, padBottom: (total - end) * rowHeight };
}

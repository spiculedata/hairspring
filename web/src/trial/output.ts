/**
 * Getting kept output ready to show. The server already keeps at most a 64 KiB tail per stream, but a worker's record
 * is not trusted to have done that, so the page bounds what it renders itself: the last 64 Ki characters, in at most the
 * last 2,000 lines. The cost is set by those caps and not by the size of the input, and what was left out is counted so
 * the page can say so.
 */

/** Characters of output that are ever rendered. */
export const OUTPUT_MAX_CHARS = 64 * 1024;
/** Lines of output that are ever rendered, counted from the end. */
export const OUTPUT_MAX_LINES = 2000;
/** Characters of one line shown before GuestText cuts it. */
export const OUTPUT_LINE_CHARS = 2000;

const LOW_SURROGATE = /^[\udc00-\udfff]/;

export interface PreparedTail {
  readonly lines: readonly string[];
  /** Characters cut from the front to stay within the character cap. */
  readonly charsDropped: number;
  /** Whole lines cut from the front to stay within the line cap. */
  readonly linesDropped: number;
}

export function prepareTail(text: string): PreparedTail {
  const start = Math.max(0, text.length - OUTPUT_MAX_CHARS);
  let kept = text.slice(start);
  // A cut that lands inside a surrogate pair would leave half a character at the front.
  const split = LOW_SURROGATE.test(kept);
  if (split) kept = kept.slice(1);
  const charsDropped = start + (split ? 1 : 0);
  const pieces = kept.split('\n');
  if (pieces.at(-1) === '') pieces.pop();
  const linesDropped = Math.max(0, pieces.length - OUTPUT_MAX_LINES);
  return { lines: pieces.slice(linesDropped), charsDropped, linesDropped };
}

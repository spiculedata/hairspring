/**
 * The seed strip as pure functions: joining the server's pages, binning for the pixels available, and keyboard moves.
 *
 * A strip is a string with one character per judged trial, in judging order. The server's codes are `p` pass, `v`
 * violation, `i` inconclusive, `u` unusable, `x` infrastructure error and `e` excluded (judged a pass but degraded or
 * unreported, so never shown as one). This file adds `-` for a trial that has no stored row. Nothing here draws.
 */

export type StripCode = 'p' | 'v' | 'i' | 'u' | 'x' | 'e' | '-';

export const STRIP_CODES: readonly StripCode[] = ['v', 'x', 'u', 'i', 'e', '-', 'p'];
export const UNKNOWN_CODE: StripCode = '-';
/** The most trials the strip holds. A run past it shows its first trials, and the page says so. */
export const MAX_STRIP_TRIALS = 250_000;
/** Bins moved by the page keys. */
const PAGE_STEP = 10;

/** Higher is worse. A bin of many trials is drawn as its worst trial, so a failure is never averaged away. */
const SEVERITY: Readonly<Record<StripCode, number>> = { v: 6, x: 5, u: 4, i: 3, e: 2, '-': 1, p: 0 };

export const severity = (code: StripCode): number => SEVERITY[code];

export interface StripState {
  readonly codes: string;
  /** The sequence number the next page should start at. */
  readonly nextSeq: number;
  /** Some trials were left out because the strip is at its cap. */
  readonly truncated: boolean;
}

export const EMPTY_STRIP: StripState = { codes: '', nextSeq: 0, truncated: false };

const KNOWN = /[^pviuxe]/g;

/**
 * Adds a page to the strip. The result is a new state; the old one is never changed.
 *
 * - Codes the page should not hold become `-`, so an unknown code is never read as a pass.
 * - A gap between pages is filled with `-`: those trials have no stored row.
 * - A page that starts before the strip's next sequence number is ignored, so a repeated response cannot add trials twice.
 * - The strip stops at [`MAX_STRIP_TRIALS`], however large a page or gap the server sends.
 */
export function appendStripPage(state: StripState, page: { first_seq: number; codes: string }): StripState {
  const { first_seq: first } = page;
  if (!Number.isSafeInteger(first) || first < state.nextSeq) return state;
  if (page.codes.length === 0) return state;
  const room = MAX_STRIP_TRIALS - state.codes.length;
  const fill = UNKNOWN_CODE.repeat(Math.min(first - state.nextSeq, room));
  const wanted = fill + page.codes.replace(KNOWN, UNKNOWN_CODE);
  const added = wanted.slice(0, Math.max(0, room));
  return {
    codes: state.codes + added,
    nextSeq: first + page.codes.length,
    truncated: state.truncated || added.length < fill.length + page.codes.length,
  };
}

export interface StripBin {
  /** First trial in the bin (inclusive). */
  readonly start: number;
  /** One past the last trial in the bin. */
  readonly end: number;
  /** The most severe class among its trials. */
  readonly worst: StripCode;
}

const asCode = (c: string | undefined): StripCode => (c !== undefined && c in SEVERITY ? (c as StripCode) : UNKNOWN_CODE);

/** Splits the strip into at most `maxBins` runs of consecutive trials, in one pass over the codes. */
export function binStrip(codes: string, maxBins: number): StripBin[] {
  const n = codes.length;
  const count = Math.min(n, Math.floor(maxBins));
  if (!(count > 0)) return [];
  const bins: StripBin[] = [];
  for (let i = 0; i < count; i++) {
    const start = Math.floor((i * n) / count);
    const end = Math.floor(((i + 1) * n) / count);
    let worst: StripCode = 'p';
    for (let j = start; j < end; j++) {
      const code = asCode(codes[j]);
      if (SEVERITY[code] > SEVERITY[worst]) worst = code;
    }
    bins.push({ start, end, worst });
  }
  return bins;
}

export type CodeCounts = Readonly<Record<StripCode, number>>;

/** How many trials have each code. Every code is present, with zero when there are none. */
export function countCodes(codes: string): CodeCounts {
  const counts: Record<StripCode, number> = { p: 0, v: 0, i: 0, u: 0, x: 0, e: 0, '-': 0 };
  for (let i = 0; i < codes.length; i++) counts[asCode(codes[i])]++;
  return counts;
}

/** The first trial in the bin that has the bin's worst class: where "open this bin" should land. */
export function worstTrial(codes: string, bin: StripBin): number {
  for (let i = bin.start; i < bin.end; i++) if (asCode(codes[i]) === bin.worst) return i;
  return bin.start;
}

/** Where a key moves the strip's position, or `undefined` for a key it does not handle. */
export function keyToBin(key: string, index: number, bins: number): number | undefined {
  if (bins <= 0) return undefined;
  const last = bins - 1;
  const clamp = (i: number) => Math.min(last, Math.max(0, i));
  switch (key) {
    case 'ArrowRight':
      return clamp(index + 1);
    case 'ArrowLeft':
      return clamp(index - 1);
    case 'PageDown':
      return clamp(index + PAGE_STEP);
    case 'PageUp':
      return clamp(index - PAGE_STEP);
    case 'Home':
      return 0;
    case 'End':
      return last;
    default:
      return undefined;
  }
}

/** The nearest bin after (`dir` 1) or before (`dir` -1) `from` that is not a plain pass. */
export function nextNotableBin(bins: readonly StripBin[], from: number, dir: 1 | -1): number | undefined {
  for (let i = from + dir; i >= 0 && i < bins.length; i += dir) if (bins[i]?.worst !== 'p') return i;
  return undefined;
}

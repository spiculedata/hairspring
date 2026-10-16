import type { RateCellData } from '../types';
import { formatPct, rateParts, rateText } from '../wording';

const PERCENT = 100;

function toPercent(fraction: number): number {
  return Number.isFinite(fraction) ? Math.min(PERCENT, Math.max(0, fraction * PERCENT)) : 0;
}

/** A 0-100% track with the interval drawn on it. The text beside it carries the same numbers. */
function IntervalBar({ lower, upper }: { lower: number; upper: number }) {
  const left = toPercent(lower);
  const width = Math.max(0, toPercent(upper) - left);
  return (
    <span className="rate-bar" role="img" aria-label={`interval ${formatPct(lower)} to ${formatPct(upper)}`}>
      <span className="rate-bar-fill" style={{ left: `${left}%`, width: `${width}%` }} />
    </span>
  );
}

/**
 * One property's rate. Always `k/n` with its interval; zero events show the exact one-sided bound; nothing rated is
 * "unrated"; a run that stopped at its first failure shows counts and no rate. The wording is in wording.ts.
 *
 * The cell is two lines: the figure (the counts, one line, never clipped) and the qualifier (which may wrap and is
 * clamped in a fixed-height row). The title carries the whole text, so a clamped qualifier can still be read.
 */
export function RateCell({ cell }: { cell: RateCellData }) {
  const { figure, qualifier } = rateParts(cell);
  const showBar = cell.kind === 'rate' && cell.n > 0;
  return (
    <span className={`rate-cell rate-${cell.kind}`} title={rateText(cell)}>
      <span className="rate-figure">{figure}</span>
      {qualifier !== undefined && <span className="rate-qualifier">{qualifier}</span>}
      {showBar && <IntervalBar lower={cell.lower} upper={cell.upper} />}
    </span>
  );
}

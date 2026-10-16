import type { RateCellData } from '../types';
import { formatPct, rateText } from '../wording';

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
 * One property's rate. Always `k/n` with its interval; zero events show the 3/n bound; nothing rated is "unrated";
 * a run that stopped at its first failure shows counts and no rate. The wording is in wording.ts.
 */
export function RateCell({ cell }: { cell: RateCellData }) {
  const showBar = cell.kind === 'rate' && cell.n > 0;
  return (
    <span className={`rate-cell rate-${cell.kind}`}>
      <span className="rate-text">{rateText(cell)}</span>
      {showBar && <IntervalBar lower={cell.lower} upper={cell.upper} />}
    </span>
  );
}

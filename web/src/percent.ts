/**
 * Percentages rounded in the direction that never overstates what the counts show. This is the rule of the CLI report
 * (crates/hairspring-props/src/percent.rs), and both are tested against the same cases
 * (crates/hairspring-props/tests/percent_cases.txt).
 *
 * An upper bound is rounded up and a lower bound down, so a shown bound is never tighter than the real one. A non-zero
 * figure below 0.01% is written `<0.01%` instead of being rounded to a number that is not true.
 */

/** Hundredths of a percent in a fraction of 1. */
const HUNDREDTHS_PER_UNIT = 10_000;
/** Absorbs float noise so that `0.0149` is `1.49%` in every direction. */
const NOISE = 1e-9;
const LESS_THAN_SHOWN = '<0.01%';

type Rounding = (hundredths: number) => number;

/** `x` in hundredths of a percent, clamped to 0..=1. NaN becomes `ifInvalid`, the value that overstates least. */
function hundredths(x: number, ifInvalid: number): number {
  const value = Number.isNaN(x) ? ifInvalid : x;
  return Math.min(1, Math.max(0, value)) * HUNDREDTHS_PER_UNIT;
}

function show(h: number, round: Rounding): string {
  if (h > 0 && h < 1) return LESS_THAN_SHOWN;
  return `${(round(h) / 100).toFixed(2)}%`;
}

/** An upper bound, rounded up. */
export function upperPct(x: number): string {
  return show(hundredths(x, 1), (h) => Math.max(0, Math.ceil(h - NOISE)));
}

/** A lower bound, rounded down. */
export function lowerPct(x: number): string {
  return show(hundredths(x, 0), (h) => Math.floor(h + NOISE));
}

/** A point estimate, rounded to nearest (halves away from zero, as `f64::round` does). */
export function nearestPct(x: number): string {
  return show(hundredths(x, 0), Math.round);
}

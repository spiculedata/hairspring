import { DETAIL_TRUNCATED_NOTE, NO_DETAIL_NOTE, NOT_A_RATE_TEXT, PERTURBING_NOTE, STOPPED_TRIAL_NOTE } from '../wording';

export interface CaveatFacts {
  readonly stoppedAtFirstFailure: boolean;
  readonly perturbing: boolean;
  /** No detail record was kept for the trial. */
  readonly noDetail: boolean;
  /** A detail record was kept but its property list was left out for size. */
  readonly truncated: boolean;
}

/**
 * The caveats that apply to this trial, most important first: what changes how the result may be read comes before what
 * only limits what is shown. The page leads with the first and keeps the rest under "About this page".
 */
export function rankedCaveats(f: CaveatFacts): readonly string[] {
  return [
    f.stoppedAtFirstFailure ? `${STOPPED_TRIAL_NOTE} (${NOT_A_RATE_TEXT})` : undefined,
    f.perturbing ? `${PERTURBING_NOTE}.` : undefined,
    f.noDetail ? NO_DETAIL_NOTE : undefined,
    f.truncated ? DETAIL_TRUNCATED_NOTE : undefined,
  ].filter((c): c is string => c !== undefined);
}

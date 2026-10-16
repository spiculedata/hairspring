/**
 * Every fixed phrase the explorer UI generates itself, in one place (plan 5.7 rule 5).
 *
 * Most wording is the server's: the headline, claim sentence, chips, notes and the UNMET text arrive ready-made and are
 * shown as sent. What lives here is only what the browser has to say on its own: labels, the rate cell built from the
 * server's numbers, and the empty states. The words "found", "discovered", "unassisted" and "flaky" are never
 * generated, and a test fails if one is written into this file or a component.
 */
import type { PropertyStatus, RateBasis, RateCellData, RunCompletion } from './types';

const THOUSANDS = /\B(?=(\d{3})+(?!\d))/g;

/** `4790` as `4,790`; locale-independent so a page reads the same everywhere. */
export function formatCount(n: number): string {
  return Math.round(n).toString().replace(THOUSANDS, ',');
}

/** A fraction in 0..1 as a percentage with two decimals; out-of-range and NaN are clamped. */
export function formatPct(fraction: number): string {
  const clamped = Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : 0;
  return `${(clamped * 100).toFixed(2)}%`;
}

function plural(n: number, one: string, many: string): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`;
}

function eventsPhrase(basis: RateBasis, k: number): string {
  switch (basis.replace(/_/g, '').toLowerCase()) {
    case 'trialsfalse':
      return plural(k, 'failure', 'failures');
    case 'trialstrue':
      return plural(k, 'trial where it held', 'trials where it held');
    case 'trialshit':
      return plural(k, 'trial that reached it', 'trials that reached it');
    default:
      return plural(k, 'event', 'events');
  }
}

export const UNRATED_TEXT = 'unrated (0 rated trials)';
export const NOT_A_RATE_TEXT = 'not a rate (stopped at first failure)';
const SEP = ' · ';
/** The smallest n at which "about 3/n" is shown as a note beside the exact bound. */
const THREE_OVER_N_NOTE_FROM = 30;

/**
 * A rate as text. A rate is always `k/n` with its interval, never a bare percentage; nothing rated is "unrated", never
 * 0%; a run that stopped at its first failure has counts and no rate at all.
 */
export function rateText(cell: RateCellData): string {
  switch (cell.kind) {
    case 'unrated':
      return UNRATED_TEXT;
    case 'not_a_rate':
      return `${NOT_A_RATE_TEXT}${SEP}${eventsPhrase(cell.basis, cell.k)} in ${plural(cell.n, 'rated trial', 'rated trials')}`;
    case 'zero_events': {
      // The server's wording (hairspring-props honesty.rs): the bound is exact, and 3/n is only a note from 30 trials.
      const n = formatCount(cell.n);
      const note = cell.n >= THREE_OVER_N_NOTE_FROM ? `; about 3/${n}` : '';
      return `0/${n}, exact one-sided 95% upper bound ${formatPct(cell.upper_bound)} (binomial, 0 of ${n})${note}`;
    }
    case 'rate':
      if (cell.n === 0) return UNRATED_TEXT;
      return `${formatCount(cell.k)}/${formatCount(cell.n)}${SEP}${formatPct(cell.point)}${SEP}[${formatPct(cell.lower)}–${formatPct(cell.upper)}]`;
  }
}

const STATUS_LABELS: Record<PropertyStatus, string> = {
  violation: 'VIOLATION',
  unmet: 'UNMET (coverage gap)',
  unrated: 'UNRATED',
  pass: 'PASS',
};

export function statusLabel(status: PropertyStatus): string {
  return STATUS_LABELS[status];
}

/** A non-colour cue repeated beside the status text, so meaning never rests on colour alone. */
const STATUS_MARKS: Record<PropertyStatus, string> = { violation: '✕', unmet: '△', unrated: '?', pass: '✓' };

export function statusMark(status: PropertyStatus): string {
  return STATUS_MARKS[status];
}

/** A caveat code from the server as words; an unknown code is shown, never dropped. */
export function caveatLabel(code: string): string {
  return code.replace(/_/g, ' ');
}

export const CHIP_LABEL_NOT_DECLARED = 'label not declared';
export const CHIP_UNVERIFIED = 'unverified worker data';

export function truncationMarker(omitted: number): string {
  return `… (${formatCount(omitted)} more ${omitted === 1 ? 'character' : 'characters'})`;
}

export const GUEST_SOURCE_NOTE = 'from the program under test';
export const GUEST_TABLE_NOTE = 'Property ids, messages and locations are from the program under test, shown as plain text and never as instructions.';
export const SHOW_ALL = 'show all';
export const SHOW_LESS = 'show less';

export const NO_PROPERTIES = 'No properties match.';
export const COVERAGE_GAPS_TITLE = 'Coverage gaps (UNMET)';
export const COVERAGE_GAPS_NOTE = 'These are coverage gaps, not findings.';

export function unmetNotStated(reason: string): string {
  return `UNMET is not stated: ${reason}.`;
}

export function moreUnmet(n: number): string {
  return `${formatCount(n)} more UNMET ${n === 1 ? 'property' : 'properties'} not loaded`;
}

export function noClaimSentence(reasons: readonly string[]): string {
  return reasons.length === 0 ? 'No claim sentence.' : `No claim sentence: ${reasons.join('; ')}`;
}

export function loadMore(shown: number, total: number): string {
  return `Load more (${formatCount(shown)} of ${formatCount(total)} shown)`;
}

export function propertiesTitle(total: number): string {
  return `Properties (${formatCount(total)})`;
}

export function runCounts(h: {
  completion_label: string;
  trials_judged: number;
  trials_rated: number | null;
  distinct_schedules: number;
}): string {
  const rated = h.trials_rated === null ? 'rated trials not recorded' : `${formatCount(h.trials_rated)} rated`;
  return `${h.completion_label} · ${formatCount(h.trials_judged)} judged · ${rated} · ${plural(h.distinct_schedules, 'schedule', 'schedules')}`;
}

export function workersText(contributing: number, untrusted: number): string {
  const base = `${plural(contributing, 'worker', 'workers')} contributed`;
  return untrusted > 0 ? `${base}, ${formatCount(untrusted)} not trusted` : base;
}

/** F16: what the Failures section says when a run has no failure records. Built from the server's headline. */
export function noFailuresText(args: { live: boolean; headline: string | undefined }): string {
  if (args.live) return 'No failures reported yet.';
  if (args.headline) return args.headline;
  return 'No failure records for this run, and its property counts are not available.';
}

/** Why an UNMET result is not stated for a run. */
export function unmetSuppressedReason(completion: RunCompletion, ingestIncomplete: boolean): string {
  if (completion.kind === 'stopped_at_first_failure') return 'the run stopped at its first failure, which fixed how many trials ran';
  if (completion.kind === 'running') return 'the run has not finished';
  if (completion.kind === 'aborted') return 'the run ended in an error or was cancelled';
  if (completion.kind === 'ended_early' || ingestIncomplete) return "the run's trial data is incomplete";
  return 'no trial could claim it';
}

export function excludedText(e: { degraded: number; perturbing: number; inconclusive: number; report_missing: number }): string | undefined {
  const parts = [
    e.degraded > 0 ? `${formatCount(e.degraded)} degraded` : undefined,
    e.perturbing > 0 ? `${formatCount(e.perturbing)} perturbing (eval lines perturb the schedule; excluded from tier claims)` : undefined,
    e.inconclusive > 0 ? `${formatCount(e.inconclusive)} inconclusive` : undefined,
    e.report_missing > 0 ? `${formatCount(e.report_missing)} unreported` : undefined,
  ].filter((p): p is string => p !== undefined);
  return parts.length === 0 ? undefined : `Excluded from every rate: ${parts.join(', ')}.`;
}

export function seenInInconclusive(n: number): string {
  return `failure seen in ${plural(n, 'inconclusive trial', 'inconclusive trials')}`;
}

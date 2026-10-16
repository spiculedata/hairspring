/**
 * Every fixed phrase the explorer UI generates itself, in one place (plan 5.7 rule 5).
 *
 * Most wording is the server's: the headline, claim sentence, chips, notes and the UNMET text arrive ready-made and are
 * shown as sent. What lives here is only what the browser has to say on its own: labels, the rate cell built from the
 * server's numbers, and the empty states. The words "found", "discovered", "unassisted" and "flaky" are never
 * generated, and a test fails if one is written into this file or a component.
 */
import { STRIP_CODES, type CodeCounts, type StripCode } from './seeds/strip';
import type { TrialClass, TrialRow } from './trial/types';
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

export interface RateParts {
  /** The counts, short enough for one line: `k/n`, or the raw counts when there is no rate. Never clipped. */
  readonly figure: string;
  /** Everything that qualifies the figure (interval, bound, why there is no rate). May be long, so it may be clamped. */
  readonly qualifier: string | undefined;
}

/** `rateText` split so the layout can keep the figure readable and clamp only the qualifier. */
export function rateParts(cell: RateCellData): RateParts {
  switch (cell.kind) {
    case 'unrated':
      return { figure: UNRATED_TEXT, qualifier: undefined };
    case 'not_a_rate':
      return {
        figure: `${eventsPhrase(cell.basis, cell.k)} in ${plural(cell.n, 'rated trial', 'rated trials')}`,
        qualifier: NOT_A_RATE_TEXT,
      };
    case 'zero_events': {
      const n = formatCount(cell.n);
      const note = cell.n >= THREE_OVER_N_NOTE_FROM ? `; about 3/${n}` : '';
      return { figure: `0/${n}`, qualifier: `exact one-sided 95% upper bound ${formatPct(cell.upper_bound)} (binomial, 0 of ${n})${note}` };
    }
    case 'rate':
      if (cell.n === 0) return { figure: UNRATED_TEXT, qualifier: undefined };
      return {
        figure: `${formatCount(cell.k)}/${formatCount(cell.n)}${SEP}${formatPct(cell.point)}`,
        qualifier: `[${formatPct(cell.lower)}–${formatPct(cell.upper)}]`,
      };
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

// ---- Failure page (one trial) ----

const VERDICT_LABELS: Record<string, string> = {
  pass: 'passed',
  violation: 'failed',
  inconclusive: 'was inconclusive',
  unusable: 'was unusable',
  infra: 'hit an infrastructure error',
};

/** "Failed in seed S" is an observation about one seed; it is not a rate and it does not say "reproduced". */
export function trialTitle(verdict: string, seed: string): string {
  const label = VERDICT_LABELS[verdict];
  return label === undefined ? `Seed ${seed}: ${verdict}` : `Seed ${seed} ${label}`;
}

export function failedInSeed(seed: string): string {
  return `failed in seed ${seed}`;
}

export const PER_THREAD_NOTE =
  "Ordinals are per thread: each is a count of that thread's own evaluations. No order across threads is recorded, and lanes are not aligned in time.";
export const LANE_SCALE_NOTE = "Each lane is drawn to its own thread's evaluation count; a longer lane does not mean more time.";
export const PERTURBING_NOTE = 'eval lines perturb the schedule; this trial is excluded from tier claims';

export function witnessText(ord: number, ordinal: string, evals: number | undefined): string {
  const of = evals === undefined ? '' : ` of the ${formatCount(evals)} that thread made`;
  return `evaluation number ${formatCount(ord)} (its ${ordinal})${of}`;
}

export const WITNESS_VALUES_NOTE = 'as the program recorded them with that evaluation';

export const STOPPED_TRIAL_NOTE =
  'This run stopped at its first failure. One failing seed says that seed failed; it is not a rate and does not say how often this property fails.';
export const NOT_REPLAYED_NOTE =
  'This page does not re-run anything. It shows what the worker recorded for this one seed. Replay on the run page re-runs a stored bundle and reports only whether that bundle still fails.';
export const NO_DETAIL_NOTE = 'No detail was kept for this trial, so only its row is shown. Detail is kept for failing, degraded, early and sampled trials.';
export const DETAIL_TRUNCATED_NOTE = 'The property list was too large to keep and is left out; processes, threads and health are shown.';
export const RAW_EVENTS_NOTE = 'Raw event lines are not stored. The counts and witnesses above are what the worker folded from them.';
export const NO_SCHEDULE_NOTE = 'No scheduler summary was kept for this trial; the figures below come from its row.';
export const NO_FAILING_PROPERTY = 'No kept property failed in this trial.';

export function outputOmitted(bytes: number): string {
  return `${formatCount(bytes)} ${bytes === 1 ? 'byte' : 'bytes'} cut from the middle of the stream; its start and end were kept`;
}

export function outputTrimmed(chars: number, lines: number): string {
  const parts = [
    chars > 0 ? `${formatCount(chars)} earlier ${chars === 1 ? 'character' : 'characters'}` : undefined,
    lines > 0 ? `${formatCount(lines)} earlier ${lines === 1 ? 'line' : 'lines'}` : undefined,
  ].filter((p): p is string => p !== undefined);
  return `… ${parts.join(' and ')} not shown here; download the stream for the whole kept tail`;
}

export function seenInInconclusive(n: number): string {
  return `failure seen in ${plural(n, 'inconclusive trial', 'inconclusive trials')}`;
}

// ---- Seed explorer (plan 5.3) ----

const CLASS_LABELS: Record<TrialClass, string> = {
  pass: 'pass',
  violation: 'violation',
  inconclusive: 'inconclusive',
  unusable: 'unusable',
  infra: 'infrastructure error',
  excluded: 'excluded',
};

/** A non-colour cue for each class, repeated beside its words. */
const CLASS_MARKS: Record<TrialClass, string> = { pass: '✓', violation: '✕', inconclusive: '?', unusable: '⊘', infra: '!', excluded: '◌' };

export const classLabel = (c: TrialClass): string => CLASS_LABELS[c];
export const classMark = (c: TrialClass): string => CLASS_MARKS[c];

const CODE_CLASSES: Record<StripCode, TrialClass | undefined> = {
  p: 'pass',
  v: 'violation',
  i: 'inconclusive',
  u: 'unusable',
  x: 'infra',
  e: 'excluded',
  '-': undefined,
};
const NO_ROW_LABEL = 'no stored row';

export function codeLabel(code: StripCode): string {
  const c = CODE_CLASSES[code];
  return c === undefined ? NO_ROW_LABEL : classLabel(c);
}

export function codeMark(code: StripCode): string {
  const c = CODE_CLASSES[code];
  return c === undefined ? '·' : classMark(c);
}

/** Why a trial the engine judged a pass is left out of every rate. */
function excludedWhy(t: { degraded: boolean; report_missing: boolean }): string {
  if (t.degraded) return 'degraded';
  if (t.report_missing) return 'no report';
  return 'not rated';
}

/** The verdict cell: the class in words with its mark, why a pass was excluded, and whether the trial is perturbing. */
export function verdictCellText(t: TrialRow): string {
  const why = t.class === 'excluded' ? ` (${excludedWhy(t)})` : '';
  const perturbing = t.perturbing ? ' · perturbing' : '';
  return `${classMark(t.class)} ${classLabel(t.class)}${why}${perturbing}`;
}

/** What the engine called the trial, for a tooltip: an excluded trial was judged a pass and is not counted as one. */
export function verdictTitle(t: TrialRow): string {
  return t.class === 'excluded' ? 'Judged a pass by the engine, but left out of every rate' : `Verdict: ${t.verdict}`;
}

export function keptText(kept: string): string {
  switch (kept) {
    case 'kept':
      return 'yes';
    case 'none':
      return 'no';
    case 'dropped_cap':
      return 'dropped (size cap)';
    default:
      return 'unknown';
  }
}

export const NO_VALUE = '–';

export function moreFailing(extra: number): string {
  return `+${formatCount(extra)} more`;
}

export function stripBinText(start: number, end: number, counts: CodeCounts): string {
  const parts = STRIP_CODES.filter((c) => counts[c] > 0).map((c) => `${formatCount(counts[c])} ${codeLabel(c)}`);
  const where = end - start === 1 ? `Trial ${formatCount(start)}` : `Trials ${formatCount(start)} to ${formatCount(end - 1)}`;
  return `${where}: ${parts.join(', ')}`;
}

export function stripSummary(counts: CodeCounts): string {
  const total = STRIP_CODES.reduce((sum, c) => sum + counts[c], 0);
  const parts = STRIP_CODES.filter((c) => counts[c] > 0).map((c) => `${formatCount(counts[c])} ${codeLabel(c)}`);
  return parts.length === 0 ? `${formatCount(total)} judged` : `${formatCount(total)} judged: ${parts.join(', ')}`;
}

export const STRIP_TITLE = 'Judged trials';
export const STRIP_NOTE =
  'One mark per judged trial, in judging order. When there are more trials than pixels, a mark stands for a group and shows the worst trial in it. Height and the table say the same as colour.';
export const STRIP_KEYS_NOTE = 'Arrow keys move one mark, Page Up and Page Down move ten, Home and End go to the ends, N and P jump to the next or previous trial that is not a pass, Enter opens the table there.';
export const STRIP_COUNTS_CAPTION = 'Judged trials by class';

export function stripTruncated(n: number): string {
  return `The strip shows the first ${formatCount(n)} trials only; later ones are in the table.`;
}

export function stoppedRunNote(lastSeq: number | undefined): string {
  const after = lastSeq === undefined ? 'after the failure' : `after trial ${formatCount(lastSeq)}`;
  return `This run stopped at its first failure. Trials that were running beside the failing one ${after} were not judged, so this list is not a sample of the seed space and its counts are not a rate.`;
}

/** The link text from a property to its seed list: what the list holds depends on the property's status. */
export function seedListLabel(status: PropertyStatus): string {
  if (status === 'violation') return 'failing seeds';
  return status === 'unmet' ? 'seeds that did not reach it' : 'seeds that reached it';
}

export const BROWSE_SEEDS = 'Browse every judged seed';
export const SEEDS_TITLE = 'Seeds';
export const TRIAL_TABLE_LABEL = 'Judged trials';
export const GUEST_IDS_NOTE = 'Property ids are from the program under test, shown as plain text and never as instructions.';
export const NO_TRIALS_MATCH = 'No judged trial matches these filters.';
export const NO_TRIALS = 'This run has no stored trial rows. Trial rows are kept for recent runs only.';
export const FILTER_ALL = 'All';
export const FILTERS_CLEAR = 'Clear filters';
export const FP_FILTER_LABEL = 'Schedule fingerprint';
export const FP_FILTER_HINT = '16 to 64 hex digits';
export const FP_FILTER_INVALID = 'A fingerprint is 16 to 64 hex digits.';
export const PROPERTY_SEARCH_LABEL = 'Property id starts with';
export const PROPERTY_PICK_NONE = 'No property id starts with that.';
export const START_AT_NOTE = 'Earlier trials are not shown.';
export const BACK_TO_START = 'Show from the first trial';
export const LOADING_TRIALS = 'Loading trials…';
export const SCROLL_HINT_SEEDS = 'Scroll sideways to see every column.';

export function propertyModeLabel(mode: 'fails' | 'hits' | 'misses'): string {
  return mode === 'fails' ? 'failed in' : mode === 'hits' ? 'reached by' : 'not reached by';
}

export function loadMoreTrials(shown: number, total: number | undefined): string {
  return total === undefined ? `Load more (${formatCount(shown)} shown)` : `Load more (${formatCount(shown)} of ${formatCount(total)} shown)`;
}

export function scannedNoMatch(throughSeq: number): string {
  return `No match yet in the trials read so far (up to trial ${formatCount(throughSeq)}). Reading is bounded per request; continue to read further.`;
}

export function continueReading(throughSeq: number): string {
  return `Read further (after trial ${formatCount(throughSeq)})`;
}

export function runningNote(judged: number): string {
  return `The run is still going: ${formatCount(judged)} trials judged so far. The list grows as trials are judged.`;
}

import { FINDING_STATUSES, type Excluded, FindingStatus, RateCellData, RunCompletion, SweepSummary } from '../types';
import { excludedText, rateText } from '../wording';
import * as w from './runPanelWording';

/** Everything the panel may say is built from these server fields. Nothing else is read. */
export interface RunPanelInput {
  /** `run.state` as sent. */
  readonly state: string;
  /** `run.trials`. */
  readonly trials: number;
  /** The property page's `header.completion`; absent until the page has loaded. */
  readonly completion: RunCompletion | undefined;
  /** The property page's `header.completion_label`, the server's own words. */
  readonly completionLabel?: string;
  /** `GET /api/runs/{id}` `sweep`: absent for a search, a live run and a sweep the server withholds a summary for. */
  readonly sweep?: SweepSummary;
  readonly sweepPartial: boolean;
  /** Failure records the run holds. */
  readonly failureCount: number;
  /** `properties.first_failing_seed`. */
  readonly firstFailingSeed?: string | null;
  /** The rate cell of the failed property, given only when the loaded page holds the one failed property in full. */
  readonly failedRates: readonly RateCellData[];
  /** `counts_by_status.violation` of the property page, when it has loaded. */
  readonly failedPropertyCount?: number;
  /** Each failure's finding status; `undefined` where the server did not send one. */
  readonly findings: ReadonlyArray<FindingStatus | undefined>;
  readonly excluded?: Excluded;
  readonly ingestIncomplete?: boolean;
  readonly unverifiedWorkerData?: boolean;
}

export interface RunPanelModel {
  readonly shows: readonly string[];
  readonly doesNotShow: readonly string[];
}

const LIVE_STATES = new Set(['queued', 'running']);
const ERRORED_STATES = new Set(['error', 'cancelled']);
const FINISHED_STATES = new Set(['passed', 'failed']);
const CONFIRMED: ReadonlySet<string> = new Set<FindingStatus>(['confirmed', 'verified']);
const KNOWN_FINDINGS: ReadonlySet<string> = new Set<string>(FINDING_STATUSES);

const only = (lines: ReadonlyArray<string | undefined>): string[] => lines.filter((l): l is string => l !== undefined);

function isLive({ state, completion }: RunPanelInput): boolean {
  return LIVE_STATES.has(state) || completion?.kind === 'running';
}

/** The run ended short of its plan, or the server marks its data incomplete: figures are counts only. */
function isStoppedEarly(i: RunPanelInput): boolean {
  if (i.sweepPartial || i.ingestIncomplete === true) return true;
  switch (i.completion?.kind) {
    case 'aborted':
    case 'ended_early':
    case 'digest_allowance_used_up':
      return true;
    case 'time_or_step_limited':
      return i.completion.trials_budget > 0 && i.completion.trials_run < i.completion.trials_budget;
    default:
      return false;
  }
}

function hasFailure(i: RunPanelInput): boolean {
  return i.failureCount > 0 || Boolean(i.firstFailingSeed) || i.state === 'failed' || i.completion?.kind === 'stopped_at_first_failure';
}

function failureLines(i: RunPanelInput): string[] {
  if (i.firstFailingSeed) return [w.failureSeed(i.firstFailingSeed)];
  if (i.failureCount > 0) return [w.failureRecords(i.failureCount)];
  return [i.completion?.kind === 'stopped_at_first_failure' ? w.STOPPED_AT_FIRST_FAILURE_SHOWS : w.MARKED_FAILED_SHOWS];
}

function earlyResult(i: RunPanelInput): RunPanelModel | undefined {
  if (isLive(i)) return { shows: [w.LIVE_SHOWS], doesNotShow: [w.LIVE_NOT] };
  if (ERRORED_STATES.has(i.state)) {
    const counts = i.sweep ? w.sweepPartialCounts(i.sweep.trials, i.sweep.violations) : w.trialsRanBeforeStop(i.trials);
    return {
      shows: only([w.ERRORED_SHOWS, (i.sweep?.trials ?? i.trials) > 0 ? counts : undefined, ...(i.failureCount > 0 ? [w.failureRecords(i.failureCount)] : [])]),
      doesNotShow: only([w.ERRORED_NOT, i.sweepPartial ? w.SWEEP_PARTIAL_RATE_NOT : undefined]),
    };
  }
  if (!FINISHED_STATES.has(i.state)) return { shows: [w.UNKNOWN_STATE_SHOWS], doesNotShow: [w.UNKNOWN_STATE_NOT] };
  if ((i.sweep?.trials ?? i.trials) === 0 && i.failureCount === 0) return { shows: [w.ZERO_SHOWS], doesNotShow: [w.ZERO_NOT] };
  return undefined;
}

function rateLines(i: RunPanelInput): string[] {
  if ((i.failedPropertyCount ?? 0) > 1) return [w.severalRates(i.failedPropertyCount ?? 0)];
  return i.failedRates.length === 1 ? [w.rateForTrialsRun(rateText(i.failedRates[0]))] : [];
}

/** A sweep the server gave a summary for, that ran to its plan: the one case a rate may be stated. */
function sweepWithSummary(i: RunPanelInput, s: SweepSummary): RunPanelModel {
  const rated = s.passes + s.violations;
  const apart = w.notInAnyRate(s);
  if (rated === 0) return { shows: [w.sweepNothingRated(s.trials)], doesNotShow: only([w.SWEEP_NOTHING_RATED_NOT, apart]) };
  if (s.violations === 0) {
    // The counts and the run disagree: say the counts and make no claim.
    if (hasFailure(i)) return { shows: [w.trialsRan(s.trials), ...failureLines(i)], doesNotShow: only([w.SWEEP_DISAGREES_NOT, apart]) };
    return { shows: [w.trialsRan(s.trials), w.sweepNoFailure(rated)], doesNotShow: only([w.SWEEP_NO_FAILURE_NOT, apart]) };
  }
  const seed = i.firstFailingSeed ?? s.first_failing_seeds[0];
  return {
    shows: only([
      w.sweepCounts(s.trials, s.passes, s.violations),
      w.ratedFailed(s.violations, rated),
      seed ? w.firstFailingSeed(seed) : undefined,
      ...rateLines(i),
    ]),
    doesNotShow: only([w.SWEEP_OUTSIDE_NOT, apart]),
  };
}

/** Counts only: the run's completion is not known yet, or it stopped early, or the server gave no sweep summary. */
function countsOnly(i: RunPanelInput, stoppedEarly: boolean): RunPanelModel {
  const s = i.sweep;
  const failure = hasFailure(i);
  const noRate = i.sweepPartial ? w.SWEEP_PARTIAL_RATE_NOT : !s && (failure || stoppedEarly) && i.completion !== undefined ? w.SWEEP_NO_SUMMARY_NOT : undefined;
  return {
    shows: only([s ? w.sweepPartialCounts(s.trials, s.violations) : w.trialsRan(i.trials), ...(!s && failure ? failureLines(i) : [])]),
    doesNotShow: only([
      i.completion === undefined ? w.COMPLETION_NOT_LOADED_NOT : undefined,
      stoppedEarly ? w.stoppedEarlyNot(i.completionLabel) : undefined,
      noRate,
      !s && !failure && !stoppedEarly && i.completion !== undefined ? w.NO_FAILURE_NOT : undefined,
    ]),
  };
}

/**
 * The server's run detail reports mode "search" for sweeps as well, so the mode is never read. Only a sweep carries a
 * summary (and only once it is over), so a summary is the one sign of a sweep; without one the panel says nothing that
 * depends on the mode.
 */
function modeLines(i: RunPanelInput, stoppedEarly: boolean): RunPanelModel {
  if (i.sweep && i.completion !== undefined && !stoppedEarly) return sweepWithSummary(i, i.sweep);
  return countsOnly(i, stoppedEarly);
}

function noFailureConfirmed(i: RunPanelInput): boolean {
  return i.failureCount > 0 && i.findings.length > 0 && i.findings.every((f) => f !== undefined && KNOWN_FINDINGS.has(f) && !CONFIRMED.has(f));
}

/**
 * What a run shows and what it does not, in plain sentences. Every sentence comes from a server field or from wording
 * already in the repository (runPanelWording.ts). A run that is going, errored, cancelled, in a state this page does not
 * know, or that ran no trial makes no result claim; "none of them failed" is said only for a finished, complete run whose
 * completion is known and the server gave a sweep summary, and a rate only for a sweep the server gave a full summary for.
 */
export function runPanelModel(i: RunPanelInput): RunPanelModel {
  const early = earlyResult(i);
  if (early) return early;
  const base = modeLines(i, isStoppedEarly(i));
  const excluded = i.excluded ? excludedText(i.excluded) : undefined;
  return {
    shows: base.shows,
    doesNotShow: only([
      ...base.doesNotShow,
      excluded,
      i.unverifiedWorkerData ? w.UNVERIFIED_NOT : undefined,
      noFailureConfirmed(i) ? w.NOT_CONFIRMED_NOT : undefined,
    ]),
  };
}

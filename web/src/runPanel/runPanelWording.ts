/**
 * The sentences of the run page's "what this run shows / does not show" panel. Each one traces to a server field or to
 * wording that is already in the repository: the search and sweep meanings are docs/architecture.md ("Sweep mode") and
 * startRunWording.ts; "excluded from every rate" is wording.ts; the completion label is the server's own. A sentence
 * that needs a number takes it as an argument, so no number is written here.
 */
import { formatCount } from '../wording';

const trialsPhrase = (n: number): string => `${formatCount(n)} ${n === 1 ? 'trial' : 'trials'}`;

export const SHOWS_HEADING = 'What this run shows';
export const DOES_NOT_SHOW_HEADING = 'What it does not show';
export const PANEL_LABEL = 'What this run shows and does not show';
export const OUTCOME_LABEL = 'Outcome';

// ---- Runs that make no result claim ----
export const LIVE_SHOWS = 'The run has not finished, so nothing here is a result yet.';
export const LIVE_NOT = 'Whether the target fails: the run has not finished.';
export const ERRORED_SHOWS = 'The run ended in an error or was cancelled.';
export const ERRORED_NOT = 'Whether the target fails: the run did not finish.';
export const UNKNOWN_STATE_SHOWS = 'The server sent a run state this page does not know, so it makes no claim.';
export const UNKNOWN_STATE_NOT = 'Anything about the target: the run state is not one this page knows.';
export const ZERO_SHOWS = 'No trials were run.';
export const ZERO_NOT = 'Anything about the target: nothing was run.';

export const trialsRan = (n: number): string => `${trialsPhrase(n)} ran.`;
export const trialsRanBeforeStop = (n: number): string => `${trialsPhrase(n)} ran before it stopped. The figures are counts only.`;

// ---- Failures ----
export const failureSeed = (seed: string): string => `A failure: seed ${seed} failed.`;
export const failureRecords = (n: number): string =>
  n === 1 ? 'A failure record was stored for this run.' : `${formatCount(n)} failure records were stored for this run.`;
export const STOPPED_AT_FIRST_FAILURE_SHOWS = 'The run stopped at its first failure.';
export const MARKED_FAILED_SHOWS = 'The server marked the run failed.';
export const firstFailingSeed = (seed: string): string => `The first failing seed is ${seed}.`;
export const NOT_CONFIRMED_NOT = 'That a failure is confirmed: no failure of this run has a confirmed or verified status.';

// ---- A run with no sweep summary ----
// The server's run detail reports mode "search" for sweeps too, so these sentences never depend on the mode.
export const NO_FAILURE_NOT = 'That the target cannot fail: only those trials were run.';

// ---- Sweep ----
export const sweepCounts = (trials: number, passes: number, violations: number): string =>
  `${trialsPhrase(trials)} ran: ${formatCount(passes)} did not fail and ${formatCount(violations)} failed.`;
export const sweepPartialCounts = (trials: number, violations: number): string =>
  violations > 0 ? `${trialsPhrase(trials)} ran, ${formatCount(violations)} of them failed.` : trialsRan(trials);
export const ratedFailed = (violations: number, rated: number): string =>
  `${formatCount(violations)} of ${formatCount(rated)} rated ${rated === 1 ? 'trial' : 'trials'} failed.`;
export const rateForTrialsRun = (rateText: string): string => `Rate for the trials run: ${rateText}`;
export const severalRates = (n: number): string => `${formatCount(n)} properties failed; any rate is in the property table.`;
export const SWEEP_OUTSIDE_NOT = 'How often it fails outside these trials: the rate is for the trials run.';
export const sweepNoFailure = (rated: number): string => `Those ${formatCount(rated)} rated ${rated === 1 ? 'trial' : 'trials'} did not fail.`;
export const SWEEP_NO_FAILURE_NOT = 'That the target cannot fail. A sweep with no failure says only that those trials did not fail.';
export const sweepNothingRated = (trials: number): string => `${trialsPhrase(trials)} ran; none was rated.`;
export const SWEEP_NOTHING_RATED_NOT = 'Whether the target fails: no trial was rated.';
export const SWEEP_PARTIAL_RATE_NOT =
  'A rate: the sweep stopped when something happened, so the figures are counts only and stopping then can bias them.';
export const SWEEP_NO_SUMMARY_NOT = 'A sweep rate: the server gave no sweep summary for this run, so no rate is stated here.';

/** Trials that are in no rate: listed apart by the server, never described as passes. Empty when there are none. */
export function notInAnyRate(s: { inconclusive: number; unusable: number; infra: number }): string | undefined {
  const parts = [
    s.inconclusive > 0 ? `${formatCount(s.inconclusive)} timed out or were cut short` : undefined,
    s.unusable > 0 ? `${formatCount(s.unusable)} could not be used` : undefined,
    s.infra > 0 ? `${formatCount(s.infra)} failed in the host` : undefined,
  ].filter((p): p is string => p !== undefined);
  return parts.length === 0 ? undefined : `Not in any rate: ${parts.join(', ')}.`;
}

export const SWEEP_DISAGREES_NOT =
  'Whether the target fails: the sweep counts show no failing trial but the run has a failure, so no claim is made.';

// ---- Stopped early, partial, or not yet known ----
export const stoppedEarlyNot = (label: string | undefined): string =>
  label === undefined
    ? 'The run stopped early or its trial data is incomplete. The figures are counts of the trials that ran, not a rate.'
    : `How the run ended, as the server marks it: ${label}. The figures are counts of the trials that ran, not a rate.`;
export const COMPLETION_NOT_LOADED_NOT = 'How the run ended: its completion has not loaded.';
export const UNVERIFIED_NOT = 'That the trial data is trustworthy: it includes unverified worker data.';

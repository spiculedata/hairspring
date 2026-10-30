import { stateTone, type Tone } from './format';
import type { RunCompletion } from './types';
import { formatCount } from './wording';

export interface RunOutcomeInput {
  /** The run's state as the server sends it. */
  readonly state: string;
  /** Trials the run has stored (`Run.trials`). */
  readonly trials: number;
  /** The run header's completion, when the property page has loaded; the run list does not have it. */
  readonly completion?: RunCompletion;
}

export interface RunOutcome {
  readonly label: string;
  /** What the label covers, such as "2 of 30 planned trials". Absent for states that make no claim about trials. */
  readonly scope?: string;
  readonly tone: Tone;
}

export const NO_FAILURE_SEEN = 'No failure seen';
export const STOPPED_EARLY = 'Stopped early';

const trialsPhrase = (n: number): string => `${formatCount(n)} ${n === 1 ? 'trial' : 'trials'}`;

function stoppedEarly(completion: RunCompletion | undefined): boolean {
  return completion !== undefined && ['time_or_step_limited', 'ended_early', 'digest_allowance_used_up'].includes(completion.kind);
}

/**
 * The outcome shown for a run. A run the server calls `passed` is never shown as "passed": that reads as a clean bill
 * of health. It is "No failure seen", with the number of trials that statement covers, or "Stopped early" when the run
 * did not finish its planned trials. Every other state is shown as sent. Only server fields are used.
 */
export function runOutcome({ state, trials, completion }: RunOutcomeInput): RunOutcome {
  if (state !== 'passed') return { label: state, tone: stateTone(state) };
  if (completion?.kind === 'time_or_step_limited') {
    return { label: STOPPED_EARLY, scope: `${formatCount(completion.trials_run)} of ${formatCount(completion.trials_budget)} planned trials`, tone: 'idle' };
  }
  if (stoppedEarly(completion)) return { label: STOPPED_EARLY, scope: trialsPhrase(trials), tone: 'idle' };
  const scope = completion?.kind === 'complete' ? `${formatCount(trials)} of ${formatCount(trials)} planned trials` : trialsPhrase(trials);
  return { label: NO_FAILURE_SEEN, scope, tone: 'good' };
}

import { stateTone, type Tone } from './format';
import type { RunCompletion } from './types';
import { formatCount } from './wording';

export interface RunOutcomeInput {
  /** The run's state as the server sends it. */
  readonly state: string;
  /** Trials the run has stored (`Run.trials`). */
  readonly trials: number;
  /** The run header's completion. Absent until the property page has loaded, and in the run list, which has none. */
  readonly completion?: RunCompletion;
}

export interface RunOutcome {
  readonly label: string;
  /** What the label covers, such as "2 of 30 planned trials". Absent for states that make no claim about trials. */
  readonly scope?: string;
  readonly tone: Tone;
  /** The server's own word, for a tooltip, so the plainer label never hides it. */
  readonly title: string;
}

export const NO_FAILURE_SEEN = 'No failure seen';
export const STOPPED_EARLY = 'Stopped early';
export const NO_TRIALS_RUN = 'No trials run';
export const FINISHED = 'Finished';
export const STOPPED_AT_FIRST_FAILURE = 'Stopped at first failure';

const trialsPhrase = (n: number): string => `${formatCount(n)} ${n === 1 ? 'trial' : 'trials'}`;

/**
 * The outcome shown for a run. A run the server calls `passed` is never shown as "passed": that reads as a clean bill
 * of health. "No failure seen" needs two things: the run's completion is known and it finished, and at least one trial
 * ran. Until the completion is known the label is the neutral "Finished". The word "planned" is used only when the
 * server sent a planned count (a run cut short by a time or step limit). Every other state is shown as sent.
 */
export function runOutcome({ state, trials, completion }: RunOutcomeInput): RunOutcome {
  const title = `Server state: ${state}`;
  if (state !== 'passed') return { label: state, tone: stateTone(state), title };
  const stopped = (scope: string): RunOutcome => ({ label: STOPPED_EARLY, scope, tone: 'idle', title });
  switch (completion?.kind) {
    case 'running':
      return { label: 'running', tone: 'busy', title };
    case 'stopped_at_first_failure':
      return { label: STOPPED_AT_FIRST_FAILURE, tone: 'bad', title };
    case 'aborted':
    case 'ended_early':
    case 'digest_allowance_used_up':
      return stopped(trialsPhrase(trials));
    case 'time_or_step_limited': {
      const { trials_run: run, trials_budget: budget } = completion;
      // A limit that was reached only after every planned trial ran did not cut the run short.
      if (budget > 0 && run < budget) return stopped(`${formatCount(run)} of ${formatCount(budget)} planned trials`);
      break;
    }
    case undefined:
      return trials === 0
        ? { label: NO_TRIALS_RUN, tone: 'idle', title }
        : { label: FINISHED, scope: trialsPhrase(trials), tone: 'idle', title };
    default:
      break;
  }
  if (trials === 0) return { label: NO_TRIALS_RUN, tone: 'idle', title };
  return { label: NO_FAILURE_SEEN, scope: trialsPhrase(trials), tone: 'good', title };
}

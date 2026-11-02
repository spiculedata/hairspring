import type { PropertyStatus } from './types';
import { REPLAY_DID_NOT_FAIL, REPLAY_FAILED_AGAIN, UNKNOWN_LABEL, statusLabel } from './wording';

/**
 * The one table from a server status word to the words and colour a person sees. The label is always text; the tone
 * only adds emphasis, and the stylesheet gives each tone its own mark as well. A word the table does not have is a
 * neutral "Unknown", never a tone that reads as a pass.
 *
 * "passed" is not in the run table on purpose: a run's pass word is shown by `runOutcome`, which needs the trial count
 * and the completion before it may say "No failure seen".
 */
export type Tone = 'good' | 'bad' | 'busy' | 'idle' | 'gap';

export interface StatusEntry {
  readonly label: string;
  readonly tone: Tone;
}

export { UNKNOWN_LABEL };

const UNKNOWN: StatusEntry = Object.freeze({ label: UNKNOWN_LABEL, tone: 'idle' });

/** Looks `key` up as an own property only, so words such as "constructor" or "__proto__" are unknown, not inherited. */
function lookup(table: Readonly<Record<string, StatusEntry>>, key: string): StatusEntry {
  return Object.hasOwn(table, key) ? table[key] : UNKNOWN;
}

export const RUN_STATES: Readonly<Record<string, StatusEntry>> = Object.freeze({
  queued: { label: 'Queued', tone: 'idle' },
  running: { label: 'Running', tone: 'busy' },
  failed: { label: 'Failed', tone: 'bad' },
  error: { label: 'Error', tone: 'bad' },
  cancelled: { label: 'Cancelled', tone: 'idle' },
});

export const PROPERTY_STATUSES: Readonly<Record<PropertyStatus, StatusEntry>> = Object.freeze({
  violation: { label: statusLabel('violation'), tone: 'bad' },
  unmet: { label: statusLabel('unmet'), tone: 'gap' },
  unrated: { label: statusLabel('unrated'), tone: 'idle' },
  pass: { label: statusLabel('pass'), tone: 'good' },
});

/** One replay is one more run of the same steps, so a replay that does not fail is neutral, not a pass. */
export const REPLAY_STATUSES = Object.freeze({
  failedAgain: { label: REPLAY_FAILED_AGAIN, tone: 'bad' },
  didNotFail: { label: REPLAY_DID_NOT_FAIL, tone: 'idle' },
} satisfies Record<string, StatusEntry>);

export const runStateEntry = (state: string): StatusEntry => lookup(RUN_STATES, state);
export const propertyStatusEntry = (status: string): StatusEntry => lookup(PROPERTY_STATUSES, status);
export const replayEntry = (failedAgain: boolean): StatusEntry => (failedAgain ? REPLAY_STATUSES.failedAgain : REPLAY_STATUSES.didNotFail);

/**
 * Plain-text summaries for pasting into an issue or a message. They are built from the server's own figures and names,
 * and from the one piece of program text a reader needs to find the failure, the property id, which is escaped and cut.
 * Nothing the program printed, no failure message, no step, no witness value and no bundle goes in: a summary is
 * shared further than the page it came from.
 */
import { FINDING_LABELS } from '../confirmationWording';
import { escapeGuest } from '../guest/escape';
import type { TrialRow } from '../trial/types';
import type { FailureSummary } from '../types';
import { trialTitle } from '../wording';

/** Characters of any one name that a summary carries. */
const NAME_CHARS = 160;

/** `text` with control, bidi and format characters written out, cut to `max` characters. */
function safe(text: string, max: number = NAME_CHARS): string {
  const { segments, truncated } = escapeGuest(text, max);
  return `${segments.map((s) => s.text).join('')}${truncated ? '…' : ''}`;
}

export interface TrialSummaryInput {
  readonly runId: string;
  readonly trial: TrialRow;
  readonly link: string;
  /** What the run says about how it ended, when it could be loaded. */
  readonly completionLabel: string | undefined;
  readonly stoppedAtFirstFailure: boolean;
}

function failingLine(trial: TrialRow): string {
  if (trial.digest_unreadable) return 'Failing properties: not known (the stored digest could not be read)';
  return `Failing properties: ${trial.failing_count}`;
}

/** What to paste about one trial: which seed failed, where to look, and what the run did and did not establish. */
export function trialSummary({ runId, trial, link, completionLabel, stoppedAtFirstFailure }: TrialSummaryInput): string {
  const firstId = trial.guest.failing_ids[0];
  return [
    `${trialTitle(trial.verdict, safe(trial.seed))}`,
    `Run: ${safe(runId)}`,
    `Trial: ${trial.seq}`,
    failingLine(trial),
    firstId === undefined ? undefined : `First failing property: ${safe(firstId)}`,
    completionLabel === undefined ? undefined : `Run state: ${safe(completionLabel)}`,
    stoppedAtFirstFailure ? 'This run stopped at its first failure; one failing seed is not a rate.' : undefined,
    `Link: ${link}`,
  ]
    .filter((line): line is string => line !== undefined)
    .join('\n');
}

export interface FailureSummaryInput {
  readonly record: FailureSummary;
  readonly link: string;
}

/** What to paste about one bundle failure: its size, its bundle hash and where it stands, as the server states it. */
export function failureSummary({ record, link }: FailureSummaryInput): string {
  const standing = record.finding === undefined ? undefined : (FINDING_LABELS[record.finding.status] ?? undefined);
  return [
    'Hairspring failure',
    `Target: ${safe(record.target)}`,
    `Run: ${safe(record.run_id)}`,
    `Failure: ${safe(record.id)}`,
    `Shortest failing steps: ${record.shrunk_len} (from ${record.original_len})`,
    `Bundle hash: ${safe(record.bundle_hash)}`,
    standing === undefined ? undefined : `Confirmation: ${standing}`,
    `Link: ${link}`,
  ]
    .filter((line): line is string => line !== undefined)
    .join('\n');
}

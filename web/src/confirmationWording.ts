import { formatCount } from './wording';

/**
 * Words for a failure's finding and its stored checks. The state and check names are the server's own
 * (`hairspring-engine` `confirm::state`, served by `GET /api/failures/{id}/checks`); the meanings are the table under
 * "The finding" in docs/determinism.md and the text `hairspring confirm` prints. Nothing here is a rate, and nothing
 * says a failure is fixed.
 */

/** The rungs of the ladder, lowest first. `not_reproducible` and `control_failed` are ends, not rungs. */
export const LADDER_RUNGS = ['candidate', 'rerun_repeated', 'confirmed', 'verified'] as const;
export const LADDER_ENDS = ['not_reproducible', 'control_failed'] as const;

export const FINDING_LABELS: Readonly<Record<string, string>> = Object.freeze({
  candidate: 'Candidate',
  rerun_repeated: 'Rerun repeated',
  confirmed: 'Confirmed',
  verified: 'Verified',
  not_reproducible: 'Not reproducible',
  control_failed: 'Control failed',
});

export const FINDING_MEANINGS: Readonly<Record<string, string>> = Object.freeze({
  candidate: 'No check has repeated the failure.',
  rerun_repeated: 'The reruns repeat the failure; no replay of the bundle has yet.',
  confirmed: 'The reruns repeat the failure, and so does the replay of its bundle.',
  verified: 'Confirmed, and the strict verify agrees.',
  not_reproducible: 'A rerun or the replay did not repeat the failure on the environment it was recorded in.',
  control_failed: 'The reruns repeat the failure, but the fixed build still fails at this seed.',
});

export const LADDER_HEADING = 'Confirmation';
export const LADDER_NOTE =
  'The server works out where a failure stands from the checks stored for it, using one confirmation. A state says how far the checks have repeated the failure. None of them says the failure is fixed.';
export const CURRENT_MARK = 'Current';
export const REACHED_MARK = 'Reached';
export const NOT_REACHED_MARK = 'Not reached';
export const END_NOTE = 'This state is an end of the ladder, not a rung.';
export const ENVIRONMENT_PINNED = 'Environment pinned';
export const ENVIRONMENT_NOT_PINNED = 'Environment not pinned';
export const UNKNOWN_FINDING_TEXT = 'The server reported a state this page has no wording for. It is shown as sent:';

// ---- Checks ----

export const CHECK_KIND_LABELS: Readonly<Record<string, string>> = Object.freeze({
  same_seed_rerun: 'Same-seed rerun',
  bundle_replay: 'Bundle replay',
  fixed_control: 'Fixed control',
  strict_verify: 'Strict verify',
});

export const CHECK_STATUS_LABELS: Readonly<Record<string, string>> = Object.freeze({
  pass: 'Pass',
  fail: 'Fail',
  inconclusive: 'Inconclusive',
  skipped: 'Skipped',
});

export const CHECKS_HEADING = 'Checks';
export const CHECKS_NOTE =
  'A check passes when it showed what it set out to: for the reruns and the replay, that the failure repeats; for the fixed control, that the fixed build is clean at the seed. Inconclusive and skipped checks show nothing either way. A claim uses only checks from trusted runners.';
export const OTHER_CONFIRMATION_NOTE =
  'The finding is worked out from one confirmation. Checks from other confirmations are listed but do not count towards it.';
export const ENVIRONMENT_DIFFERS_NOTE = 'inconclusive: environment differs from the recording; this does not show the bug is fixed';
export const ENV_DIFFERS = 'Environment differs from the recording';
export const ENV_MATCHES = 'Environment matches the recording';
export const ENV_NOT_REPORTED = 'Not reported';

export const NO_CHECKS_TEXT = 'No checks recorded. This failure has not been confirmed.';
export const NO_CHECKS_HINT = 'To record checks, run:';
export const noChecksCommand = (failureId: string): string => `hairspring confirm ${failureId}`;

export function countsText(c: { passed: number; failed: number; inconclusive: number }): string {
  return `${formatCount(c.passed)} passed, ${formatCount(c.failed)} failed, ${formatCount(c.inconclusive)} inconclusive`;
}

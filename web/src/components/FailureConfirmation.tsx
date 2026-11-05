import { api } from '../api';
import { NO_CHECKS_HINT, NO_CHECKS_TEXT, noChecksCommand } from '../confirmationWording';
import { usePolling } from '../usePolling';
import { CheckTable } from './CheckTable';
import { ConfirmationLadder } from './ConfirmationLadder';
import { ConfirmRun } from './ConfirmRun';
import { Loading, LoadFailed } from './Notices';
import { UpdatedAt } from './UpdatedAt';

const POLL_MS = 30_000;

/**
 * The confirmation section of a failure page: where the failure stands and the checks that say so, from
 * `GET /api/failures/{id}/checks` only. It reads and shows; it starts nothing itself (`ConfirmRun` does, for a caller the
 * server allows). A failed load names what was loading and
 * why, and is never shown as "no checks".
 */
export function FailureConfirmation({ failureId }: { failureId: string }) {
  const checks = usePolling(() => api.failureChecks(failureId), POLL_MS);
  const data = checks.data;
  if (data === undefined) {
    return checks.error === undefined ? (
      <Loading what="confirmation checks" />
    ) : (
      <LoadFailed what="the confirmation checks" message={checks.error} network={checks.networkError} retries={!checks.notFound} />
    );
  }
  return (
    <>
      {checks.error !== undefined && <LoadFailed what="the confirmation checks" message={checks.error} network={checks.networkError} retries={!checks.notFound} />}
      <ConfirmationLadder finding={data.finding} environmentPinned={data.environment_pinned} />
      <ConfirmRun failureId={failureId} onFinished={checks.reload} />
      {data.checks.length === 0 ? (
        <>
          <p>{NO_CHECKS_TEXT}</p>
          <p className="muted">{NO_CHECKS_HINT}</p>
          <pre className="state">{noChecksCommand(failureId)}</pre>
        </>
      ) : (
        <CheckTable checks={data.checks} confirmationId={data.confirmation_id} />
      )}
      <UpdatedAt at={checks.updatedAt} />
    </>
  );
}

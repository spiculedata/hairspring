import { api } from '../api';
import { FailuresTable } from '../components/FailuresTable';
import { Loading, LoadFailed } from '../components/Notices';
import { UpdatedAt } from '../components/UpdatedAt';
import { usePolling } from '../usePolling';
import { NO_FAILURES_YET, failuresListCutOff } from '../wording';

const LIMIT = 50;
const POLL_MS = 5000;

/** Failures across runs, newest first. It shows only what `GET /api/failures` sends. */
export function FailuresPage() {
  const failures = usePolling(() => api.failures(LIMIT), POLL_MS);
  const list = failures.data;
  return (
    <>
      <h1>Failures</h1>
      {failures.error && <LoadFailed what="failures" message={failures.error} />}
      <UpdatedAt at={failures.updatedAt} />
      {list === undefined && failures.error === undefined && <Loading what="failures" />}
      {list?.length === 0 && <p className="muted">{NO_FAILURES_YET}</p>}
      {list !== undefined && list.length > 0 && <FailuresTable failures={list} />}
      {list !== undefined && list.length >= LIMIT && <p className="muted">{failuresListCutOff(list.length)}</p>}
    </>
  );
}

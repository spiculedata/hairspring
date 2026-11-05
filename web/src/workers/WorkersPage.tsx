import { api } from '../api';
import { Link } from '../components/Layout';
import { Loading, LoadFailed } from '../components/Notices';
import { ScrollRegion } from '../components/ScrollRegion';
import { StatusBadge } from '../components/StatusBadge';
import { TimeStamp } from '../components/TimeStamp';
import { UpdatedAt } from '../components/UpdatedAt';
import { useMe } from '../meStore';
import type { Worker } from '../types';
import { usePolling } from '../usePolling';
import { capabilityLines, hiddenIdNote, idColumnShown, jobsSummary } from './workersModel';
import {
  attemptsText,
  CAPS_NOTE,
  heldMore,
  HEADER_CAPS,
  HEADER_DOING,
  HEADER_ID,
  HEADER_LAST_SEEN,
  HEADER_NAME,
  HEADER_STATE,
  HEADER_TRUST,
  HOLDING_NOTHING,
  ID_HIDDEN_NOTE,
  jobKindSuffix,
  NO_WORKERS,
  NO_WORKERS_HINT,
  queueLine,
  SCROLL_HINT_WORKERS,
  STATE_NOT_REPORTED,
  STATE_NOTE,
  TRUST_NOT_REPORTED,
  TRUST_NOTE,
  WORKERS_HEADING,
} from './workersWording';

const POLL_MS = 3000;
const TABLE_MAX_HEIGHT = 640;

function Holding({ worker }: { worker: Worker }) {
  const { jobs, more } = jobsSummary(worker);
  if (jobs.length === 0 && more === 0) return <span className="muted">{HOLDING_NOTHING}</span>;
  return (
    <ul className="worker-list">
      {jobs.map((job) => (
        <li key={`${job.run_id}-${job.lease_until}`}>
          <Link to={`/runs/${job.run_id}`}>{job.run_id}</Link>
          {`${jobKindSuffix(job.kind)}${attemptsText(job.attempts)}`}
        </li>
      ))}
      {more > 0 && <li className="muted">{heldMore(more)}</li>}
    </ul>
  );
}

function WorkerRow({ worker, showId }: { worker: Worker; showId: boolean }) {
  return (
    <tr>
      <td>{worker.name}</td>
      <td>
        {worker.state === undefined ? <span className="muted">{STATE_NOT_REPORTED}</span> : <StatusBadge kind="workerState" state={worker.state} />}
      </td>
      <td>
        {worker.trusted === undefined ? <span className="muted">{TRUST_NOT_REPORTED}</span> : <StatusBadge kind="workerTrust" trusted={worker.trusted} />}
      </td>
      <td>
        <ul className="worker-list">
          {capabilityLines(worker).map((line, i) => (
            <li key={`${i}-${line}`}>{line}</li>
          ))}
        </ul>
      </td>
      <td>
        <Holding worker={worker} />
      </td>
      <td>
        <TimeStamp ms={worker.last_seen} />
      </td>
      {showId && <td>{worker.id === undefined ? null : <code>{worker.id}</code>}</td>}
    </tr>
  );
}

/**
 * The workers the server lists: what each is doing, whether it is trusted, what it said it can do and what it holds,
 * with the queue behind them. It only reads. The id column follows what the server sent: the id is a credential, so a
 * read caller is told why it is missing. A server that does not report state leaves those cells saying so.
 */
export function WorkersPage() {
  const w = usePolling(api.workersOverview, POLL_MS);
  const me = useMe();
  const workers = w.data?.workers;
  const showId = workers !== undefined && idColumnShown(workers);
  return (
    <>
      <h1>{WORKERS_HEADING}</h1>
      {w.error && <LoadFailed what="workers" message={w.error} network={w.networkError} />}
      <UpdatedAt at={w.updatedAt} />
      {workers === undefined && w.error === undefined && <Loading what="workers" />}
      {w.data?.queue && <p>{queueLine(w.data.queue.queued, w.data.queue.running)}</p>}
      {workers?.length === 0 && (
        <>
          <p className="muted">{NO_WORKERS}</p>
          <p className="muted">{NO_WORKERS_HINT}</p>
        </>
      )}
      {workers !== undefined && workers.length > 0 && (
        <ScrollRegion label="Workers" hint={SCROLL_HINT_WORKERS} maxHeight={TABLE_MAX_HEIGHT}>
          <table className="workers">
            <thead>
              <tr>
                <th>{HEADER_NAME}</th>
                <th>{HEADER_STATE}</th>
                <th>{HEADER_TRUST}</th>
                <th>{HEADER_CAPS}</th>
                <th>{HEADER_DOING}</th>
                <th>{HEADER_LAST_SEEN}</th>
                {showId && <th>{HEADER_ID}</th>}
              </tr>
            </thead>
            <tbody>
              {workers.map((worker, i) => (
                <WorkerRow key={worker.id ?? `${worker.name}-${i}`} worker={worker} showId={showId} />
              ))}
            </tbody>
          </table>
        </ScrollRegion>
      )}
      {hiddenIdNote(me) && <p className="muted">{ID_HIDDEN_NOTE}</p>}
      <p className="muted">{STATE_NOTE}</p>
      <p className="muted">{TRUST_NOTE}</p>
      <p className="muted">{CAPS_NOTE}</p>
    </>
  );
}

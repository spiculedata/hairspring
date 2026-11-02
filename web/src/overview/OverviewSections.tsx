import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { StatusBadge } from '../components/StatusBadge';
import { TimeStamp } from '../components/TimeStamp';
import { failurePath, runPath } from '../router';
import type { FailureSummary, Run, Worker } from '../types';
import './overview.css';
import { newestFailures } from './overviewModel';
import {
  ALL_FAILURES_LINK,
  ALL_RUNS_LINK,
  ALL_WORKERS_LINK,
  ATTENTION_FAILURES_HEADING,
  ATTENTION_RUNS_HEADING,
  NO_RECENT_RUNS,
  NO_WORKERS,
  RECENT_RUNS_HEADING,
  WORKERS_NOTE,
  WORKER_LAST_SEEN,
  workersRegistered,
} from './overviewWording';

const ERROR_CHARS = 140;
const MESSAGE_CHARS = 140;

function RunBadge({ run }: { run: Run }) {
  return <StatusBadge kind="run" state={run.state} trials={run.trials} />;
}

/** Failed or errored runs: each links to its page and shows the state as the server sent it, and the run's error text if any. */
export function AttentionRuns({ runs }: { runs: readonly Run[] }) {
  return (
    <>
      <h3>{ATTENTION_RUNS_HEADING}</h3>
      <ul className="overview-list">
        {runs.map((r) => (
          <li key={r.id}>
            <Link to={runPath(r.id)}>{r.id}</Link> <RunBadge run={r} /> <span className="muted">{r.target}</span>
            <TimeStamp ms={r.created_at} />
            {r.error !== null && r.error !== '' && (
              <div className="overview-detail">
                <GuestText text={r.error} max={ERROR_CHARS} expandable={false} />
              </div>
            )}
          </li>
        ))}
      </ul>
    </>
  );
}

/** The newest failures from `GET /api/failures`. That list has no confirmation status, so none is shown. */
export function AttentionFailures({ failures, limit }: { failures: readonly FailureSummary[]; limit: number }) {
  return (
    <>
      <h3>{ATTENTION_FAILURES_HEADING}</h3>
      <ul className="overview-list">
        {newestFailures(failures, limit).map((f) => {
            const to = failurePath(f.id);
            return (
              <li key={f.id}>
                {to === undefined ? f.id : <Link to={to}>{f.id}</Link>} <span className="muted">{f.target}</span>
                <TimeStamp ms={f.created_at} />
                <div className="overview-detail">
                  <GuestText text={f.message} max={MESSAGE_CHARS} expandable={false} />
                </div>
              </li>
            );
          })}
      </ul>
      <p>
        <Link to="/failures">{ALL_FAILURES_LINK}</Link>
      </p>
    </>
  );
}

export function RecentRuns({ runs }: { runs: readonly Run[] }) {
  return (
    <>
      <h2 id="overview-recent">{RECENT_RUNS_HEADING}</h2>
      {runs.length === 0 ? (
        <p className="muted">{NO_RECENT_RUNS}</p>
      ) : (
        <table className="runs">
          <thead>
            <tr>
              <th>Run</th>
              <th>Target</th>
              <th>State</th>
              <th>Started</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((r) => (
              <tr key={r.id}>
                <td>
                  <Link to={runPath(r.id)}>{r.id}</Link>
                </td>
                <td>{r.target}</td>
                <td>
                  <RunBadge run={r} />
                </td>
                <td>
                  <TimeStamp ms={r.created_at} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p>
        <Link to="/runs">{ALL_RUNS_LINK}</Link>
      </p>
    </>
  );
}

/** A count of the workers the server lists and when one was last heard from. It says nothing about what they are doing. */
export function WorkersSummary({ workers }: { workers: readonly Worker[] }) {
  const latest = workers.reduce<number | undefined>((best, w) => (best === undefined || w.last_seen > best ? w.last_seen : best), undefined);
  return (
    <>
      {workers.length === 0 ? (
        <p className="muted">{NO_WORKERS}</p>
      ) : (
        <p>
          {workersRegistered(workers.length)}
          {latest !== undefined && (
            <>
              . {WORKER_LAST_SEEN}: <TimeStamp ms={latest} />
            </>
          )}
        </p>
      )}
      <p className="muted">{WORKERS_NOTE}</p>
      <p>
        <Link to="/workers">{ALL_WORKERS_LINK}</Link>
      </p>
    </>
  );
}

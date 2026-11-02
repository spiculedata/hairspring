import { failurePath, runPath } from '../router';
import type { FailureSummary } from '../types';
import { formatCount } from '../wording';
import { GuestText } from './GuestText';
import { Link } from './Layout';
import { TimeStamp } from './TimeStamp';

const MESSAGE_CHARS = 140;
const RUN_ID_PLAIN = /^[A-Za-z0-9_-]{1,64}$/;

/** The failures newest first, as a new array; failures made at the same time keep the order the server sent them in. */
export function newestFirst(failures: readonly FailureSummary[]): FailureSummary[] {
  return failures
    .map((f, i) => ({ f, i }))
    .sort((a, b) => b.f.created_at - a.f.created_at || a.i - b.i)
    .map(({ f }) => f);
}

/**
 * Failures across runs, from `GET /api/failures`. That list carries no confirmation status and no occurrence count, so
 * none is shown: each row says what the server says (which run, which target, the message, the step counts and when it
 * was stored) and links to the run and to the failure's own page. The message is from the program under test.
 */
export function FailuresTable({ failures }: { failures: readonly FailureSummary[] }) {
  return (
    <table className="runs failures">
      <thead>
        <tr>
          <th>Failure</th>
          <th>Target</th>
          <th>Message</th>
          <th>Run</th>
          <th className="num opt">Original steps</th>
          <th className="num opt">Shortest steps</th>
          <th>Recorded</th>
        </tr>
      </thead>
      <tbody>
        {newestFirst(failures).map((f) => {
          const to = failurePath(f.id);
          return (
            <tr key={f.id}>
              <td>{to === undefined ? f.id : <Link to={to}>{f.id}</Link>}</td>
              <td>{f.target}</td>
              <td>
                <GuestText text={f.message} max={MESSAGE_CHARS} expandable={false} />
              </td>
              <td>{RUN_ID_PLAIN.test(f.run_id) ? <Link to={runPath(f.run_id)}>{f.run_id}</Link> : f.run_id}</td>
              <td className="num opt">{formatCount(f.original_len)}</td>
              <td className="num opt">{formatCount(f.shrunk_len)}</td>
              <td>
                <TimeStamp ms={f.created_at} />
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

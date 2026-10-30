import type { Run } from '../types';
import { relativeTime } from '../format';
import { StateBadge } from './Badge';
import { Link } from './Layout';

/** The runs list. The Steps column is optional: the stylesheet drops it on a phone. */
export function RunsTable({ runs }: { runs: readonly Run[] }) {
  return (
    <table className="runs">
      <thead>
        <tr>
          <th>Run</th>
          <th>Target</th>
          <th>State</th>
          <th className="num opt">Steps</th>
          <th>Started</th>
        </tr>
      </thead>
      <tbody>
        {runs.map((r) => (
          <tr key={r.id}>
            <td>
              <Link to={`/runs/${r.id}`}>{r.id}</Link>
            </td>
            <td>{r.target}</td>
            <td>
              <StateBadge state={r.state} trials={r.trials} />
            </td>
            <td className="num opt">{r.steps.toLocaleString()}</td>
            <td>{relativeTime(r.created_at)}</td>
          </tr>
        ))}
        {runs.length === 0 && (
          <tr>
            <td colSpan={5} className="muted">
              No runs yet. Start one above, or run <code>hairspring test</code>.
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

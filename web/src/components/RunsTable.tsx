import { RUNS_FILTERED_EMPTY } from '../lists/listWording';
import type { Run } from '../types';
import { StatusBadge } from './StatusBadge';
import { Link } from './Layout';
import { TimeStamp } from './TimeStamp';

/**
 * The runs list. The Steps column is optional: the stylesheet drops it on a phone. `filtered` says the list is narrowed
 * by filters, so an empty list means nothing matched and not that no run was ever started.
 */
export function RunsTable({ runs, filtered = false }: { runs: readonly Run[]; filtered?: boolean }) {
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
              <span className="muted steps-note">{`${r.steps.toLocaleString()} steps`}</span>
            </td>
            <td>{r.target}</td>
            <td>
              <StatusBadge kind="run" state={r.state} trials={r.trials} />
            </td>
            <td className="num opt">{r.steps.toLocaleString()}</td>
            <td>
              <TimeStamp ms={r.created_at} />
            </td>
          </tr>
        ))}
        {runs.length === 0 && (
          <tr>
            <td colSpan={5} className="muted">
              {filtered ? (
                RUNS_FILTERED_EMPTY
              ) : (
                <>
                  No runs yet. Start one above, or run <code>hairspring test</code>.
                </>
              )}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

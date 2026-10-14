import { api } from '../api';
import { relativeTime } from '../format';
import { usePolling } from '../usePolling';

export function Targets() {
  const t = usePolling(api.targets, 60_000);
  return (
    <>
      <h1>Targets</h1>
      {t.error && <p className="error" role="alert">{t.error}</p>}
      <table>
        <thead>
          <tr><th>Name</th><th>Kind</th><th>Budget</th><th>Description</th></tr>
        </thead>
        <tbody>
          {(t.data ?? []).map((x) => (
            <tr key={x.name}>
              <td>{x.name}</td>
              <td>{x.kind}</td>
              <td className="muted">{x.budget.max_steps.toLocaleString()} steps / {x.budget.wall_secs}s</td>
              <td>{x.description}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

export function Workers() {
  const w = usePolling(api.workers, 3000);
  return (
    <>
      <h1>Workers</h1>
      {w.error && <p className="error" role="alert">{w.error}</p>}
      <table>
        <thead>
          <tr><th>Name</th><th>Id</th><th>Last seen</th></tr>
        </thead>
        <tbody>
          {(w.data ?? []).map((x, i) => (
            <tr key={x.id ?? `${x.name}-${i}`}>
              <td>{x.name}</td>
              <td>{x.id ? <code>{x.id}</code> : <span className="muted">hidden</span>}</td>
              <td>{relativeTime(x.last_seen)}</td>
            </tr>
          ))}
          {w.data?.length === 0 && <tr><td colSpan={3} className="muted">No workers registered.</td></tr>}
        </tbody>
      </table>
    </>
  );
}

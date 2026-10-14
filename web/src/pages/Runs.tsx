import { useState } from 'react';
import { api } from '../api';
import { Link } from '../components/Layout';
import { StateBadge } from '../components/Badge';
import { formatSeedInput, relativeTime } from '../format';
import { navigate } from '../router';
import { usePolling } from '../usePolling';

function NewRun() {
  const targets = usePolling(api.targets, 60_000);
  const [target, setTarget] = useState('');
  const [seed, setSeed] = useState('');
  const [message, setMessage] = useState<string>();
  const chosen = target || targets.data?.[0]?.name || '';

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const parsed = formatSeedInput(seed);
    if (!parsed.ok) return setMessage(parsed.message);
    try {
      const { run } = await api.startRun(chosen, parsed.value);
      navigate(`/runs/${run.id}`);
    } catch (err) {
      setMessage(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <form className="card inline" onSubmit={submit}>
      <label>
        Target
        <select value={chosen} onChange={(e) => setTarget(e.target.value)}>
          {(targets.data ?? []).map((t) => (
            <option key={t.name} value={t.name}>
              {t.name} ({t.kind})
            </option>
          ))}
        </select>
      </label>
      <label>
        Seed (optional)
        <input inputMode="numeric" placeholder="random" value={seed} onChange={(e) => setSeed(e.target.value)} />
      </label>
      <button type="submit" disabled={!chosen}>
        Start run
      </button>
      {message && <span className="error" role="alert">{message}</span>}
    </form>
  );
}

export function Runs() {
  const runs = usePolling(() => api.runs(50), 2000);
  return (
    <>
      <h1>Runs</h1>
      <NewRun />
      {runs.error && <p className="error" role="alert">{runs.error}</p>}
      <table>
        <thead>
          <tr>
            <th>Run</th>
            <th>Target</th>
            <th>State</th>
            <th className="num">Steps</th>
            <th>Started</th>
          </tr>
        </thead>
        <tbody>
          {(runs.data ?? []).map((r) => (
            <tr key={r.id}>
              <td>
                <Link to={`/runs/${r.id}`}>{r.id}</Link>
              </td>
              <td>{r.target}</td>
              <td>
                <StateBadge state={r.state} />
              </td>
              <td className="num">{r.steps.toLocaleString()}</td>
              <td>{relativeTime(r.created_at)}</td>
            </tr>
          ))}
          {runs.data?.length === 0 && (
            <tr>
              <td colSpan={5} className="muted">
                No runs yet. Start one above, or run <code>anthill test</code>.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </>
  );
}

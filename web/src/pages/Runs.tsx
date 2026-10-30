import { useState } from 'react';
import { api } from '../api';
import { RunsTable } from '../components/RunsTable';
import { formatSeedInput } from '../format';
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
      <RunsTable runs={runs.data ?? []} />
    </>
  );
}

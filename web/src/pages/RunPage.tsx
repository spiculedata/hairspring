import { useEffect, useState } from 'react';
import { api, streamRunEvents } from '../api';
import { Link } from '../components/Layout';
import { StateBadge } from '../components/Badge';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { FirstFailingSeed } from '../explorer/FirstFailingSeed';
import { PropertyOverviewPanel } from '../explorer/PropertyOverviewPanel';
import { usePropertyOverview } from '../explorer/usePropertyOverview';
import { formatDuration, relativeTime } from '../format';
import { seedsPath } from '../seeds/filters';
import { BROWSE_SEEDS, noFailuresText } from '../wording';
import type { SseEvent } from '../sse';
import { usePolling } from '../usePolling';

const MAX_LOG_LINES = 200;
const TERMINAL = new Set(['passed', 'failed', 'error', 'cancelled']);

function Activity({ runId, active }: { runId: string; active: boolean }) {
  const [lines, setLines] = useState<SseEvent[]>([]);
  useEffect(() => {
    const ctl = new AbortController();
    streamRunEvents(runId, (e) => setLines((prev) => [...prev, e].slice(-MAX_LOG_LINES)), ctl.signal).catch(() => {
      // Aborted on navigation, or the run ended: the polled state remains authoritative.
    });
    return () => ctl.abort();
  }, [runId, active]);
  return (
    <section>
      <h2>Activity</h2>
      <pre className="log" aria-live="polite">
        {lines.length === 0 ? 'Waiting for events…' : lines.map((l) => `${l.event.padEnd(9)} ${l.data}`).join('\n')}
      </pre>
    </section>
  );
}

export function RunPage({ id }: { id: string }) {
  const detail = usePolling(() => api.run(id), 1000, (d) => TERMINAL.has(d.run.state));
  const [cancelMsg, setCancelMsg] = useState<string>();
  const run = detail.data?.run;
  const live = run === undefined || !TERMINAL.has(run.state);
  const properties = usePropertyOverview(id, live);

  if (detail.notFound) return <p className="error" role="alert">This run does not exist (it may have been removed).</p>;
  if (detail.error && !run) return <p className="error" role="alert">{detail.error}</p>;
  if (!run) return <p className="muted">Loading…</p>;

  const duration = run.started_at && run.finished_at ? formatDuration(run.finished_at - run.started_at) : undefined;
  return (
    <>
      <p>
        <Link to="/">← Runs</Link>
      </p>
      <h1>
        {run.target} <StateBadge state={run.state} />
      </h1>
      <dl className="facts">
        <dt>Run</dt>
        <dd><code>{run.id}</code></dd>
        <dt>Seed</dt>
        <dd><code>{run.seed}</code></dd>
        <dt>Backend</dt>
        <dd>{run.backend}</dd>
        <dt>Progress</dt>
        <dd>{run.steps.toLocaleString()} steps, {run.trials} trials{duration ? `, ${duration}` : ''}</dd>
        <dt>Created</dt>
        <dd>{relativeTime(run.created_at)}</dd>
      </dl>
      {run.error && <p className="error" role="alert">{run.error}</p>}
      {live && (
        <button
          onClick={async () => {
            try {
              const r = await api.cancel(run.id);
              setCancelMsg(r.cancelled ? 'Cancelled' : 'Already finished');
              detail.reload();
            } catch (e) {
              setCancelMsg(e instanceof Error ? e.message : String(e));
            }
          }}
        >
          Cancel run
        </button>
      )}
      {cancelMsg && <span className="muted"> {cancelMsg}</span>}
      <ErrorBoundary label="Property table" onRetry={properties.reload}>
        <PropertyOverviewPanel model={properties} />
      </ErrorBoundary>
      <p>
        <Link to={seedsPath(run.id, {}) ?? '/'}>{BROWSE_SEEDS}</Link>
      </p>
      <h2>Failures</h2>
      {detail.data?.failures.length === 0 && <p className="muted">{noFailuresText({ live, headline: properties.page?.headline })}</p>}
      <FirstFailingSeed runId={run.id} seed={detail.data?.properties?.first_failing_seed} />
      <ul>
        {detail.data?.failures.map((f) => (
          <li key={f.id}>
            <Link to={`/failures/${f.id}`}>{f.message}</Link> <span className="muted">({f.shrunk_len} steps)</span>
          </li>
        ))}
      </ul>
      <Activity runId={run.id} active={live} />
    </>
  );
}

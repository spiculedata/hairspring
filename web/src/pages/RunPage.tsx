import { useState } from 'react';
import { api } from '../api';
import { ActivityLog } from '../components/ActivityLog';
import { Link } from '../components/Layout';
import { StatusBadge } from '../components/StatusBadge';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { CoveragePanel } from '../coverage/CoveragePanel';
import { FirstFailingSeed } from '../explorer/FirstFailingSeed';
import { PropertyOverviewPanel } from '../explorer/PropertyOverviewPanel';
import { usePropertyOverview } from '../explorer/usePropertyOverview';
import { Loading, NotFound } from '../components/Notices';
import { TimeStamp } from '../components/TimeStamp';
import { useReportMissing } from '../PageStatus';
import { RunPanel } from '../runPanel/RunPanel';
import { UpdatedAt } from '../components/UpdatedAt';
import { formatDuration } from '../format';
import { seedsPath } from '../seeds/filters';
import { BROWSE_SEEDS, noFailuresText } from '../wording';
import { navigate, runPath, useSearch } from '../router';
import { usePolling } from '../usePolling';
import { Tabs } from '../components/Tabs';
import { RUN_TABS, runTabFromSearch, runTabSearch, type RunTabId } from './runTabs';
import { useRunActivity } from './useRunActivity';

const TERMINAL = new Set(['passed', 'failed', 'error', 'cancelled']);

export function RunPage({ id }: { id: string }) {
  const detail = usePolling(() => api.run(id), 1000, (d) => TERMINAL.has(d.run.state));
  const [cancelMsg, setCancelMsg] = useState<string>();
  const run = detail.data?.run;
  const live = run === undefined || !TERMINAL.has(run.state);
  const properties = usePropertyOverview(id, live);
  const activity = useRunActivity(id, live);
  const tab = runTabFromSearch(useSearch());
  useReportMissing(detail.notFound);

  if (detail.notFound) return <NotFound what="run" />;
  if (detail.error && !run) return <p className="error" role="alert">{detail.error}</p>;
  if (!run) return <Loading what="run" />;

  const failures = detail.data?.failures ?? [];
  const duration = run.started_at && run.finished_at ? formatDuration(run.finished_at - run.started_at) : undefined;
  return (
    <>
      <p>
        <Link to="/runs">← Runs</Link>
      </p>
      <h1>
        {run.target} <StatusBadge kind="run" state={run.state} trials={run.trials} completion={properties.page?.header.completion} />
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
        <dd>
          <TimeStamp ms={run.created_at} />
        </dd>
      </dl>
      <UpdatedAt at={detail.updatedAt} />
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
      {detail.data && (
        <ErrorBoundary label="What this run shows" onRetry={detail.reload}>
          <RunPanel detail={detail.data} page={properties.page} />
        </ErrorBoundary>
      )}
      <Tabs
        label="Run sections"
        idPrefix="run"
        tabs={RUN_TABS.map((t) => (t.id === 'failures' ? { ...t, count: failures.length } : t))}
        selected={tab}
        onSelect={(next: string) => navigate(`${runPath(run.id)}${runTabSearch(next as RunTabId)}`)}
      >
        {tab === 'properties' && (
          <>
            <ErrorBoundary label="Property table" onRetry={properties.reload}>
              <PropertyOverviewPanel model={properties} />
            </ErrorBoundary>
            <ErrorBoundary label="Coverage chart">
              <CoveragePanel runId={run.id} live={live} />
            </ErrorBoundary>
          </>
        )}
        {tab === 'seeds' && (
          <p>
            <Link to={seedsPath(run.id, {}) ?? '/'}>{BROWSE_SEEDS}</Link>
          </p>
        )}
        {tab === 'failures' && (
          <>
            <h2>Failures</h2>
            {failures.length === 0 && <p className="muted">{noFailuresText({ live, headline: properties.page?.headline })}</p>}
            <FirstFailingSeed runId={run.id} seed={detail.data?.properties?.first_failing_seed} />
            <ul>
              {failures.map((f) => (
                <li key={f.id}>
                  <Link to={`/failures/${f.id}`}>{f.message}</Link> <span className="muted">({f.shrunk_len} steps)</span>
                </li>
              ))}
            </ul>
          </>
        )}
        {tab === 'activity' && (
          <section>
            <h2>Activity</h2>
            <ActivityLog lines={activity} />
          </section>
        )}
      </Tabs>
    </>
  );
}

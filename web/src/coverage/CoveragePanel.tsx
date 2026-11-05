import { useEffect, useRef } from 'react';
import { api } from '../api';
import { LoadFailed, Loading } from '../components/Notices';
import { usePolling } from '../usePolling';
import { CoverageChart } from './CoverageChart';
import { isAllZero } from './coverageModel';
import { COVERAGE_EMPTY, COVERAGE_TITLE, coverageNothingSeen } from './coverageWording';

/** While a run is live its series grows, so it is loaded again every few seconds. */
export const COVERAGE_REFRESH_MS = 3000;

/**
 * The run page's coverage section. A live run is refreshed; when it ends the series is loaded once more, so the page
 * ends on the final figures and not on the last refresh that happened to land before the end.
 */
export function CoveragePanel({ runId, live }: { runId: string; live: boolean }) {
  const coverage = usePolling(() => api.coverage(runId), COVERAGE_REFRESH_MS, () => !live);
  const wasLive = useRef(live);
  useEffect(() => {
    if (wasLive.current && !live) coverage.reload();
    wasLive.current = live;
  }, [live, coverage.reload]);

  const { data, error } = coverage;
  return (
    <section className="coverage" aria-labelledby="coverage-title">
      <h2 id="coverage-title">{COVERAGE_TITLE}</h2>
      {error && <LoadFailed what="coverage" message={error} retries={live} />}
      {!data && !error && <Loading what="coverage" />}
      {data && data.points.length === 0 && <p className="muted">{COVERAGE_EMPTY}</p>}
      {data && data.points.length > 0 && isAllZero(data.points) && <p className="muted">{coverageNothingSeen(data.trials)}</p>}
      {data && data.points.length > 0 && !isAllZero(data.points) && <CoverageChart coverage={data} />}
    </section>
  );
}

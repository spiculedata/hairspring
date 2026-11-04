import { api } from '../api';
import { Loading, LoadFailed } from '../components/Notices';
import { UpdatedAt } from '../components/UpdatedAt';
import { Link } from '../components/Layout';
import { FirstRunGuide } from '../overview/FirstRunGuide';
import { AttentionFailures, AttentionRuns, RecentRuns, WorkersSummary } from '../overview/OverviewSections';
import { attentionRuns, recentRuns } from '../overview/overviewModel';
import {
  ATTENTION_HEADING,
  NO_ATTENTION_FAILURES,
  NO_ATTENTION_RUNS,
  OVERVIEW_HEADING,
  GLOSSARY_HINT_AFTER,
  GLOSSARY_HINT_BEFORE,
  GLOSSARY_HINT_LINK,
  WORKERS_HEADING,
  lastGoodLoad,
  attentionNone,
} from '../overview/overviewWording';
import type { FailureSummary, Run } from '../types';
import { usePolling } from '../usePolling';

/** How many of the newest runs are fetched; "needs attention" and "recent runs" are both chosen from these. */
const RUNS_LOOKED_AT = 20;
const ATTENTION_SHOWN = 5;
const RECENT_SHOWN = 5;
const FAILURES_LOOKED_AT = 5;
const RUNS_POLL_MS = 5000;
const FAILURES_POLL_MS = 5000;
const WORKERS_POLL_MS = 10_000;
const TARGETS_POLL_MS = 60_000;

interface AttentionProps {
  readonly runs: readonly Run[] | undefined;
  readonly runsFailed: boolean;
  readonly failures: readonly FailureSummary[] | undefined;
  readonly failuresFailed: boolean;
}

/**
 * Failed or errored runs among the newest runs, and the newest stored failures. Both come straight from server fields.
 * "None" is said only when both lists loaded without error, and it states what was looked at.
 */
function Attention({ runs, runsFailed, failures, failuresFailed }: AttentionProps) {
  const failedRuns = runs === undefined ? [] : attentionRuns(runs, ATTENTION_SHOWN);
  const failureCount = failures?.length ?? 0;
  const bothLoaded = runs !== undefined && failures !== undefined && !runsFailed && !failuresFailed;
  return (
    <section aria-labelledby="overview-attention">
      <h2 id="overview-attention">{ATTENTION_HEADING}</h2>
      {runs === undefined && !runsFailed && <Loading what="runs" />}
      {failures === undefined && !failuresFailed && <Loading what="failures" />}
      {bothLoaded && failedRuns.length === 0 && failureCount === 0 && <p className="muted">{attentionNone(runs.length)}</p>}
      {failedRuns.length > 0 && <AttentionRuns runs={failedRuns} />}
      {failedRuns.length === 0 && runs !== undefined && !runsFailed && failureCount > 0 && <p className="muted">{NO_ATTENTION_RUNS}</p>}
      {failureCount > 0 && failures !== undefined && <AttentionFailures failures={failures} limit={FAILURES_LOOKED_AT} />}
      {failureCount === 0 && failures !== undefined && !failuresFailed && failedRuns.length > 0 && <p className="muted">{NO_ATTENTION_FAILURES}</p>}
    </section>
  );
}

/**
 * The home page: what needs a look, the newest runs, how many workers the server lists and, when the server has no stored
 * runs, what a newcomer does next. Four endpoints are polled on their own intervals, each with its own error, so one that
 * fails never hides or blanks the others, and an error is never read as "nothing there".
 */
export function OverviewPage() {
  const runs = usePolling(() => api.runs(RUNS_LOOKED_AT), RUNS_POLL_MS);
  const failures = usePolling(() => api.failures(FAILURES_LOOKED_AT), FAILURES_POLL_MS);
  const workers = usePolling(api.workers, WORKERS_POLL_MS);
  const targets = usePolling(api.targets, TARGETS_POLL_MS);
  const hasNoRuns = runs.data?.length === 0;
  // After a failed poll the last good list may be out of date, so an empty one no longer shows the guide.
  const isRunsEmpty = hasNoRuns && runs.error === undefined;
  const hasNoFailures = failures.data?.length === 0;
  const showGuide = isRunsEmpty;
  const showAttention = !(hasNoRuns && hasNoFailures);
  return (
    <>
      <h1>{OVERVIEW_HEADING}</h1>
      {runs.error && <LoadFailed what="runs" message={runs.error} />}
      {runs.error && runs.data !== undefined && <p className="muted">{lastGoodLoad('runs')}</p>}
      {failures.error && <LoadFailed what="failures" message={failures.error} />}
      {failures.error && failures.data !== undefined && <p className="muted">{lastGoodLoad('failures')}</p>}
      {workers.error && <LoadFailed what="workers" message={workers.error} />}
      {workers.error && workers.data !== undefined && <p className="muted">{lastGoodLoad('workers')}</p>}
      {showGuide && targets.error && <LoadFailed what="targets" message={targets.error} />}
      <UpdatedAt at={runs.updatedAt} />
      {showGuide && <FirstRunGuide targets={targets.data} targetsFailed={targets.error !== undefined} />}
      {showAttention && (
        <Attention
          runs={runs.data}
          runsFailed={runs.error !== undefined}
          failures={failures.data}
          failuresFailed={failures.error !== undefined}
        />
      )}
      {runs.data !== undefined && !hasNoRuns && (
        <section aria-labelledby="overview-recent">
          <RecentRuns runs={recentRuns(runs.data, RECENT_SHOWN)} />
        </section>
      )}
      <section aria-labelledby="overview-workers">
        <h2 id="overview-workers">{WORKERS_HEADING}</h2>
        {workers.data !== undefined ? (
          <WorkersSummary workers={workers.data} />
        ) : (
          workers.error === undefined && <Loading what="workers" />
        )}
      </section>
      <p className="muted">
        {GLOSSARY_HINT_BEFORE}
        <Link to="/glossary">{GLOSSARY_HINT_LINK}</Link>
        {GLOSSARY_HINT_AFTER}
      </p>
    </>
  );
}

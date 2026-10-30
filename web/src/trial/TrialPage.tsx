import { useState } from 'react';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { ErrorLine } from '../components/ErrorLine';
import { Loading, LoadFailed, NotFound } from '../components/Notices';
import { Link } from '../components/Layout';
import { useReportMissing } from '../PageStatus';
import { runPath } from '../router';
import { usePolling } from '../usePolling';
import { trialApi } from './api';
import { TrialView } from './TrialView';
import { downloadOutput, useOutputTails, useRunContext, useTrialDetail } from './useTrial';
import type { StreamName } from './types';

const LOOKUP_RETRY_MS = 30_000;

function Loaded({ runId, seq }: { runId: string; seq: number }) {
  const detail = useTrialDetail(runId, seq);
  const context = useRunContext(runId);
  const tails = useOutputTails(runId, seq, detail.data?.output_streams ?? []);
  const [downloadError, setDownloadError] = useState<string>();
  useReportMissing(detail.notFound);

  if (detail.notFound) return <NotFound what="trial" />;
  if (detail.error && !detail.data) return <LoadFailed what="this trial" message={detail.error} />;
  if (!detail.data) return <Loading what="trial" />;
  const onDownload = (stream: StreamName) => {
    setDownloadError(undefined);
    downloadOutput(runId, seq, stream).catch((e: unknown) => setDownloadError(e instanceof Error ? e.message : String(e)));
  };
  return (
    <>
      <TrialView detail={detail.data} context={context} tails={tails} onDownload={onDownload} />
      {downloadError !== undefined && <ErrorLine message={downloadError} />}
    </>
  );
}

/** The failure page for one trial of a run, by sequence number. */
export function TrialPage({ runId, seq }: { runId: string; seq: number }) {
  return (
    <ErrorBoundary label="Failure page">
      <Loaded runId={runId} seq={seq} />
    </ErrorBoundary>
  );
}

/** The failure page for the trial that ran a seed: the seed is looked up once, then the trial page is shown. */
export function SeedPage({ runId, seed }: { runId: string; seed: string }) {
  const lookup = usePolling(() => trialApi.seqOfSeed(runId, seed).then((seq) => ({ seq })), LOOKUP_RETRY_MS, () => true);
  useReportMissing(lookup.notFound);
  if (lookup.notFound) return <NotFound what="run" />;
  if (lookup.error && !lookup.data) return <LoadFailed what="this seed" message={lookup.error} />;
  if (!lookup.data) return <Loading what="seed" />;
  if (lookup.data.seq === undefined) {
    return (
      <>
        <p>
          <Link to={runPath(runId)}>← Run</Link>
        </p>
        <p className="muted">This run has no stored trial for that seed. Trial rows are kept for recent runs only.</p>
      </>
    );
  }
  return <TrialPage runId={runId} seq={lookup.data.seq} />;
}

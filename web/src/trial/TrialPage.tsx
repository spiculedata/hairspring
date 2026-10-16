import { useState } from 'react';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { runPath } from '../router';
import { usePolling } from '../usePolling';
import { trialApi } from './api';
import { TrialView } from './TrialView';
import { downloadOutput, useOutputTails, useRunContext, useTrialDetail } from './useTrial';
import type { StreamName } from './types';

const NOT_FOUND_TEXT = 'This trial does not exist in this run (the run may have been removed, or its trial rows expired).';
const ERROR_CHARS = 300;
const LOOKUP_RETRY_MS = 30_000;

/** An error message as escaped text: it is only ever words from the server or the network, and is treated as untrusted anyway. */
function ErrorLine({ message }: { message: string }) {
  return (
    <p className="error" role="alert">
      <GuestText text={message} max={ERROR_CHARS} expandable={false} />
    </p>
  );
}

function Loaded({ runId, seq }: { runId: string; seq: number }) {
  const detail = useTrialDetail(runId, seq);
  const context = useRunContext(runId);
  const tails = useOutputTails(runId, seq, detail.data?.output_streams ?? []);
  const [downloadError, setDownloadError] = useState<string>();

  if (detail.notFound) return <ErrorLine message={NOT_FOUND_TEXT} />;
  if (detail.error && !detail.data) return <ErrorLine message={detail.error} />;
  if (!detail.data) return <p className="muted">Loading…</p>;
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
  if (lookup.notFound) return <ErrorLine message="This run does not exist." />;
  if (lookup.error && !lookup.data) return <ErrorLine message={lookup.error} />;
  if (!lookup.data) return <p className="muted">Loading…</p>;
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

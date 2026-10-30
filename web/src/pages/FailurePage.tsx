import { useState } from 'react';
import { api } from '../api';
import { useReportMissing } from '../PageStatus';
import { Link } from '../components/Layout';
import { Loading, LoadFailed, NotFound } from '../components/Notices';
import { ReplayVerdict } from '../components/ReplayVerdict';
import type { ReplayResult } from '../types';
import { usePolling } from '../usePolling';

function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

export function FailurePage({ id }: { id: string }) {
  const detail = usePolling(() => api.failure(id), 3_600_000, () => true);
  const [replay, setReplay] = useState<ReplayResult>();
  const [replayError, setReplayError] = useState<string>();
  const [open, setOpen] = useState<number>();
  useReportMissing(detail.notFound);

  if (detail.notFound) return <NotFound what="failure" />;
  if (detail.error) return <LoadFailed what="this failure" message={detail.error} retries={false} />;
  if (!detail.data) return <Loading what="failure" />;
  const { record, steps, bundle } = detail.data;
  const stepsToShow = replay?.steps ?? steps;

  return (
    <>
      <p>
        <Link to={`/runs/${record.run_id}`}>← Run</Link>
      </p>
      <h1>{record.target}: shortest failing steps</h1>
      <p>{record.message}</p>
      <div className="row">
        <button
          onClick={async () => {
            try {
              setReplayError(undefined);
              setReplay(await api.replay(bundle));
            } catch (e) {
              setReplayError(e instanceof Error ? e.message : String(e));
            }
          }}
        >
          Replay
        </button>
        <button onClick={() => download(`${record.target}-${record.bundle_hash.slice(0, 8)}.hairspring`, bundle)}>Download bundle</button>
        <button onClick={() => void navigator.clipboard?.writeText(bundle)}>Copy bundle</button>
      </div>
      {replayError && <p className="error" role="alert">{replayError}</p>}
      {replay && <ReplayVerdict failedAgain={replay.reproduced} />}
      <ol className="timeline">
        {stepsToShow.map((s) => (
          <li key={s.index}>
            <button className="linklike" aria-expanded={open === s.index} onClick={() => setOpen(open === s.index ? undefined : s.index)}>
              <span className="step-n">{s.index + 1}</span> {s.label}
            </button>
            {open === s.index && <pre className="state">{s.state}</pre>}
          </li>
        ))}
      </ol>
      <h2>Bundle</h2>
      <pre className="log">{bundle}</pre>
    </>
  );
}

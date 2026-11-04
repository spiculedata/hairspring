import { useEffect, useState } from 'react';
import { Link } from '../components/Layout';
import { failurePath, trialPath } from '../router';
import { CopyButton } from '../share/CopyButton';
import { failureSummary } from '../share/summary';
import { trialApi } from '../trial/api';
import type { FailureSummary } from '../types';

/**
 * The sequence number of the run's first violating trial. A failed lookup, or a run with none kept (a simulation
 * target has no trials), shows no link and no error: the failure page is complete without it.
 */
function useFirstViolationSeq(runId: string): number | undefined {
  const [seq, setSeq] = useState<number>();
  useEffect(() => {
    setSeq(undefined);
    const ctl = new AbortController();
    trialApi
      .firstViolation(runId, ctl.signal)
      .then((row) => {
        if (!ctl.signal.aborted && row !== undefined) setSeq(row.seq);
      })
      .catch(() => undefined);
    return () => ctl.abort();
  }, [runId]);
  return seq;
}

/** What a reader can do from a bundle failure: open the run's failing trial, copy the link, copy a summary with no program text. */
export function FailureActions({ record }: { record: FailureSummary }) {
  const seq = useFirstViolationSeq(record.run_id);
  const trialHref = seq === undefined ? undefined : trialPath(record.run_id, seq);
  const link = (): string => new URL(failurePath(record.id) ?? window.location.pathname, window.location.href).href;
  return (
    <div className="row failure-actions">
      {trialHref !== undefined && seq !== undefined && <Link to={trialHref}>{`Open trial ${seq}`}</Link>}
      <CopyButton label="Copy link" text={link} />
      <CopyButton label="Copy summary" text={() => failureSummary({ record, link: link() })} />
    </div>
  );
}

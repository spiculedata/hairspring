import { useEffect, useState } from 'react';
import { api } from '../api';
import { Link } from '../components/Layout';
import { failurePath, trialPath } from '../router';
import { CopyButton } from '../share/CopyButton';
import { trialSummary } from '../share/summary';
import type { RunContext } from './TrialHeader';
import type { TrialRow } from './types';

const LINK_TEXT = 'Shortest failing steps for this run';

/**
 * The id of the run's first bundle failure, from the run's own detail. A failed lookup shows no link and no error: the
 * trial page is complete without it.
 */
function useRunFailureId(runId: string): string | undefined {
  const [id, setId] = useState<string>();
  useEffect(() => {
    let current = true;
    api
      .run(runId)
      .then((d) => {
        const first = Array.isArray(d.failures) ? d.failures[0] : undefined;
        if (current && first !== undefined) setId(first.id);
      })
      .catch(() => undefined);
    return () => {
      current = false;
    };
  }, [runId]);
  return id;
}

interface Props {
  runId: string;
  trial: TrialRow;
  context: RunContext | undefined;
}

/** What a reader can do from a trial: go to the run's bundle failure, copy the link, copy a summary with no program output. */
export function TrialActions({ runId, trial, context }: Props) {
  const failureId = useRunFailureId(runId);
  const failureHref = failureId === undefined ? undefined : failurePath(failureId);
  const link = (): string => new URL(trialPath(runId, trial.seq) ?? window.location.pathname, window.location.href).href;
  return (
    <div className="row trial-actions">
      {failureHref !== undefined && <Link to={failureHref}>{LINK_TEXT}</Link>}
      <CopyButton label="Copy link" text={link} />
      <CopyButton
        label="Copy summary"
        text={() =>
          trialSummary({
            runId,
            trial,
            link: link(),
            completionLabel: context?.completionLabel,
            stoppedAtFirstFailure: context?.stoppedAtFirstFailure === true,
          })
        }
      />
    </div>
  );
}

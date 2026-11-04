import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { TierChips } from '../components/TierChips';
import { runPath } from '../router';
import { trialTitle } from '../wording';
import type { TrialRow } from './types';

/** What the run this trial belongs to says about how it ended, when the page could load it. */
export interface RunContext {
  readonly stoppedAtFirstFailure: boolean;
  readonly completionLabel: string;
}

const REASON_CHARS = 300;

function exitText(t: TrialRow): string {
  if (t.signal !== null) return `signal ${t.signal}`;
  if (t.timed_out) return 'timed out';
  return t.exit_code === null ? 'no exit status' : `exit ${t.exit_code}`;
}

/** The title, label chips and how the trial ended. The caveats that apply are in the ranked caveat and About sections. */
export function TrialHeader({ runId, trial, context }: { runId: string; trial: TrialRow; context: RunContext | undefined }) {
  return (
    <header className="trial-header">
      <p>
        <Link to={runPath(runId)}>← Run</Link>
      </p>
      <h1>{trialTitle(trial.verdict, trial.seed)}</h1>
      <TierChips chips={[]} caveats={[]} unverified={!trial.worker_trusted} />
      {trial.guest.reason !== '' && (
        <p>
          <GuestText text={trial.guest.reason} max={REASON_CHARS} />
        </p>
      )}
      <p className="muted">
        Trial {trial.seq} · {exitText(trial)} · {trial.rated ? 'rated' : 'not rated'} · {trial.degraded ? 'degraded' : 'not degraded'}
        {context !== undefined && ` · run: ${context.completionLabel}`}
      </p>
    </header>
  );
}

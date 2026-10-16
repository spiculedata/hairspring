import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { TierChips } from '../components/TierChips';
import { runPath } from '../router';
import { NOT_A_RATE_TEXT, NOT_REPLAYED_NOTE, PERTURBING_NOTE, STOPPED_TRIAL_NOTE, trialTitle } from '../wording';
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

/** The title, label chips, how the trial ended and what this page does and does not claim. */
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
      {trial.perturbing && <p className="note">{PERTURBING_NOTE}.</p>}
      {context?.stoppedAtFirstFailure === true && (
        <p className="note">
          {STOPPED_TRIAL_NOTE} ({NOT_A_RATE_TEXT})
        </p>
      )}
      <p className="muted">{NOT_REPLAYED_NOTE}</p>
    </header>
  );
}

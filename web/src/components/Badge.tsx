import { runOutcome } from '../runOutcome';
import type { RunCompletion } from '../types';

export function StateBadge({ state, trials, completion }: { state: string; trials?: number; completion?: RunCompletion }) {
  const outcome = runOutcome({ state, trials: trials ?? 0, completion });
  // Without a trial count there is no honest scope to give, so the label stands alone.
  const scope = trials === undefined ? undefined : outcome.scope;
  return (
    <>
      <span className={`badge badge-${outcome.tone}`}>{outcome.label}</span>
      {scope !== undefined && <span className="muted badge-scope"> {scope}</span>}
    </>
  );
}

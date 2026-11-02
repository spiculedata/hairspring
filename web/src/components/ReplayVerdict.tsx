import { REPLAY_CAVEAT } from '../wording';
import { StatusBadge } from './StatusBadge';

/**
 * The result of replaying a bundle once. It says whether this replay failed, and that one replay is not a
 * confirmation; the server's confirmation checks are the only place a failure is called confirmed. The verdict is a
 * badge, neutral when the replay did not fail, and the caveat is in the neutral text colour, so it does not read as
 * part of the verdict.
 */
export function ReplayVerdict({ failedAgain }: { failedAgain: boolean }) {
  return (
    <div role="status">
      <p>
        <StatusBadge kind="replay" failedAgain={failedAgain} />
      </p>
      <p className="muted">{REPLAY_CAVEAT}</p>
    </div>
  );
}

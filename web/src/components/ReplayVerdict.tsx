import { REPLAY_CAVEAT, replayVerdict } from '../wording';

/**
 * The result of replaying a bundle once. It says whether this replay failed, and that one replay is not a
 * confirmation; the server's confirmation checks are the only place a failure is called confirmed.
 */
export function ReplayVerdict({ failedAgain }: { failedAgain: boolean }) {
  return (
    <p role="status" className={failedAgain ? 'error' : 'ok'}>
      {replayVerdict(failedAgain)} {REPLAY_CAVEAT}
    </p>
  );
}

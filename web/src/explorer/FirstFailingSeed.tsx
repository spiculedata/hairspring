import { Link } from '../components/Layout';
import { seedPath } from '../router';

/**
 * A link from a run to the failure page of its lowest failing seed. "Lowest" is the server's own ordering of the seeds
 * that failed; it says nothing about how often the run fails, so no rate is shown here.
 */
export function FirstFailingSeed({ runId, seed }: { runId: string; seed: string | null | undefined }) {
  if (seed === null || seed === undefined) return null;
  const to = seedPath(runId, seed);
  if (to === undefined) return null;
  return (
    <p>
      Lowest failing seed: <Link to={to}>{seed}</Link> (open its failure page)
    </p>
  );
}

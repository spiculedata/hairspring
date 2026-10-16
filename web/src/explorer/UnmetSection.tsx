import { GuestText } from '../components/GuestText';
import type { PropertyRow, RunHeader } from '../types';
import { COVERAGE_GAPS_NOTE, COVERAGE_GAPS_TITLE, moreUnmet, unmetNotStated, unmetSuppressedReason } from '../wording';
import { kindLabel } from './PropertyRowView';

interface Props {
  rows: readonly PropertyRow[];
  /** UNMET properties in the whole run, from the status counts. */
  unmetTotal: number;
  header: RunHeader;
}

/**
 * UNMET results, apart from violations and worded as coverage gaps (plan 5.7 rule 3). When the server withholds the
 * claim (stopped at first failure, incomplete ingest) the section says why instead of stating a gap.
 */
export function UnmetSection({ rows, unmetTotal, header }: Props) {
  const unmet = rows.filter((r) => r.status === 'unmet');
  if (unmet.length === 0 && unmetTotal === 0) return null;
  const suppressed = unmet.some((r) => r.unmet === null);
  const notLoaded = Math.max(0, unmetTotal - unmet.length);
  return (
    <section className="unmet" aria-labelledby="unmet-title">
      <h2 id="unmet-title">{COVERAGE_GAPS_TITLE}</h2>
      <p className="muted">{COVERAGE_GAPS_NOTE}</p>
      {suppressed && <p>{unmetNotStated(unmetSuppressedReason(header.completion, header.ingest_incomplete))}</p>}
      <ul>
        {unmet.map((r) => (
          <li key={r.idx}>
            <span className="kind">{kindLabel(r.kind)}</span> <GuestText text={r.guest.id} max={96} expandable={false} />
            {r.unmet !== null && <span className="unmet-note">{r.unmet}</span>}
          </li>
        ))}
      </ul>
      {notLoaded > 0 && <p className="muted">{moreUnmet(notLoaded)}</p>}
    </section>
  );
}
